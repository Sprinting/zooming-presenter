//! Several document windows, like Keynote: New and Open get a window of
//! their own unless the front one is an untouched new document; shared
//! settings follow from window to window; closing the first window moves
//! the next document into it.
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::shell::Shell;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, Origin};

fn harness() -> Harness<'static, Shell> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| Shell::new(ZebraApp::new(&cc.egui_ctx, None, None, AppOptions::default())));
    h.run_steps(3);
    h
}

fn add_frame(w: &mut ZebraApp) {
    w.editor.apply(Command::AddFrame { style: Default::default(), x: 0.0, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: None, add_to_path: false, find_space: false }, Origin::Human).unwrap();
}

#[test]
fn new_reuses_an_untouched_window_and_otherwise_opens_one() {
    let mut h = harness();
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(3);
    assert_eq!(h.state().windows.len(), 1, "an untouched new document is replaced");

    add_frame(&mut h.state_mut().windows[0]);
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(4);
    let s = h.state();
    assert_eq!(s.windows.len(), 2);
    assert_eq!((s.windows[0].doc_id.as_str(), s.windows[1].doc_id.as_str()), ("d1", "d2"));
    assert_eq!(s.front, 1, "the new window is in front");
    assert!(s.windows[1].front && !s.windows[0].front);
    assert_eq!(s.windows[0].editor.doc.elements.len(), 1, "the first document is kept");
    assert!(s.windows[1].editor.doc.elements.is_empty());
}

#[test]
fn shared_settings_follow_from_window_to_window() {
    let mut h = harness();
    add_frame(&mut h.state_mut().windows[0]);
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(4);
    assert!(h.state().windows[0].snapping);
    h.state_mut().windows[1].snapping = false;
    h.state_mut().windows[1].default_theme = "night".into();
    h.run_steps(2);
    assert!(!h.state().windows[0].snapping);
    assert_eq!(h.state().windows[0].default_theme, "night");
}

#[test]
fn closing_windows_keeps_the_others() {
    let mut h = harness();
    add_frame(&mut h.state_mut().windows[0]);
    h.state_mut().windows[0].editor.mark_saved(None);
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(4);
    add_frame(&mut h.state_mut().windows[1]);
    h.state_mut().windows[1].editor.mark_saved(None);
    h.state_mut().windows[1].queued.push(Action::New);
    h.run_steps(4);
    assert_eq!(h.state().windows.len(), 3);

    // Close the middle one.
    h.state_mut().windows[1].queued.push(Action::Close);
    h.run_steps(3);
    let ids: Vec<String> = h.state().windows.iter().map(|w| w.doc_id.clone()).collect();
    assert_eq!(ids, ["d1", "d3"]);

    // Close the first: the next document moves into the first window.
    h.state_mut().windows[0].queued.push(Action::Close);
    h.run_steps(3);
    let s = h.state();
    assert_eq!(s.windows.len(), 1);
    assert_eq!(s.windows[0].doc_id, "d3");
    assert_eq!(s.front, 0);
    assert!(s.windows[0].front);
}

#[test]
fn a_window_with_unsaved_changes_asks_before_closing() {
    let mut h = harness();
    add_frame(&mut h.state_mut().windows[0]);
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(4);
    add_frame(&mut h.state_mut().windows[1]);
    h.state_mut().windows[1].queued.push(Action::Close);
    h.run_steps(3);
    assert_eq!(h.state().windows.len(), 2, "still open");
    assert!(h.state().windows[1].sheet_showing(), "Save Changes is asked");
}

#[test]
fn closing_the_last_window_keeps_zebra_running() {
    let mut h = harness();
    add_frame(&mut h.state_mut().windows[0]);
    h.state_mut().windows[0].editor.mark_saved(None);
    h.state_mut().windows[0].queued.push(Action::Close);
    h.run_steps(3);
    let s = h.state();
    assert!(s.empty, "like Keynote: no window, still running");
    assert_eq!(s.windows.len(), 1);
    assert!(s.windows[0].is_pristine(), "a new document waits in the hidden window");
    assert!(!s.windows[0].force_quit);
}

#[test]
fn quit_asks_one_unsaved_window_at_a_time_and_cancel_stops_it() {
    let mut h = harness();
    add_frame(&mut h.state_mut().windows[0]);
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(4);
    add_frame(&mut h.state_mut().windows[1]);
    h.state_mut().set_front(0);
    h.state_mut().windows[0].queued.push(Action::Quit);
    h.run_steps(4);
    let s = h.state();
    assert_eq!(s.windows.len(), 2);
    assert!(s.windows[1].sheet_showing(), "the last window asks first");
    assert!(!s.windows[0].sheet_showing(), "one sheet at a time");
    assert_eq!(s.front, 1, "the window asking is brought to the front");

    // Cancel: the quit stops and both documents stay.
    let k = h.state_mut().windows[1].sheet.take().unwrap();
    zebra_app::sheets::resolve(&mut h.state_mut().windows[1], k, &zebra_app::t!("sheet-cancel"));
    h.run_steps(4);
    let s = h.state();
    assert_eq!(s.windows.len(), 2);
    assert!(!s.windows.iter().any(|w| w.sheet_showing() || w.quit_pending));

    // Quit again; Don't Save on the last window moves on to the first.
    h.state_mut().windows[0].queued.push(Action::Quit);
    h.run_steps(4);
    let k = h.state_mut().windows[1].sheet.take().unwrap();
    zebra_app::sheets::resolve(&mut h.state_mut().windows[1], k, &zebra_app::t!("sheet-dont-save"));
    h.run_steps(6);
    let s = h.state();
    assert_eq!(s.windows.len(), 1);
    assert!(s.windows[0].sheet_showing(), "then the first window asks");
}

#[test]
fn an_agent_opens_a_document_behind_and_closes_it() {
    use zebra_api::Documents;
    let mut h = harness();
    add_frame(&mut h.state_mut().windows[0]);
    let ctx = h.ctx.clone();
    let id = h.state_mut().agent_documents(&ctx).new_document().unwrap();
    h.run_steps(3);
    let s = h.state();
    assert_eq!(s.windows.len(), 2);
    assert_eq!(s.front, 0, "the person's window stays in front");
    assert!(s.windows[1].opened_by_agent && s.windows[1].doc_id == id);

    h.state_mut().agent_documents(&ctx).close(&id, false).unwrap();
    assert_eq!(h.state_mut().agent_documents(&ctx).list().len(), 1, "gone from the list at once");
    h.run_steps(3);
    assert_eq!(h.state().windows.len(), 1);

    // The last one closed: no documents, and a new one shows the window again.
    h.state_mut().windows[0].editor.mark_saved(None);
    h.state_mut().agent_documents(&ctx).close("d1", false).unwrap();
    h.run_steps(3);
    assert!(h.state().empty);
    assert!(h.state_mut().agent_documents(&ctx).list().is_empty());
    let again = h.state_mut().agent_documents(&ctx).new_document().unwrap();
    h.run_steps(2);
    assert!(!h.state().empty);
    assert_eq!(h.state_mut().agent_documents(&ctx).list()[0].id, again);
}

/// Mac round, 18 Nov: with no window open, Settings… and About Zebra each
/// brought the hidden document window back as a blank "Untitled". They are
/// windows of their own, so no document window comes with them; anything
/// else asked for from the menu bar still does.
#[test]
fn settings_and_about_with_no_window_open_no_document() {
    let mut h = harness();
    h.state_mut().windows[0].queued.push(Action::Close);
    h.run_steps(3);
    assert!(h.state().empty);

    h.state_mut().windows[0].queued.push(Action::Settings);
    h.run_steps(4);
    assert!(h.state().empty, "Settings alone opens no document window");
    assert!(h.state().windows[0].show_settings);
    h.state_mut().windows[0].show_settings = false;

    h.state_mut().windows[0].queued.push(Action::About);
    h.run_steps(4);
    assert!(h.state().empty, "nor does About");
    assert!(h.state().windows[0].show_about);
    h.state_mut().windows[0].show_about = false;
    h.run_steps(2);

    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(4);
    assert!(!h.state().empty, "File › New shows a window again");
}

/// Mac round, 18 Nov: a window made by File › New came up 800 by 600
/// whatever the size of the window it was made from.
#[test]
fn a_new_window_opens_at_the_size_of_the_one_it_came_from() {
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 30.0), egui::vec2(1440.0, 834.0));
    let mut from = egui::ViewportInfo { inner_rect: Some(rect), ..Default::default() };
    assert_eq!(zebra_app::app::open_size_from(&from), Some(egui::vec2(1440.0, 834.0)));
    from.fullscreen = Some(true);
    assert_eq!(zebra_app::app::open_size_from(&from), None, "full screen: macOS decides");
    assert_eq!(zebra_app::app::open_size_from(&egui::ViewportInfo::default()), None);
}

/// Mac round, 18 Nov: two new documents were both named "Untitled". The
/// second is "Untitled 2", as in Keynote; a typed title is left alone.
#[test]
fn new_documents_are_numbered() {
    let mut h = harness();
    h.state_mut().windows[0].welcome = None;
    h.state_mut().windows[0].editor.doc.title = "Untitled".into();
    add_frame(&mut h.state_mut().windows[0]);
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(4);
    add_frame(&mut h.state_mut().windows[1]);
    h.state_mut().windows[1].queued.push(Action::New);
    h.run_steps(4);
    let titles: Vec<String> = h.state().windows.iter().map(|w| w.window_title().replace(" — Edited", "")).collect();
    assert_eq!(titles, ["Untitled", "Untitled 2", "Untitled 3"]);
    h.state_mut().windows[1].editor.doc.title = "Plan".into();
    assert_eq!(h.state().windows[1].window_title().replace(" — Edited", ""), "Plan");
    assert_eq!(zebra_app::app::untitled_name("Untitled", "Untitled", 0), "Untitled");
}

/// zebra-core leaves a new document untitled; the app names it in the
/// person's language (window title, Save As name, export names).
#[test]
fn a_new_document_is_named_by_the_app() {
    assert_eq!(zebra_core::Document::default().title, "", "core types no English title");
    let mut h = harness();
    h.run_steps(2);
    let untitled = zebra_app::i18n::tr("app-untitled");
    assert_eq!(h.state().windows[0].editor.doc.title, untitled);
    h.state_mut().windows[0].queued.push(Action::New);
    h.run_steps(4);
    assert!(h.state().windows.iter().all(|w| w.editor.doc.title == untitled));
    assert_eq!(zebra_app::app::new_document("classic").title, untitled);
    assert_eq!(zebra_app::app::untitled_name("", &untitled, 0), untitled, "an empty title (an old or agent file) reads as untitled");
}
