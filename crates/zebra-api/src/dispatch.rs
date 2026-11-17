//! The Dispatcher runs on the thread that owns the `Editor`.
use std::collections::VecDeque;
use std::path::Path;
use std::time::Instant;

use base64::Engine;
use serde_json::{Value, json};
use zebra_core::editor::Event;
use zebra_core::view::Permission;
use zebra_core::{Editor, Mode, Origin, Query, ZError};
use zebra_render::CaptureTarget;

use crate::protocol::{self, Call, CaptureReq, ExportFormat, RpcError, WaitUntil};
use crate::server::Inbound;
use crate::{ApiServer, Dispatcher};

/// How long a mutating call waits for a human gesture (drag, typing) to end
/// before failing with `busy`.
const HUMAN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
const MAX_HOST_EVENTS: usize = 10_000;

pub(crate) struct Pending {
    client: u64,
    id: Option<Value>,
    origin: Origin,
    call: Call,
    deadline: Instant,
}

impl Dispatcher {
    pub(crate) fn pump_impl(&mut self, server: &ApiServer, editor: &mut Editor) -> bool {
        let inner = &server.inner;
        if !self.initialized {
            self.initialized = true;
            editor.view.agent_permission = inner.permission;
        }
        let mut handled = false;

        // 1. Calls deferred because a human gesture was in progress.
        let mut still = VecDeque::new();
        while let Some(p) = self.deferred.pop_front() {
            let blocked_client = still.iter().any(|q: &Pending| q.client == p.client) || self.jobs.iter().any(|j| j.client == p.client);
            if blocked_client || (editor.human_gesture_active() && Instant::now() < p.deadline) {
                still.push_back(p);
                continue;
            }
            handled = true;
            self.run(server, editor, p);
        }
        self.deferred = still;

        // 2. New requests.
        loop {
            let msg = inner.rx.lock().unwrap().try_recv();
            let Ok(msg) = msg else { break };
            handled = true;
            match msg {
                Inbound::Disconnected { client } => {
                    if self.tx_owner == Some(client) {
                        self.tx_owner = None;
                        if editor.in_transaction() && !editor.human_gesture_active() {
                            editor.abort();
                        }
                    }
                    self.waits.retain(|w| w.client != client);
                    self.deferred.retain(|w| w.client != client);
                }
                Inbound::Call { client, id, origin, call } => {
                    let p = Pending { client, id, origin, call, deadline: Instant::now() + HUMAN_GRACE };
                    let must_queue = self.deferred.iter().any(|q| q.client == client)
                        || self.jobs.iter().any(|j| j.client == client)
                        || (p.call.mutates() && editor.human_gesture_active());
                    if must_queue {
                        self.deferred.push_back(p);
                    } else {
                        self.run(server, editor, p);
                    }
                }
            }
        }

        // 3. Renders finished on workers.
        let mut j = 0;
        while j < self.jobs.len() {
            match self.jobs[j].rx.try_recv() {
                Ok(r) => {
                    let job = self.jobs.remove(j);
                    reply(server, job.client, &job.id, r);
                    handled = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => j += 1,
                Err(_) => {
                    let job = self.jobs.remove(j);
                    reply(server, job.client, &job.id, Err(RpcError::app("render_failed", "the render thread stopped")));
                }
            }
        }

        // 4. Waits.
        let now = Instant::now();
        let mut i = 0;
        while i < self.waits.len() {
            let w = &self.waits[i];
            let done = condition_holds(editor, &w.until);
            if done || now >= w.deadline {
                let w = self.waits.remove(i);
                let r = if done {
                    Ok(wait_result(editor))
                } else {
                    Err(RpcError::app("timeout", format!("timed out waiting for {:?}", w.until))
                        .hint("increase timeout_ms, or check `state`"))
                };
                reply(server, w.client, &w.id, r);
                handled = true;
            } else {
                i += 1;
            }
        }

        // 5. Events nobody has drained yet (e.g. human edits).
        self.flush_events(server, editor);

        inner.shared.poll.store(!self.waits.is_empty() || !self.deferred.is_empty() || !self.jobs.is_empty(), std::sync::atomic::Ordering::SeqCst);
        handled
    }

    fn flush_events(&mut self, server: &ApiServer, editor: &mut Editor) {
        let evs = editor.drain_events();
        for e in evs {
            let v = serde_json::to_value(&e).unwrap_or(Value::Null);
            let name = v.get("event").and_then(|n| n.as_str()).unwrap_or("").to_string();
            server.inner.shared.broadcast_event(&name, &v);
            self.events.push(e);
        }
        if self.events.len() > MAX_HOST_EVENTS {
            let extra = self.events.len() - MAX_HOST_EVENTS;
            self.events.drain(..extra);
        }
    }

    fn run(&mut self, server: &ApiServer, editor: &mut Editor, p: Pending) {
        if let Call::Wait { until, timeout } = &p.call {
            if condition_holds(editor, until) {
                reply(server, p.client, &p.id, Ok(wait_result(editor)));
            } else {
                self.waits.push(Wait { client: p.client, id: p.id, until: until.clone(), deadline: Instant::now() + *timeout });
            }
            return;
        }
        // Captures and exports render a snapshot on a worker so the window
        // never stalls on a big render; the reply is sent when it's done.
        let heavy = matches!(&p.call, Call::Capture(_))
            || matches!(&p.call, Call::Export { format: ExportFormat::Png | ExportFormat::Pdf, .. });
        if heavy && !self.headless {
            let limited = editor.view.agent_permission != Permission::Full;
            let mut snap = Editor::new(editor.doc.clone());
            snap.view = editor.view.clone();
            snap.measure = self.renderer.measure();
            let renderer = self.renderer.clone();
            let call = p.call;
            let (tx, rx) = std::sync::mpsc::channel();
            let waker = server.inner.shared.waker.clone();
            let spawned = std::thread::Builder::new().name("zebra-capture".into()).spawn(move || {
                let r = match call {
                    Call::Capture(req) => Self::capture(&renderer, &snap, &req, limited),
                    Call::Export { format, path, capture } => Self::export(&renderer, &snap, format, path, &capture, limited),
                    _ => unreachable!(),
                };
                let _ = tx.send(r);
                waker();
            });
            match spawned {
                Ok(_) => self.jobs.push(Job { client: p.client, id: p.id, rx }),
                Err(e) => reply(server, p.client, &p.id, Err(RpcError::app("io", format!("cannot start a render thread: {e}")))),
            }
            return;
        }
        let r = self.execute(server, editor, p.client, &p.origin, p.call);
        // Events caused by this call go out before its reply.
        self.flush_events(server, editor);
        reply(server, p.client, &p.id, r);
    }

    fn execute(&mut self, _server: &ApiServer, editor: &mut Editor, client: u64, origin: &Origin, call: Call) -> Result<Value, RpcError> {
        let perm = editor.view.agent_permission;
        let limited = perm != Permission::Full;
        Ok(match call {
            Call::Apply { command, if_revision } => {
                check_rev(editor, if_revision)?;
                to_value(editor.apply(command, origin.clone())?)
            }
            Call::Batch { commands, label, if_revision } => {
                check_rev(editor, if_revision)?;
                if commands.is_empty() {
                    return Err(RpcError::invalid("`commands` is empty"));
                }
                to_value(editor.apply_batch(commands, label, origin.clone())?)
            }
            Call::Begin { label } => {
                if let Some(owner) = self.tx_owner {
                    if owner != client && editor.in_transaction() {
                        return Err(RpcError::app("busy", "another client has a transaction open").hint("retry after it commits"));
                    }
                }
                editor.begin(label, origin.clone())?;
                self.tx_owner = Some(client);
                json!({ "ok": true, "revision": editor.revision })
            }
            Call::Commit | Call::Abort => {
                let is_commit = matches!(call, Call::Commit);
                if self.tx_owner != Some(client) || !editor.in_transaction() {
                    self.tx_owner = None;
                    return Err(RpcError::app("no_transaction", "you have no open transaction").hint("call `begin {\"label\": ...}` first"));
                }
                if is_commit {
                    editor.commit();
                } else {
                    editor.abort();
                }
                self.tx_owner = None;
                json!({ "ok": true, "revision": editor.revision })
            }
            Call::View(cmd) => editor.view(cmd, origin.clone())?,
            Call::Query(q) => editor.query(q)?,
            Call::Describe => editor.query(Query::Describe)?,
            Call::State => editor.query(Query::State)?,
            Call::Undo => json!({ "undone": editor.undo(origin.clone())?, "revision": editor.revision }),
            Call::Redo => json!({ "redone": editor.redo(origin.clone())?, "revision": editor.revision }),
            Call::Capture(req) => Self::capture(&self.renderer, editor, &req, limited)?,
            Call::Open { path, force } => {
                need_full(perm, "open")?;
                check_agent_allowed(editor)?;
                if editor.dirty() && !force {
                    return Err(RpcError::app("unsaved_changes", "the current document has unsaved changes")
                        .hint("save first, or pass \"force\": true to discard them"));
                }
                let doc = zebra_core::io::load(&path)?;
                editor.load(doc, Some(path.clone()));
                json!({ "ok": true, "path": path, "revision": editor.revision })
            }
            Call::Save { path } => {
                need_full(perm, "save")?;
                let path = path.or_else(|| editor.file_path.clone()).ok_or_else(|| {
                    RpcError::app("no_path", "the document has never been saved").hint("pass {\"path\": \"/abs/name.zebra\"}")
                })?;
                write_document(editor, &path)?;
                editor.mark_saved(Some(path.clone()));
                json!({ "ok": true, "path": path })
            }
            Call::Export { format, path, capture } => Self::export(&self.renderer, editor, format, path, &capture, limited)?,
            Call::Wait { .. } => unreachable!("handled in run"),
            Call::Disconnect => {
                // Any agent may switch AI control off; only the human can turn
                // it back on (there is deliberately no API to enable it).
                self.disconnect_requested = true;
                if self.headless {
                    self.quit_requested = true;
                }
                json!({ "ok": true, "message": "AI control is now off. Only the person using Zebra can turn it back on." })
            }
            Call::Quit => {
                if !self.headless {
                    return Err(RpcError::app("permission", "quit is only available for headless instances")
                        .hint("the human closes the window"));
                }
                self.quit_requested = true;
                json!({ "ok": true })
            }
        })
    }

    fn render(r: &zebra_render::Renderer, editor: &Editor, req: &CaptureReq, limited: bool) -> Result<(Vec<u8>, u32, u32, CaptureTarget), RpcError> {
        let target = req.target.clone().unwrap_or(if limited { CaptureTarget::AgentView } else { CaptureTarget::Viewport });
        let (w, h) = match (req.width, req.height) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, (w as f64 * 0.625).round().max(16.0) as u32),
            (None, Some(h)) => ((h as f64 * 1.6).round().max(16.0) as u32, h),
            (None, None) => (1280, 800),
        };
        if let CaptureTarget::Element { id } = &target {
            if editor.doc.get(id).is_none() {
                return Err(ZError::not_found(id, editor.doc.elements.keys().cloned()).into());
            }
        }
        if let CaptureTarget::Step { index, .. } = &target {
            if *index >= editor.doc.path.len() {
                return Err(RpcError::invalid(format!("step {index} does not exist; the path has {} steps (0-based)", editor.doc.path.len())));
            }
        }
        let png = r.capture(editor, &target, w, h)?;
        Ok((png, w, h, target))
    }

    fn capture(r: &zebra_render::Renderer, editor: &Editor, req: &CaptureReq, limited: bool) -> Result<Value, RpcError> {
        let (png, w, h, target) = Self::render(r, editor, req, limited)?;
        let tgt = serde_json::to_value(&target).unwrap_or(Value::Null);
        if let Some(p) = &req.path {
            write_file(p, &png)?;
            Ok(json!({ "path": p, "bytes": png.len(), "width": w, "height": h, "target": tgt }))
        } else {
            Ok(json!({ "format": "png", "width": w, "height": h, "target": tgt,
                        "data_base64": base64::engine::general_purpose::STANDARD.encode(&png) }))
        }
    }

    fn export(r: &zebra_render::Renderer, editor: &Editor, format: ExportFormat, path: Option<std::path::PathBuf>, cap: &CaptureReq, limited: bool) -> Result<Value, RpcError> {
        let (bytes, mime) = match format {
            ExportFormat::Png => {
                let mut cap = cap.clone();
                if cap.target.is_none() {
                    cap.target = Some(CaptureTarget::Overview);
                }
                cap.path = None;
                (Self::render(r, editor, &cap, limited)?.0, "image/png")
            }
            ExportFormat::Pdf => (r.export_pdf(editor)?, "application/pdf"),
            ExportFormat::Json => {
                let json = zebra_core::io::to_json(&editor.doc);
                if path.is_none() {
                    return Ok(json!({ "format": "json", "document": serde_json::from_str::<Value>(&json).unwrap_or(Value::Null) }));
                }
                (json.into_bytes(), "application/json")
            }
            ExportFormat::Zebra => (zebra_core::io::save_bytes(&editor.doc)?, "application/zip"),
        };
        match path {
            Some(p) => {
                write_file(&p, &bytes)?;
                Ok(json!({ "path": p, "bytes": bytes.len(), "mime": mime }))
            }
            None => Ok(json!({ "mime": mime, "bytes": bytes.len(),
                               "data_base64": base64::engine::general_purpose::STANDARD.encode(&bytes) })),
        }
    }
}

/// A capture or export rendering on a worker thread.
pub(crate) struct Job {
    client: u64,
    id: Option<Value>,
    rx: std::sync::mpsc::Receiver<Result<Value, RpcError>>,
}

pub(crate) struct Wait {
    client: u64,
    id: Option<Value>,
    until: WaitUntil,
    deadline: Instant,
}

fn reply(server: &ApiServer, client: u64, id: &Option<Value>, r: Result<Value, RpcError>) {
    let Some(id) = id else { return };
    let line = match r {
        Ok(v) => protocol::response_ok(id, v),
        Err(e) => protocol::response_err(id, &e),
    };
    server.inner.shared.send(client, line);
}

fn to_value<T: serde::Serialize>(t: T) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

fn check_rev(editor: &Editor, r: Option<u64>) -> Result<(), RpcError> {
    match r {
        Some(r) if r != editor.revision => Err(RpcError::app(
            "stale_revision",
            format!("document is at revision {}, not {r}", editor.revision),
        )
        .hint("someone changed the document; re-read it (`describe`) and retry with the current revision")),
        _ => Ok(()),
    }
}

fn need_full(perm: Permission, what: &str) -> Result<(), RpcError> {
    if perm != Permission::Full {
        return Err(RpcError::app("permission", format!("`{what}` needs 'full' agent permission"))
            .hint("the human can grant it from the agent status pill, or launch with --agent-permission full; `export` works without it"));
    }
    Ok(())
}

fn check_agent_allowed(editor: &Editor) -> Result<(), RpcError> {
    if editor.view.agent_paused {
        return Err(RpcError::app("paused", "the human paused agents").hint("wait for the agents_paused event"));
    }
    if editor.human_gesture_active() {
        return Err(RpcError::app("busy", "the human is in the middle of an edit").hint("retry in a moment"));
    }
    Ok(())
}

fn write_file(p: &Path, bytes: &[u8]) -> Result<(), RpcError> {
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(RpcError::app("io", format!("directory {} does not exist", parent.display())));
        }
    }
    std::fs::write(p, bytes).map_err(|e| RpcError::app("io", format!("{}: {e}", p.display())))
}

fn write_document(editor: &mut Editor, path: &Path) -> Result<(), RpcError> {
    if path.extension().is_some_and(|e| e == "json") {
        write_file(path, zebra_core::io::to_json(&editor.doc).as_bytes())
    } else {
        zebra_core::io::save(&mut editor.doc, path).map_err(Into::into)
    }
}

fn condition_holds(editor: &Editor, until: &WaitUntil) -> bool {
    let idle = editor.view.flight.is_none() && !editor.human_gesture_active();
    match until {
        WaitUntil::Idle => idle,
        WaitUntil::Step(n) => editor.view.mode == Mode::Present && editor.view.present.step == Some(*n) && editor.view.flight.is_none(),
        WaitUntil::Revision(r) => editor.revision >= *r,
        WaitUntil::Mode(m) => editor.view.mode == *m,
    }
}

fn wait_result(editor: &Editor) -> Value {
    json!({
        "ok": true,
        "revision": editor.revision,
        "mode": editor.view.mode,
        "step": editor.view.present.step,
        "build": editor.view.present.build,
        "camera": editor.view.camera,
    })
}

/// Event names, for `subscribe {events}` validation.
pub const EVENT_NAMES: &[&str] =
    &["document_changed", "selection_changed", "mode_changed", "step_changed", "agents_paused", "saved"];

pub(crate) fn _assert_event_names(e: &Event) -> &'static str {
    // Compile-time reminder: keep EVENT_NAMES in sync with zebra_core::editor::Event.
    match e {
        Event::DocumentChanged { .. } => EVENT_NAMES[0],
        Event::SelectionChanged { .. } => EVENT_NAMES[1],
        Event::ModeChanged { .. } => EVENT_NAMES[2],
        Event::StepChanged { .. } => EVENT_NAMES[3],
        Event::AgentsPaused { .. } => EVENT_NAMES[4],
        Event::Saved { .. } => EVENT_NAMES[5],
    }
}
