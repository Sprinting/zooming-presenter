//! End-to-end: a headless dispatcher loop on a thread, real Unix socket,
//! real client.
use std::path::PathBuf;
use std::sync::Once;
use std::time::Duration;

use serde_json::{Value, json};
use zebra_api::client::{Client, ClientError};
use zebra_api::{ApiOptions, HeadlessHandle, Permission};

fn scratch() -> PathBuf {
    static INIT: Once = Once::new();
    let dir = std::env::temp_dir().join(format!("zebra-api-test-{}", std::process::id()));
    INIT.call_once(|| {
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once before any server starts in this test binary.
        unsafe { std::env::set_var("ZEBRA_DATA_DIR", dir.join("data")) };
    });
    dir
}

fn start(perm: Permission) -> HeadlessHandle {
    scratch();
    HeadlessHandle::spawn(ApiOptions { socket: None, permission: perm }, None).expect("headless starts")
}

fn connect(h: &HeadlessHandle, name: &str) -> Client {
    Client::connect_hello(&h.socket, name, &h.token).expect("hello")
}

fn rpc_err(r: Result<Value, ClientError>) -> zebra_api::RpcError {
    match r {
        Err(ClientError::Rpc(e)) => e,
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

#[test]
fn full_session() {
    let h = start(Permission::Full);
    let dir = scratch();

    // Discovery file exists and points at us.
    let inst = zebra_api::discovery::list().into_iter().find(|i| i.socket == h.socket).expect("discovery file");
    assert_eq!(inst.pid, std::process::id());
    assert_eq!(inst.token, h.token);
    let mode = std::fs::metadata(&inst.file).map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777).unwrap();
    assert_eq!(mode, 0o600);
    let sock_mode = std::fs::metadata(&h.socket).map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777).unwrap();
    assert_eq!(sock_mode, 0o600);

    // Unauthenticated calls are rejected; ping is allowed.
    let mut anon = Client::connect(&h.socket).unwrap();
    assert_eq!(anon.call("ping", json!({})).unwrap(), json!("pong"));
    let e = rpc_err(anon.call("describe", json!({})));
    assert_eq!(e.kind(), "unauthenticated");
    assert!(e.message.contains("hello"), "{e}");
    let e = rpc_err(anon.call("hello", json!({"name":"x","token":"nope"})));
    assert_eq!(e.kind(), "unauthenticated");

    let mut c = connect(&h, "tester");
    let mut watcher = connect(&h, "watcher");
    let sub = watcher.call("subscribe", json!({})).unwrap();
    assert!(sub["subscribed"].as_array().unwrap().len() >= 5);

    // Batch: 3 frames on the path, text inside each.
    let out = c
        .call(
            "batch",
            json!({"label": "Build deck", "commands": [
                {"op":"set_title","title":"Demo"},
                {"op":"add_frame","id":"intro","x":0,"y":0,"name":"Intro","add_to_path":true},
                {"op":"add_text","text":"Hello Zebra","style":"title","place":{"relative_to":"intro","side":"inside"}},
                {"op":"add_frame","id":"middle","x":1200,"y":0,"style":"circle","add_to_path":true},
                {"op":"add_text","text":"Middle","place":{"relative_to":"middle","side":"inside"}},
                {"op":"add_frame","id":"end","x":2400,"y":0,"add_to_path":true},
                {"op":"add_shape","shape":"arrow","x":600,"y":0,"w":200,"h":50}
            ]}),
        )
        .unwrap();
    let rev = out["revision"].as_u64().unwrap();
    assert!(out["created"].as_array().unwrap().len() >= 6, "{out}");

    // The watcher sees an origin-tagged document_changed event.
    let ev = loop {
        let ev = watcher.next_event(Some(Duration::from_secs(5))).unwrap().expect("event");
        if ev["event"] == "document_changed" {
            break ev;
        }
    };
    assert_eq!(ev["origin"], json!({"kind":"agent","name":"tester"}));
    assert_eq!(ev["label"], "Build deck");

    let d = c.call("describe", json!({})).unwrap();
    let outline = d["outline"].as_str().unwrap();
    assert!(outline.contains("Hello Zebra") && outline.contains("intro"), "{outline}");
    let st = c.call("state", json!({})).unwrap();
    assert_eq!(st["path_length"], 3);
    assert_eq!(st["agent_permission"], "full");
    let q = c.call("query", json!({"query":{"q":"elements","kind":"frame"}})).unwrap();
    assert_eq!(q["elements"].as_array().unwrap().len(), 3);
    // String shorthand.
    let p = c.call("query", json!({"query":"path"})).unwrap();
    assert_eq!(p["steps"].as_array().unwrap().len(), 3);

    // Single apply, then undo it. The batch stays.
    let a = c.call("apply", json!({"command":{"op":"add_text","text":"oops","x":0,"y":900}})).unwrap();
    assert!(a["revision"].as_u64().unwrap() > rev);
    let u = c.call("undo", json!({})).unwrap();
    assert!(u["undone"].is_string(), "{u}");
    let q = c.call("query", json!({"query":{"q":"elements","text":"oops"}})).unwrap();
    assert!(q["elements"].as_array().unwrap().is_empty());
    let r = c.call("redo", json!({})).unwrap();
    assert!(r["redone"].is_string());
    c.call("undo", json!({})).unwrap();

    // if_revision guards stale writes.
    let e = rpc_err(c.call("apply", json!({"command":{"op":"set_title","title":"x"},"if_revision": 1})));
    assert_eq!(e.kind(), "stale_revision");

    // Transactions: begin/apply/abort leaves nothing behind.
    c.call("begin", json!({"label":"tx"})).unwrap();
    c.call("apply", json!({"command":{"op":"add_text","text":"temp","x":0,"y":1200}})).unwrap();
    c.call("abort", json!({})).unwrap();
    let q = c.call("query", json!({"query":{"q":"elements","text":"temp"}})).unwrap();
    assert!(q["elements"].as_array().unwrap().is_empty());
    let e = rpc_err(c.call("commit", json!({})));
    assert_eq!(e.kind(), "no_transaction");

    // Present + wait for arrival at step 1 (full permission drives the window).
    c.call("view", json!({"command":{"op":"present","step":0}})).unwrap();
    c.call("view", json!({"command":{"op":"next"}})).unwrap();
    let w = c.call("wait", json!({"until":{"step":1},"timeout_ms":10000})).unwrap();
    assert_eq!(w["step"], 1);
    let w = c.call("wait", json!({"until":"idle","timeout_ms":10000})).unwrap();
    assert_eq!(w["ok"], true);
    let e = rpc_err(c.call("wait", json!({"until":{"step":2},"timeout_ms":100})));
    assert_eq!(e.kind(), "timeout");
    c.call("view", json!({"command":{"op":"exit_present"}})).unwrap();

    // Capture: base64 PNG or a file. Tolerate an unimplemented renderer.
    match c.call("capture", json!({"target":{"step":0},"width":320,"height":200})) {
        Ok(v) => {
            use base64::Engine;
            let png = base64::engine::general_purpose::STANDARD.decode(v["data_base64"].as_str().unwrap()).unwrap();
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
            let path = dir.join("shot.png");
            let v = c.call("capture", json!({"target":"overview","width":200,"height":120,"path":path})).unwrap();
            assert!(v["bytes"].as_u64().unwrap() > 0);
            assert!(path.exists());
        }
        Err(ClientError::Rpc(e)) if e.kind() == "unimplemented" => eprintln!("capture not implemented yet"),
        Err(e) => panic!("capture failed: {e}"),
    }
    let e = rpc_err(c.call("capture", json!({"target":{"step":99}})));
    assert_eq!(e.kind(), "invalid_params");
    let e = rpc_err(c.call("capture", json!({"target":{"element":"nope"}})));
    assert_eq!(e.kind(), "not_found");

    // Save / open roundtrip.
    let file = dir.join("deck.zebra");
    c.call("save", json!({"path": file})).unwrap();
    assert!(file.exists());
    c.call("apply", json!({"command":{"op":"set_title","title":"Changed"}})).unwrap();
    let e = rpc_err(c.call("open", json!({"path": file})));
    assert_eq!(e.kind(), "unsaved_changes");
    c.call("open", json!({"path": file, "force": true})).unwrap();
    let st = c.call("state", json!({})).unwrap();
    assert_eq!(st["title"], "Demo");
    assert_eq!(st["path_length"], 3);
    assert_eq!(st["dirty"], false);

    // Export JSON to a file and inline.
    let j = c.call("export", json!({"format":"json"})).unwrap();
    assert_eq!(j["document"]["title"], "Demo");
    let jp = dir.join("deck.json");
    c.call("export", json!({"format":"json","path": jp})).unwrap();
    assert!(std::fs::read_to_string(&jp).unwrap().contains("Hello Zebra"));
    // A web page, to a file and inline.
    let hp = dir.join("deck.html");
    let page = c.call("export", json!({"format":"html","path": hp})).unwrap();
    assert_eq!((page["folder"].as_bool(), page["mime"].as_str()), (Some(false), Some("text/html")), "{page}");
    assert!(std::fs::read_to_string(&hp).unwrap().starts_with("<!doctype html>"));
    let page = c.call("export", json!({"format":"html"})).unwrap();
    assert!(page["data_base64"].as_str().unwrap().len() > 100_000, "player and document inline");
    let e = rpc_err(c.call("export", json!({"format":"html","path": hp, "folder": true})));
    assert!(e.message.contains("unknown field `folder`"), "{e}");

    // Images by path.
    let img = dir.join("dot.png");
    write_png(&img);
    let r = c.call("apply", json!({"command":{"op":"add_image","path": img,"x":0,"y":700,"width":100}})).unwrap();
    assert_eq!(r["created"].as_array().unwrap().len(), 1);

    // Helpful errors.
    let e = rpc_err(c.call("apply", json!({"command":{"op":"add_txet","text":"x"}})));
    assert_eq!(e.kind(), "invalid_params");
    assert!(e.message.contains("unknown variant"), "{e}");
    assert!(e.data["valid_values"].as_array().unwrap().iter().any(|v| v == "add_text"));
    let e = rpc_err(c.call("apply", json!({"command":{"op":"add_text"}})));
    assert!(e.message.contains("text"), "{e}");
    assert!(e.hint_text().unwrap().contains("add_text"), "{e}");
    let e = rpc_err(c.call("apply", json!({"command":{"op":"zoom_to","id":"intro"}})));
    assert!(e.hint_text().unwrap().contains("view"), "{e}");
    let e = rpc_err(c.call("apply", json!({"command":{"op":"delete","ids":["intr"]}})));
    assert_eq!(e.kind(), "not_found");
    assert!(e.data["valid_values"].as_array().unwrap().iter().any(|v| v == "intro"), "{e}");
    let e = rpc_err(c.call("frobnicate", json!({})));
    assert_eq!(e.code, -32601);
    assert!(e.data["valid_values"].as_array().unwrap().iter().any(|v| v == "apply"));
    let e = rpc_err(c.call("add_text", json!({})));
    assert!(e.hint_text().unwrap().contains("apply"), "{e}");
    let e = rpc_err(c.call("open", json!({"path":"relative.zebra"})));
    assert!(e.message.contains("absolute"));
    // Raw garbage gets a parse error, connection survives.
    let resp = c.raw(json!({"id": 99, "method": "ping"})).unwrap();
    assert_eq!(resp["result"], "pong");

    // Catalogue.
    let cat = c.call("commands", json!({})).unwrap();
    assert!(cat["commands"].as_array().unwrap().iter().any(|x| x["name"] == "add_frame" && x["example"].is_object()));
    let s = c.call("schema", json!({})).unwrap();
    assert!(s["command"]["oneOf"].is_array() || s["command"]["anyOf"].is_array());

    // Disconnect ends the loop and cleans up.
    let file_path = inst.file.clone();
    c.call("disconnect", json!({})).unwrap();
    for _ in 0..100 {
        if h.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(h.is_finished());
    assert!(!file_path.exists(), "discovery file removed");
    assert!(!h.socket.exists(), "socket removed");
}

#[test]
fn observe_permission_cannot_edit() {
    let h = start(Permission::Observe);
    let mut c = connect(&h, "observer");
    c.call("describe", json!({})).unwrap();
    let e = rpc_err(c.call("apply", json!({"command":{"op":"add_text","text":"x"}})));
    assert_eq!(e.kind(), "permission");
    let e = rpc_err(c.call("save", json!({"path":"/tmp/x.zebra"})));
    assert_eq!(e.kind(), "permission");
    h.stop().unwrap();
}

#[test]
fn edit_permission_uses_private_camera() {
    let h = start(Permission::Edit);
    let mut c = connect(&h, "editor");
    c.call("batch", json!({"commands":[{"op":"add_frame","id":"a","x":5000,"y":5000,"add_to_path":true}]})).unwrap();
    let before = c.call("state", json!({})).unwrap()["camera"].clone();
    c.call("view", json!({"command":{"op":"zoom_to","id":"a"}})).unwrap();
    let st = c.call("state", json!({})).unwrap();
    assert_eq!(st["camera"], before, "human camera untouched");
    assert_ne!(st["agent_camera"], before);
    let e = rpc_err(c.call("view", json!({"command":{"op":"present"}})));
    assert_eq!(e.kind(), "permission");
    h.stop().ok();
}

#[test]
fn paused_returns_paused() {
    // Pause is a human-only control; drive it through an in-process editor
    // with the real dispatcher, then talk over the socket.
    scratch();
    use std::sync::Arc;
    let waker: zebra_api::Waker = Arc::new(|| {});
    let server = zebra_api::ApiServer::start(ApiOptions::default(), waker).unwrap();
    let socket = server.socket_path();
    let token = server.token().to_string();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop2 = stop.clone();
    let host = std::thread::spawn(move || {
        let mut editor = zebra_core::Editor::default();
        let mut d = zebra_api::Dispatcher::new(Arc::new(zebra_render::Renderer::new()));
        editor.view(zebra_core::ViewCommand::PauseAgents { paused: true }, zebra_core::Origin::Human).unwrap();
        let mut saw_pause_event = false;
        while !stop2.load(std::sync::atomic::Ordering::SeqCst) {
            d.pump(&server, &mut editor);
            for e in d.take_events() {
                if matches!(e, zebra_api::Event::AgentsPaused { paused: true }) {
                    saw_pause_event = true;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(saw_pause_event, "host receives events via take_events");
        server.clients().len()
    });
    let mut c = Client::connect_hello(&socket, "pauser", &token).unwrap();
    let e = rpc_err(c.call("apply", json!({"command":{"op":"add_text","text":"x"}})));
    assert_eq!(e.kind(), "paused");
    assert!(e.hint_text().is_some());
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(host.join().unwrap(), 1, "clients() lists the connected agent");
}

fn write_png(p: &std::path::Path) {
    // 1x1 red PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01,
        0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41,
        0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    std::fs::write(p, PNG).unwrap();
}

/// In the app (not headless) captures render on a worker: the editor's
/// loop keeps running, and the reply arrives once the render is done.
#[test]
fn app_captures_render_off_the_ui_thread() {
    scratch();
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let woke = Arc::new(AtomicBool::new(false));
    let w2 = woke.clone();
    let server = zebra_api::ApiServer::start(ApiOptions { socket: None, permission: Permission::Full }, Arc::new(move || w2.store(true, Ordering::SeqCst))).unwrap();
    let socket = server.socket_path().to_path_buf();
    let token = server.token().to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let ui = std::thread::spawn(move || {
        let renderer = Arc::new(zebra_render::Renderer::new());
        let mut ed = zebra_core::Editor::default();
        ed.measure = renderer.measure();
        let mut d = zebra_api::Dispatcher::new(renderer);
        let mut frames = 0u64;
        while !stop2.load(Ordering::SeqCst) {
            d.pump(&server, &mut ed);
            frames += 1;
            std::thread::sleep(Duration::from_millis(2));
        }
        frames
    });
    let mut c = Client::connect_hello(&socket, "cap", &token).unwrap();
    c.call("apply", json!({"op":"add_frame","x":0,"y":0,"w":800,"h":600})).unwrap();
    let v = c.call("capture", json!({"target":"overview","width":640,"height":400})).unwrap();
    assert_eq!(v["width"], 640);
    assert!(v["data_base64"].as_str().unwrap().len() > 100);
    // Calls after a capture still run in order.
    let pdf = c.call("export", json!({"format":"pdf"})).unwrap();
    assert_eq!(pdf["mime"], "application/pdf");
    assert!(c.call("describe", json!({})).is_ok());
    stop.store(true, Ordering::SeqCst);
    assert!(ui.join().unwrap() > 0);
    assert!(woke.load(Ordering::SeqCst));
}

#[test]
fn a_labelled_apply_is_its_own_named_undo_step() {
    let h = start(Permission::Edit);
    let mut c = connect(&h, "labeller");
    c.call("apply", json!({"command":{"op":"add_text","text":"One","x":0,"y":0}})).unwrap();
    c.call("apply", json!({"label":"Rewrite the title","command":{"op":"add_text","text":"Two","x":0,"y":100}})).unwrap();
    let hist = c.call("query", json!({"query":{"q":"history"}})).unwrap();
    let labels: Vec<&str> = hist["undo"].as_array().unwrap().iter().map(|e| e["label"].as_str().unwrap()).collect();
    assert_eq!(labels.len(), 2, "never merged with the step before: {labels:?}");
    assert!(labels.contains(&"Rewrite the title"), "{labels:?}");
    let e = rpc_err(c.call("apply", json!({"label":5,"command":{"op":"add_text","text":"x","x":0,"y":0}})));
    assert!(e.message.contains("label"), "{e}");
}

/// A host with two open documents: "Pitch" in front and "Notes" behind.
struct TwoDocs {
    eds: Vec<(String, zebra_core::Editor)>,
}

impl zebra_api::Documents for TwoDocs {
    fn list(&self) -> Vec<zebra_api::DocInfo> {
        self.eds.iter().enumerate().map(|(i, (id, e))| zebra_api::doc_info(id, e, i == 0)).collect()
    }
    fn editor(&mut self, id: &str) -> Option<&mut zebra_core::Editor> {
        self.eds.iter_mut().find(|(i, _)| i == id).map(|(_, e)| e)
    }
}

#[test]
fn agents_list_documents_and_address_one_by_name_or_id() {
    scratch();
    use std::sync::Arc;
    let waker: zebra_api::Waker = Arc::new(|| {});
    let server = zebra_api::ApiServer::start(ApiOptions { socket: None, permission: Permission::Full }, waker).unwrap();
    let (socket, token) = (server.socket_path(), server.token().to_string());
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop2 = stop.clone();
    let host = std::thread::spawn(move || {
        let named = |t: &str| {
            let mut e = zebra_core::Editor::default();
            e.doc.title = t.into();
            e
        };
        let mut docs = TwoDocs { eds: vec![("d1".into(), named("Pitch")), ("d2".into(), named("Notes"))] };
        let mut d = zebra_api::Dispatcher::new(Arc::new(zebra_render::Renderer::new()));
        while !stop2.load(std::sync::atomic::Ordering::SeqCst) {
            d.pump_docs(&server, &mut docs);
            std::thread::sleep(Duration::from_millis(5));
        }
        docs.eds.iter().map(|(_, e)| e.doc.elements.len()).collect::<Vec<_>>()
    });
    let mut c = Client::connect_hello(&socket, "multi", &token).unwrap();
    let list = c.call("documents", json!({})).unwrap();
    let names: Vec<&str> = list["documents"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Pitch", "Notes"]);
    assert_eq!(list["documents"][0]["front"], true);

    // No document named: the front one.
    c.call("apply", json!({"command":{"op":"add_frame"}})).unwrap();
    // By name, and by id.
    c.call("apply", json!({"document":"Notes","command":{"op":"add_frame"}})).unwrap();
    c.call("apply", json!({"document":"d2","op":"add_frame"})).unwrap();
    let st = c.call("state", json!({"document":"notes"})).unwrap();
    assert_eq!(st["document"]["id"], "d2");
    let e = rpc_err(c.call("describe", json!({"document":"Nope"})));
    assert_eq!(e.kind(), "not_found");
    assert!(e.to_string().contains("Pitch"), "{e}");
    let e = rpc_err(c.call("state", json!({"document": 3})));
    assert_eq!(e.kind(), "invalid_params");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(host.join().unwrap(), vec![1, 2]);
}

#[test]
fn an_agent_must_read_again_after_someone_else_changed_the_document() {
    let h = start(Permission::Edit);
    let mut a = connect(&h, "alpha");
    let mut b = connect(&h, "beta");
    // A new, untouched document needs no read.
    a.call("apply", json!({"command":{"op":"add_text","id":"t1","text":"one"}})).unwrap();
    // beta never read it.
    let e = rpc_err(b.call("apply", json!({"command":{"op":"delete","ids":["t1"]}})));
    assert_eq!(e.kind(), "stale_revision");
    b.call("describe", json!({})).unwrap();
    // alpha's own writes keep it current; beta's read is now out of date.
    a.call("apply", json!({"command":{"op":"add_text","id":"t2","text":"two"}})).unwrap();
    a.call("apply", json!({"command":{"op":"add_text","id":"t3","text":"three"}})).unwrap();
    let e = rpc_err(b.call("apply", json!({"command":{"op":"delete","ids":["t1"]}})));
    assert_eq!(e.kind(), "stale_revision");
    assert!(e.message.contains("by alpha"), "{e}");
    // Any query counts as reading.
    b.call("query", json!({"query":{"q":"element","id":"t1"}})).unwrap();
    b.call("apply", json!({"command":{"op":"delete","ids":["t1"]}})).unwrap();
    h.stop().unwrap();
}

/// A host that can open and close documents, like the app.
struct Windows {
    eds: Vec<(String, zebra_core::Editor)>,
    next: u32,
}

impl zebra_api::Documents for Windows {
    fn list(&self) -> Vec<zebra_api::DocInfo> {
        self.eds.iter().enumerate().map(|(i, (id, e))| zebra_api::doc_info(id, e, i == 0)).collect()
    }
    fn editor(&mut self, id: &str) -> Option<&mut zebra_core::Editor> {
        self.eds.iter_mut().find(|(i, _)| i == id).map(|(_, e)| e)
    }
    fn new_document(&mut self) -> Result<String, zebra_api::RpcError> {
        let id = format!("d{}", self.next);
        self.next += 1;
        // Like the app: a new document takes the level the others have.
        let mut e = zebra_core::Editor::default();
        if let Some((_, first)) = self.eds.first() {
            e.view.agent_permission = first.view.agent_permission;
        }
        self.eds.push((id.clone(), e));
        Ok(id)
    }
    fn close(&mut self, id: &str, _discard: bool) -> Result<(), zebra_api::RpcError> {
        self.eds.retain(|(i, _)| i != id);
        Ok(())
    }
}

fn windows_host(perm: Permission) -> (PathBuf, String, std::sync::Arc<std::sync::atomic::AtomicBool>, std::thread::JoinHandle<Vec<(String, usize, bool)>>) {
    scratch();
    use std::sync::Arc;
    let waker: zebra_api::Waker = Arc::new(|| {});
    let server = zebra_api::ApiServer::start(ApiOptions { socket: None, permission: perm }, waker).unwrap();
    let (socket, token) = (server.socket_path().to_path_buf(), server.token().to_string());
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop2 = stop.clone();
    let host = std::thread::spawn(move || {
        let mut person = zebra_core::Editor::default();
        person.doc.title = "Pitch".into();
        person.apply(zebra_core::Command::AddFrame { style: Default::default(), x: 0.0, y: 0.0, w: 800.0, h: 600.0, rotation: 0.0, name: None, id: Some("f1".into()), add_to_path: false, find_space: false }, zebra_core::Origin::Human).unwrap();
        let mut docs = Windows { eds: vec![("d1".into(), person)], next: 2 };
        let mut d = zebra_api::Dispatcher::new(Arc::new(zebra_render::Renderer::new()));
        while !stop2.load(std::sync::atomic::Ordering::SeqCst) {
            d.pump_docs(&server, &mut docs);
            std::thread::sleep(Duration::from_millis(5));
        }
        docs.eds.iter().map(|(id, e)| (id.clone(), e.doc.elements.len(), e.dirty())).collect()
    });
    (socket, token, stop, host)
}

#[test]
fn an_agent_makes_a_document_copies_into_it_and_closes_it() {
    let (socket, token, stop, host) = windows_host(Permission::Edit);
    let mut c = Client::connect_hello(&socket, "builder", &token).unwrap();
    let made = c.call("new_document", json!({})).unwrap();
    let id = made["document"]["id"].as_str().unwrap().to_string();
    assert_eq!(id, "d2");
    assert_eq!(made["document"]["front"], false, "the person's document stays in front");

    // From the person's document into the new one, no clipboard involved.
    let out = c.call("copy_to", json!({"document":"Pitch","ids":["f1"],"to":id,"at":{"x":2000,"y":0}})).unwrap();
    assert_eq!(out["document"], "d2");
    let new_frame = out["created"][0].as_str().unwrap().to_string();
    let q = c.call("query", json!({"document":"d2","query":{"q":"element","id":new_frame}})).unwrap();
    assert!(q.to_string().contains("2000"), "{q}");
    let e = rpc_err(c.call("copy_to", json!({"ids":["nope"],"to":"d2"})));
    assert_eq!(e.kind(), "not_found");

    // Unsaved changes: closing is refused, and naming the document is required.
    let e = rpc_err(c.call("close_document", json!({})));
    assert_eq!(e.kind(), "invalid_params");
    let e = rpc_err(c.call("close_document", json!({"document":"d2"})));
    assert_eq!(e.kind(), "unsaved_changes");
    // Throwing changes away takes full permission.
    let e = rpc_err(c.call("close_document", json!({"document":"d2","discard":true})));
    assert_eq!(e.kind(), "permission");

    // An untouched new document closes without fuss.
    let blank = c.call("new_document", json!({})).unwrap()["document"]["id"].as_str().unwrap().to_string();
    c.call("close_document", json!({"document": blank})).unwrap();
    let list = c.call("documents", json!({})).unwrap();
    assert_eq!(list["documents"].as_array().unwrap().len(), 2);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let left = host.join().unwrap();
    assert_eq!(left, vec![("d1".to_string(), 1, true), ("d2".to_string(), 1, true)]);
}

#[test]
fn discarding_needs_a_read_of_the_latest_version() {
    let (socket, token, stop, host) = windows_host(Permission::Full);
    let mut c = Client::connect_hello(&socket, "closer", &token).unwrap();
    // The person's document has unsaved work this agent never read.
    let e = rpc_err(c.call("close_document", json!({"document":"d1","discard":true})));
    assert_eq!(e.kind(), "stale_revision", "{e}");
    c.call("describe", json!({"document":"d1"})).unwrap();
    c.call("close_document", json!({"document":"d1","discard":true})).unwrap();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(host.join().unwrap().is_empty());
}

#[test]
fn open_and_save_refuse_a_file_another_document_has_open() {
    let (socket, token, stop, host) = windows_host(Permission::Full);
    let dir = scratch().join("already-open");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("pitch.zebra");
    let _ = std::fs::remove_file(&file);
    let mut c = Client::connect_hello(&socket, "files", &token).unwrap();
    c.call("save", json!({"document":"d1","path":file})).unwrap();
    let id = c.call("new_document", json!({})).unwrap()["document"]["id"].as_str().unwrap().to_string();

    // Opening it in the second document: refused, naming the first.
    let e = rpc_err(c.call("open", json!({"document":id,"path":file})));
    assert_eq!(e.kind(), "already_open", "{e}");
    assert!(e.to_string().contains("\"d1\""), "{e}");
    // Another spelling of the same file too.
    let other_spelling = dir.join(".").join("pitch.zebra");
    let e = rpc_err(c.call("open", json!({"document":id,"path":other_spelling})));
    assert_eq!(e.kind(), "already_open", "{e}");
    // Saving over it from the second document: refused.
    c.call("apply", json!({"document":id,"command":{"op":"add_frame"}})).unwrap();
    let e = rpc_err(c.call("save", json!({"document":id,"path":file})));
    assert_eq!(e.kind(), "already_open", "{e}");
    // The document's own file, and another path, are fine.
    c.call("save", json!({"document":"d1"})).unwrap();
    c.call("save", json!({"document":id,"path":dir.join("notes.zebra")})).unwrap();
    c.call("open", json!({"document":"d1","path":file})).unwrap();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    host.join().unwrap();
}

fn sample_commands() -> Vec<zebra_core::Command> {
    vec![serde_json::from_value(json!({"op":"add_text","id":"hello","text":"From the sample","x":0,"y":0})).unwrap()]
}

#[test]
fn my_themes_and_my_templates_and_the_sample() {
    let data = scratch().join("data");
    let h = start(Permission::Full);
    let mut c = connect(&h, "librarian");
    c.call("describe", json!({})).unwrap();

    // save_theme with to_library: into the document and My Themes.
    let mut t = zebra_core::Theme::resolve("night");
    t.id = "lib-theme".into();
    t.name = "Library theme".into();
    let r = c.call("apply", json!({"command":{"op":"save_theme","theme":t,"to_library":true}})).unwrap();
    let file = std::path::PathBuf::from(r["library"][0].as_str().expect("the reply names the file"));
    assert!(file.starts_with(data.join("Themes")) && file.exists(), "{r}");
    // Gone from the document, it is still in My Themes, and set_theme brings it back.
    c.call("apply", json!({"command":{"op":"delete_theme","id":"lib-theme"}})).unwrap();
    let th = c.call("query", json!({"query":"themes"})).unwrap();
    assert!(th["library"].as_array().unwrap().iter().any(|l| l["id"] == "lib-theme"), "{th}");
    c.call("apply", json!({"command":{"op":"set_theme","theme":"lib-theme"}})).unwrap();
    let th = c.call("query", json!({"query":"themes"})).unwrap();
    assert_eq!(th["current"], "lib-theme");
    assert!(th["custom"].as_array().unwrap().iter().any(|t| t["id"] == "lib-theme"), "copied into the document: {th}");
    let e = rpc_err(c.call("apply", json!({"command":{"op":"set_theme","theme":"nope"}})));
    assert!(e.to_string().contains("lib-theme") || e.data["valid_values"].to_string().contains("lib-theme"), "{e}");

    // Save as Template, then start a document from it.
    c.call("apply", json!({"command":{"op":"add_frame","id":"tf","x":0,"y":0,"add_to_path":true}})).unwrap();
    let r = c.call("save_template", json!({"name":"Quarterly"})).unwrap();
    assert_eq!(r["template"], "saved:Quarterly");
    assert!(data.join("Templates/Quarterly.zebra").exists(), "{r}");
    let tl = c.call("query", json!({"query":"templates"})).unwrap();
    assert!(tl["saved"].as_array().unwrap().iter().any(|s| s["id"] == "saved:Quarterly"), "{tl}");
    c.call("apply", json!({"command":{"op":"delete","ids":["tf"]}})).unwrap();
    c.call("apply", json!({"command":{"op":"clear_path"}})).unwrap();
    c.call("apply", json!({"command":{"op":"add_text","id":"old","text":"old","x":0,"y":0}})).unwrap();
    let e = rpc_err(c.call("apply", json!({"command":{"op":"apply_template","template":"saved:Quarterly"}})));
    assert_eq!(e.kind(), "not_empty");
    c.call("apply", json!({"label":"From my template","command":{"op":"apply_template","template":"saved:Quarterly","replace":true}})).unwrap();
    let st = c.call("state", json!({})).unwrap();
    assert_eq!(st["path_length"], 1);
    assert!(c.call("query", json!({"query":{"q":"element","id":"tf"}})).is_ok());
    assert!(c.call("query", json!({"query":{"q":"element","id":"old"}})).is_err());
    // Not tied to the template's file: the document has no path.
    assert!(st["file"].is_null(), "{st}");
    // One undo step back.
    c.call("undo", json!({})).unwrap();
    assert!(c.call("query", json!({"query":{"q":"element","id":"old"}})).is_ok());
    let e = rpc_err(c.call("apply", json!({"command":{"op":"apply_template","template":"saved:Nope"}})));
    assert!(e.data["valid_values"].to_string().contains("saved:Quarterly"), "{e}");

    // The sample, where the host has one (the app's is in its language files).
    zebra_api::library::install(Some(sample_commands));
    assert_eq!(c.call("query", json!({"query":"templates"})).unwrap()["sample"], true);
    c.call("apply", json!({"command":{"op":"apply_template","template":"sample","replace":true}})).unwrap();
    assert!(c.call("query", json!({"query":{"q":"element","id":"hello"}})).is_ok());
    h.stop().unwrap();

    // Writing into the person's library takes full permission.
    let h = start(Permission::Edit);
    let mut c = connect(&h, "editor");
    c.call("describe", json!({})).unwrap();
    assert_eq!(rpc_err(c.call("save_template", json!({}))).kind(), "permission");
    let e = rpc_err(c.call("apply", json!({"command":{"op":"save_theme","theme":t,"to_library":true}})));
    assert_eq!(e.kind(), "permission");
    // Using them doesn't.
    c.call("apply", json!({"command":{"op":"apply_template","template":"saved:Quarterly"}})).unwrap();
    h.stop().unwrap();
}
