//! Mac widget kit: AppKit-faithful controls drawn in egui.
//!
//! Kartik's rule: no AppKit controls inside the egui UI. These widgets copy
//! the classic (Big Sur to Sonoma) metrics by hand instead:
//! - regular controls 22 pt high, small 19 pt, mini 16 pt;
//! - 5 pt corner radius on bezels, 6 pt on menus and popovers;
//! - 13 pt text on controls, 11 pt for small labels, 10 pt toolbar labels;
//! - the system accent for default buttons, checkboxes, switches and focus;
//! - a white raised bezel with a hairline edge and a 1 px drop line in
//!   light mode; a flat grey plate in dark mode.
//!
//! Everything takes its colours from [`Palette`], so light and dark both
//! work. Icons come from [`crate::icons`].
//!
//! # Inspector sections (for other streams)
//!
//! ```ignore
//! use crate::widgets as w;
//! w::section(ui, "Video", |ui| {
//!     w::row(ui, "Start", |ui| { w::number_field(ui, &mut start, w::Num::secs()); });
//!     w::row(ui, "", |ui| { w::checkbox(ui, &mut looped, "Loop"); });
//! });
//! ```
//! See `inspector.rs` for how a section is registered for an element kind.
use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, WidgetInfo,
    Role,
};

use crate::icons::{paint_icon, Icon};
use crate::style::{self, accent, Palette};

/// Control heights (AppKit regular / small / mini).
pub const H_REGULAR: f32 = 22.0;
pub const H_SMALL: f32 = 19.0;
pub const H_MINI: f32 = 16.0;
/// Bezel radius.
pub const RADIUS: f32 = 5.0;
/// Width of the label column in inspector rows.
pub const LABEL_W: f32 = 74.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Size {
    #[default]
    Regular,
    Small,
    Mini,
}

impl Size {
    pub fn height(self) -> f32 {
        match self {
            Size::Regular => H_REGULAR,
            Size::Small => H_SMALL,
            Size::Mini => H_MINI,
        }
    }
    pub fn font(self) -> FontId {
        style::ui_font(match self {
            Size::Regular => 13.0,
            Size::Small => 11.0,
            Size::Mini => 10.0,
        })
    }
}

fn pal(ui: &Ui) -> Palette {
    Palette::for_dark(ui.visuals().dark_mode)
}

/// Text colour that reads on top of the accent (white, except on a very
/// light accent such as Yellow, where AppKit also uses dark text).
pub fn on_accent() -> Color32 {
    let a = accent();
    let l = 0.2126 * a.r() as f32 + 0.7152 * a.g() as f32 + 0.0722 * a.b() as f32;
    if l > 200.0 { Color32::from_gray(0x20) } else { Color32::WHITE }
}

/// The raised bezel shared by push buttons, popups, segmented controls and
/// steppers.
pub fn paint_bezel(ui: &Ui, rect: Rect, pressed: bool, enabled: bool) {
    let p = pal(ui);
    let painter = ui.painter();
    let fill = if pressed { p.control_pressed } else { p.control };
    if !p.dark {
        // 1 px drop line under the bezel (AppKit's subtle shadow).
        painter.rect_filled(rect.translate(Vec2::new(0.0, 0.5)), RADIUS, Color32::from_black_alpha(if enabled { 22 } else { 10 }));
    }
    let fill = if enabled { fill } else { fill.gamma_multiply(if p.dark { 0.6 } else { 1.0 }) };
    painter.rect(rect, RADIUS, fill, Stroke::new(if p.dark { 0.0 } else { 0.6 }, p.control_edge), StrokeKind::Inside);
    if p.dark {
        // top highlight edge in dark mode
        painter.line_segment([rect.left_top() + Vec2::new(RADIUS, 0.5), rect.right_top() + Vec2::new(-RADIUS, 0.5)], Stroke::new(0.6, Color32::from_white_alpha(20)));
    }
}

/// The accent focus ring (3 pt, translucent), drawn around a focused control.
pub fn focus_ring(ui: &Ui, rect: Rect, radius: f32) {
    let c = accent().gamma_multiply(0.55);
    ui.painter().rect_stroke(rect.expand(1.5), radius + 1.5, Stroke::new(3.0, c), StrokeKind::Middle);
}

fn text_color(ui: &Ui, enabled: bool) -> Color32 {
    let p = pal(ui);
    if enabled { p.text } else { p.disabled }
}

// ----------------------------------------------------------------------
// Push buttons

/// A bordered push button (NSButton, `.rounded` bezel), regular size.
pub fn push_button(ui: &mut Ui, text: &str) -> Response {
    button_impl(ui, text, None, Size::Regular, false, true, None)
}

/// Push button with options: size, enabled, minimum width.
pub fn push_button_ext(ui: &mut Ui, text: &str, size: Size, enabled: bool, min_w: Option<f32>) -> Response {
    button_impl(ui, text, None, size, false, enabled, min_w)
}

/// The default button (accent-filled, Return activates it in dialogs).
pub fn default_button(ui: &mut Ui, text: &str) -> Response {
    button_impl(ui, text, None, Size::Regular, true, true, None)
}

/// The default button with a minimum width (dialog button rows use 80 pt).
pub fn default_button_w(ui: &mut Ui, text: &str, min_w: f32) -> Response {
    button_impl(ui, text, None, Size::Regular, true, true, Some(min_w))
}

/// The default button, dimmed when `enabled` is false (a sheet's Insert
/// until its field holds something it can take).
pub fn default_button_ext(ui: &mut Ui, text: &str, enabled: bool, min_w: Option<f32>) -> Response {
    button_impl(ui, text, None, Size::Regular, true, enabled, min_w)
}

/// Push button with a leading icon.
pub fn icon_text_button(ui: &mut Ui, icon: Icon, text: &str, enabled: bool) -> Response {
    button_impl(ui, text, Some(icon), Size::Regular, false, enabled, None)
}

fn button_impl(ui: &mut Ui, text: &str, icon: Option<Icon>, size: Size, default: bool, enabled: bool, min_w: Option<f32>) -> Response {
    let font = size.font();
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER);
    let pad = if size == Size::Regular { 12.0 } else { 9.0 };
    let icon_w = if icon.is_some() { size.height() - 6.0 + 4.0 } else { 0.0 };
    let w = (galley.size().x + pad * 2.0 + icon_w).max(min_w.unwrap_or(if size == Size::Regular && !text.is_empty() { 60.0 } else { 0.0 }));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, size.height()), if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, text));
    if ui.is_rect_visible(rect) {
        let pressed = enabled && resp.is_pointer_button_down_on();
        let (tc, ic);
        if default && enabled {
            let a = accent();
            let fill = if pressed { a.lerp_to_gamma(Color32::BLACK, 0.15) } else { a };
            ui.painter().rect_filled(rect.translate(Vec2::new(0.0, 0.5)), RADIUS, Color32::from_black_alpha(25));
            ui.painter().rect_filled(rect, RADIUS, fill);
            tc = on_accent();
            ic = tc;
        } else {
            paint_bezel(ui, rect, pressed, enabled);
            tc = text_color(ui, enabled);
            ic = tc;
        }
        let mut x = rect.center().x - (galley.size().x + icon_w) / 2.0;
        if let Some(icon) = icon {
            let s = size.height() - 6.0;
            paint_icon(ui.painter(), Rect::from_min_size(Pos2::new(x, rect.center().y - s / 2.0), Vec2::splat(s)), icon, ic);
            x += icon_w;
        }
        ui.painter().galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, tc);
        if resp.has_focus() {
            focus_ring(ui, rect, RADIUS);
        }
    }
    resp
}

/// A borderless icon button (toolbar-style "recessed" or inline glyph
/// buttons): no bezel at rest, a soft plate on hover and press. `label`
/// names it for VoiceOver (translated: usually its tooltip).
pub fn icon_button(ui: &mut Ui, icon: Icon, size: f32, enabled: bool, active: bool, label: &str) -> Response {
    let p = pal(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if active {
            painter.rect_filled(rect, RADIUS, p.pressed);
        } else if enabled && resp.is_pointer_button_down_on() {
            painter.rect_filled(rect, RADIUS, p.pressed);
        } else if enabled && resp.hovered() {
            painter.rect_filled(rect, RADIUS, p.hover);
        }
        let c = if !enabled { p.disabled } else if active { accent() } else { p.text };
        let pad = (size * 0.2).round();
        paint_icon(painter, rect.shrink(pad), icon, c);
    }
    resp
}

// ----------------------------------------------------------------------
// Toolbar item: icon over a 10 pt label (Keynote's unified toolbar).

#[derive(Clone, Copy, Default)]
pub struct ToolbarState {
    pub active: bool,
    pub enabled: bool,
    /// Draws a small ▾ after the label: the item opens a menu.
    pub menu: bool,
    /// Tints the glyph (Play uses the accent; the agent item uses violet).
    pub tint: Option<Color32>,
}

/// Toolbar item width: at least 44 pt, or the label width plus padding.
pub fn toolbar_item(ui: &mut Ui, icon: Icon, label: &str, st: ToolbarState) -> Response {
    let p = pal(ui);
    let font = style::ui_font(11.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
    let caret = if st.menu { 8.0 } else { 0.0 };
    let w = (galley.size().x + caret + 10.0).max(44.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 44.0), if st.enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, st.enabled, label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        // Glyph plate: 30×24 rounded, like the NSToolbar "button" bezel on hover.
        let plate = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 14.0), Vec2::new(34.0, 26.0));
        if st.active {
            painter.rect_filled(plate, 6.0, p.pressed);
        } else if st.enabled && resp.is_pointer_button_down_on() {
            painter.rect_filled(plate, 6.0, p.pressed);
        } else if st.enabled && resp.hovered() {
            painter.rect_filled(plate, 6.0, p.hover);
        }
        let gc = if !st.enabled {
            p.disabled
        } else if let Some(t) = st.tint {
            t
        } else if st.active {
            accent()
        } else {
            p.text
        };
        paint_icon(painter, Rect::from_center_size(plate.center(), Vec2::splat(19.0)), icon, gc);
        let lc = if st.enabled { p.text } else { p.disabled };
        let lx = rect.center().x - (galley.size().x + caret) / 2.0;
        let ly = rect.bottom() - galley.size().y - 1.0;
        painter.galley(Pos2::new(lx, ly), galley.clone(), lc);
        if st.menu {
            let c = Pos2::new(lx + galley.size().x + 5.0, ly + galley.size().y / 2.0 + 0.5);
            painter.add(Shape::convex_polygon(vec![c + Vec2::new(-2.5, -1.3), c + Vec2::new(2.5, -1.3), c + Vec2::new(0.0, 1.7)], lc, Stroke::NONE));
        }
    }
    resp
}

/// A borderless button for floating bars (the text format bar): 24 pt
/// high, 7 pt side padding, a soft plate on hover and a pressed plate with
/// accent text while `on` (AppKit's recessed/toolbar button). One style for
/// every item in the bar, toggles and plain buttons alike; they used to mix
/// tinted chips, bordered bezels and bare text.
pub fn bar_toggle(ui: &mut Ui, on: bool, text: impl Into<egui::WidgetText>) -> Response {
    bar_item(ui, Some(on), text)
}

/// [`bar_toggle`] without a state (A+, Colour, Done).
pub fn bar_button(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    bar_item(ui, None, text)
}

fn bar_item(ui: &mut Ui, on: Option<bool>, text: impl Into<egui::WidgetText>) -> Response {
    let p = pal(ui);
    let text: egui::WidgetText = text.into();
    let galley = text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    let w = (galley.size().x + 14.0).max(24.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 24.0), Sense::click());
    resp.widget_info(|| match on {
        Some(o) => WidgetInfo::selected(Role::Button, ui.is_enabled(), o, galley.text()),
        None => WidgetInfo::labeled(Role::Button, ui.is_enabled(), galley.text()),
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let lit = on == Some(true);
        if lit || resp.is_pointer_button_down_on() {
            painter.rect_filled(rect, RADIUS, p.pressed);
        } else if resp.hovered() {
            painter.rect_filled(rect, RADIUS, p.hover);
        }
        let c = if lit { accent() } else { p.text };
        let pos = Pos2::new(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0);
        painter.galley(pos, galley, c);
    }
    resp
}

/// A thin vertical divider between groups in a floating bar.
pub fn bar_divider(ui: &mut Ui) {
    let p = pal(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 24.0), Sense::hover());
    let x = rect.center().x.round() + 0.5;
    ui.painter().line_segment([Pos2::new(x, rect.top() + 4.0), Pos2::new(x, rect.bottom() - 4.0)], Stroke::new(1.0, p.border));
}

/// A vertical hairline between toolbar groups.
pub fn toolbar_separator(ui: &mut Ui) {
    let p = pal(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 44.0), Sense::hover());
    ui.painter().line_segment([Pos2::new(rect.center().x, rect.top() + 6.0), Pos2::new(rect.center().x, rect.top() + 26.0)], Stroke::new(1.0, p.border));
}

// ----------------------------------------------------------------------
// Segmented control

#[derive(Clone, Copy)]
pub enum Seg<'a> {
    Text(&'a str),
    Icon(Icon, &'a str),
}

impl Seg<'_> {
    fn tip(&self) -> &str {
        match self {
            Seg::Text(t) => t,
            Seg::Icon(_, t) => t,
        }
    }
}

/// Exclusive segmented control (NSSegmentedControl, select-one). Returns a
/// response that is `changed()` when the user picked a different segment.
/// `width`: total width, or `None` to size to content.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, Seg)], width: Option<f32>) -> Response {
    let states: Vec<bool> = items.iter().map(|(v, _)| v == current).collect();
    let segs: Vec<Seg> = items.iter().map(|(_, s)| *s).collect();
    let (mut resp, clicked) = segmented_impl(ui, &states, &segs, width, Size::Regular);
    if let Some(i) = clicked {
        if items[i].0 != *current {
            *current = items[i].0;
            resp.mark_changed();
        }
    }
    resp
}

/// Segmented control where each segment toggles on its own (B / I / U).
/// Returns the index of the segment clicked this frame.
pub fn segmented_toggles(ui: &mut Ui, states: &[bool], segs: &[Seg], width: Option<f32>) -> Option<usize> {
    segmented_impl(ui, states, segs, width, Size::Regular).1
}

/// Tab bar for the top of an inspector or settings pane: a segmented
/// control centred in the available width.
pub fn tab_bar<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, &str)]) -> Response {
    let segs: Vec<(T, Seg)> = items.iter().map(|(v, l)| (*v, Seg::Text(l))).collect();
    let w = ui.available_width();
    ui.vertical_centered(|ui| segmented(ui, current, &segs, Some(w.min(items.len() as f32 * 66.0)))).inner
}

fn segmented_impl(ui: &mut Ui, states: &[bool], segs: &[Seg], width: Option<f32>, size: Size) -> (Response, Option<usize>) {
    let p = pal(ui);
    let n = segs.len().max(1);
    let font = size.font();
    let seg_ws: Vec<f32> = segs
        .iter()
        .map(|s| match s {
            Seg::Text(t) => ui.painter().layout_no_wrap(t.to_string(), font.clone(), Color32::PLACEHOLDER).size().x + 18.0,
            Seg::Icon(..) => 30.0,
        })
        .collect();
    let natural: f32 = seg_ws.iter().sum();
    // Never narrower than the labels need: squeezed segments overlap.
    let total = width.unwrap_or(natural).max(natural);
    let widest = seg_ws.iter().cloned().fold(0.0, f32::max);
    // Equal widths when a width is given and every label fits
    // (AppKit's `.fillEqually`); otherwise share the spare room out.
    let ws: Vec<f32> = if width.is_some() && total / n as f32 >= widest {
        vec![total / n as f32; n]
    } else {
        seg_ws.iter().map(|w| w + (total - natural) / n as f32).collect()
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(total, size.height()), Sense::hover());
    let mut clicked = None;
    if ui.is_rect_visible(rect) {
        paint_bezel(ui, rect, false, ui.is_enabled());
        if p.dark {
            ui.painter().rect_filled(rect, RADIUS, Color32::from_gray(0x3c));
        } else {
            ui.painter().rect(rect, RADIUS, Color32::from_gray(0xe3), Stroke::new(0.6, p.control_edge), StrokeKind::Inside);
        }
    }
    let mut x = rect.left();
    for (i, seg) in segs.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(ws[i], rect.height()));
        x += ws[i];
        let id = resp.id.with(i);
        let sr = ui.interact(r, id, Sense::click());
        let on = states.get(i).copied().unwrap_or(false);
        sr.widget_info(|| WidgetInfo::selected(Role::Button, ui.is_enabled(), on, seg.tip()));
        let sr = if let Seg::Icon(_, tip) = seg { if !tip.is_empty() { sr.on_hover_text(*tip) } else { sr } } else { sr };
        if sr.clicked() {
            clicked = Some(i);
        }
        let painter = ui.painter();
        if on {
            // Raised plate for the selected segment.
            let plate = r.shrink2(Vec2::new(1.5, 1.5));
            if !p.dark {
                painter.rect_filled(plate.translate(Vec2::new(0.0, 0.5)), RADIUS - 1.0, Color32::from_black_alpha(30));
            }
            painter.rect_filled(plate, RADIUS - 1.0, p.segment_on);
        } else if sr.is_pointer_button_down_on() {
            painter.rect_filled(r.shrink(1.5), RADIUS - 1.0, p.pressed);
        } else if i > 0 && !states.get(i - 1).copied().unwrap_or(false) {
            // Divider between two unselected segments.
            painter.line_segment([Pos2::new(r.left(), r.top() + 5.0), Pos2::new(r.left(), r.bottom() - 5.0)], Stroke::new(1.0, p.control_edge));
        }
        let c = text_color(ui, ui.is_enabled());
        match seg {
            Seg::Text(t) => {
                let f = if on { style::semibold(font.size) } else { font.clone() };
                painter.text(r.center(), Align2::CENTER_CENTER, *t, f, c);
            }
            Seg::Icon(icon, _) => {
                paint_icon(painter, Rect::from_center_size(r.center(), Vec2::splat(15.0)), *icon, c);
            }
        }
    }
    (resp, clicked)
}

// ----------------------------------------------------------------------
// Pop-up button (NSPopUpButton): bezel + label + accent chevron box.

/// A pop-up button. `add_contents` fills the menu; use [`menu_item`] /
/// [`menu_check`] inside it. Returns the button response.
pub fn popup_button(ui: &mut Ui, id_salt: impl std::hash::Hash + std::fmt::Debug, label: &str, width: f32, add_contents: impl FnOnce(&mut Ui)) -> Response {
    popup_button_named(ui, id_salt, None, label, width, add_contents)
}

/// A pop-up button that VoiceOver names `name` ("Weight"), reading the
/// chosen item (`label`) as its value.
pub fn popup_button_named(ui: &mut Ui, id_salt: impl std::hash::Hash + std::fmt::Debug, name: Option<&str>, label: &str, width: f32, add_contents: impl FnOnce(&mut Ui)) -> Response {
    let p = pal(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, H_REGULAR), Sense::hover());
    let resp = ui.interact(rect, ui.scope_id().with(("popup", id_salt)), Sense::click());
    resp.widget_info(|| match name {
        Some(n) => {
            let mut info = WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), n);
            info.current_text_value = Some(label.to_string());
            info
        }
        None => WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), label),
    });
    if ui.is_rect_visible(rect) {
        let pressed = resp.is_pointer_button_down_on();
        paint_bezel(ui, rect, pressed, ui.is_enabled());
        let painter = ui.painter();
        let tc = text_color(ui, ui.is_enabled());
        let galley = painter.layout_no_wrap(label.to_string(), style::ui_font(13.0), tc);
        let clip = Rect::from_min_max(rect.min, Pos2::new(rect.right() - 22.0, rect.bottom()));
        painter.with_clip_rect(clip).galley(Pos2::new(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0), galley, tc);
        // Accent chevron box (Big Sur+ pop-up style).
        let bx = Rect::from_min_size(Pos2::new(rect.right() - 18.0, rect.top() + 3.0), Vec2::new(15.0, rect.height() - 6.0));
        painter.rect_filled(bx, 4.0, if ui.is_enabled() { accent() } else { p.disabled });
        paint_icon(painter, bx.shrink2(Vec2::new(1.0, 1.5)), Icon::UpDown, on_accent());
        if resp.has_focus() {
            focus_ring(ui, rect, RADIUS);
        }
    }
    egui::Popup::menu(&resp).style(crate::widgets::menu_style).width(width.max(140.0)).show(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        add_contents(ui)
    });
    resp
}

/// Style for every menu and pop-up menu: egui's menu style, with no gap
/// between rows. NSMenu rows are 22 pt and touch; egui's 6 pt item
/// spacing made them 28 pt apart and the separators 15 pt.
pub fn menu_style(style: &mut egui::Style) {
    egui::containers::menu::menu_style(style);
    style.spacing.item_spacing.y = 0.0;
}

/// A plain menu row (13 pt, accent highlight on hover, optional shortcut).
pub fn menu_item(ui: &mut Ui, label: &str, shortcut: Option<&str>) -> Response {
    menu_row(ui, None, label, shortcut, true)
}

/// A menu row with a leading ✓ when `checked` (NSMenuItem state on).
pub fn menu_check(ui: &mut Ui, checked: bool, label: &str, shortcut: Option<&str>) -> Response {
    menu_row(ui, Some(checked), label, shortcut, true)
}

pub fn menu_row(ui: &mut Ui, checked: Option<bool>, label: &str, shortcut: Option<&str>, enabled: bool) -> Response {
    let p = pal(ui);
    // Wide enough for the title, a 24 pt gap and the shortcut: the menu
    // grows to fit (a fixed width ran long titles into their shortcuts).
    let font = style::ui_font(13.0);
    let lw = ui.painter().layout_no_wrap(label.to_string(), font.clone(), Color32::PLACEHOLDER).size().x;
    let sw = shortcut.map_or(0.0, |s| 24.0 + ui.painter().layout_no_wrap(s.to_string(), font.clone(), Color32::PLACEHOLDER).size().x);
    let need = 22.0 + lw + sw + 10.0;
    let w = ui.available_width().max(120.0).max(need);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 22.0), if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| match checked {
        Some(c) => WidgetInfo::selected(Role::CheckBox, enabled, c, label),
        None => WidgetInfo::labeled(Role::Button, enabled, label),
    });
    let hot = enabled && resp.hovered();
    let painter = ui.painter();
    if hot {
        painter.rect_filled(rect, 4.0, accent());
    }
    let tc = if hot { on_accent() } else if enabled { p.text } else { p.disabled };
    if checked == Some(true) {
        paint_icon(painter, Rect::from_center_size(Pos2::new(rect.left() + 11.0, rect.center().y), Vec2::splat(11.0)), Icon::Check, tc);
    }
    painter.text(Pos2::new(rect.left() + 22.0, rect.center().y), Align2::LEFT_CENTER, label, font.clone(), tc);
    if let Some(s) = shortcut {
        let sc = if hot { on_accent() } else { p.weak };
        painter.text(Pos2::new(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, s, font, sc);
    }
    if resp.clicked() {
        ui.close();
    }
    resp
}

/// A menu row with a colour swatch before the title (NSMenu image column).
pub fn menu_swatch(ui: &mut Ui, color: Color32, label: &str) -> Response {
    let resp = menu_row(ui, None, label, None, true);
    let r = Rect::from_center_size(Pos2::new(resp.rect.left() + 11.0, resp.rect.center().y), Vec2::splat(11.0));
    ui.painter().rect(r, 2.5, color, Stroke::new(0.6, Color32::from_black_alpha(50)), StrokeKind::Inside);
    resp
}

/// Menu separator (NSMenuItem.separator).
pub fn menu_separator(ui: &mut Ui) {
    let p = pal(ui);
    // NSMenu separator: 11 pt high, the line inset like the row text.
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 11.0), Sense::hover());
    let y = rect.center().y.round() + 0.5;
    ui.painter().line_segment([Pos2::new(rect.left() + 10.0, y), Pos2::new(rect.right() - 10.0, y)], Stroke::new(1.0, p.border));
}

// ----------------------------------------------------------------------
// Checkbox, radio, switch

/// NSButton checkbox: 14 pt rounded square, accent fill with a white ✓.
pub fn checkbox(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let p = pal(ui);
    let font = style::ui_font(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
    let w = 14.0 + if label.is_empty() { 0.0 } else { 6.0 + galley.size().x };
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(w, H_REGULAR), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| WidgetInfo::selected(Role::CheckBox, ui.is_enabled(), *on, label));
    if ui.is_rect_visible(rect) {
        let b = Rect::from_min_size(Pos2::new(rect.left(), rect.center().y - 7.0), Vec2::splat(14.0));
        let painter = ui.painter();
        if *on {
            painter.rect_filled(b, 3.5, if ui.is_enabled() { accent() } else { p.disabled });
            paint_icon(painter, b.shrink(2.5), Icon::Check, on_accent());
        } else {
            paint_bezel(ui, b, resp.is_pointer_button_down_on(), ui.is_enabled());
        }
        if !label.is_empty() {
            ui.painter().galley(Pos2::new(b.right() + 6.0, rect.center().y - galley.size().y / 2.0), galley, text_color(ui, ui.is_enabled()));
        }
        if resp.has_focus() {
            focus_ring(ui, b, 3.5);
        }
    }
    resp
}

/// NSSlider, horizontal, no tick marks: a 4 pt track filled with the
/// accent up to a 18 pt round white knob. Arrow keys step by 5 % when it
/// has focus. `width` is the whole control, knob included.
pub fn slider(ui: &mut Ui, value: &mut f64, range: std::ops::RangeInclusive<f64>, width: f32, label: &str) -> Response {
    let p = pal(ui);
    const KNOB: f32 = 18.0;
    let (lo, hi) = (*range.start(), *range.end());
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(width, H_REGULAR), Sense::click_and_drag());
    let x0 = rect.left() + KNOB / 2.0;
    let x1 = rect.right() - KNOB / 2.0;
    let old = *value;
    if let Some(pos) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() || resp.drag_started() {
            let t = ((pos.x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0) as f64;
            *value = lo + t * (hi - lo);
        }
    }
    if resp.has_focus() {
        let step = (hi - lo) * 0.05;
        ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowUp) {
                *value = (*value + step).min(hi);
            }
            if i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowDown) {
                *value = (*value - step).max(lo);
            }
        });
    }
    if *value != old {
        resp.mark_changed();
    }
    let v = *value;
    resp.widget_info(|| WidgetInfo::slider(ui.is_enabled(), v, label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let t = ((v - lo) / (hi - lo).max(f64::EPSILON)).clamp(0.0, 1.0) as f32;
        let kx = (x0 + t * (x1 - x0)).round();
        let cy = rect.center().y.round();
        let track = Rect::from_min_max(Pos2::new(x0 - 2.0, cy - 2.0), Pos2::new(x1 + 2.0, cy + 2.0));
        let empty = if p.dark { Color32::from_gray(0x55) } else { Color32::from_gray(0xd8) };
        painter.rect_filled(track, 2.0, empty);
        let fill = if ui.is_enabled() { accent() } else { p.disabled };
        painter.rect_filled(Rect::from_min_max(track.min, Pos2::new(kx, track.max.y)), 2.0, fill);
        let c = Pos2::new(kx, cy);
        // Soft shadow, then the knob with a hairline edge.
        painter.circle_filled(c + Vec2::new(0.0, 0.5), KNOB / 2.0 + 0.5, Color32::from_black_alpha(if p.dark { 90 } else { 40 }));
        let knob = if p.dark { Color32::from_gray(if resp.is_pointer_button_down_on() { 0xb0 } else { 0xcc }) } else if resp.is_pointer_button_down_on() { Color32::from_gray(0xf0) } else { Color32::WHITE };
        painter.circle(c, KNOB / 2.0 - 0.3, knob, Stroke::new(0.6, p.control_edge));
        if resp.has_focus() {
            focus_ring(ui, Rect::from_center_size(c, Vec2::splat(KNOB)), KNOB / 2.0);
        }
    }
    resp
}

/// NSButton radio: 14 pt circle, accent fill with a white dot.
pub fn radio(ui: &mut Ui, selected: bool, label: &str) -> Response {
    let p = pal(ui);
    let galley = ui.painter().layout_no_wrap(label.to_string(), style::ui_font(13.0), Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(20.0 + galley.size().x, H_REGULAR), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(Role::RadioButton, ui.is_enabled(), selected, label));
    if ui.is_rect_visible(rect) {
        let c = Pos2::new(rect.left() + 7.0, rect.center().y);
        let painter = ui.painter();
        if selected {
            painter.circle_filled(c, 7.0, accent());
            painter.circle_filled(c, 2.8, on_accent());
        } else {
            if !p.dark {
                painter.circle_filled(c + Vec2::new(0.0, 0.5), 7.0, Color32::from_black_alpha(22));
            }
            painter.circle(c, 7.0, p.control, Stroke::new(0.6, p.control_edge));
        }
        painter.galley(Pos2::new(rect.left() + 20.0, rect.center().y - galley.size().y / 2.0), galley, text_color(ui, ui.is_enabled()));
    }
    resp
}

/// NSSwitch (mini size, 26×15 like Big Sur's inspector switches).
pub fn switch(ui: &mut Ui, on: &mut bool, label_for_a11y: &str) -> Response {
    let p = pal(ui);
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(32.0, 18.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| WidgetInfo::selected(Role::Switch, ui.is_enabled(), *on, label_for_a11y));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(resp.id, *on);
        let off = if p.dark { Color32::from_gray(0x55) } else { Color32::from_gray(0xd9) };
        let bg = off.lerp_to_gamma(accent(), t);
        let painter = ui.painter();
        painter.rect(rect, 9.0, bg, Stroke::new(0.6, if p.dark { Color32::TRANSPARENT } else { p.control_edge.gamma_multiply(1.0 - t) }), StrokeKind::Inside);
        let x = egui::lerp((rect.left() + 9.0)..=(rect.right() - 9.0), t);
        painter.circle_filled(Pos2::new(x, rect.center().y + 0.5), 7.6, Color32::from_black_alpha(35));
        painter.circle_filled(Pos2::new(x, rect.center().y), 7.5, if p.dark { Color32::from_gray(0xe6) } else { Color32::WHITE });
    }
    resp
}

// ----------------------------------------------------------------------
// Text field and number field with stepper

/// NSTextField (rounded bezel): white field, hairline edge, accent focus ring.
pub fn text_field(ui: &mut Ui, text: &mut String, width: f32, hint: &str) -> Response {
    let p = pal(ui);
    let rect = ui.allocate_space(Vec2::new(width, H_REGULAR)).1;
    paint_field(ui, rect, &p);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(6.0, 2.0))));
    let r = child.add(
        egui::TextEdit::singleline(text)
            .frame(egui::Frame::NONE)
            .font(style::ui_font(13.0))
            .hint_text(hint)
            .desired_width(width - 12.0)
            .align(Align2::LEFT_CENTER)
            .margin(egui::Margin::ZERO),
    );
    if r.has_focus() {
        focus_ring(ui, rect, 4.0);
    }
    r
}

/// NSSearchField: a rounded field with a magnifier, and a clear button
/// once something is typed.
pub fn search_field(ui: &mut Ui, text: &mut String, width: f32, hint: &str) -> Response {
    let p = pal(ui);
    let rect = ui.allocate_space(Vec2::new(width, H_REGULAR)).1;
    // A rounded rectangle (6 pt), as in Big Sur to Sonoma, not a capsule.
    let r = 6.0;
    ui.painter().rect(rect, r, p.field, Stroke::new(if p.dark { 0.0 } else { 0.6 }, p.control_edge), StrokeKind::Inside);
    // Magnifier.
    let c = egui::pos2(rect.left() + 13.0, rect.center().y - 1.0);
    let glyph = Stroke::new(1.3, p.weak);
    ui.painter().circle_stroke(c, 4.2, glyph);
    ui.painter().line_segment([c + Vec2::splat(3.0), c + Vec2::splat(6.2)], glyph);
    let clear_w = if text.is_empty() { 0.0 } else { 20.0 };
    let field = Rect::from_min_max(egui::pos2(rect.left() + 24.0, rect.top() + 2.0), egui::pos2(rect.right() - 8.0 - clear_w, rect.bottom() - 2.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field));
    let resp = child.add(
        egui::TextEdit::singleline(text)
            .frame(egui::Frame::NONE)
            .font(style::ui_font(13.0))
            .hint_text(hint)
            .desired_width(field.width())
            .align(Align2::LEFT_CENTER)
            .margin(egui::Margin::ZERO),
    );
    // Esc clears what was typed, as in every Mac search field.
    if resp.lost_focus() && !text.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        text.clear();
        let mut r2 = resp.clone();
        r2.mark_changed();
        return r2;
    }
    if !text.is_empty() {
        let cr = Rect::from_center_size(egui::pos2(rect.right() - 13.0, rect.center().y), Vec2::splat(14.0));
        let b = ui.interact(cr, resp.id.with("clear"), Sense::click());
        ui.painter().circle_filled(cr.center(), 7.0, if b.hovered() { p.text } else { p.weak });
        let d = 2.6;
        let x = Stroke::new(1.4, p.field);
        ui.painter().line_segment([cr.center() - Vec2::splat(d), cr.center() + Vec2::splat(d)], x);
        ui.painter().line_segment([cr.center() + Vec2::new(-d, d), cr.center() + Vec2::new(d, -d)], x);
        if b.clicked() {
            text.clear();
            resp.request_focus();
            let mut r2 = resp.clone();
            r2.mark_changed();
            if resp.has_focus() {
                focus_ring(ui, rect, r);
            }
            return r2;
        }
    }
    if resp.has_focus() {
        focus_ring(ui, rect, r);
    }
    resp
}

fn paint_field(ui: &Ui, rect: Rect, p: &Palette) {
    let painter = ui.painter();
    painter.rect(rect, 4.0, p.field, Stroke::new(if p.dark { 0.0 } else { 0.6 }, p.control_edge), StrokeKind::Inside);
    if !p.dark {
        // inset top shadow line
        painter.line_segment([rect.left_top() + Vec2::new(4.0, 0.8), rect.right_top() + Vec2::new(-4.0, 0.8)], Stroke::new(0.6, Color32::from_black_alpha(18)));
    }
}

/// Options for [`number_field`].
#[derive(Clone, Copy)]
pub struct Num {
    pub step: f64,
    pub min: f64,
    pub max: f64,
    pub decimals: usize,
    pub suffix: &'static str,
    pub width: f32,
    /// Multiply for display (e.g. 100 for a 0..1 value shown as %).
    pub display_scale: f64,
}

impl Default for Num {
    fn default() -> Self {
        Num { step: 1.0, min: f64::MIN, max: f64::MAX, decimals: 0, suffix: "", width: 64.0, display_scale: 1.0 }
    }
}

impl Num {
    pub fn pt() -> Num {
        Num { suffix: " pt", min: 0.0, ..Default::default() }
    }
    pub fn deg() -> Num {
        Num { suffix: "°", decimals: 0, ..Default::default() }
    }
    pub fn pct() -> Num {
        Num { suffix: "%", display_scale: 100.0, step: 0.05, min: 0.0, ..Default::default() }
    }
    pub fn secs() -> Num {
        Num { suffix: " s", decimals: 1, step: 0.5, min: 0.0, ..Default::default() }
    }
    pub fn width(mut self, w: f32) -> Num {
        self.width = w;
        self
    }
    pub fn range(mut self, min: f64, max: f64) -> Num {
        self.min = min;
        self.max = max;
        self
    }
    pub fn step(mut self, s: f64) -> Num {
        self.step = s;
        self
    }
    pub fn decimals(mut self, d: usize) -> Num {
        self.decimals = d;
        self
    }
}

/// The unit after a number, translated: " pt" and " s" are words in some
/// languages, and some put a space before "%".
fn unit_text(suffix: &str) -> String {
    match suffix {
        " pt" => format!(" {}", crate::t!("widgets-unit-pt")),
        " s" => format!(" {}", crate::t!("widgets-unit-seconds")),
        "%" => crate::t!("widgets-unit-percent"),
        "°" => crate::t!("widgets-unit-degrees"),
        s => s.to_string(),
    }
}

/// A number field with an NSStepper beside it. Typing commits on Return or
/// when focus leaves; the stepper arrows add or subtract `step`; dragging
/// vertically on the stepper scrubs. `changed()` fires on every committed
/// change; `dragged()`/`has_focus()` tell the caller a gesture is ongoing.
pub fn number_field(ui: &mut Ui, value: &mut f64, opt: Num) -> Response {
    let p = pal(ui);
    let id = ui.next_auto_id();
    let suffix = unit_text(opt.suffix);
    let fmt = |v: f64| format!("{:.*}{}", opt.decimals, v * opt.display_scale, suffix);
    let total = Vec2::new(opt.width + 15.0, H_REGULAR);
    let (outer, _) = ui.allocate_exact_size(total, Sense::hover());
    let field = Rect::from_min_size(outer.min, Vec2::new(opt.width, H_REGULAR));
    let stepper = Rect::from_min_size(Pos2::new(field.right() + 3.0, outer.top() + 1.0), Vec2::new(12.0, H_REGULAR - 2.0));
    paint_field(ui, field, &p);

    let buf_id = id.with("buf");
    let mut buf: String = ui.data(|d| d.get_temp(buf_id)).unwrap_or_else(|| fmt(*value));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field.shrink2(Vec2::new(5.0, 2.0))).id_salt(id));
    let te = child.add(
        egui::TextEdit::singleline(&mut buf)
            .id(id.with("te"))
            .frame(egui::Frame::NONE)
            .font(style::ui_font(13.0))
            .desired_width(opt.width - 10.0)
            .align(Align2::RIGHT_CENTER)
            .margin(egui::Margin::ZERO),
    );
    let mut resp = te.clone();
    let commit = |buf: &str, value: &mut f64| -> bool {
        // Text the person didn't change is not a new value: the field
        // shows the value rounded, and writing that back would change the
        // object just for clicking in and out.
        if buf == fmt(*value) {
            return false;
        }
        let t: String = buf.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == ',').collect();
        if let Ok(v) = t.replace(',', ".").parse::<f64>() {
            let v = (v / opt.display_scale).clamp(opt.min, opt.max);
            if (v - *value).abs() > 1e-12 {
                *value = v;
                return true;
            }
        }
        false
    };
    if te.has_focus() {
        focus_ring(ui, field, 4.0);
        ui.data_mut(|d| d.insert_temp(buf_id, buf.clone()));
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && commit(&buf, value) {
            resp.mark_changed();
        }
    } else {
        if te.lost_focus() && commit(&buf, value) {
            resp.mark_changed();
        }
        ui.data_mut(|d| d.remove::<String>(buf_id));
    }

    // Stepper: two halves; drag scrubs.
    let sr = ui.interact(stepper, id.with("stepper"), Sense::click_and_drag());
    paint_bezel(ui, stepper, sr.is_pointer_button_down_on(), ui.is_enabled());
    let painter = ui.painter();
    let mid = stepper.center().y;
    painter.line_segment([Pos2::new(stepper.left() + 2.0, mid), Pos2::new(stepper.right() - 2.0, mid)], Stroke::new(0.6, p.control_edge));
    let c = text_color(ui, ui.is_enabled());
    let up = Rect::from_min_max(stepper.min, Pos2::new(stepper.right(), mid));
    let dn = Rect::from_min_max(Pos2::new(stepper.left(), mid), stepper.max);
    for (r, d) in [(up, -1.0f32), (dn, 1.0)] {
        let cc = r.center();
        painter.add(Shape::convex_polygon(vec![cc + Vec2::new(-2.8, -1.4 * d), cc + Vec2::new(2.8, -1.4 * d), cc + Vec2::new(0.0, 1.8 * d)], c, Stroke::NONE));
    }
    let mut nv = *value;
    if sr.clicked() {
        if let Some(pp) = sr.interact_pointer_pos() {
            nv += if pp.y < mid { opt.step } else { -opt.step };
        }
    }
    if sr.dragged() {
        nv -= sr.drag_delta().y as f64 * opt.step * 0.25;
    }
    nv = nv.clamp(opt.min, opt.max);
    if (nv - *value).abs() > 1e-12 {
        *value = nv;
        resp.mark_changed();
        ui.data_mut(|d| d.remove::<String>(buf_id));
    }
    // Report the stepper drag as part of the gesture.
    resp = resp.union(sr);
    resp
}

// ----------------------------------------------------------------------
// Colour well and swatches

/// One swatch in a [`swatch_grid`].
pub struct Swatch<'a> {
    pub color: Color32,
    /// Opaque key returned when picked (a theme slot like `theme:accent`,
    /// `""` for "theme default", or `#rrggbb`).
    pub key: &'a str,
    pub tip: &'a str,
}

/// Grid of 16 pt square swatches (Keynote's colour presets). Duplicate
/// colours are dropped, so the grid never shows two identical chips.
/// Returns the picked key.
pub fn swatch_grid(ui: &mut Ui, swatches: &[Swatch], selected: &str) -> Option<String> {
    let p = pal(ui);
    let mut seen: Vec<Color32> = vec![];
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for s in swatches {
            if s.color != Color32::TRANSPARENT && seen.contains(&s.color) && s.key != selected {
                continue;
            }
            seen.push(s.color);
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
            resp.widget_info(|| WidgetInfo::selected(Role::Button, true, s.key == selected, s.tip));
            let painter = ui.painter();
            if s.color == Color32::TRANSPARENT {
                painter.rect(r.shrink(1.0), 3.0, p.field, Stroke::new(0.6, p.control_edge), StrokeKind::Inside);
                painter.line_segment([r.left_bottom() + Vec2::new(3.0, -3.0), r.right_top() + Vec2::new(-3.0, 3.0)], Stroke::new(1.3, Color32::from_rgb(0xff, 0x3b, 0x30)));
            } else {
                painter.rect(r.shrink(1.0), 3.0, s.color, Stroke::new(0.6, Color32::from_black_alpha(45)), StrokeKind::Inside);
            }
            if s.key == selected {
                painter.rect_stroke(r.expand(1.0), 4.5, Stroke::new(2.0, accent()), StrokeKind::Inside);
            } else if resp.hovered() {
                painter.rect_stroke(r.expand(0.5), 4.0, Stroke::new(1.0, p.weak), StrokeKind::Inside);
            }
            if resp.on_hover_text(s.tip).clicked() {
                picked = Some(s.key.to_string());
            }
        }
    });
    picked
}

/// NSColorWell look: a bezel with the colour inset. Clicking opens a
/// popover with an HSV picker (the system colour panel is AppKit, which the
/// egui-only rule keeps out). Returns `Some(colour)` while the user edits.
pub fn color_well(ui: &mut Ui, id_salt: impl std::hash::Hash + std::fmt::Debug, color: Color32) -> Option<Color32> {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(44.0, H_REGULAR), Sense::hover());
    let resp = ui.interact(rect, ui.scope_id().with(("well", id_salt)), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::ColorWell, ui.is_enabled(), crate::t!("widgets-colour-well")));
    paint_bezel(ui, rect, resp.is_pointer_button_down_on(), ui.is_enabled());
    ui.painter().rect(rect.shrink2(Vec2::new(5.0, 4.0)), 2.0, color, Stroke::new(0.6, Color32::from_black_alpha(50)), StrokeKind::Inside);
    let mut out = None;
    let pid = resp.id;
    egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).frame(popover_frame(ui)).show(|ui| {
        if let Some(c) = color_picker(ui, pid, color) {
            out = Some(c);
        }
    });
    out.filter(|c| *c != color)
}

/// The popover's frame: a menu's look with a 12 pt inset.
fn popover_frame(ui: &Ui) -> egui::Frame {
    egui::Frame::menu(ui.style()).inner_margin(12)
}

/// Saturation/brightness square, hue strip and a hex field, at 200 pt:
/// the colour panel's wheel tab cut down to what a popover needs. The
/// hue is remembered while the colour is grey, where it can't be derived.
fn color_picker(ui: &mut Ui, id: Id, color: Color32) -> Option<Color32> {
    use egui::ecolor::Hsva;
    const W: f32 = 200.0;
    let key = id.with("hsva");
    let mut hsva = ui.ctx().data(|d| d.get_temp::<(Color32, Hsva)>(key)).filter(|(c, _)| *c == color).map(|(_, h)| h).unwrap_or_else(|| Hsva::from(color));
    let before = hsva;
    let p = pal(ui);
    ui.set_width(W);
    ui.spacing_mut().item_spacing.y = 8.0;

    // Saturation (x) by brightness (y) for the current hue.
    let (sq, sq_resp) = ui.allocate_exact_size(Vec2::new(W, 150.0), Sense::click_and_drag());
    if let Some(pos) = sq_resp.interact_pointer_pos() {
        hsva.s = ((pos.x - sq.left()) / sq.width()).clamp(0.0, 1.0);
        hsva.v = 1.0 - ((pos.y - sq.top()) / sq.height()).clamp(0.0, 1.0);
    }
    let mut mesh = egui::Mesh::default();
    const N: usize = 12;
    for yi in 0..=N {
        for xi in 0..=N {
            let (x, y) = (xi as f32 / N as f32, yi as f32 / N as f32);
            let c: Color32 = Hsva::new(hsva.h, x, 1.0 - y, 1.0).into();
            mesh.colored_vertex(Pos2::new(sq.left() + x * sq.width(), sq.top() + y * sq.height()), c);
            if xi < N && yi < N {
                let i = (yi * (N + 1) + xi) as u32;
                let r = (N + 1) as u32;
                mesh.add_triangle(i, i + 1, i + r);
                mesh.add_triangle(i + 1, i + r + 1, i + r);
            }
        }
    }
    ui.painter().add(Shape::mesh(mesh));
    ui.painter().rect_stroke(sq, 3.0, Stroke::new(0.6, p.control_edge), StrokeKind::Inside);
    let knob = Pos2::new(sq.left() + hsva.s * sq.width(), sq.top() + (1.0 - hsva.v) * sq.height());
    ui.painter().circle_stroke(knob, 5.0, Stroke::new(2.0, Color32::WHITE));
    ui.painter().circle_stroke(knob, 6.0, Stroke::new(0.6, Color32::from_black_alpha(90)));

    // Hue strip.
    let (hr, h_resp) = ui.allocate_exact_size(Vec2::new(W, 12.0), Sense::click_and_drag());
    if let Some(pos) = h_resp.interact_pointer_pos() {
        hsva.h = ((pos.x - hr.left()) / hr.width()).clamp(0.0, 0.9999);
    }
    let mut mesh = egui::Mesh::default();
    const HN: usize = 36;
    for i in 0..=HN {
        let t = i as f32 / HN as f32;
        let c: Color32 = Hsva::new(t, 1.0, 1.0, 1.0).into();
        let x = hr.left() + t * hr.width();
        mesh.colored_vertex(Pos2::new(x, hr.top()), c);
        mesh.colored_vertex(Pos2::new(x, hr.bottom()), c);
        if i < HN {
            let j = (i * 2) as u32;
            mesh.add_triangle(j, j + 1, j + 2);
            mesh.add_triangle(j + 1, j + 3, j + 2);
        }
    }
    ui.painter().add(Shape::mesh(mesh));
    ui.painter().rect_stroke(hr, 3.0, Stroke::new(0.6, p.control_edge), StrokeKind::Inside);
    let hx = hr.left() + hsva.h * hr.width();
    let hk = Rect::from_center_size(Pos2::new(hx, hr.center().y), Vec2::new(6.0, hr.height() + 4.0));
    ui.painter().rect(hk, 2.0, Color32::WHITE, Stroke::new(0.6, Color32::from_black_alpha(90)), StrokeKind::Inside);

    let mut picked: Color32 = hsva.into();
    // The colour and its hex code, editable.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (sw, _) = ui.allocate_exact_size(Vec2::new(44.0, H_REGULAR), Sense::hover());
        paint_bezel(ui, sw, false, true);
        ui.painter().rect(sw.shrink2(Vec2::new(5.0, 4.0)), 2.0, picked, Stroke::new(0.6, Color32::from_black_alpha(50)), StrokeKind::Inside);
        let buf_id = id.with("hex");
        let mut buf = ui.ctx().data(|d| d.get_temp::<String>(buf_id)).unwrap_or_else(|| hex(picked));
        let field = text_field(ui, &mut buf, W - 44.0 - 8.0, "");
        if field.has_focus() {
            if let Some(c) = parse_hex6(&buf) {
                if c != picked {
                    picked = c;
                    hsva = Hsva::from(c);
                }
            }
            ui.ctx().data_mut(|d| d.insert_temp(buf_id, buf));
        } else {
            ui.ctx().data_mut(|d| d.remove::<String>(buf_id));
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(key, (picked, hsva)));
    (hsva != before).then_some(picked)
}

fn parse_hex6(s: &str) -> Option<Color32> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

// ----------------------------------------------------------------------
// Inspector layout: sections and label/control rows

/// An inspector section: a semibold 13 pt title-case header, then the
/// content, then a full-width separator. This is the API other streams use
/// to add inspector content.
pub fn section<R>(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let p = pal(ui);
    ui.add_space(8.0);
    if !title.is_empty() {
        ui.label(egui::RichText::new(title).font(style::semibold(13.0)).color(p.text));
        ui.add_space(2.0);
    }
    let r = ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        add_contents(ui)
    }).inner;
    ui.add_space(8.0);
    separator(ui);
    r
}

/// A section whose header is a disclosure triangle (collapsed state is
/// remembered per `id_salt` for the session).
pub fn disclosure_section<R>(ui: &mut Ui, id_salt: &str, title: &str, default_open: bool, add_contents: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let p = pal(ui);
    let id = Id::unique(("disclosure", id_salt));
    let mut open: bool = ui.data(|d| d.get_temp(id)).unwrap_or(default_open);
    ui.add_space(8.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 18.0), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(Role::DisclosureTriangle, true, open, title));
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    let painter = ui.painter();
    painter.text(Pos2::new(rect.left(), rect.center().y), Align2::LEFT_CENTER, title, style::semibold(13.0), p.text);
    let tri = Rect::from_center_size(Pos2::new(rect.right() - 8.0, rect.center().y), Vec2::splat(10.0));
    paint_icon(painter, tri, if open { Icon::ChevronDown } else { Icon::ChevronRight }, p.weak);
    let r = if open {
        ui.add_space(4.0);
        Some(ui.scope(|ui| add_contents(ui)).inner)
    } else {
        None
    };
    ui.add_space(8.0);
    separator(ui);
    r
}

/// A row with a right-aligned 11 pt label in a fixed left column and the
/// controls to its right (Keynote inspector layout).
pub fn row<R>(ui: &mut Ui, label: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let p = pal(ui);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(LABEL_W, H_REGULAR), Sense::hover());
        if !label.is_empty() {
            ui.painter().text(Pos2::new(r.right() - 2.0, r.center().y), Align2::RIGHT_CENTER, label, style::ui_font(11.0), p.weak);
        }
        add_contents(ui)
    })
    .inner
}

/// Full-width hairline separator.
pub fn separator(ui: &mut Ui) {
    let p = pal(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0, p.border));
}

/// 11 pt secondary text (captions, help under a control).
pub fn caption(ui: &mut Ui, text: impl Into<String>) -> Response {
    let p = pal(ui);
    let label = egui::Label::new(egui::RichText::new(text.into()).font(style::ui_font(11.0)).color(p.weak)).wrap();
    // In a menu or popover (a justified column) the words would be spread
    // to both edges, with wide gaps: captions are always set flush left.
    if ui.layout().is_vertical() && ui.layout().horizontal_justify() {
        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| ui.add(label)).inner
    } else {
        ui.add(label)
    }
}

/// Small grid of equal-width push buttons, `cols` per row (Keynote's
/// Arrange buttons). Returns the index clicked.
pub fn button_grid(ui: &mut Ui, labels: &[(&str, &str, bool)], cols: usize) -> Option<usize> {
    let mut out = None;
    let w = ((ui.available_width() - 6.0 * (cols as f32 - 1.0)) / cols as f32).floor();
    for chunk in labels.chunks(cols).enumerate() {
        ui.horizontal(|ui| {
            for (j, (l, tip, en)) in chunk.1.iter().enumerate() {
                let r = push_button_ext(ui, l, Size::Regular, *en, Some(w));
                let r = if tip.is_empty() { r } else { r.on_hover_text(*tip) };
                if r.clicked() {
                    out = Some(chunk.0 * cols + j);
                }
            }
        });
    }
    out
}

// ----------------------------------------------------------------------
// Transient messages: the HUD bezel, and a note under the toolbar about
// what just happened on the canvas. No web toasts.

/// A short note centred just under the top of `rect` (the canvas, so just
/// under the toolbar), in the style of the info bars there: one line in a
/// floating panel. `alpha` fades it out.
pub fn note_bar(ctx: &egui::Context, rect: Rect, text: &str, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let p = Palette::of(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("zebra-note-bar")));
    let galley = painter.layout(text.to_string(), style::ui_font(13.0), p.text.gamma_multiply(alpha), (rect.width() - 64.0).max(120.0));
    let size = galley.size() + Vec2::new(24.0, 16.0);
    let r = Rect::from_min_size(Pos2::new(rect.center().x - size.x / 2.0, rect.top() + 12.0), size);
    painter.rect_filled(r.translate(Vec2::new(0.0, 2.0)), CornerRadius::same(6), Color32::from_black_alpha(((if p.dark { 90.0 } else { 30.0 }) * alpha) as u8));
    painter.rect(r, CornerRadius::same(6), p.float.gamma_multiply(alpha), Stroke::new(1.0, p.border.gamma_multiply(alpha)), StrokeKind::Inside);
    painter.galley(r.min + Vec2::new(12.0, 8.0), galley, p.text);
}

/// The volume-bezel idiom: centred, rounded, dark and opaque in this style,
/// over `rect`. `alpha` fades it out.
pub fn hud(ctx: &egui::Context, rect: Rect, text: &str, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::unique("zebra-hud")));
    let font = style::ui_font(13.0);
    let max_w = 380.0;
    let galley = painter.layout(text.to_string(), font, Color32::WHITE.gamma_multiply(alpha), max_w);
    let size = galley.size() + Vec2::new(32.0, 20.0);
    let r = Rect::from_center_size(Pos2::new(rect.center().x, rect.bottom() - 70.0 - size.y / 2.0), size);
    painter.rect_filled(r.translate(Vec2::new(0.0, 2.0)), CornerRadius::same(10), Color32::from_black_alpha((50.0 * alpha) as u8));
    painter.rect_filled(r, CornerRadius::same(10), Color32::from_rgba_unmultiplied(0x26, 0x26, 0x28, (240.0 * alpha) as u8));
    painter.galley(r.min + Vec2::new(16.0, 10.0), galley, Color32::WHITE);
}
