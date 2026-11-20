//! Beta features over the socket: importing PowerPoint is refused while
//! they are off, with a hint that only the person can turn them on; agents
//! read the setting in `state` and have no way to change it.
use std::path::PathBuf;
use std::sync::Once;

use serde_json::{Value, json};
use zebra_api::client::{Client, ClientError};
use zebra_api::{ApiOptions, HeadlessHandle, Permission};

fn start(beta: bool) -> HeadlessHandle {
    static INIT: Once = Once::new();
    let dir: PathBuf = std::env::temp_dir().join(format!("zebra-api-beta-{}", std::process::id()));
    INIT.call_once(|| {
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once before any server starts in this test binary.
        unsafe { std::env::set_var("ZEBRA_DATA_DIR", dir.join("data")) };
    });
    HeadlessHandle::spawn_with_beta(ApiOptions { socket: None, permission: Permission::Full }, None, beta).expect("instance starts")
}

fn deck() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../zebra-import/tests/fixtures/deck16x9.pptx")
}

fn refused(r: Result<Value, ClientError>) -> zebra_api::RpcError {
    match r {
        Err(ClientError::Rpc(e)) => e,
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn powerpoint_import_waits_for_the_person() {
    let h = start(false);
    let mut c = Client::connect_hello(&h.socket, "importer", &h.token).unwrap();
    let state = c.call("state", json!({})).unwrap();
    assert_eq!(state["beta_features"], json!(false), "{state}");
    let rev = state["revision"].clone();
    c.call("describe", json!({})).unwrap();
    let e = refused(c.call("import", json!({"path": deck()})));
    assert_eq!(e.kind(), "beta_off", "{e}");
    let words = format!("{e}").to_lowercase();
    assert!(words.contains("settings") && words.contains("only the person"), "{words}");
    for bad in ["engine", "betteroffice", "crate", "flag"] {
        assert!(!words.contains(bad), "{bad}: {words}");
    }
    // Nothing came in.
    assert_eq!(c.call("state", json!({})).unwrap()["revision"], rev);
    // Not a PowerPoint file: refused as before, not as a beta feature.
    let txt = std::env::temp_dir().join(format!("zebra-api-beta-{}.txt", std::process::id()));
    std::fs::write(&txt, "hello").unwrap();
    assert_eq!(refused(c.call("import", json!({"path": txt}))).kind(), "unsupported");
}

#[test]
fn agents_cannot_turn_it_on() {
    let h = start(false);
    let mut c = Client::connect_hello(&h.socket, "sneaky", &h.token).unwrap();
    for op in ["set_beta_features", "beta_features", "set_beta"] {
        assert!(c.call("apply", json!({"command": {"op": op, "on": true}})).is_err(), "apply {op}");
        assert!(c.call("view", json!({"command": {"op": op, "on": true}})).is_err(), "view {op}");
    }
    assert_eq!(c.call("state", json!({})).unwrap()["beta_features"], json!(false));
    // The catalogue offers no way either.
    let cmds = c.call("commands", json!({})).unwrap().to_string();
    assert!(!cmds.contains("set_beta"), "{cmds}");
}

#[test]
fn powerpoint_import_works_once_turned_on() {
    let h = start(true);
    let mut c = Client::connect_hello(&h.socket, "importer", &h.token).unwrap();
    assert_eq!(c.call("state", json!({})).unwrap()["beta_features"], json!(true));
    c.call("describe", json!({})).unwrap();
    let out = c.call("import", json!({"path": deck()})).unwrap();
    assert!(out["pages"].as_u64().unwrap_or(0) >= 1, "{out}");
}
