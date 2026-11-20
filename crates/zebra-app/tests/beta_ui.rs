//! Beta features in the app: the Settings › General row turns them on and
//! off, and importing PowerPoint while they are off offers to turn them on
//! (Turn On carries on with the import; Cancel leaves everything as it was).
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zebra_app::i18n::tr;
use zebra_app::sheets::SheetKind;
use zebra_app::{import_ui, AppOptions, ZebraApp};
use zebra_core::beta::BetaFeature;

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
        let _ = std::fs::create_dir_all(&dir);
        h.render().unwrap().save(std::path::Path::new(&dir).join(name)).unwrap();
    }
}

fn deck() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../zebra-import/tests/fixtures/deck16x9.pptx")
}

/// Step until the import has read the file and offered (or gone on).
fn until_read(h: &mut Harness<'static, ZebraApp>) {
    for _ in 0..300 {
        h.step();
        if !matches!(h.state().import, Some(import_ui::Flow::Reading { .. })) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run_steps(3);
}

#[test]
fn settings_row_turns_beta_features_on_and_off() {
    let mut h = harness();
    assert!(!h.state().editor.view.beta_features, "off until the person turns it on");
    h.state_mut().show_settings = true;
    h.state_mut().settings_pane = zebra_app::settings::Pane::General;
    h.run_steps(4);
    shot(&mut h, "settings-general-beta.png");
    h.get_by_label(&tr("settings-beta-caption"));
    h.get_by_label(&tr("settings-beta-switch")).click();
    h.run_steps(3);
    assert!(h.state().editor.view.beta_features);
    shot(&mut h, "settings-general-beta-on.png");
    h.get_by_label(&tr("settings-beta-switch")).click();
    h.run_steps(3);
    assert!(!h.state().editor.view.beta_features);
}

#[test]
fn powerpoint_import_offers_beta_features_then_carries_on() {
    let mut h = harness();
    import_ui::start(h.state_mut(), deck());
    until_read(&mut h);
    assert_eq!(h.state().sheet, Some(SheetKind::BetaOffer(BetaFeature::PptxImport)));
    assert!(h.state().import.is_none(), "nothing is imported while the offer is up");
    shot(&mut h, "beta-offer-pptx.png");
    h.get_by_label(&tr("beta-offer-turn-on")).click();
    h.run_steps(3);
    assert!(h.state().editor.view.beta_features);
    until_read(&mut h);
    // The import goes on: the slides to bring in are asked for.
    assert!(matches!(h.state().import, Some(import_ui::Flow::Choose { .. })), "the import carries on");
}

#[test]
fn cancelling_the_offer_imports_nothing() {
    let mut h = harness();
    let before = h.state().editor.revision;
    import_ui::start(h.state_mut(), deck());
    until_read(&mut h);
    assert_eq!(h.state().sheet, Some(SheetKind::BetaOffer(BetaFeature::PptxImport)));
    h.get_by_label(&tr("sheet-cancel")).click();
    h.run_steps(3);
    let app = h.state();
    assert!(!app.editor.view.beta_features);
    assert!(app.import.is_none() && app.sheet.is_none() && app.beta_pending.is_none());
    assert_eq!(app.editor.revision, before);
}
