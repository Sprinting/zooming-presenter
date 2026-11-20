//! The `import` method: a PDF's pages in one frame with a path step each, one undo
//! step, read off the dispatcher's thread. Needs PDFium (see
//! crates/zebra-import/tests/pdf.rs); without it the PDF parts are skipped.
use std::path::PathBuf;

use serde_json::{Value, json};
use zebra_api::client::{Client, ClientError};
use zebra_api::{ApiOptions, HeadlessHandle, Permission};

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("zebra-api-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn engine() -> bool {
    if std::env::var_os("ZEBRA_PDFIUM").is_none() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/pdfium");
        let lib = if cfg!(target_os = "macos") { "libpdfium.dylib" } else { "libpdfium.so" };
        if let Some(p) = ["mac-arm64", "mac-univ", "mac-x64", "linux-x64"].iter().map(|p| root.join(p).join("lib").join(lib)).find(|p| p.is_file()) {
            // SAFETY: before any server thread of this binary starts.
            unsafe { std::env::set_var("ZEBRA_PDFIUM", p) };
        }
    }
    let ok = zebra_import::pdf::available();
    if !ok {
        eprintln!("SKIPPED: PDFium not found (scripts/fetch-pdfium.sh linux-x64, or ZEBRA_PDFIUM)");
    }
    ok
}

fn rpc_err(r: Result<Value, ClientError>) -> zebra_api::RpcError {
    match r {
        Err(ClientError::Rpc(e)) => e,
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

#[test]
fn import_adds_pages_as_one_undo_step() {
    let pdf_ok = engine();
    let dir = scratch();
    // SAFETY: as above.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", dir.join("data")) };
    let h = HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Full }, None).expect("headless");
    let mut c = Client::connect_hello(&h.socket, "importer", &h.token).unwrap();

    // Bad params are refused before any file is read.
    assert_eq!(rpc_err(c.call("import", json!({}))).kind(), "invalid_params");
    assert_eq!(rpc_err(c.call("import", json!({"path":"relative.pdf"}))).kind(), "invalid_params");
    assert_eq!(rpc_err(c.call("import", json!({"path":"/x.pdf","from":3,"to":2}))).kind(), "invalid_params");
    assert_eq!(rpc_err(c.call("import", json!({"path":"/x.pdf","from":0}))).kind(), "invalid_params");
    let e = rpc_err(c.call("import", json!({"path":"/x.pdf","pages":"all"})));
    assert!(e.message.contains("pages"), "{e}");
    // A file that isn't a PDF, and one that isn't there.
    let txt = dir.join("notes.txt");
    std::fs::write(&txt, "hello").unwrap();
    assert_eq!(rpc_err(c.call("import", json!({"path": txt}))).kind(), "unsupported");
    assert_eq!(rpc_err(c.call("import", json!({"path": dir.join("missing.pdf")}))).kind(), "io");
    if !pdf_ok {
        return;
    }

    // A three-step deck, exported as PDF, imported back after it.
    c.call("describe", json!({})).unwrap();
    c.call(
        "batch",
        json!({"label":"deck","commands":[
            {"op":"add_frame","id":"a","x":0,"y":0,"add_to_path":true},
            {"op":"add_text","text":"One","style":"title","place":{"relative_to":"a","side":"inside"}},
            {"op":"add_frame","id":"b","x":1200,"y":0,"add_to_path":true},
            {"op":"add_text","text":"Two","place":{"relative_to":"b","side":"inside"}},
            {"op":"add_frame","id":"c","x":2400,"y":0,"add_to_path":true}
        ]}),
    )
    .unwrap();
    let pdf = dir.join("deck.pdf");
    c.call("export", json!({"format":"pdf","path": pdf})).unwrap();
    let before = c.call("describe", json!({})).unwrap();
    let out = c.call("import", json!({"path": pdf, "from": 1, "to": 2})).unwrap();
    assert_eq!(out["pages"], 2, "{out}");
    let frames: Vec<String> = out["frames"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect();
    // One frame around both pages; each page is its own step.
    assert_eq!(frames.len(), 1);
    let path = c.call("query", json!({"query":{"q":"path"}})).unwrap();
    let steps = path["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 5, "{path}");
    // The new frame sits right of everything that was there.
    for f in &frames {
        let e = c.call("query", json!({"query":{"q":"element","id":f}})).unwrap();
        let x0 = e["bounds"][0].as_f64().unwrap_or_else(|| panic!("{e}"));
        assert!(x0 > 2400.0 + 400.0, "{e}");
    }
    let undone = c.call("undo", json!({})).unwrap();
    assert_eq!(undone["undone"], "Import deck.pdf");
    // The same document again (only the revision number moved on).
    let strip = |v: Value| v["outline"].as_str().unwrap().split_once("\nPath:").unwrap().1.to_string();
    assert_eq!(strip(c.call("describe", json!({})).unwrap()), strip(before));
    // Beyond the last page.
    c.call("describe", json!({})).unwrap();
    assert_eq!(rpc_err(c.call("import", json!({"path": pdf, "from": 9}))).kind(), "io");
}

#[test]
fn import_needs_edit_permission() {
    let dir = scratch();
    // SAFETY: as above.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", dir.join("data2")) };
    let h = HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Observe }, None).expect("headless");
    let mut c = Client::connect_hello(&h.socket, "watcher", &h.token).unwrap();
    assert_eq!(rpc_err(c.call("import", json!({"path":"/tmp/x.pdf"}))).kind(), "permission");
}
