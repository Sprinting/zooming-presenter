//! Inspector sections for stream 3's objects: shape corners and line
//! style, icons, and connectors.
use egui::{Color32, Ui};
use zebra_core::{Arrowhead, Dash, ElementKind, ElementPatch, Route, ShapeKind};

use crate::inspector::{colour_picker, theme_swatches, Insp, Tab};
use crate::widgets::{self as w, Num, Seg};

/// Line style names, in the order of [`DASHES`].
fn dash_labels() -> [String; 3] {
    [crate::t!("inspector-dash-solid"), crate::t!("inspector-dash-dashed"), crate::t!("inspector-dash-dotted")]
}

const DASHES: [Dash; 3] = [Dash::Solid, Dash::Dashed, Dash::Dotted];

fn dash_segments(labels: &[String; 3]) -> [(Dash, Seg<'_>); 3] {
    [(DASHES[0], Seg::Text(&labels[0])), (DASHES[1], Seg::Text(&labels[1])), (DASHES[2], Seg::Text(&labels[2]))]
}

/// Swatches for icons and connectors: text, title, accent, colours 1 to 4,
/// then `last` (a theme key and its translated name).
fn swatch_slots(last: (&'static str, String)) -> [(&'static str, String); 8] {
    [
        ("theme:body", crate::t!("inspector-swatch-text")),
        ("theme:title", crate::t!("inspector-swatch-title")),
        ("theme:accent", crate::t!("inspector-swatch-accent")),
        ("theme:shape1", crate::t!("inspector-swatch-colour-1")),
        ("theme:shape2", crate::t!("inspector-swatch-colour-2")),
        ("theme:shape3", crate::t!("inspector-swatch-colour-3")),
        ("theme:shape4", crate::t!("inspector-swatch-colour-4")),
        last,
    ]
}

pub fn sections(insp: &mut Insp, ui: &mut Ui, tab: Tab) {
    if tab != Tab::Style {
        return;
    }
    match insp.el.kind.clone() {
        ElementKind::Shape(s) => {
            w::section(ui, &crate::t!("inspector-section-outline"), |ui| {
                if matches!(s.shape, ShapeKind::Rectangle | ShapeKind::RoundedRect | ShapeKind::Callout | ShapeKind::SpeechBubble | ShapeKind::StickyNote) {
                    let mut cr = s.corner_radius;
                    let max = insp.el.size.w.min(insp.el.size.h) / 2.0;
                    w::row(ui, &crate::t!("inspector-corners"), |ui| {
                        let r = w::number_field(ui, &mut cr, Num::pt().range(0.0, max).step(1.0).decimals(0));
                        insp.track(&r);
                        if r.changed() {
                            insp.set(ElementPatch { corner_radius: Some(cr), ..Default::default() });
                        }
                    });
                }
                let mut d = s.dash;
                let wdt = ui.available_width();
                let labels = dash_labels();
                if w::segmented(ui, &mut d, &dash_segments(&labels), Some(wdt)).changed() {
                    insp.set(ElementPatch { dash: Some(d), ..Default::default() });
                }
            });
        }
        ElementKind::Icon(i) => {
            w::section(ui, &crate::t!("inspector-section-icon"), |ui| {
                w::row(ui, "", |ui| {
                    if w::push_button(ui, &crate::t!("inspector-replace-icon")).on_hover_text(crate::t!("inspector-replace-icon-tip")).clicked() {
                        let id = insp.el.id.clone();
                        insp.app.library.open_replace(id);
                    }
                });
                let mut sw = i.stroke_width;
                w::row(ui, &crate::t!("inspector-weight"), |ui| {
                    let r = w::number_field(ui, &mut sw, Num::pt().range(0.5, 4.0).step(0.25).decimals(2));
                    insp.track(&r);
                    if r.changed() {
                        insp.set(ElementPatch { stroke_width: Some(sw), ..Default::default() });
                    }
                });
            });
            w::section(ui, &crate::t!("inspector-section-colour"), |ui| {
                let slots = swatch_slots(("theme:background", crate::t!("inspector-swatch-background")));
                let sw = theme_swatches(insp, &slots);
                if let Some(k) = colour_picker(ui, "icon-colour", &i.color, Color32::GRAY, &sw) {
                    insp.set(ElementPatch { color: Some(k), ..Default::default() });
                }
            });
        }
        ElementKind::Connector(c) => {
            w::section(ui, &crate::t!("inspector-section-route"), |ui| {
                let mut r = c.route;
                let wdt = ui.available_width();
                let (straight, elbow, curved) = (crate::t!("inspector-route-straight"), crate::t!("inspector-route-elbow"), crate::t!("inspector-route-curved"));
                let routes = [(Route::Straight, Seg::Text(&straight)), (Route::Elbow, Seg::Text(&elbow)), (Route::Curved, Seg::Text(&curved))];
                if w::segmented(ui, &mut r, &routes, Some(wdt)).changed() {
                    insp.set(ElementPatch { route: Some(r), ..Default::default() });
                }
                let none = crate::t!("inspector-head-none");
                let heads = [(Arrowhead::None, none.clone()), (Arrowhead::Arrow, crate::t!("inspector-head-arrow")), (Arrowhead::Open, crate::t!("inspector-head-open-arrow")), (Arrowhead::Dot, crate::t!("inspector-head-dot"))];
                for (label, cur, is_end) in [(crate::t!("inspector-head-start"), c.start, false), (crate::t!("inspector-head-end"), c.end, true)] {
                    let mut picked = None;
                    w::row(ui, &label, |ui| {
                        let name = heads.iter().find(|h| h.0 == cur).map(|h| h.1.as_str()).unwrap_or(none.as_str());
                        let wdt = ui.available_width();
                        w::popup_button(ui, ("head", is_end), name, wdt, |ui| {
                            for (h, l) in &heads {
                                let h = *h;
                                if ui.selectable_label(cur == h, l.as_str()).clicked() {
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
                w::row(ui, &crate::t!("inspector-connector-label"), |ui| {
                    let r = w::text_field(ui, &mut label, ui.available_width(), &crate::t!("inspector-connector-label-hint"));
                    insp.track(&r);
                    if r.changed() {
                        insp.set(ElementPatch { label: Some(label.clone()), ..Default::default() });
                    }
                });
                let attached = [&c.from, &c.to].iter().filter(|e| e.element.is_some()).count();
                w::caption(ui, crate::t!("inspector-connector-attached", attached = attached));
            });
            w::section(ui, &crate::t!("inspector-section-line"), |ui| {
                let slots = swatch_slots(("theme:frame", crate::t!("inspector-swatch-frame")));
                let sw = theme_swatches(insp, &slots);
                if let Some(k) = colour_picker(ui, "connector-colour", &c.color, Color32::GRAY, &sw) {
                    insp.set(ElementPatch { color: Some(k), ..Default::default() });
                }
                let mut wd = c.width;
                w::row(ui, &crate::t!("inspector-width"), |ui| {
                    let r = w::number_field(ui, &mut wd, Num::pt().range(0.1, 200.0).step(0.5).decimals(1));
                    insp.track(&r);
                    if r.changed() {
                        insp.set(ElementPatch { stroke_width: Some(wd), ..Default::default() });
                    }
                });
                let mut d = c.dash;
                let wdt = ui.available_width();
                let labels = dash_labels();
                if w::segmented(ui, &mut d, &dash_segments(&labels), Some(wdt)).changed() {
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
        ElementKind::Image(zebra_core::ImageData { symbol: Some(look), .. }) => crate::symbols_ui::inspector(insp, ui, &look),
        ElementKind::Image(i) => {
            if let Some(p) = crate::media_ui::image_section(insp.app, ui, &id, &i) {
                insp.set(p);
            }
        }
        ElementKind::Video(v) => {
            // No heading of its own: the inspector's title already says Video.
            let (p, dragging) = crate::media_ui::video_section(insp.app, ui, &id, &v);
            insp.gesture |= dragging;
            if let Some(p) = p {
                insp.set(p);
            }
        }
        _ => {}
    }
}
