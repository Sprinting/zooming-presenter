//! Regression tests for the adversarial review of 18 Nov 2026 (build
//! 7630968). Each test is named after the finding it pins down.
use serde_json::json;
use zebra_core::view::Mode;
use zebra_core::*;

const H: Origin = Origin::Human;

fn cmd(j: serde_json::Value) -> Command {
    serde_json::from_value(j).unwrap()
}

fn agent() -> Origin {
    Origin::agent("probe")
}

fn rect(ed: &mut Editor, id: &str, x: f64) {
    ed.apply(cmd(json!({"op":"add_shape","id":id,"shape":"rectangle","x":x,"y":0,"w":100,"h":100})), H).unwrap();
}

/// Three frames on the path.
fn deck() -> Editor {
    let mut ed = Editor::default();
    for (i, x) in [0, 2000, 4000].iter().enumerate() {
        ed.apply(cmd(json!({"op":"add_frame","id":format!("f{}", i + 1),"x":x,"y":0,"w":800,"h":600,"add_to_path":true})), H).unwrap();
    }
    ed
}

fn save_load(ed: &Editor) -> Document {
    io::load_bytes(&io::save_bytes(&ed.doc).unwrap()).unwrap()
}

// ---------------------------------------------------------------- crashes

#[test]
fn c2_presenting_survives_the_path_getting_shorter() {
    let mut ed = deck();
    ed.view(ViewCommand::Present { step: Some(2) }, H).unwrap();
    // Undo of the last add_frame removes step 3 under the show.
    ed.undo(H).unwrap();
    assert_eq!(ed.doc.path.len(), 2);
    assert_eq!(ed.view.present.step, Some(1), "the position moves back onto the path");
    assert!(ed.at_end());
    for c in [ViewCommand::Prev, ViewCommand::Next, ViewCommand::Next, ViewCommand::Prev] {
        ed.view(c, H).unwrap();
    }
    // An agent with full permission clears the whole path mid-show.
    ed.view.agent_permission = zebra_core::view::Permission::Full;
    ed.view(ViewCommand::GotoStep { step: 1 }, H).unwrap();
    ed.apply(cmd(json!({"op":"clear_path"})), agent()).unwrap();
    assert_eq!(ed.view.present.step, None);
    for c in [ViewCommand::Prev, ViewCommand::Next] {
        ed.view(c, H).unwrap();
    }
    assert_eq!(ed.view.mode, Mode::Present);
}

#[test]
fn c2_a_step_losing_builds_keeps_the_build_count_in_range() {
    let mut ed = deck();
    rect(&mut ed, "a", 0.0);
    ed.apply(cmd(json!({"op":"update_step","index":0,"builds":["a"]})), H).unwrap();
    ed.view(ViewCommand::Present { step: Some(0) }, H).unwrap();
    ed.view(ViewCommand::Next, H).unwrap();
    assert_eq!(ed.view.present.build, 1);
    ed.undo(H).unwrap();
    assert_eq!(ed.view.present.build, 0);
}

#[test]
fn c3_formatting_a_missing_id_is_an_error_not_a_panic() {
    let mut ed = deck();
    for c in [json!({"op":"format_text","id":"nope","format":{"bold":true}}), json!({"op":"format_paragraphs","id":"nope","indent":1})] {
        let e = ed.apply(cmd(c), agent()).unwrap_err();
        assert_eq!(e.code, "not_found");
    }
}

#[test]
fn c4_revert_agent_changes_stops_at_an_entry_that_will_not_undo() {
    let mut ed = Editor::default();
    rect(&mut ed, "a", 0.0);
    ed.apply(cmd(json!({"op":"move","ids":["a"],"dx":10,"dy":0})), agent()).unwrap();
    // Break the document behind the history's back, so the agent's entry
    // can no longer be undone (its patch no longer applies).
    ed.doc.elements.remove("a");
    ed.doc.z_order.clear();
    let n = ed.revert_agent_session("probe");
    assert_eq!(n, 0, "it gives up instead of looping for ever");
}

#[test]
fn c5_text_formats_out_of_range_are_clipped_when_added_and_when_loaded() {
    let mut ed = Editor::default();
    let text = json!({"type":"text","text":"ab\ncd",
        "paras":[{"indent":9,"list":"number"},{"indent":200}],
        "spans":[{"start":3,"end":99,"bold":true},{"start":2,"end":2,"italic":true},{"start":4,"end":1,"strike":true}]});
    ed.apply(cmd(json!({"op":"add_element","element":{"id":"t","kind":text.clone()}})), agent()).unwrap();
    let ElementKind::Text(t) = &ed.doc.elements["t"].kind else { panic!() };
    assert!(t.paras.iter().all(|p| p.indent <= 8));
    assert!(t.spans.iter().all(|s| s.start < s.end && s.end <= 5));
    ed.query(Query::Describe).unwrap();

    // The same from a file written by something else.
    let mut v: serde_json::Value = serde_json::from_str(&io::to_json(&ed.doc)).unwrap();
    v["elements"]["t"]["kind"] = text;
    let doc = io::from_json(&v.to_string()).unwrap();
    let ElementKind::Text(t) = &doc.elements["t"].kind else { panic!() };
    assert!(t.paras.iter().all(|p| p.indent <= 8) && t.spans.iter().all(|s| s.start < s.end && s.end <= 5));
    let mut ed = Editor::default();
    ed.load(doc, None);
    ed.query(Query::Describe).unwrap();

    // to_markdown itself never trusts the ranges.
    let raw = TextData {
        text: "ab".into(),
        paras: vec![ParaFormat { indent: 40, list: Some(ListKind::Number), align: None }],
        spans: vec![TextSpan { start: 0, end: 0, ..Default::default() }, TextSpan { start: 1, end: 9, ..Default::default() }],
        ..Default::default()
    };
    assert!(zebra_core::rich::to_markdown(&raw).contains("ab"));
}

#[test]
fn c6_a_colour_with_a_non_ascii_character_is_refused() {
    for c in ["#é1", "#€", "#ééé", "#12é45", "#１２３"] {
        assert_eq!(zebra_core::theme::parse_hex(c), None, "{c}");
    }
    assert_eq!(zebra_core::theme::parse_hex("#fa0"), Some([255, 170, 0, 255]));
    let mut ed = Editor::default();
    let e = ed.apply(cmd(json!({"op":"add_shape","shape":"rectangle","x":0,"y":0,"w":10,"h":10,"fill":"#é1"})), agent()).unwrap_err();
    assert_eq!(e.code, "invalid_params");
}

// ------------------------------------------------------- data loss, trust

#[test]
fn d1_numbers_that_overflow_are_refused_and_the_file_still_opens() {
    let mut ed = Editor::default();
    rect(&mut ed, "a", 0.0);
    ed.apply(cmd(json!({"op":"move","ids":["a"],"dx":9e14,"dy":0})), agent()).unwrap();
    let before = ed.doc.clone();
    let bad = [
        json!({"op":"move","ids":["a"],"dx":1e308,"dy":0}),
        json!({"op":"move","ids":["a"],"dx":9e14,"dy":0}),
        json!({"op":"rotate","ids":["a"],"degrees":1e308,"about":[1e300,0]}),
        json!({"op":"scale","ids":["a"],"factor":1e-320}),
        json!({"op":"scale","ids":["a"],"factor":1e300}),
        json!({"op":"update","id":"a","patch":{"x":1.7e308,"w":1.7e308}}),
        json!({"op":"add_ink","points":[[-1e308,0],[1e308,0]],"color":"#000000","width":4}),
        json!({"op":"wrap_in_frame","ids":["a"],"padding":5e305}),
        json!({"op":"add_frame","x":0,"y":0,"w":1e308,"h":1e308,"find_space":true}),
        json!({"op":"arrange","ids":["a"],"layout":"row","gap":f64::MAX}),
    ];
    for c in bad {
        let label = c.to_string();
        assert!(ed.apply(cmd(c), agent()).is_err(), "{label} should be refused");
        assert_eq!(ed.doc, before, "{label} left something behind");
    }
    // Everything still saves, reloads and undoes.
    assert_eq!(save_load(&ed).elements["a"].transform.x, 9e14);
    ed.undo(H).unwrap();
    assert_eq!(ed.doc.elements["a"].transform.x, 0.0);
}

#[test]
fn d1_a_file_with_null_numbers_opens_repaired() {
    let mut ed = Editor::default();
    rect(&mut ed, "a", 5.0);
    rect(&mut ed, "b", 300.0);
    let mut v: serde_json::Value = serde_json::from_str(&io::to_json(&ed.doc)).unwrap();
    v["elements"]["a"]["transform"]["x"] = serde_json::Value::Null;
    v["elements"]["a"]["size"]["w"] = serde_json::Value::Null;
    v["z_order"] = json!(["a", "a", "b", "ghost"]);
    let doc = io::from_json(&v.to_string()).expect("the file opens");
    assert_eq!(doc.z_order, vec!["a".to_string(), "b".to_string()]);
    assert!(zebra_core::check::element(&doc.elements["a"]).is_ok());
    assert_eq!(doc.elements["b"], ed.doc.elements["b"], "a sound element is untouched");
}

#[test]
fn d3_an_agents_undo_never_ends_the_persons_drag() {
    let mut ed = Editor::default();
    rect(&mut ed, "a", 0.0);
    ed.begin("Move", H).unwrap();
    ed.apply(cmd(json!({"op":"move","ids":["a"],"dx":50,"dy":0})), H).unwrap();
    let e = ed.undo(agent()).unwrap_err();
    assert_eq!(e.code, "busy");
    assert!(ed.human_gesture_active(), "the drag is still open");
    assert_eq!(ed.doc.elements["a"].transform.x, 50.0);
    assert_eq!(ed.redo(agent()).unwrap_err().code, "busy");
    ed.commit_as(&H);
    assert_eq!(ed.undo(agent()).unwrap().as_deref(), Some("Move"));
}

#[test]
fn r23_the_end_of_a_drag_does_not_commit_an_agents_transaction() {
    let mut ed = Editor::default();
    rect(&mut ed, "a", 0.0);
    ed.begin("Batch", agent()).unwrap();
    ed.apply(cmd(json!({"op":"move","ids":["a"],"dx":50,"dy":0})), agent()).unwrap();
    assert!(!ed.commit_as(&H), "a mouse-up is the person's commit only");
    assert!(!ed.abort_as(&H));
    assert!(ed.in_transaction());
    assert_eq!(ed.transaction_owner(), Some(&agent()));
    assert!(ed.commit_as(&agent()));
    assert_eq!(ed.history().last().unwrap().label, "Batch");
}

#[test]
fn d4_auto_path_will_not_drop_notes_without_a_yes() {
    let mut ed = deck();
    ed.apply(cmd(json!({"op":"add_step","target":{"kind":"overview"},"notes":"Thank everyone"})), H).unwrap();
    ed.apply(cmd(json!({"op":"update_step","index":0,"notes":"On a frame"})), H).unwrap();
    let before = ed.doc.clone();
    let e = ed.apply(cmd(json!({"op":"auto_path"})), agent()).unwrap_err();
    assert_eq!(e.code, "notes_lost");
    assert!(e.message.contains('4'), "{e}");
    assert_eq!(ed.doc, before);
    ed.apply(cmd(json!({"op":"auto_path","discard_notes":true})), agent()).unwrap();
    assert_eq!(ed.doc.path.len(), 3);
    assert_eq!(ed.doc.path[0].notes, "On a frame", "a frame's notes always stay");
}

#[test]
fn d5_abort_drops_the_selection_of_what_it_removed() {
    let mut ed = Editor::default();
    ed.begin("Draw", H).unwrap();
    rect(&mut ed, "a", 0.0);
    ed.view(ViewCommand::Select { ids: vec!["a".into()] }, H).unwrap();
    ed.abort();
    assert!(ed.view.selection.is_empty());
}

// --------------------------------------------------------- wrong behaviour

#[test]
fn w1_duplicating_shapes_with_their_connector_connects_the_copies() {
    let mut ed = Editor::default();
    rect(&mut ed, "a", 0.0);
    rect(&mut ed, "b", 400.0);
    ed.apply(cmd(json!({"op":"add_connector","id":"c","from":{"element":"a"},"to":{"element":"b"}})), H).unwrap();
    let out = ed.apply(cmd(json!({"op":"duplicate","ids":["a","b","c"],"dx":0,"dy":300})), H).unwrap();
    let copy = out.created.iter().find_map(|id| match &ed.doc.elements[id].kind {
        ElementKind::Connector(c) => Some((id.clone(), c.clone())),
        _ => None,
    });
    let (cid, c) = copy.expect("the connector was copied");
    let (from, to) = (c.from.element.unwrap(), c.to.element.unwrap());
    assert!(out.created.contains(&from) && out.created.contains(&to), "ends point at the copies");
    assert!((ed.doc.elements[&cid].transform.y - 300.0).abs() < 60.0, "and it sits with them");
    // Copied alone, it comes loose rather than staying tied to the originals.
    let out = ed.apply(cmd(json!({"op":"duplicate","ids":["c"],"dx":0,"dy":600})), H).unwrap();
    let ElementKind::Connector(c) = &ed.doc.elements[&out.created[0]].kind else { panic!() };
    assert!(c.from.element.is_none() && c.to.element.is_none());
    assert!((ed.doc.elements[&out.created[0]].transform.y - 600.0).abs() < 60.0, "a loose copy is offset");
}

#[test]
fn w2_a_locked_object_is_not_arranged_reordered_or_grouped() {
    let mut ed = Editor::default();
    rect(&mut ed, "a", 0.0);
    rect(&mut ed, "b", 400.0);
    ed.apply(cmd(json!({"op":"update","id":"a","patch":{"locked":true}})), H).unwrap();
    let before = ed.doc.clone();
    for c in [
        json!({"op":"arrange","ids":["a","b"],"layout":"row"}),
        json!({"op":"z_order","ids":["a"],"order":"front"}),
        json!({"op":"group","ids":["a","b"]}),
        json!({"op":"align","ids":["a","b"],"edge":"left"}),
        json!({"op":"move","ids":["a"],"dx":5,"dy":5}),
    ] {
        let label = c.to_string();
        assert_eq!(ed.apply(cmd(c), H).unwrap_err().code, "locked", "{label}");
        assert_eq!(ed.doc, before, "{label}");
    }
}

#[test]
fn w3_z_order_with_an_id_twice_keeps_one_entry() {
    let mut ed = Editor::default();
    rect(&mut ed, "a", 0.0);
    rect(&mut ed, "b", 400.0);
    for order in ["front", "back", "forward", "backward"] {
        ed.apply(cmd(json!({"op":"z_order","ids":["a","a"],"order":order})), agent()).unwrap();
        let mut z = ed.doc.z_order.clone();
        z.sort();
        assert_eq!(z, vec!["a".to_string(), "b".to_string()], "{order}");
    }
}

#[test]
fn w4_find_space_for_a_long_thin_frame_is_quick() {
    let mut ed = Editor::default();
    for i in 0..40 {
        rect(&mut ed, &format!("r{i}"), i as f64 * 150.0);
    }
    let t = std::time::Instant::now();
    ed.apply(cmd(json!({"op":"add_frame","x":0,"y":0,"w":100000,"h":1,"find_space":true})), agent()).unwrap();
    assert!(t.elapsed().as_millis() < 500, "took {:?}", t.elapsed());
}

#[test]
fn w5_deleting_a_shape_takes_its_label_with_it() {
    let mut ed = Editor::default();
    rect(&mut ed, "a", 0.0);
    ed.apply(cmd(json!({"op":"add_text","id":"t","text":"Label","on":"a"})), H).unwrap();
    assert!(ed.doc.elements["t"].group.is_some());
    ed.apply(cmd(json!({"op":"delete","ids":["a"]})), H).unwrap();
    assert!(ed.doc.elements.is_empty(), "{:?}", ed.doc.elements.keys());
    ed.undo(H).unwrap();
    assert_eq!(ed.doc.elements.len(), 2);

    // A group of three losing one stays a group; losing two, it dissolves.
    let mut ed = Editor::default();
    for (i, id) in ["a", "b", "c"].iter().enumerate() {
        rect(&mut ed, id, i as f64 * 300.0);
    }
    ed.apply(cmd(json!({"op":"group","ids":["a","b","c"]})), H).unwrap();
    ed.apply(cmd(json!({"op":"delete","ids":["a"]})), agent()).unwrap();
    assert!(ed.doc.elements["b"].group.is_some());
    ed.apply(cmd(json!({"op":"delete","ids":["b"]})), agent()).unwrap();
    assert!(ed.doc.elements["c"].group.is_none(), "a group of one is no group");
}

#[test]
fn w6_numbers_survive_save_and_load_exactly() {
    let mut ed = Editor::default();
    let xs = [0.1 + 0.2, 1.0 / 3.0, 123456.789e-7, 5e-324, 987654321012345.6, 0.30000000000000004, 2.2250738585072014e-308];
    for (i, x) in xs.iter().enumerate() {
        ed.apply(cmd(json!({"op":"add_shape","id":format!("s{i}"),"shape":"rectangle","x":x,"y":x * 0.7,"w":100,"h":100})), H).unwrap();
    }
    let back = save_load(&ed);
    for (id, e) in &ed.doc.elements {
        assert_eq!(e.transform.x.to_bits(), back.elements[id].transform.x.to_bits(), "{id}");
        assert_eq!(e.transform.y.to_bits(), back.elements[id].transform.y.to_bits(), "{id}");
    }
}

// ------------------------------------------------------- found by reading

fn zip_with(entries: &[(&str, &[u8])], doc: &Document) -> Vec<u8> {
    use std::io::Write;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    z.start_file("document.json", o).unwrap();
    z.write_all(io::to_json(doc).as_bytes()).unwrap();
    for (name, bytes) in entries {
        z.start_file(*name, o).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
fn r27_an_asset_name_cannot_leave_the_media_cache() {
    let mp4 = zebra_core::media::fixtures::tiny_mp4(320, 240, 10, true);
    let bytes = zip_with(&[("assets/../../evil.mp4", &mp4), ("assets/sub/dir.mp4", &mp4), ("assets/ok123.mp4", &mp4)], &Document::default());
    let doc = io::load_bytes(&bytes).unwrap();
    assert_eq!(doc.assets.keys().collect::<Vec<_>>(), vec!["ok123"]);
    for key in ["../../evil", "a/b", "..", "", "a.b"] {
        assert!(!zebra_core::check::safe_asset_key(key), "{key}");
        assert!(zebra_core::media::materialize(key, "mp4", &mp4).is_err(), "{key}");
    }
    assert!(zebra_core::media::materialize("ok", "../mp4", &mp4).is_err());
}

#[test]
fn r11_one_unreadable_asset_does_not_stop_the_file_opening() {
    let mut ed = Editor::default();
    rect(&mut ed, "a", 0.0);
    let bytes = zip_with(&[("assets/abc.xyz", b"not media at all"), ("assets/def.png", b"junk")], &ed.doc);
    let doc = io::load_bytes(&bytes).expect("opens");
    assert!(doc.assets.is_empty());
    assert_eq!(doc.elements.len(), 1);
}

#[test]
fn r28_a_frame_and_its_child_selected_together_move_once() {
    let mut ed = Editor::default();
    ed.apply(cmd(json!({"op":"add_frame","id":"f","x":0,"y":0,"w":800,"h":600})), H).unwrap();
    rect(&mut ed, "in", 100.0);
    rect(&mut ed, "out", 3000.0);
    let rel = |ed: &Editor| ed.doc.elements["in"].transform.x - ed.doc.elements["f"].transform.x;
    let r0 = rel(&ed);
    ed.apply(cmd(json!({"op":"align","ids":["f","in","out"],"edge":"right"})), H).unwrap();
    assert!((rel(&ed) - r0).abs() < 1e-9, "the child kept its place in the frame");
    ed.apply(cmd(json!({"op":"arrange","ids":["f","in","out","f"],"layout":"column"})), H).unwrap();
    assert!((rel(&ed) - r0).abs() < 1e-9);
}

#[test]
fn r14_a_camera_needs_a_positive_zoom() {
    let mut ed = Editor::default();
    for zoom in [0.0, -2.0, f64::NAN] {
        let cam = Camera { cx: 0.0, cy: 0.0, zoom, rotation: 0.0 };
        assert!(ed.view(ViewCommand::SetCamera { camera: cam, animate: false }, H).is_err(), "{zoom}");
    }
    assert_eq!(ed.view.camera, Camera::default());
}

#[test]
fn undo_and_redo_say_which_elements_changed() {
    let mut ed = Editor::default();
    rect(&mut ed, "a", 0.0);
    rect(&mut ed, "b", 300.0);
    ed.apply(cmd(json!({"op":"move","ids":["b"],"dx":5,"dy":0})), H).unwrap();
    ed.drain_events();
    ed.undo(H).unwrap();
    let ev = ed.drain_events();
    let ids = ev.iter().find_map(|e| match e {
        zebra_core::editor::Event::DocumentChanged { ids, .. } => Some(ids.clone()),
        _ => None,
    });
    assert_eq!(ids, Some(vec!["b".to_string()]));
}

/// Speed fix 5: undo used to cost about 35 times an edit, because the
/// whole draw order and path went through JSON.
#[test]
fn undo_in_a_big_deck_costs_about_what_the_edit_did() {
    let mut ed = Editor::default();
    let cmds: Vec<Command> = (0..20_000).map(|i| cmd(json!({"op":"add_shape","shape":"rectangle","x":(i % 200) * 150,"y":(i / 200) * 150,"w":100,"h":100}))).collect();
    ed.apply_batch(cmds, Some("fill".into()), H).unwrap();
    let first = ed.doc.z_order[0].clone();
    let time = |f: &mut dyn FnMut()| {
        let t = std::time::Instant::now();
        f();
        t.elapsed().as_secs_f64()
    };
    let (mut edit, mut undo) = (f64::MAX, f64::MAX);
    for _ in 0..5 {
        edit = edit.min(time(&mut || {
            ed.apply(cmd(json!({"op":"z_order","ids":[first],"order":"front"})), H).unwrap();
        }));
        undo = undo.min(time(&mut || {
            ed.undo(H).unwrap();
        }));
    }
    assert_eq!(ed.doc.z_order[0], first);
    assert!(undo < edit * 3.0 + 0.02, "edit {:.1} ms, undo {:.1} ms", edit * 1e3, undo * 1e3);
}

/// R13: deleting a custom theme goes back to the theme it was made from.
#[test]
fn r13_deleting_a_custom_theme_returns_to_its_base() {
    let mut ed = Editor::default();
    let mut t = Theme::by_id("night").expect("a built-in theme");
    t.base = Some(t.id.clone());
    t.id = "custom-1".into();
    ed.apply(Command::SaveTheme { theme: t.clone(), apply: true }, H).unwrap();
    assert_eq!(ed.doc.theme, "custom-1");
    ed.apply(Command::DeleteTheme { id: "custom-1".into() }, H).unwrap();
    assert_eq!(ed.doc.theme, "night");
    // A base that no longer exists: the default.
    t.base = Some("custom-9".into());
    ed.apply(Command::SaveTheme { theme: t, apply: true }, H).unwrap();
    ed.apply(Command::DeleteTheme { id: "custom-1".into() }, H).unwrap();
    assert_eq!(ed.doc.theme, "classic");
}

/// R20: lint on a big deck is quick (it compared every pair of elements,
/// measuring text for each), and still finds the overlaps.
#[test]
fn r20_lint_is_fast_on_a_big_deck_and_still_finds_overlaps() {
    let mut ed = Editor::default();
    let mut cmds = vec![];
    for i in 0..3000 {
        let (x, y) = ((i % 60) as f64 * 400.0, (i / 60) as f64 * 300.0);
        cmds.push(Command::AddShape { shape: ShapeKind::Rectangle, x, y, w: 200.0, h: 100.0, rotation: 0.0, fill: None, stroke: None, id: Some(format!("s{i}").as_str().into()) });
    }
    // Two that overlap half and half.
    cmds.push(Command::AddShape { shape: ShapeKind::Rectangle, x: 100.0, y: 0.0, w: 200.0, h: 100.0, rotation: 0.0, fill: None, stroke: None, id: Some("over".into()) });
    ed.apply_batch(cmds, None, H).unwrap();
    let t = std::time::Instant::now();
    let issues = zebra_core::describe::lint(&ed);
    assert!(t.elapsed().as_secs_f64() < 2.0, "lint took {:?}", t.elapsed());
    let overlaps: Vec<&serde_json::Value> = issues.iter().filter(|i| i["kind"] == "overlap").collect();
    assert_eq!(overlaps.len(), 1, "{overlaps:?}");
    assert_eq!(overlaps[0]["ids"], serde_json::json!(["s0", "over"]));
}

/// R28: a picture cropped to no width at all is refused; it used to give
/// a transform of infinity or NaN.
#[test]
fn r28_a_crop_of_no_size_is_refused() {
    let mut ed = Editor::default();
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let id = ed.apply(cmd(json!({"op":"add_image","data_base64":png,"x":0,"y":0})), H).unwrap().created[0].clone();
    let mut kind = serde_json::to_value(&ed.doc.get(&id).unwrap().kind).unwrap();
    let n = ed.doc.elements.len();
    for (w, h) in [(0.0, 0.5), (0.5, 0.0), (-0.2, 0.5)] {
        let crop = json!({"x":0.0,"y":0.0,"w":w,"h":h});
        kind["crop"] = crop.clone();
        let r = ed.apply(cmd(json!({"op":"add_element","element":{"x":0,"y":0,"kind":kind}})), agent());
        assert!(r.is_err(), "a new picture with crop {w} x {h} must be refused");
        // As a change to a picture that exists, the crop is pulled back
        // into range instead (never narrower than 1%).
        ed.apply(cmd(json!({"op":"update","id":id,"patch":{"crop":crop}})), agent()).unwrap();
        let e = ed.doc.get(&id).unwrap();
        let zebra_core::ElementKind::Image(d) = &e.kind else { unreachable!() };
        assert!(d.crop.is_none_or(|c| c.w >= 0.01 && c.h >= 0.01 && c.w <= 1.0 && c.h <= 1.0), "{:?}", d.crop);
        assert!(e.size.w.is_finite() && e.size.h.is_finite() && e.size.w > 0.0 && e.size.h > 0.0, "{:?}", e.size);
        assert!(e.transform.scale.is_finite());
    }
    assert_eq!(ed.doc.elements.len(), n);
    let e = ed.doc.get(&id).unwrap();
    assert!(e.size.w.is_finite() && e.size.h.is_finite() && e.size.w > 0.0);
}
