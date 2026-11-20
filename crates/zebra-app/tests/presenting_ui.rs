//! Stream 4 UI tests: speaker notes, playback options, presenter view,
//! template chooser, layout gallery and the theme editor.
use egui::{Event, Key};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, Mode, Origin};

fn harness(demo: Option<&str>) -> Harness<'static, ZebraApp> {
    let demo = demo.map(|s| s.to_string());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(move |cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo, ..Default::default() }));
    h.run_steps(3);
    h
}

fn perform(h: &mut Harness<'static, ZebraApp>, a: Action) {
    h.state_mut().queued.push(a);
    // Windows settle their size over a few frames.
    h.run_steps(8);
}

#[test]
fn typing_notes_under_the_sidebar_saves_them_as_one_step() {
    let mut h = harness(Some("sample"));
    h.state_mut().selected_step = Some(2);
    h.run_steps(2);
    let before = h.state().editor.history().len();
    h.get_by_role(egui::accesskit::Role::MultilineTextInput).click();
    h.run_steps(2);
    h.event(Event::Text("Ask who has seen".into()));
    h.run_steps(1);
    h.event(Event::Text(" a zoom talk".into()));
    h.run_steps(2);
    // Not saved per keystroke...
    assert_eq!(h.state().editor.history().len(), before);
    // ...but when the field loses focus.
    h.key_press(Key::Escape);
    h.run_steps(3);
    let app = h.state();
    assert_eq!(app.editor.doc.path[2].notes, "Ask who has seen a zoom talk");
    assert_eq!(app.editor.history().len(), before + 1);
}

#[test]
fn playback_options_set_autoplay_and_loop() {
    let mut h = harness(Some("sample"));
    perform(&mut h, Action::PlaybackOptions);
    h.get_by_label("10 s").click();
    h.run_steps(2);
    h.get_by_label("Start again after the last step").click();
    h.run_steps(2);
    let play = h.state().editor.doc.play;
    assert_eq!(play.autoplay, Some(10.0));
    assert!(play.looping);
    h.get_by_label("Off").click();
    h.run_steps(2);
    assert_eq!(h.state().editor.doc.play.autoplay, None);
    // Presenter view is an app setting, not part of the document.
    h.get_by_label("Show presenter view").click();
    h.run_steps(2);
    assert!(h.state().stage.presenter_view);
}

#[test]
fn presenter_view_shows_notes_and_drives_the_show() {
    let mut h = harness(Some("sample,notes,presenter"));
    h.run_steps(3);
    assert_eq!(h.state().editor.view.mode, Mode::Present);
    // Embedded in tests: the presenter view is a window inside the main one.
    h.get_by_label_contains("Open with the question");
    h.get_by_label_contains("Step 2 of 6");
    h.get_by_label("Next").click();
    h.run_steps(2);
    assert_eq!(h.state().editor.view.present.build, 1, "Next reveals the first fade-in");
    h.get_by_label("Exit").click();
    h.run_steps(3);
    assert_eq!(h.state().editor.view.mode, Mode::Edit);
}

#[test]
fn template_chooser_creates_a_presentation() {
    let mut h = harness(Some("empty"));
    perform(&mut h, Action::ShowTemplates);
    assert!(h.state().welcome.is_some());
    h.get_by_label("Pitch").click();
    h.run_steps(2);
    h.get_by_label("Create").click();
    h.run_steps(4);
    let app = h.state();
    assert!(app.welcome.is_none());
    assert_eq!(app.editor.doc.title, "Pitch");
    assert_eq!(app.editor.doc.path.len(), 7);
    assert!(!app.editor.dirty(), "a fresh template isn't an unsaved change");
}

#[test]
fn layout_gallery_inserts_frames_on_the_path() {
    let mut h = harness(Some("sample"));
    let steps = h.state().editor.doc.path.len();
    perform(&mut h, Action::InsertLayout);
    h.get_by_label("Grid of Four").click();
    h.run_steps(3);
    let app = h.state();
    assert!(!app.show_layouts, "the gallery closes after a pick");
    assert_eq!(app.editor.doc.path.len(), steps + 4);
    assert_eq!(app.selection().len(), 1, "the new layout's first frame is selected");
    assert_eq!(app.editor.undo_label().as_deref(), Some("Insert Layout: Grid of Four (You)"), "the undo name is the layout's name, not its id");
}

#[test]
fn theme_editor_makes_a_custom_theme_live() {
    let mut h = harness(Some("sample"));
    perform(&mut h, Action::ThemeEditor);
    let draft = h.state().theme_ed.as_ref().map(|t| t.draft.id.clone()).unwrap();
    assert_eq!(draft, "custom-1");
    // Pick a different base theme from the menu: applies live.
    h.get_by_label("Choose a theme…").click();
    h.run_steps(2);
    h.get_by_label("Chalkboard").click();
    h.run_steps(3);
    {
        let app = h.state();
        assert_eq!(app.editor.doc.theme, "custom-1");
        assert_eq!(app.editor.theme().background, zebra_core::Theme::resolve("chalkboard").background);
    }
    // One undo step once the burst of edits settles.
    std::thread::sleep(std::time::Duration::from_millis(900));
    h.run_steps(3);
    assert!(!h.state().editor.in_transaction());
    h.get_by_label("Done").click();
    h.run_steps(2);
    assert!(h.state().theme_ed.is_none());
    h.state_mut().editor.undo(Origin::Human).unwrap();
    assert_eq!(h.state().editor.doc.theme, "classic");
}

#[test]
fn placeholders_turn_into_content_when_typed_into() {
    let mut h = harness(Some("empty"));
    let out = h.state_mut().editor.apply(Command::AddLayout { layout: "title".into(), x: 0.0, y: 0.0, width: Some(1600.0), add_to_path: true }, Origin::Human).unwrap();
    let ph = out.created.iter().find(|id| h.state().editor.doc.get(id).unwrap().placeholder).cloned().unwrap();
    h.state_mut().editor.apply(Command::Update { id: ph.clone(), patch: zebra_core::ElementPatch { text: Some("Hello".into()), ..Default::default() } }, Origin::Human).unwrap();
    assert!(!h.state().editor.doc.get(&ph).unwrap().placeholder);
}


#[test]
fn typing_a_step_number_while_presenting_jumps_there() {
    let mut h = harness(Some("sample"));
    h.key_press(Key::F5);
    h.run_steps(2);
    assert_eq!(h.state().editor.view.present.step, Some(0));
    let n = h.state().editor.doc.path.len();
    assert!(n >= 4);
    // Like Keynote: type the number, then Return.
    h.event(Event::Text("4".into()));
    h.run_steps(1);
    assert_eq!(h.state().stage.jump.as_deref(), Some("4"));
    assert_eq!(h.state().editor.view.present.step, Some(0), "nothing moves until Return");
    if let Some(dir) = std::env::var_os("ZEBRA_SHOTS") {
        h.run_steps(30);
        h.render().unwrap().save(std::path::Path::new(&dir).join("present-go-to.png")).unwrap();
    }
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().editor.view.present.step, Some(3));
    assert!(h.state().stage.jump.is_none());
    // Esc closes the field without leaving the show; a number past the end does nothing.
    h.event(Event::Text("9".into()));
    h.event(Event::Text("9".into()));
    h.run_steps(1);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().editor.view.mode, Mode::Present);
    assert!(h.state().stage.jump.is_none());
    h.event(Event::Text(format!("{}", n + 1)));
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().editor.view.present.step, Some(3));
    // Then Return steps on as usual.
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().editor.view.present.step.map(|s| s >= 3), Some(true));
}

/// W6 of the 20 Nov audit: a digit typed by mistake opened the field, and
/// then a clicker's Page Down or the arrows did nothing. Now they close the
/// field and step as usual; Backspace to empty closes it too.
#[test]
fn clicker_keys_close_the_go_to_field_and_step() {
    let mut h = harness(Some("sample"));
    h.key_press(Key::F5);
    h.run_steps(2);
    assert_eq!(h.state().editor.view.present.step, Some(0));
    h.event(Event::Text("2".into()));
    h.run_steps(1);
    assert_eq!(h.state().stage.jump.as_deref(), Some("2"));
    h.key_press(Key::PageDown);
    h.run_steps(2);
    assert!(h.state().stage.jump.is_none(), "Page Down closes the field");
    assert_eq!(h.state().editor.view.present.step, Some(1), "and steps on");
    h.event(Event::Text("3".into()));
    h.run_steps(1);
    h.key_press(Key::ArrowLeft);
    h.run_steps(2);
    assert!(h.state().stage.jump.is_none());
    assert_eq!(h.state().editor.view.present.step, Some(0), "the left arrow steps back");
    // Backspace deletes the digit and, with the field empty, closes it,
    // without stepping back.
    h.event(Event::Text("3".into()));
    h.run_steps(1);
    h.key_press(Key::Backspace);
    h.run_steps(2);
    assert!(h.state().stage.jump.is_none(), "Backspace to empty closes the field");
    assert_eq!(h.state().editor.view.present.step, Some(0));
    assert_eq!(h.state().editor.view.mode, Mode::Present);
}

/// Every text drawn this frame.
fn drawn_texts(h: &Harness<'static, ZebraApp>) -> Vec<String> {
    fn walk(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    for c in &h.output().shapes {
        walk(&c.shape, &mut out);
    }
    out
}

/// Cloud pass: an agent's cursor and name showed on the screen the
/// audience sees. It shows while editing, never while presenting.
#[test]
fn the_agent_marker_is_not_shown_to_the_audience() {
    let mut h = harness(Some("sample"));
    let name = h.state().agent_name();
    let put_cursor = |h: &mut Harness<'static, ZebraApp>| {
        let a = h.state_mut();
        a.editor.agent_cursor = Some(a.editor.view.camera.center());
        a.editor.agent_cursor_time = a.editor.view.now;
    };
    put_cursor(&mut h);
    h.run_steps(2);
    assert!(drawn_texts(&h).iter().any(|t| t.contains(&name)), "shown in the editor");
    perform(&mut h, Action::PresentStart);
    assert_eq!(h.state().editor.view.mode, Mode::Present);
    put_cursor(&mut h);
    h.run_steps(2);
    assert!(!drawn_texts(&h).iter().any(|t| t.contains(&name)), "not on the stage");
}
