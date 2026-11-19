//! Spec 08: the path sidebar's keyboard focus, inserting a step between
//! steps, and the sidebar bugs found on 19 Nov.
use egui::{Event, Key, Modifiers, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, Origin, StepTarget, ViewCommand};

const H: Origin = Origin::Human;

fn data_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("zebra-path-sidebar-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once, before any test of this binary starts a window.
        unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };
        dir
    })
    .clone()
}

fn harness() -> Harness<'static, ZebraApp> {
    data_dir();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 1000.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h
}

fn frame(w: &mut ZebraApp, x: f64, on_path: bool) -> String {
    let out = w.editor.apply(Command::AddFrame { style: Default::default(), x, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: None, add_to_path: on_path, find_space: false }, H).unwrap();
    out.created[0].to_string()
}

/// A long path: one frame per step, added in one go (fast to set up).
fn long_path(h: &mut Harness<'static, ZebraApp>, n: usize) {
    let cmds: Vec<Command> = (0..n)
        .map(|i| Command::AddFrame { style: Default::default(), x: i as f64 * 1000.0, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: None, add_to_path: true, find_space: false })
        .collect();
    h.state_mut().editor.apply_batch(cmds, None, H).unwrap();
    h.run_steps(3);
}

/// 19 Nov: step numbers of 1,000 and up ran into the thumbnail, the
/// number column being a fixed 22 points.
#[test]
fn the_number_column_fits_the_widest_step_number() {
    let mut h = harness();
    long_path(&mut h, 3);
    let short = h.state().step_number_w;
    assert_eq!(short, 22.0, "short paths keep the usual column");
    long_path(&mut h, 1197);
    assert_eq!(h.state().editor.doc.path.len(), 1200);
    let wide = h.state().step_number_w;
    let text = h.ctx.fonts_mut(|f| f.layout_no_wrap("8888".into(), zebra_app::style::semibold(11.0), egui::Color32::WHITE).size().x);
    assert!(wide >= text + 8.0, "column {wide} holds four digits ({text}) with room either side");
}

/// 19 Nov: an agent's goto_step flew the camera but left the sidebar where
/// it was; only a change of selection scrolled it.
#[test]
fn goto_step_from_an_agent_scrolls_the_sidebar() {
    let mut h = harness();
    long_path(&mut h, 300);
    let on = h.state().thumbs.showing();
    assert!(!on.contains(&240), "starts at the top: {on:?}");
    // Flying the person's camera is for agents with full permission.
    h.state_mut().editor.view.agent_permission = zebra_core::view::Permission::Full;
    h.state_mut().editor.view(ViewCommand::GotoStep { step: 240 }, Origin::agent("a")).unwrap();
    h.run_steps(4);
    let on = h.state().thumbs.showing();
    assert!(on.contains(&240), "scrolled to the step: {on:?}");
    // The person's own goto_step scrolls too.
    h.state_mut().editor.view(ViewCommand::GotoStep { step: 10 }, H).unwrap();
    h.run_steps(4);
    assert!(h.state().thumbs.showing().contains(&10));
}

/// 19 Nov: the step nearest the camera is found without measuring every
/// step on each camera change, and still agrees with doing so.
#[test]
fn the_nearest_step_agrees_with_measuring_every_step() {
    let mut h = harness();
    long_path(&mut h, 400);
    h.state_mut().view_cmd(ViewCommand::ClearSelection);
    h.state_mut().selected_step = None;
    for i in [0usize, 7, 199, 398] {
        let mut cam = h.state().editor.camera_for_step(i);
        cam.cx += 120.0;
        h.state_mut().editor.view.camera = cam;
        h.run_steps(1);
        assert_eq!(h.state().current_step(), Some(i));
    }
}

fn click_at(h: &mut Harness<'static, ZebraApp>, p: Pos2, modifiers: Modifiers) {
    // Held keys are read from the input state, not from the click.
    h.input_mut().events.push(Event::ModifiersChanged(modifiers));
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers });
    h.run_steps(1);
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.run_steps(1);
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

fn right_click_at(h: &mut Harness<'static, ZebraApp>, p: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn key(h: &mut Harness<'static, ZebraApp>, k: Key) {
    key_mods(h, Modifiers::NONE, k);
}

fn key_mods(h: &mut Harness<'static, ZebraApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run_steps(2);
}

/// Where a world point is on screen now.
fn screen(h: &Harness<'static, ZebraApp>, x: f64, y: f64) -> Pos2 {
    let a = h.state();
    a.canvas.w2s(&a.editor, zebra_core::geom::Vec2::new(x, y))
}

/// A point on the canvas with nothing under it: below the frames, which
/// sit in a row across the middle of the view.
fn empty_canvas(h: &Harness<'static, ZebraApp>, dx: f32) -> Pos2 {
    let r = h.state().canvas.rect;
    Pos2::new(r.min.x + 60.0 + dx, r.max.y - 160.0)
}

/// Three frames, the first used by steps 1 and 3: [A, B, A]. The whole
/// deck in view, nothing selected.
fn frame_used_twice(h: &mut Harness<'static, ZebraApp>) -> (String, String) {
    let a = frame(h.state_mut(), 0.0, true);
    let b = frame(h.state_mut(), 1000.0, true);
    h.state_mut().editor.apply(Command::AddStep { target: StepTarget::Element { id: a.clone().into() }, index: None, builds: vec![], notes: String::new() }, H).unwrap();
    settle(h);
    (a, b)
}

/// Nothing selected, the whole deck in view, no flight under way.
fn settle(h: &mut Harness<'static, ZebraApp>) {
    h.state_mut().view_cmd(ViewCommand::ClearSelection);
    h.state_mut().view_cmd(ViewCommand::FitAll { animate: false });
    h.run_steps(3);
}

fn row(h: &Harness<'static, ZebraApp>, i: usize) -> Pos2 {
    h.state().step_rows[i].center()
}

fn target(h: &Harness<'static, ZebraApp>, i: usize) -> StepTarget {
    h.state().editor.doc.path[i].target.clone()
}

/// Spec 08 A: clicking a step gives the path list the keys; Up/Down move
/// through the steps (the camera following); clicking the canvas gives the
/// keys back, and the arrows pan again.
#[test]
fn keys_follow_the_pane_clicked_last() {
    let mut h = harness();
    long_path(&mut h, 4);
    settle(&mut h);
    {
        let r = row(&h, 1);
        click_at(&mut h, r, Modifiers::NONE)
    };
    assert!(h.state().path_focus && zebra_app::sidebar::list_focused(h.state()));
    assert_eq!(h.state().selected_step, Some(1));
    key(&mut h, Key::ArrowDown);
    assert_eq!(h.state().selected_step, Some(2), "Down moved to the next step");
    let to = h.state().editor.camera_for_step(2);
    let goal = h.state().editor.view.target_camera();
    assert!((goal.cx - to.cx).abs() < 1e-6, "the canvas flies there");
    key(&mut h, Key::ArrowUp);
    key(&mut h, Key::ArrowUp);
    assert_eq!(h.state().selected_step, Some(0));
    key(&mut h, Key::ArrowUp);
    assert_eq!(h.state().selected_step, Some(0), "stays on the first");
    key(&mut h, Key::End);
    assert_eq!(h.state().selected_step, Some(3));
    key(&mut h, Key::Home);
    assert_eq!(h.state().selected_step, Some(0));
    // Return starts nothing.
    key(&mut h, Key::Enter);
    assert_eq!(h.state().editor.view.mode, zebra_core::Mode::Edit);
    // A click on empty canvas: the keys go back to the canvas.
    h.run_steps(60);
    settle(&mut h);
    let empty = empty_canvas(&h, 0.0);
    click_at(&mut h, empty, Modifiers::NONE);
    assert!(!h.state().path_focus);
    let cam = h.state().editor.view.target_camera();
    key(&mut h, Key::ArrowDown);
    assert_eq!(h.state().selected_step, Some(0), "Down no longer moves the step");
    assert_ne!(h.state().editor.view.target_camera().cy, cam.cy, "it pans the canvas");
    // Escape hands the keys back too.
    {
        let r = row(&h, 2);
        click_at(&mut h, r, Modifiers::NONE)
    };
    assert!(h.state().path_focus);
    key(&mut h, Key::Escape);
    assert!(!h.state().path_focus);
    assert_eq!(h.state().editor.doc.path.len(), 4);
}

/// Spec 08 A: a frame used by two steps lights only the step picked, and
/// Delete takes that step (never the frame) out, as one undo step.
#[test]
fn delete_removes_the_picked_step_of_a_frame_used_twice() {
    let mut h = harness();
    let (a, b) = frame_used_twice(&mut h);
    {
        let r = row(&h, 2);
        click_at(&mut h, r, Modifiers::NONE)
    };
    assert_eq!(zebra_app::sidebar::selected_steps(h.state()), vec![2], "only the row clicked, not every step of its frame");
    key(&mut h, Key::ArrowUp);
    assert_eq!(zebra_app::sidebar::selected_steps(h.state()), vec![1]);
    key(&mut h, Key::ArrowDown);
    let undo = h.state().editor.history().len();
    key(&mut h, Key::Backspace);
    let path: Vec<StepTarget> = h.state().editor.doc.path.iter().map(|s| s.target.clone()).collect();
    assert_eq!(path, vec![StepTarget::Element { id: a.clone().into() }, StepTarget::Element { id: b.clone().into() }], "the third step went");
    assert!(h.state().editor.doc.get(&a).is_some(), "the frame stays");
    assert_eq!(h.state().editor.history().len(), undo + 1, "one undo step");
    assert_eq!(h.state().selected_step, Some(1), "the step before it is selected");
    assert!(h.state().path_focus, "the list keeps the keys");
    // Forward delete does the same, through the same action.
    key(&mut h, Key::Delete);
    assert_eq!(h.state().editor.doc.path.len(), 1);
    assert!(h.state().editor.doc.get(&b).is_some());
    key_mods(&mut h, Modifiers::COMMAND, Key::Z);
    key_mods(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(h.state().editor.doc.path.len(), 3, "each removal undoes in one step");
}

/// Spec 08 A: Shift+Up/Down grow a selection of several steps only when
/// there is one already; Edit › Delete (the menu) agrees with the keys.
#[test]
fn shift_arrows_grow_a_selection_and_the_menu_agrees() {
    let mut h = harness();
    long_path(&mut h, 6);
    settle(&mut h);
    {
        let r = row(&h, 1);
        click_at(&mut h, r, Modifiers::NONE)
    };
    key_mods(&mut h, Modifiers::SHIFT, Key::ArrowDown);
    assert_eq!(zebra_app::sidebar::step_selection(h.state()), vec![1], "one step: Shift does nothing");
    {
        let r = row(&h, 2);
        click_at(&mut h, r, Modifiers::SHIFT)
    };
    assert_eq!(zebra_app::sidebar::step_selection(h.state()), vec![1, 2]);
    key_mods(&mut h, Modifiers::SHIFT, Key::ArrowDown);
    assert_eq!(zebra_app::sidebar::step_selection(h.state()), vec![1, 2, 3]);
    key_mods(&mut h, Modifiers::SHIFT, Key::ArrowUp);
    assert_eq!(zebra_app::sidebar::step_selection(h.state()), vec![1, 2]);
    // ⌘-click adds one more, apart from the range.
    {
        let r = row(&h, 4);
        click_at(&mut h, r, Modifiers::COMMAND)
    };
    assert_eq!(zebra_app::sidebar::step_selection(h.state()), vec![1, 2, 4]);
    let st = zebra_app::menus::state(h.state(), &Action::Delete);
    assert!(st.enabled, "Edit › Delete is on for the steps");
    let undo = h.state().editor.history().len();
    let ctx = h.ctx.clone();
    h.state_mut().perform(&ctx, Action::Delete);
    h.run_steps(2);
    assert_eq!(h.state().editor.doc.path.len(), 3, "the three steps went, no frame");
    assert_eq!(h.state().editor.doc.elements.len(), 6);
    assert_eq!(h.state().editor.history().len(), undo + 1);
    assert_eq!(h.state().editor.history().last().unwrap().label, zebra_app::t!("sidebar-undo-remove-step", count = 3));
    // Plain Up/Down drop back to one step.
    key(&mut h, Key::ArrowDown);
    assert_eq!(zebra_app::sidebar::step_selection(h.state()).len(), 1);
}

/// Spec 08 B1: right-click › Add Step After This One adds the frame
/// selected on the canvas right after the step.
#[test]
fn add_step_after_this_one_uses_the_selected_frame() {
    let mut h = harness();
    long_path(&mut h, 3);
    let extra = frame(h.state_mut(), 0.0, false);
    settle(&mut h);
    h.state_mut().select(vec![extra.clone().into()]);
    h.run_steps(2);
    {
        let r = row(&h, 0);
        right_click_at(&mut h, r)
    };
    h.get_by_label(&zebra_app::i18n::tr("sidebar-add-step-after")).click();
    h.run_steps(3);
    assert_eq!(h.state().editor.doc.path.len(), 4);
    assert_eq!(target(&h, 1), StepTarget::Element { id: extra.into() });
    assert!(h.state().step_pick.is_none());
}

/// Spec 08 B2: the gap between two rows offers a +; it inserts an empty
/// row that the next frame clicked on the canvas fills, as one undo step.
#[test]
fn the_gap_inserts_a_step_filled_by_a_clicked_frame() {
    let mut h = harness();
    long_path(&mut h, 3);
    let extra = frame(h.state_mut(), 1000.0, false);
    // Far from the others, so a click finds just it.
    h.state_mut().editor.apply(Command::Move { ids: vec![extra.clone().into()], dx: 0.0, dy: 2000.0, with_contents: false }, H).unwrap();
    settle(&mut h);
    let gap = Pos2::new(row(&h, 0).x, (h.state().step_rows[0].max.y + h.state().step_rows[1].min.y) / 2.0);
    let plus = Pos2::new(h.state().step_rows[0].min.x + h.state().step_number_w / 2.0 + 1.0, gap.y);
    h.input_mut().events.push(Event::PointerMoved(plus));
    h.run_steps(2);
    let tip = zebra_app::i18n::tr("sidebar-insert-step-tip");
    h.get_by_label(&tip).click();
    h.run_steps(3);
    assert_eq!(h.state().step_pick.as_ref().map(|p| p.index), Some(1), "an empty step waits before step 2");
    assert_eq!(h.state().editor.doc.path.len(), 3, "nothing in the document yet");
    let undo = h.state().editor.history().len();
    let at = screen(&h, 1000.0, 2000.0);
    click_at(&mut h, at, Modifiers::NONE);
    assert!(h.state().step_pick.is_none());
    assert_eq!(h.state().editor.doc.path.len(), 4);
    assert_eq!(target(&h, 1), StepTarget::Element { id: extra.clone().into() });
    assert_eq!(h.state().editor.history().len(), undo + 1, "one undo step");
    assert_eq!(h.state().editor.history().last().unwrap().label, zebra_app::i18n::tr("sidebar-undo-insert-step"));
    key_mods(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(h.state().editor.doc.path.len(), 3, "one undo takes the insert back");
}

/// Spec 08 B2: an insert cancelled (Escape, empty canvas, elsewhere in the
/// sidebar) leaves the document and its undo history as they were.
#[test]
fn a_cancelled_insert_leaves_no_undo_step() {
    let mut h = harness();
    long_path(&mut h, 3);
    settle(&mut h);
    let before = h.state().editor.doc.clone();
    let undo = h.state().editor.history().len();
    for how in 0..3 {
        zebra_app::sidebar::start_pick(h.state_mut(), 2);
        h.run_steps(2);
        assert!(h.state().step_pick.is_some());
        match how {
            0 => key(&mut h, Key::Escape),
            1 => {
                let empty = empty_canvas(&h, 0.0);
                click_at(&mut h, empty, Modifiers::NONE)
            }
            _ => {
        let r = row(&h, 0);
        click_at(&mut h, r, Modifiers::NONE)
    },
        }
        assert!(h.state().step_pick.is_none(), "cancelled ({how})");
        assert_eq!(h.state().editor.doc, before);
        assert_eq!(h.state().editor.history().len(), undo, "no undo step ({how})");
    }
}

/// Spec 08 B2: a frame drawn while a step waits fills it; the frame and the
/// step are one undo step.
#[test]
fn a_drawn_frame_fills_the_waiting_step_in_one_undo() {
    let mut h = harness();
    long_path(&mut h, 2);
    settle(&mut h);
    let undo = h.state().editor.history().len();
    let elements = h.state().editor.doc.elements.len();
    // Right-click › Add Step After This One with nothing selected: the
    // empty row waits after step 1.
    {
        let r = row(&h, 0);
        right_click_at(&mut h, r)
    };
    h.get_by_label(&zebra_app::i18n::tr("sidebar-add-step-after")).click();
    h.run_steps(3);
    assert_eq!(h.state().step_pick.as_ref().map(|p| p.index), Some(1));
    h.state_mut().view_cmd(ViewCommand::SetTool { tool: zebra_core::Tool::Frame });
    let (a, b) = (empty_canvas(&h, 0.0), empty_canvas(&h, 200.0) + egui::vec2(0.0, 100.0));
    h.input_mut().events.push(Event::PointerMoved(a));
    h.input_mut().events.push(Event::PointerButton { pos: a, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for k in 1..=4 {
        h.input_mut().events.push(Event::PointerMoved(a + (b - a) * (k as f32 / 4.0)));
        h.run_steps(1);
    }
    h.input_mut().events.push(Event::PointerButton { pos: b, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().editor.doc.elements.len(), elements + 1, "a frame was drawn");
    assert_eq!(h.state().editor.doc.path.len(), 3);
    assert!(h.state().step_pick.is_none());
    assert_eq!(h.state().editor.history().len(), undo + 1, "frame and step: one undo step");
    key_mods(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(h.state().editor.doc.elements.len(), elements);
    assert_eq!(h.state().editor.doc.path.len(), 2);
}

/// Spec 08 B3: rows show an open hand (they drag), a closed one while
/// dragging.
#[test]
fn rows_show_a_grab_hand() {
    let mut h = harness();
    long_path(&mut h, 3);
    settle(&mut h);
    let p = row(&h, 1);
    h.input_mut().events.push(Event::PointerMoved(p));
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Grab);
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.input_mut().events.push(Event::PointerMoved(p + egui::vec2(0.0, 40.0)));
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Grabbing);
    h.input_mut().events.push(Event::PointerButton { pos: p + egui::vec2(0.0, 40.0), button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// Spec 08 A: something selected on the canvas another way (a toolbar
/// insert) takes the keys from the list, so Escape doesn't give them back.
#[test]
fn a_canvas_selection_takes_the_keys_from_the_list() {
    let mut h = harness();
    long_path(&mut h, 3);
    settle(&mut h);
    let r = row(&h, 1);
    click_at(&mut h, r, Modifiers::NONE);
    assert!(zebra_app::sidebar::list_focused(h.state()));
    let ctx = h.ctx.clone();
    h.state_mut().perform(&ctx, Action::InsertFrame);
    h.run_steps(2);
    assert!(!h.state().path_focus);
    key(&mut h, Key::Escape);
    assert!(h.state().editor.view.selection.is_empty());
    assert!(!zebra_app::sidebar::list_focused(h.state()), "the canvas keeps the keys");
    let n = h.state().editor.doc.elements.len();
    key(&mut h, Key::ArrowDown);
    assert_eq!(h.state().selected_step, Some(1), "Down pans; the step stays");
    assert_eq!(h.state().editor.doc.elements.len(), n);
}
