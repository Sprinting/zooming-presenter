//! Regression tests for the API-side bugs of the adversarial review
//! (18 Nov 2026).
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use serde_json::{Value, json};
use zebra_api::client::{Client, ClientError};
use zebra_api::{ApiOptions, HeadlessHandle, Permission};
use zebra_core::Origin;

fn scratch() -> PathBuf {
    static INIT: Once = Once::new();
    let dir = std::env::temp_dir().join(format!("zebra-api-review-{}", std::process::id()));
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

/// An in-process host whose editor the test can also touch, as the person
/// would. Returns how many pumps asked for a redraw.
struct Host {
    socket: PathBuf,
    token: String,
    editor: Arc<Mutex<zebra_core::Editor>>,
    redraws: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Host {
    fn start(perm: Permission) -> Host {
        scratch();
        let waker: zebra_api::Waker = Arc::new(|| {});
        let server = zebra_api::ApiServer::start(ApiOptions { socket: None, permission: perm }, waker).unwrap();
        let (socket, token) = (server.socket_path(), server.token().to_string());
        let editor = Arc::new(Mutex::new(zebra_core::Editor::default()));
        let redraws = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (e2, r2, s2) = (editor.clone(), redraws.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            let mut d = zebra_api::Dispatcher::new(Arc::new(zebra_render::Renderer::new()));
            while !s2.load(Ordering::SeqCst) {
                if d.pump(&server, &mut e2.lock().unwrap()) {
                    r2.fetch_add(1, Ordering::SeqCst);
                }
                let _ = d.take_events();
                std::thread::sleep(Duration::from_millis(3));
            }
        });
        Host { socket, token, editor, redraws, stop, thread: Some(thread) }
    }
    fn connect(&self, name: &str) -> Client {
        Client::connect_hello(&self.socket, name, &self.token).expect("hello")
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Speed fix 2: a call that only reads asks for no redraw.
#[test]
fn reads_do_not_redraw_the_window() {
    let host = Host::start(Permission::Edit);
    let mut c = host.connect("reader");
    c.call("describe", json!({})).unwrap();
    c.call("apply", json!({"command":{"op":"add_text","text":"x"}})).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    let before = host.redraws.load(Ordering::SeqCst);
    assert!(before >= 1, "an edit redraws");
    for _ in 0..5 {
        c.call("describe", json!({})).unwrap();
        c.call("state", json!({})).unwrap();
        c.call("query", json!({"query":"path"})).unwrap();
        c.call("documents", json!({})).unwrap();
    }
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(host.redraws.load(Ordering::SeqCst), before, "20 reads, no redraw");
    c.call("view", json!({"command":{"op":"select","ids":[]}})).ok();
    c.call("apply", json!({"command":{"op":"add_text","text":"y"}})).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert!(host.redraws.load(Ordering::SeqCst) > before);
}

/// R6: looking is not writing files.
#[test]
fn an_observer_cannot_write_files() {
    let dir = scratch();
    let h = start(Permission::Observe);
    let mut c = connect(&h, "observer");
    for (method, params) in [
        ("capture", json!({"path": dir.join("r6.png")})),
        ("export", json!({"format":"json","path": dir.join("r6.json")})),
        ("export", json!({"format":"zebra","path": dir.join("r6.zebra")})),
    ] {
        let e = rpc_err(c.call(method, params));
        assert_eq!(e.kind(), "permission", "{method}");
    }
    assert!(!dir.join("r6.png").exists() && !dir.join("r6.json").exists() && !dir.join("r6.zebra").exists());
    // Without a path the data comes back in the reply, as before.
    let v = c.call("export", json!({"format":"json"})).unwrap();
    assert!(v["document"].is_object());
    h.stop().unwrap();
}

/// R4: `save` to a .json path wrote the document without its pictures
/// and called it saved.
#[test]
fn save_does_not_write_json_and_call_it_saved() {
    let dir = scratch();
    let h = start(Permission::Full);
    let mut c = connect(&h, "saver");
    c.call("describe", json!({})).unwrap();
    c.call("apply", json!({"command":{"op":"add_text","text":"x"}})).unwrap();
    let e = rpc_err(c.call("save", json!({"path": dir.join("r4.json")})));
    assert_eq!(e.kind(), "invalid_params");
    assert!(e.hint_text().unwrap().contains("export"));
    assert_eq!(c.call("state", json!({})).unwrap()["dirty"], true);
    c.call("save", json!({"path": dir.join("r4.zebra")})).unwrap();
    assert_eq!(c.call("state", json!({})).unwrap()["dirty"], false);
    h.stop().unwrap();
}

/// R28: a commit from a client that has no transaction left the real
/// owner's transaction without an owner.
#[test]
fn one_agents_stray_commit_leaves_the_others_transaction_alone() {
    let host = Host::start(Permission::Edit);
    let (mut a, mut b) = (host.connect("a"), host.connect("b"));
    a.call("describe", json!({})).unwrap();
    a.call("begin", json!({"label":"a's batch"})).unwrap();
    a.call("apply", json!({"command":{"op":"add_text","text":"one"}})).unwrap();
    assert_eq!(rpc_err(b.call("commit", json!({}))).kind(), "no_transaction");
    assert_eq!(rpc_err(b.call("abort", json!({}))).kind(), "no_transaction");
    assert!(host.editor.lock().unwrap().in_transaction());
    a.call("apply", json!({"command":{"op":"add_text","text":"two"}})).unwrap();
    a.call("commit", json!({})).unwrap();
    let e = host.editor.lock().unwrap();
    assert!(!e.in_transaction());
    assert_eq!(e.history().len(), 1, "one step, as a asked");
}

/// D3 and R23 through the API: an agent's undo, commit or abort never
/// touches the drag the person is in the middle of.
#[test]
fn an_agent_cannot_end_the_persons_drag() {
    let host = Host::start(Permission::Edit);
    let mut c = host.connect("agent");
    c.call("describe", json!({})).unwrap();
    let id = c.call("apply", json!({"command":{"op":"add_shape","shape":"rectangle","x":0,"y":0,"w":100,"h":100}})).unwrap()["created"][0].as_str().unwrap().to_string();
    {
        let mut e = host.editor.lock().unwrap();
        e.begin("Move", Origin::Human).unwrap();
        e.apply(zebra_core::Command::Move { ids: vec![id.as_str().into()], dx: 50.0, dy: 0.0, with_contents: true }, Origin::Human).unwrap();
    }
    for m in ["commit", "abort"] {
        assert_eq!(rpc_err(c.call(m, json!({}))).kind(), "no_transaction", "{m}");
    }
    let mut e = host.editor.lock().unwrap();
    assert!(e.in_transaction(), "the drag is still the person's");
    assert_eq!(e.transaction_owner(), Some(&Origin::Human));
    e.commit_as(&Origin::Human);
    assert_eq!(e.doc.get(&zebra_core::ElementId::from(id.as_str())).unwrap().transform.x, 50.0);
}

/// `zebractl raw '[1]'` panicked the client; the relay shares the code.
#[test]
fn a_raw_message_that_is_not_an_object_is_an_error_not_a_panic() {
    let h = start(Permission::Edit);
    let mut c = connect(&h, "raw");
    for v in [json!([1]), json!(1), json!("x"), Value::Null] {
        assert_eq!(rpc_err(c.raw(v)).kind(), "invalid_params");
    }
    assert_eq!(c.call("ping", json!({})).unwrap(), "pong");
    h.stop().unwrap();
}

/// R14: "You" is the person in the history and the status bar.
#[test]
fn an_agent_cannot_call_itself_you() {
    let h = start(Permission::Edit);
    for (given, shown) in [("You", "agent \"You\""), (" you ", "agent \"you\""), ("", "agent"), ("a\nb", "ab"), ("Claude", "Claude")] {
        let mut c = Client::connect(&h.socket).unwrap();
        let v = c.call("hello", json!({"name": given, "token": h.token})).unwrap();
        assert_eq!(v["name"], shown, "{given:?}");
    }
    h.stop().unwrap();
}

/// R14: a subscriber that never reads is disconnected once its queue is
/// large, instead of the queue growing without end.
#[test]
fn a_subscriber_that_never_reads_is_dropped() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let host = Host::start(Permission::Edit);
    // A raw connection: hello, subscribe, then never read again.
    let mut deaf = UnixStream::connect(&host.socket).unwrap();
    writeln!(deaf, "{}", json!({"jsonrpc":"2.0","id":1,"method":"hello","params":{"name":"deaf","token":host.token}})).unwrap();
    writeln!(deaf, "{}", json!({"jsonrpc":"2.0","id":2,"method":"subscribe","params":{}})).unwrap();
    let mut c = host.connect("writer");
    c.call("describe", json!({})).unwrap();
    // Each event carries a long label; a few hundred MB would pile up.
    let label = "x".repeat(256 * 1024);
    let mut gone = false;
    for i in 0..400 {
        c.call("batch", json!({"label": label, "commands":[{"op":"add_text","text":format!("t{i}")}]})).unwrap();
        if i > 80 && c.call("state", json!({})).is_ok() {
            // The deaf client's socket has been shut: a write to it fails.
            if writeln!(deaf, "{}", json!({"jsonrpc":"2.0","id":3,"method":"ping"})).is_err() {
                gone = true;
                break;
            }
        }
    }
    assert!(gone, "the subscriber that stopped reading was disconnected");
    // The one that reads is untouched.
    assert_eq!(c.call("ping", json!({})).unwrap(), "pong");
}

/// Hostile-input audit L2: replies count toward the same cap. A client
/// that asks for big replies and never reads them is disconnected, instead
/// of its queue growing for as long as Zebra runs.
#[test]
fn a_client_that_never_reads_its_replies_is_dropped() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let host = Host::start(Permission::Edit);
    let mut c = host.connect("writer");
    c.call("describe", json!({})).unwrap();
    c.call("apply", json!({"command":{"op":"add_text","id":"big","text":"y".repeat(1 << 20)}})).unwrap();
    let mut deaf = UnixStream::connect(&host.socket).unwrap();
    writeln!(deaf, "{}", json!({"jsonrpc":"2.0","id":1,"method":"hello","params":{"name":"deaf","token":host.token}})).unwrap();
    let mut gone = false;
    for i in 0..200 {
        // Each reply is over 1 MB.
        if writeln!(deaf, "{}", json!({"jsonrpc":"2.0","id":i + 2,"method":"query","params":{"q":"element","id":"big"}})).is_err() {
            gone = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(gone, "the client that stopped reading was disconnected");
    assert_eq!(c.call("ping", json!({})).unwrap(), "pong");
}

/// R28: nothing in a request is acted on before `hello`. `add_image
/// {path}` used to open the file first, so anyone who could reach the
/// socket could learn which files exist.
#[test]
fn r28_no_file_is_touched_before_hello() {
    let h = start(Permission::Edit);
    let mut anon = Client::connect(&h.socket).unwrap();
    for path in ["/definitely/not/here.png", "/etc/hosts"] {
        let e = rpc_err(anon.call("apply", json!({ "command": { "op": "add_image", "path": path } })));
        assert_eq!(e.kind(), "unauthenticated", "{e}");
        assert!(!e.message.contains(path), "{e}");
    }
    let e = rpc_err(anon.call("no_such_method", json!({})));
    assert_eq!(e.kind(), "unauthenticated", "{e}");
}

/// Saves run on a worker: two agents saving at once both get their
/// reply (the second waits for the first), and a save that fails says so
/// to the agent that asked.
#[test]
fn saves_run_aside_one_at_a_time() {
    let dir = scratch();
    let h = start(Permission::Full);
    let mut a = connect(&h, "saver-a");
    let mut b = connect(&h, "saver-b");
    a.call("describe", json!({})).unwrap();
    a.call("apply", json!({"command":{"op":"add_text","text":"x"}})).unwrap();
    let (pa, pb) = (dir.join("both-a.zebra"), dir.join("both-b.zebra"));
    let pb2 = pb.clone();
    let t = std::thread::spawn(move || b.call("save", json!({"path": pb2})).map(|v| v["ok"].clone()));
    assert_eq!(a.call("save", json!({"path": pa})).unwrap()["ok"], true);
    assert_eq!(t.join().unwrap().unwrap(), true);
    assert!(pa.exists() && pb.exists());
    assert_eq!(a.call("state", json!({})).unwrap()["dirty"], false);
    let e = rpc_err(a.call("save", json!({"path": "/nonexistent-dir-for-zebra/x.zebra"})));
    assert_eq!(e.kind(), "io", "{e:?}");
    h.stop().unwrap();
}
