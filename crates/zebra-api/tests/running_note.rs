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

    // M1 (Mac pass 21 Nov): a killed Zebra's note whose pid came back as
    // another app (Notes) was believed. Here the pid is this test's, alive,
    // but the note was written an hour before this process started: it
    // can't be this process's, so it is swept.
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let stale = discovery::instances_dir().join(format!("{}.running", std::process::id()));
    std::fs::write(&stale, "{}").unwrap();
    std::fs::File::options().write(true).open(&stale).unwrap().set_modified(old).unwrap();
    assert!(discovery::running_apps().is_empty(), "a reused pid isn't a Zebra");
    assert!(!stale.exists(), "and its note is swept");
    assert!(discovery::not_accepting_message().contains("isn't open"));
    // Written now by this process: believed.
    let note = discovery::RunningNote::write().unwrap();
    assert_eq!(discovery::running_apps(), vec![std::process::id()]);
    drop(note);
    // The process start time is known here (Linux /proc, macOS proc_pidinfo).
    let started = discovery::process_started(std::process::id()).expect("own start time");
    let age = std::time::SystemTime::now().duration_since(started).unwrap();
    assert!(age < std::time::Duration::from_secs(600), "started {age:?} ago");
    assert_eq!(discovery::process_started(0), None);

    // M19 (Mac pass 21 Nov): a 6 GB spill file stayed in Autosave/Media.
    // The launch sweep kept any file whose pid was running, even when the
    // pid had come back as another process. What made a file must have
    // started before it was written. (The file rules: zebra-core's
    // tests/spill.rs.)
    let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
    let now = std::time::SystemTime::now();
    assert!(discovery::made_by_live(child.id(), Some(now)), "written after it started: its own");
    assert!(!discovery::made_by_live(child.id(), Some(now - std::time::Duration::from_secs(3600))), "written before it started: a reused pid");
    assert!(discovery::made_by_live(child.id(), None), "time unknown: running is enough");
    let media = dir.join("Media");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join(format!("{}-0-abc.png", child.id())), b"live").unwrap();
    std::fs::write(media.join("999999999-0-abc.png"), b"gone").unwrap();
    let c = zebra_core::media::collect_spill(&media, discovery::made_by_live);
    assert_eq!(c.files, 1);
    assert!(media.join(format!("{}-0-abc.png", child.id())).exists());
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
