//! Video through the API: a poster from a frame of the video (`set_poster`
//! with `at`) and `play` / `pause`. Its own test binary: it sets
//! ZEBRA_FAKE_MEDIA (synthetic frames, so this runs off macOS too).
use base64::Engine;
use serde_json::{Value, json};
use zebra_api::client::{Client, ClientError};
use zebra_api::{ApiOptions, HeadlessHandle, Permission};

fn rpc_err(r: Result<Value, ClientError>) -> zebra_api::RpcError {
    match r {
        Err(ClientError::Rpc(e)) => e,
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

#[test]
fn poster_from_a_frame_and_play_needs_a_window() {
    let dir = std::env::temp_dir().join(format!("zebra-media-api-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: the only test in this binary, before any thread starts.
    unsafe {
        std::env::set_var("ZEBRA_DATA_DIR", dir.join("data"));
        std::env::set_var("ZEBRA_MEDIA_CACHE", dir.join("cache"));
        std::env::set_var("ZEBRA_FAKE_MEDIA", "1");
    }
    let h = HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Full }, None).unwrap();
    let mut c = Client::connect_hello(&h.socket, "media", &h.token).unwrap();
    let mp4 = zebra_core::media::fixtures::tiny_mp4(320, 240, 10, true);
    let b64 = base64::engine::general_purpose::STANDARD.encode(mp4);
    let out = c.call("apply", json!({"command":{"op":"add_video","data_base64":b64}})).unwrap();
    let id = out["created"][0].as_str().unwrap().to_string();

    let r = c.call("apply", json!({"command":{"op":"set_poster","id":id,"at":2.5}})).unwrap();
    assert!(r["revision"].as_u64().unwrap() > 0, "{r}");
    let el = c.call("query", json!({"query":{"q":"element","id":id}})).unwrap();
    assert!(el.to_string().contains("poster"), "the poster is set: {el}");

    let e = rpc_err(c.call("apply", json!({"command":{"op":"set_poster","id":id,"at":-1}})));
    assert_eq!(e.kind(), "invalid_params");
    let e = rpc_err(c.call("apply", json!({"command":{"op":"set_poster","id":"nope","at":1}})));
    assert_eq!(e.kind(), "not_found");

    // Headless has no window: play says so instead of a silent ok.
    let e = rpc_err(c.call("view", json!({"command":{"op":"play","id":id}})));
    assert_eq!(e.kind(), "unsupported", "{e}");
    // Pausing is harmless.
    c.call("view", json!({"command":{"op":"pause","id":id}})).unwrap();
    h.stop().unwrap();

    // Edit permission: play and pause are present-mode commands, full only.
    let h = HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Edit }, None).unwrap();
    let mut c = Client::connect_hello(&h.socket, "media2", &h.token).unwrap();
    let e = rpc_err(c.call("view", json!({"command":{"op":"pause","id":"v1"}})));
    assert_eq!(e.kind(), "permission", "{e}");
    h.stop().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
