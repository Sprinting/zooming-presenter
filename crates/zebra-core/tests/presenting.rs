//! Stream 4: notes, autoplay, custom themes, templates and layouts.
use base64::Engine;
use serde_json::json;
use zebra_core::command::*;
use zebra_core::view::Mode;
use zebra_core::*;

const H: Origin = Origin::Human;

fn cmd(j: serde_json::Value) -> Command {
    serde_json::from_value(j).unwrap()
}

fn three_steps() -> Editor {
    let mut ed = Editor::default();
    ed.apply_batch(
        vec![
            cmd(json!({"op":"add_frame","id":"f1","x":0,"y":0,"add_to_path":true})),
            cmd(json!({"op":"add_frame","id":"f2","x":2000,"y":0,"add_to_path":true})),
            cmd(json!({"op":"add_text","id":"t1","text":"x","x":2000,"y":0})),
            cmd(json!({"op":"add_frame","id":"f3","x":4000,"y":0,"add_to_path":true})),
            cmd(json!({"op":"update_step","index":1,"builds":["t1"]})),
        ],
        Some("build".into()),
        H,
    )
    .unwrap();
    ed
}

/// Run the clock forward to `t`, ticking like a host at 60 fps.
fn run_until(ed: &mut Editor, t: f64) {
    let mut now = ed.view.now;
    while now < t {
        now += 1.0 / 60.0;
        ed.view.tick(now);
        ed.autoplay_tick();
    }
}

fn pos(ed: &Editor) -> (Option<usize>, usize) {
    (ed.view.present.step, ed.view.present.build)
}

#[test]
fn notes_and_timings_are_step_fields() {
    let mut ed = three_steps();
    ed.apply(cmd(json!({"op":"update_step","index":0,"notes":"Open with a question","duration":2.5,"dwell":7})), H).unwrap();
    let s = &ed.doc.path[0];
    assert_eq!((s.notes.as_str(), s.duration, s.dwell), ("Open with a question", Some(2.5), Some(7.0)));
    ed.apply(cmd(json!({"op":"update_step","index":0,"dwell":-1})), H).unwrap();
    assert_eq!(ed.doc.path[0].dwell, None);
    let p = ed.query(Query::Path).unwrap();
    assert_eq!(p["steps"][0]["notes"], "Open with a question");
}

#[test]
fn autoplay_advances_steps_and_builds_then_stops() {
    let mut ed = three_steps();
    ed.apply(cmd(json!({"op":"set_playback","autoplay":4})), H).unwrap();
    assert_eq!(ed.doc.play.autoplay, Some(4.0));
    ed.view(ViewCommand::Present { step: Some(0) }, H).unwrap();
    assert_eq!(pos(&ed), (Some(0), 0));
    // Nothing happens before the camera has rested for 4 s.
    run_until(&mut ed, 2.0);
    assert_eq!(pos(&ed), (Some(0), 0));
    run_until(&mut ed, 6.0);
    assert_eq!(pos(&ed), (Some(1), 0));
    // Builds are sub-steps.
    run_until(&mut ed, 12.0);
    assert_eq!(pos(&ed), (Some(1), 1));
    run_until(&mut ed, 30.0);
    assert_eq!(pos(&ed), (Some(2), 0));
    // No loop: stays at the end.
    run_until(&mut ed, 60.0);
    assert_eq!(pos(&ed), (Some(2), 0));
    assert_eq!(ed.autoplay_tick(), None);
}

#[test]
fn autoplay_loops_pauses_and_honours_dwell() {
    let mut ed = three_steps();
    ed.apply(cmd(json!({"op":"set_playback","autoplay":10,"loop":true})), H).unwrap();
    ed.apply(cmd(json!({"op":"update_step","index":0,"dwell":2})), H).unwrap();
    ed.view(ViewCommand::Present { step: Some(2) }, H).unwrap();
    // Record where autoplay goes and when.
    let mut seen = vec![(0.0, pos(&ed))];
    let mut now = 0.0;
    while now < 30.0 {
        now += 1.0 / 60.0;
        ed.view.tick(now);
        ed.autoplay_tick();
        if seen.last().unwrap().1 != pos(&ed) {
            seen.push((now, pos(&ed)));
        }
    }
    let order: Vec<_> = seen.iter().map(|s| s.1).collect();
    assert_eq!(&order[..3], &[(Some(2), 0), (Some(0), 0), (Some(1), 0)], "loops back to the first step: {seen:?}");
    // Step 0 dwells 2 s (plus its flight), not 10.
    let gap = seen[2].0 - seen[1].0;
    assert!(gap < 6.0, "{seen:?}");
    ed.view(ViewCommand::GotoStep { step: 1 }, H).unwrap();
    // Paused: nothing moves.
    ed.view(ViewCommand::PauseAutoplay { paused: true }, H).unwrap();
    let t = ed.view.now;
    run_until(&mut ed, t + 40.0);
    assert_eq!(pos(&ed), (Some(1), 0));
    ed.view(ViewCommand::PauseAutoplay { paused: false }, H).unwrap();
    let t = ed.view.now;
    run_until(&mut ed, t + 11.0);
    assert_eq!(pos(&ed), (Some(1), 1));
    // Turning autoplay off.
    ed.view(ViewCommand::ExitPresent, H).unwrap();
    ed.apply(cmd(json!({"op":"set_playback","autoplay":0})), H).unwrap();
    assert!(ed.doc.play.autoplay.is_none() && ed.doc.play.looping);
    assert_eq!(ed.view.mode, Mode::Edit);
}

fn png(w: u32, h: u32) -> String {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
    let mut b = std::io::Cursor::new(vec![]);
    img.write_to(&mut b, image::ImageFormat::Png).unwrap();
    base64::engine::general_purpose::STANDARD.encode(b.into_inner())
}

fn my_theme(ed: &Editor) -> serde_json::Value {
    let mut t = serde_json::to_value(Theme::resolve("paper")).unwrap();
    t["id"] = "mine".into();
    t["name"] = "Mine".into();
    t["background"] = "#102030".into();
    t["title"]["font"] = "Serif".into();
    let _ = ed;
    t
}

#[test]
fn custom_theme_saves_applies_undoes_and_round_trips() {
    let mut ed = three_steps();
    let out = ed.apply(cmd(json!({"op":"add_asset","data_base64": png(64, 32)})), H).unwrap();
    let key = out.assets[0].clone();
    let mut t = my_theme(&ed);
    t["layers"] = json!([{"asset": key, "depth": 0.2}, {"asset": key, "depth": 0.5, "opacity": 0.5}]);
    t["background_image"] = key.clone().into();
    ed.apply(cmd(json!({"op":"save_theme","theme": t, "apply": true})), H).unwrap();
    assert_eq!(ed.doc.theme, "mine");
    assert_eq!(ed.theme().background, "#102030");
    assert_eq!(ed.theme().layers.len(), 2);
    assert!(ed.query(Query::Themes).unwrap()["custom"][0]["id"] == "mine");

    // Undo removes it and restores the old theme.
    ed.undo(H).unwrap();
    assert_eq!(ed.doc.theme, "classic");
    assert!(ed.doc.themes.is_empty());
    ed.redo(H).unwrap();
    assert_eq!(ed.doc.theme, "mine");

    // The file keeps the theme and its pictures.
    let bytes = io::save_bytes(&ed.doc).unwrap();
    let back = io::load_bytes(&bytes).unwrap();
    assert_eq!(back.themes, ed.doc.themes);
    assert!(back.assets.contains_key(&key));

    // Deleting the theme in use falls back to classic.
    ed.apply(cmd(json!({"op":"delete_theme","id":"mine"})), H).unwrap();
    assert_eq!(ed.doc.theme, "classic");
}

#[test]
fn bad_themes_are_refused() {
    let mut ed = three_steps();
    let mut t = my_theme(&ed);
    t["id"] = "classic".into();
    let e = ed.apply(cmd(json!({"op":"save_theme","theme": t})), H).unwrap_err();
    assert!(e.message.contains("built-in"), "{e:?}");
    let mut t = my_theme(&ed);
    t["accent"] = "theme:accent".into();
    assert!(ed.apply(cmd(json!({"op":"save_theme","theme": t})), H).is_err());
    let mut t = my_theme(&ed);
    t["layers"] = json!([{"asset":"nope"}]);
    assert_eq!(ed.apply(cmd(json!({"op":"save_theme","theme": t})), H).unwrap_err().code, "not_found");
    let mut t = my_theme(&ed);
    t["layers"] = json!([{"asset":"a"},{"asset":"a"},{"asset":"a"},{"asset":"a"}]);
    assert!(ed.apply(cmd(json!({"op":"save_theme","theme": t})), H).is_err());
    assert!(ed.doc.themes.is_empty());
}

#[test]
fn every_template_builds_a_presentation() {
    let list = ed_templates();
    assert_eq!(list.len(), 8);
    for id in list {
        let mut ed = Editor::default();
        ed.apply(cmd(json!({"op":"apply_template","template": id})), H).unwrap();
        if id == "blank" {
            assert!(ed.doc.elements.is_empty());
            continue;
        }
        assert!(ed.doc.path.len() >= 3, "{id}: {} steps", ed.doc.path.len());
        assert!(ed.doc.elements.values().any(|e| e.placeholder), "{id} has placeholders");
        for s in &ed.doc.path {
            let StepTarget::Element { id: f } = &s.target else { panic!() };
            assert!(ed.doc.get(f).unwrap().is_frame());
        }
        // Every text sits inside some frame and is readable size-wise.
        for e in ed.doc.elements.values().filter(|e| matches!(e.kind, ElementKind::Text(_))) {
            assert!(ed.doc.parent_frame(&e.id).is_some(), "{id}: text {} not in a frame", e.id);
        }
        // Applying again needs `replace`.
        assert_eq!(ed.apply(cmd(json!({"op":"apply_template","template":"pitch"})), H).unwrap_err().code, "not_empty");
        let n = ed.doc.elements.len();
        ed.apply(cmd(json!({"op":"apply_template","template":"pitch","replace":true})), H).unwrap();
        assert_ne!(ed.doc.elements.len(), 0);
        ed.undo(H).unwrap();
        assert_eq!(ed.doc.elements.len(), n);
    }
}

fn ed_templates() -> Vec<String> {
    let v = Editor::default().query(Query::Templates).unwrap();
    v["templates"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn layouts_insert_frames_on_the_path_with_placeholders() {
    let v = Editor::default().query(Query::Templates).unwrap();
    let layouts: Vec<String> = v["layouts"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap().to_string()).collect();
    assert!(layouts.len() >= 8);
    for id in &layouts {
        let mut ed = three_steps();
        let out = ed.apply(cmd(json!({"op":"add_layout","layout": id, "x": 10000, "y": 0, "width": 2000})), H).unwrap();
        assert!(ed.doc.path.len() > 3, "{id} adds steps");
        let b = ed.bounds_of(&out.created);
        assert!(b.width() <= 2000.0 * 1.02 && (b.center().x - 10000.0).abs() < 100.0, "{id}: {b:?}");
    }
    // Typing into a placeholder makes it content.
    let mut ed = Editor::default();
    let out = ed.apply(cmd(json!({"op":"add_layout","layout":"title","add_to_path":false})), H).unwrap();
    assert!(ed.doc.path.is_empty());
    let ph = out.created.iter().find(|id| ed.doc.get(id).unwrap().placeholder).unwrap().clone();
    ed.apply(cmd(json!({"op":"update","id": ph, "patch": {"text": "Quarterly review"}})), H).unwrap();
    assert!(!ed.doc.get(&ph).unwrap().placeholder);
    assert!(ed.apply(cmd(json!({"op":"add_layout","layout":"nope"})), H).unwrap_err().valid_values.contains(&"grid".to_string()));
}

#[test]
fn perf_query_reports_document_numbers() {
    let mut ed = three_steps();
    for i in 0..10 {
        ed.perf.record_frame(i as f64 * 0.5, 2.0 + i as f32);
    }
    ed.view.now = 4.6;
    let v = ed.query(Query::Perf).unwrap();
    assert_eq!(v["document"]["elements"], 4);
    assert_eq!(v["frames_sampled"], 10);
    assert!(v["frame_ms_p95"].as_f64().unwrap() >= 10.0);
    assert!(v.get("heap_mb").is_some());
}

#[test]
fn old_files_without_new_fields_still_load() {
    let json = r#"{"format_version":1,"title":"x","theme":"classic","elements":{},"z_order":[],"path":[{"target":{"kind":"overview"}}],"next_id":1}"#;
    let d = io::from_json(json).unwrap();
    assert!(d.themes.is_empty() && d.play == PlaySettings::default() && d.path[0].dwell.is_none());
    // And new defaults aren't written out.
    let s = io::to_json(&Document::default());
    assert!(!s.contains("\"play\"") && !s.contains("\"themes\""));
}

fn texts(ed: &Editor) -> Vec<String> {
    let mut v: Vec<String> = ed.doc.elements.values().filter_map(|e| if let ElementKind::Text(t) = &e.kind { Some(t.text.clone()) } else { None }).collect();
    v.sort();
    v
}

fn frame_names(ed: &Editor) -> Vec<String> {
    let mut v: Vec<String> = ed.doc.elements.values().filter(|e| e.is_frame() && !e.name.is_empty()).map(|e| e.name.clone()).collect();
    v.sort();
    v
}

/// Built-in content carries keys; the editor fills in the words when it is
/// placed: the labeler's (the app's translation), else the English.
#[test]
fn placed_content_gets_words_not_keys() {
    let mut ed = Editor::default();
    ed.apply(cmd(json!({"op":"apply_template","template":"pitch"})), H).unwrap();
    assert!(texts(&ed).contains(&"Add a title".to_string()), "{:?}", texts(&ed));
    assert!(frame_names(&ed).contains(&"Problem".to_string()), "{:?}", frame_names(&ed));
    ed.apply(cmd(json!({"op":"add_diagram","diagram":"numbered_list","x":90000,"y":0})), H).unwrap();
    ed.apply(cmd(json!({"op":"add_layout","layout":"comparison","x":-90000,"y":0})), H).unwrap();
    let all = texts(&ed);
    assert!(all.contains(&"3".to_string()), "numbers stay numbers");
    assert!(all.iter().chain(frame_names(&ed).iter()).all(|t| !t.starts_with("content-")), "{all:?}");

    fn german(p: Phrase) -> Option<String> {
        match p {
            Phrase::Content("content-add-a-title") => Some("Titel hinzufügen".into()),
            Phrase::Content("content-frame-before") => Some("Vorher".into()),
            _ => None,
        }
    }
    let mut ed = Editor::default();
    ed.labeler = Some(german);
    ed.apply(cmd(json!({"op":"add_layout","layout":"comparison"})), H).unwrap();
    assert!(texts(&ed).contains(&"Titel hinzufügen".to_string()), "{:?}", texts(&ed));
    assert!(texts(&ed).contains(&"After".to_string()), "no translation: English");
    assert!(frame_names(&ed).contains(&"Vorher".to_string()), "{:?}", frame_names(&ed));
}
