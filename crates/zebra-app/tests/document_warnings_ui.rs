//! Keynote's Document Warnings: what opening or importing couldn't bring
//! in is listed in a window that opens by itself after the open, can be
//! opened again from View › Show Document Warnings, and can be copied.
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::import::ImportNote;
use zebra_core::warnings::{MediaWord, Warning};

fn harness() -> Harness<'static, ZebraApp> {
    let dir = std::env::temp_dir().join(format!("zebra-warnings-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    // SAFETY: set before any window of this binary starts.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h
}

fn warnings() -> Vec<Warning> {
    vec![
        Warning::AssetNotRead { key: "abc".into(), kind: MediaWord::Video, disk_full: true },
        Warning::Import { file: "Deck.pptx".into(), note: ImportNote::PageSkipped, count: 2, slides: vec![3, 4] },
    ]
}

#[test]
fn warnings_from_an_open_are_listed_and_can_be_copied() {
    let mut h = harness();
    assert!(!zebra_app::menus::state(h.state(), &Action::ShowDocumentWarnings).enabled, "none yet");
    // A document opened with two problems.
    let mut doc = zebra_core::Document::default();
    doc.warnings.extend(warnings());
    let app = h.state_mut();
    let prepared = zebra_core::Editor::prepare(doc, &*app.editor.measure, &|_| {});
    let path = std::env::temp_dir().join(format!("zebra-warnings-{}/Shown.zebra", std::process::id()));
    app.finish_open(&path, Ok(prepared));
    h.run_steps(3);
    assert!(h.state().show_warnings, "the window opens by itself");
    let lines = [
        "A video couldn't be unpacked because the disk is full. It shows as missing.",
        "Deck.pptx: Slides 3\u{2013}4 couldn't be read and were left out.",
    ];
    for l in lines {
        assert!(h.query_by_label(l).is_some(), "{l} shown");
    }
    assert_eq!(zebra_app::warnings_ui::text(h.state()), lines.join("\n"), "what Copy copies");
    // Closed, then opened again from the View menu.
    h.state_mut().show_warnings = false;
    h.run_steps(2);
    assert!(h.query_by_label(lines[0]).is_none());
    assert!(zebra_app::menus::state(h.state(), &Action::ShowDocumentWarnings).enabled);
    h.state_mut().queued.push(Action::ShowDocumentWarnings);
    h.run_steps(3);
    assert!(h.query_by_label(lines[0]).is_some());
    // The Copy button is there (what it copies is checked above).
    assert!(h.query_by_label("Copy").is_some());
}

#[test]
fn an_import_keeps_its_notes_as_warnings() {
    let mut h = harness();
    let mut deck = zebra_core::import::ImportedDeck::default();
    deck.notes.insert(ImportNote::Chart, 1);
    h.state_mut().editor.doc.warn_import("Q3.pptx", &deck);
    assert_eq!(zebra_app::warnings_ui::text(h.state()), "Q3.pptx: 1 chart was left out.");
    assert_eq!(h.state().editor.doc.warnings[0].english(), "Q3.pptx: 1 chart left out");
}
