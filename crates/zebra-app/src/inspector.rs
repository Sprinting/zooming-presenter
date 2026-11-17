//! The Format inspector: a persistent right-hand pane (⌥⌘I), modelled on
//! Keynote's. It stays put whatever is selected (no canvas reflow): with
//! nothing selected it shows the presentation's settings; with a selection
//! it shows segmented tabs (Style · Text · Arrange · Path) for what the
//! selection supports.
//!
//! # Adding a section (other build streams)
//!
//! Write a function with the [`SectionFn`] signature in your own file and
//! add it to [`EXTRA_SECTIONS`] (one line). It is called for every tab of
//! every selection; return early unless the element kind and tab are yours.
//! Draw with the widget kit (`crate::widgets`), and change the element only
//! through [`Insp::set`] (one undo step per gesture, through the command
//! bus) or `insp.app.apply(Command::…)` for non-patch commands.
//!
//! ```ignore
//! pub fn video_sections(insp: &mut Insp, ui: &mut Ui, tab: Tab) {
//!     let ElementKind::Video(v) = &insp.el.kind else { return };
//!     if tab != Tab::Style { return; }
//!     w::section(ui, "Video", |ui| {
//!         let mut looped = v.looped;
//!         w::row(ui, "", |ui| {
//!             if w::checkbox(ui, &mut looped, "Loop").changed() {
//!                 insp.set(ElementPatch { looped: Some(looped), ..Default::default() });
//!             }
//!         });
//!     });
//! }
//! ```
//! Document-level sections (nothing selected) go in [`EXTRA_DOC_SECTIONS`].
use egui::{Color32, RichText, Ui};
use zebra_core::{
    Command, Element, ElementId, ElementKind, ElementPatch, FrameStyle, Origin, ShapeKind, StepTarget, TextStyle, Theme,
};

use crate::actions::{frame_name, Action};
use crate::app::ZebraApp;
use crate::icons::Icon;
use crate::style::{self, c32, Palette};
use crate::widgets::{self as w, Num, Seg, Swatch};

const H: Origin = Origin::Human;

/// Inspector width (Keynote's format inspector).
pub const WIDTH: f32 = 260.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Style,
    Text,
    Arrange,
    Path,
}

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Tab::Style => "Style",
            Tab::Text => "Text",
            Tab::Arrange => "Arrange",
            Tab::Path => "Path",
        }
    }
}

/// An inspector section drawn for the current selection. Called for every
/// tab; return early unless the element kind and tab are yours.
pub type SectionFn = fn(&mut Insp, &mut Ui, Tab);

/// An inspector section drawn when nothing is selected (presentation
/// settings).
pub type DocSectionFn = fn(&mut ZebraApp, &mut Ui);

/// Sections added by other streams (media, text, presenting). Order here is
/// the order on screen, after the built-in sections of the same tab.
pub static EXTRA_SECTIONS: &[SectionFn] = &[crate::inspector_s3::sections, crate::inspector_s3::media, crate::inspector_s3::path_step];

/// Document sections added by other streams (theme editor, autoplay...).
pub static EXTRA_DOC_SECTIONS: &[DocSectionFn] = &[crate::media_ui::document_section];

/// Which tabs an element kind offers. Other streams add their kinds here.
pub fn tabs_for(kind: Option<&ElementKind>, multi: bool) -> Vec<Tab> {
    if multi {
        return vec![Tab::Arrange, Tab::Path];
    }
    match kind {
        Some(ElementKind::Text(_)) => vec![Tab::Text, Tab::Arrange, Tab::Path],
        Some(_) => vec![Tab::Style, Tab::Arrange, Tab::Path],
        None => vec![],
    }
}

/// The state a section works with: the app, the selected element (a
/// snapshot taken this frame) and the pending patch.
pub struct Insp<'a> {
    pub app: &'a mut ZebraApp,
    /// The selected element (the first one when several are selected).
    pub el: Element,
    pub sel: Vec<ElementId>,
    pub theme: Theme,
    patch: Option<ElementPatch>,
    pub gesture: bool,
}

impl Insp<'_> {
    /// Change the selected element. Live edits (drags, typing, steppers)
    /// coalesce into one undo step per gesture when the control's response
    /// was passed to [`Insp::track`].
    pub fn set(&mut self, patch: ElementPatch) {
        self.patch = Some(merge(self.patch.take(), patch));
    }
    /// Mark a control as mid-gesture (dragging or focused) so its changes
    /// join one undo step.
    pub fn track(&mut self, r: &egui::Response) {
        self.gesture |= r.dragged() || r.has_focus() || r.is_pointer_button_down_on();
    }
    pub fn color(&self, key: &str) -> Color32 {
        c32(self.theme.color(&key.to_string()))
    }
}

fn merge(a: Option<ElementPatch>, b: ElementPatch) -> ElementPatch {
    let Some(mut a) = a else { return b };
    let v = serde_json::to_value(&b).unwrap_or_default();
    let mut av = serde_json::to_value(&a).unwrap_or_default();
    if let (Some(ao), Some(bo)) = (av.as_object_mut(), v.as_object()) {
        for (k, val) in bo {
            ao.insert(k.clone(), val.clone());
        }
        if let Ok(m) = serde_json::from_value(av) {
            a = m;
        }
    }
    a
}

/// Draw the inspector pane contents.
pub fn ui(app: &mut ZebraApp, ui: &mut Ui) {
    let p = Palette::of(ui.ctx());
    let sel = app.selection();
    let el = sel.first().and_then(|id| app.editor.doc.get(id)).cloned();

    if sel.is_empty() || el.is_none() {
        document(app, ui, &p);
        return;
    }
    let el = el.unwrap();
    let tabs = tabs_for(Some(&el.kind), sel.len() > 1);
    let mut tab = app.inspector_tab;
    if !tabs.contains(&tab) {
        tab = tabs[0];
    }
    ui.add_space(10.0);
    let items: Vec<(Tab, &str)> = tabs.iter().map(|t| (*t, t.label())).collect();
    if w::tab_bar(ui, &mut tab, &items).changed() {
        app.inspector_tab = tab;
    }
    ui.add_space(6.0);
    w::separator(ui);

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.set_width(ui.available_width() - 4.0);
        let theme = app.editor.theme();
        let mut insp = Insp { app, el: el.clone(), sel: sel.clone(), theme, patch: None, gesture: false };
        header(&mut insp, ui, &p);
        if sel.len() > 1 {
            match tab {
                Tab::Arrange => arrange_multi(&mut insp, ui),
                Tab::Path => path_tab(&mut insp, ui, &p),
                _ => {}
            }
        } else {
            match tab {
                Tab::Style => style_tab(&mut insp, ui),
                Tab::Text => text_tab(&mut insp, ui),
                Tab::Arrange => arrange_tab(&mut insp, ui, &p),
                Tab::Path => path_tab(&mut insp, ui, &p),
            }
        }
        for f in EXTRA_SECTIONS {
            f(&mut insp, ui, tab);
        }
        finish(insp);
    });
}

/// Apply the pending patch; one undo step per gesture.
fn finish(insp: Insp) {
    let Insp { app, el, patch, gesture, .. } = insp;
    if let Some(patch) = patch {
        if gesture && !app.editor.in_transaction() {
            let _ = app.editor.begin("Change Properties", H);
        }
        let r = app.editor.apply(Command::Update { id: el.id.clone(), patch }, H);
        app.report(r);
    }
    // Crop mode (stream 2) keeps its own transaction open.
    if !gesture && app.editor.human_gesture_active() && matches!(app.canvas.gesture, crate::canvas::Gesture::None) && !crate::media_ui::crop_active(app) {
        app.editor.commit();
    }
}

fn kind_name(e: &Element) -> String {
    match &e.kind {
        ElementKind::Frame(f) => format!("{} Frame", frame_name(f.style)),
        ElementKind::Text(t) => match t.style {
            TextStyle::Title => "Title".into(),
            TextStyle::Subtitle => "Subtitle".into(),
            TextStyle::Body => "Text".into(),
        },
        ElementKind::Shape(s) => s.shape.label().into(),
        ElementKind::Icon(i) => format!("Icon · {}", i.name),
        ElementKind::Video(_) => "Video".into(),
        ElementKind::Connector(_) => "Connector".into(),
        ElementKind::Image(_) => "Image".into(),
        ElementKind::Ink(_) => "Drawing".into(),
        #[allow(unreachable_patterns)]
        _ => "Object".into(),
    }
}

fn header(insp: &mut Insp, ui: &mut Ui, p: &Palette) {
    ui.add_space(8.0);
    let title = if insp.sel.len() > 1 { format!("{} Objects", insp.sel.len()) } else { kind_name(&insp.el) };
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(style::semibold(13.0)).color(p.text));
        if insp.sel.len() == 1 && insp.el.locked {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("Locked").font(style::ui_font(11.0)).color(p.weak));
            });
        }
    });
    // The element id is for agents; show it only while ⌥ is held.
    if insp.sel.len() == 1 && ui.input(|i| i.modifiers.alt) {
        w::caption(ui, format!("ID {}", insp.el.id));
    }
}

// ----------------------------------------------------------------------
// Colour helpers

pub fn theme_swatches<'a>(insp: &Insp, slots: &[(&'a str, &'a str)]) -> Vec<(Color32, &'a str, &'a str)> {
    slots.iter().map(|(k, tip)| (if k.is_empty() { Color32::TRANSPARENT } else { insp.color(k) }, *k, *tip)).collect()
}

/// Theme swatches plus a colour well for a custom colour. Returns the new
/// colour key (`theme:*`, `""`, or `#rrggbb`).
pub fn colour_picker(ui: &mut Ui, id: &str, current: &str, default_color: Color32, swatches: &[(Color32, &str, &str)]) -> Option<String> {
    let mut out = None;
    let sw: Vec<Swatch> = swatches.iter().map(|(c, k, t)| Swatch { color: *c, key: k, tip: t }).collect();
    if let Some(k) = w::swatch_grid(ui, &sw, current) {
        out = Some(k);
    }
    let cur_color = swatches.iter().find(|(_, k, _)| *k == current).map(|s| s.0).or_else(|| parse_hex(current)).unwrap_or(default_color);
    w::row(ui, "Custom", |ui| {
        if let Some(c) = w::color_well(ui, id, cur_color) {
            out = Some(w::hex(c));
        }
        if current.starts_with('#') {
            ui.label(RichText::new(current).font(style::ui_font(11.0)).color(ui.visuals().weak_text_color()));
        }
    });
    out
}

fn parse_hex(s: &str) -> Option<Color32> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

// ----------------------------------------------------------------------
// Style tab

fn style_tab(insp: &mut Insp, ui: &mut Ui) {
    let el = insp.el.clone();
    match &el.kind {
        ElementKind::Frame(f) => {
            w::section(ui, "Frame", |ui| {
                let mut st = f.style;
                let items: Vec<(FrameStyle, Seg)> = FrameStyle::ALL.iter().map(|s| (*s, Seg::Icon(Icon::Frame(*s), frame_name(*s)))).collect();
                let wdt = ui.available_width();
                if w::segmented(ui, &mut st, &items, Some(wdt)).changed() {
                    insp.set(ElementPatch { frame_style: Some(st), ..Default::default() });
                }
                let mut name = el.name.clone();
                w::row(ui, "Name", |ui| {
                    let r = w::text_field(ui, &mut name, ui.available_width(), &el.label(&insp.app.editor.doc));
                    insp.track(&r);
                    if r.changed() {
                        insp.set(ElementPatch { name: Some(name.clone()), ..Default::default() });
                    }
                });
                let n = insp.app.editor.doc.frame_contents(&el.id).len();
                w::caption(ui, format!("Contains {n} object{}.", if n == 1 { "" } else { "s" }));
            });
            w::section(ui, "Colour", |ui| {
                let cur = f.color.clone().unwrap_or_default();
                let sw = theme_swatches(insp, &[("theme:frame", "Theme frame colour"), ("theme:accent", "Accent"), ("theme:shape1", "Colour 1"), ("theme:shape2", "Colour 2"), ("theme:shape3", "Colour 3"), ("theme:shape4", "Colour 4"), ("theme:title", "Title")]);
                let cur_key = if cur.is_empty() { "theme:frame" } else { cur.as_str() };
                let def = insp.color("theme:frame");
                if let Some(k) = colour_picker(ui, "frame-colour", cur_key, def, &sw) {
                    let k = if k == "theme:frame" { String::new() } else { k };
                    insp.set(ElementPatch { color: Some(k), ..Default::default() });
                }
            });
        }
        ElementKind::Shape(s) => {
            w::section(ui, "Shape", |ui| {
                // All 39 library shapes (stream 3); lines stay lines.
                let wdt = ui.available_width();
                let mut picked = None;
                w::popup_button(ui, "shape-kind", s.shape.label(), wdt, |ui| {
                    for &k in ShapeKind::ALL.iter().filter(|k| k.is_line() == s.shape.is_line()) {
                        if ui.selectable_label(s.shape == k, k.label()).clicked() {
                            picked = Some(k);
                        }
                    }
                });
                if let Some(k) = picked {
                    insp.set(ElementPatch { shape: Some(k), ..Default::default() });
                }
            });
            let slots = [("", "None"), ("theme:shape1", "Colour 1"), ("theme:shape2", "Colour 2"), ("theme:shape3", "Colour 3"), ("theme:shape4", "Colour 4"), ("theme:accent", "Accent"), ("theme:frame", "Frame"), ("theme:title", "Title"), ("theme:background", "Background")];
            if !s.shape.is_line() {
                w::section(ui, "Fill", |ui| {
                    let cur = s.fill.clone().unwrap_or_default();
                    let sw = theme_swatches(insp, &slots);
                    if let Some(k) = colour_picker(ui, "fill", &cur, Color32::GRAY, &sw) {
                        insp.set(ElementPatch { fill: Some(k), ..Default::default() });
                    }
                });
            }
            w::section(ui, if s.shape.is_line() { "Line" } else { "Border" }, |ui| {
                let cur = s.stroke.clone().unwrap_or_default();
                let sw = theme_swatches(insp, &slots);
                if let Some(k) = colour_picker(ui, "stroke", &cur, Color32::GRAY, &sw) {
                    insp.set(ElementPatch { stroke: Some(k), ..Default::default() });
                }
                let mut sw = s.stroke_width;
                w::row(ui, "Width", |ui| {
                    let r = w::number_field(ui, &mut sw, Num::pt().range(0.0, 200.0).step(0.5).decimals(1));
                    insp.track(&r);
                    if r.changed() {
                        insp.set(ElementPatch { stroke_width: Some(sw), ..Default::default() });
                    }
                });
            });
        }
        ElementKind::Image(i) => {
            w::section(ui, "Image", |ui| {
                let mut alt = i.alt.clone();
                w::row(ui, "Description", |ui| {
                    let r = w::text_field(ui, &mut alt, ui.available_width(), "Alt text");
                    insp.track(&r);
                    if r.changed() {
                        insp.set(ElementPatch { alt: Some(alt.clone()), ..Default::default() });
                    }
                });
                w::caption(ui, "Read aloud by VoiceOver and used in exported PDFs.");
            });
        }
        ElementKind::Ink(ink) => {
            w::section(ui, "Drawing", |ui| {
                let cur = ink.color.clone();
                let sw = theme_swatches(insp, &[("theme:title", "Title"), ("theme:body", "Body"), ("theme:accent", "Accent"), ("theme:shape1", "Colour 1"), ("theme:shape2", "Colour 2"), ("theme:shape3", "Colour 3"), ("theme:shape4", "Colour 4")]);
                if let Some(k) = colour_picker(ui, "ink", &cur, Color32::BLACK, &sw) {
                    insp.set(ElementPatch { color: Some(k), ..Default::default() });
                }
            });
        }
        _ => {}
    }
}

// ----------------------------------------------------------------------
// Text tab

fn text_tab(insp: &mut Insp, ui: &mut Ui) {
    let el = insp.el.clone();
    let ElementKind::Text(t) = &el.kind else { return };
    w::section(ui, "", |ui| {
        let label = match t.style {
            TextStyle::Title => "Title",
            TextStyle::Subtitle => "Subtitle",
            TextStyle::Body => "Body",
        };
        let wdt = ui.available_width();
        w::popup_button(ui, "para-style", label, wdt, |ui| {
            for (s, l) in [(TextStyle::Title, "Title"), (TextStyle::Subtitle, "Subtitle"), (TextStyle::Body, "Body")] {
                if w::menu_check(ui, t.style == s, l, None).clicked() {
                    insp.set(ElementPatch { style: Some(s), ..Default::default() });
                }
            }
        });
    });
    w::section(ui, "Font", |ui| {
        let font = match t.style {
            TextStyle::Title => insp.theme.title.font.clone(),
            TextStyle::Subtitle => insp.theme.subtitle.font.clone(),
            TextStyle::Body => insp.theme.body.font.clone(),
        };
        let font = match font.as_str() {
            "Serif" => "Lora",
            "Hand" => "Caveat",
            "Mono" => "JetBrains Mono",
            _ => "Inter",
        };
        w::caption(ui, format!("{font} (from the theme)"));
        let wdt = ui.available_width();
        if let Some(i) = w::segmented_toggles(ui, &[t.bold, t.italic], &[Seg::Icon(Icon::Bold, "Bold  ⌘B"), Seg::Icon(Icon::Italic, "Italic  ⌘I")], Some(wdt * 0.5)) {
            match i {
                0 => insp.set(ElementPatch { bold: Some(!t.bold), ..Default::default() }),
                _ => insp.set(ElementPatch { italic: Some(!t.italic), ..Default::default() }),
            }
        }
        let cur = t.color.clone().unwrap_or_default();
        let def_key = match t.style {
            TextStyle::Title => "theme:title",
            TextStyle::Subtitle => "theme:subtitle",
            TextStyle::Body => "theme:body",
        };
        let sw = theme_swatches(insp, &[(def_key, "Theme default"), ("theme:title", "Title"), ("theme:subtitle", "Subtitle"), ("theme:body", "Body"), ("theme:accent", "Accent"), ("theme:shape1", "Colour 1"), ("theme:shape2", "Colour 2"), ("theme:shape3", "Colour 3"), ("theme:shape4", "Colour 4")]);
        let cur_key = if cur.is_empty() { def_key } else { cur.as_str() };
        let def = insp.color(def_key);
        if let Some(k) = colour_picker(ui, "text-colour", cur_key, def, &sw) {
            insp.set(ElementPatch { color: Some(if k == def_key { String::new() } else { k }), ..Default::default() });
        }
    });
    w::section(ui, "Alignment", |ui| {
        let mut a = t.align;
        let items = [
            (zebra_core::Align::Left, Seg::Icon(Icon::AlignLeft, "Align Left")),
            (zebra_core::Align::Center, Seg::Icon(Icon::AlignCenter, "Centre")),
            (zebra_core::Align::Right, Seg::Icon(Icon::AlignRight, "Align Right")),
        ];
        ui.horizontal(|ui| {
            let wdt = ui.available_width() * 0.55;
            if w::segmented(ui, &mut a, &items, Some(wdt)).changed() {
                insp.set(ElementPatch { align: Some(a), ..Default::default() });
            }
        });
        let mut bullets = t.bullets;
        w::row(ui, "Bullets", |ui| {
            let wdt = ui.available_width();
            let mut v = bullets;
            if w::segmented(ui, &mut v, &[(false, Seg::Text("None")), (true, Seg::Text("Bullets"))], Some(wdt)).changed() {
                bullets = v;
                insp.set(ElementPatch { bullets: Some(bullets), ..Default::default() });
            }
        });
    });
    w::section(ui, "", |ui| {
        if w::push_button_ext(ui, "Edit Text", w::Size::Regular, !el.locked, Some(ui.available_width())).on_hover_text("Edit the text in place  ↩ or double-click").clicked() {
            crate::canvas::begin_edit_existing(insp.app, el.id.clone());
        }
    });
}

// ----------------------------------------------------------------------
// Arrange tab

fn arrange_tab(insp: &mut Insp, ui: &mut Ui, _p: &Palette) {
    let el = insp.el.clone();
    let tr = el.transform;
    w::section(ui, "Size", |ui| {
        // W × H as the audience sees it (local size × scale).
        let ww = el.size.w * tr.scale;
        let hh = el.size.h * tr.scale;
        let mut scale = tr.scale;
        w::row(ui, "Width", |ui| {
            let mut v = ww;
            let r = w::number_field(ui, &mut v, Num { decimals: 0, min: 1e-6, ..Default::default() }.width(72.0));
            insp.track(&r);
            if r.changed() && ww > 0.0 {
                scale = tr.scale * v / ww;
                insp.set(ElementPatch { scale: Some(scale), ..Default::default() });
            }
        });
        w::row(ui, "Height", |ui| {
            let mut v = hh;
            let r = w::number_field(ui, &mut v, Num { decimals: 0, min: 1e-6, ..Default::default() }.width(72.0));
            insp.track(&r);
            if r.changed() && hh > 0.0 {
                scale = tr.scale * v / hh;
                insp.set(ElementPatch { scale: Some(scale), ..Default::default() });
            }
        });
        w::caption(ui, "Width and height keep the proportions, as in Prezi.");
    });
    w::section(ui, "Rotate", |ui| {
        let mut rot = tr.rotation;
        w::row(ui, "Angle", |ui| {
            let r = w::number_field(ui, &mut rot, Num::deg().step(1.0).width(72.0));
            insp.track(&r);
            if r.changed() {
                insp.set(ElementPatch { rotation: Some(rot), ..Default::default() });
            }
        });
    });
    w::disclosure_section(ui, "position", "Position", false, |ui| {
        let (mut x, mut y) = (tr.x, tr.y);
        w::row(ui, "X", |ui| {
            let r = w::number_field(ui, &mut x, Num { decimals: 0, ..Default::default() }.width(72.0));
            insp.track(&r);
            if r.changed() {
                insp.set(ElementPatch { x: Some(x), ..Default::default() });
            }
        });
        w::row(ui, "Y", |ui| {
            let r = w::number_field(ui, &mut y, Num { decimals: 0, ..Default::default() }.width(72.0));
            insp.track(&r);
            if r.changed() {
                insp.set(ElementPatch { y: Some(y), ..Default::default() });
            }
        });
        w::caption(ui, "Canvas coordinates of the centre.");
    });
    arrange_common(insp, ui);
}

fn arrange_common(insp: &mut Insp, ui: &mut Ui) {
    let sel = insp.sel.clone();
    let doc = &insp.app.editor.doc;
    let locked = !sel.is_empty() && sel.iter().all(|id| doc.get(id).is_some_and(|e| e.locked));
    w::section(ui, "Layer", |ui| {
        let b = [
            ("Forward", "Bring forward one layer  ]", !locked),
            ("Backward", "Send backward one layer  [", !locked),
            ("To Front", "Bring to front  ⌘]", !locked),
            ("To Back", "Send to back  ⌘[", !locked),
        ];
        match w::button_grid(ui, &b, 2) {
            Some(0) => insp.app.queued.push(Action::BringForward),
            Some(1) => insp.app.queued.push(Action::SendBackward),
            Some(2) => insp.app.queued.push(Action::BringToFront),
            Some(3) => insp.app.queued.push(Action::SendToBack),
            _ => {}
        }
    });
    w::section(ui, "", |ui| {
        let b = [
            (if locked { "Unlock" } else { "Lock" }, if locked { "Unlock so it can be moved and edited  ⌘L" } else { "Lock in place  ⌘L" }, true),
            ("Duplicate", "Duplicate the selection  ⌘D", true),
        ];
        match w::button_grid(ui, &b, 2) {
            Some(0) => insp.app.queued.push(Action::ToggleLock),
            Some(1) => insp.app.queued.push(Action::Duplicate),
            _ => {}
        }
        if w::push_button_ext(ui, "Frame Selection", w::Size::Regular, true, Some(ui.available_width())).on_hover_text("Draw a frame around the selection  ⌘G").clicked() {
            insp.app.queued.push(Action::WrapInFrame);
        }
    });
}

fn arrange_multi(insp: &mut Insp, ui: &mut Ui) {
    use zebra_core::command::{AlignEdge, Layout};
    let sel = insp.sel.clone();
    w::section(ui, "Align", |ui| {
        let b = [("Left", "", true), ("Centre", "", true), ("Right", "", true), ("Top", "", true), ("Middle", "", true), ("Bottom", "", true)];
        let edges = [AlignEdge::Left, AlignEdge::CenterX, AlignEdge::Right, AlignEdge::Top, AlignEdge::CenterY, AlignEdge::Bottom];
        if let Some(i) = w::button_grid(ui, &b, 3) {
            insp.app.apply(Command::Align { ids: sel.clone(), edge: edges[i] });
        }
    });
    w::section(ui, "Arrange", |ui| {
        let b = [("Row", "Arrange in a row", true), ("Column", "Arrange in a column", true), ("Grid", "Arrange in a grid", true), ("Circle", "Arrange in a circle", true)];
        let lay = [Layout::Row, Layout::Column, Layout::Grid, Layout::Circle];
        if let Some(i) = w::button_grid(ui, &b, 2) {
            insp.app.apply(Command::Arrange { ids: sel.clone(), layout: lay[i], gap: None });
        }
    });
    arrange_common(insp, ui);
}

// ----------------------------------------------------------------------
// Path tab

fn path_tab(insp: &mut Insp, ui: &mut Ui, _p: &Palette) {
    let sel = insp.sel.clone();
    let steps: Vec<usize> = {
        let doc = &insp.app.editor.doc;
        let mut v: Vec<usize> = sel.iter().flat_map(|id| doc.steps_using(id)).collect();
        v.sort();
        v.dedup();
        v
    };
    w::section(ui, "Path", |ui| {
        if steps.is_empty() {
            w::caption(ui, if sel.len() > 1 { "None of these objects is on the path." } else { "Not on the path." });
            if w::push_button_ext(ui, "Add to Path", w::Size::Regular, true, Some(ui.available_width())).on_hover_text("Add a step that zooms to the selection  ⇧⌘A").clicked() {
                insp.app.queued.push(Action::AddToPath);
            }
        } else {
            let list = steps.iter().map(|s| (s + 1).to_string()).collect::<Vec<_>>().join(", ");
            w::caption(ui, format!("On the path as step{} {list}.", if steps.len() == 1 { "" } else { "s" }));
            if w::push_button_ext(ui, "Remove from Path", w::Size::Regular, true, Some(ui.available_width())).on_hover_text("Remove every step that zooms to this").clicked() {
                let cmds: Vec<Command> = steps.iter().rev().map(|i| Command::RemoveStep { index: *i }).collect();
                let r = insp.app.editor.apply_batch(cmds, Some("Remove from Path".into()), H);
                insp.app.report(r);
            }
        }
    });

    // Fade-in builds: classic Prezi "Animate" within a step.
    if sel.len() == 1 {
        let id = insp.el.id.clone();
        let doc = &insp.app.editor.doc;
        let fade_steps: Vec<usize> = (0..doc.path.len())
            .filter(|i| match &doc.path[*i].target {
                StepTarget::Element { id: f } => *f != id && doc.frame_contents(f).contains(&id),
                _ => false,
            })
            .collect();
        if let Some(&si) = fade_steps.last() {
            let builds = doc.path[si].builds.clone();
            w::section(ui, "Animation", |ui| {
                let mut on = builds.contains(&id);
                if w::checkbox(ui, &mut on, &format!("Fade in on step {}", si + 1)).on_hover_text("Appears with a click after the camera arrives").changed() {
                    let mut b = builds;
                    if on {
                        b.push(id.clone());
                    } else {
                        b.retain(|x| *x != id);
                    }
                    insp.app.apply(Command::UpdateStep { index: si, target: None, builds: Some(b), notes: None, duration: None, dwell: None });
                }
            });
        }
    }
}

// ----------------------------------------------------------------------
// Nothing selected: presentation settings

fn document(app: &mut ZebraApp, ui: &mut Ui, p: &Palette) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.set_width(ui.available_width() - 4.0);
        ui.add_space(12.0);
        ui.label(RichText::new("Presentation").font(style::semibold(13.0)).color(p.text));
        w::section(ui, "", |ui| {
            let mut title = app.title_buf.clone().unwrap_or_else(|| app.editor.doc.title.clone());
            w::row(ui, "Title", |ui| {
                let r = w::text_field(ui, &mut title, ui.available_width(), "Untitled");
                if r.has_focus() {
                    app.title_buf = Some(title.clone());
                }
                if r.lost_focus() {
                    app.title_buf = None;
                    let t = title.trim().to_string();
                    if !t.is_empty() && t != app.editor.doc.title {
                        app.apply(Command::SetTitle { title: t });
                    }
                }
            });
        });
        w::section(ui, "Theme", |ui| {
            let cur = app.editor.doc.theme.clone();
            let name = app.editor.theme().name.clone();
            let wdt = ui.available_width();
            w::popup_button(ui, "theme", &name, wdt, |ui| {
                for t in Theme::builtins() {
                    if w::menu_check(ui, t.id == cur, &t.name, None).clicked() {
                        app.queued.push(Action::SetTheme(t.id.clone()));
                    }
                }
            });
            let th = app.editor.theme();
            ui.horizontal(|ui| {
                for slot in ["theme:background", "theme:title", "theme:body", "theme:frame", "theme:accent", "theme:shape1", "theme:shape2"] {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                    ui.painter().rect(r, 3.0, c32(th.color(&slot.to_string())), egui::Stroke::new(0.6, Color32::from_black_alpha(45)), egui::StrokeKind::Inside);
                }
            });
        });
        w::section(ui, "Path", |ui| {
            let n = app.editor.doc.path.len();
            w::caption(ui, match n {
                0 => "No steps yet. Select a frame and choose Add to Path, or use Edit Path.".to_string(),
                1 => "1 step.".to_string(),
                n => format!("{n} steps."),
            });
            let b = [("Edit Path", "Click objects to add steps  P", true), ("From Frames", "Replace the path with every frame, in reading order", true)];
            match w::button_grid(ui, &b, 2) {
                Some(0) => app.queued.push(Action::TogglePathEdit),
                Some(1) => app.queued.push(Action::AutoPath),
                _ => {}
            }
        });
        for f in EXTRA_DOC_SECTIONS {
            f(app, ui);
        }
    });
}
