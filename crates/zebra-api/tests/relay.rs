//! The zebractl relay (hostile-input audit M6, L5).
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use serde_json::json;
use zebra_api::client::Client;
use zebra_api::{ApiOptions, HeadlessHandle, Permission};

fn start() -> HeadlessHandle {
    let dir = std::env::temp_dir().join(format!("zebra-api-relay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: set before the server starts; the only test here sets it.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", dir.join("data")) };
    HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Edit }, None).expect("headless starts")
}

/// One zebractl process that connects and says nothing stalled every
/// later call for that agent. Now it stalls only itself, and is dropped
/// after the timeout.
#[test]
fn a_silent_client_does_not_block_the_relay() {
    let h = start();
    let relay = zebra_api::relay::socket_for(&h.socket, "silent-test");
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
