//! AI control: the first time is always a person. A launch flag (`--api`)
//! does nothing until someone has switched AI control on in the window.
//! Its own test binary, because it sets ZEBRA_DATA_DIR for the process.
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp};

fn harness(api: bool, dir: &std::path::Path) -> Harness<'static, ZebraApp> {
    let api = api.then(|| zebra_api::ApiOptions { socket: Some(dir.join(format!("z{}.sock", rand_suffix()))), permission: zebra_api::Permission::Edit });
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(move |cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { api, demo: Some("empty".into()), ..Default::default() }));
    h.run_steps(3);
    h
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() % 1_000_000
}

#[test]
fn launch_flag_needs_a_person_to_switch_ai_control_on_first() {
    let dir = std::env::temp_dir().join(format!("zebra-ai-grant-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: the only test in this binary.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };
    assert!(!zebra_api::discovery::human_has_granted());
    // An agent switching it off before any grant doesn't make one.
    zebra_api::discovery::record_ai_switch(false).unwrap();
    assert!(!zebra_api::discovery::human_has_granted());

    // 1. `--api` on a fresh install: AI control stays off.
    {
        let h = harness(true, &dir);
        assert!(h.state().api.is_none() && !h.state().ai_enabled, "--api must not switch AI control on before a person has");
        assert!(zebra_api::discovery::list().is_empty());
    }

    // 2. The person switches it on in the window, then off again.
    {
        let mut h = harness(false, &dir);
        h.get_by_label("AI").click();
        h.run_steps(3);
        let switch = || egui::accesskit::Role::Switch;
        h.get_by_role_and_label(switch(), "Let AI agents control Zebra").click();
        h.run_steps(3);
        assert!(h.state().ai_enabled && h.state().ai_saved && h.state().api.is_some());
        assert!(zebra_api::discovery::human_has_granted(), "the person's click records the grant");
        assert_eq!(zebra_api::discovery::ai_switch().on, Some(true));
    }

    // 2b. The kept switch (not the app's settings file) brings it back on.
    {
        let mut h = harness(false, &dir);
        assert!(h.state().ai_enabled && h.state().ai_saved, "the person's choice is kept with the grant");
        let switch = || egui::accesskit::Role::Switch;
        h.get_by_label("AI").click();
        h.run_steps(3);
        h.get_by_role_and_label(switch(), "Let AI agents control Zebra").click();
        h.run_steps(3);
        assert!(!h.state().ai_enabled && !h.state().ai_saved && h.state().api.is_none());
        assert_eq!(zebra_api::discovery::ai_switch(), zebra_api::discovery::AiSwitch { granted: true, on: Some(false) });
    }
    {
        let h = harness(false, &dir);
        assert!(!h.state().ai_enabled, "switched off stays off");
    }

    // 3. From then on the flag works, and it is not saved as the person's choice.
    {
        let h = harness(true, &dir);
        assert!(h.state().api.is_some() && h.state().ai_enabled, "--api works once a person has switched AI control on");
        assert!(!h.state().ai_saved, "a flag launch must not be saved as the person's choice");
    }
    let _ = std::fs::remove_dir_all(dir);
}
