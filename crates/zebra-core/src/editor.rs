//! The editor: the single mutation point for document and view state.
use std::collections::{BTreeSet, HashSet, VecDeque};

use base64::Engine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::camera::Camera;
use crate::command::*;
use crate::error::{ZError, ZResult};
use crate::geom::{Rect, Transform, Vec2};
use crate::model::*;
use crate::theme::{valid_color, Theme};
use crate::view::{Mode, Permission, ViewState};

/// Who issued a change. Shown in history and the activity panel.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Origin {
    Human,
    Agent { name: String },
}

impl Origin {
    pub fn agent(name: impl Into<String>) -> Self {
        Origin::Agent { name: name.into() }
    }
    pub fn is_agent(&self) -> bool {
        matches!(self, Origin::Agent { .. })
    }
    pub fn display(&self) -> String {
        match self {
            Origin::Human => "You".into(),
            Origin::Agent { name } => name.clone(),
        }
    }
}

/// Measures text. The default is a rough estimator; the renderer installs a
/// real (parley) measurer so layout, hit testing and drawing agree.
pub trait TextMeasure: Send + Sync {
    /// Returns (width, height) in local units. `wrap_width` None = no wrapping.
    fn measure(&self, text: &TextData, theme: &Theme, wrap_width: Option<f64>) -> (f64, f64);
}

pub struct EstimateMeasure;

impl TextMeasure for EstimateMeasure {
    fn measure(&self, t: &TextData, theme: &Theme, wrap: Option<f64>) -> (f64, f64) {
        let size = theme.text_style(t.style).size;
        let cw = size * if t.bold { 0.6 } else { 0.55 };
        let indent = if t.bullets { size * 1.2 } else { 0.0 };
        let mut lines = 0usize;
        let mut maxw: f64 = 0.0;
        for para in t.text.split('\n') {
            let w = para.chars().count() as f64 * cw + indent;
            match wrap {
                Some(ww) if ww > 0.0 && w > ww => {
                    lines += (w / ww).ceil() as usize;
                    maxw = maxw.max(ww);
                }
                _ => {
                    lines += 1;
                    maxw = maxw.max(w);
                }
            }
        }
        (maxw.max(size * 0.6), lines.max(1) as f64 * size * 1.25)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub label: String,
    pub origin: Origin,
    pub revision: u64,
    pub time: f64,
    pub ids: Vec<ElementId>,
    /// What changed, oldest first (several when agent steps were grouped).
    /// Undo applies the backward patches newest first.
    #[serde(skip)]
    deltas: Vec<crate::delta::Delta>,
    /// Unlabelled agent steps auto-group with the next one (within 2 s);
    /// explicitly labelled batches and transactions never merge.
    #[serde(skip)]
    mergeable: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Outcome {
    pub revision: u64,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub created: Vec<ElementId>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub changed: Vec<ElementId>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub deleted: Vec<ElementId>,
}

impl Outcome {
    fn touched(&self) -> Vec<ElementId> {
        let mut v = self.created.clone();
        v.extend(self.changed.iter().cloned());
        v
    }
}

/// Notifications for API subscribers and the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    DocumentChanged { revision: u64, origin: Origin, label: String, ids: Vec<ElementId> },
    SelectionChanged { selection: Vec<ElementId> },
    ModeChanged { mode: Mode },
    StepChanged { step: Option<usize>, build: usize },
    AgentsPaused { paused: bool },
    Saved { path: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub time: f64,
    pub origin: Origin,
    pub label: String,
    pub ids: Vec<ElementId>,
}

struct Tx {
    label: String,
    origin: Origin,
    before: Document,
    ids: BTreeSet<ElementId>,
}

pub struct Editor {
    pub doc: Document,
    pub view: ViewState,
    pub revision: u64,
    saved_revision: u64,
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    tx: Option<Tx>,
    pub measure: Box<dyn TextMeasure>,
    /// Newest last. Capped.
    pub activity: VecDeque<Activity>,
    /// Elements recently touched by an agent: (id, time). For the flash outline.
    pub flashes: Vec<(ElementId, f64)>,
    /// Where the agent last acted (world), for the agent cursor.
    pub agent_cursor: Option<Vec2>,
    pub file_path: Option<std::path::PathBuf>,
    events: Vec<Event>,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new(Document::default())
    }
}

const AGENT_GROUP_SECS: f64 = 2.0;

impl Editor {
    pub fn new(doc: Document) -> Self {
        Editor {
            doc,
            view: ViewState::default(),
            revision: 0,
            saved_revision: 0,
            undo: vec![],
            redo: vec![],
            tx: None,
            measure: Box::new(EstimateMeasure),
            activity: VecDeque::new(),
            flashes: vec![],
            agent_cursor: None,
            file_path: None,
            events: vec![],
        }
    }

    pub fn theme(&self) -> Theme {
        Theme::resolve(&self.doc.theme)
    }
    pub fn dirty(&self) -> bool {
        self.revision != self.saved_revision
    }
    pub fn mark_saved(&mut self, path: Option<std::path::PathBuf>) {
        self.saved_revision = self.revision;
        if let Some(p) = path {
            self.events.push(Event::Saved { path: p.display().to_string() });
            self.file_path = Some(p);
        }
    }
    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
    pub fn history(&self) -> &[HistoryEntry] {
        &self.undo
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<String> {
        self.undo.last().map(|h| format!("{} ({})", h.label, h.origin.display()))
    }
    pub fn in_transaction(&self) -> bool {
        self.tx.is_some()
    }
    pub fn human_gesture_active(&self) -> bool {
        matches!(&self.tx, Some(t) if t.origin == Origin::Human)
    }

    /// Replace the whole document (open file). Clears history.
    pub fn load(&mut self, mut doc: Document, path: Option<std::path::PathBuf>) {
        self.tx = None;
        self.remeasure_all(&mut doc);
        self.doc = doc;
        self.undo.clear();
        self.redo.clear();
        self.revision += 1;
        self.saved_revision = self.revision;
        self.file_path = path;
        self.view.selection.clear();
        self.view.agent_selection.clear();
        self.view.present = Default::default();
        self.view.mode = Mode::Edit;
        let cam = self.fit_all_camera();
        self.view.fly_to(cam, false);
        self.view.agent_camera = cam;
        self.events.push(Event::DocumentChanged {
            revision: self.revision,
            origin: Origin::Human,
            label: "Open".into(),
            ids: vec![],
        });
    }

    fn remeasure_all(&self, doc: &mut Document) {
        let theme = Theme::resolve(&doc.theme);
        for e in doc.elements.values_mut() {
            measure_text(&*self.measure, &theme, e);
        }
    }

    // ------------------------------------------------------------------
    // Transactions (a drag, or an agent's named batch) = one undo step.

    /// Start a transaction. Applies inside it are live but recorded once.
    pub fn begin(&mut self, label: impl Into<String>, origin: Origin) -> ZResult<()> {
        if let Some(t) = &self.tx {
            if t.origin == origin {
                return Ok(()); // nested: join
            }
            return Err(busy());
        }
        self.check_agent(&origin)?;
        self.tx = Some(Tx { label: label.into(), origin, before: self.doc.clone(), ids: BTreeSet::new() });
        Ok(())
    }

    /// Commit the open transaction. No-op if nothing changed.
    pub fn commit(&mut self) {
        let Some(tx) = self.tx.take() else { return };
        if tx.before == self.doc {
            return;
        }
        let ids: Vec<_> = tx.ids.into_iter().collect();
        self.record(tx.label, tx.origin, tx.before, ids, false);
    }

    /// Abort: restore the document as it was at `begin`.
    pub fn abort(&mut self) {
        if let Some(tx) = self.tx.take() {
            if tx.before != self.doc {
                self.doc = tx.before;
                self.revision += 1;
                self.events.push(Event::DocumentChanged {
                    revision: self.revision,
                    origin: tx.origin,
                    label: format!("Cancel {}", tx.label),
                    ids: vec![],
                });
            }
        }
    }

    fn check_agent(&self, origin: &Origin) -> ZResult<()> {
        if origin.is_agent() {
            if self.view.agent_paused {
                return Err(ZError::new("paused", "the human paused agents")
                    .hint("wait for the human to resume (watch the agents_paused event)"));
            }
            if self.view.agent_permission == Permission::Observe {
                return Err(ZError::new("permission", "agent permission is 'observe' (read-only)"));
            }
            if self.view.mode == Mode::Present && self.view.agent_permission != Permission::Full {
                return Err(ZError::new("presenting", "the human is presenting; edits are held")
                    .hint("retry after the mode_changed event"));
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Applying commands

    pub fn apply(&mut self, cmd: Command, origin: Origin) -> ZResult<Outcome> {
        let label = command_label(&cmd);
        self.apply_batch_inner(vec![cmd], Some(label), origin, true)
    }

    /// Atomic: all commands apply or none. One undo step.
    /// A batch with an explicit `label` is its own undo step (never merged
    /// with neighbouring agent steps).
    pub fn apply_batch(&mut self, cmds: Vec<Command>, label: Option<String>, origin: Origin) -> ZResult<Outcome> {
        let mergeable = label.is_none();
        self.apply_batch_inner(cmds, label, origin, mergeable)
    }

    fn apply_batch_inner(&mut self, cmds: Vec<Command>, label: Option<String>, origin: Origin, mergeable: bool) -> ZResult<Outcome> {
        self.check_agent(&origin)?;
        if let Some(t) = &self.tx {
            if t.origin != origin {
                return Err(busy());
            }
        }
        let label = label.unwrap_or_else(|| match cmds.as_slice() {
            [c] => command_label(c),
            _ => format!("{} changes", cmds.len()),
        });
        let before = self.doc.clone();
        let mut out = Outcome::default();
        for (i, cmd) in cmds.into_iter().enumerate() {
            if let Err(mut e) = self.apply_inner(cmd, &mut out) {
                self.doc = before;
                if i > 0 {
                    e.message = format!("command #{i} failed (batch rolled back): {}", e.message);
                }
                return Err(e);
            }
        }
        if self.doc == before {
            out.revision = self.revision;
            return Ok(out);
        }
        // fix selections pointing at deleted elements
        let doc = &self.doc;
        self.view.selection.retain(|id| doc.elements.contains_key(id));
        self.view.agent_selection.retain(|id| doc.elements.contains_key(id));

        let ids = out.touched();
        if let Some(tx) = &mut self.tx {
            tx.ids.extend(ids.iter().cloned());
            self.revision += 1;
            self.events.push(Event::DocumentChanged { revision: self.revision, origin: origin.clone(), label, ids });
        } else {
            self.record(label, origin.clone(), before, ids, mergeable);
        }
        if origin.is_agent() {
            let now = self.view.now;
            for id in out.touched() {
                self.flashes.push((id, now));
            }
            if let Some(c) = out.touched().last().and_then(|id| self.doc.get(id)) {
                self.agent_cursor = Some(c.bounds().center());
            }
            self.flashes.retain(|(_, t)| now - *t < 2.0);
        }
        out.revision = self.revision;
        Ok(out)
    }

    fn record(&mut self, label: String, origin: Origin, before: Document, ids: Vec<ElementId>, allow_merge: bool) {
        self.revision += 1;
        let now = self.view.now;
        self.redo.clear();
        let delta = crate::delta::Delta::between(&before, &self.doc);
        drop(before);
        let merged = allow_merge
            && origin.is_agent()
            && self.undo.last().is_some_and(|h| h.mergeable && h.origin == origin && now - h.time < AGENT_GROUP_SECS);
        if merged {
            let h = self.undo.last_mut().unwrap();
            let n = h.label.strip_suffix(" changes").and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
            h.label = format!("{} changes", n + 1);
            h.time = now;
            h.revision = self.revision;
            h.ids.extend(ids.iter().cloned());
            h.deltas.push(delta);
        } else {
            self.undo.push(HistoryEntry { label: label.clone(), origin: origin.clone(), revision: self.revision, time: now, ids: ids.clone(), deltas: vec![delta], mergeable: allow_merge });
            if self.undo.len() > 500 {
                self.undo.remove(0);
            }
        }
        self.activity.push_back(Activity { time: now, origin: origin.clone(), label: label.clone(), ids: ids.clone() });
        while self.activity.len() > 300 {
            self.activity.pop_front();
        }
        self.events.push(Event::DocumentChanged { revision: self.revision, origin, label, ids });
    }

    pub fn undo(&mut self, origin: Origin) -> ZResult<Option<String>> {
        self.check_agent(&origin)?;
        self.commit();
        let Some(h) = self.undo.pop() else { return Ok(None) };
        if let Err(e) = replay(&mut self.doc, h.deltas.iter().rev().map(|d| (&d.backward, &d.forward))) {
            self.undo.push(h);
            return Err(ZError::new("internal", format!("undo failed: {e}")));
        }
        let label = h.label.clone();
        self.redo.push(h);
        self.after_history_jump(&format!("Undo {label}"), origin);
        Ok(Some(label))
    }

    pub fn redo(&mut self, origin: Origin) -> ZResult<Option<String>> {
        self.check_agent(&origin)?;
        self.commit();
        let Some(h) = self.redo.pop() else { return Ok(None) };
        if let Err(e) = replay(&mut self.doc, h.deltas.iter().map(|d| (&d.forward, &d.backward))) {
            self.redo.push(h);
            return Err(ZError::new("internal", format!("redo failed: {e}")));
        }
        let label = h.label.clone();
        self.undo.push(h);
        self.after_history_jump(&format!("Redo {label}"), origin);
        Ok(Some(label))
    }

    /// Bytes held by undo and redo deltas (approximate, for diagnostics).
    pub fn history_bytes(&self) -> usize {
        self.undo.iter().chain(&self.redo).flat_map(|h| &h.deltas).map(|d| d.approx_bytes()).sum()
    }

    fn after_history_jump(&mut self, label: &str, origin: Origin) {
        self.revision += 1;
        let doc = &self.doc;
        self.view.selection.retain(|id| doc.elements.contains_key(id));
        self.view.agent_selection.retain(|id| doc.elements.contains_key(id));
        self.events.push(Event::DocumentChanged { revision: self.revision, origin, label: label.into(), ids: vec![] });
    }

    /// Revert everything a given agent did since the last human change
    /// ("throw away the agent's work").
    pub fn revert_agent_session(&mut self, name: &str) -> usize {
        let agent = Origin::agent(name);
        let mut n = 0;
        while self.undo.last().is_some_and(|h| h.origin == agent) {
            let _ = self.undo(Origin::Human);
            n += 1;
        }
        n
    }

    fn element_mut(&mut self, id: &str) -> ZResult<&mut Element> {
        if !self.doc.elements.contains_key(id) {
            return Err(ZError::not_found(id, self.doc.elements.keys().cloned()));
        }
        Ok(self.doc.elements.get_mut(id).unwrap())
    }

    fn check_ids(&self, ids: &[ElementId]) -> ZResult<()> {
        if ids.is_empty() {
            return Err(ZError::invalid("ids is empty"));
        }
        for id in ids {
            if !self.doc.elements.contains_key(id) {
                return Err(ZError::not_found(id, self.doc.elements.keys().cloned()));
            }
        }
        Ok(())
    }

    /// ids plus, for frames, everything inside them (when `with_contents`).
    pub fn expand(&self, ids: &[ElementId], with_contents: bool) -> Vec<ElementId> {
        let mut seen = BTreeSet::new();
        let mut out = vec![];
        for id in ids {
            if seen.insert(id.clone()) {
                out.push(id.clone());
            }
            if with_contents && self.doc.get(id).is_some_and(|e| e.is_frame()) {
                for c in self.doc.frame_contents(id) {
                    if seen.insert(c.clone()) {
                        out.push(c);
                    }
                }
            }
        }
        out
    }

    fn insert_element(&mut self, mut e: Element, out: &mut Outcome) -> ZResult<ElementId> {
        let id = match e.id.trim() {
            "" => self.doc.alloc_id(e.kind.id_prefix()),
            given => {
                if self.doc.elements.contains_key(given) {
                    return Err(ZError::new("conflict", format!("id '{given}' already exists")).hint("omit `id` to get a fresh one"));
                }
                given.to_string()
            }
        };
        e.id = id.clone();
        validate_element(&e, &self.doc)?;
        measure_text(&*self.measure, &self.theme(), &mut e);
        // Frames go behind the lowest element they enclose, so drawing a frame
        // around content keeps the content visible (Prezi behaviour).
        let pos = if e.is_frame() {
            let b = e.bounds();
            self.doc
                .z_order
                .iter()
                .position(|o| self.doc.get(o).is_some_and(|x| b.contains_rect(&x.bounds()) && x.bounds().area() < b.area()))
                .unwrap_or(self.doc.z_order.len())
        } else {
            self.doc.z_order.len()
        };
        self.doc.z_order.insert(pos, id.clone());
        self.doc.elements.insert(id.clone(), e);
        out.created.push(id.clone());
        Ok(id)
    }

    fn apply_inner(&mut self, cmd: Command, out: &mut Outcome) -> ZResult<()> {
        let theme = self.theme();
        match cmd {
            Command::AddElement { element: n } => {
                let (dw, dh) = default_size(&n.kind);
                let e = Element {
                    id: n.id.unwrap_or_default(),
                    transform: Transform { x: n.x, y: n.y, scale: n.scale, rotation: n.rotation },
                    size: Size { w: n.w.unwrap_or(dw), h: n.h.unwrap_or(dh) },
                    locked: false,
                    name: n.name,
                    kind: n.kind,
                };
                let e = if let ElementKind::Text(mut t) = e.kind.clone() {
                    t.fixed_width = t.fixed_width || n.w.is_some();
                    Element { kind: ElementKind::Text(t), ..e }
                } else {
                    e
                };
                self.insert_element(e, out)?;
            }
            Command::AddText { text, x, y, style, scale, place, id } => {
                let mut e = Element {
                    id: id.unwrap_or_default(),
                    transform: Transform { x, y, scale: scale.unwrap_or(1.0), rotation: 0.0 },
                    size: Size { w: 0.0, h: 0.0 },
                    locked: false,
                    name: String::new(),
                    kind: ElementKind::Text(TextData { text, style, ..Default::default() }),
                };
                measure_text(&*self.measure, &theme, &mut e);
                if let Some(p) = place {
                    self.place(&mut e, &p, scale.is_none())?;
                }
                self.insert_element(e, out)?;
            }
            Command::AddFrame { style, x, y, w, h, rotation, name, id, add_to_path } => {
                let (w, h) = if style == FrameStyle::Circle { (w.max(h), w.max(h)) } else { (w, h) };
                let e = Element {
                    id: id.unwrap_or_default(),
                    transform: Transform { x, y, scale: 1.0, rotation },
                    size: Size { w, h },
                    locked: false,
                    name: name.unwrap_or_default(),
                    kind: ElementKind::Frame(FrameData { style, color: None }),
                };
                let id = self.insert_element(e, out)?;
                if add_to_path {
                    self.doc.path.push(PathStep { target: StepTarget::Element { id }, builds: vec![], notes: String::new(), duration: None, voiceover: None });
                }
            }
            Command::AddShape { shape, x, y, w, h, rotation, fill, stroke, id } => {
                let (fill, stroke) = match (fill, stroke) {
                    (None, None) => match shape {
                        ShapeKind::Line | ShapeKind::Arrow => (None, Some("theme:shape1".to_string())),
                        _ => (Some("theme:shape1".to_string()), None),
                    },
                    fs => fs,
                };
                let e = Element {
                    id: id.unwrap_or_default(),
                    transform: Transform { x, y, scale: 1.0, rotation },
                    size: Size { w, h },
                    locked: false,
                    name: String::new(),
                    kind: ElementKind::Shape(ShapeData { shape, fill, stroke, stroke_width: 4.0 }),
                };
                self.insert_element(e, out)?;
            }
            Command::AddImage { data_base64, x, y, width, alt, id } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data_base64.trim())
                    .map_err(|e| ZError::invalid(format!("data_base64 is not valid base64: {e}")))?;
                let key = self.add_asset(bytes)?;
                let a = &self.doc.assets[&key];
                let (pw, ph) = (a.width as f64, a.height as f64);
                let scale = width.map(|w| w / pw).unwrap_or(1.0);
                let e = Element {
                    id: id.unwrap_or_default(),
                    transform: Transform { x, y, scale, rotation: 0.0 },
                    size: Size { w: pw, h: ph },
                    locked: false,
                    name: String::new(),
                    kind: ElementKind::Image(ImageData { asset: key, alt, crop: None, mask: ImageMask::Rectangle }),
                };
                self.insert_element(e, out)?;
            }
            Command::AddVideo { data_base64, path, x, y, width, alt, poster_base64, autoplay, looping, muted, id } => {
                let (key, info) = self.load_media(&data_base64, path.as_deref(), MediaKind::Video)?;
                let poster = match poster_base64.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
                    Some(p) => Some(self.load_media(p, None, MediaKind::Image)?.0),
                    None => None,
                };
                // Size from the file; else from the poster; else 16:9.
                let (pw, ph) = match (info.width, info.height) {
                    (w, h) if w > 0 && h > 0 => (w as f64, h as f64),
                    _ => poster
                        .as_ref()
                        .and_then(|p| self.doc.assets.get(p))
                        .filter(|a| a.width > 0 && a.height > 0)
                        .map(|a| (a.width as f64, a.height as f64))
                        .unwrap_or((640.0, 360.0)),
                };
                let scale = width.map(|w| w / pw).unwrap_or(1.0);
                let e = Element {
                    id: id.unwrap_or_default(),
                    transform: Transform { x, y, scale, rotation: 0.0 },
                    size: Size { w: pw, h: ph },
                    locked: false,
                    name: String::new(),
                    kind: ElementKind::Video(VideoData {
                        asset: key,
                        poster,
                        autoplay,
                        looping,
                        muted,
                        start: 0.0,
                        end: None,
                        duration: info.duration,
                        alt,
                    }),
                };
                self.insert_element(e, out)?;
            }
            Command::ReplaceImage { id, data_base64, path } => {
                self.check_ids(std::slice::from_ref(&id))?;
                self.check_unlocked(std::slice::from_ref(&id))?;
                let (key, info) = self.load_media(&data_base64, path.as_deref(), MediaKind::Image)?;
                let e = self.element_mut(&id)?;
                let ElementKind::Image(img) = &mut e.kind else {
                    return Err(ZError::invalid(format!("'{id}' is a {}, not an image", e.kind.type_name())));
                };
                img.asset = key;
                img.crop = None;
                // Keep the width; the height follows the new picture.
                let (nw, nh) = (info.width.max(1) as f64, info.height.max(1) as f64);
                let world_w = e.size.w * e.transform.scale;
                e.size = Size { w: nw, h: nh };
                e.transform.scale = world_w / nw;
                out.changed.push(id);
            }
            Command::SetPoster { id, data_base64, path } => {
                let poster = if data_base64.trim().is_empty() && path.is_none() {
                    None
                } else {
                    Some(self.load_media(&data_base64, path.as_deref(), MediaKind::Image)?.0)
                };
                let e = self.element_mut(&id)?;
                let ElementKind::Video(v) = &mut e.kind else {
                    return Err(ZError::invalid(format!("'{id}' is a {}, not a video", e.kind.type_name())));
                };
                v.poster = poster;
                out.changed.push(id);
            }
            Command::SetSoundtrack { data_base64, path, looping, volume, name } => {
                let new_asset = if data_base64.trim().is_empty() && path.is_none() {
                    None
                } else {
                    Some(self.load_media(&data_base64, path.as_deref(), MediaKind::Audio)?.0)
                };
                let file_name = path.as_deref().and_then(|p| std::path::Path::new(p).file_stem()).map(|s| s.to_string_lossy().to_string());
                let t = match (new_asset, self.doc.soundtrack.take()) {
                    (Some(asset), old) => Soundtrack {
                        asset,
                        looping: old.as_ref().map(|o| o.looping).unwrap_or(true),
                        volume: old.as_ref().map(|o| o.volume).unwrap_or(0.6),
                        name: file_name.unwrap_or_default(),
                    },
                    (None, Some(old)) => old,
                    (None, None) => {
                        return Err(ZError::invalid("there is no soundtrack yet").hint("give data_base64 or path"));
                    }
                };
                let mut t = t;
                if let Some(l) = looping {
                    t.looping = l;
                }
                if let Some(v) = volume {
                    if !v.is_finite() {
                        return Err(ZError::invalid("volume must be a number from 0 to 1"));
                    }
                    t.volume = v.clamp(0.0, 1.0);
                }
                if let Some(n) = name {
                    t.name = n;
                }
                self.doc.soundtrack = Some(t);
            }
            Command::RemoveSoundtrack => {
                self.doc.soundtrack = None;
            }
            Command::SetVoiceover { index, data_base64, path } => {
                self.check_step(index)?;
                let v = if data_base64.trim().is_empty() && path.is_none() {
                    None
                } else {
                    Some(self.load_media(&data_base64, path.as_deref(), MediaKind::Audio)?.0)
                };
                self.doc.path[index].voiceover = v;
            }
            Command::AddInk { points, color, width, highlighter } => {
                if points.len() < 2 {
                    return Err(ZError::invalid("ink needs at least 2 points"));
                }
                let pts: Vec<Vec2> = points.iter().map(|p| Vec2::new(p[0], p[1])).collect();
                let b = Rect::from_points(pts.iter().copied()).unwrap();
                let c = b.center();
                let e = Element {
                    id: String::new(),
                    transform: Transform::at(c.x, c.y),
                    size: Size { w: b.width().max(width), h: b.height().max(width) },
                    locked: false,
                    name: String::new(),
                    kind: ElementKind::Ink(InkData { points: pts.iter().map(|p| *p - c).collect(), color, width, highlighter }),
                };
                self.insert_element(e, out)?;
            }
            Command::Update { id, patch } => {
                self.element_mut(&id)?;
                // Changing a text's style (e.g. Body -> Title) must not push it
                // out of the frame it sits in: remember the frame first.
                let keep_in = if patch.style.is_some() && patch.scale.is_none() { self.doc.parent_frame(&id) } else { None };
                let frame_box = keep_in.and_then(|f| self.doc.get(&f)).map(|f| (f.world_size(), f.transform.rotation));
                let crop_px = match (&patch.crop, &self.doc.elements[&id].kind) {
                    (Some(_), ElementKind::Image(i)) => self.doc.assets.get(&i.asset).map(|a| (a.width as f64, a.height as f64)),
                    _ => None,
                };
                let e = self.doc.elements.get_mut(&id).unwrap();
                apply_patch(e, &patch)?;
                if let (Some(c), Some(px)) = (patch.crop, crop_px) {
                    apply_crop(e, c, px);
                }
                measure_text(&*self.measure, &theme, e);
                if let Some(((fw, fh), frot)) = frame_box {
                    // Fit within 90% of the frame, measured in the frame's own axes
                    // when the text shares its rotation (the usual case).
                    let (w, h) = e.world_size();
                    let k = if (normalize_deg(e.transform.rotation - frot)).abs() < 1.0 {
                        (fw * 0.9 / w).min(fh * 0.9 / h)
                    } else {
                        fw.min(fh) * 0.9 / w.hypot(h)
                    };
                    if k < 1.0 {
                        e.transform.scale *= k;
                    }
                }
                out.changed.push(id);
            }
            Command::Move { ids, dx, dy, with_contents } => {
                self.check_ids(&ids)?;
                self.check_unlocked(&ids)?;
                for id in self.expand(&ids, with_contents) {
                    let e = self.doc.elements.get_mut(&id).unwrap();
                    e.transform.x += dx;
                    e.transform.y += dy;
                    out.changed.push(id);
                }
            }
            Command::Scale { ids, factor, about, with_contents } => {
                self.check_ids(&ids)?;
                self.check_unlocked(&ids)?;
                if !(factor.is_finite() && factor > 0.0) {
                    return Err(ZError::invalid("factor must be > 0"));
                }
                let about = about.map(|a| Vec2::new(a[0], a[1])).unwrap_or_else(|| self.bounds_of(&ids).center());
                for id in self.expand(&ids, with_contents) {
                    let e = self.doc.elements.get_mut(&id).unwrap();
                    let p = about + (e.transform.center() - about) * factor;
                    e.transform.x = p.x;
                    e.transform.y = p.y;
                    e.transform.scale *= factor;
                    out.changed.push(id);
                }
            }
            Command::Rotate { ids, degrees, about, with_contents } => {
                self.check_ids(&ids)?;
                self.check_unlocked(&ids)?;
                let about = about.map(|a| Vec2::new(a[0], a[1])).unwrap_or_else(|| self.bounds_of(&ids).center());
                for id in self.expand(&ids, with_contents) {
                    let e = self.doc.elements.get_mut(&id).unwrap();
                    let p = about + (e.transform.center() - about).rotate_deg(degrees);
                    e.transform.x = p.x;
                    e.transform.y = p.y;
                    e.transform.rotation = normalize_deg(e.transform.rotation + degrees);
                    out.changed.push(id);
                }
            }
            Command::Delete { ids } => {
                self.check_ids(&ids)?;
                self.check_unlocked(&ids)?;
                let set: HashSet<&ElementId> = ids.iter().collect();
                self.doc.z_order.retain(|z| !set.contains(z));
                for id in &ids {
                    self.doc.elements.remove(id);
                }
                self.doc.path.retain(|s| !matches!(&s.target, StepTarget::Element { id } if set.contains(id)));
                for s in &mut self.doc.path {
                    s.builds.retain(|b| !set.contains(b));
                }
                self.gc_assets();
                out.deleted.extend(ids);
            }
            Command::Duplicate { ids, dx, dy } => {
                self.check_ids(&ids)?;
                let all = self.expand(&ids, true);
                let ordered: Vec<ElementId> = self.doc.z_order.iter().filter(|z| all.contains(z)).cloned().collect();
                for old in ordered {
                    let mut e = self.doc.elements[&old].clone();
                    e.id = String::new();
                    e.transform.x += dx;
                    e.transform.y += dy;
                    e.locked = false;
                    let nid = self.doc.alloc_id(e.kind.id_prefix());
                    e.id = nid.clone();
                    self.doc.z_order.push(nid.clone());
                    self.doc.elements.insert(nid.clone(), e);
                    out.created.push(nid);
                }
            }
            Command::ZOrder { ids, order } => {
                self.check_ids(&ids)?;
                let z = &mut self.doc.z_order;
                match order {
                    ZOrderOp::Front => {
                        z.retain(|x| !ids.contains(x));
                        z.extend(ids.iter().cloned());
                    }
                    ZOrderOp::Back => {
                        z.retain(|x| !ids.contains(x));
                        let mut v = ids.clone();
                        v.extend(z.drain(..));
                        *z = v;
                    }
                    ZOrderOp::Forward => {
                        for i in (0..z.len().saturating_sub(1)).rev() {
                            if ids.contains(&z[i]) && !ids.contains(&z[i + 1]) {
                                z.swap(i, i + 1);
                            }
                        }
                    }
                    ZOrderOp::Backward => {
                        for i in 1..z.len() {
                            if ids.contains(&z[i]) && !ids.contains(&z[i - 1]) {
                                z.swap(i, i - 1);
                            }
                        }
                    }
                }
                out.changed.extend(ids);
            }
            Command::WrapInFrame { ids, style, padding, add_to_path } => {
                self.check_ids(&ids)?;
                let b = self.bounds_of(&ids);
                let pad = b.width().max(b.height()) * padding.max(0.0);
                let r = b.inflate(pad);
                let c = r.center();
                let (w, h) = if style == FrameStyle::Circle {
                    let d = r.width().hypot(r.height());
                    (d, d)
                } else {
                    (r.width(), r.height())
                };
                self.apply_inner(
                    Command::AddFrame { style, x: c.x, y: c.y, w, h, rotation: 0.0, name: None, id: None, add_to_path },
                    out,
                )?;
            }
            Command::Arrange { ids, layout, gap } => {
                self.check_ids(&ids)?;
                self.arrange(&ids, layout, gap, out);
            }
            Command::Align { ids, edge } => {
                self.check_ids(&ids)?;
                let all = self.bounds_of(&ids);
                for id in &ids {
                    let b = self.doc.elements[id].bounds();
                    let (dx, dy) = match edge {
                        AlignEdge::Left => (all.x0 - b.x0, 0.0),
                        AlignEdge::Right => (all.x1 - b.x1, 0.0),
                        AlignEdge::Top => (0.0, all.y0 - b.y0),
                        AlignEdge::Bottom => (0.0, all.y1 - b.y1),
                        AlignEdge::CenterX => (all.center().x - b.center().x, 0.0),
                        AlignEdge::CenterY => (0.0, all.center().y - b.center().y),
                    };
                    self.apply_inner(Command::Move { ids: vec![id.clone()], dx, dy, with_contents: true }, out)?;
                }
            }
            Command::AddStep { target, index, builds, notes } => {
                self.validate_target(&target)?;
                for b in &builds {
                    self.check_ids(std::slice::from_ref(b))?;
                }
                let i = index.unwrap_or(self.doc.path.len());
                if i > self.doc.path.len() {
                    return Err(ZError::invalid(format!("index {i} > path length {}", self.doc.path.len())));
                }
                self.doc.path.insert(i, PathStep { target, builds, notes, duration: None, voiceover: None });
            }
            Command::RemoveStep { index } => {
                self.check_step(index)?;
                self.doc.path.remove(index);
            }
            Command::MoveStep { from, to } => {
                self.check_step(from)?;
                self.check_step(to)?;
                let s = self.doc.path.remove(from);
                self.doc.path.insert(to, s);
            }
            Command::UpdateStep { index, target, builds, notes, duration } => {
                self.check_step(index)?;
                if let Some(t) = &target {
                    self.validate_target(t)?;
                }
                if let Some(b) = &builds {
                    for id in b {
                        self.check_ids(std::slice::from_ref(id))?;
                    }
                }
                let s = &mut self.doc.path[index];
                if let Some(t) = target {
                    s.target = t;
                }
                if let Some(b) = builds {
                    s.builds = b;
                }
                if let Some(n) = notes {
                    s.notes = n;
                }
                if let Some(d) = duration {
                    s.duration = if d < 0.0 { None } else { Some(d) };
                }
            }
            Command::ClearPath => self.doc.path.clear(),
            Command::AutoPath => {
                let mut frames: Vec<&Element> = self.doc.ordered().filter(|e| e.is_frame()).collect();
                if frames.is_empty() {
                    return Err(ZError::invalid("there are no frames to build a path from").hint("add frames first (add_frame / wrap_in_frame)"));
                }
                let band = {
                    let mut hs: Vec<f64> = frames.iter().map(|f| f.bounds().height()).collect();
                    hs.sort_by(f64::total_cmp);
                    hs[hs.len() / 2].max(1e-9)
                };
                frames.sort_by(|a, b| {
                    let (ca, cb) = (a.bounds().center(), b.bounds().center());
                    ((ca.y / band).floor()).total_cmp(&(cb.y / band).floor()).then(ca.x.total_cmp(&cb.x))
                });
                self.doc.path = frames
                    .iter()
                    .map(|f| PathStep { target: StepTarget::Element { id: f.id.clone() }, builds: vec![], notes: String::new(), duration: None, voiceover: None })
                    .collect();
            }
            Command::SetTheme { theme } => {
                if Theme::by_id(&theme).is_none() {
                    return Err(ZError::invalid(format!("unknown theme '{theme}'")).values(Theme::ids()));
                }
                self.doc.theme = theme;
                let mut d = std::mem::take(&mut self.doc);
                self.remeasure_all(&mut d);
                self.doc = d;
            }
            Command::SetTitle { title } => self.doc.title = title,
        }
        Ok(())
    }

    fn check_unlocked(&self, ids: &[ElementId]) -> ZResult<()> {
        for id in ids {
            if self.doc.elements[id].locked {
                return Err(ZError::new("locked", format!("'{id}' is locked")).hint("unlock it first: update {patch:{locked:false}}"));
            }
        }
        Ok(())
    }

    fn check_step(&self, i: usize) -> ZResult<()> {
        if i >= self.doc.path.len() {
            return Err(ZError::invalid(format!("step {i} out of range (path has {} steps, 0-based)", self.doc.path.len())));
        }
        Ok(())
    }

    fn validate_target(&self, t: &StepTarget) -> ZResult<()> {
        match t {
            StepTarget::Element { id } => self.check_ids(std::slice::from_ref(id)),
            StepTarget::View { view } => {
                if view.width > 0.0 && view.height > 0.0 {
                    Ok(())
                } else {
                    Err(ZError::invalid("view width/height must be > 0"))
                }
            }
            StepTarget::Overview => Ok(()),
        }
    }

    pub fn bounds_of(&self, ids: &[ElementId]) -> Rect {
        ids.iter()
            .filter_map(|id| self.doc.get(id))
            .map(|e| e.bounds())
            .reduce(|a, b| a.union(&b))
            .unwrap_or(Rect { x0: 0.0, y0: 0.0, x1: 0.0, y1: 0.0 })
    }

    fn place(&self, e: &mut Element, p: &Placement, auto_scale: bool) -> ZResult<()> {
        let r = self
            .doc
            .get(&p.relative_to)
            .ok_or_else(|| ZError::not_found(&p.relative_to, self.doc.elements.keys().cloned()))?;
        let rb = r.bounds();
        if p.side == Side::Inside {
            if auto_scale {
                let frac = match &e.kind {
                    ElementKind::Text(t) => match t.style {
                        TextStyle::Title => 0.12,
                        TextStyle::Subtitle => 0.08,
                        TextStyle::Body => 0.055,
                    },
                    _ => 0.3,
                };
                let theme = self.theme();
                let fs = match &e.kind {
                    ElementKind::Text(t) => theme.text_style(t.style).size,
                    _ => e.size.h,
                };
                let (rw, rh) = r.world_size();
                let mut s = rh * frac / fs;
                // keep inside 85% of the frame width
                if e.size.w * s > rw * 0.85 {
                    s = rw * 0.85 / e.size.w;
                }
                e.transform.scale = s;
            }
            e.transform.rotation = r.transform.rotation;
            e.transform.x = r.transform.x;
            e.transform.y = r.transform.y;
            return Ok(());
        }
        if auto_scale && !r.is_frame() {
            e.transform.scale = r.transform.scale;
        }
        let (w, h) = (e.size.w * e.transform.scale, e.size.h * e.transform.scale);
        let gap = p.gap.unwrap_or(rb.width().max(rb.height()) * 0.1);
        let c = rb.center();
        let (x, y) = match p.side {
            Side::Below => (c.x, rb.y1 + gap + h / 2.0),
            Side::Above => (c.x, rb.y0 - gap - h / 2.0),
            Side::Left => (rb.x0 - gap - w / 2.0, c.y),
            Side::Right => (rb.x1 + gap + w / 2.0, c.y),
            Side::Inside => unreachable!(),
        };
        e.transform.x = x;
        e.transform.y = y;
        Ok(())
    }

    fn arrange(&mut self, ids: &[ElementId], layout: Layout, gap: Option<f64>, out: &mut Outcome) {
        let n = ids.len();
        let sizes: Vec<(f64, f64)> = ids.iter().map(|id| {
            let b = self.doc.elements[id].bounds();
            (b.width(), b.height())
        }).collect();
        let avg = sizes.iter().map(|(w, h)| w.max(*h)).sum::<f64>() / n as f64;
        let gap = gap.unwrap_or(avg * 0.2);
        let all = self.bounds_of(ids);
        let c = all.center();
        let mut targets: Vec<Vec2> = vec![];
        match layout {
            Layout::Row => {
                let total: f64 = sizes.iter().map(|s| s.0).sum::<f64>() + gap * (n as f64 - 1.0);
                let mut x = c.x - total / 2.0;
                for (w, _) in &sizes {
                    targets.push(Vec2::new(x + w / 2.0, c.y));
                    x += w + gap;
                }
            }
            Layout::Column => {
                let total: f64 = sizes.iter().map(|s| s.1).sum::<f64>() + gap * (n as f64 - 1.0);
                let mut y = c.y - total / 2.0;
                for (_, h) in &sizes {
                    targets.push(Vec2::new(c.x, y + h / 2.0));
                    y += h + gap;
                }
            }
            Layout::Grid => {
                let cols = (n as f64).sqrt().ceil() as usize;
                let rows = n.div_ceil(cols);
                let cw = sizes.iter().map(|s| s.0).fold(0.0, f64::max) + gap;
                let ch = sizes.iter().map(|s| s.1).fold(0.0, f64::max) + gap;
                for i in 0..n {
                    let (r, k) = (i / cols, i % cols);
                    targets.push(Vec2::new(
                        c.x + (k as f64 - (cols as f64 - 1.0) / 2.0) * cw,
                        c.y + (r as f64 - (rows as f64 - 1.0) / 2.0) * ch,
                    ));
                }
            }
            Layout::Circle => {
                let maxs = sizes.iter().map(|s| s.0.max(s.1)).fold(0.0, f64::max);
                let radius = ((maxs + gap) * n as f64 / std::f64::consts::TAU).max(maxs);
                for i in 0..n {
                    let a = std::f64::consts::TAU * i as f64 / n as f64 - std::f64::consts::FRAC_PI_2;
                    targets.push(Vec2::new(c.x + radius * a.cos(), c.y + radius * a.sin()));
                }
            }
        }
        for (id, t) in ids.iter().zip(targets) {
            let b = self.doc.elements[id].bounds().center();
            let d = t - b;
            for m in self.expand(std::slice::from_ref(id), true) {
                let e = self.doc.elements.get_mut(&m).unwrap();
                e.transform.x += d.x;
                e.transform.y += d.y;
                out.changed.push(m);
            }
        }
    }

    /// Load media for a command from base64 or a file path, checking it is
    /// the wanted kind. Video and audio files are copied into the media
    /// cache and memory-mapped, so they never sit in the heap.
    fn load_media(&mut self, data_base64: &str, path: Option<&str>, want: MediaKind) -> ZResult<(String, crate::media::MediaInfo)> {
        let what = match want {
            MediaKind::Image => "an image",
            MediaKind::Video => "a video",
            MediaKind::Audio => "an audio file",
        };
        let check = |info: &crate::media::MediaInfo| -> ZResult<()> {
            if info.kind != want {
                return Err(ZError::invalid(format!("this file is {}, not {what}", match info.kind {
                    MediaKind::Image => "an image",
                    MediaKind::Video => "a video",
                    MediaKind::Audio => "audio",
                })));
            }
            Ok(())
        };
        if let Some(p) = path.filter(|p| !p.trim().is_empty()) {
            let p = std::path::Path::new(p);
            let ioe = |e: std::io::Error| ZError::new("io", format!("cannot read '{}': {e}", p.display()));
            let f = std::fs::File::open(p).map_err(ioe)?;
            // SAFETY: read-only and short-lived: used to sniff and hash, then
            // dropped. The asset maps the cache copy, not this file.
            let map = unsafe { memmap2::Mmap::map(&f) }.map_err(ioe)?;
            let info = crate::media::info(&map)?;
            check(&info)?;
            let key = content_key(&map);
            if !self.doc.assets.contains_key(&key) {
                let bytes: AssetBytes = if want == MediaKind::Image {
                    map.to_vec().into()
                } else {
                    let m = crate::media::import_file(p, &key, &info.ext).map_err(ioe)?;
                    let len = m.len();
                    AssetBytes::Mapped { map: std::sync::Arc::new(m), start: 0, len }
                };
                self.doc.assets.insert(key.clone(), Asset { ext: info.ext.clone(), bytes, width: info.width, height: info.height });
            }
            return Ok((key, info));
        }
        if data_base64.trim().is_empty() {
            return Err(ZError::invalid(format!("give {what} as data_base64 or path")));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data_base64.trim())
            .map_err(|e| ZError::invalid(format!("data_base64 is not valid base64: {e}")))?;
        let info = crate::media::info(&bytes)?;
        check(&info)?;
        let key = content_key(&bytes);
        self.doc
            .assets
            .entry(key.clone())
            .or_insert(Asset { ext: info.ext.clone(), bytes: bytes.into(), width: info.width, height: info.height });
        Ok((key, info))
    }

    /// Store image bytes as an asset; returns the key.
    pub fn add_asset(&mut self, bytes: Vec<u8>) -> ZResult<String> {
        let (ext, w, h) = image_info(&bytes)?;
        let key = content_key(&bytes);
        self.doc.assets.entry(key.clone()).or_insert(Asset { ext, bytes: bytes.into(), width: w, height: h });
        Ok(key)
    }

    fn gc_assets(&mut self) {
        let used: HashSet<String> = self
            .doc
            .elements
            .values()
            .filter_map(|e| match &e.kind {
                ElementKind::Image(i) => Some(i.asset.clone()),
                _ => None,
            })
            .collect();
        // Keep assets referenced by undo history: only gc unreferenced ones
        // that no history snapshot uses either. Simplest safe choice: keep.
        let _ = used;
    }

    // ------------------------------------------------------------------
    // View commands

    pub fn view(&mut self, cmd: ViewCommand, origin: Origin) -> ZResult<serde_json::Value> {
        let agent_limited = origin.is_agent() && self.view.agent_permission != Permission::Full;
        if origin.is_agent() {
            if let ViewCommand::PauseAgents { .. } | ViewCommand::SetAgentPermission { .. } | ViewCommand::FollowAgent { .. } = cmd {
                return Err(ZError::new("permission", "only the human can change agent controls"));
            }
            if agent_limited
                && matches!(
                    cmd,
                    ViewCommand::Present { .. }
                        | ViewCommand::Next
                        | ViewCommand::Prev
                        | ViewCommand::GotoStep { .. }
                        | ViewCommand::Overview
                        | ViewCommand::Blank { .. }
                        | ViewCommand::ExitPresent
                        | ViewCommand::SetMode { .. }
                        | ViewCommand::SetTool { .. }
                        | ViewCommand::SetFrameStyle { .. }
                        | ViewCommand::SetViewport { .. }
                        | ViewCommand::SetReduceMotion { .. }
                )
            {
                return Err(ZError::new("permission", "this view command needs 'full' agent permission")
                    .hint("the human can grant it from the agent status pill, or launch with --agent-permission full; captures of any step work without it"));
            }
        }
        let vp = self.view.viewport;
        // Camera/selection commands from limited agents act on the agent's
        // private camera/selection, never the human's.
        macro_rules! cam_set {
            ($cam:expr, $animate:expr) => {{
                let c: Camera = $cam;
                if agent_limited {
                    self.view.agent_camera = c;
                } else {
                    self.view.fly_to(c, $animate);
                    if self.view.mode == Mode::Present {
                        self.view.present.off_path = true;
                    }
                }
            }};
        }
        let base_cam = if agent_limited { self.view.agent_camera } else { self.view.target_camera() };
        match cmd {
            ViewCommand::SetCamera { camera, animate } => cam_set!(camera, animate),
            ViewCommand::Zoom { factor, anchor } => {
                if !(factor.is_finite() && factor > 0.0) {
                    return Err(ZError::invalid("factor must be > 0"));
                }
                let mut c = base_cam;
                let a = anchor.map(|a| Vec2::new(a[0], a[1])).unwrap_or(vp * 0.5);
                c.zoom_at(factor, a, vp);
                cam_set!(c, false)
            }
            ViewCommand::Pan { dx, dy } => {
                let mut c = base_cam;
                c.pan_screen(Vec2::new(dx, dy));
                cam_set!(c, false)
            }
            ViewCommand::ZoomTo { id, animate } => {
                let e = self.doc.get(&id).ok_or_else(|| ZError::not_found(&id, self.doc.elements.keys().cloned()))?;
                let c = self.camera_for_element(e);
                cam_set!(c, animate)
            }
            ViewCommand::FitAll { animate } => {
                let c = self.fit_all_camera();
                cam_set!(c, animate)
            }
            ViewCommand::ZoomToSelection => {
                let sel = if agent_limited { &self.view.agent_selection } else { &self.view.selection };
                if sel.is_empty() {
                    return Err(ZError::invalid("nothing selected"));
                }
                let c = Camera::fit_rect(&self.bounds_of(&sel.clone()), vp, 0.1);
                cam_set!(c, true)
            }
            ViewCommand::Select { ids } | ViewCommand::AddToSelection { ids } if ids.iter().any(|i| !self.doc.elements.contains_key(i)) => {
                let bad = ids.iter().find(|i| !self.doc.elements.contains_key(*i)).unwrap();
                return Err(ZError::not_found(bad, self.doc.elements.keys().cloned()));
            }
            ViewCommand::Select { ids } => self.set_selection(ids, agent_limited),
            ViewCommand::AddToSelection { ids } => {
                let mut s = if agent_limited { self.view.agent_selection.clone() } else { self.view.selection.clone() };
                for i in ids {
                    if !s.contains(&i) {
                        s.push(i);
                    }
                }
                self.set_selection(s, agent_limited)
            }
            ViewCommand::ClearSelection => self.set_selection(vec![], agent_limited),
            ViewCommand::SelectAll => {
                let all = self.doc.z_order.iter().filter(|i| !self.doc.elements[*i].locked).cloned().collect();
                self.set_selection(all, agent_limited)
            }
            ViewCommand::SetTool { tool } => self.view.tool = tool,
            ViewCommand::SetFrameStyle { style } => self.view.frame_style = style,
            ViewCommand::SetMode { mode } => match mode {
                Mode::Present => return self.view(ViewCommand::Present { step: None }, origin),
                m => {
                    if self.view.mode == Mode::Present {
                        self.view.present = Default::default();
                    }
                    self.set_mode(m)
                }
            },
            ViewCommand::Present { step } => {
                if let Some(s) = step {
                    self.check_step(s)?;
                }
                self.set_mode(Mode::Present);
                self.view.present = Default::default();
                let s = step.or(if self.doc.path.is_empty() { None } else { Some(0) });
                self.goto_step(s, 0);
            }
            ViewCommand::Next => {
                self.require_present()?;
                self.next();
            }
            ViewCommand::Prev => {
                self.require_present()?;
                self.prev();
            }
            ViewCommand::GotoStep { step } => {
                self.check_step(step)?;
                if self.view.mode != Mode::Present {
                    // In the editor this just flies the camera to the step.
                    let c = self.camera_for_step(step);
                    cam_set!(c, true)
                } else {
                    self.goto_step(Some(step), 0);
                }
            }
            ViewCommand::Overview => {
                self.require_present()?;
                self.goto_step(None, 0);
            }
            ViewCommand::Blank { blank } => {
                self.require_present()?;
                self.view.present.blank = blank;
            }
            ViewCommand::ExitPresent => {
                if self.view.mode == Mode::Present {
                    self.set_mode(Mode::Edit);
                    self.view.present.blank = None;
                }
            }
            ViewCommand::SetViewport { width, height } => {
                if width < 1.0 || height < 1.0 {
                    return Err(ZError::invalid("viewport must be at least 1x1"));
                }
                self.view.viewport = Vec2::new(width, height);
            }
            ViewCommand::SetReduceMotion { on } => self.view.reduce_motion = on,
            ViewCommand::FollowAgent { on } => self.view.follow_agent = on,
            ViewCommand::PauseAgents { paused } => {
                self.view.agent_paused = paused;
                if paused && self.tx.as_ref().is_some_and(|t| t.origin.is_agent()) {
                    self.commit();
                }
                self.events.push(Event::AgentsPaused { paused });
            }
            ViewCommand::SetAgentPermission { permission } => self.view.agent_permission = permission,
        }
        // Follow-agent: mirror the agent camera onto the human view.
        if agent_limited && self.view.follow_agent {
            let c = self.view.agent_camera;
            self.view.fly_to(c, true);
        }
        Ok(json!({"ok": true}))
    }

    fn require_present(&self) -> ZResult<()> {
        if self.view.mode != Mode::Present {
            return Err(ZError::new("not_presenting", "not in present mode").hint("send {\"op\":\"present\"} first"));
        }
        Ok(())
    }

    fn set_mode(&mut self, m: Mode) {
        if self.view.mode != m {
            self.view.mode = m;
            self.events.push(Event::ModeChanged { mode: m });
        }
    }

    fn set_selection(&mut self, ids: Vec<ElementId>, agent: bool) {
        if agent {
            self.view.agent_selection = ids;
        } else if self.view.selection != ids {
            self.view.selection = ids.clone();
            self.events.push(Event::SelectionChanged { selection: ids });
        }
    }

    fn next(&mut self) {
        let p = &self.view.present;
        if p.blank.is_some() {
            self.view.present.blank = None;
            return;
        }
        match p.step {
            None => {
                if !self.doc.path.is_empty() {
                    self.goto_step(Some(0), 0)
                }
            }
            Some(i) => {
                let builds = self.doc.path.get(i).map(|s| s.builds.len()).unwrap_or(0);
                if p.off_path {
                    let b = p.build;
                    self.goto_step(Some(i), b);
                } else if p.build < builds {
                    self.view.present.build += 1;
                    self.events.push(Event::StepChanged { step: Some(i), build: self.view.present.build });
                } else if i + 1 < self.doc.path.len() {
                    self.goto_step(Some(i + 1), 0);
                }
            }
        }
    }

    fn prev(&mut self) {
        let p = self.view.present.clone();
        match p.step {
            None => {}
            Some(i) if p.off_path => self.goto_step(Some(i), p.build),
            Some(i) if p.build > 0 => {
                self.view.present.build -= 1;
                self.events.push(Event::StepChanged { step: Some(i), build: self.view.present.build });
            }
            Some(0) => self.goto_step(None, 0),
            Some(i) => {
                let b = self.doc.path[i - 1].builds.len();
                self.goto_step(Some(i - 1), b);
            }
        }
    }

    fn goto_step(&mut self, step: Option<usize>, build: usize) {
        let cam = match step {
            Some(i) => self.camera_for_step(i),
            None => self.fit_all_camera(),
        };
        let dur = step.and_then(|i| self.doc.path[i].duration);
        self.view.fly_to(cam, true);
        if let (Some(d), Some(f)) = (dur, self.view.flight.as_mut()) {
            f.duration = d;
        }
        self.view.present.step = step;
        self.view.present.build = build;
        self.view.present.off_path = false;
        self.events.push(Event::StepChanged { step, build });
    }

    // ------------------------------------------------------------------
    // Camera helpers

    pub fn camera_for_element(&self, e: &Element) -> Camera {
        let (w, h) = e.world_size();
        let margin = if e.is_frame() { 0.04 } else { 0.12 };
        Camera::fit(
            &ViewRect { cx: e.transform.x, cy: e.transform.y, width: w, height: h, rotation: e.transform.rotation },
            self.view.viewport,
            margin,
        )
    }

    pub fn camera_for_target(&self, t: &StepTarget) -> Camera {
        match t {
            StepTarget::Element { id } => match self.doc.get(id) {
                Some(e) => self.camera_for_element(e),
                None => self.fit_all_camera(),
            },
            StepTarget::View { view } => Camera::fit(view, self.view.viewport, 0.0),
            StepTarget::Overview => self.fit_all_camera(),
        }
    }

    pub fn camera_for_step(&self, i: usize) -> Camera {
        self.doc.path.get(i).map(|s| self.camera_for_target(&s.target)).unwrap_or_else(|| self.fit_all_camera())
    }

    pub fn fit_all_camera(&self) -> Camera {
        match self.doc.bounds() {
            Some(b) => Camera::fit_rect(&b, self.view.viewport, 0.06),
            None => Camera::default(),
        }
    }

    /// Elements hidden right now because their fade-in build hasn't happened.
    /// Only applies in present mode.
    pub fn hidden_builds(&self) -> HashSet<ElementId> {
        let mut hidden = HashSet::new();
        if self.view.mode != Mode::Present {
            return hidden;
        }
        let p = &self.view.present;
        for (i, s) in self.doc.path.iter().enumerate() {
            match p.step {
                Some(cur) if i < cur => {}
                Some(cur) if i == cur => hidden.extend(s.builds.iter().skip(p.build).cloned()),
                _ => hidden.extend(s.builds.iter().cloned()),
            }
        }
        hidden
    }

    /// Hidden elements for a capture of step `i` with `build` builds shown.
    pub fn hidden_builds_at(&self, step: usize, build: usize) -> HashSet<ElementId> {
        let mut hidden = HashSet::new();
        for (i, s) in self.doc.path.iter().enumerate() {
            if i == step {
                hidden.extend(s.builds.iter().skip(build).cloned());
            } else if i > step {
                hidden.extend(s.builds.iter().cloned());
            }
        }
        hidden
    }

    // ------------------------------------------------------------------
    // Queries

    pub fn query(&self, q: Query) -> ZResult<serde_json::Value> {
        crate::describe::query(self, q)
    }
}

fn busy() -> ZError {
    ZError::new("busy", "the human is in the middle of an edit").hint("retry in a moment")
}

pub fn normalize_deg(d: f64) -> f64 {
    let mut d = d % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d <= -180.0 {
        d += 360.0;
    }
    d
}

fn default_size(k: &ElementKind) -> (f64, f64) {
    match k {
        ElementKind::Frame(_) => (800.0, 600.0),
        ElementKind::Text(_) => (0.0, 0.0),
        ElementKind::Shape(_) => (200.0, 200.0),
        ElementKind::Image(_) => (400.0, 300.0),
        ElementKind::Video(_) => (640.0, 360.0),
        ElementKind::Ink(_) => (100.0, 100.0),
    }
}

/// Recompute a text element's box from its content.
pub fn measure_text(m: &dyn TextMeasure, theme: &Theme, e: &mut Element) {
    if let ElementKind::Text(t) = &e.kind {
        if t.fixed_width {
            let (_, h) = m.measure(t, theme, Some(e.size.w));
            e.size.h = h;
        } else {
            let (w, h) = m.measure(t, theme, None);
            e.size = Size { w, h };
        }
    }
}

fn validate_element(e: &Element, doc: &Document) -> ZResult<()> {
    let t = &e.transform;
    if !(t.x.is_finite() && t.y.is_finite() && t.rotation.is_finite()) {
        return Err(ZError::invalid("x, y and rotation must be finite"));
    }
    if !(t.scale.is_finite() && t.scale > 0.0) {
        return Err(ZError::invalid("scale must be > 0"));
    }
    if !(e.size.w >= 0.0 && e.size.h >= 0.0) {
        return Err(ZError::invalid("w and h must be >= 0"));
    }
    let check = |c: &Option<String>| match c {
        Some(c) if !valid_color(c) => Err(ZError::invalid(format!("bad colour '{c}'"))
            .hint("use #rrggbb, #rrggbbaa or a theme slot")
            .values(Theme::COLOR_SLOTS.iter().map(|s| s.to_string()).collect())),
        _ => Ok(()),
    };
    match &e.kind {
        ElementKind::Text(d) => check(&d.color)?,
        ElementKind::Shape(d) => {
            check(&d.fill)?;
            check(&d.stroke)?;
        }
        ElementKind::Frame(d) => check(&d.color)?,
        ElementKind::Ink(d) => check(&Some(d.color.clone()))?,
        ElementKind::Image(i) => {
            if !doc.assets.get(&i.asset).is_some_and(|a| a.media_kind() == MediaKind::Image) {
                return Err(ZError::invalid(format!("unknown image asset '{}'", i.asset)).hint("use add_image"));
            }
        }
        ElementKind::Video(v) => {
            if !doc.assets.get(&v.asset).is_some_and(|a| a.media_kind() == MediaKind::Video) {
                return Err(ZError::invalid(format!("unknown video asset '{}'", v.asset)).hint("use add_video"));
            }
            if let Some(p) = &v.poster {
                if !doc.assets.get(p).is_some_and(|a| a.media_kind() == MediaKind::Image) {
                    return Err(ZError::invalid(format!("unknown poster asset '{p}'")).hint("use set_poster"));
                }
            }
        }
    }
    Ok(())
}

/// Crop an image so the visible content stays put: the box shrinks or
/// grows around the new crop and its centre moves with it.
fn apply_crop(e: &mut Element, c: Crop, (pw, ph): (f64, f64)) {
    let ElementKind::Image(img) = &mut e.kind else { return };
    if pw <= 0.0 || ph <= 0.0 {
        return;
    }
    let old = img.crop.unwrap_or(Crop::FULL);
    let new = c.clamped();
    // Local units per source pixel (the box may have been stretched).
    let kx = e.size.w / (pw * old.w);
    let ky = e.size.h / (ph * old.h);
    let (ocx, ocy) = old.center();
    let (ncx, ncy) = new.center();
    let shift = Vec2::new((ncx - ocx) * pw * kx, (ncy - ocy) * ph * ky);
    let moved = e.transform.apply(shift);
    e.transform.x = moved.x;
    e.transform.y = moved.y;
    e.size = Size { w: pw * new.w * kx, h: ph * new.h * ky };
    img.crop = if new.is_full() { None } else { Some(new) };
}

fn apply_patch(e: &mut Element, p: &ElementPatch) -> ZResult<()> {
    let kind = e.kind.type_name();
    let wrong = |field: &str| ZError::invalid(format!("'{field}' does not apply to a {kind}"));
    let color = |c: &str| -> ZResult<Option<String>> {
        if c.is_empty() {
            Ok(None)
        } else if valid_color(c) {
            Ok(Some(c.to_string()))
        } else {
            Err(ZError::invalid(format!("bad colour '{c}'")).values(Theme::COLOR_SLOTS.iter().map(|s| s.to_string()).collect()))
        }
    };
    if p.locked.is_none() && e.locked {
        return Err(ZError::new("locked", format!("'{}' is locked", e.id)).hint("include \"locked\": false in the patch"));
    }
    if let Some(v) = p.x {
        e.transform.x = v;
    }
    if let Some(v) = p.y {
        e.transform.y = v;
    }
    if let Some(v) = p.scale {
        if !(v > 0.0 && v.is_finite()) {
            return Err(ZError::invalid("scale must be > 0"));
        }
        e.transform.scale = v;
    }
    if let Some(v) = p.rotation {
        e.transform.rotation = normalize_deg(v);
    }
    if let Some(v) = p.w {
        e.size.w = v.max(1.0);
        if let ElementKind::Text(t) = &mut e.kind {
            t.fixed_width = true;
        }
    }
    if let Some(v) = p.h {
        if matches!(e.kind, ElementKind::Text(_)) {
            return Err(ZError::invalid("text height follows its content; set w (wrap width) or scale instead"));
        }
        e.size.h = v.max(1.0);
    }
    if let Some(v) = p.locked {
        e.locked = v;
    }
    if let Some(v) = &p.name {
        e.name = v.clone();
    }
    let is = |k: &str| kind == k;
    if p.text.is_some() || p.style.is_some() || p.bold.is_some() || p.italic.is_some() || p.align.is_some() || p.bullets.is_some() {
        let ElementKind::Text(t) = &mut e.kind else { return Err(wrong("text/style/bold/italic/align/bullets")) };
        if let Some(v) = &p.text {
            t.text = v.clone();
        }
        if let Some(v) = p.style {
            t.style = v;
        }
        if let Some(v) = p.bold {
            t.bold = v;
        }
        if let Some(v) = p.italic {
            t.italic = v;
        }
        if let Some(v) = p.align {
            t.align = v;
        }
        if let Some(v) = p.bullets {
            t.bullets = v;
        }
    }
    if let Some(c) = &p.color {
        let c = color(c)?;
        match &mut e.kind {
            ElementKind::Text(t) => t.color = c,
            ElementKind::Frame(f) => f.color = c,
            ElementKind::Ink(i) => i.color = c.unwrap_or_else(|| "theme:accent".into()),
            ElementKind::Shape(s) => s.fill = c,
            _ => return Err(wrong("color")),
        }
    }
    if p.shape.is_some() || p.fill.is_some() || p.stroke.is_some() || p.stroke_width.is_some() {
        let ElementKind::Shape(s) = &mut e.kind else { return Err(wrong("shape/fill/stroke/stroke_width")) };
        if let Some(v) = p.shape {
            s.shape = v;
        }
        if let Some(v) = &p.fill {
            s.fill = color(v)?;
        }
        if let Some(v) = &p.stroke {
            s.stroke = color(v)?;
        }
        if let Some(v) = p.stroke_width {
            s.stroke_width = v.max(0.0);
        }
    }
    if let Some(v) = p.frame_style {
        let ElementKind::Frame(f) = &mut e.kind else { return Err(wrong("frame_style")) };
        f.style = v;
        if v == FrameStyle::Circle {
            let d = e.size.w.max(e.size.h);
            e.size = Size { w: d, h: d };
        }
    }
    if let Some(v) = &p.alt {
        match &mut e.kind {
            ElementKind::Image(i) => i.alt = v.clone(),
            ElementKind::Video(d) => d.alt = v.clone(),
            _ => return Err(wrong("alt")),
        }
    }
    if p.crop.is_some() && !is("image") {
        return Err(wrong("crop"));
    }
    if let Some(m) = p.mask {
        let ElementKind::Image(i) = &mut e.kind else { return Err(wrong("mask")) };
        i.mask = m;
    }
    if p.autoplay.is_some() || p.looping.is_some() || p.muted.is_some() || p.start.is_some() || p.end.is_some() {
        let ElementKind::Video(v) = &mut e.kind else { return Err(wrong("autoplay/loop/muted/start/end")) };
        if let Some(b) = p.autoplay {
            v.autoplay = b;
        }
        if let Some(b) = p.looping {
            v.looping = b;
        }
        if let Some(b) = p.muted {
            v.muted = b;
        }
        if let Some(t) = p.start {
            if !t.is_finite() {
                return Err(ZError::invalid("start must be a number of seconds"));
            }
            v.start = t.max(0.0);
        }
        if let Some(t) = p.end {
            if !t.is_finite() {
                return Err(ZError::invalid("end must be a number of seconds"));
            }
            v.end = (t >= 0.0).then_some(t);
        }
        if let Some(end) = v.end {
            if end <= v.start {
                return Err(ZError::invalid(format!("end ({end}s) must be after start ({}s)", v.start)));
            }
        }
        if v.duration > 0.0 && v.start >= v.duration {
            return Err(ZError::invalid(format!("start ({}s) is past the end of the video ({}s)", v.start, v.duration)));
        }
    }
    Ok(())
}

pub fn command_label(c: &Command) -> String {
    match c {
        Command::AddElement { element } => format!("Add {}", element.kind.type_name()),
        Command::AddText { text, .. } => format!("Add text \"{}\"", truncate(text.lines().next().unwrap_or(""), 24)),
        Command::AddFrame { .. } => "Add frame".into(),
        Command::AddShape { shape, .. } => format!("Add {}", serde_json::to_value(shape).unwrap().as_str().unwrap_or("shape")),
        Command::AddImage { .. } => "Insert image".into(),
        Command::AddVideo { .. } => "Insert video".into(),
        Command::ReplaceImage { .. } => "Replace image".into(),
        Command::SetPoster { .. } => "Set poster frame".into(),
        Command::SetSoundtrack { data_base64, path, .. } if !data_base64.is_empty() || path.is_some() => "Add soundtrack".into(),
        Command::SetSoundtrack { .. } => "Change soundtrack".into(),
        Command::RemoveSoundtrack => "Remove soundtrack".into(),
        Command::SetVoiceover { data_base64, path, .. } if data_base64.is_empty() && path.is_none() => "Remove voice-over".into(),
        Command::SetVoiceover { .. } => "Add voice-over".into(),
        Command::Update { patch, .. } if patch.crop.is_some() => "Crop".into(),
        Command::AddInk { highlighter, .. } => if *highlighter { "Highlight" } else { "Draw" }.into(),
        Command::Update { patch, .. } if patch.text.is_some() => "Edit text".into(),
        Command::Update { .. } => "Change properties".into(),
        Command::Move { .. } => "Move".into(),
        Command::Scale { .. } => "Resize".into(),
        Command::Rotate { .. } => "Rotate".into(),
        Command::Delete { ids } => format!("Delete {}", plural(ids.len(), "object")),
        Command::Duplicate { .. } => "Duplicate".into(),
        Command::ZOrder { .. } => "Change order".into(),
        Command::WrapInFrame { .. } => "Frame selection".into(),
        Command::Arrange { .. } => "Arrange".into(),
        Command::Align { .. } => "Align".into(),
        Command::AddStep { .. } => "Add path step".into(),
        Command::RemoveStep { .. } => "Remove path step".into(),
        Command::MoveStep { .. } => "Reorder path".into(),
        Command::UpdateStep { .. } => "Edit path step".into(),
        Command::ClearPath => "Clear path".into(),
        Command::AutoPath => "Path from frames".into(),
        Command::SetTheme { theme } => format!("Theme: {theme}"),
        Command::SetTitle { .. } => "Rename".into(),
    }
}

fn plural(n: usize, w: &str) -> String {
    if n == 1 { format!("1 {w}") } else { format!("{n} {w}s") }
}

fn content_key(bytes: &[u8]) -> String {
    // FNV-1a 64 + length: stable across runs and platforms.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}{:x}", bytes.len())
}

/// (ext, width, height) of an image's bytes.
pub fn image_info(bytes: &[u8]) -> ZResult<(String, u32, u32)> {
    if bytes.len() > 4 && (bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml")) {
        return Err(ZError::invalid("SVG import is not supported yet").hint("convert to PNG"));
    }
    let fmt = image::guess_format(bytes).map_err(|_| ZError::invalid("unrecognised image format").hint("use PNG, JPEG, WebP or GIF"))?;
    let ext = match fmt {
        image::ImageFormat::Png => "png",
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::WebP => "webp",
        image::ImageFormat::Gif => "gif",
        _ => return Err(ZError::invalid("unsupported image format").hint("use PNG, JPEG, WebP or GIF")),
    };
    let (w, h) = image::ImageReader::with_format(std::io::Cursor::new(bytes), fmt)
        .into_dimensions()
        .map_err(|e| ZError::invalid(format!("cannot read image: {e}")))?;
    Ok((ext.into(), w, h))
}

/// Apply `(patch, inverse)` pairs in order. If one fails, roll back the ones
/// already applied so `doc` is unchanged.
fn replay<'a>(doc: &mut Document, steps: impl Iterator<Item = (&'a json_patch::Patch, &'a json_patch::Patch)>) -> Result<(), String> {
    let mut done: Vec<&json_patch::Patch> = vec![];
    for (p, inv) in steps {
        if let Err(e) = crate::delta::apply(doc, p) {
            for inv in done.into_iter().rev() {
                let _ = crate::delta::apply(doc, inv);
            }
            return Err(e);
        }
        done.push(inv);
    }
    Ok(())
}
