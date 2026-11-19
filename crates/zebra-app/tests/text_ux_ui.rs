//! Spec 14: Prezi's text width handle, and Tab / Backspace in lists and
//! indented paragraphs as in TextEdit and Pages.
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, ElementKind, ListKind, Origin, ParaFormat, TextData, Vec2};

const H: Origin = Origin::Human;

fn harness() -> Harness<'static, ZebraApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h
}

const WORDS: &str = "The quick brown fox jumps over the lazy dog";

/// A text box with `WORDS` at the middle of the view, at `scale` and
/// `rotation`, selected.
fn text_box(h: &mut Harness<'static, ZebraApp>, scale: f64, rotation: f64) -> String {
    let c = {
        let a = h.state();
        a.canvas.s2w(&a.editor, a.canvas.rect.center())
    };
    let id = {
        let app = h.state_mut();
        let out = app
            .editor
            .apply(Command::AddText { text: WORDS.into(), x: c.x, y: c.y, style: Default::default(), scale: Some(scale / app.editor.view.camera.zoom), place: None, id: None, markdown: false, on: None, color: None }, H)
            .unwrap();
        let id = out.created[0].clone();
        if rotation != 0.0 {
            app.editor.apply(Command::Rotate { ids: vec![id.clone()], degrees: rotation, about: None, with_contents: false }, H).unwrap();
        }
        app.select(vec![id.clone()]);
        id
    };
    h.run_steps(3);
    id.to_string()
}

fn text(h: &Harness<'static, ZebraApp>, id: &str) -> (zebra_core::Element, TextData) {
    let e = h.state().editor.doc.get(id).unwrap().clone();
    let ElementKind::Text(t) = &e.kind else { panic!("not a text") };
    let t = t.clone();
    (e, t)
}

fn w2s(h: &Harness<'static, ZebraApp>, p: Vec2) -> Pos2 {
    let a = h.state();
    a.canvas.w2s(&a.editor, p)
}

/// The middle of the box's left (`false`) or right (`true`) edge, on screen.
fn handle(h: &Harness<'static, ZebraApp>, id: &str, right: bool) -> Pos2 {
    let c = h.state().editor.doc.get(id).unwrap().corners();
    let (a, b) = if right { (c[1], c[2]) } else { (c[0], c[3]) };
    w2s(h, (a + b) * 0.5)
}

fn press(h: &mut Harness<'static, ZebraApp>, p: Pos2) {
    h.event(Event::PointerMoved(p));
    h.step();
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
}

fn move_to(h: &mut Harness<'static, ZebraApp>, from: Pos2, to: Pos2) {
    for i in 1..=10 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 10.0)));
        h.step();
    }
}

fn release(h: &mut Harness<'static, ZebraApp>, p: Pos2) {
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    h.step();
}

fn drag(h: &mut Harness<'static, ZebraApp>, from: Pos2, to: Pos2) {
    press(h, from);
    move_to(h, from, to);
    release(h, to);
}

fn shot(h: &mut Harness<'static, ZebraApp>, name: &str) {
    if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
        h.render().unwrap().save(std::path::Path::new(&dir).join(name)).unwrap();
    }
}

fn near(a: Vec2, b: Vec2, tol: f64) -> bool {
    (a - b).len() < tol
}

/// Dragging the right width handle in: the text wraps there and grows
/// taller, its left edge stays put, it is one undo step, and Undo gives
/// back the box that grew with its words.
#[test]
fn the_width_handle_wraps_text_in_one_undo_step() {
    let mut h = harness();
    let id = text_box(&mut h, 1.0, 0.0);
    let (e0, t0) = text(&h, &id);
    assert!(!t0.fixed_width, "a new text grows with its words");
    shot(&mut h, "text-width-handles.png");
    let undo = h.state().editor.history().len();
    let from = handle(&h, &id, true);
    let left = handle(&h, &id, false);
    let to = Pos2::new(left.x + (from.x - left.x) * 0.4, from.y);
    // Hovering the handle shows the sideways resize cursor.
    h.event(Event::PointerMoved(from));
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::ResizeHorizontal);
    press(&mut h, from);
    move_to(&mut h, from, to);
    // Live while dragging.
    let (mid, tm) = text(&h, &id);
    assert!(tm.fixed_width && mid.size.h > e0.size.h * 1.5, "wraps while dragging: {:?} -> {:?}", e0.size, mid.size);
    shot(&mut h, "text-width-mid-drag.png");
    release(&mut h, to);
    let (e1, t1) = text(&h, &id);
    let zoom = h.state().editor.view.camera.zoom;
    let want = (to.x - left.x) as f64 / zoom / e1.transform.scale;
    assert!(t1.fixed_width);
    assert!((e1.size.w - want).abs() < 2.0, "wraps at the handle: {} vs {want}", e1.size.w);
    assert!(e1.size.h > e0.size.h * 1.5, "taller: {} -> {}", e0.size.h, e1.size.h);
    assert!(near(e1.corners()[0], e0.corners()[0], 0.5 / zoom), "the top-left corner stays: {:?} vs {:?}", e1.corners()[0], e0.corners()[0]);
    assert_eq!(h.state().editor.history().len(), undo + 1, "one undo step");
    assert_eq!(h.state().editor.history().last().unwrap().label, zebra_app::i18n::tr("canvas-undo-text-width"));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    let (e2, t2) = text(&h, &id);
    assert!(!t2.fixed_width, "undo gives back auto width");
    assert_eq!(e2.size, e0.size);
    assert!(near(e2.transform.center(), e0.transform.center(), 1e-9));
}

/// The handle works in the box's own units: on a rotated, scaled box the
/// drag along its side sets the width and keeps the opposite corner; a
/// width never goes below about one letter.
#[test]
fn the_width_handle_respects_rotation_and_scale() {
    let mut h = harness();
    let id = text_box(&mut h, 0.7, 90.0);
    let (e0, _) = text(&h, &id);
    assert!((e0.transform.scale - 1.0).abs() > 0.1, "a scaled box: {}", e0.transform.scale);
    let zoom = h.state().editor.view.camera.zoom;
    // Left handle: the right edge (top-right corner) stays put.
    let from = handle(&h, &id, false);
    let along = (w2s(&h, e0.corners()[1]) - w2s(&h, e0.corners()[0])).normalized();
    assert!(along.y > 0.9, "rotated 90°: the box runs down the screen");
    let d = 120.0;
    drag(&mut h, from, from + along * d);
    let (e1, t1) = text(&h, &id);
    let want = e0.size.w - d as f64 / zoom / e0.transform.scale;
    assert!(t1.fixed_width);
    assert!((e1.size.w - want).abs() < 1.0, "local width {} vs {want}", e1.size.w);
    assert!(near(e1.corners()[1], e0.corners()[1], 0.5 / zoom), "the top-right corner stays");
    assert_eq!(e1.transform.rotation, e0.transform.rotation);
    assert_eq!(e1.transform.scale, e0.transform.scale);
    // Far past the other side: about one letter wide, not negative.
    let from = handle(&h, &id, true);
    drag(&mut h, from, from - along * 2000.0);
    let (e2, _) = text(&h, &id);
    let body = h.state().editor.theme().text_style(zebra_core::TextStyle::Body).size;
    assert!(e2.size.w > 0.0 && (e2.size.w - body * 0.6).abs() < 1e-6, "one letter wide: {}", e2.size.w);
    assert!(near(e2.corners()[0], e1.corners()[0], 0.5 / zoom), "the top-left corner stays");
}

/// Width handles only for a single text box.
#[test]
fn no_width_handles_on_a_multi_selection() {
    let mut h = harness();
    let a = text_box(&mut h, 1.0, 0.0);
    let (e, _) = text(&h, &a);
    let b = {
        let app = h.state_mut();
        let c = e.transform.center();
        let out = app.editor.apply(Command::AddText { text: "Other".into(), x: c.x, y: c.y + e.size.h * e.transform.scale * 3.0, style: Default::default(), scale: Some(e.transform.scale), place: None, id: None, markdown: false, on: None, color: None }, H).unwrap();
        out.created[0].clone()
    };
    h.state_mut().select(vec![a.clone().into(), b]);
    h.run_steps(2);
    let undo = h.state().editor.history().len();
    let from = handle(&h, &a, true);
    drag(&mut h, from, from - egui::vec2(100.0, 0.0));
    let (_, t) = text(&h, &a);
    assert!(!t.fixed_width, "no width change from a multi-selection");
    assert_eq!(h.state().editor.history().len(), undo + 1, "it was a move");
}

/// While typing, the box shows the handles too; dragging one sets the
/// wrap width of the new text it makes.
#[test]
fn the_width_handle_works_while_typing_a_new_text() {
    let mut h = harness();
    let c = h.state().canvas.rect.center();
    zebra_app::canvas::begin_new_text(h.state_mut(), c);
    h.run_steps(2);
    h.event(Event::Text(WORDS.into()));
    h.run_steps(3);
    let (left, right, y, w0) = {
        let a = h.state();
        let t = a.canvas.text.as_ref().unwrap();
        assert!(t.wrap.is_none(), "a new box grows while typing");
        let tl = a.canvas.w2s(&a.editor, t.anchor);
        (tl.x, tl.x + t.size_px.x, tl.y + t.size_px.y / 2.0, t.size_px.x)
    };
    let from = Pos2::new(right + 4.0, y);
    let to = Pos2::new(left + w0 * 0.4 + 4.0, y);
    drag(&mut h, from, to);
    {
        let a = h.state();
        let t = a.canvas.text.as_ref().expect("still typing after the drag");
        let px = (t.scale * a.editor.view.camera.zoom) as f32;
        let wrap = t.wrap.expect("a wrap width") as f32 * px;
        assert!((wrap - w0 * 0.4).abs() < 2.0, "wraps at the handle: {wrap} vs {}", w0 * 0.4);
        assert!(t.size_px.y > 40.0, "more than one line: {:?}", t.size_px);
    }
    shot(&mut h, "text-width-while-typing.png");
    h.key_press(Key::Escape);
    h.run_steps(3);
    let e = h.state().editor.doc.ordered().next().unwrap().clone();
    let ElementKind::Text(t) = &e.kind else { panic!() };
    assert!(t.fixed_width && t.text == WORDS);
    assert_eq!(h.state().editor.history().len(), 1, "adding the text is one undo step");
}

/// Editing a text already on the canvas: a drag of its handle is its own
/// undo step and the typing goes on.
#[test]
fn the_width_handle_works_while_editing_a_text() {
    let mut h = harness();
    let id = text_box(&mut h, 1.0, 0.0);
    let (e0, _) = text(&h, &id);
    zebra_app::canvas::begin_edit_existing(h.state_mut(), id.clone().into());
    h.run_steps(3);
    let undo = h.state().editor.history().len();
    let from = handle(&h, &id, true) + egui::vec2(4.0, 0.0);
    let left = handle(&h, &id, false);
    drag(&mut h, from, Pos2::new(left.x + (from.x - left.x) * 0.5, from.y));
    assert!(h.state().canvas.text.is_some(), "still editing");
    let (e1, t1) = text(&h, &id);
    assert!(t1.fixed_width && e1.size.w < e0.size.w * 0.6);
    let labels: Vec<_> = h.state().editor.history().iter().map(|e| e.label.clone()).collect();
    assert_eq!(h.state().editor.history().len(), undo + 1, "{labels:?}");
    assert_eq!(h.state().canvas.text.as_ref().unwrap().wrap, Some(e1.size.w));
    h.event(Event::Text("!".into()));
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(3);
    let (_, t2) = text(&h, &id);
    assert!(t2.fixed_width && t2.text.ends_with('!'));
}

/// A text with paragraphs `paras`, being edited, the caret at the end.
fn editing(h: &mut Harness<'static, ZebraApp>, body: &str, paras: Vec<ParaFormat>) -> String {
    let id = text_box(h, 1.0, 0.0);
    let patch = zebra_core::ElementPatch { text: Some(body.into()), paras: Some(paras), ..Default::default() };
    h.state_mut().editor.apply(Command::Update { id: id.clone().into(), patch }, H).unwrap();
    zebra_app::canvas::begin_edit_existing(h.state_mut(), id.clone().into());
    h.run_steps(3);
    id
}

fn key(h: &mut Harness<'static, ZebraApp>, k: Key) {
    h.key_press(k);
    h.run_steps(2);
}

fn para(h: &Harness<'static, ZebraApp>, n: usize) -> (ListKind, u8) {
    let t = h.state().canvas.text.as_ref().unwrap();
    let mut r = t.rich.clone();
    zebra_core::rich::edit_text(&mut r, &t.buf);
    let (l, i, _) = zebra_core::rich::para(&r, n);
    (l, i)
}

fn buf(h: &Harness<'static, ZebraApp>) -> String {
    h.state().canvas.text.as_ref().unwrap().buf.clone()
}

/// Backspace at the start of a list item drops the marker first, then
/// outdents a level at a time, and only at level 0 joins the paragraph to
/// the one before. Committed, the whole edit is one undo step.
#[test]
fn backspace_at_the_start_of_a_list_item_outdents() {
    let mut h = harness();
    let id = editing(&mut h, "first\nsecond", vec![ParaFormat::default(), ParaFormat { indent: 1, list: Some(ListKind::Bullet), align: None }]);
    // To the start of "second".
    key(&mut h, Key::Home);
    assert_eq!(para(&h, 1), (ListKind::Bullet, 1));
    key(&mut h, Key::Backspace);
    assert_eq!(buf(&h), "first\nsecond", "the text is kept");
    assert_eq!(para(&h, 1), (ListKind::None, 1), "the marker goes first");
    key(&mut h, Key::Backspace);
    assert_eq!(para(&h, 1), (ListKind::None, 0), "then one level out");
    assert_eq!(buf(&h), "first\nsecond");
    key(&mut h, Key::Backspace);
    assert_eq!(buf(&h), "firstsecond", "at level 0 it joins");
    let undo = h.state().editor.history().len();
    key(&mut h, Key::Escape);
    assert_eq!(text(&h, &id).1.text, "firstsecond");
    assert_eq!(h.state().editor.history().len(), undo + 1, "one undo step");
}

/// Backspace in the middle of an indented paragraph deletes a letter as
/// usual; Shift-Tab outdents.
#[test]
fn backspace_mid_paragraph_deletes_and_shift_tab_outdents() {
    let mut h = harness();
    editing(&mut h, "first\nsecond", vec![ParaFormat::default(), ParaFormat { indent: 2, list: None, align: None }]);
    key(&mut h, Key::Backspace);
    assert_eq!(buf(&h), "first\nsecon");
    assert_eq!(para(&h, 1), (ListKind::None, 2));
    h.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
    h.run_steps(2);
    assert_eq!(para(&h, 1), (ListKind::None, 1));
}

/// Tab in the middle of a line of a plain paragraph types spaces (the
/// canvas text has no tab stops); at the start of a paragraph it indents.
#[test]
fn tab_mid_line_types_spaces() {
    let mut h = harness();
    editing(&mut h, "first\nsecond", vec![]);
    key(&mut h, Key::ArrowLeft);
    key(&mut h, Key::ArrowLeft);
    key(&mut h, Key::Tab);
    assert_eq!(buf(&h), "first\nseco    nd");
    assert_eq!(para(&h, 1), (ListKind::None, 0), "not indented");
}

#[test]
fn tab_at_line_start_indents() {
    let mut h = harness();
    editing(&mut h, "first\nsecond", vec![]);
    for _ in 0..6 {
        key(&mut h, Key::ArrowLeft);
    }
    key(&mut h, Key::Tab);
    assert_eq!(buf(&h), "first\nsecond");
    assert_eq!(para(&h, 1), (ListKind::None, 1));
}
