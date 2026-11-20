//! AI control: the person switches it on, an agent can only switch it off.
//! Its own test binary, because it sets ZEBRA_DATA_DIR for the process (and
//! the switch it keeps would turn AI control on in other tests' windows).
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zebra_app::{AppOptions, ZebraApp};

fn harness(demo: Option<&str>) -> Harness<'static, ZebraApp> {
    let demo = demo.map(|s| s.to_string());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(0.02)
        .build_eframe(move |cc| ZebraApp::new(&cc.egui_ctx, None, None, AppOptions { demo, ..Default::default() }));
    h.run_steps(3);
    h
}

#[test]
fn ai_control_is_switched_on_by_the_human_and_off_by_an_agent() {
    let dir = std::env::temp_dir().join(format!("zebra-ai-toggle-{}", std::process::id()));
    // SAFETY: no other test in this binary reads or writes the environment.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };
    let mut h = harness(Some("empty"));
    assert!(h.state().api.is_none(), "AI control must be off by default");
    assert!(zebra_api::discovery::list().is_empty());

    // Human: click the AI toolbar item, then the switch.
    h.get_by_label("AI").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::Switch, "Let AI agents control Zebra").click();
    h.run_steps(3);
    assert!(h.state().ai_enabled && h.state().api.is_some(), "the switch should turn AI control on");
    let socket = h.state().api.as_ref().unwrap().socket_path();
    assert_eq!(zebra_api::discovery::list().len(), 1, "a running agent must be able to find Zebra");

    // Agent: discover, then switch it off. There is no method to switch it on.
    let t = std::thread::spawn(move || {
        let mut c = zebra_api::client::Client::discover(None, None, None, "test-agent").unwrap();
        assert!(c.call("enable", serde_json::json!({})).is_err());
        c.call("disconnect", serde_json::json!({})).unwrap()
    });
    let start = std::time::Instant::now();
    while h.state().api.is_some() && start.elapsed().as_secs() < 10 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let reply = t.join().unwrap();
    assert_eq!(reply["ok"], true);
    assert!(h.state().api.is_none() && !h.state().ai_enabled, "disconnect should switch AI control off");
    assert!(!socket.exists(), "socket should be removed");
    assert!(zebra_api::discovery::list().is_empty(), "discovery file should be removed");
    let err = zebra_api::client::Client::discover(None, None, None, "test-agent").err().unwrap().to_string();
    assert!(err.contains("AI control") && err.contains("Ask the person"), "{err}");
    let _ = std::fs::remove_dir_all(dir);
}
