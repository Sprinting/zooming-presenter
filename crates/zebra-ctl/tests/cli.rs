//! Runs the real `zebractl` binary against an in-process headless instance.
use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;
use zebra_api::{ApiOptions, HeadlessHandle, Permission};

fn ctl(h: &HeadlessHandle, data: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_zebractl"))
        .args(args)
        .env("ZEBRA_DATA_DIR", data)
        .env("ZEBRA_TOKEN", &h.token)
        .arg("--socket")
        .arg(&h.socket)
        .output()
        .unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into())
}

#[test]
fn zebractl_end_to_end() {
    let dir = std::env::temp_dir().join(format!("zebractl-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let data = dir.join("data");
    // SAFETY: single-threaded at this point.
    unsafe { std::env::set_var("ZEBRA_DATA_DIR", &data) };
    let h = HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Full }, None).unwrap();

    let (ok, out, err) = ctl(&h, &data, &["add", "frame", "--id", "a", "--x", "-100", "--path", "--name", "Intro"]);
    assert!(ok, "{err}");
    assert!(out.contains("\"a\""), "{out}");
    // No --agent: it works, but says the name is shared.
    assert!(err.contains("--agent"), "{err}");
    // A named agent: a frame's --name and the agent's --agent don't clash,
    // and it must read what "zebractl" did before changing anything.
    let (ok, _, err) = ctl(&h, &data, &["--agent", "named", "add", "frame", "--id", "b", "--x", "900", "--name", "Two"]);
    assert!(!ok && err.contains("stale_revision"), "{err}");
    let (ok, _, _) = ctl(&h, &data, &["--agent", "named", "describe"]);
    assert!(ok);
    let (ok, _, err) = ctl(&h, &data, &["--agent", "named", "add", "frame", "--id", "b", "--x", "900", "--name", "Two"]);
    assert!(ok && !err.contains("--agent"), "{err}");
    let (ok, out, _) = ctl(&h, &data, &["describe"]);
    assert!(ok && out.contains("Two"), "{out}");
    let (ok, _, err) = ctl(&h, &data, &["add", "text", "Hello CLI", "--inside", "a", "--style", "title"]);
    assert!(ok, "{err}");
    let (ok, out, _) = ctl(&h, &data, &["describe"]);
    assert!(ok && out.contains("Hello CLI"), "{out}");
    // Calls go through one relay holding this agent's single connection.
    let relay = zebra_api::relay::socket_for(&h.socket, "zebractl");
    assert!(relay.exists(), "relay socket {}", relay.display());
    let (ok, out, _) = ctl(&h, &data, &["apply", r#"[{"op":"set_title","title":"T"},{"op":"auto_path"}]"#, "--label", "Two"]);
    assert!(ok, "{out}");
    let (ok, out, _) = ctl(&h, &data, &["query", "history"]);
    assert!(ok && out.contains("\"Two\""), "{out}");
    let (ok, _, err) = ctl(&h, &data, &["apply", r#"{"op":"add_txet"}"#]);
    assert!(!ok && err.contains("invalid_params") && err.contains("add_text"), "{err}");
    let shot = dir.join("s.png");
    let (ok, _, err) = ctl(&h, &data, &["screenshot", "--step", "0", "-o", shot.to_str().unwrap(), "--width", "160", "--height", "100"]);
    assert!(ok || err.contains("unimplemented"), "{err}");
    let (ok, out, err) = ctl(&h, &data, &["present", "0"]);
    assert!(ok, "{out}{err}");
    let (ok, out, _) = ctl(&h, &data, &["wait", "step", "0"]);
    assert!(ok, "{out}");
    let (ok, out, _) = ctl(&h, &data, &["state"]);
    let st: Value = serde_json::from_str(&out).unwrap();
    assert!(ok && st["mode"] == "present");
    let file = dir.join("d.zebra");
    let (ok, _, err) = ctl(&h, &data, &["save", file.to_str().unwrap()]);
    assert!(ok && file.exists(), "{err}");
    let (ok, out, _) = ctl(&h, &data, &["raw", r#"{"method":"ping"}"#]);
    assert!(ok && out.contains("pong"));
    let (ok, out, _) = ctl(&h, &data, &["commands", "--markdown"]);
    assert!(ok && out.contains("| `add_text` |"));

    // MCP over the real binary's stdio.
    let mut child = Command::new(env!("CARGO_BIN_EXE_zebractl"))
        .args(["mcp", "--socket"])
        .arg(&h.socket)
        .env("ZEBRA_TOKEN", &h.token)
        .env("ZEBRA_DATA_DIR", &data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut i = child.stdin.take().unwrap();
        writeln!(i, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18"}}}}"#).unwrap();
        writeln!(i, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
        writeln!(i, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"get_state","arguments":{{}}}}}}"#).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert_eq!(lines[1]["result"]["isError"], false, "{text}");
    assert!(lines[1]["result"]["content"][0]["text"].as_str().unwrap().contains("revision"));
    h.stop().unwrap();
}
