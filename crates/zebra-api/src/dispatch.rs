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
use crate::{ApiServer, Dispatcher, Documents};

/// How long a mutating call waits for a human gesture (drag, typing) to end
/// before failing with `busy`.
const HUMAN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
const MAX_HOST_EVENTS: usize = 10_000;

pub(crate) struct Pending {
    client: u64,
    id: Option<Value>,
    origin: Origin,
    call: Call,
    /// The `document` the call named (id or name); None = the front one.
    document: Option<String>,
    deadline: Instant,
}

/// The id of the document a call is for: the one it names (by id, then by
/// name or path), else the front one.
fn resolve(docs: &dyn Documents, want: Option<&str>) -> Result<String, RpcError> {
    let list = docs.list();
    let Some(w) = want else {
        return list.iter().find(|d| d.front).or(list.first()).map(|d| d.id.clone()).ok_or_else(|| RpcError::app("not_found", "no document is open"));
    };
    if let Some(d) = list.iter().find(|d| d.id == w) {
        return Ok(d.id.clone());
    }
    let mut hits: Vec<&crate::DocInfo> = list.iter().filter(|d| d.name == w || d.path.as_ref().is_some_and(|p| p.as_os_str() == w)).collect();
    if hits.is_empty() {
        hits = list.iter().filter(|d| d.name.eq_ignore_ascii_case(w)).collect();
    }
    match hits.as_slice() {
        [d] => Ok(d.id.clone()),
        [] => Err(RpcError::app("not_found", format!("no open document is called '{w}'"))
            .hint("call `documents` for the open documents; pass an id or a name")
            .values(list.iter().flat_map(|d| [d.id.clone(), d.name.clone()]).collect())),
        _ => Err(RpcError::invalid(format!("{} open documents are called '{w}'", hits.len()))
            .hint("pass the document's id instead")
            .values(hits.iter().map(|d| d.id.clone()).collect())),
    }
}

/// A fingerprint of everything a window draws that a call can change:
/// which documents are open, their revision, and their view (cameras,
/// selections, mode, step, agent status and marks).
fn shown(docs: &mut dyn Documents) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for d in docs.list() {
        (&d.id, &d.name, d.edited, d.front).hash(&mut h);
        if let Some(e) = docs.editor(&d.id) {
            let v = &e.view;
            e.revision.hash(&mut h);
            e.loads.hash(&mut h);
            e.flashes.len().hash(&mut h);
            e.in_transaction().hash(&mut h);
            format!(
                "{:?}",
                ((&v.camera, v.flight.is_some(), &v.selection, &v.agent_selection, &v.agent_camera, &v.mode, &v.tool), (&v.present, v.follow_agent, v.agent_paused, &v.agent_permission, &v.agent_status, &e.agent_cursor, e.agent_cursor_time.to_bits()))
            )
            .hash(&mut h);
        }
    }
    h.finish()
}

fn gesture(docs: &mut dyn Documents, want: Option<&str>) -> bool {
    resolve(docs, want).ok().and_then(|id| docs.editor(&id).map(|e| e.human_gesture_active())).unwrap_or(false)
}

impl Dispatcher {
    pub(crate) fn pump_impl(&mut self, server: &ApiServer, docs: &mut dyn Documents) -> bool {
        let inner = &server.inner;
        if !self.initialized {
            for d in docs.list() {
                if let Some(e) = docs.editor(&d.id) {
                    e.view.agent_permission = inner.permission;
                }
            }
            self.initialized = true;
        }
        // What the window shows, before: a call that only reads leaves it
        // as it was, and the window has nothing to redraw.
        let before = shown(docs);
        let mut handled = false;

        // 1. Calls deferred because a human gesture was in progress.
        let mut still = VecDeque::new();
        while let Some(p) = self.deferred.pop_front() {
            let blocked_client = still.iter().any(|q: &Pending| q.client == p.client) || self.jobs.iter().any(|j| j.client == p.client);
            if blocked_client || (gesture(docs, p.document.as_deref()) && Instant::now() < p.deadline) {
                still.push_back(p);
                continue;
            }
            self.run(server, docs, p);
        }
        self.deferred = still;

        // 2. New requests.
        loop {
            let msg = inner.rx.lock().unwrap().try_recv();
            let Ok(msg) = msg else { break };
            match msg {
                Inbound::Disconnected { client } => {
                    // The status bar names who is connected.
                    handled = true;
                    let owned: Vec<String> = self.tx_owner.iter().filter(|(_, c)| **c == client).map(|(d, _)| d.clone()).collect();
                    for d in owned {
                        self.tx_owner.remove(&d);
                        if let Some(editor) = docs.editor(&d) {
                            if editor.in_transaction() && !editor.human_gesture_active() {
                                editor.abort();
                            }
                        }
                    }
                    self.waits.retain(|w| w.client != client);
                    self.deferred.retain(|w| w.client != client);
                }
                Inbound::Call { client, id, origin, call, document } => {
                    let p = Pending { client, id, origin, call, document, deadline: Instant::now() + HUMAN_GRACE };
                    let must_queue = self.deferred.iter().any(|q| q.client == client)
                        || self.jobs.iter().any(|j| j.client == client)
                        || (p.call.mutates() && gesture(docs, p.document.as_deref()));
                    if must_queue {
                        self.deferred.push_back(p);
                    } else {
                        self.run(server, docs, p);
                    }
                }
            }
        }

        // 3. Renders and imports finished on workers.
        let mut j = 0;
        while j < self.jobs.len() {
            match self.jobs[j].rx.try_recv() {
                Ok(JobDone::Reply(r)) => {
                    let job = self.jobs.remove(j);
                    reply(server, job.client, &job.id, r);
                }
                Ok(JobDone::Import(done)) => {
                    let job = self.jobs.remove(j);
                    let r = self.finish_import(docs, *done);
                    self.flush_events(server, docs);
                    reply(server, job.client, &job.id, r);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => j += 1,
                Err(_) => {
                    let job = self.jobs.remove(j);
                    reply(server, job.client, &job.id, Err(RpcError::app("render_failed", "the render thread stopped")));
                }
            }
        }

        // 3b. Saves.
        self.pump_saves(server, docs);

        // 4. Waits.
        let now = Instant::now();
        let mut i = 0;
        while i < self.waits.len() {
            let w = &self.waits[i];
            let state = docs.editor(&w.doc).map(|e| (condition_holds(e, &w.until), e));
            let r = match state {
                None => Some(Err(RpcError::app("not_found", "the document was closed while waiting"))),
                Some((true, e)) => Some(Ok(wait_result(e))),
                Some((false, _)) if now >= w.deadline => Some(Err(RpcError::app("timeout", format!("timed out waiting for {:?}", w.until))
                    .hint("increase timeout_ms, or check `state`"))),
                Some((false, _)) => None,
            };
            match r {
                Some(r) => {
                    let w = self.waits.remove(i);
                    reply(server, w.client, &w.id, r);
                }
                None => i += 1,
            }
        }

        // 5. Events nobody has drained yet (e.g. human edits).
        self.flush_events(server, docs);

        inner.shared.poll.store(!self.waits.is_empty() || !self.deferred.is_empty() || !self.jobs.is_empty() || !self.saves.is_empty(), std::sync::atomic::Ordering::SeqCst);
        handled || self.disconnect_requested || self.quit_requested || shown(docs) != before
    }

    fn flush_events(&mut self, server: &ApiServer, docs: &mut dyn Documents) {
        let several = docs.list().len() > 1;
        for d in docs.list() {
            let Some(editor) = docs.editor(&d.id) else { continue };
            for e in editor.drain_events() {
                let mut v = serde_json::to_value(&e).unwrap_or(Value::Null);
                let name = v.get("event").and_then(|n| n.as_str()).unwrap_or("").to_string();
                if several {
                    if let Some(o) = v.as_object_mut() {
                        o.insert("document".into(), json!(d.id));
                    }
                }
                server.inner.shared.broadcast_event(&name, &v);
                self.events.push(e);
            }
        }
        if self.events.len() > MAX_HOST_EVENTS {
            let extra = self.events.len() - MAX_HOST_EVENTS;
            self.events.drain(..extra);
        }
    }

    fn run(&mut self, server: &ApiServer, docs: &mut dyn Documents, p: Pending) {
        if let Call::Documents = &p.call {
            reply(server, p.client, &p.id, Ok(json!({ "documents": docs.list() })));
            return;
        }
        if let Call::NewDocument = &p.call {
            let r = self.new_document(docs, &p.origin);
            self.flush_events(server, docs);
            return reply(server, p.client, &p.id, r);
        }
        if let Call::CloseDocument { discard } = &p.call {
            let r = match p.document.as_deref() {
                None => Err(RpcError::invalid("`close_document` needs \"document\": the id or name of the document to close")
                    .hint("call `documents` for the open documents")),
                Some(want) => resolve(docs, Some(want)).and_then(|doc| self.close_document(docs, &doc, &p.origin, *discard)),
            };
            self.flush_events(server, docs);
            return reply(server, p.client, &p.id, r);
        }
        if let Call::CopyTo { .. } = &p.call {
            let r = self.copy_to(docs, p.document.as_deref(), &p.origin, p.call);
            self.flush_events(server, docs);
            return reply(server, p.client, &p.id, r);
        }
        let doc = match resolve(docs, p.document.as_deref()) {
            Ok(d) => d,
            Err(e) => return reply(server, p.client, &p.id, Err(e)),
        };
        let info = docs.list().into_iter().find(|d| d.id == doc);
        let Some(editor) = docs.editor(&doc) else {
            return reply(server, p.client, &p.id, Err(RpcError::app("not_found", "that document was just closed")));
        };
        if let Call::Wait { until, timeout } = &p.call {
            if condition_holds(editor, until) {
                reply(server, p.client, &p.id, Ok(wait_result(editor)));
            } else {
                self.waits.push(Wait { client: p.client, id: p.id, doc, until: until.clone(), deadline: Instant::now() + *timeout });
            }
            return;
        }
        if let Call::Import { .. } = &p.call {
            let seen = self.seen.get(&(doc.clone(), p.origin.display())).copied();
            let allowed = need_edit(editor.view.agent_permission, "import")
                .and_then(|_| check_agent_allowed(editor))
                .and_then(|_| if p.origin.is_agent() { check_read(editor, seen) } else { Ok(()) });
            if let Err(e) = allowed {
                return reply(server, p.client, &p.id, Err(e));
            }
            let Call::Import { path, first, last } = p.call else { unreachable!() };
            let theme = editor.theme();
            let (tx, rx) = std::sync::mpsc::channel();
            let waker = server.inner.shared.waker.clone();
            let (doc, origin) = (doc.clone(), p.origin.clone());
            // Reading a big file takes a while: off the window's thread.
            let spawned = std::thread::Builder::new().name("zebra-import".into()).spawn(move || {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let deck = zebra_import::open(&path).and_then(|r| {
                    let mut o = zebra_import::Options::all(theme);
                    o.first = first.unwrap_or(1);
                    o.last = last.unwrap_or(usize::MAX);
                    let pages = r.page_count();
                    if o.first > pages {
                        return Err(zebra_import::ImportError::Read(format!("the file has {pages} pages; `from` is {}", o.first)));
                    }
                    r.read(&o, &zebra_import::Control::default())
                });
                let _ = tx.send(JobDone::Import(Box::new(ImportDone { doc, origin, name, deck })));
                waker();
            });
            match spawned {
                Ok(_) => self.jobs.push(Job { client: p.client, id: p.id, rx }),
                Err(e) => reply(server, p.client, &p.id, Err(RpcError::app("io", format!("cannot start an import thread: {e}")))),
            }
            return;
        }
        // Captures and exports render a snapshot on a worker so the window
        // never stalls on a big render; the reply is sent when it's done.
        // Writing a file is more than looking.
        let to_file = match &p.call {
            Call::Capture(req) => req.path.is_some(),
            Call::Export { path, .. } => path.is_some(),
            _ => false,
        };
        if to_file {
            if let Err(e) = need_edit(editor.view.agent_permission, "capture or export to a file") {
                return reply(server, p.client, &p.id, Err(e.hint("leave `path` out to get the data in the reply")));
            }
        }
        // A save writes a snapshot on a worker (the window stays
        // responsive); the reply goes when it is done. One at a time per
        // document: a save while another runs waits for it.
        if let Call::Save { path } = &p.call {
            match save_path(editor, path.clone()) {
                Ok(path) => {
                    self.saves.push(SaveWait { client: p.client, id: p.id, doc, origin: p.origin, path, started: false });
                    self.pump_saves(server, docs);
                }
                Err(e) => reply(server, p.client, &p.id, Err(e)),
            }
            return;
        }
        let heavy = matches!(&p.call, Call::Capture(_))
            || matches!(&p.call, Call::Export { format: ExportFormat::Png | ExportFormat::Pdf | ExportFormat::Html, .. });
        if heavy && !self.headless {
            let limited = editor.view.agent_permission != Permission::Full;
            let mut snap = Editor::new(editor.doc.clone());
            snap.view = editor.view.clone();
            snap.measure = self.renderer.measure();
            let renderer = self.renderer.clone();
            let labels = self.web_labels.clone();
            let call = p.call;
            let (tx, rx) = std::sync::mpsc::channel();
            let waker = server.inner.shared.waker.clone();
            let spawned = std::thread::Builder::new().name("zebra-capture".into()).spawn(move || {
                let r = match call {
                    Call::Capture(req) => Self::capture(&renderer, &snap, &req, limited),
                    Call::Export { format, path, capture } => Self::export(&renderer, &snap, format, path, &capture, limited, &labels),
                    _ => unreachable!(),
                };
                let _ = tx.send(JobDone::Reply(r));
                waker();
            });
            match spawned {
                Ok(_) => self.jobs.push(Job { client: p.client, id: p.id, rx }),
                Err(e) => reply(server, p.client, &p.id, Err(RpcError::app("io", format!("cannot start a render thread: {e}")))),
            }
            return;
        }
        let is_state = matches!(p.call, Call::State);
        let mut r = self.execute(server, editor, &doc, p.client, &p.origin, p.call);
        // `state` says which document it describes.
        if let (true, Ok(Value::Object(o)), Some(info)) = (is_state, &mut r, info) {
            o.insert("document".into(), json!(info));
        }
        // Events caused by this call go out before its reply.
        self.flush_events(server, docs);
        reply(server, p.client, &p.id, r);
    }

    fn new_document(&mut self, docs: &mut dyn Documents, origin: &Origin) -> Result<Value, RpcError> {
        if let Some(e) = docs.any_editor() {
            need_edit(e.view.agent_permission, "new_document")?;
            if e.view.agent_paused {
                return Err(RpcError::app("paused", "the human paused agents").hint("wait for the agents_paused event"));
            }
        }
        let id = docs.new_document()?;
        // A document you made yourself counts as read.
        if let Some(e) = docs.editor(&id) {
            self.seen.insert((id.clone(), origin.display()), e.revision);
        }
        let info = docs.list().into_iter().find(|d| d.id == id);
        Ok(json!({ "document": info, "hint": format!("pass \"document\": \"{id}\" on your calls to work in it") }))
    }

    /// Never drops the person's work by accident: unsaved changes need
    /// `discard`, full permission, and a read of the latest version.
    fn close_document(&mut self, docs: &mut dyn Documents, doc: &str, origin: &Origin, discard: bool) -> Result<Value, RpcError> {
        let key = (doc.to_string(), origin.display());
        let seen = self.seen.get(&key).copied();
        let Some(editor) = docs.editor(doc) else { return Err(RpcError::app("not_found", "that document was just closed")) };
        need_edit(editor.view.agent_permission, "close_document")?;
        check_agent_allowed(editor)?;
        if editor.dirty() {
            if !discard {
                return Err(RpcError::app("unsaved_changes", "the document has unsaved changes")
                    .hint("save it first (`save`, needs a path if it was never saved), or pass \"discard\": true to throw the changes away"));
            }
            need_full(editor.view.agent_permission, "close_document with discard")?;
            if origin.is_agent() {
                check_read(editor, seen)?;
            }
        }
        if docs.asking(doc) {
            return Err(RpcError::app("busy", "Zebra is asking the person something about this document").hint("wait for them to answer, then try again"));
        }
        docs.close(doc, discard)?;
        self.seen.remove(&key);
        Ok(json!({ "ok": true, "closed": doc }))
    }

    fn copy_to(&mut self, docs: &mut dyn Documents, from: Option<&str>, origin: &Origin, call: Call) -> Result<Value, RpcError> {
        let Call::CopyTo { ids, to, at, offset } = call else { unreachable!() };
        let src = resolve(docs, from)?;
        let dst = resolve(docs, Some(&to))?;
        let clip = {
            let Some(e) = docs.editor(&src) else { return Err(RpcError::app("not_found", "that document was just closed")) };
            if let Some(missing) = ids.iter().find(|id| e.doc.get(id).is_none()) {
                return Err(RpcError::app("not_found", format!("no element '{missing}' in the document you copy from"))
                    .hint("`query` {\"q\":\"elements\"} lists them; pass \"document\" for the one to copy from"));
            }
            zebra_core::clip::Clip::of(e, &ids)
        };
        let key = (dst.clone(), origin.display());
        let seen = self.seen.get(&key).copied();
        let Some(editor) = docs.editor(&dst) else { return Err(RpcError::app("not_found", "that document was just closed")) };
        need_edit(editor.view.agent_permission, "copy_to")?;
        check_agent_allowed(editor)?;
        if origin.is_agent() {
            check_read(editor, seen)?;
        }
        let (dx, dy) = match (at, clip.bounds()) {
            (Some((x, y)), Some(b)) => (x - b.center().x, y - b.center().y),
            _ => offset,
        };
        let label = format!("Copy from {}", docs.list().into_iter().find(|d| d.id == src).map(|d| d.name).unwrap_or(src.clone()));
        let editor = docs.editor(&dst).expect("checked above");
        let out = editor.paste_clip(&clip, dx, dy, Some(label), origin.clone())?;
        self.seen.insert(key, editor.revision);
        let mut v = to_value(out);
        if let Value::Object(o) = &mut v {
            o.insert("document".into(), json!(dst));
        }
        Ok(v)
    }

    fn execute(&mut self, server: &ApiServer, editor: &mut Editor, doc: &str, client: u64, origin: &Origin, call: Call) -> Result<Value, RpcError> {
        // Read before write: an agent may change the document only if it
        // has read it since anyone else last changed it.
        let key = (doc.to_string(), origin.display());
        // Reading it, or opening one yourself, counts as having read it.
        let reads = matches!(call, Call::Describe | Call::Query(_) | Call::Open { .. });
        let writes = matches!(call, Call::Apply { .. } | Call::Batch { .. } | Call::Begin { .. } | Call::Undo | Call::Redo);
        if writes && origin.is_agent() {
            check_read(editor, self.seen.get(&key).copied())?;
        }
        let r = self.execute_inner(server, editor, doc, client, origin, call);
        if r.is_ok() && (reads || writes) {
            self.seen.insert(key, editor.revision);
        }
        r
    }

    fn execute_inner(&mut self, _server: &ApiServer, editor: &mut Editor, doc: &str, client: u64, origin: &Origin, call: Call) -> Result<Value, RpcError> {
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
                if let Some(&owner) = self.tx_owner.get(doc) {
                    if owner != client && editor.in_transaction() {
                        return Err(RpcError::app("busy", "another client has a transaction open").hint("retry after it commits"));
                    }
                }
                editor.begin(label, origin.clone())?;
                self.tx_owner.insert(doc.to_string(), client);
                json!({ "ok": true, "revision": editor.revision })
            }
            Call::Commit | Call::Abort => {
                let is_commit = matches!(call, Call::Commit);
                // Only the client that opened it, and only its own: never
                // another client's, never one the person's drag has open.
                if self.tx_owner.get(doc) != Some(&client) || editor.transaction_owner() != Some(origin) {
                    if self.tx_owner.get(doc) == Some(&client) {
                        self.tx_owner.remove(doc);
                    }
                    return Err(RpcError::app("no_transaction", "you have no open transaction").hint("call `begin {\"label\": ...}` first"));
                }
                if is_commit {
                    editor.commit_as(origin);
                } else {
                    editor.abort_as(origin);
                }
                self.tx_owner.remove(doc);
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
                // (Saves are run by `run`, on a worker; this is the direct
                // route for hosts that call `execute` themselves.)
                let path = save_path(editor, path)?;
                zebra_core::io::save(&mut editor.doc, &path)?;
                editor.mark_saved(Some(path.clone()));
                json!({ "ok": true, "path": path })
            }
            Call::Export { format, path, capture } => Self::export(&self.renderer, editor, format, path, &capture, limited, &self.web_labels)?,
            Call::Wait { .. } | Call::Documents | Call::NewDocument | Call::CloseDocument { .. } | Call::CopyTo { .. } | Call::Import { .. } => {
                unreachable!("handled in run")
            }
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
        // Unless told otherwise, the shape of the person's window, so a step
        // shows exactly what they will see when presenting.
        let vp = editor.view.viewport;
        let aspect = if vp.x > 1.0 && vp.y > 1.0 { vp.y / vp.x } else { 0.625 };
        let (w, h) = match (req.width, req.height) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, (w as f64 * aspect).round().max(16.0) as u32),
            (None, Some(h)) => ((h as f64 / aspect).round().max(16.0) as u32, h),
            (None, None) => {
                let s = (1600.0 / vp.x.max(vp.y).max(1.0)).min(1.0);
                if vp.x > 1.0 && vp.y > 1.0 { ((vp.x * s).round() as u32, (vp.y * s).round() as u32) } else { (1280, 800) }
            }
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

    fn export(r: &zebra_render::Renderer, editor: &Editor, format: ExportFormat, path: Option<std::path::PathBuf>, cap: &CaptureReq, limited: bool, labels: &zebra_web::export::Labels) -> Result<Value, RpcError> {
        let (bytes, mime) = match format {
            ExportFormat::Html => {
                let Some(p) = path else {
                    let page = zebra_web::export::build(r, editor, labels, zebra_web::export::INLINE_LIMIT)?;
                    if page.is_folder() {
                        return Err(RpcError::app("too_big", "this page keeps its video and audio in files beside it").hint("give a `path`; a folder is written there"));
                    }
                    return Ok(json!({ "mime": "text/html", "bytes": page.html.len(), "notes": page.notes,
                                      "data_base64": base64::engine::general_purpose::STANDARD.encode(page.html.as_bytes()) }));
                };
                let w = zebra_web::export::export(r, editor, labels, &p)?;
                return Ok(json!({ "path": w.page, "folder": w.folder, "bytes": w.bytes, "mime": "text/html", "notes": w.notes }));
            }
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

/// A capture, export or import running on a worker thread.
pub(crate) struct Job {
    client: u64,
    id: Option<Value>,
    rx: std::sync::mpsc::Receiver<JobDone>,
}

pub(crate) enum JobDone {
    /// The reply, as is.
    Reply(Result<Value, RpcError>),
    /// A file read for `import`, to add to its document.
    Import(Box<ImportDone>),
}

pub(crate) struct ImportDone {
    doc: String,
    origin: Origin,
    /// The file's name, for the undo step.
    name: String,
    deck: Result<zebra_import::ImportedDeck, zebra_import::ImportError>,
}

fn import_error(e: zebra_import::ImportError) -> RpcError {
    use zebra_import::ImportError as E;
    let code = match &e {
        E::NotSupported => "unsupported",
        E::Damaged(_) => "damaged",
        E::Encrypted => "encrypted",
        E::NoEngine(_) => "no_engine",
        E::Read(_) => "io",
        E::Cancelled => "cancelled",
    };
    let r = RpcError::app(code, e.to_string());
    match e {
        E::NotSupported => r.hint(format!("Zebra imports: {}", zebra_import::EXTENSIONS.join(", "))),
        _ => r,
    }
}

impl Dispatcher {
    /// Add an imported file to its document (still open?) as one undo step.
    fn finish_import(&mut self, docs: &mut dyn Documents, done: ImportDone) -> Result<Value, RpcError> {
        let deck = done.deck.map_err(import_error)?;
        let key = (done.doc.clone(), done.origin.display());
        let seen = self.seen.get(&key).copied();
        let Some(editor) = docs.editor(&done.doc) else { return Err(RpcError::app("not_found", "the document was closed during the import")) };
        need_edit(editor.view.agent_permission, "import")?;
        check_agent_allowed(editor)?;
        if done.origin.is_agent() {
            // Changed by someone else while the file was read: their
            // change could be what the pages now overlap.
            check_read(editor, seen)?;
        }
        let out = editor.import_deck(&deck, format!("Import {}", done.name), done.origin.clone())?;
        self.seen.insert(key, editor.revision);
        let frames: Vec<String> = out.created.iter().filter(|id| editor.doc.get(id).is_some_and(|e| e.is_frame())).cloned().collect();
        let notes: Vec<Value> = deck.notes.iter().map(|(n, c)| json!({ "note": n, "count": c, "text": n.english(*c) })).collect();
        let mut v = to_value(out);
        if let Value::Object(o) = &mut v {
            o.insert("pages".into(), json!(deck.pages.len()));
            o.insert("frames".into(), json!(frames));
            o.insert("notes".into(), json!(notes));
        }
        Ok(v)
    }
}

/// An agent's save: waiting for another save of the document to finish,
/// or running.
pub(crate) struct SaveWait {
    client: u64,
    id: Option<Value>,
    doc: String,
    origin: Origin,
    path: std::path::PathBuf,
    started: bool,
}

impl Dispatcher {
    /// Start saves that can start; reply to those that finished.
    fn pump_saves(&mut self, server: &ApiServer, docs: &mut dyn Documents) {
        let mut i = 0;
        while i < self.saves.len() {
            let w = &mut self.saves[i];
            let r: Option<Result<Value, RpcError>> = match docs.editor(&w.doc) {
                None => Some(Err(RpcError::app("not_found", "the document was closed while saving"))),
                Some(editor) => {
                    editor.poll_save();
                    let mut r = None;
                    if !w.started {
                        let waker = server.inner.shared.waker.clone();
                        match editor.start_save(w.path.clone(), w.origin.clone(), Box::new(move || waker())) {
                            Ok(()) => w.started = true,
                            // Another save of the document is running.
                            Err(e) if e.code == "busy" => {}
                            Err(e) => r = Some(Err(e.into())),
                        }
                    }
                    if w.started {
                        let (origin, path) = (&w.origin, &w.path);
                        r = editor.take_saved(|s| s.by == *origin && s.path == *path).pop().map(|s| s.result.map(|()| json!({ "ok": true, "path": path })).map_err(RpcError::from));
                    }
                    r
                }
            };
            match r {
                Some(r) => {
                    let w = self.saves.remove(i);
                    reply(server, w.client, &w.id, r);
                }
                None => i += 1,
            }
        }
    }
}

pub(crate) struct Wait {
    client: u64,
    id: Option<Value>,
    /// The document whose state is awaited.
    doc: String,
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

/// Has the document changed since this agent last read or wrote it (at
/// revision `seen`)? A brand-new, untouched document needs no read.
fn check_read(editor: &Editor, seen: Option<u64>) -> Result<(), RpcError> {
    let now = editor.revision;
    let Some(seen) = seen else {
        if now == 0 {
            return Ok(());
        }
        return Err(RpcError::app("stale_revision", format!("you haven't read this document yet (it is at revision {now})"))
            .hint("read it first (`describe`, or `query` for what you need), then make your change"));
    };
    if seen == now {
        return Ok(());
    }
    // Who changed it since: the person and/or other agents.
    let mut who: Vec<String> = vec![];
    for h in editor.history().iter().filter(|h| h.revision > seen) {
        let n = if h.origin.is_agent() { h.origin.display() } else { "the person".to_string() };
        if !who.contains(&n) {
            who.push(n);
        }
    }
    let by = if who.is_empty() { String::new() } else { format!(" by {}", who.join(", ")) };
    Err(RpcError::app("stale_revision", format!("the document changed{by} since you last read it (you read revision {seen}; it is now at {now})"))
        .hint("read it again (`describe`, or `query` for the parts you are changing), then retry"))
}

/// Where `save` writes: the path given, else the document's own file.
fn save_path(editor: &Editor, path: Option<std::path::PathBuf>) -> Result<std::path::PathBuf, RpcError> {
    need_full(editor.view.agent_permission, "save")?;
    let path = path.or_else(|| editor.file_path.clone()).ok_or_else(|| {
        RpcError::app("no_path", "the document has never been saved").hint("pass {\"path\": \"/abs/name.zebra\"}")
    })?;
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
        // JSON has no pictures or video in it: writing it is an export,
        // and the document is not saved by it.
        return Err(RpcError::invalid("`save` writes a .zebra file; a .json file would leave the images and media out")
            .hint("use export {\"format\": \"json\", \"path\": ...} for the JSON, and save to a .zebra path"));
    }
    Ok(path)
}

fn need_full(perm: Permission, what: &str) -> Result<(), RpcError> {
    if perm != Permission::Full {
        return Err(RpcError::app("permission", format!("`{what}` needs 'full' agent permission"))
            .hint("the person can grant it from the AI button in Zebra's toolbar; `export` works without it"));
    }
    Ok(())
}

fn need_edit(perm: Permission, what: &str) -> Result<(), RpcError> {
    if perm == Permission::Observe {
        return Err(RpcError::app("permission", format!("`{what}` needs 'edit' agent permission"))
            .hint("the person can grant it from the AI button in Zebra's toolbar"));
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
