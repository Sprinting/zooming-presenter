//! Frames over the socket (spec 18d): what a frame holds follows the
//! outline it draws, and an agent can select a frame on its own, or move
//! it alone, as a person does with Option-click.
use std::path::PathBuf;
use std::sync::Once;

use serde_json::{Value, json};
use zebra_api::client::{Client, ClientError};
use zebra_api::{ApiOptions, HeadlessHandle, Permission, RpcError};

fn start(permission: Permission) -> HeadlessHandle {
    static INIT: Once = Once::new();
    let dir: PathBuf = std::env::temp_dir().join(format!("zebra-api-frame-outline-{}", std::process::id()));
    INIT.call_once(|| {
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once before any server starts in this test binary.
        unsafe { std::env::set_var("ZEBRA_DATA_DIR", dir.join("data")) };
    });
    HeadlessHandle::spawn(ApiOptions { socket: None, permission }, None).expect("instance starts")
}

fn refused(r: Result<Value, ClientError>) -> RpcError {
    match r {
        Err(ClientError::Rpc(e)) => e,
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

fn element(c: &mut Client, id: &str) -> Value {
    c.call("query", json!({"query":{"q":"element","id":id}})).unwrap()
}

fn set_up(c: &mut Client) {
    c.call(
        "batch",
        json!({"label": "Set up", "commands": [
            {"op":"add_frame","id":"f1","x":0,"y":0,"w":1000,"h":1000,"style":"rectangle"},
            {"op":"add_shape","id":"mid","shape":"rectangle","x":0,"y":0,"w":200,"h":200},
            {"op":"add_shape","id":"corner","shape":"rectangle","x":430,"y":430,"w":60,"h":60}
        ]}),
    )
    .unwrap();
}

#[test]
fn a_frame_holds_what_lies_in_its_outline() {
    let h = start(Permission::Edit);
    let mut c = Client::connect_hello(&h.socket, "framer", &h.token).unwrap();
    set_up(&mut c);
    assert_eq!(element(&mut c, "f1")["contents"], json!(["mid", "corner"]));
    // A circle: the corner object lies outside it now.
    c.call("apply", json!({"command":{"op":"update","id":"f1","patch":{"frame_style":"circle"}}})).unwrap();
    assert_eq!(element(&mut c, "f1")["contents"], json!(["mid"]));
    assert_eq!(element(&mut c, "corner")["parent_frame"], Value::Null);
    // Moving the frame alone leaves what it holds.
    c.call("apply", json!({"command":{"op":"move","ids":["f1"],"dx":300,"dy":0,"with_contents":false}})).unwrap();
    assert_eq!(element(&mut c, "mid")["transform"]["x"], json!(0.0));
    assert_eq!(element(&mut c, "f1")["transform"]["x"], json!(300.0));
    // With its contents (the default), what it holds goes with it.
    c.call("apply", json!({"command":{"op":"move","ids":["f1"],"dx":-300,"dy":0}})).unwrap();
    assert_eq!(element(&mut c, "mid")["transform"]["x"], json!(-300.0));
    h.stop().unwrap();
}

#[test]
fn select_a_frame_on_its_own() {
    let h = start(Permission::Full);
    let mut c = Client::connect_hello(&h.socket, "framer", &h.token).unwrap();
    set_up(&mut c);
    c.call("view", json!({"command":{"op":"select","ids":["f1"],"frame_only":true}})).unwrap();
    let st = c.call("state", json!({})).unwrap();
    assert_eq!(st["selection"], json!(["f1"]));
    assert_eq!(st["frame_only"], json!(true));
    // Any other selection ends it.
    c.call("view", json!({"command":{"op":"select","ids":["f1"]}})).unwrap();
    assert_eq!(c.call("state", json!({})).unwrap()["frame_only"], json!(false));

    // Strict: one frame, and no made-up fields.
    let e = refused(c.call("view", json!({"command":{"op":"select","ids":["f1","mid"],"frame_only":true}})));
    assert_eq!(e.kind(), "invalid_params", "{e}");
    assert!(e.message.contains("one frame"), "{e}");
    let e = refused(c.call("view", json!({"command":{"op":"select","ids":["mid"],"frame_only":true}})));
    assert_eq!(e.kind(), "invalid_params", "{e}");
    assert!(e.message.contains("'mid' is a shape"), "{e}");
    let e = refused(c.call("view", json!({"command":{"op":"select","ids":["f1"],"alone":true}})));
    assert_eq!(e.kind(), "invalid_params", "{e}");
    let vals: Vec<&str> = e.data["valid_values"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(vals, ["frame_only", "ids"], "{e}");
    h.stop().unwrap();
}

#[test]
fn only_full_control_selects_for_the_person() {
    let h = start(Permission::Edit);
    let mut c = Client::connect_hello(&h.socket, "framer", &h.token).unwrap();
    set_up(&mut c);
    let e = refused(c.call("view", json!({"command":{"op":"select","ids":["f1"],"frame_only":true}})));
    assert_eq!(e.kind(), "permission", "{e}");
    assert!(e.hint_text().unwrap_or("").contains("with_contents"), "{e}");
    h.stop().unwrap();
}
