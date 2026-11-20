//! Spec 18d: a frame holds what lies inside the outline it draws.
//! Option-click (or Option-drag) a frame takes it on its own, to move
//! without what it holds; so does the right-click menu's Select Frame
//! Only. Making a rectangle frame a circle says how many objects it lets
//! go, in a note under the toolbar.
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use zebra_app::actions::Action;
use zebra_app::i18n::tr;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, Origin};

const H: Origin = Origin::Human;

fn harness() -> Harness<'static, ZebraApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h
}

fn shot(h: &mut Harness<'static, ZebraApp>, name: &str) {
    if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
        h.render().unwrap().save(std::path::Path::new(&dir).join(name)).unwrap();
    }
}

/// The deck: rectangle frame "f" 500 by 340 points on screen in the middle
/// of the canvas, a box "mid" in its middle and two near its corners
/// ("nw", "se"): inside the rectangle, outside the circle it would draw.
/// The screen point of the middle of the frame's left edge.
fn deck(h: &mut Harness<'static, ZebraApp>) -> Pos2 {
    let (c, z) = {
        let a = h.state();
        (a.canvas.s2w(&a.editor, a.canvas.rect.center()), a.editor.view.camera.zoom)
    };
    let app = h.state_mut();
    let k = 1.0 / z;
    let mut add = |j: serde_json::Value| {
        let cmd: Command = serde_json::from_value(j).unwrap();
        app.editor.apply(cmd, H).unwrap();
    };
    add(serde_json::json!({"op":"add_frame","id":"f","x":c.x,"y":c.y,"w":500.0 * k,"h":340.0 * k,"style":"rectangle"}));
    add(serde_json::json!({"op":"add_shape","id":"mid","shape":"rectangle","x":c.x,"y":c.y,"w":80.0 * k,"h":60.0 * k}));
    add(serde_json::json!({"op":"add_shape","id":"nw","shape":"ellipse","x":c.x - 210.0 * k,"y":c.y - 130.0 * k,"w":40.0 * k,"h":40.0 * k}));
    add(serde_json::json!({"op":"add_shape","id":"se","shape":"ellipse","x":c.x + 210.0 * k,"y":c.y + 130.0 * k,"w":40.0 * k,"h":40.0 * k}));
    let _ = app.editor.view(zebra_core::ViewCommand::ClearSelection, H);
    let left = app.canvas.rect.center() - egui::vec2(250.0, 0.0);
    h.run_steps(3);
    left
}

fn x_of(h: &Harness<'static, ZebraApp>, id: &str) -> f64 {
    h.state().editor.doc.get(id).unwrap().transform.x
}

fn frame_only(h: &Harness<'static, ZebraApp>) -> Option<String> {
    h.state().editor.frame_only().cloned()
}

fn modifiers(h: &mut Harness<'static, ZebraApp>, m: Modifiers) {
    h.event(Event::ModifiersChanged(m));
    h.step();
}

fn press(h: &mut Harness<'static, ZebraApp>, p: Pos2, m: Modifiers) {
    h.event(Event::PointerMoved(p));
    h.step();
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: m });
    h.step();
}

fn release(h: &mut Harness<'static, ZebraApp>, p: Pos2, m: Modifiers) {
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: m });
    h.step();
    h.step();
}

/// Drag from `from` by `by` in ten moves; `mid` runs halfway through.
fn drag(h: &mut Harness<'static, ZebraApp>, from: Pos2, by: egui::Vec2, m: Modifiers, mut mid: impl FnMut(&mut Harness<'static, ZebraApp>)) {
    press(h, from, m);
    for i in 1..=10 {
        if i == 5 {
            mid(h);
        }
        h.event(Event::PointerMoved(from + by * (i as f32 / 10.0)));
        h.step();
    }
    release(h, from + by, Modifiers::NONE);
}

/// An Option-click on a frame selects it on its own; a drag then moves it
/// and leaves what it holds. A plain click selects it with its contents
/// again.
#[test]
fn option_click_takes_the_frame_alone() {
    let mut h = harness();
    let edge = deck(&mut h);
    modifiers(&mut h, Modifiers::ALT);
    press(&mut h, edge, Modifiers::ALT);
    release(&mut h, edge, Modifiers::ALT);
    modifiers(&mut h, Modifiers::NONE);
    assert_eq!(h.state().editor.view.selection, vec!["f".to_string()]);
    assert_eq!(frame_only(&h).as_deref(), Some("f"), "the frame on its own");
    // The inspector says so.
    h.get_by_label("Selected without its 3 objects.");
    shot(&mut h, "frame-selected-alone.png");

    let (f0, mid0, nw0) = (x_of(&h, "f"), x_of(&h, "mid"), x_of(&h, "nw"));
    let z = h.state().editor.view.camera.zoom;
    drag(&mut h, edge, egui::vec2(120.0, 0.0), Modifiers::NONE, |_| {});
    assert!((x_of(&h, "f") - (f0 + 120.0 / z)).abs() < 1.0 / z, "the frame moved");
    assert_eq!((x_of(&h, "mid"), x_of(&h, "nw")), (mid0, nw0), "what it holds stayed");
    assert_eq!(h.state().editor.history().last().unwrap().label, tr("canvas-undo-move"), "undo names it Move");
    assert_eq!(frame_only(&h).as_deref(), Some("f"), "still on its own after the drag");

    // A plain click: the frame with what it holds.
    let edge = edge + egui::vec2(120.0, 0.0);
    press(&mut h, edge, Modifiers::NONE);
    release(&mut h, edge, Modifiers::NONE);
    assert_eq!(h.state().editor.view.selection, vec!["f".to_string()]);
    assert_eq!(frame_only(&h), None);
}

/// Option held at the press decides: let go of it mid-drag and the frame
/// still moves alone. Without Option the frame carries what it holds.
#[test]
fn option_drag_moves_the_frame_alone_whatever_happens_to_option() {
    let mut h = harness();
    let edge = deck(&mut h);
    let (f0, mid0) = (x_of(&h, "f"), x_of(&h, "mid"));
    modifiers(&mut h, Modifiers::ALT);
    drag(&mut h, edge, egui::vec2(-150.0, 0.0), Modifiers::ALT, |h| modifiers(h, Modifiers::NONE));
    assert!(x_of(&h, "f") < f0, "the frame moved");
    assert_eq!(x_of(&h, "mid"), mid0, "what it holds stayed");
    assert_eq!(frame_only(&h).as_deref(), Some("f"));

    // Undo, then drag without Option: everything goes.
    h.state_mut().editor.undo(H).unwrap();
    let _ = h.state_mut().editor.view(zebra_core::ViewCommand::ClearSelection, H);
    h.run_steps(2);
    drag(&mut h, edge, egui::vec2(-150.0, 0.0), Modifiers::NONE, |_| {});
    assert!(x_of(&h, "f") < f0);
    assert_eq!(x_of(&h, "mid") - mid0, x_of(&h, "f") - f0, "what it holds went with it");
    assert_eq!(frame_only(&h), None);
}

/// Right-click a frame › Select Frame Only. Also in the Edit menu.
#[test]
fn select_frame_only_from_the_right_click_menu() {
    let mut h = harness();
    let edge = deck(&mut h);
    assert!(!zebra_app::menus::state(h.state(), &Action::SelectFrameOnly).enabled, "nothing selected");
    h.event(Event::PointerMoved(edge));
    h.step();
    h.event(Event::PointerButton { pos: edge, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: edge, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().editor.view.selection, vec!["f".to_string()]);
    assert_eq!(frame_only(&h), None, "selected with what it holds");
    assert!(zebra_app::menus::state(h.state(), &Action::SelectFrameOnly).enabled);
    shot(&mut h, "frame-context-menu.png");
    h.get_by_label(&tr("action-select-frame-only")).click();
    h.run_steps(3);
    assert_eq!(frame_only(&h).as_deref(), Some("f"));

    // Nudged with an arrow key: the frame alone.
    let (f0, mid0) = (x_of(&h, "f"), x_of(&h, "mid"));
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    assert!(x_of(&h, "f") > f0);
    assert_eq!(x_of(&h, "mid"), mid0);

    // Not for a shape.
    h.state_mut().select(vec!["mid".into()]);
    assert!(!zebra_app::menus::state(h.state(), &Action::SelectFrameOnly).enabled);
}

/// A rectangle frame made a circle in the inspector lets go of what lies
/// outside the circle, and says so under the toolbar.
#[test]
fn rectangle_to_circle_says_what_it_lets_go() {
    let mut h = harness();
    deck(&mut h);
    assert_eq!(h.state().editor.frame_contents("f").len(), 3);
    h.state_mut().select(vec!["f".into()]);
    h.run_steps(4);
    // The inspector's Circle (the toolbar has a Circle frame tool too).
    let inspector_x = h.state().canvas.rect.right();
    let label = tr("frame-style-circle");
    h.get_all_by_label(&label).find(|n| n.rect().left() > inspector_x).expect("the inspector's Circle").click();
    h.run_steps(3);
    let app = h.state();
    assert_eq!(app.editor.frame_contents("f"), vec!["mid".to_string()]);
    let note = app.note.as_ref().expect("a note");
    assert_eq!(note.text, "2 objects no longer move with this frame.");
    shot(&mut h, "frame-circle-note.png");

    // Back to a rectangle: it holds them again, with nothing to say.
    h.state_mut().note = None;
    let label = tr("frame-style-rectangle");
    h.get_all_by_label(&label).find(|n| n.rect().left() > inspector_x).expect("the inspector's Rectangle").click();
    h.run_steps(3);
    assert_eq!(h.state().editor.frame_contents("f").len(), 3);
    assert!(h.state().note.is_none());
}
