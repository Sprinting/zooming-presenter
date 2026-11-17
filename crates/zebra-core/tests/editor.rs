use zebra_core::*;
use zebra_core::command::*;
use zebra_core::view::Mode;

fn cmd(j: serde_json::Value) -> Command {
    serde_json::from_value(j).unwrap()
}

fn sample() -> Editor {
    let mut ed = Editor::default();
    let agent = Origin::agent("test");
    ed.apply_batch(
        vec![
            cmd(serde_json::json!({"op":"add_frame","id":"f1","x":0,"y":0,"w":800,"h":600,"add_to_path":true})),
            cmd(serde_json::json!({"op":"add_text","id":"t1","text":"Hello","style":"title","place":{"relative_to":"f1","side":"inside"}})),
            cmd(serde_json::json!({"op":"add_frame","id":"f2","style":"circle","x":2000,"y":500,"w":400,"h":400,"rotation":30,"add_to_path":true})),
            cmd(serde_json::json!({"op":"add_shape","shape":"arrow","x":1000,"y":200})),
        ],
        Some("build".into()),
        agent,
    )
    .unwrap();
    ed
}

#[test]
fn batch_and_undo_restore_exactly() {
    let mut ed = sample();
    assert_eq!(ed.doc.elements.len(), 4);
    assert_eq!(ed.doc.path.len(), 2);
    let before = ed.doc.clone();
    ed.apply(cmd(serde_json::json!({"op":"move","ids":["f1"],"dx":100,"dy":50})), Origin::Human).unwrap();
    // frame carried its text
    assert_eq!(ed.doc.get("t1").unwrap().transform.x, 100.0);
    ed.undo(Origin::Human).unwrap();
    assert_eq!(ed.doc, before);
    ed.redo(Origin::Human).unwrap();
    assert_eq!(ed.doc.get("t1").unwrap().transform.x, 100.0);
}

#[test]
fn failing_batch_rolls_back() {
    let mut ed = sample();
    let before = ed.doc.clone();
    let r = ed.apply_batch(
        vec![
            cmd(serde_json::json!({"op":"move","ids":["f1"],"dx":1,"dy":1})),
            cmd(serde_json::json!({"op":"delete","ids":["nope"]})),
        ],
        None,
        Origin::agent("a"),
    );
    let e = r.unwrap_err();
    assert_eq!(e.code, "not_found");
    assert_eq!(ed.doc, before);
}

#[test]
fn present_steps_and_builds() {
    let mut ed = sample();
    ed.apply(cmd(serde_json::json!({"op":"update_step","index":0,"builds":["t1"]})), Origin::Human).unwrap();
    ed.view(ViewCommand::Present { step: None }, Origin::Human).unwrap();
    assert_eq!(ed.view.mode, Mode::Present);
    assert!(ed.hidden_builds().contains("t1"));
    ed.view(ViewCommand::Next, Origin::Human).unwrap();
    assert_eq!(ed.view.present.build, 1);
    assert!(ed.hidden_builds().is_empty());
    ed.view(ViewCommand::Next, Origin::Human).unwrap();
    assert_eq!(ed.view.present.step, Some(1));
    // camera flies; after the flight it is rotated like the frame
    ed.view.tick(100.0);
    assert!((ed.view.camera.rotation - 30.0).abs() < 1e-9);
    ed.view(ViewCommand::Prev, Origin::Human).unwrap();
    assert_eq!(ed.view.present.step, Some(0));
    assert_eq!(ed.view.present.build, 1);
}

#[test]
fn agents_cannot_touch_human_view_or_edit_while_paused() {
    let mut ed = sample();
    let cam = ed.view.camera;
    ed.view(ViewCommand::Zoom { factor: 2.0, anchor: None }, Origin::agent("a")).unwrap();
    assert_eq!(ed.view.camera, cam);
    ed.view(ViewCommand::Select { ids: vec!["f1".into()] }, Origin::agent("a")).unwrap();
    assert!(ed.view.selection.is_empty());
    assert_eq!(ed.view.agent_selection, vec!["f1".to_string()]);
    assert!(ed.view(ViewCommand::Present { step: None }, Origin::agent("a")).is_err());
    ed.view(ViewCommand::PauseAgents { paused: true }, Origin::Human).unwrap();
    let e = ed.apply(Command::SetTitle { title: "x".into() }, Origin::agent("a")).unwrap_err();
    assert_eq!(e.code, "paused");
}

#[test]
fn human_gesture_blocks_agents_and_is_one_undo_step() {
    let mut ed = sample();
    let n = ed.history().len();
    ed.begin("Drag", Origin::Human).unwrap();
    for _ in 0..10 {
        ed.apply(cmd(serde_json::json!({"op":"move","ids":["f2"],"dx":5,"dy":0})), Origin::Human).unwrap();
    }
    assert_eq!(ed.apply(Command::SetTitle { title: "x".into() }, Origin::agent("a")).unwrap_err().code, "busy");
    ed.commit();
    assert_eq!(ed.history().len(), n + 1);
}

#[test]
fn save_load_roundtrip() {
    let ed = sample();
    let bytes = io::save_bytes(&ed.doc).unwrap();
    let back = io::load_bytes(&bytes).unwrap();
    assert_eq!(back, ed.doc);
}

#[test]
fn describe_mentions_ids_and_path() {
    let ed = sample();
    let s = describe::describe(&ed);
    assert!(s.contains("f1 frame bracket"), "{s}");
    assert!(s.contains("t1 text title \"Hello\""), "{s}");
    assert!(s.contains("1. f1"), "{s}");
}

#[test]
fn schema_generates() {
    let s = schemars::schema_for!(Command);
    let j = serde_json::to_string(&s).unwrap();
    assert!(j.contains("add_text"));
}

#[test]
fn style_change_keeps_text_inside_its_frame() {
    let mut ed = Editor::default();
    ed.apply_batch(vec![
        cmd(serde_json::json!({"op":"add_frame","id":"f","x":0,"y":0,"w":480,"h":220})),
        cmd(serde_json::json!({"op":"add_text","id":"t","text":"Why cities grow","x":0,"y":0})),
    ], None, Origin::Human).unwrap();
    assert_eq!(ed.doc.parent_frame("t").as_deref(), Some("f"));
    ed.apply(cmd(serde_json::json!({"op":"update","id":"t","patch":{"style":"title"}})), Origin::Human).unwrap();
    let fb = ed.doc.get("f").unwrap().bounds();
    assert!(fb.contains_rect(&ed.doc.get("t").unwrap().bounds()), "title overflowed its frame");
}
