//! Spec 18a/18b over the API: a theme style's weight is set with
//! save_theme, read back from the themes query and describe, a family's
//! weights come from the fonts query, and format_text bold:false makes
//! words Regular in a heavy style.
use std::path::PathBuf;
use std::sync::Once;

use serde_json::json;
use zebra_api::client::Client;
use zebra_api::{ApiOptions, HeadlessHandle, Permission};

fn start() -> HeadlessHandle {
    static INIT: Once = Once::new();
    let dir: PathBuf = std::env::temp_dir().join(format!("zebra-api-weight-{}", std::process::id()));
    INIT.call_once(|| {
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once before any server starts in this test binary.
        unsafe { std::env::set_var("ZEBRA_DATA_DIR", dir.join("data")) };
    });
    HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Edit }, None).expect("instance starts")
}

#[test]
fn weights_over_the_api() {
    let h = start();
    let mut c = Client::connect_hello(&h.socket, "weights", &h.token).unwrap();
    let q = |c: &mut Client, q: serde_json::Value| c.call("query", json!({ "query": q })).unwrap();

    let fonts = q(&mut c, json!({"q":"fonts","family":"Sans"}));
    let names: Vec<&str> = fonts["weights"].as_array().unwrap().iter().map(|w| w["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Ultralight", "Thin", "Light", "Regular", "Medium", "Semibold", "Bold", "Heavy", "Black"]);
    let hand: Vec<u64> = q(&mut c, json!({"q":"fonts","family":"Hand"}))["weights"].as_array().unwrap().iter().map(|w| w["weight"].as_u64().unwrap()).collect();
    assert_eq!(hand, [400, 700]);

    // Built-in themes report Regular.
    let themes = q(&mut c, json!({"q":"themes"}));
    assert_eq!(themes["themes"][0]["title"]["weight"], 400);

    c.call(
        "apply",
        json!({"command": {"op":"save_theme","apply":true,"theme":{"id":"heavy","name":"Heavy","background":"#ffffff",
            "title":{"font":"Sans","color":"#000000","size":72,"weight":800},
            "subtitle":{"font":"Sans","color":"#333333","size":40,"weight":600},
            "body":{"font":"Sans","color":"#333333","size":24},
            "frame":"#3d7ab8","frame_fill":"#e8f0f8","accent":"#e8792b","shapes":["#3d7ab8","#e8792b","#6aa84f","#9e9e9e"]}}}),
    )
    .unwrap();
    let themes = q(&mut c, json!({"q":"themes"}));
    let custom = &themes["custom"][0];
    assert_eq!((custom["title"]["weight"].as_u64(), custom["body"]["weight"].as_u64()), (Some(800), Some(400)), "a style without a weight is Regular");
    let outline = q(&mut c, json!({"q":"describe"}))["outline"].as_str().unwrap().to_string();
    assert!(outline.contains("Styles: title Sans Heavy 72 · subtitle Sans Semibold 40 · body Sans Regular 24"), "{outline}");

    // A weight outside 1..1000 is refused with a hint.
    let bad = c.call("apply", json!({"command": {"op":"save_theme","theme":{"id":"bad","name":"Bad","background":"#ffffff",
        "title":{"font":"Sans","color":"#000000","size":72,"weight":0},
        "subtitle":{"font":"Sans","color":"#333333","size":40},"body":{"font":"Sans","color":"#333333","size":24},
        "frame":"#3d7ab8","frame_fill":"#e8f0f8","accent":"#e8792b","shapes":["#3d7ab8","#e8792b","#6aa84f","#9e9e9e"]}}}));
    assert!(bad.is_err());

    // bold:false on words of the Heavy title makes them Regular.
    c.call("apply", json!({"command": {"op":"add_text","id":"t","text":"Big title","style":"title","x":0,"y":0}})).unwrap();
    c.call("apply", json!({"command": {"op":"format_text","id":"t","find":"Big","format":{"bold":false}}})).unwrap();
    let doc: zebra_core::Document = serde_json::from_value(q(&mut c, json!({"q":"document"}))).unwrap();
    let zebra_core::ElementKind::Text(t) = &doc.get("t").unwrap().kind else { panic!("not text") };
    assert_eq!(zebra_core::rich::weight_at(t, 800, 0), 400);
    assert_eq!(zebra_core::rich::weight_at(t, 800, 4), 800);
}
