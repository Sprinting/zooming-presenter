//! A killed app leaves its discovery file. A later launch given the same
//! --api-socket path must not lose its socket when zebractl sweeps the old
//! file. Own binary: sets ZEBRA_DATA_DIR.
use std::os::unix::net::UnixListener;
use zebra_api::discovery::{self, Instance};

#[test]
fn sweeping_a_dead_apps_file_keeps_the_live_socket_on_the_same_path() {
    let dir = std::env::temp_dir().join(format!("zebra-stale-disc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // SAFETY: set before any thread reads the environment.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &dir) };
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("z.sock");
    let _live = UnixListener::bind(&sock).unwrap();
    let inst = |pid: u32, token: &str, started_at: u64| Instance {
        pid,
        socket: sock.clone(),
        token: token.into(),
        version: "0.1.0".into(),
        api_version: 1,
        started_at,
        file: Default::default(),
    };
    discovery::write(&inst(2_147_483_000, "dead", 100), "2147483000.json").unwrap();
    discovery::write(&inst(std::process::id(), "live", 200), "live.json").unwrap();
    for _ in 0..3 {
        let all = discovery::list();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].token, "live");
        assert!(sock.exists(), "the live app's socket is still there");
        assert_eq!(discovery::find(None, Some(&sock)).unwrap().token, "live");
    }
    assert!(!discovery::instances_dir().join("2147483000.json").exists(), "the dead app's file is swept");
    // Nothing listening: a dead app's socket is swept with its file.
    drop(_live);
    discovery::write(&inst(2_147_483_000, "dead", 100), "2147483000.json").unwrap();
    std::fs::remove_file(discovery::instances_dir().join("live.json")).unwrap();
    assert!(discovery::list().is_empty());
    assert!(!sock.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
