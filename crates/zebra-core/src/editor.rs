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
    /// Font files (bytes, extension) of an installed family, so a theme
    /// using it can embed it in the document. Empty for bundled families
    /// and when unknown.
    fn font_files(&self, _family: &str) -> Vec<(Vec<u8>, String)> {
        vec![]
    }
    /// Installed font families, sorted.
    fn font_families(&self) -> Vec<String> {
        vec![]
    }
    /// Make the document's embedded fonts available before measuring it.
    fn prepare(&self, _doc: &Document) {}
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
    /// Keys of assets added (`add_asset`).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub assets: Vec<String>,
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
    /// Frame times and cache sizes reported by the host (`perf` query).
    pub perf: crate::perf::PerfStats,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new(Document::default())
    }
}

const AGENT_GROUP_SECS: f64 = 2.0;
/// Longest transition, dwell or autoplay time accepted, seconds.
const MAX_SECONDS: f64 = 600.0;
/// World width of a template.
const TEMPLATE_WIDTH: f64 = 6000.0;

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
            perf: Default::default(),
        }
    }

    pub fn theme(&self) -> Theme {
        Theme::resolve_in(&self.doc, &self.doc.theme)
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
    /// The entry Redo would re-apply (for "Redo Rotate" in the Edit menu).
    pub fn redo_entry(&self) -> Option<&HistoryEntry> {
        self.redo.last()
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
        self.measure.prepare(doc);
        let theme = Theme::resolve_in(doc, &doc.theme);
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
        // Connectors follow whatever they are stuck to.
        crate::connector::refresh(&mut self.doc);
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
                    placeholder: false,
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
            Command::AddText { text, x, y, style, scale, place, id, markdown } => {
                let data = if markdown { TextData { style, ..crate::rich::from_markdown(&text) } } else { TextData { text, style, ..Default::default() } };
                let mut e = Element {
                    id: id.unwrap_or_default(),
                    transform: Transform { x, y, scale: scale.unwrap_or(1.0), rotation: 0.0 },
                    size: Size { w: 0.0, h: 0.0 },
                    locked: false,
                    name: String::new(),
                    placeholder: false,
                    kind: ElementKind::Text(data),
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
                    placeholder: false,
                    kind: ElementKind::Frame(FrameData { style, color: None }),
                };
                let id = self.insert_element(e, out)?;
                if add_to_path {
                    self.doc.path.push(PathStep { target: StepTarget::Element { id }, builds: vec![], notes: String::new(), duration: None, dwell: None });
                }
            }
            Command::AddShape { shape, x, y, w, h, rotation, fill, stroke, id } => {
                let (fill, stroke) = match (fill, stroke) {
                    (None, None) => match shape {
                        k if k.is_line() => (None, Some("theme:shape1".to_string())),
                        ShapeKind::StickyNote => (Some("#ffe98a".to_string()), None),
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
                    placeholder: false,
                    kind: ElementKind::Shape(ShapeData { shape, fill, stroke, stroke_width: 4.0, ..Default::default() }),
                };
                self.insert_element(e, out)?;
            }
            Command::AddIcon { name, x, y, size, color, id } => {
                if crate::icons::get(&name).is_none() {
                    let near: Vec<String> = crate::icons::search(&name.replace('-', " "), 8).iter().map(|i| i.name.to_string()).collect();
                    return Err(ZError::invalid(format!("no icon named '{name}'")).hint("search with the icons query").values(near));
                }
                if !(size.is_finite() && size > 0.0) {
                    return Err(ZError::invalid("size must be > 0"));
                }
                let e = Element {
                    id: id.unwrap_or_default(),
                    transform: Transform::at(x, y),
                    size: Size { w: size, h: size },
                    locked: false,
                    name: String::new(),
                    placeholder: false,
                    kind: ElementKind::Icon(IconData { name, color: color.unwrap_or_else(|| "theme:body".into()), stroke_width: 2.0 }),
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
                    placeholder: false,
                    kind: ElementKind::Image(ImageData { asset: key, alt }),
                };
                self.insert_element(e, out)?;
            }
            Command::AddConnector { from, to, route, start, end, color, width, label, id } => {
                for end in [&from, &to] {
                    if let Some(t) = &end.element {
                        let te = self.doc.get(t).ok_or_else(|| ZError::not_found(t, std::iter::empty()))?;
                        if matches!(te.kind, ElementKind::Connector(_)) {
                            return Err(ZError::invalid("a connector can't attach to another connector"));
                        }
                    } else if !(end.x.is_finite() && end.y.is_finite()) {
                        return Err(ZError::invalid("connector ends need an element or finite x, y"));
                    }
                }
                if !(width.is_finite() && width > 0.0) {
                    return Err(ZError::invalid("width must be > 0"));
                }
                let ep = |s: &crate::command::EndSpec| Endpoint { element: s.element.clone(), x: s.x, y: s.y, dir: Vec2::ZERO };
                let e = Element {
                    id: id.unwrap_or_default(),
                    transform: Transform::at(0.0, 0.0),
                    size: Size { w: 0.0, h: 0.0 },
                    locked: false,
                    name: String::new(),
                    placeholder: false,
                    kind: ElementKind::Connector(ConnectorData {
                        from: ep(&from),
                        to: ep(&to),
                        route,
                        start,
                        end,
                        color: color.unwrap_or_else(|| "theme:body".into()),
                        width,
                        dash: Dash::Solid,
                        label,
                    }),
                };
                self.insert_element(e, out)?;
                crate::connector::refresh(&mut self.doc);
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
                    placeholder: false,
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
                let e = self.doc.elements.get_mut(&id).unwrap();
                apply_patch(e, &patch)?;
                if patch.text.is_some() {
                    // Typing into a placeholder makes it real content.
                    e.placeholder = false;
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
            Command::FormatText { id, start, end, find, format } => {
                self.check_unlocked(std::slice::from_ref(&id))?;
                self.element_mut(&id)?;
                let e = self.doc.elements.get_mut(&id).unwrap();
                let ElementKind::Text(t) = &mut e.kind else { return Err(ZError::invalid(format!("{id} is not a text"))) };
                if let Some(c) = format.color.as_deref().filter(|c| !c.is_empty() && !valid_color(c)) {
                    return Err(ZError::invalid(format!("bad colour '{c}'")).values(Theme::COLOR_SLOTS.iter().map(|s| s.to_string()).collect()));
                }
                let n = t.text.chars().count();
                let ranges: Vec<(usize, usize)> = match (&find, start, end) {
                    (Some(f), _, _) if !f.is_empty() => {
                        let found: Vec<(usize, usize)> = t
                            .text
                            .match_indices(f.as_str())
                            .map(|(b, m)| {
                                let s = t.text[..b].chars().count();
                                (s, s + m.chars().count())
                            })
                            .collect();
                        if found.is_empty() {
                            return Err(ZError::invalid(format!("\"{f}\" does not occur in {id}")).hint("`find` is case-sensitive and matches the plain text"));
                        }
                        found
                    }
                    _ => vec![(start.unwrap_or(0).min(n), end.unwrap_or(n).min(n))],
                };
                for (a, b) in ranges {
                    crate::rich::apply_format(t, a, b, &format);
                }
                measure_text(&*self.measure, &theme, e);
                out.changed.push(id);
            }
            Command::FormatParagraphs { id, first, last, list, indent, indent_by, align } => {
                self.check_unlocked(std::slice::from_ref(&id))?;
                self.element_mut(&id)?;
                let e = self.doc.elements.get_mut(&id).unwrap();
                let ElementKind::Text(t) = &mut e.kind else { return Err(ZError::invalid(format!("{id} is not a text"))) };
                let count = t.text.split('\n').count();
                let first = first.unwrap_or(0);
                if first >= count {
                    return Err(ZError::invalid(format!("{id} has {count} paragraph(s); first is {first}")));
                }
                crate::rich::format_paragraphs(t, first, last.unwrap_or(count - 1), list, indent, indent_by, align);
                measure_text(&*self.measure, &theme, e);
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
                self.doc.path.insert(i, PathStep { target, builds, notes, duration: None, dwell: None });
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
            Command::UpdateStep { index, target, builds, notes, duration, dwell } => {
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
                    s.duration = if d < 0.0 { None } else { Some(d.min(MAX_SECONDS)) };
                }
                if let Some(d) = dwell {
                    s.dwell = if d <= 0.0 || !d.is_finite() { None } else { Some(d.min(MAX_SECONDS)) };
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
                    .map(|f| PathStep { target: StepTarget::Element { id: f.id.clone() }, builds: vec![], notes: String::new(), duration: None, dwell: None })
                    .collect();
            }
            Command::SetTheme { theme } => {
                if !Theme::ids_in(&self.doc).contains(&theme) {
                    return Err(ZError::invalid(format!("unknown theme '{theme}'")).values(Theme::ids_in(&self.doc)));
                }
                self.set_theme_id(theme);
            }
            Command::SetTitle { title } => self.doc.title = title,
            Command::SetPlayback { autoplay, looping } => {
                if let Some(a) = autoplay {
                    self.doc.play.autoplay = if a <= 0.0 || !a.is_finite() { None } else { Some(a.min(MAX_SECONDS)) };
                }
                if let Some(l) = looping {
                    self.doc.play.looping = l;
                }
            }
            Command::SaveTheme { mut theme, apply } => {
                self.validate_theme(&theme)?;
                self.embed_fonts(&mut theme);
                let id = theme.id.clone();
                match self.doc.themes.iter_mut().find(|t| t.id == id) {
                    Some(t) => *t = theme,
                    None => self.doc.themes.push(theme),
                }
                if apply || self.doc.theme == id {
                    self.set_theme_id(id);
                }
            }
            Command::DeleteTheme { id } => {
                let Some(i) = self.doc.themes.iter().position(|t| t.id == id) else {
                    return Err(ZError::new("not_found", format!("no custom theme '{id}'"))
                        .values(self.doc.themes.iter().map(|t| t.id.clone()).collect()));
                };
                self.doc.themes.remove(i);
                if self.doc.theme == id {
                    self.set_theme_id("classic".into());
                }
            }
            Command::AddAsset { data_base64 } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data_base64.trim())
                    .map_err(|e| ZError::invalid(format!("data_base64: {e}")))?;
                let key = self.add_asset(bytes)?;
                out.assets.push(key);
            }
            Command::ApplyTemplate { template, replace } => {
                let Some(t) = crate::templates::template(&template) else {
                    return Err(ZError::invalid(format!("unknown template '{template}'"))
                        .values(crate::templates::TEMPLATES.iter().map(|t| t.id.to_string()).collect()));
                };
                if !self.doc.elements.is_empty() || !self.doc.path.is_empty() {
                    if !replace {
                        return Err(ZError::new("not_empty", "the canvas isn't empty")
                            .hint("pass \"replace\": true to clear it, or use add_layout to add to it"));
                    }
                    out.deleted.extend(self.doc.elements.keys().cloned());
                    self.doc.elements.clear();
                    self.doc.z_order.clear();
                    self.doc.path.clear();
                }
                if self.doc.theme != t.theme {
                    self.set_theme_id(t.theme.into());
                }
                let ids = self.add_pieces(&t.pieces, Vec2::new(0.0, 0.0), TEMPLATE_WIDTH, out)?;
                for i in t.path {
                    if let Some(Some(id)) = ids.get(i) {
                        self.doc.path.push(PathStep { target: StepTarget::Element { id: id.clone() }, builds: vec![], notes: String::new(), duration: None, dwell: None });
                    }
                }
            }
            Command::AddLayout { layout, x, y, width, add_to_path } => {
                let Some(pieces) = crate::templates::layout(&layout) else {
                    return Err(ZError::invalid(format!("unknown layout '{layout}'"))
                        .values(crate::templates::LAYOUTS.iter().map(|t| t.id.to_string()).collect()));
                };
                let w = width.unwrap_or(1600.0);
                if !(w.is_finite() && w > 0.0 && x.is_finite() && y.is_finite()) {
                    return Err(ZError::invalid("x, y and width must be finite, width > 0"));
                }
                let ids = self.add_pieces(&pieces, Vec2::new(x, y), w, out)?;
                if add_to_path {
                    for (p, id) in pieces.iter().zip(ids) {
                        if let (crate::templates::Piece::Frame { path: true, .. }, Some(id)) = (p, id) {
                            self.doc.path.push(PathStep { target: StepTarget::Element { id }, builds: vec![], notes: String::new(), duration: None, dwell: None });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn set_theme_id(&mut self, id: String) {
        self.doc.theme = id;
        let mut d = std::mem::take(&mut self.doc);
        self.remeasure_all(&mut d);
        self.doc = d;
    }

    fn validate_theme(&self, t: &Theme) -> ZResult<()> {
        if t.id.trim().is_empty() {
            return Err(ZError::invalid("a theme needs an id"));
        }
        if Theme::is_builtin(&t.id) {
            return Err(ZError::invalid(format!("'{}' is a built-in theme and can't be replaced", t.id))
                .hint("save a copy under a new id, e.g. \"my-theme\""));
        }
        for (field, c) in t.colors() {
            if crate::theme::parse_hex(c).is_none() {
                return Err(ZError::invalid(format!("{field}: bad colour '{c}'")).hint("theme colours are #rrggbb or #rrggbbaa"));
            }
        }
        for s in [&t.title, &t.subtitle, &t.body] {
            if !(s.size.is_finite() && s.size > 0.0) || s.font.trim().is_empty() {
                return Err(ZError::invalid("text styles need a font and a size > 0"));
            }
        }
        if t.layers.len() > crate::theme::MAX_LAYERS {
            return Err(ZError::invalid(format!("a 3D background has at most {} layers", crate::theme::MAX_LAYERS)));
        }
        for l in &t.layers {
            if !(0.0..=1.0).contains(&l.depth) || !(0.0..=1.0).contains(&l.opacity) {
                return Err(ZError::invalid("layer depth and opacity are between 0 and 1"));
            }
        }
        for key in t.background_image.iter().chain(t.layers.iter().map(|l| &l.asset)) {
            if !self.doc.assets.contains_key(key) {
                return Err(ZError::new("not_found", format!("no asset '{key}'")).hint("add the picture with add_asset first"));
            }
        }
        Ok(())
    }

    /// Embed the files of every non-bundled font the theme uses, and drop
    /// embedded fonts it no longer uses.
    fn embed_fonts(&mut self, t: &mut Theme) {
        let families: BTreeSet<String> = [&t.title.font, &t.subtitle.font, &t.body.font].into_iter().cloned().collect();
        t.fonts.retain(|f, _| families.contains(f));
        for fam in families {
            if t.fonts.contains_key(&fam) {
                continue;
            }
            let files = self.measure.font_files(&fam);
            if files.is_empty() {
                continue;
            }
            let mut keys = vec![];
            for (bytes, ext) in files {
                let key = content_key(&bytes);
                self.doc.assets.entry(key.clone()).or_insert(Asset { ext, bytes: bytes.into(), width: 0, height: 0 });
                keys.push(key);
            }
            t.fonts.insert(fam, keys);
        }
    }

    /// Turn unit-space pieces into elements centred at `at`, `width` wide.
    /// Returns the new id of each piece.
    fn add_pieces(&mut self, pieces: &[crate::templates::Piece], at: Vec2, width: f64, out: &mut Outcome) -> ZResult<Vec<Option<ElementId>>> {
        use crate::templates::Piece;
        let theme = self.theme();
        let mut ids = vec![];
        for p in pieces {
            let e = match p {
                Piece::Frame { style, x, y, w, h, rot, name, .. } => Element {
                    id: String::new(),
                    transform: Transform { x: at.x + x * width, y: at.y + y * width, scale: 1.0, rotation: *rot },
                    size: Size { w: w * width, h: h * width },
                    locked: false,
                    name: name.to_string(),
                    placeholder: false,
                    kind: ElementKind::Frame(FrameData { style: *style, color: None }),
                },
                Piece::Text { style, text, x, y, w, h, bullets, align, ph } => {
                    let mut e = Element {
                        id: String::new(),
                        transform: Transform { x: at.x + x * width, y: at.y + y * width, scale: 1.0, rotation: 0.0 },
                        size: Size { w: 0.0, h: 0.0 },
                        locked: false,
                        name: String::new(),
                        placeholder: *ph,
                        kind: ElementKind::Text(TextData { text: text.to_string(), style: *style, bullets: *bullets, align: *align, ..Default::default() }),
                    };
                    measure_text(&*self.measure, &theme, &mut e);
                    let k = (w * width / e.size.w.max(1e-9)).min(h * width / e.size.h.max(1e-9));
                    e.transform.scale = if k.is_finite() && k > 0.0 { k } else { 1.0 };
                    e
                }
                Piece::Shape { shape, x, y, w, h, rot, fill, stroke } => Element {
                    id: String::new(),
                    transform: Transform { x: at.x + x * width, y: at.y + y * width, scale: 1.0, rotation: *rot },
                    size: Size { w: w * width, h: h * width },
                    locked: false,
                    name: String::new(),
                    placeholder: false,
                    kind: ElementKind::Shape(ShapeData {
                        shape: *shape,
                        fill: fill.map(String::from),
                        stroke: stroke.map(String::from),
                        stroke_width: (width * 0.004).max(1.0),
                        ..Default::default()
                    }),
                },
            };
            ids.push(Some(self.insert_element(e, out)?));
        }
        Ok(ids)
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

    /// Store image bytes as an asset; returns the key.
    pub fn add_asset(&mut self, bytes: Vec<u8>) -> ZResult<String> {
        let (ext, w, h) = image_info(&bytes)?;
        let key = content_key(&bytes);
        self.doc.assets.entry(key.clone()).or_insert(Asset { ext, bytes: bytes.into(), width: w, height: h });
        Ok(key)
    }

    /// Copy assets (theme pictures and fonts) from another document, e.g.
    /// a saved theme file. Content-addressed, so existing keys are kept.
    pub fn import_assets<'a>(&mut self, from: &Document, keys: impl IntoIterator<Item = &'a str>) {
        for k in keys {
            if let Some(a) = from.assets.get(k) {
                self.doc.assets.entry(k.to_string()).or_insert_with(|| a.clone());
            }
        }
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
            ViewCommand::PauseAutoplay { paused } => {
                self.view.present.autoplay_paused = paused;
                self.view.present.arrived_at = self.view.now;
            }
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
    // Autoplay

    /// Seconds per step/build at the current position, if autoplay applies.
    pub fn autoplay_interval(&self) -> Option<f64> {
        let step = self.view.present.step.and_then(|i| self.doc.path.get(i));
        step.and_then(|s| s.dwell).or(self.doc.play.autoplay).filter(|d| *d > 0.0)
    }

    /// True when the presentation is at its last step with every build shown.
    pub fn at_end(&self) -> bool {
        let p = &self.view.present;
        match p.step {
            Some(i) => i + 1 >= self.doc.path.len() && p.build >= self.doc.path.get(i).map(|s| s.builds.len()).unwrap_or(0),
            None => self.doc.path.is_empty(),
        }
    }

    /// Advance autoplay. Call every frame after [`ViewState::tick`]. Returns
    /// the seconds until the next automatic advance (for the host to
    /// schedule a repaint), or None when autoplay isn't running.
    pub fn autoplay_tick(&mut self) -> Option<f64> {
        let now = self.view.now;
        let pos = (self.view.present.step, self.view.present.build);
        if self.view.mode != Mode::Present {
            self.view.present.seen = None;
            return None;
        }
        // The clock starts when the camera comes to rest at a new position.
        if self.view.present.seen != Some(pos) || self.view.flight.is_some() {
            self.view.present.seen = Some(pos);
            self.view.present.arrived_at = now;
        }
        let p = &self.view.present;
        if p.autoplay_paused || p.blank.is_some() || p.off_path || self.view.flight.is_some() {
            return None;
        }
        let interval = self.autoplay_interval()?;
        let left = p.arrived_at + interval - now;
        if left > 0.0 {
            return Some(left);
        }
        if self.at_end() {
            if !self.doc.play.looping || self.doc.path.is_empty() {
                return None;
            }
            self.goto_step(Some(0), 0);
        } else {
            self.next();
        }
        self.view.present.arrived_at = now;
        self.autoplay_interval()
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
        ElementKind::Ink(_) => (100.0, 100.0),
        ElementKind::Icon(_) => (96.0, 96.0),
        ElementKind::Connector(_) => (0.0, 0.0),
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
        ElementKind::Connector(d) => {
            check(&Some(d.color.clone()))?;
            if !(d.width.is_finite() && d.width > 0.0) {
                return Err(ZError::invalid("width must be > 0"));
            }
        }
        ElementKind::Icon(d) => {
            check(&Some(d.color.clone()))?;
            if crate::icons::get(&d.name).is_none() {
                return Err(ZError::invalid(format!("no icon named '{}'", d.name)).hint("search with the icons query"));
            }
        }
        ElementKind::Image(i) => {
            if !doc.assets.contains_key(&i.asset) {
                return Err(ZError::invalid(format!("unknown asset '{}'", i.asset)).hint("use add_image"));
            }
        }
    }
    Ok(())
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
    if p.text.is_some() || p.style.is_some() || p.bold.is_some() || p.italic.is_some() || p.align.is_some() || p.bullets.is_some() || p.spans.is_some() || p.paras.is_some() {
        let ElementKind::Text(t) = &mut e.kind else { return Err(wrong("text/style/bold/italic/align/bullets")) };
        if let Some(v) = &p.text {
            // Formatting stays on the characters that survive the edit.
            crate::rich::edit_text(t, v);
        }
        if let Some(v) = &p.spans {
            t.spans = v.clone();
        }
        if let Some(v) = &p.paras {
            t.paras = v.clone();
        }
        if p.spans.is_some() || p.paras.is_some() {
            crate::rich::normalize(t);
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
            ElementKind::Icon(i) => i.color = c.unwrap_or_else(|| "theme:body".into()),
            ElementKind::Connector(k) => k.color = c.unwrap_or_else(|| "theme:body".into()),
            ElementKind::Shape(s) => s.fill = c,
            _ => return Err(wrong("color")),
        }
    }
    let connector_only = p.from.is_some() || p.to.is_some() || p.route.is_some() || p.start_head.is_some() || p.end_head.is_some() || p.label.is_some();
    if let ElementKind::Connector(k) = &mut e.kind {
        if p.shape.is_some() || p.fill.is_some() || p.stroke.is_some() || p.corner_radius.is_some() || p.icon.is_some() {
            return Err(wrong("shape/fill/stroke/corner_radius/icon"));
        }
        let tf = e.transform;
        for (spec, end) in [(&p.from, &mut k.from), (&p.to, &mut k.to)] {
            if let Some(s) = spec {
                let l = tf.inverse(Vec2::new(s.x, s.y));
                *end = Endpoint { element: s.element.clone(), x: l.x, y: l.y, dir: Vec2::ZERO };
            }
        }
        if let Some(v) = p.route {
            k.route = v;
        }
        if let Some(v) = p.start_head {
            k.start = v;
        }
        if let Some(v) = p.end_head {
            k.end = v;
        }
        if let Some(v) = &p.label {
            k.label = v.clone();
        }
        if let Some(v) = p.stroke_width {
            k.width = v.max(0.1);
        }
        if let Some(v) = p.dash {
            k.dash = v;
        }
    } else if connector_only {
        return Err(wrong("from/to/route/start_head/end_head/label"));
    } else if let ElementKind::Icon(i) = &mut e.kind {
        if p.shape.is_some() || p.fill.is_some() || p.stroke.is_some() || p.corner_radius.is_some() || p.dash.is_some() {
            return Err(wrong("shape/fill/stroke/corner_radius/dash"));
        }
        if let Some(v) = p.stroke_width {
            i.stroke_width = v.clamp(0.25, 6.0);
        }
        if let Some(n) = &p.icon {
            if crate::icons::get(n).is_none() {
                return Err(ZError::invalid(format!("no icon named '{n}'")).hint("search with the icons query"));
            }
            i.name = n.clone();
        }
    } else if p.icon.is_some() {
        return Err(wrong("icon"));
    } else if p.shape.is_some() || p.fill.is_some() || p.stroke.is_some() || p.stroke_width.is_some() || p.corner_radius.is_some() || p.dash.is_some() {
        let ElementKind::Shape(s) = &mut e.kind else { return Err(wrong("shape/fill/stroke/stroke_width/corner_radius/dash")) };
        if let Some(v) = p.corner_radius {
            s.corner_radius = v.max(0.0);
        }
        if let Some(v) = p.dash {
            s.dash = v;
        }
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
        if !is("image") {
            return Err(wrong("alt"));
        }
        if let ElementKind::Image(i) = &mut e.kind {
            i.alt = v.clone();
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
        Command::AddIcon { name, .. } => format!("Add icon {name}"),
        Command::AddConnector { .. } => "Add connector".into(),
        Command::AddInk { highlighter, .. } => if *highlighter { "Highlight" } else { "Draw" }.into(),
        Command::Update { patch, .. } if patch.text.is_some() => "Edit text".into(),
        Command::FormatText { .. } => "Format text".into(),
        Command::FormatParagraphs { .. } => "Format paragraphs".into(),
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
        Command::SetPlayback { .. } => "Change autoplay".into(),
        Command::SaveTheme { .. } => "Edit theme".into(),
        Command::DeleteTheme { .. } => "Delete theme".into(),
        Command::AddAsset { .. } => "Add picture".into(),
        Command::ApplyTemplate { template, .. } => format!("Template: {template}"),
        Command::AddLayout { layout, .. } => format!("Insert layout: {layout}"),
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
