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
    Command, Element, ElementId, ElementKind, ElementPatch, FrameStyle, ListKind, Origin, ShapeKind, StepTarget, TextStyle, Theme,
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
    pub fn label(self) -> String {
        match self {
            Tab::Style => crate::t!("inspector-tab-style"),
            Tab::Text => crate::t!("inspector-tab-text"),
            Tab::Arrange => crate::t!("inspector-tab-arrange"),
            Tab::Path => crate::t!("inspector-tab-path"),
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
    let labels: Vec<(Tab, String)> = tabs.iter().map(|t| (*t, t.label())).collect();
    let items: Vec<(Tab, &str)> = labels.iter().map(|(t, l)| (*t, l.as_str())).collect();
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
            let _ = app.editor.begin(crate::t!("inspector-undo-change-properties"), H);
        }
        // A new frame style can leave things outside the outline (a
        // rectangle made a circle): say how many no longer move with it.
        let held = patch.frame_style.map(|_| app.editor.frame_contents(&el.id));
        let r = app.editor.apply(Command::Update { id: el.id.clone(), patch }, H);
        let ok = r.is_ok();
        app.report(r);
        if let (Some(before), true) = (held, ok) {
            let after = app.editor.frame_contents(&el.id);
            let released = before.iter().filter(|id| !after.contains(id)).count();
            if released > 0 {
                app.note(crate::t!("canvas-note-frame-released", count = released));
            }
        }
    }
    // Crop mode (stream 2) keeps its own transaction open.
    if !gesture && app.editor.human_gesture_active() && matches!(app.canvas.gesture, crate::canvas::Gesture::None) && !crate::media_ui::crop_active(app) {
        app.editor.commit_as(&crate::app::HUMAN);
    }
}

fn kind_name(e: &Element) -> String {
    match &e.kind {
        ElementKind::Frame(f) => crate::t!("inspector-kind-frame", style = frame_name(f.style)),
        ElementKind::Text(t) => match t.style {
            TextStyle::Title => crate::t!("inspector-kind-title"),
            TextStyle::Subtitle => crate::t!("inspector-kind-subtitle"),
            TextStyle::Body => crate::t!("inspector-kind-text"),
        },
        ElementKind::Shape(s) => crate::words::shape_name(s.shape),
        ElementKind::Icon(i) => crate::t!("inspector-kind-icon", name = i.name.as_str()),
        ElementKind::Video(_) => crate::t!("inspector-kind-video"),
        ElementKind::Connector(_) => crate::t!("inspector-kind-connector"),
        ElementKind::Image(i) if i.symbol.is_some() => crate::t!("inspector-kind-symbol"),
        ElementKind::Image(_) => crate::t!("inspector-kind-image"),
        ElementKind::Ink(_) => crate::t!("inspector-kind-drawing"),
        #[allow(unreachable_patterns)]
        _ => crate::t!("inspector-kind-object"),
    }
}

/// The selection is exactly one whole group.
fn one_group(insp: &Insp) -> bool {
    let doc = &insp.app.editor.doc;
    let g = insp.sel.first().and_then(|id| doc.get(id)).and_then(|e| e.group.clone());
    g.is_some() && insp.sel.iter().all(|id| doc.get(id).is_some_and(|e| e.group == g)) && doc.expand_groups(&insp.sel).len() == insp.sel.len()
}

fn header(insp: &mut Insp, ui: &mut Ui, p: &Palette) {
    ui.add_space(8.0);
    let title = match insp.sel.len() {
        1 => kind_name(&insp.el),
        n if one_group(insp) => crate::t!("inspector-group-of", count = n),
        n => crate::t!("inspector-objects-count", count = n),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(style::semibold(13.0)).color(p.text));
        if insp.sel.len() == 1 && insp.el.locked {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(crate::t!("inspector-locked")).font(style::ui_font(11.0)).color(p.weak));
            });
        }
    });
    // The element id is for agents; show it only while ⌥ is held.
    if insp.sel.len() == 1 && ui.input(|i| i.modifiers.alt) {
        w::caption(ui, crate::t!("inspector-element-id", id = insp.el.id.to_string()));
    }
}

// ----------------------------------------------------------------------
// Colour helpers

/// `slots`: (theme key, translated tooltip).
pub fn theme_swatches<'a>(insp: &Insp, slots: &'a [(&'a str, String)]) -> Vec<(Color32, &'a str, &'a str)> {
    slots.iter().map(|(k, tip)| (if k.is_empty() { Color32::TRANSPARENT } else { insp.color(k) }, *k, tip.as_str())).collect()
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
    w::row(ui, &crate::t!("inspector-custom-colour"), |ui| {
        if let Some(c) = w::color_well(ui, id, cur_color) {
            out = Some(w::hex(c));
        }
        if current.starts_with('#') {
            ui.label(RichText::new(current).font(style::ui_font(11.0)).color(ui.visuals().weak_text_color()));
        }
    });
    out
}

/// A tooltip followed by the action's live shortcut.
fn tip(text: String, action: Action) -> String {
    crate::i18n::with_shortcut(text, &action)
}

/// [`w::button_grid`] for translated (owned) labels and tooltips.
fn button_grid(ui: &mut Ui, labels: &[(String, String, bool)], cols: usize) -> Option<usize> {
    let b: Vec<(&str, &str, bool)> = labels.iter().map(|(l, t, e)| (l.as_str(), t.as_str(), *e)).collect();
    w::button_grid(ui, &b, cols)
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
            w::section(ui, &crate::t!("inspector-section-frame"), |ui| {
                let mut st = f.style;
                let items: Vec<(FrameStyle, Seg)> = FrameStyle::ALL.iter().map(|s| (*s, Seg::Icon(Icon::Frame(*s), frame_name(*s)))).collect();
                let wdt = ui.available_width();
                if w::segmented(ui, &mut st, &items, Some(wdt)).changed() {
                    insp.set(ElementPatch { frame_style: Some(st), ..Default::default() });
                }
                let mut name = el.name.clone();
                w::row(ui, &crate::t!("inspector-name"), |ui| {
                    let r = w::text_field(ui, &mut name, ui.available_width(), &crate::words::element_label(&el, &insp.app.editor.doc));
                    insp.track(&r);
                    if r.changed() {
                        insp.set(ElementPatch { name: Some(name.clone()), ..Default::default() });
                    }
                });
                let n = insp.app.editor.frame_contents(&el.id).len();
                if n > 0 && insp.app.editor.frame_only() == Some(&el.id) {
                    w::caption(ui, crate::t!("inspector-frame-only", count = n));
                } else {
                    w::caption(ui, crate::t!("inspector-frame-contains", count = n));
                }
            });
            w::section(ui, &crate::t!("inspector-section-colour"), |ui| {
                let cur = f.color.clone().unwrap_or_default();
                let slots = [
                    ("theme:frame", crate::t!("inspector-swatch-theme-frame")),
                    ("theme:accent", crate::t!("inspector-swatch-accent")),
                    ("theme:shape1", crate::t!("inspector-swatch-colour-1")),
                    ("theme:shape2", crate::t!("inspector-swatch-colour-2")),
                    ("theme:shape3", crate::t!("inspector-swatch-colour-3")),
                    ("theme:shape4", crate::t!("inspector-swatch-colour-4")),
                    ("theme:title", crate::t!("inspector-swatch-title")),
                ];
                let sw = theme_swatches(insp, &slots);
                let cur_key = if cur.is_empty() { "theme:frame" } else { cur.as_str() };
                let def = insp.color("theme:frame");
                if let Some(k) = colour_picker(ui, "frame-colour", cur_key, def, &sw) {
                    let k = if k == "theme:frame" { String::new() } else { k };
                    insp.set(ElementPatch { color: Some(k), ..Default::default() });
                }
            });
        }
        ElementKind::Shape(s) => {
            w::section(ui, &crate::t!("inspector-section-shape"), |ui| {
                // All 39 library shapes (stream 3); lines stay lines.
                let wdt = ui.available_width();
                let mut picked = None;
                w::popup_button(ui, "shape-kind", &crate::words::shape_name(s.shape), wdt, |ui| {
                    for &k in ShapeKind::ALL.iter().filter(|k| k.is_line() == s.shape.is_line()) {
                        if w::menu_check(ui, s.shape == k, &crate::words::shape_name(k), None).clicked() {
                            picked = Some(k);
                        }
                    }
                });
                if let Some(k) = picked {
                    insp.set(ElementPatch { shape: Some(k), ..Default::default() });
                }
            });
            let slots = [
                ("", crate::t!("inspector-swatch-none")),
                ("theme:shape1", crate::t!("inspector-swatch-colour-1")),
                ("theme:shape2", crate::t!("inspector-swatch-colour-2")),
                ("theme:shape3", crate::t!("inspector-swatch-colour-3")),
                ("theme:shape4", crate::t!("inspector-swatch-colour-4")),
                ("theme:accent", crate::t!("inspector-swatch-accent")),
                ("theme:frame", crate::t!("inspector-swatch-frame")),
                ("theme:title", crate::t!("inspector-swatch-title")),
                ("theme:background", crate::t!("inspector-swatch-background")),
            ];
            if !s.shape.is_line() {
                w::section(ui, &crate::t!("inspector-section-fill"), |ui| {
                    let cur = s.fill.clone().unwrap_or_default();
                    let sw = theme_swatches(insp, &slots);
                    if let Some(k) = colour_picker(ui, "fill", &cur, Color32::GRAY, &sw) {
                        insp.set(ElementPatch { fill: Some(k), ..Default::default() });
                    }
                });
            }
            w::section(ui, &if s.shape.is_line() { crate::t!("inspector-section-line") } else { crate::t!("inspector-section-border") }, |ui| {
                let cur = s.stroke.clone().unwrap_or_default();
                let sw = theme_swatches(insp, &slots);
                if let Some(k) = colour_picker(ui, "stroke", &cur, Color32::GRAY, &sw) {
                    insp.set(ElementPatch { stroke: Some(k), ..Default::default() });
                }
                let mut sw = s.stroke_width;
                w::row(ui, &crate::t!("inspector-width"), |ui| {
                    let r = w::number_field(ui, &mut sw, Num::pt().range(0.0, 200.0).step(0.5).decimals(1));
                    insp.track(&r);
                    if r.changed() {
                        insp.set(ElementPatch { stroke_width: Some(sw), ..Default::default() });
                    }
                });
            });
        }
        // Symbols show their description in their own section.
        ElementKind::Image(i) if i.symbol.is_none() => {
            w::section(ui, &crate::t!("inspector-section-image"), |ui| {
                let mut alt = i.alt.clone();
                w::row(ui, &crate::t!("inspector-description"), |ui| {
                    let r = w::text_field(ui, &mut alt, ui.available_width(), &crate::t!("inspector-alt-text-hint"));
                    insp.track(&r);
                    if r.changed() {
                        insp.set(ElementPatch { alt: Some(alt.clone()), ..Default::default() });
                    }
                });
                w::caption(ui, crate::t!("inspector-alt-text-caption"));
            });
        }
        ElementKind::Ink(ink) => {
            w::section(ui, &crate::t!("inspector-section-drawing"), |ui| {
                let cur = ink.color.clone();
                let slots = [
                    ("theme:title", crate::t!("inspector-swatch-title")),
                    ("theme:body", crate::t!("inspector-swatch-body")),
                    ("theme:accent", crate::t!("inspector-swatch-accent")),
                    ("theme:shape1", crate::t!("inspector-swatch-colour-1")),
                    ("theme:shape2", crate::t!("inspector-swatch-colour-2")),
                    ("theme:shape3", crate::t!("inspector-swatch-colour-3")),
                    ("theme:shape4", crate::t!("inspector-swatch-colour-4")),
                ];
                let sw = theme_swatches(insp, &slots);
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
        let para_name = |s: TextStyle| match s {
            TextStyle::Title => crate::t!("inspector-para-title"),
            TextStyle::Subtitle => crate::t!("inspector-para-subtitle"),
            TextStyle::Body => crate::t!("inspector-para-body"),
        };
        let label = para_name(t.style);
        let wdt = ui.available_width();
        w::popup_button(ui, "para-style", &label, wdt, |ui| {
            for s in [TextStyle::Title, TextStyle::Subtitle, TextStyle::Body] {
                if w::menu_check(ui, t.style == s, &para_name(s), None).clicked() {
                    insp.set(ElementPatch { style: Some(s), ..Default::default() });
                }
            }
        });
    });
    w::section(ui, &crate::t!("inspector-section-font"), |ui| {
        // The face the style is set in: "Inter Semibold (from the theme)".
        let st = insp.theme.text_style(t.style).clone();
        let family = zebra_render::text::family_for(&st.font).to_string();
        let face = crate::words::style_face_name(&insp.app.font_faces(&st.font), st.weight);
        w::caption(ui, crate::t!("inspector-font-from-theme", font = format!("{family} {face}")));
        let wdt = ui.available_width();
        let bold = crate::i18n::with_shortcut(crate::t!("inspector-bold"), &Action::Bold);
        let italic = crate::i18n::with_shortcut(crate::t!("inspector-italic"), &Action::Italic);
        // Bold is on for Bold or heavier, the style's weight included.
        let is_bold = zebra_core::rich::all_bold(&t, st.weight, 0, 0);
        if let Some(i) = w::segmented_toggles(ui, &[is_bold, t.italic], &[Seg::Icon(Icon::Bold, &bold), Seg::Icon(Icon::Italic, &italic)], Some(wdt * 0.5)) {
            match i {
                // Same as Format › Bold (one undo step, heavy styles too).
                0 => insp.app.queued.push(Action::Bold),
                _ => insp.set(ElementPatch { italic: Some(!t.italic), ..Default::default() }),
            }
        }
        let cur = t.color.clone().unwrap_or_default();
        let def_key = match t.style {
            TextStyle::Title => "theme:title",
            TextStyle::Subtitle => "theme:subtitle",
            TextStyle::Body => "theme:body",
        };
        let slots = [
            (def_key, crate::t!("inspector-swatch-theme-default")),
            ("theme:title", crate::t!("inspector-swatch-title")),
            ("theme:subtitle", crate::t!("inspector-swatch-subtitle")),
            ("theme:body", crate::t!("inspector-swatch-body")),
            ("theme:accent", crate::t!("inspector-swatch-accent")),
            ("theme:shape1", crate::t!("inspector-swatch-colour-1")),
            ("theme:shape2", crate::t!("inspector-swatch-colour-2")),
            ("theme:shape3", crate::t!("inspector-swatch-colour-3")),
            ("theme:shape4", crate::t!("inspector-swatch-colour-4")),
        ];
        let sw = theme_swatches(insp, &slots);
        let cur_key = if cur.is_empty() { def_key } else { cur.as_str() };
        let def = insp.color(def_key);
        if let Some(k) = colour_picker(ui, "text-colour", cur_key, def, &sw) {
            insp.set(ElementPatch { color: Some(if k == def_key { String::new() } else { k }), ..Default::default() });
        }
    });
    w::section(ui, &crate::t!("inspector-section-alignment"), |ui| {
        let mut a = t.align;
        let (left, centre, right) = (crate::t!("inspector-align-left"), crate::t!("inspector-align-centre"), crate::t!("inspector-align-right"));
        let items = [
            (zebra_core::Align::Left, Seg::Icon(Icon::AlignLeft, &left)),
            (zebra_core::Align::Center, Seg::Icon(Icon::AlignCenter, &centre)),
            (zebra_core::Align::Right, Seg::Icon(Icon::AlignRight, &right)),
        ];
        ui.horizontal(|ui| {
            let wdt = ui.available_width() * 0.55;
            if w::segmented(ui, &mut a, &items, Some(wdt)).changed() {
                insp.set(ElementPatch { align: Some(a), ..Default::default() });
            }
        });
        // What the paragraphs actually show (the list formatting from the
        // edit bar or an agent), not just the box-wide default: neither
        // segment is lit for numbered or mixed lists.
        let n = t.text.split('\n').count();
        let lists: Vec<ListKind> = (0..n).map(|i| zebra_core::rich::para(t, i).0).collect();
        let cur = if lists.iter().all(|l| *l == ListKind::Bullet) {
            Some(true)
        } else if lists.iter().all(|l| *l == ListKind::None) {
            Some(false)
        } else {
            None
        };
        let (none, with_bullets) = (crate::t!("inspector-bullets-none"), crate::t!("inspector-bullets-on"));
        w::row(ui, &crate::t!("inspector-bullets"), |ui| {
            let wdt = ui.available_width();
            let mut v = cur;
            if w::segmented(ui, &mut v, &[(Some(false), Seg::Text(&none)), (Some(true), Seg::Text(&with_bullets))], Some(wdt)).changed() {
                let list = if v == Some(true) { ListKind::Bullet } else { ListKind::None };
                let doc = &insp.app.editor.doc;
                let cmds: Vec<Command> = insp
                    .sel
                    .iter()
                    .filter(|id| doc.get(id).is_some_and(|e| matches!(e.kind, ElementKind::Text(_))))
                    .map(|id| Command::FormatParagraphs { id: id.clone(), first: None, last: None, list: Some(list), indent: None, indent_by: 0, align: None })
                    .collect();
                let r = insp.app.editor.apply_batch(cmds, Some(crate::t!("inspector-undo-format-paragraphs")), H);
                insp.app.report(r);
            }
        });
    });
    w::section(ui, "", |ui| {
        if w::push_button_ext(ui, &crate::t!("inspector-edit-text"), w::Size::Regular, !el.locked, Some(ui.available_width())).on_hover_text(crate::t!("inspector-edit-text-tip", key = "↩")).clicked() {
            crate::canvas::begin_edit_existing(insp.app, el.id.clone());
        }
    });
}

// ----------------------------------------------------------------------
// Arrange tab

fn arrange_tab(insp: &mut Insp, ui: &mut Ui, _p: &Palette) {
    let el = insp.el.clone();
    let tr = el.transform;
    w::section(ui, &crate::t!("inspector-section-size"), |ui| {
        // W × H as the audience sees it (local size × scale).
        let ww = el.size.w * tr.scale;
        let hh = el.size.h * tr.scale;
        let mut scale = tr.scale;
        w::row(ui, &crate::t!("inspector-width"), |ui| {
            let mut v = ww;
            let r = w::number_field(ui, &mut v, Num { decimals: 0, min: 1e-6, ..Default::default() }.width(72.0));
            insp.track(&r);
            if r.changed() && ww > 0.0 {
                scale = tr.scale * v / ww;
                insp.set(ElementPatch { scale: Some(scale), ..Default::default() });
            }
        });
        w::row(ui, &crate::t!("inspector-height"), |ui| {
            let mut v = hh;
            let r = w::number_field(ui, &mut v, Num { decimals: 0, min: 1e-6, ..Default::default() }.width(72.0));
            insp.track(&r);
            if r.changed() && hh > 0.0 {
                scale = tr.scale * v / hh;
                insp.set(ElementPatch { scale: Some(scale), ..Default::default() });
            }
        });
        w::caption(ui, crate::t!("inspector-size-caption"));
    });
    w::section(ui, &crate::t!("inspector-section-rotate"), |ui| {
        let mut rot = tr.rotation;
        w::row(ui, &crate::t!("inspector-angle"), |ui| {
            let r = w::number_field(ui, &mut rot, Num::deg().step(1.0).width(72.0));
            insp.track(&r);
            if r.changed() {
                insp.set(ElementPatch { rotation: Some(rot), ..Default::default() });
            }
        });
    });
    w::disclosure_section(ui, "position", &crate::t!("inspector-section-position"), false, |ui| {
        let (mut x, mut y) = (tr.x, tr.y);
        w::row(ui, &crate::t!("inspector-pos-x"), |ui| {
            let r = w::number_field(ui, &mut x, Num { decimals: 0, ..Default::default() }.width(72.0));
            insp.track(&r);
            if r.changed() {
                insp.set(ElementPatch { x: Some(x), ..Default::default() });
            }
        });
        w::row(ui, &crate::t!("inspector-pos-y"), |ui| {
            let r = w::number_field(ui, &mut y, Num { decimals: 0, ..Default::default() }.width(72.0));
            insp.track(&r);
            if r.changed() {
                insp.set(ElementPatch { y: Some(y), ..Default::default() });
            }
        });
        w::caption(ui, crate::t!("inspector-position-caption"));
    });
    arrange_common(insp, ui);
}

fn arrange_common(insp: &mut Insp, ui: &mut Ui) {
    let sel = insp.sel.clone();
    let doc = &insp.app.editor.doc;
    let locked = !sel.is_empty() && sel.iter().all(|id| doc.get(id).is_some_and(|e| e.locked));
    w::section(ui, &crate::t!("inspector-section-layer"), |ui| {
        let b = [
            (crate::t!("inspector-forward"), tip(crate::t!("inspector-forward-tip"), Action::BringForward), !locked),
            (crate::t!("inspector-backward"), tip(crate::t!("inspector-backward-tip"), Action::SendBackward), !locked),
            (crate::t!("inspector-to-front"), tip(crate::t!("inspector-to-front-tip"), Action::BringToFront), !locked),
            (crate::t!("inspector-to-back"), tip(crate::t!("inspector-to-back-tip"), Action::SendToBack), !locked),
        ];
        match button_grid(ui, &b, 2) {
            Some(0) => insp.app.queued.push(Action::BringForward),
            Some(1) => insp.app.queued.push(Action::SendBackward),
            Some(2) => insp.app.queued.push(Action::BringToFront),
            Some(3) => insp.app.queued.push(Action::SendToBack),
            _ => {}
        }
    });
    w::section(ui, "", |ui| {
        let grouped = sel.iter().any(|id| insp.app.editor.doc.get(id).is_some_and(|e| e.group.is_some()));
        let mut b = vec![
            if locked {
                (crate::t!("inspector-unlock"), tip(crate::t!("inspector-unlock-tip"), Action::ToggleLock), true)
            } else {
                (crate::t!("inspector-lock"), tip(crate::t!("inspector-lock-tip"), Action::ToggleLock), true)
            },
            (crate::t!("inspector-duplicate"), tip(crate::t!("inspector-duplicate-tip"), Action::Duplicate), true),
        ];
        if sel.len() > 1 {
            b.push((crate::t!("inspector-group"), tip(crate::t!("inspector-group-tip"), Action::Group), !locked && !one_group(insp)));
            b.push((crate::t!("inspector-ungroup"), tip(crate::t!("inspector-ungroup-tip"), Action::Ungroup), !locked && grouped));
        }
        match button_grid(ui, &b, 2) {
            Some(0) => insp.app.queued.push(Action::ToggleLock),
            Some(1) => insp.app.queued.push(Action::Duplicate),
            Some(2) => insp.app.queued.push(Action::Group),
            Some(3) => insp.app.queued.push(Action::Ungroup),
            _ => {}
        }
        if w::push_button_ext(ui, &crate::t!("inspector-frame-selection"), w::Size::Regular, true, Some(ui.available_width())).on_hover_text(tip(crate::t!("inspector-frame-selection-tip"), Action::WrapInFrame)).clicked() {
            insp.app.queued.push(Action::WrapInFrame);
        }
    });
}

fn arrange_multi(insp: &mut Insp, ui: &mut Ui) {
    use zebra_core::command::{AlignEdge, Layout};
    let sel = insp.sel.clone();
    w::section(ui, &crate::t!("inspector-section-align"), |ui| {
        let b = [
            (crate::t!("inspector-align-edge-left"), String::new(), true),
            (crate::t!("inspector-align-edge-centre"), String::new(), true),
            (crate::t!("inspector-align-edge-right"), String::new(), true),
            (crate::t!("inspector-align-edge-top"), String::new(), true),
            (crate::t!("inspector-align-edge-middle"), String::new(), true),
            (crate::t!("inspector-align-edge-bottom"), String::new(), true),
        ];
        let edges = [AlignEdge::Left, AlignEdge::CenterX, AlignEdge::Right, AlignEdge::Top, AlignEdge::CenterY, AlignEdge::Bottom];
        if let Some(i) = button_grid(ui, &b, 3) {
            insp.app.apply(Command::Align { ids: sel.clone(), edge: edges[i] });
        }
    });
    w::section(ui, &crate::t!("inspector-section-arrange"), |ui| {
        let b = [
            (crate::t!("inspector-arrange-row"), crate::t!("inspector-arrange-row-tip"), true),
            (crate::t!("inspector-arrange-column"), crate::t!("inspector-arrange-column-tip"), true),
            (crate::t!("inspector-arrange-grid"), crate::t!("inspector-arrange-grid-tip"), true),
            (crate::t!("inspector-arrange-circle"), crate::t!("inspector-arrange-circle-tip"), true),
        ];
        let lay = [Layout::Row, Layout::Column, Layout::Grid, Layout::Circle];
        if let Some(i) = button_grid(ui, &b, 2) {
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
    w::section(ui, &crate::t!("inspector-section-path"), |ui| {
        if steps.is_empty() {
            w::caption(ui, if sel.len() > 1 { crate::t!("inspector-not-on-path-multi") } else { crate::t!("inspector-not-on-path") });
            if w::push_button_ext(ui, &crate::t!("inspector-add-to-path"), w::Size::Regular, true, Some(ui.available_width())).on_hover_text(tip(crate::t!("inspector-add-to-path-tip"), Action::AddToPath)).clicked() {
                insp.app.queued.push(Action::AddToPath);
            }
        } else {
            let list = steps.iter().map(|s| (s + 1).to_string()).collect::<Vec<_>>().join(", ");
            w::caption(ui, crate::t!("inspector-on-path-steps", count = steps.len(), list = list.as_str()));
            if w::push_button_ext(ui, &crate::t!("inspector-remove-from-path"), w::Size::Regular, true, Some(ui.available_width())).on_hover_text(crate::t!("inspector-remove-from-path-tip")).clicked() {
                let cmds: Vec<Command> = steps.iter().rev().map(|i| Command::RemoveStep { index: *i }).collect();
                let r = insp.app.editor.apply_batch(cmds, Some(crate::t!("inspector-undo-remove-from-path")), H);
                insp.app.report(r);
            }
        }
    });

    // Fade-in builds within a step.
    if sel.len() == 1 {
        let id = insp.el.id.clone();
        let doc = &insp.app.editor.doc;
        let fade_steps: Vec<usize> = (0..doc.path.len())
            .filter(|i| match &doc.path[*i].target {
                StepTarget::Element { id: f } => *f != id && insp.app.editor.frame_contents(f).contains(&id),
                _ => false,
            })
            .collect();
        if let Some(&si) = fade_steps.last() {
            let builds = doc.path[si].builds.clone();
            w::section(ui, &crate::t!("inspector-section-animation"), |ui| {
                let mut on = builds.contains(&id);
                if w::checkbox(ui, &mut on, &crate::t!("inspector-fade-in-on-step", step = (si + 1))).on_hover_text(crate::t!("inspector-fade-in-tip")).changed() {
                    let mut b = builds;
                    if on {
                        b.push(id.clone());
                    } else {
                        b.retain(|x| *x != id);
                    }
                    insp.app.apply(Command::UpdateStep { index: si, target: None, builds: Some(b), notes: None, duration: None, dwell: None, hide_builds: None });
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
        ui.label(RichText::new(crate::t!("inspector-presentation")).font(style::semibold(13.0)).color(p.text));
        w::section(ui, "", |ui| {
            let mut title = app.title_buf.clone().unwrap_or_else(|| app.editor.doc.title.clone());
            w::row(ui, &crate::t!("inspector-doc-title"), |ui| {
                let r = w::text_field(ui, &mut title, ui.available_width(), &crate::t!("inspector-untitled"));
                app.title_seen = ui.ctx().cumulative_pass_nr();
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
        w::section(ui, &crate::t!("inspector-section-theme"), |ui| {
            let cur = app.editor.doc.theme.clone();
            let name = crate::words::theme_name(&app.editor.theme());
            let wdt = ui.available_width();
            w::popup_button(ui, "theme", &name, wdt, |ui| {
                for t in Theme::builtins() {
                    if w::menu_check(ui, t.id == cur, &crate::words::theme_name(&t), None).clicked() {
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
        w::section(ui, &crate::t!("inspector-section-path"), |ui| {
            let n = app.editor.doc.path.len();
            w::caption(ui, crate::t!("inspector-path-steps", count = n));
            let b = [
                (crate::t!("inspector-edit-path"), tip(crate::t!("inspector-edit-path-tip"), Action::TogglePathEdit), true),
                (crate::t!("inspector-from-frames"), crate::t!("inspector-from-frames-tip"), true),
            ];
            match button_grid(ui, &b, 2) {
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
