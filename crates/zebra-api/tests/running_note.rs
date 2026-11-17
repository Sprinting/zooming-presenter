//! With AI control off nothing listens, but zebractl can still tell "AI
//! control is off" from "Zebra isn't open". Own binary: sets ZEBRA_DATA_DIR.
use zebra_api::discovery;

#[test]
fn an_open_zebra_with_ai_off_says_so() {
    let dir = std::env::temp_dir().join(format!("zebra-running-note-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // SAFETY: set before any thread reads the environment.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };
    assert!(discovery::not_accepting_message().contains("isn't open"));
    // A note left by a crashed app (dead pid) is swept, not believed.
    std::fs::create_dir_all(discovery::instances_dir()).unwrap();
    std::fs::write(discovery::instances_dir().join("999999999.running"), "{}").unwrap();
    assert!(discovery::running_apps().is_empty());
    let note = discovery::RunningNote::write().unwrap();
    let m = discovery::not_accepting_message();
    assert!(m.contains("AI control is switched off") && m.contains(&std::process::id().to_string()), "{m}");
    assert!(discovery::list().is_empty(), "the note is not an instance: nothing to connect to");
    drop(note);
    assert!(discovery::not_accepting_message().contains("isn't open"));
    let _ = std::fs::remove_dir_all(&dir);
}
