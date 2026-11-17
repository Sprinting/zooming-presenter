//! Inspector sections for stream 3's objects: shape corners and line
//! style, icons, and connectors.
use egui::{Color32, Ui};
use zebra_core::{Arrowhead, Dash, ElementKind, ElementPatch, Route, ShapeKind};

use crate::inspector::{colour_picker, theme_swatches, Insp, Tab};
use crate::widgets::{self as w, Num, Seg};

const DASHES: [(Dash, Seg); 3] = [(Dash::Solid, Seg::Text("Solid")), (Dash::Dashed, Seg::Text("Dashed")), (Dash::Dotted, Seg::Text("Dotted"))];

pub fn sections(insp: &mut Insp, ui: &mut Ui, tab: Tab) {
    if tab != Tab::Style {
        return;
    }
    match insp.el.kind.clone() {
        ElementKind::Shape(s) => {
            w::section(ui, "Outline", |ui| {
                if matches!(s.shape, ShapeKind::Rectangle | ShapeKind::RoundedRect | ShapeKind::Callout | ShapeKind::SpeechBubble | ShapeKind::StickyNote) {
                    let mut cr = s.corner_radius;
                    let max = insp.el.size.w.min(insp.el.size.h) / 2.0;
                    w::row(ui, "Corners", |ui| {
                        let r = w::number_field(ui, &mut cr, Num::pt().range(0.0, max).step(1.0).decimals(0));
                        insp.track(&r);
                        if r.changed() {
                            insp.set(ElementPatch { corner_radius: Some(cr), ..Default::default() });
                        }
                    });
                }
                let mut d = s.dash;
                let wdt = ui.available_width();
                if w::segmented(ui, &mut d, &DASHES, Some(wdt)).changed() {
                    insp.set(ElementPatch { dash: Some(d), ..Default::default() });
                }
            });
        }
        ElementKind::Icon(i) => {
            w::section(ui, "Icon", |ui| {
                w::row(ui, "", |ui| {
                    if w::push_button(ui, "Replace…").on_hover_text("Pick another icon from the library").clicked() {
                        let id = insp.el.id.clone();
                        insp.app.library.open_replace(id);
                    }
                });
                let mut sw = i.stroke_width;
                w::row(ui, "Weight", |ui| {
                    let r = w::number_field(ui, &mut sw, Num::pt().range(0.5, 4.0).step(0.25).decimals(2));
                    insp.track(&r);
                    if r.changed() {
                        insp.set(ElementPatch { stroke_width: Some(sw), ..Default::default() });
                    }
                });
            });
            w::section(ui, "Colour", |ui| {
                let slots = [("theme:body", "Text"), ("theme:title", "Title"), ("theme:accent", "Accent"), ("theme:shape1", "Colour 1"), ("theme:shape2", "Colour 2"), ("theme:shape3", "Colour 3"), ("theme:shape4", "Colour 4"), ("theme:background", "Background")];
                let sw = theme_swatches(insp, &slots);
                if let Some(k) = colour_picker(ui, "icon-colour", &i.color, Color32::GRAY, &sw) {
                    insp.set(ElementPatch { color: Some(k), ..Default::default() });
                }
            });
        }
        ElementKind::Connector(c) => {
            w::section(ui, "Connector", |ui| {
                let mut r = c.route;
                let wdt = ui.available_width();
                let routes = [(Route::Straight, Seg::Text("Straight")), (Route::Elbow, Seg::Text("Elbow")), (Route::Curved, Seg::Text("Curved"))];
                if w::segmented(ui, &mut r, &routes, Some(wdt)).changed() {
                    insp.set(ElementPatch { route: Some(r), ..Default::default() });
                }
                let heads = [(Arrowhead::None, "None"), (Arrowhead::Arrow, "Arrow"), (Arrowhead::Open, "Open Arrow"), (Arrowhead::Dot, "Dot")];
                for (label, cur, is_end) in [("Start", c.start, false), ("End", c.end, true)] {
                    let mut picked = None;
                    w::row(ui, label, |ui| {
                        let name = heads.iter().find(|h| h.0 == cur).map(|h| h.1).unwrap_or("None");
                        let wdt = ui.available_width();
                        w::popup_button(ui, ("head", is_end), name, wdt, |ui| {
                            for (h, l) in heads {
                                if ui.selectable_label(cur == h, l).clicked() {
                                    picked = Some(h);
                                }
                            }
                        });
                    });
                    if let Some(h) = picked {
                        insp.set(if is_end { ElementPatch { end_head: Some(h), ..Default::default() } } else { ElementPatch { start_head: Some(h), ..Default::default() } });
                    }
                }
                let mut label = c.label.clone();
                w::row(ui, "Label", |ui| {
                    let r = w::text_field(ui, &mut label, ui.available_width(), "Text on the line");
                    insp.track(&r);
                    if r.changed() {
                        insp.set(ElementPatch { label: Some(label.clone()), ..Default::default() });
                    }
                });
                let attached = [&c.from, &c.to].iter().filter(|e| e.element.is_some()).count();
                w::caption(ui, format!("{attached} of 2 ends attached. Drag an end onto an object to stick it there."));
            });
            w::section(ui, "Line", |ui| {
                let slots = [("theme:body", "Text"), ("theme:title", "Title"), ("theme:accent", "Accent"), ("theme:shape1", "Colour 1"), ("theme:shape2", "Colour 2"), ("theme:shape3", "Colour 3"), ("theme:shape4", "Colour 4"), ("theme:frame", "Frame")];
                let sw = theme_swatches(insp, &slots);
                if let Some(k) = colour_picker(ui, "connector-colour", &c.color, Color32::GRAY, &sw) {
                    insp.set(ElementPatch { color: Some(k), ..Default::default() });
                }
                let mut wd = c.width;
                w::row(ui, "Width", |ui| {
                    let r = w::number_field(ui, &mut wd, Num::pt().range(0.1, 200.0).step(0.5).decimals(1));
                    insp.track(&r);
                    if r.changed() {
                        insp.set(ElementPatch { stroke_width: Some(wd), ..Default::default() });
                    }
                });
                let mut d = c.dash;
                let wdt = ui.available_width();
                if w::segmented(ui, &mut d, &DASHES, Some(wdt)).changed() {
                    insp.set(ElementPatch { dash: Some(d), ..Default::default() });
                }
            });
        }
        _ => {}
    }
}

/// Stream 4's speaker notes and timings for the step this element is, on
/// the Path tab.
pub fn path_step(insp: &mut Insp, ui: &mut Ui, tab: Tab) {
    if tab != Tab::Path || insp.sel.len() != 1 {
        return;
    }
    let id = insp.el.id.clone();
    let step = insp.app.editor.doc.path.iter().position(|s| matches!(&s.target, zebra_core::StepTarget::Element { id: t } if *t == id));
    if let Some(step) = step {
        let p = crate::style::Palette::of(ui.ctx());
        crate::presenting::step_section(insp.app, ui, &p, step);
        crate::media_ui::step_section(insp.app, ui, step);
    }
}

/// Stream 2's image (crop, mask, replace) and video (playback, trim,
/// poster) sections on the Style tab.
pub fn media(insp: &mut Insp, ui: &mut Ui, tab: Tab) {
    if tab != Tab::Style || insp.sel.len() != 1 {
        return;
    }
    let id = insp.el.id.clone();
    match insp.el.kind.clone() {
        ElementKind::Image(i) => {
            if let Some(p) = crate::media_ui::image_section(insp.app, ui, &id, &i) {
                insp.set(p);
            }
        }
        ElementKind::Video(v) => {
            w::section(ui, "Video", |ui| {
                let (p, dragging) = crate::media_ui::video_section(insp.app, ui, &id, &v);
                insp.gesture |= dragging;
                if let Some(p) = p {
                    insp.set(p);
                }
            });
        }
        _ => {}
    }
}
