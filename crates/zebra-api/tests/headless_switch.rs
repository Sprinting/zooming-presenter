//! `zebra --headless --api` only runs while the person's AI control switch
//! is on. Its own test binary, because it sets ZEBRA_DATA_DIR.
use zebra_api::ApiOptions;

#[test]
fn headless_is_refused_while_ai_control_is_off() {
    let dir = std::env::temp_dir().join(format!("zebra-headless-switch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: the only test in this binary.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };

    // Never switched on by a person.
    let e = zebra_api::run_headless(ApiOptions::default(), None).unwrap_err();
    assert!(e.to_string().contains("switches it on"), "{e}");

    // Switched on, then off (the person, or an agent's `disconnect`).
    zebra_api::discovery::record_ai_switch(true).unwrap();
    zebra_api::discovery::record_ai_switch(false).unwrap();
    let e = zebra_api::run_headless(ApiOptions::default(), None).unwrap_err();
    assert!(e.to_string().contains("switched off"), "{e}");
    assert!(zebra_api::discovery::list().is_empty(), "nothing listens");
    let _ = std::fs::remove_dir_all(dir);
}
