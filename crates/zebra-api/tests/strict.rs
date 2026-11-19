//! Unknown fields are refused, never ignored: one test per command family,
//! over the real socket and through the MCP bridge.
use std::path::PathBuf;
use std::sync::Once;

use serde_json::{Value, json};
use zebra_api::client::{Client, ClientError};
use zebra_api::{ApiOptions, HeadlessHandle, Permission, RpcError};

fn start() -> HeadlessHandle {
    static INIT: Once = Once::new();
    let dir: PathBuf = std::env::temp_dir().join(format!("zebra-api-strict-{}", std::process::id()));
    INIT.call_once(|| {
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once before any server starts in this test binary.
        unsafe { std::env::set_var("ZEBRA_DATA_DIR", dir.join("data")) };
    });
    HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Full }, None).expect("headless starts")
}

fn refused(r: Result<Value, ClientError>) -> RpcError {
    match r {
        Err(ClientError::Rpc(e)) => e,
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

/// The error names the field and the command and lists what's accepted.
fn assert_unknown(e: &RpcError, field: &str, what: &str, accepted: &str) {
    assert_eq!(e.kind(), "invalid_params", "{e}");
    assert!(e.message.contains(&format!("unknown field `{field}`")), "{e}");
    assert!(e.message.contains(what), "{e}");
    assert_eq!(e.data["field"], field, "{e}");
    let vals: Vec<&str> = e.data["valid_values"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
    assert!(vals.contains(&accepted), "{accepted} not in {vals:?}: {e}");
}

#[test]
fn every_family_refuses_unknown_fields() {
    let h = start();
    let mut c = Client::connect_hello(&h.socket, "strict", &h.token).unwrap();
    c.call("apply", json!({"command":{"op":"add_frame","id":"f1","x":0,"y":0}})).unwrap();
    c.call("apply", json!({"command":{"op":"add_text","id":"t1","text":"Hi","x":0,"y":0}})).unwrap();
    let rev = c.call("state", json!({})).unwrap()["revision"].clone();

    // Document commands: apply (top level and nested) and batch.
    let e = refused(c.call("apply", json!({"command":{"op":"add_ink","points":[[0,0],[9,9]],"colour":"#ff0000"}})));
    assert_unknown(&e, "colour", "command `add_ink`", "color");
    assert!(e.hint_text().unwrap_or("").contains("add_ink"), "example in the hint: {e}");
    let e = refused(c.call("apply", json!({"command":{"op":"update","id":"t1","patch":{"txt":"x"}}})));
    assert_unknown(&e, "patch.txt", "command `update`", "text");
    let e = refused(c.call("batch", json!({"commands":[{"op":"set_title","title":"A"},{"op":"move","ids":["t1"],"dx":1,"dy":1,"dz":1}]})));
    assert_unknown(&e, "dz", "command `move`", "dy");
    assert!(e.message.starts_with("commands[1]:"), "{e}");
    // Nothing was applied.
    assert_eq!(c.call("state", json!({})).unwrap()["revision"], rev);

    // View commands and queries.
    let e = refused(c.call("view", json!({"command":{"op":"zoom","factor":2,"smooth":true}})));
    assert_unknown(&e, "smooth", "view command `zoom`", "factor");
    let e = refused(c.call("query", json!({"query":{"q":"elements","inside":"f1"}})));
    assert_unknown(&e, "inside", "query `elements`", "within");
    // Flat forms are checked the same way.
    let e = refused(c.call("view", json!({"op":"zoom_to","id":"f1","speed":3})));
    assert_unknown(&e, "speed", "view command `zoom_to`", "id");
    let e = refused(c.call("query", json!({"q":"element","id":"t1","full":true})));
    assert_unknown(&e, "full", "query `element`", "id");

    // Method params, and objects inside them.
    let e = refused(c.call("apply", json!({"command":{"op":"set_title","title":"B"},"lable":"Rename"})));
    assert_unknown(&e, "lable", "method `apply`", "label");
    let e = refused(c.call("batch", json!({"commands":[],"atomic":true})));
    assert_unknown(&e, "atomic", "method `batch`", "commands");
    let e = refused(c.call("capture", json!({"target":"overview","scale":2})));
    assert_unknown(&e, "scale", "method `capture`", "width");
    let e = refused(c.call("capture", json!({"target":{"step":0,"zoom":2}})));
    assert_unknown(&e, "zoom", "capture target", "build");
    let e = refused(c.call("wait", json!({"until":{"revison":3}})));
    assert_unknown(&e, "revison", "`until`", "revision");
    let e = refused(c.call("describe", json!({"verbose":true})));
    assert!(e.message.contains("unknown field `verbose` in method `describe`; accepted fields: none"), "{e}");
    let e = refused(c.call("copy_to", json!({"ids":["t1"],"to":"x","at":{"x":0,"y":0,"z":0}})));
    assert_unknown(&e, "z", "`at`", "y");

    // The request itself: a field beside `params` would be lost.
    let r = c.raw(json!({"id":77,"method":"apply","command":{"op":"set_title","title":"C"}})).unwrap();
    let e = RpcError::from_json(&r["error"]);
    assert_unknown(&e, "command", "the request", "params");

    // What's known still works: `document`, a flat apply with a label.
    c.call("apply", json!({"op":"set_title","title":"Fine","label":"Rename","if_revision":null})).unwrap();
    let doc = c.call("documents", json!({})).unwrap()["documents"][0]["id"].clone();
    c.call("describe", json!({ "document": doc })).unwrap();
    c.call("query", json!({"query":{"q":"elements","within":"f1"}})).unwrap();
    h.stop().unwrap();
}

#[test]
fn mcp_tools_refuse_unknown_arguments() {
    let h = start();
    let (sock, tok) = (h.socket.clone(), h.token.clone());
    let mut mcp = zebra_api::mcp::McpServer::new(move || Ok(Client::connect_hello(&sock, "mcp-strict", &tok)?));
    let call = |id: i64, name: &str, args: Value| json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}});
    let input = [
        call(1, "apply_commands", json!({"commands":[{"op":"set_title","title":"A"}],"labl":"x"})),
        call(2, "apply_commands", json!({"commands":[{"op":"add_ink","points":[[0,0],[1,1]],"colour":"#000000"}]})),
        call(3, "capture", json!({"target":"overview","format":"jpeg"})),
        call(4, "view", json!({"command":{"op":"fit_all","padding":10}})),
        call(5, "query", json!({"query":{"q":"path","all":true}})),
        call(6, "apply_commands", json!({"commands":[{"op":"set_title","title":"Fine"}],"label":"Rename"})),
    ];
    let text: String = input.iter().map(|v| v.to_string() + "\n").collect();
    let mut out = Vec::new();
    mcp.serve(std::io::Cursor::new(text), &mut out).unwrap();
    let resps: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let text_of = |id: i64| -> (bool, String) {
        let r = resps.iter().find(|r| r["id"] == id).unwrap();
        (r["result"]["isError"] == true, r["result"]["content"][0]["text"].as_str().unwrap_or("").to_string())
    };
    for (id, field, what) in [(1, "labl", "tool `apply_commands`"), (2, "colour", "command `add_ink`"), (3, "format", "tool `capture`"), (4, "padding", "view command `fit_all`"), (5, "all", "query `path`")] {
        let (err, t) = text_of(id);
        assert!(err, "{id}: {t}");
        assert!(t.contains(&format!("unknown field `{field}`")) && t.contains(what), "{id}: {t}");
    }
    assert!(text_of(1).1.contains("label"), "lists accepted arguments: {}", text_of(1).1);
    let (err, t) = text_of(6);
    assert!(!err, "{t}");
    h.stop().unwrap();
}

/// Every catalogue example passes the unknown-field check as it would be
/// sent (`add_image` reads its file first, so its `path` is swapped for
/// the bytes the way the server does).
#[test]
fn every_catalogue_example_still_parses() {
    let dir = std::env::temp_dir().join(format!("zebra-api-strict-ex-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("logo.png");
    image_png(&png);
    for (kind, ex) in zebra_api::catalogue::EXAMPLES {
        let mut v: Value = serde_json::from_str(ex).unwrap();
        if v["op"] == "add_image" {
            v["path"] = json!(png);
        }
        let r = match kind {
            zebra_api::catalogue::Kind::Command => zebra_api::protocol::parse("apply", &json!({ "command": v })).map(|_| ()),
            zebra_api::catalogue::Kind::View => zebra_api::protocol::parse("view", &json!({ "command": v })).map(|_| ()),
            zebra_api::catalogue::Kind::Query => zebra_api::protocol::parse("query", &json!({ "query": v })).map(|_| ()),
        };
        assert!(r.is_ok(), "{ex}: {}", r.err().unwrap());
    }
}

fn image_png(p: &std::path::Path) {
    // A 1x1 PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
        0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
        0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    std::fs::write(p, PNG).unwrap();
}
