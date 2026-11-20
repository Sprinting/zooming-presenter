//! `split_text` over the socket: an agent drags words out of a box the way
//! a person does (spec 18c), with strict fields and one undo step.
use std::path::PathBuf;
use std::sync::Once;

use serde_json::{Value, json};
use zebra_api::client::{Client, ClientError};
use zebra_api::{ApiOptions, HeadlessHandle, Permission};
use zebra_core::model::{ElementKind, ListKind, SpanFormat};

fn start() -> HeadlessHandle {
    static INIT: Once = Once::new();
    let dir: PathBuf = std::env::temp_dir().join(format!("zebra-api-split-{}", std::process::id()));
    INIT.call_once(|| {
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once before any server starts in this test binary.
        unsafe { std::env::set_var("ZEBRA_DATA_DIR", dir.join("data")) };
    });
    HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Edit }, None).expect("instance starts")
}

fn doc(c: &mut Client) -> zebra_core::Document {
    serde_json::from_value(c.call("query", json!({"query":{"q":"document"}})).unwrap()).unwrap()
}

fn text(d: &zebra_core::Document, id: &str) -> zebra_core::TextData {
    match &d.get(id).unwrap_or_else(|| panic!("no {id}")).kind {
        ElementKind::Text(t) => t.clone(),
        _ => panic!("{id} is not a text"),
    }
}

#[test]
fn split_text_moves_words_into_a_new_box() {
    let h = start();
    let mut c = Client::connect_hello(&h.socket, "splitter", &h.token).unwrap();
    c.call(
        "batch",
        json!({"label": "Set up", "commands": [
            {"op":"add_frame","id":"f1","x":2000,"y":0,"w":800,"h":600},
            {"op":"add_text","id":"t","text":"Zebras have stripes\nflies get confused\nthe end","x":0,"y":0},
            {"op":"format_text","id":"t","find":"confused","format":{"bold":true}},
            {"op":"format_paragraphs","id":"t","first":1,"last":1,"list":"bullet"}
        ]}),
    )
    .unwrap();
    // The bullet paragraph, whole, into the frame.
    let out = c.call("apply", json!({"command":{"op":"split_text","id":"t","start":20,"end":38,"x":2000,"y":0}})).unwrap();
    let new = out["created"][0].as_str().expect("the new box's id").to_string();
    let d = doc(&mut c);
    let t = text(&d, &new);
    assert_eq!(t.text, "flies get confused");
    assert_eq!(t.spans[0].format, SpanFormat { bold: Some(true), ..Default::default() });
    assert_eq!(zebra_core::rich::para(&t, 0).0, ListKind::Bullet);
    assert_eq!(text(&d, "t").text, "Zebras have stripes\nthe end");
    assert_eq!(d.parent_frame(&new).as_deref(), Some("f1"), "it landed in the frame");
    let hist = c.call("query", json!({"query":{"q":"history"}})).unwrap();
    assert_eq!(hist["undo"][0]["label"], "Move text", "newest first: {hist}");

    // One undo step.
    c.call("undo", json!({})).unwrap();
    let d = doc(&mut c);
    assert_eq!(text(&d, "t").text, "Zebras have stripes\nflies get confused\nthe end");
    assert!(d.get(&new).is_none());

    // Copy, placed by default just below; the source stays.
    let out = c.call("apply", json!({"command":{"op":"split_text","id":"t","start":0,"end":6,"copy":true}})).unwrap();
    let copy = out["created"][0].as_str().unwrap().to_string();
    let d = doc(&mut c);
    assert_eq!(text(&d, &copy).text, "Zebras");
    assert_eq!(text(&d, "t").text, "Zebras have stripes\nflies get confused\nthe end");
    assert!(d.get(&copy).unwrap().bounds().y0 > d.get("t").unwrap().bounds().y1);
    let hist = c.call("query", json!({"query":{"q":"history"}})).unwrap();
    assert_eq!(hist["undo"][0]["label"], "Copy text", "{hist}");
    h.stop().unwrap();
}

#[test]
fn split_text_refuses_unknown_fields_and_bad_ranges() {
    let h = start();
    let mut c = Client::connect_hello(&h.socket, "splitter", &h.token).unwrap();
    c.call("apply", json!({"command":{"op":"add_text","id":"t","text":"Hello","x":0,"y":0}})).unwrap();
    let rev = c.call("state", json!({})).unwrap()["revision"].clone();
    let refused = |r: Result<Value, ClientError>| match r {
        Err(ClientError::Rpc(e)) => e,
        other => panic!("expected an RPC error, got {other:?}"),
    };
    let e = refused(c.call("apply", json!({"command":{"op":"split_text","id":"t","start":0,"end":2,"to":4}})));
    assert_eq!(e.kind(), "invalid_params", "{e}");
    assert!(e.message.contains("unknown field `to`") && e.message.contains("split_text"), "{e}");
    let accepted: Vec<&str> = e.data["valid_values"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(accepted, ["copy", "end", "id", "start", "x", "y"], "{e}");
    let e = refused(c.call("apply", json!({"command":{"op":"split_text","id":"t","start":2,"end":9}})));
    assert_eq!(e.kind(), "invalid_params", "{e}");
    let e = refused(c.call("apply", json!({"command":{"op":"split_text","id":"t","start":0,"end":2,"y":3}})));
    assert_eq!(e.kind(), "invalid_params", "{e}");
    assert_eq!(c.call("state", json!({})).unwrap()["revision"], rev, "nothing changed");
    h.stop().unwrap();
}
