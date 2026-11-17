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

/// Every kind of edit, then undo one step at a time: each undo must land
/// exactly on the document as it was before that step; redo must replay it.
#[test]
fn delta_undo_redo_matches_snapshots_step_by_step() {
    let mut ed = sample();
    let h = Origin::Human;
    let agent = Origin::agent("bot");
    let mut snaps = vec![ed.doc.clone()];
    let steps: Vec<(Origin, serde_json::Value)> = vec![
        (h.clone(), serde_json::json!({"op":"move","ids":["f1"],"dx":120,"dy":-40})),
        (h.clone(), serde_json::json!({"op":"update","id":"t1","patch":{"text":"Hello again","bold":true}})),
        (h.clone(), serde_json::json!({"op":"z_order","ids":["t1"],"order":"back"})),
        (h.clone(), serde_json::json!({"op":"add_ink","points":[[0,0],[10,5],[20,0]]})),
        (h.clone(), serde_json::json!({"op":"duplicate","ids":["f2"]})),
        (h.clone(), serde_json::json!({"op":"move_step","from":0,"to":1})),
        (h.clone(), serde_json::json!({"op":"set_theme","theme":"night"})),
        (h.clone(), serde_json::json!({"op":"rotate","ids":["f2"],"degrees":15})),
        (h.clone(), serde_json::json!({"op":"delete","ids":["f1"]})),
        (h.clone(), serde_json::json!({"op":"auto_path"})),
    ];
    for (o, j) in steps {
        ed.apply(cmd(j), o).unwrap();
        ed.view.now += 5.0;
        snaps.push(ed.doc.clone());
    }
    // Two quick agent edits group into one undo step.
    ed.apply(cmd(serde_json::json!({"op":"set_title","title":"Agent 1"})), agent.clone()).unwrap();
    ed.view.now += 0.5;
    ed.apply(cmd(serde_json::json!({"op":"add_text","text":"from agent","x":5,"y":5})), agent.clone()).unwrap();
    snaps.push(ed.doc.clone());
    // A human drag as a transaction is one step.
    ed.view.now += 5.0;
    ed.begin("Drag", h.clone()).unwrap();
    for _ in 0..10 {
        ed.apply(cmd(serde_json::json!({"op":"move","ids":["f2"],"dx":3,"dy":1})), h.clone()).unwrap();
    }
    ed.commit();
    snaps.push(ed.doc.clone());

    let fin = ed.doc.clone();
    for want in snaps.iter().rev().skip(1) {
        ed.undo(h.clone()).unwrap().expect("an undo step");
        assert!(ed.doc == *want, "undo did not restore the earlier document");
    }
    assert!(ed.undo(h.clone()).unwrap().is_some(), "the sample build itself is undoable");
    assert!(ed.doc.elements.is_empty());
    while ed.redo(h.clone()).unwrap().is_some() {}
    assert!(ed.doc == fin, "redo all must replay to the final document");
    assert!(ed.history_bytes() < 64 * 1024, "deltas, not snapshots: {} bytes", ed.history_bytes());
}

#[test]
fn revert_agent_and_history_cap_still_work_with_deltas() {
    let mut ed = sample();
    let before = ed.doc.clone();
    let bot = Origin::agent("bot");
    for i in 0..3 {
        ed.apply(cmd(serde_json::json!({"op":"move","ids":["f2"],"dx":10,"dy":0})), bot.clone()).unwrap();
        ed.view.now += 5.0 + i as f64; // separate steps
    }
    assert_eq!(ed.revert_agent_session("bot"), 3);
    assert!(ed.doc == before, "reverting the agent restores the document exactly");
    for _ in 0..520 {
        ed.apply(cmd(serde_json::json!({"op":"move","ids":["f1"],"dx":1,"dy":0})), Origin::Human).unwrap();
    }
    assert_eq!(ed.history().len(), 500);
    let mut n = 0;
    while ed.undo(Origin::Human).unwrap().is_some() {
        n += 1;
    }
    assert_eq!(n, 500);
    // The oldest 20 moves fell off the cap; the rest unwound exactly.
    assert_eq!(ed.doc.elements["f1"].transform.x, before.elements["f1"].transform.x + 20.0);
}

fn tiny_png(seed: u8) -> Vec<u8> {
    let img = image::RgbImage::from_fn(8, 8, |x, y| image::Rgb([x as u8 * 30, y as u8 * 30, seed]));
    let mut out = std::io::Cursor::new(vec![]);
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn saved_media_is_memory_mapped_and_survives_resave() {
    use base64::Engine;
    let dir = std::env::temp_dir().join(format!("zebra-assets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("deck.zebra");
    let mut ed = Editor::default();
    let b64 = |b: Vec<u8>| base64::engine::general_purpose::STANDARD.encode(b);
    ed.apply(cmd(serde_json::json!({"op":"add_image","id":"a","data_base64":b64(tiny_png(1))})), Origin::Human).unwrap();
    ed.apply(cmd(serde_json::json!({"op":"add_image","id":"b","data_base64":b64(tiny_png(2)),"x":100})), Origin::Human).unwrap();
    assert!(ed.doc.assets.values().all(|a| !a.bytes.is_mapped()), "new media starts in memory");
    let original: Vec<Vec<u8>> = ed.doc.assets.values().map(|a| a.bytes.to_vec()).collect();

    io::save(&mut ed.doc, &path).unwrap();
    assert!(ed.doc.assets.values().all(|a| a.bytes.is_mapped()), "after saving, media is read from the file");

    let mut back = io::load(&path).unwrap();
    assert!(back.assets.values().all(|a| a.bytes.is_mapped()));
    assert_eq!(back.assets.values().map(|a| a.bytes.to_vec()).collect::<Vec<_>>(), original);

    // Delete one image and save over the file we're mapped from: the
    // mapping stays valid, and the unused asset isn't written.
    back.elements.remove("b");
    back.z_order.retain(|z| z != "b");
    io::save(&mut back, &path).unwrap();
    assert_eq!(back.assets.values().map(|a| a.bytes.to_vec()).collect::<Vec<_>>(), original, "old mappings still readable");
    let again = io::load(&path).unwrap();
    assert_eq!(again.assets.len(), 1, "only used media is saved");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn icons_and_library_shapes_via_commands() {
    let mut ed = Editor::default();
    let h = Origin::Human;
    ed.apply(cmd(serde_json::json!({"op":"add_icon","id":"n1","name":"lightbulb","x":10,"y":20,"size":120})), h.clone()).unwrap();
    let e = ed.doc.get("n1").unwrap();
    assert_eq!((e.size.w, e.size.h), (120.0, 120.0));
    // Unknown names fail with suggestions.
    let err = ed.apply(cmd(serde_json::json!({"op":"add_icon","name":"light-bulb"})), h.clone()).unwrap_err();
    assert!(serde_json::to_string(&err).unwrap().contains("lightbulb"), "{err:?}");
    // Recolour, reweight and swap.
    ed.apply(cmd(serde_json::json!({"op":"update","id":"n1","patch":{"color":"theme:accent","stroke_width":1.5,"icon":"rocket"}})), h.clone()).unwrap();
    match &ed.doc.get("n1").unwrap().kind {
        ElementKind::Icon(i) => assert_eq!((i.name.as_str(), i.color.as_str(), i.stroke_width), ("rocket", "theme:accent", 1.5)),
        k => panic!("{k:?}"),
    }
    assert!(ed.apply(cmd(serde_json::json!({"op":"update","id":"n1","patch":{"fill":"#ff0000"}})), h.clone()).is_err());
    // Every library shape can be added; sticky notes default to yellow.
    for k in ShapeKind::ALL {
        let o = ed.apply(Command::AddShape { shape: *k, x: 0.0, y: 0.0, w: 100.0, h: 80.0, rotation: 0.0, fill: None, stroke: None, id: None }, h.clone()).unwrap();
        if *k == ShapeKind::StickyNote {
            match &ed.doc.get(&o.created[0]).unwrap().kind {
                ElementKind::Shape(s) => assert_eq!(s.fill.as_deref(), Some("#ffe98a")),
                k => panic!("{k:?}"),
            }
        }
    }
    ed.apply(cmd(serde_json::json!({"op":"add_shape","id":"rr","shape":"rounded_rect"})), h.clone()).unwrap();
    ed.apply(cmd(serde_json::json!({"op":"update","id":"rr","patch":{"corner_radius":30,"dash":"dotted"}})), h.clone()).unwrap();
    let saved = serde_json::to_value(ed.doc.get("rr").unwrap()).unwrap();
    assert_eq!(saved["kind"]["corner_radius"], 30.0);
    assert_eq!(saved["kind"]["dash"], "dotted");
    // Queries.
    let q = ed.query(Query::Icons { query: "idea".into(), limit: 5 }).unwrap();
    assert!(q["icons"].as_array().unwrap().iter().any(|i| i["name"] == "lightbulb"), "{q}");
    let q = ed.query(Query::Shapes).unwrap();
    assert_eq!(q.as_array().unwrap().len(), ShapeKind::ALL.len());
    assert!(describe::describe(&ed).contains("icon rocket"));
}

#[test]
fn connectors_stick_to_elements_and_survive_deletion() {
    let mut ed = Editor::default();
    let h = Origin::Human;
    ed.apply(cmd(serde_json::json!({"op":"add_shape","id":"a","shape":"rectangle","x":0,"y":0,"w":100,"h":100})), h.clone()).unwrap();
    ed.apply(cmd(serde_json::json!({"op":"add_shape","id":"b","shape":"ellipse","x":400,"y":0,"w":100,"h":100})), h.clone()).unwrap();
    ed.apply(cmd(serde_json::json!({"op":"add_connector","id":"c","from":{"element":"a"},"to":{"element":"b"},"label":"leads to"})), h.clone()).unwrap();
    let ends = |ed: &Editor| {
        let e = ed.doc.get("c").unwrap();
        let ElementKind::Connector(c) = &e.kind else { panic!() };
        (e.transform.apply(Vec2::new(c.from.x, c.from.y)), e.transform.apply(Vec2::new(c.to.x, c.to.y)))
    };
    let (p, q) = ends(&ed);
    assert!((p.x - 50.0).abs() < 1e-9 && p.y.abs() < 1e-9, "starts on a's right edge: {p:?}");
    assert!((q.x - 350.0).abs() < 1e-9 && q.y.abs() < 1e-9, "ends on b's left edge: {q:?}");
    // Moving b drags the end along, in the same undo step.
    ed.apply(cmd(serde_json::json!({"op":"move","ids":["b"],"dx":0,"dy":400})), h.clone()).unwrap();
    let (_, q) = ends(&ed);
    let d = (q - Vec2::new(400.0, 400.0)).len();
    assert!((d - 50.0).abs() < 1e-6, "on the circle's edge: {q:?}");
    assert!(ed.doc.get("c").unwrap().bounds().contains(q));
    ed.undo(h.clone()).unwrap();
    assert!((ends(&ed).1.x - 350.0).abs() < 1e-9);
    // Elbow routes leave from side midpoints.
    ed.apply(cmd(serde_json::json!({"op":"update","id":"c","patch":{"route":"elbow","end_head":"dot"}})), h.clone()).unwrap();
    // Hit test follows the line, not the box.
    let c = ed.doc.get("c").unwrap().clone();
    assert!(c.hit(Vec2::new(200.0, 0.0), 1.0, false));
    // Deleting b frees that end where it was.
    ed.apply(cmd(serde_json::json!({"op":"delete","ids":["b"]})), h.clone()).unwrap();
    let e = ed.doc.get("c").unwrap();
    let ElementKind::Connector(k) = &e.kind else { panic!() };
    assert_eq!(k.to.element, None);
    assert!((ends(&ed).1.x - 350.0).abs() < 1e-9);
    assert!(describe::describe(&ed).contains("connector a -> point"));
    // Free ends can be moved.
    ed.apply(cmd(serde_json::json!({"op":"update","id":"c","patch":{"to":{"x":-300,"y":-300}}})), h.clone()).unwrap();
    assert_eq!(ends(&ed).1, Vec2::new(-300.0, -300.0));
    assert!(ed.apply(cmd(serde_json::json!({"op":"add_connector","from":{"element":"c"},"to":{"x":0,"y":0}})), h).is_err());
}

#[test]
fn groups_select_move_duplicate_and_ungroup_together() {
    let mut ed = Editor::default();
    let h = Origin::Human;
    for (id, x) in [("a", 0.0), ("b", 300.0), ("c", 600.0)] {
        ed.apply(cmd(serde_json::json!({"op":"add_shape","id":id,"shape":"rectangle","x":x,"y":0,"w":100,"h":100})), h.clone()).unwrap();
    }
    assert!(ed.apply(cmd(serde_json::json!({"op":"group","ids":["a"]})), h.clone()).is_err(), "one element is not a group");
    let out = ed.apply(cmd(serde_json::json!({"op":"group","ids":["a","b"]})), h.clone()).unwrap();
    let g = out.group.clone().expect("group id returned");
    assert_eq!(ed.doc.get("a").unwrap().group.as_deref(), Some(g.as_str()));
    // Selecting one member selects the group.
    ed.view(zebra_core::ViewCommand::Select { ids: vec!["b".into()] }, h.clone()).unwrap();
    assert_eq!(ed.view.selection, vec!["b".to_string(), "a".to_string()]);
    assert!(zebra_core::describe::describe(&ed).contains(&format!("group={g}")));
    // Grouping a member with c folds the old group into the new one.
    let g2 = ed.apply(cmd(serde_json::json!({"op":"group","ids":["a","c"]})), h.clone()).unwrap().group.unwrap();
    assert!(["a", "b", "c"].iter().all(|i| ed.doc.get(i).unwrap().group.as_deref() == Some(g2.as_str())));
    ed.undo(h.clone()).unwrap();
    assert_eq!(ed.doc.get("c").unwrap().group, None, "undo restores the old grouping");
    assert_eq!(ed.doc.get("b").unwrap().group.as_deref(), Some(g.as_str()));
    // Duplicating the group makes a new group; copying one member alone doesn't.
    let copies = ed.apply(cmd(serde_json::json!({"op":"duplicate","ids":["a","b"]})), h.clone()).unwrap().created;
    let cg = ed.doc.get(&copies[0]).unwrap().group.clone().unwrap();
    assert_ne!(cg, g);
    assert_eq!(ed.doc.get(&copies[1]).unwrap().group.as_deref(), Some(cg.as_str()));
    let lone = ed.apply(cmd(serde_json::json!({"op":"duplicate","ids":["a"]})), h.clone()).unwrap().created;
    assert_eq!(ed.doc.get(&lone[0]).unwrap().group, None);
    // Ungroup by any member dissolves the whole group.
    ed.apply(cmd(serde_json::json!({"op":"ungroup","ids":["b"]})), h.clone()).unwrap();
    assert!(ed.doc.get("a").unwrap().group.is_none() && ed.doc.get("b").unwrap().group.is_none());
    // The group survives a save round trip.
    ed.apply(cmd(serde_json::json!({"op":"group","ids":["a","b"],"id":"team"})), h.clone()).unwrap();
    let json = serde_json::to_string(&ed.doc).unwrap();
    let doc: zebra_core::Document = serde_json::from_str(&json).unwrap();
    assert_eq!(doc.get("b").unwrap().group.as_deref(), Some("team"));
}
