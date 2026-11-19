//! Speed fix 3: the Symbols panel gives its memory back when it closes.
//! Its own test binary: it points Zebra at a small clip-art pack.
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::symbols::library;

const BOX: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 16"><rect width="32" height="16" fill="#e33"/></svg>"##;

#[test]
fn closing_the_symbols_panel_frees_its_tiles_and_svgs() {
    let dir = std::env::temp_dir().join(format!("zebra-symbols-memory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut dat, mut idx) = (Vec::new(), String::new());
    for i in 0..12 {
        let z = ruzstd::encoding::compress_to_vec(BOX.as_bytes(), ruzstd::encoding::CompressionLevel::Fastest);
        idx += &format!("box {i}\tsquare\t{}\t{}\n", dat.len(), z.len());
        dat.extend(z);
    }
    std::fs::write(dir.join("emoji.dat"), dat).unwrap();
    std::fs::write(dir.join("emoji.idx"), idx).unwrap();
    std::fs::write(dir.join("sets.json"), serde_json::json!({"format": 2, "sets": [{"id":"C1","key":"emoji","name":"Emoji","group":"emoji","tint":"","author":"A","licence":"MIT","count":12}]}).to_string()).unwrap();
    // SAFETY: the only test in this binary; set before the library is read.
    unsafe {
        std::env::set_var("ZEBRA_SYMBOLS_DIR", &dir);
        std::env::set_var("ZEBRA_DATA_DIR", dir.join("data"));
    }

    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(|cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h.state_mut().queued.push(Action::InsertSymbols);
    h.state_mut().library.query = "box".into();
    let t0 = std::time::Instant::now();
    while (h.state().symbols.tiles() == 0 || library().unwrap().cached() == 0) && t0.elapsed().as_secs() < 10 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(h.state().symbols.tiles() > 0, "the grid drew tiles");
    assert!(library().unwrap().cached() > 0, "and read their SVGs");
    // While it is open, they stay.
    h.run_steps(5);
    assert!(h.state().symbols.tiles() > 0);

    h.state_mut().library.open = false;
    h.run_steps(3);
    assert_eq!(h.state().symbols.tiles(), 0);
    assert_eq!(library().unwrap().cached(), 0);
    assert_eq!(library().unwrap().dictionaries(), 0, "and the set dictionaries");
    let _ = std::fs::remove_dir_all(dir);
}
