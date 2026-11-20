//! Drag selected words out of a text box, or to another place in it
//! (spec 18c), as in TextEdit: press on the selection and move more than
//! four points. A click without moving places the caret.
//!
//! egui's TextEdit has no drag and drop of its own, so while words are
//! selected the selection gets widgets of its own, on top of the text
//! field, that take the press before the field sees it.
//!
//! During the drag a faded copy of the words follows the pointer at the
//! box's on-screen size, and the words stay in the box, dimmed. Over the
//! canvas a thin outline shows where the new box will land; over the box
//! itself the insertion caret shows where the words will move to. Esc
//! cancels; Option held at the drop copies instead of moving.
//!
//! A drop becomes zebra-core commands, one undo step each: `split_text`
//! for a new box, an `update` named Move Text for a move inside the box.
use egui::text::{CCursor, CCursorRange, Galley, LayoutJob};
use egui::{CursorIcon, Id, LayerId, Order, Pos2, Rect as ERect, Sense, Stroke, StrokeKind, Ui};
use zebra_core::{rich, Command, ElementId, ElementKind, ElementPatch, Origin, TextData};

use crate::actions::Action;
use crate::app::ZebraApp;
use crate::canvas::TextEditState;

/// How far the pointer moves, in points, before a press on the selection
/// becomes a drag.
pub const START_AFTER: f32 = 4.0;

/// How faint the dragged copy is drawn, and the words left in the box.
const COPY_ALPHA: f32 = 0.55;
pub const DIM_ALPHA: f32 = 0.3;

/// A press on the selected words, and the drag it may become.
#[derive(Debug, Clone, PartialEq)]
pub struct TextDrag {
    /// The words, in characters.
    pub sel: (usize, usize),
    /// Where the button went down (screen).
    pub origin: Pos2,
    /// From the faded copy's top-left to the pointer.
    pub grab: egui::Vec2,
    /// Moved far enough to be a drag (until then it may be a click).
    pub moving: bool,
    /// Esc pressed: nothing happens when the button comes up.
    pub cancelled: bool,
    /// Where the words would land now.
    pub target: Target,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// Off the canvas, or on the words themselves: nothing would happen.
    Nowhere,
    /// A new box, this outline on screen.
    NewBox(ERect),
    /// The same box, before this character.
    Inside(usize),
}

/// What a drop asks for. Done after the text field is drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum Drop {
    NewBox { sel: (usize, usize), rect: ERect, copy: bool },
    Inside { sel: (usize, usize), to: usize, copy: bool },
}

/// The text field as drawn this frame.
pub struct Field<'a> {
    pub galley: &'a Galley,
    /// Where the galley is painted (screen).
    pub origin: Pos2,
    /// The box's outline on screen: a drop inside it moves the words in it.
    pub rect: ERect,
    pub canvas: ERect,
    /// The TextEdit's id.
    pub id: Id,
    /// The selection the field has now.
    pub sel: (usize, usize),
    /// Wrap width on screen of a fixed-width box.
    pub wrap: Option<f32>,
}

#[derive(Default)]
pub struct Outcome {
    pub drop: Option<Drop>,
    /// Move to New Text Box was chosen from the selection's menu.
    pub move_out: bool,
    /// The pointer is on the selection, or a drag or its menu is going on:
    /// the field must keep the keys (the press took them away).
    pub keep_focus: bool,
}

/// Characters `a..b` of the text being typed, as a box of their own.
fn piece(t: &TextEditState, a: usize, b: usize) -> TextData {
    let mut r = t.rich.clone();
    rich::edit_text(&mut r, &t.buf);
    rich::slice(&r, a, b)
}

/// The selection holds words (not just spaces): it can be dragged out.
pub fn has_words(t: &TextEditState) -> bool {
    let (a, b) = t.sel;
    a < b && !piece(t, a, b).text.is_empty()
}

/// One rectangle per row the characters `a..b` cover, on screen.
pub fn selection_rects(g: &Galley, origin: Pos2, a: usize, b: usize) -> Vec<ERect> {
    let mut out = vec![];
    let mut at = 0usize;
    for row in &g.rows {
        let n = row.glyphs.len();
        let (lo, hi) = (a.max(at), b.min(at + n));
        if lo < hi {
            let x0 = row.x_offset(egui::text::CharIndex(lo - at));
            let x1 = row.x_offset(egui::text::CharIndex(hi - at));
            let r = row.rect();
            out.push(ERect::from_min_max(Pos2::new(r.min.x + x0, r.min.y), Pos2::new(r.min.x + x1, r.max.y)).translate(origin.to_vec2()));
        }
        at += n + usize::from(row.ends_with_newline);
    }
    out
}

/// Fade the characters `range` (all with `None`) of a layout to `alpha`.
pub fn fade(job: &mut LayoutJob, range: Option<(usize, usize)>, alpha: f32) {
    let text = job.text.clone();
    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(b, _)| b);
    let (ba, bb) = range.map_or((0, text.len()), |(a, b)| (byte(a), byte(b)));
    if ba >= bb {
        return;
    }
    let mut sections = Vec::with_capacity(job.sections.len() + 2);
    for s in job.sections.drain(..) {
        let (x, y) = (s.byte_range.start.0, s.byte_range.end.0);
        let cuts = [x, ba.clamp(x, y), bb.clamp(x, y), y];
        for (i, w) in cuts.windows(2).enumerate() {
            if w[0] >= w[1] {
                continue;
            }
            let mut part = s.clone();
            part.byte_range = egui::text::ByteIndex(w[0])..egui::text::ByteIndex(w[1]);
            if w[0] != x {
                part.leading_space = 0.0;
            }
            if i == 1 {
                part.format.color = part.format.color.gamma_multiply(alpha);
                part.format.underline.color = part.format.underline.color.gamma_multiply(alpha);
                part.format.strikethrough.color = part.format.strikethrough.color.gamma_multiply(alpha);
            }
            sections.push(part);
        }
    }
    job.sections = sections;
}

/// The selection's own widgets, the drag and the drop. `layout` lays out a
/// piece of the text as the field draws it (for the faded copy).
pub fn interact(ui: &mut Ui, t: &mut TextEditState, f: &Field, layout: &dyn Fn(&TextData) -> LayoutJob) -> Outcome {
    let mut out = Outcome::default();
    let ctx = ui.ctx().clone();
    let (a, b) = t.drag.as_ref().map_or(f.sel, |d| d.sel);
    let words = a < b && !piece(t, a, b).text.is_empty();

    // Widgets over the selected words, on top of the field.
    let mut rows = vec![];
    if words {
        for (i, r) in selection_rects(f.galley, f.origin, a, b).into_iter().enumerate() {
            rows.push(ui.interact(r, f.id.with(("selected-words", i)), Sense::click_and_drag()));
        }
    }
    // Over a selection the Mac shows the arrow, not the I-beam.
    if t.drag.is_none() && rows.iter().any(|r| r.hovered()) {
        ctx.set_cursor_icon(CursorIcon::Default);
    }
    out.keep_focus = rows.iter().any(|r| r.contains_pointer() || r.is_pointer_button_down_on());

    // Right-click on the selection: its menu.
    let mut menu = false;
    for r in &rows {
        egui::Popup::context_menu(r).style(crate::widgets::menu_style).width(200.0).show(|ui| {
            menu = true;
            if crate::widgets::menu_item(ui, &Action::MoveToNewTextBox.label(), None).clicked() {
                out.move_out = true;
                ui.close();
            }
        });
    }
    t.sel_menu = menu || rows.iter().any(|r| r.secondary_clicked());
    out.keep_focus |= t.sel_menu;

    // The press.
    let pressed = ui.input(|i| i.pointer.primary_pressed());
    if t.drag.is_none() && pressed && rows.iter().any(|r| r.is_pointer_button_down_on()) {
        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or_default();
        let first = (a..b).find(|&i| t.buf.chars().nth(i).is_some_and(|c| !c.is_whitespace())).unwrap_or(a);
        let tl = f.galley.pos_from_cursor(CCursor::new(first)).min + f.origin.to_vec2();
        t.drag = Some(TextDrag { sel: (a, b), origin, grab: origin - tl, moving: false, cancelled: false, target: Target::Nowhere });
    }

    t.on_words = out.keep_focus || t.drag.is_some();
    let Some(mut d) = t.drag.take() else { return out };
    out.keep_focus = true;
    ctx.request_repaint();
    let (p, down, alt) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down(), i.modifiers.alt));
    let p = p.unwrap_or(d.origin);
    if !d.moving && !d.cancelled && p.distance(d.origin) > START_AFTER {
        d.moving = true;
    }
    if d.moving && !d.cancelled {
        let words = piece(t, d.sel.0, d.sel.1);
        let mut job = layout(&words);
        fade(&mut job, None, COPY_ALPHA);
        let copy = ctx.fonts_mut(|fo| fo.layout_job(job));
        let tl = p - d.grab;
        let size = egui::vec2(f.wrap.unwrap_or(copy.size().x), copy.size().y);
        d.target = if f.rect.contains(p) {
            Target::Inside(usize::from(f.galley.cursor_from_pos(p - f.origin).index))
        } else if f.canvas.contains(p) {
            Target::NewBox(ERect::from_min_size(tl, size))
        } else {
            Target::Nowhere
        };
        let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, f.id.with("dragged-words")));
        painter.galley(tl, copy, egui::Color32::PLACEHOLDER);
        match d.target {
            Target::NewBox(r) => {
                painter.rect_stroke(r.expand(3.0), 0.0, Stroke::new(1.0, crate::style::accent()), StrokeKind::Outside);
            }
            Target::Inside(i) if !(d.sel.0..=d.sel.1).contains(&i) => {
                // The insertion caret, over the faded copy.
                let c = f.galley.pos_from_cursor(CCursor::new(i)).translate(f.origin.to_vec2());
                painter.line_segment([c.center_top(), c.center_bottom()], Stroke::new(2.0, crate::style::accent()));
            }
            _ => {}
        }
        // Option copies: the green plus.
        ctx.set_cursor_icon(if alt { CursorIcon::Copy } else { CursorIcon::Default });
    }
    if down {
        t.drag = Some(d);
        return out;
    }
    // The button came up.
    if d.cancelled {
        return out;
    }
    if !d.moving {
        // A click: the caret goes there, as in TextEdit.
        let c = f.galley.cursor_from_pos(d.origin - f.origin);
        if let Some(mut st) = egui::TextEdit::load_state(&ctx, f.id) {
            st.cursor.set_char_range(Some(CCursorRange::one(c)));
            st.store(&ctx, f.id);
        }
        let i = usize::from(c.index);
        t.sel = (i, i);
        return out;
    }
    out.drop = match d.target {
        Target::NewBox(rect) => Some(Drop::NewBox { sel: d.sel, rect, copy: alt }),
        Target::Inside(to) if !(d.sel.0..=d.sel.1).contains(&to) => Some(Drop::Inside { sel: d.sel, to, copy: alt }),
        _ => None,
    };
    out
}

/// Esc during a drag: cancel it (and nothing else). True when it did.
pub fn cancel(app: &mut ZebraApp) -> bool {
    match app.canvas.text.as_mut().and_then(|t| t.drag.as_mut()) {
        Some(d) if !d.cancelled => {
            d.cancelled = true;
            true
        }
        _ => false,
    }
}

/// Words of `id` into a new box at `at` (world centre), or just below.
fn split(app: &mut ZebraApp, id: ElementId, (a, b): (usize, usize), at: Option<zebra_core::Vec2>, copy: bool) {
    let n = match app.editor.doc.get(&id).map(|e| &e.kind) {
        Some(ElementKind::Text(t)) => t.text.chars().count(),
        _ => return,
    };
    let (a, b) = (a.min(n), b.min(n));
    if a >= b {
        return;
    }
    let cmd = Command::SplitText { id, start: a, end: b, x: at.map(|c| c.x), y: at.map(|c| c.y), copy };
    if let Some(out) = app.apply(cmd) {
        // The new box, selected, not being edited.
        app.select(out.created);
    }
}

/// Do a drop.
pub fn apply(app: &mut ZebraApp, drop: Drop) {
    match drop {
        Drop::NewBox { sel, rect, copy } => {
            let Some(id) = crate::canvas::commit_text_edit(app) else { return };
            let at = app.canvas.s2w(&app.editor, rect.center());
            split(app, id, sel, Some(at), copy);
        }
        Drop::Inside { sel, to, copy } => {
            let Some(id) = crate::canvas::commit_text_edit(app) else { return };
            let Some(ElementKind::Text(old)) = app.editor.doc.get(&id).map(|e| e.kind.clone()) else { return };
            let mut new = old.clone();
            let moved = rich::move_within(&mut new, sel.0, sel.1, to, copy);
            if moved.is_some() {
                let patch = ElementPatch { text: Some(new.text), spans: Some(new.spans), paras: Some(new.paras), ..Default::default() };
                let label = if copy { crate::t!("history-copy-text") } else { crate::t!("history-move-text") };
                let r = app.editor.apply_batch(vec![Command::Update { id: id.clone(), patch }], Some(label), Origin::Human);
                app.report(r);
            }
            // Typing goes on, the words still selected where they landed.
            crate::canvas::begin_edit_existing(app, id);
            if let Some(t) = app.canvas.text.as_mut() {
                t.pending_sel = Some(moved.unwrap_or(sel));
            }
        }
    }
}

/// Format › Text › Move to New Text Box: the selected words into a new
/// box just below this one.
pub fn move_to_new_box(app: &mut ZebraApp) {
    let Some(sel) = app.canvas.text.as_ref().filter(|t| has_words(t)).map(|t| t.sel) else { return };
    let Some(id) = crate::canvas::commit_text_edit(app) else { return };
    split(app, id, sel, None, false);
}
