//! The zebractl relay (hostile-input audit M6, L5).
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use serde_json::json;
use zebra_api::client::Client;
use zebra_api::{ApiOptions, HeadlessHandle, Permission};

fn start() -> HeadlessHandle {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let dir = std::env::temp_dir().join(format!("zebra-api-relay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once, before any server starts in this test binary.
        unsafe { std::env::set_var("ZEBRA_DATA_DIR", dir.join("data")) };
    });
    HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Edit }, None).expect("headless starts")
}

/// One zebractl process that connects and says nothing stalled every
/// later call for that agent. Now it stalls only itself, and is dropped
/// after the timeout.
#[test]
fn a_silent_client_does_not_block_the_relay() {
    let h = start();
    let relay = zebra_api::relay::socket_for(&h.socket, "silent-test").unwrap();
    let (zs, tok, rs) = (h.socket.clone(), h.token.clone(), relay.clone());
    std::thread::spawn(move || zebra_api::relay::run(&zs, &tok, "silent-test", &rs));
    let t0 = Instant::now();
    while !relay.exists() && t0.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut silent = UnixStream::connect(&relay).unwrap();
    std::thread::sleep(Duration::from_millis(100));

    let t = Instant::now();
    let mut c = Client::connect_hello(&relay, "silent-test", &h.token).unwrap();
    assert_eq!(c.call("ping", json!({})).unwrap(), "pong");
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());

    // The silent one is closed once CLIENT_TIMEOUT has passed.
    silent.set_read_timeout(Some(zebra_api::relay::CLIENT_TIMEOUT * 2)).unwrap();
    let mut b = [0u8; 1];
    assert_eq!(silent.read(&mut b).unwrap_or(1), 0, "dropped with EOF");
    h.stop().unwrap();
}

/// Audit L5: relay sockets were named from a 32-bit hash, so two agent
/// names could share one relay (and act as each other). Now 64 bits, and a
/// relay that answers with another name is not used.
#[test]
fn a_relay_is_used_only_by_its_own_agent() {
    let a = zebra_api::relay::socket_for(std::path::Path::new("/tmp/z/1.sock"), "alice").unwrap();
    let name = a.file_name().unwrap().to_str().unwrap();
    assert_eq!(name.len(), "1.agent".len() + 16 + ".sock".len(), "{name}");
    assert_ne!(Some(a.clone()), zebra_api::relay::socket_for(std::path::Path::new("/tmp/z/1.sock"), "bob"));

    let h = start();
    let relay = zebra_api::relay::socket_for(&h.socket, "alice").unwrap();
    let (zs, tok, rs) = (h.socket.clone(), h.token.clone(), relay.clone());
    std::thread::spawn(move || zebra_api::relay::run(&zs, &tok, "alice", &rs));
    let t0 = Instant::now();
    while !relay.exists() && t0.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut c = zebra_api::relay::connect(&relay, "alice", &h.token).unwrap();
    assert_eq!(c.call("ping", json!({})).unwrap(), "pong");
    // Bob, finding Alice's relay at his socket name, does not use it.
    assert!(zebra_api::relay::connect(&relay, "bob", &h.token).is_err());
    h.stop().unwrap();
}

/// A long `--api-socket` path: the relay's name beside it would not fit a
/// socket address (`sun_path`), and every call waited 2 s for a relay that
/// could never start. Now it goes in a short private folder, or there is
/// none and the caller connects directly.
#[cfg(target_os = "linux")]
#[test]
fn a_relay_socket_always_fits() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::temp_dir().join(format!("zr-{}", std::process::id()));
    let deep = base.join("x".repeat(zebra_api::relay::SUN_PATH - base.as_os_str().len() - 12));
    let sock = deep.join("z.sock");
    assert!(zebra_api::relay::fits_socket(&sock), "the app's own socket fits");
    let private = base.join("run");
    std::fs::create_dir_all(&private).unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
    // SAFETY: no other test in this binary reads XDG_RUNTIME_DIR.
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", &private) };
    let r = zebra_api::relay::socket_for(&sock, "alice").expect("a short relay socket");
    assert!(r.as_os_str().as_bytes().len() < zebra_api::relay::SUN_PATH);
    assert_eq!(r.parent(), Some(private.as_path()));
    assert_ne!(Some(r.clone()), zebra_api::relay::socket_for(&sock, "bob"));
    assert_ne!(Some(r), zebra_api::relay::socket_for(&deep.join("y.sock"), "alice"), "another instance");
    // A folder others can write to is never used: no relay, a direct call.
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(zebra_api::relay::socket_for(&sock, "alice"), None);
    // A short path keeps its relay beside it.
    let short = zebra_api::relay::socket_for(std::path::Path::new("/tmp/z/1.sock"), "alice").unwrap();
    assert_eq!(short.parent(), Some(std::path::Path::new("/tmp/z")));
    let _ = std::fs::remove_dir_all(&base);
}
