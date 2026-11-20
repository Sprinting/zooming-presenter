//! AI control: the first time is always a person. A launch flag (`--api`)
//! does nothing until someone has switched AI control on in the window.
//! Its own test binary, because it sets ZEBRA_DATA_DIR for the process.
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp};

fn harness(api: bool, dir: &std::path::Path) -> Harness<'static, ZebraApp> {
    harness_with(api.then_some(zebra_api::Permission::Full), dir)
}

/// `perm`: `--api` with the most `--agent-permission` allows (Full when
/// the flag isn't given).
fn harness_with(perm: Option<zebra_api::Permission>, dir: &std::path::Path) -> Harness<'static, ZebraApp> {
    let api = perm.map(|permission| zebra_api::ApiOptions { socket: Some(dir.join(format!("z{}.sock", rand_suffix()))), permission });
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
        h.get_by_role_and_label(switch(), "Allow AI control").click();
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
        h.get_by_role_and_label(switch(), "Allow AI control").click();
        h.run_steps(3);
        assert!(!h.state().ai_enabled && !h.state().ai_saved && h.state().api.is_none());
        assert_eq!(zebra_api::discovery::ai_switch(), zebra_api::discovery::AiSwitch { granted: true, on: Some(false) });
    }
    {
        let h = harness(false, &dir);
        assert!(!h.state().ai_enabled, "switched off stays off");
    }

    // 2c. Switched off (by the person, or an agent's `disconnect`), a
    // relaunch with `--api` must not switch it back on.
    {
        let h = harness(true, &dir);
        assert!(h.state().api.is_none() && !h.state().ai_enabled, "--api must not undo a switch-off");
        assert!(zebra_api::discovery::list().is_empty());
    }
    // An agent's `disconnect` records the same off as the person's click.
    zebra_api::discovery::record_ai_switch(true).unwrap();
    zebra_api::discovery::record_ai_switch(false).unwrap();
    {
        let h = harness(true, &dir);
        assert!(!h.state().ai_enabled, "--api after disconnect stays off");
    }

    // 3. Switched on again by the person, the flag works.
    zebra_api::discovery::record_ai_switch(true).unwrap();
    {
        let h = harness(true, &dir);
        assert!(h.state().api.is_some() && h.state().ai_enabled, "--api works once a person has switched AI control on");
    }
    // `--agent-permission full` can't raise it past the person's choice
    // (Edit: this person never chose), but `observe` lowers it.
    {
        let h = harness_with(Some(zebra_api::Permission::Full), &dir);
        assert!(h.state().ai_enabled);
        assert_eq!(h.state().editor.view.agent_permission, zebra_api::Permission::Edit, "a flag must not raise permission");
    }
    {
        let h = harness_with(Some(zebra_api::Permission::Observe), &dir);
        assert_eq!(h.state().editor.view.agent_permission, zebra_api::Permission::Observe, "a flag may lower permission");
    }
    // An older grant that didn't keep the choice still lets the flag work.
    std::fs::write(zebra_api::discovery::human_grant_file(), b"").unwrap();
    assert_eq!(zebra_api::discovery::ai_switch().on, None);
    {
        let h = harness(true, &dir);
        assert!(h.state().ai_enabled, "an older grant keeps today's behaviour");
        assert!(!h.state().ai_saved, "a flag launch must not be saved as the person's choice");
    }
    assert_eq!(zebra_api::discovery::ai_switch().on, None, "nor kept with the grant");
    let _ = std::fs::remove_dir_all(dir);
}
