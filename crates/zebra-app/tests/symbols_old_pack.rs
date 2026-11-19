//! A clip-art pack in the old format (raw SVGs, format 1) is refused, and
//! the Symbols tab says this Zebra can't read it rather than that nothing
//! is installed. Its own test binary: it points Zebra at that pack.
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::{AppOptions, ZebraApp};

#[test]
fn an_old_symbol_pack_is_named_as_unreadable() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../zebra-core/tests/data/symbols-v1");
    let data = std::env::temp_dir().join(format!("zebra-symbols-old-{}", std::process::id()));
    // SAFETY: the only test in this binary; set before the library is read.
    unsafe {
        std::env::set_var("ZEBRA_SYMBOLS_DIR", &dir);
        std::env::set_var("ZEBRA_DATA_DIR", &data);
    }
    assert!(zebra_core::symbols::library().is_none());
    assert!(zebra_core::symbols::library_error().is_some_and(|e| e.contains("format 1")));

    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h.state_mut().queued.push(Action::InsertSymbols);
    h.run_steps(5);
    h.get_by_label("This version of Zebra can't read the symbol library.");
    if let Some(out) = std::env::var_os("ZEBRA_SHOT") {
        h.render().unwrap().save(out).unwrap();
    }
    let _ = std::fs::remove_dir_all(data);
}
