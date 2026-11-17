use zebra_core::*;
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

#[test]
fn frame_style_is_visual_only() {
    let mut ed = Editor::default();
    ed.apply(cmd(serde_json::json!({"op":"add_frame","id":"f","x":0,"y":0,"w":480,"h":220})), Origin::Human).unwrap();
    for style in ["circle", "bracket"] {
        ed.apply(cmd(serde_json::json!({"op":"update","id":"f","patch":{"frame_style":style}})), Origin::Human).unwrap();
        let f = ed.doc.get("f").unwrap();
        assert_eq!((f.size.w, f.size.h), (480.0, 220.0), "box kept after {style}");
    }
    // As a circle, it is drawn, hit and contains things inscribed in the box.
    ed.apply(cmd(serde_json::json!({"op":"update","id":"f","patch":{"frame_style":"circle"}})), Origin::Human).unwrap();
    let f = ed.doc.get("f").unwrap().clone();
    assert_eq!(f.visual_size(), (220.0, 220.0));
    assert!(f.hit(Vec2::new(110.0, 0.0), 1.0, false), "rim");
    assert!(!f.hit(Vec2::new(200.0, 0.0), 1.0, false), "outside the circle, inside the box");
    assert!(!f.hit(Vec2::new(0.0, 0.0), 1.0, false), "interior is not the rim");
    assert!(f.hit(Vec2::new(0.0, 0.0), 1.0, true));
}

#[test]
fn overview_shows_later_fade_ins_dimmed() {
    let mut ed = sample();
    ed.apply(cmd(serde_json::json!({"op":"add_step","target":{"kind":"overview"},"index":0})), Origin::Human).unwrap();
    ed.apply(cmd(serde_json::json!({"op":"update_step","index":1,"builds":["t1"]})), Origin::Human).unwrap();
    ed.view(cmd_view(serde_json::json!({"op":"present","step":0})), Origin::Human).unwrap();
    assert!(ed.hidden_builds().contains("t1"));
    assert!(ed.dimmed_builds().contains("t1"), "dimmed in the overview");
    ed.apply(cmd(serde_json::json!({"op":"update_step","index":0,"hide_builds":true})), Origin::Human).unwrap();
    assert!(ed.dimmed_builds().is_empty(), "the step can opt out");
    ed.apply(cmd(serde_json::json!({"op":"update_step","index":0,"hide_builds":false})), Origin::Human).unwrap();
    ed.view(cmd_view(serde_json::json!({"op":"next"})), Origin::Human).unwrap();
    assert!(ed.dimmed_builds().is_empty(), "not dimmed on a normal step");
}

fn cmd_view(j: serde_json::Value) -> ViewCommand {
    serde_json::from_value(j).unwrap()
}

#[test]
fn text_typed_on_a_shape_is_centred_wrapped_and_grouped() {
    let mut ed = Editor::default();
    let h = Origin::Human;
    ed.apply(cmd(serde_json::json!({"op":"add_shape","id":"n","shape":"sticky_note","x":100,"y":50,"w":260,"h":260})), h.clone()).unwrap();
    let out = ed.apply(cmd(serde_json::json!({"op":"add_text","text":"Ship the beta on Friday after the review","on":"n"})), h.clone()).unwrap();
    let t = ed.doc.get(&out.created[0]).unwrap().clone();
    let ElementKind::Text(td) = &t.kind else { panic!() };
    assert!(td.fixed_width && td.align == Align::Center);
    assert!((t.transform.center() - Vec2::new(100.0, 50.0)).len() < 1e-9);
    let b = t.bounds();
    assert!(b.x1 - b.x0 <= 260.0 * 0.8 + 1e-6, "wraps inside: {b:?}");
    assert!(t.group.is_some() && t.group == ed.doc.get("n").unwrap().group);
    // One undo removes the text and the grouping.
    ed.undo(h.clone()).unwrap();
    assert_eq!(ed.doc.elements.len(), 1);
    assert!(ed.doc.get("n").unwrap().group.is_none());
    // Lines and connectors have no inside.
    ed.apply(cmd(serde_json::json!({"op":"add_shape","id":"l","shape":"line","x":0,"y":0,"w":100,"h":0})), h.clone()).unwrap();
    assert!(ed.apply(cmd(serde_json::json!({"op":"add_text","text":"x","on":"l"})), h.clone()).is_err());
}

#[test]
fn a_delete_entry_keeps_the_ids_it_deleted() {
    let mut ed = Editor::default();
    let h = Origin::Human;
    ed.apply(cmd(serde_json::json!({"op":"add_shape","id":"a","shape":"rectangle","x":0,"y":0,"w":100,"h":100})), h.clone()).unwrap();
    ed.apply(cmd(serde_json::json!({"op":"add_shape","id":"b","shape":"rectangle","x":300,"y":0,"w":100,"h":100})), h.clone()).unwrap();
    ed.apply(cmd(serde_json::json!({"op":"delete","ids":["a","b"]})), Origin::agent("beta")).unwrap();
    let e = ed.history().last().unwrap();
    assert_eq!(e.label, "Delete 2 objects");
    assert_eq!(e.ids.len(), 2, "{:?}", e.ids);
    assert_eq!(ed.activity.back().unwrap().ids.len(), 2);
}

#[test]
fn grouped_agent_steps_count_up_through_the_labeler() {
    fn words(p: Phrase) -> Option<String> {
        match p {
            Phrase::Changes(n) => Some(format!("{n} Änderungen")),
            Phrase::Command(_) => Some("Schritt".into()),
        }
    }
    let mut ed = Editor::default();
    ed.labeler = Some(words);
    let a = Origin::agent("beta");
    for i in 0..3 {
        ed.apply(cmd(serde_json::json!({"op":"add_frame","x":i * 1000,"y":0,"w":800,"h":600})), a.clone()).unwrap();
    }
    assert_eq!(ed.history().len(), 1);
    assert_eq!(ed.history()[0].label, "3 Änderungen");
    assert_eq!(ed.activity.back().unwrap().label, "Schritt");
}

#[test]
fn an_agent_status_is_one_short_line_and_empty_clears_it() {
    let mut ed = Editor::default();
    let a = Origin::agent("beta");
    ed.view(cmd_view(serde_json::json!({"op":"set_status","text":"Rewriting\n  the intro"})), a.clone()).unwrap();
    assert_eq!(ed.view.agent_status["beta"].text, "Rewriting the intro");
    let long = "x".repeat(300);
    ed.view(cmd_view(serde_json::json!({"op":"set_status","text":long})), a.clone()).unwrap();
    assert_eq!(ed.view.agent_status["beta"].text.chars().count(), 120);
    ed.view(cmd_view(serde_json::json!({"op":"set_status","text":""})), a).unwrap();
    assert!(ed.view.agent_status.is_empty());
    assert!(ed.view(cmd_view(serde_json::json!({"op":"set_status","text":"hi"})), Origin::Human).is_err());
}

#[test]
fn a_frame_asked_to_find_space_lands_beside_what_is_there() {
    let mut ed = Editor::default();
    let a = Origin::agent("test");
    ed.apply(cmd(serde_json::json!({"op":"add_frame","id":"big","x":0,"y":0,"w":4000,"h":3000})), a.clone()).unwrap();
    ed.apply(cmd(serde_json::json!({"op":"add_frame","id":"a","x":0,"y":0,"w":800,"h":600})), a.clone()).unwrap();
    // Asked for the same spot: moves off "a" but stays inside "big".
    ed.apply(cmd(serde_json::json!({"op":"add_frame","id":"b","x":0,"y":0,"w":800,"h":600,"find_space":true})), a.clone()).unwrap();
    let (fa, fb) = (ed.doc.elements["a"].bounds(), ed.doc.elements["b"].bounds());
    assert!(!fa.intersects(&fb), "{fa:?} {fb:?}");
    assert!(ed.doc.elements["big"].bounds().contains_rect(&fb));
    let c = fb.center();
    assert!(c.x.hypot(c.y) < 1400.0, "too far: {c:?}");
    // Free spot asked for: kept exactly.
    ed.apply(cmd(serde_json::json!({"op":"add_frame","id":"c","x":9000,"y":9000,"find_space":true})), a.clone()).unwrap();
    assert_eq!((ed.doc.elements["c"].transform.x, ed.doc.elements["c"].transform.y), (9000.0, 9000.0));
    // Without the flag, frames still go exactly where asked.
    ed.apply(cmd(serde_json::json!({"op":"add_frame","id":"d","x":0,"y":0,"w":800,"h":600})), a).unwrap();
    assert_eq!(ed.doc.elements["d"].transform.x, 0.0);
}

#[test]
fn history_names_each_object_once() {
    let mut ed = sample();
    let agent = Origin::agent("test");
    ed.apply_batch(
        vec![cmd(serde_json::json!({"op":"move","ids":["t1"],"dx":10,"dy":0})), cmd(serde_json::json!({"op":"move","ids":["t1"],"dx":10,"dy":0}))],
        None,
        agent.clone(),
    )
    .unwrap();
    // Merged into the same entry as a quick second burst.
    ed.apply(cmd(serde_json::json!({"op":"move","ids":["t1"],"dx":10,"dy":0})), agent.clone()).unwrap();
    let last = ed.history().last().unwrap();
    assert_eq!(last.ids, vec![ElementId::from("t1")], "{:?}", last.ids);
    ed.apply(cmd(serde_json::json!({"op":"delete","ids":["t1"]})), agent).unwrap();
    assert!(ed.history().last().unwrap().ids.contains(&ElementId::from("t1")));
}

#[test]
fn objects_copy_into_another_document_with_their_pictures_groups_and_connectors() {
    use base64::Engine;
    let b64 = |b: Vec<u8>| base64::engine::general_purpose::STANDARD.encode(b);
    let h = Origin::Human;
    let mut src = Editor::default();
    src.apply(cmd(serde_json::json!({"op":"add_frame","id":"f1","x":0,"y":0,"w":800,"h":600})), h.clone()).unwrap();
    src.apply(cmd(serde_json::json!({"op":"add_image","id":"a","data_base64":b64(tiny_png(7)),"x":-100})), h.clone()).unwrap();
    src.apply(cmd(serde_json::json!({"op":"add_shape","id":"s","shape":"rectangle","x":100})), h.clone()).unwrap();
    src.apply(cmd(serde_json::json!({"op":"group","ids":["a","s"]})), h.clone()).unwrap();
    src.apply(cmd(serde_json::json!({"op":"add_shape","id":"out","shape":"rectangle","x":5000})), h.clone()).unwrap();
    src.apply(cmd(serde_json::json!({"op":"add_connector","id":"c1","from":{"element":"a"},"to":{"element":"out"}})), h.clone()).unwrap();

    // The frame brings what's inside it; the picture brings its group.
    let clip = zebra_core::clip::Clip::of(&src, &["f1".to_string(), "c1".to_string()]);
    let ids: Vec<&str> = clip.elements.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"a") && ids.contains(&"s") && ids.contains(&"c1"), "{ids:?}");
    assert!(!ids.contains(&"out"));
    assert_eq!(clip.assets.len(), 1);

    let mut dst = Editor::default();
    dst.apply(cmd(serde_json::json!({"op":"add_frame","id":"f1","x":0,"y":0})), h.clone()).unwrap();
    let out = dst.paste_clip(&clip, 10.0, 0.0, Some("Copy".into()), h.clone()).unwrap();
    assert_eq!(out.created.len(), clip.elements.len());
    assert!(!out.created.contains(&"f1".to_string()), "fresh ids, the old f1 is kept: {:?}", out.created);
    assert_eq!(dst.doc.elements.len(), 1 + clip.elements.len());

    let img = dst.doc.elements.values().find(|e| matches!(e.kind, ElementKind::Image(_))).unwrap();
    let ElementKind::Image(d) = &img.kind else { unreachable!() };
    assert!(dst.doc.assets.contains_key(&d.asset), "the picture's bytes came along");
    assert_eq!(img.transform.x, -90.0);
    let shape = dst.doc.elements.values().find(|e| matches!(e.kind, ElementKind::Shape(_))).unwrap();
    assert!(img.group.is_some() && img.group == shape.group, "still grouped");
    let conn = dst.doc.elements.values().find(|e| matches!(e.kind, ElementKind::Connector(_))).unwrap();
    let ElementKind::Connector(c) = &conn.kind else { unreachable!() };
    assert_eq!(c.from.element.as_deref(), Some(img.id.as_str()), "stuck to the copied picture");
    assert_eq!(c.to.element, None, "loose where the other end wasn't copied");

    // One undo step takes it all back.
    dst.undo(h).unwrap();
    assert_eq!(dst.doc.elements.len(), 1);
}

#[test]
fn sonnet_run_fixes_fill_ink_ids_colour_and_text_on_shapes() {
    let h = Origin::agent("sonnet");
    let mut ed = Editor::default();
    // An empty fill means none, as in `update`.
    ed.apply(cmd(serde_json::json!({"op":"add_shape","id":"s","shape":"rectangle","fill":"","stroke":"theme:accent","w":400,"h":200})), h.clone()).unwrap();
    let ElementKind::Shape(sd) = &ed.doc.elements["s"].kind else { panic!() };
    assert_eq!(sd.fill, None);
    assert!(sd.stroke.is_some());
    // add_ink keeps the id it's given.
    ed.apply(cmd(serde_json::json!({"op":"add_ink","id":"r2a","points":[[0,0],[40,30]]})), h.clone()).unwrap();
    assert!(ed.doc.elements.contains_key("r2a"));
    // Text on a shape takes a colour, and goes where the shape goes.
    ed.apply(cmd(serde_json::json!({"op":"add_text","id":"lbl","text":"Hi","on":"s","color":"#ff0000"})), h.clone()).unwrap();
    let ElementKind::Text(t) = &ed.doc.elements["lbl"].kind else { panic!() };
    assert_eq!(t.color.as_deref(), Some("#ff0000"));
    ed.apply(cmd(serde_json::json!({"op":"update","id":"s","patch":{"x":500,"y":-40}})), h.clone()).unwrap();
    let l = &ed.doc.elements["lbl"].transform;
    assert_eq!((l.x, l.y), (500.0, -40.0), "the label moved with update");
    ed.apply(cmd(serde_json::json!({"op":"move","ids":["s"],"dx":10,"dy":0})), h.clone()).unwrap();
    assert_eq!(ed.doc.elements["lbl"].transform.x, 510.0, "and with move");
    ed.apply(cmd(serde_json::json!({"op":"update","id":"s","patch":{"rotation":90}})), h.clone()).unwrap();
    assert_eq!(ed.doc.elements["lbl"].transform.rotation, 90.0, "and turns with it");
}

#[test]
fn resize_sets_the_size_on_the_canvas() {
    use base64::Engine;
    let b64 = |b: Vec<u8>| base64::engine::general_purpose::STANDARD.encode(b);
    let h = Origin::agent("sonnet");
    let mut ed = Editor::default();
    // An 8x8 picture shown at 4x.
    ed.apply(cmd(serde_json::json!({"op":"add_image","id":"i","data_base64":b64(tiny_png(1)),"width":32})), h.clone()).unwrap();
    ed.apply(cmd(serde_json::json!({"op":"resize","id":"i","width":500,"height":538})), h.clone()).unwrap();
    let (w, hh) = ed.doc.elements["i"].world_size();
    assert!((w - 500.0).abs() < 1e-6 && (hh - 500.0).abs() < 1e-6, "fits the box, keeps its shape: {w}x{hh}");
    ed.apply(cmd(serde_json::json!({"op":"resize","id":"i","height":100})), h.clone()).unwrap();
    assert!((ed.doc.elements["i"].world_size().1 - 100.0).abs() < 1e-6);

    ed.apply(cmd(serde_json::json!({"op":"add_shape","id":"s","shape":"rectangle","w":100,"h":100})), h.clone()).unwrap();
    ed.apply(cmd(serde_json::json!({"op":"scale","ids":["s"],"factor":2})), h.clone()).unwrap();
    ed.apply(cmd(serde_json::json!({"op":"resize","id":"s","width":300,"height":50})), h.clone()).unwrap();
    assert_eq!(ed.doc.elements["s"].world_size(), (300.0, 50.0));

    ed.apply(cmd(serde_json::json!({"op":"add_text","id":"t","text":"Some words","x":0,"y":0})), h.clone()).unwrap();
    assert!(ed.apply(cmd(serde_json::json!({"op":"resize","id":"t","height":50})), h.clone()).is_err(), "text height follows its words");
    ed.apply(cmd(serde_json::json!({"op":"resize","id":"t","width":400})), h.clone()).unwrap();
    assert!((ed.doc.elements["t"].world_size().0 - 400.0).abs() < 1e-6);
    assert!(ed.apply(cmd(serde_json::json!({"op":"resize","id":"t"})), h).is_err());
}

#[test]
fn clearing_the_path_keeps_speaker_notes_unless_told() {
    let h = Origin::agent("sonnet");
    let mut ed = sample();
    ed.apply(cmd(serde_json::json!({"op":"update_step","index":1,"notes":"Mention the budget"})), h.clone()).unwrap();
    let e = ed.apply(cmd(serde_json::json!({"op":"clear_path"})), h.clone()).unwrap_err();
    assert!(e.to_string().contains("2"), "{e}");
    assert_eq!(ed.doc.path.len(), 2);
    // Path from Frames keeps the notes of a frame that was already a step.
    ed.apply(cmd(serde_json::json!({"op":"auto_path"})), h.clone()).unwrap();
    assert!(ed.doc.path.iter().any(|s| s.notes == "Mention the budget"));
    ed.apply(cmd(serde_json::json!({"op":"clear_path","discard_notes":true})), h).unwrap();
    assert!(ed.doc.path.is_empty());
}
