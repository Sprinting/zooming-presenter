//! Spec 18c: drag selected words out of a text box, or to another place in
//! it, as in TextEdit; Option copies; Esc cancels; a click without moving
//! places the caret. And Format › Text › Move to New Text Box.
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use zebra_app::actions::Action;
use zebra_app::i18n::tr;
use zebra_app::text_drag::Target;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, ElementKind, Origin, TextData};

const H: Origin = Origin::Human;
const WORDS: &str = "The quick brown fox jumps";

fn harness() -> Harness<'static, ZebraApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h
}

fn range_of(s: &str, needle: &str) -> (usize, usize) {
    let b = s.find(needle).unwrap();
    let a = s[..b].chars().count();
    (a, a + needle.chars().count())
}

/// A text box with `WORDS` a little above the middle of the view, being
/// edited, with `sel` selected. Its id.
fn editing(h: &mut Harness<'static, ZebraApp>, sel: &str) -> String {
    let c = {
        let a = h.state();
        a.canvas.s2w(&a.editor, a.canvas.rect.center() - egui::vec2(0.0, 120.0))
    };
    let id = {
        let app = h.state_mut();
        let scale = 1.0 / app.editor.view.camera.zoom;
        let out = app.editor.apply(Command::AddText { text: WORDS.into(), x: c.x, y: c.y, style: Default::default(), scale: Some(scale), place: None, id: None, on: None, color: None }, H).unwrap();
        out.created[0].clone()
    };
    h.run_steps(2);
    zebra_app::canvas::begin_edit_existing(h.state_mut(), id.clone());
    h.state_mut().canvas.text.as_mut().unwrap().pending_sel = Some(range_of(WORDS, sel));
    h.run_steps(3);
    assert_eq!(h.state().canvas.text.as_ref().unwrap().sel, range_of(WORDS, sel), "the words are selected");
    id
}

fn text(h: &Harness<'static, ZebraApp>, id: &str) -> TextData {
    match &h.state().editor.doc.get(id).unwrap_or_else(|| panic!("no {id}")).kind {
        ElementKind::Text(t) => t.clone(),
        _ => panic!("{id} is not a text"),
    }
}

/// Where character `i` of the text being typed is on screen (the middle
/// of the caret there).
fn char_at(h: &Harness<'static, ZebraApp>, i: usize) -> Pos2 {
    let t = h.state().canvas.text.as_ref().expect("typing");
    let (g, o) = t.laid_out.as_ref().expect("laid out");
    g.pos_from_cursor(egui::text::CCursor::new(i)).center() + o.to_vec2()
}

/// The middle of the selected words.
fn selection_middle(h: &Harness<'static, ZebraApp>) -> Pos2 {
    let (a, b) = h.state().canvas.text.as_ref().unwrap().sel;
    let (pa, pb) = (char_at(h, a), char_at(h, b));
    Pos2::new((pa.x + pb.x) / 2.0, pa.y)
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

fn release(h: &mut Harness<'static, ZebraApp>, p: Pos2, modifiers: Modifiers) {
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers });
    h.step();
    h.step();
}

fn shot(h: &mut Harness<'static, ZebraApp>, name: &str) {
    if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
        h.render().unwrap().save(std::path::Path::new(&dir).join(name)).unwrap();
    }
}

fn last_label(h: &Harness<'static, ZebraApp>) -> String {
    h.state().editor.history().last().map(|e| e.label.clone()).unwrap_or_default()
}

fn drag_state(h: &Harness<'static, ZebraApp>) -> zebra_app::text_drag::TextDrag {
    h.state().canvas.text.as_ref().expect("typing").drag.clone().expect("a drag")
}

/// Selected words dragged onto empty canvas: a faded copy follows the
/// pointer with the new box's outline, the words stay dimmed in the box;
/// at the drop they leave it for a new box there, selected, not being
/// edited, as one undo step named Move Text.
#[test]
fn dragging_words_onto_the_canvas_makes_a_new_box() {
    let mut h = harness();
    let id = editing(&mut h, "brown fox");
    let steps = h.state().editor.history().len();
    let from = selection_middle(&h);
    let to = from + egui::vec2(-60.0, 220.0);
    press(&mut h, from);
    move_to(&mut h, from, to);
    let d = drag_state(&h);
    assert!(d.moving && !d.cancelled);
    let Target::NewBox(outline) = d.target else { panic!("over the canvas: {:?}", d.target) };
    assert!(outline.contains(to), "the outline is under the pointer: {outline:?} {to:?}");
    assert_eq!(text(&h, &id).text, WORDS, "the words stay in the box until the drop");
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
    shot(&mut h, "drag-text-out-mid-drag.png");
    release(&mut h, to, Modifiers::NONE);

    let app = h.state();
    assert!(app.canvas.text.is_none(), "not in editing mode");
    let sel = app.editor.view.selection.clone();
    assert_eq!(sel.len(), 1);
    let new = sel[0].clone();
    assert_ne!(new, id);
    assert_eq!(text(&h, &new).text, "brown fox");
    assert_eq!(text(&h, &id).text, "The quick jumps", "one space left between the words");
    let at = app.canvas.s2w(&app.editor, outline.center());
    let c = app.editor.doc.get(&new).unwrap().transform.center();
    let zoom = app.editor.view.camera.zoom;
    assert!((c - at).len() * zoom < 2.0, "it lands where the outline was: {c:?} vs {at:?}");
    assert_eq!(app.editor.history().len(), steps + 1, "one undo step");
    assert_eq!(last_label(&h), tr("history-move-text"));
    assert_eq!(last_label(&h), "Move Text");
    shot(&mut h, "drag-text-out-after-drop.png");

    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert!(h.state().editor.doc.get(&new).is_none());
    assert_eq!(text(&h, &id).text, WORDS);
}

/// Option held at the drop copies instead (the green-plus cursor while
/// it's held).
#[test]
fn option_at_the_drop_copies() {
    let mut h = harness();
    let id = editing(&mut h, "quick");
    let from = selection_middle(&h);
    let to = from + egui::vec2(80.0, 240.0);
    press(&mut h, from);
    move_to(&mut h, from, to);
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.event(Event::PointerMoved(to + egui::vec2(1.0, 0.0)));
    h.step();
    h.step();
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Copy, "the copy cursor");
    shot(&mut h, "drag-text-out-option-copy.png");
    release(&mut h, to + egui::vec2(1.0, 0.0), Modifiers::ALT);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
    let new = h.state().editor.view.selection[0].clone();
    assert_eq!(text(&h, &new).text, "quick");
    assert_eq!(text(&h, &id).text, WORDS, "the source keeps its words");
    assert_eq!(last_label(&h), tr("history-copy-text"));
}

/// Over the box itself the insertion caret shows where the words go, and
/// the drop moves them there; typing goes on with them selected.
#[test]
fn dragging_inside_the_box_moves_the_words_there() {
    let mut h = harness();
    let id = editing(&mut h, "quick");
    let steps = h.state().editor.history().len();
    let from = selection_middle(&h);
    // Just before "jumps".
    let j = range_of(WORDS, "jumps").0;
    let to = char_at(&h, j) + egui::vec2(1.0, 0.0);
    press(&mut h, from);
    move_to(&mut h, from, to);
    let d = drag_state(&h);
    assert_eq!(d.target, Target::Inside(j), "the caret before jumps");
    shot(&mut h, "drag-text-reorder-caret.png");
    release(&mut h, to, Modifiers::NONE);
    h.run_steps(3);
    let t = h.state().canvas.text.as_ref().expect("still typing");
    assert_eq!(t.buf, "The brown fox quick jumps");
    assert_eq!(t.sel, range_of("The brown fox quick jumps", "quick"), "the moved words stay selected");
    assert_eq!(text(&h, &id).text, "The brown fox quick jumps");
    assert_eq!(h.state().editor.history().len(), steps + 1);
    assert_eq!(last_label(&h), tr("history-move-text"));
    assert_eq!(h.state().editor.doc.elements.len(), 1, "no new box");
    shot(&mut h, "drag-text-reorder-after-drop.png");
}

/// Esc during the drag cancels it: nothing moves when the button comes
/// up, and the words are still selected in the box being edited.
#[test]
fn esc_cancels_the_drag() {
    let mut h = harness();
    let id = editing(&mut h, "brown");
    let steps = h.state().editor.history().len();
    let from = selection_middle(&h);
    let to = from + egui::vec2(0.0, 200.0);
    press(&mut h, from);
    move_to(&mut h, from, to);
    h.key_press(Key::Escape);
    h.step();
    h.step();
    assert!(drag_state(&h).cancelled);
    release(&mut h, to, Modifiers::NONE);
    h.run_steps(2);
    let t = h.state().canvas.text.as_ref().expect("still typing: Esc ended only the drag");
    assert!(t.drag.is_none());
    assert_eq!(t.sel, range_of(WORDS, "brown"));
    assert_eq!(text(&h, &id).text, WORDS);
    assert_eq!(h.state().editor.doc.elements.len(), 1);
    assert_eq!(h.state().editor.history().len(), steps);
}

/// A click on the selection without moving (or moving less than 4
/// points) places the caret there, as in TextEdit.
#[test]
fn a_click_on_the_selection_places_the_caret() {
    let mut h = harness();
    let id = editing(&mut h, "quick brown");
    let steps = h.state().editor.history().len();
    let i = range_of(WORDS, "brown").0 + 2;
    let p = char_at(&h, i);
    press(&mut h, p);
    release(&mut h, p, Modifiers::NONE);
    h.run_steps(2);
    let t = h.state().canvas.text.as_ref().expect("still typing");
    assert_eq!(t.sel, (i, i), "the caret where the click was");
    assert!(t.drag.is_none());
    // Typing goes in there.
    h.event(Event::Text("X".into()));
    h.run_steps(2);
    assert_eq!(h.state().canvas.text.as_ref().unwrap().buf, "The quick brXown fox jumps");

    // Three points of wobble is still a click.
    let mut h = harness();
    editing(&mut h, "quick brown");
    let p = char_at(&h, i);
    press(&mut h, p);
    move_to(&mut h, p, p + egui::vec2(3.0, 0.0));
    release(&mut h, p + egui::vec2(3.0, 0.0), Modifiers::NONE);
    h.run_steps(2);
    let t = h.state().canvas.text.as_ref().expect("still typing");
    assert_eq!(t.sel.0, t.sel.1, "a caret, not a drag");
    assert_eq!(h.state().editor.doc.elements.len(), 1);
    let _ = (id, steps);
}

/// Format › Text › Move to New Text Box: on while typing with words
/// selected; the words go to a new box just below, one step, Move Text.
#[test]
fn the_menu_item_moves_the_words_to_a_box_below() {
    let mut h = harness();
    let state = |h: &Harness<'static, ZebraApp>| zebra_app::menus::state(h.state(), &Action::MoveToNewTextBox);
    assert!(!state(&h).enabled, "off with nothing being typed");
    assert_eq!(state(&h).title, "Move to New Text Box");
    let id = editing(&mut h, "brown fox");
    assert!(state(&h).enabled, "on with words selected");
    let steps = h.state().editor.history().len();
    h.state_mut().queued.push(Action::MoveToNewTextBox);
    h.run_steps(3);
    let app = h.state();
    assert!(app.canvas.text.is_none());
    let new = app.editor.view.selection[0].clone();
    assert_eq!(text(&h, &new).text, "brown fox");
    assert_eq!(text(&h, &id).text, "The quick jumps");
    let (o, n) = (app.editor.doc.get(&id).unwrap().bounds(), app.editor.doc.get(&new).unwrap().bounds());
    assert!(n.y0 > o.y1 && n.y0 - o.y1 < o.y1 - o.y0, "just below: {n:?} under {o:?}");
    assert_eq!(app.editor.history().len(), steps + 1);
    assert_eq!(last_label(&h), "Move Text");

    // Off with only a caret.
    let mut h = harness();
    editing(&mut h, "brown");
    h.state_mut().canvas.text.as_mut().unwrap().sel = (3, 3);
    assert!(!state(&h).enabled);
}

/// The edit-mode right-click menu on the selection has the item too.
#[test]
fn the_right_click_menu_on_the_selection_moves_the_words() {
    let mut h = harness();
    let id = editing(&mut h, "brown fox");
    let p = selection_middle(&h);
    h.event(Event::PointerMoved(p));
    h.step();
    h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let t = h.state().canvas.text.as_ref().expect("still typing with the menu open");
    assert_eq!(t.sel, range_of(WORDS, "brown fox"), "the selection stays");
    shot(&mut h, "drag-text-context-menu.png");
    h.get_by_label("Move to New Text Box").click();
    h.run_steps(4);
    assert!(h.state().canvas.text.is_none());
    let new = h.state().editor.view.selection[0].clone();
    assert_eq!(text(&h, &new).text, "brown fox");
    assert_eq!(text(&h, &id).text, "The quick jumps");
    assert_eq!(last_label(&h), "Move Text");
}

/// The Format menu's Text submenu, for the screenshot (the egui menu bar
/// stands in for the Mac's).
#[test]
fn the_format_menu_has_a_text_submenu() {
    let mut h = harness();
    editing(&mut h, "brown fox");
    // The menu bar's Format (the inspector has a Format tab too).
    h.get_all_by_label(&tr("menu-format")).next().unwrap().click();
    h.run_steps(2);
    // The submenu row (the toolbar and the inspector say Text too): the
    // one just under the Format menu's title.
    let fmt_x = h.get_all_by_label(&tr("menu-format")).next().unwrap().rect().center().x;
    h.get_all_by_label(&tr("menu-text")).filter(|n| n.rect().top() < 200.0 && (n.rect().left() - fmt_x).abs() < 120.0).next_back().expect("the Text submenu").hover();
    h.run_steps(4);
    assert!(h.query_by_label("Move to New Text Box").is_some());
    shot(&mut h, "drag-text-format-menu.png");
}
