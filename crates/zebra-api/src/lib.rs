//! Zebra automation API: "drive Zebra like Playwright drives Chrome".
//!
//! CONTRACT used by `zebra-app` (keep these signatures stable):
//!
//! ```ignore
//! let server = ApiServer::start(ApiOptions::default(), waker)?;  // background thread(s)
//! // on the UI thread, every frame (and whenever `waker` fires):
//! dispatcher.pump(&server, &mut editor);                         // applies queued requests
//! for ev in dispatcher.take_events() { /* update UI from events */ }
//! ```
//! Headless: `zebra --headless --api` calls [`run_headless`].
//!
//! Wire protocol: JSON-RPC 2.0, one JSON object per line, over a Unix domain
//! socket (mode 0600, parent dir 0700). Discovery file per running instance
//! (see [`discovery`]). The first call on a connection must be
//! `hello {"name", "token"}`; the name becomes `Origin::Agent{name}`.
//!
//! ## Events and the host
//! When the API is enabled, the [`Dispatcher`] is the one that calls
//! `editor.drain_events()` (it must, to forward events to subscribers).
//! Every drained event, whoever caused it (human or agent), is also kept for
//! the host: call [`Dispatcher::take_events`] after `pump` each frame instead
//! of draining the editor yourself. (Up to 10 000 are buffered if the host
//! never takes them.) If the host calls `editor.drain_events()` itself before
//! `pump`, subscribers simply miss those events.
//!
//! ## Threads
//! The socket threads never touch the `Editor`. They authenticate, answer
//! editor-free methods (`ping`, `commands`, `schema`, `subscribe`), parse
//! params into typed calls (reading `add_image {path}` files there), and
//! queue them; `waker` is called so the UI thread runs `pump` promptly.
//! While a `wait` or a call deferred by a human gesture is pending, the
//! server keeps calling `waker` every ~30 ms.

use std::path::PathBuf;
use std::sync::Arc;

pub use zebra_core::view::Permission;

pub mod catalogue;
pub mod client;
pub mod discovery;
mod dispatch;
mod headless;
pub mod mcp;
pub mod protocol;
pub mod relay;
mod server;
pub mod strict;

pub use dispatch::EVENT_NAMES;
pub use protocol::RpcError;
pub use zebra_core::editor::Event;

#[derive(Debug, Clone)]
pub struct ApiOptions {
    /// Socket path; default: per-user temp dir + pid.
    pub socket: Option<PathBuf>,
    /// Initial agent permission. Applied to `editor.view.agent_permission`
    /// on the first `pump`; the human can change it later from the UI.
    pub permission: Permission,
}

impl Default for ApiOptions {
    fn default() -> Self {
        ApiOptions { socket: None, permission: Permission::Edit }
    }
}

/// A connected (authenticated) client, for the agent status bar.
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub id: u64,
    pub name: String,
}

/// Wakes the UI thread when a request arrives (eframe: `ctx.request_repaint()`).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// The listening socket plus its discovery file. Dropping it closes the
/// socket, disconnects clients and removes the discovery file.
pub struct ApiServer {
    pub(crate) inner: server::ServerInner,
}

impl ApiServer {
    pub fn start(opts: ApiOptions, waker: Waker) -> anyhow::Result<ApiServer> {
        Ok(ApiServer { inner: server::start(opts, waker, false)? })
    }
    pub(crate) fn start_headless(opts: ApiOptions, waker: Waker) -> anyhow::Result<ApiServer> {
        Ok(ApiServer { inner: server::start(opts, waker, true)? })
    }
    pub fn socket_path(&self) -> PathBuf {
        self.inner.socket.clone()
    }
    /// Authenticated clients, oldest first.
    pub fn clients(&self) -> Vec<ClientInfo> {
        self.inner.clients()
    }
    /// The auth token (also in the discovery file).
    pub fn token(&self) -> &str {
        &self.inner.token
    }
    /// The discovery file, if it could be written.
    pub fn discovery_file(&self) -> Option<PathBuf> {
        self.inner.discovery_file.clone()
    }
}

/// Runs on the thread that owns the `Editor` (main thread in the app).
/// One open document, as agents see it in `documents`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DocInfo {
    /// Stable for as long as the document is open ("d1", "d2", ...).
    pub id: String,
    /// The file name without extension, or the title of an unsaved one.
    pub name: String,
    pub path: Option<PathBuf>,
    /// The document the person is looking at; calls without `document` go here.
    pub front: bool,
    /// Unsaved changes.
    pub edited: bool,
}

/// The host's open documents.
pub trait Documents {
    /// Every open document, front one included.
    fn list(&self) -> Vec<DocInfo>;
    /// The editor of the document with this id.
    fn editor(&mut self, id: &str) -> Option<&mut zebra_core::Editor>;
    /// Some document's editor, for settings every document shares (agent
    /// permission, paused), even when no document is open.
    fn any_editor(&mut self) -> Option<&mut zebra_core::Editor> {
        let id = self.list().first()?.id.clone();
        self.editor(&id)
    }
    /// Open a new, empty document in a window of its own; its id.
    fn new_document(&mut self) -> Result<String, RpcError> {
        Err(RpcError::app("unsupported", "this Zebra has a single document").hint("run the Zebra app for several documents"))
    }
    /// A question to the person about this document is on screen (Save
    /// Changes and the like): it can't close under them.
    fn asking(&self, _id: &str) -> bool {
        false
    }
    /// Close the document (the caller has dealt with unsaved changes):
    /// `discard` says they are to be thrown away.
    fn close(&mut self, _id: &str, _discard: bool) -> Result<(), RpcError> {
        Err(RpcError::app("unsupported", "this Zebra has a single document").hint("run the Zebra app for several documents"))
    }
}

/// A host with a single document.
pub struct One<'a>(pub &'a mut zebra_core::Editor);

impl Documents for One<'_> {
    fn list(&self) -> Vec<DocInfo> {
        vec![doc_info("d1", self.0, true)]
    }
    fn editor(&mut self, id: &str) -> Option<&mut zebra_core::Editor> {
        (id == "d1").then_some(&mut *self.0)
    }
}

/// [`DocInfo`] for `editor`, named after its file or title.
pub fn doc_info(id: &str, editor: &zebra_core::Editor, front: bool) -> DocInfo {
    let name = editor
        .file_path
        .as_ref()
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| editor.doc.title.clone());
    DocInfo { id: id.to_string(), name, path: editor.file_path.clone(), front, edited: editor.dirty() }
}

pub struct Dispatcher {
    pub renderer: Arc<zebra_render::Renderer>,
    /// Words of an exported web page's control bar; the app sets them in
    /// the person's language.
    pub web_labels: zebra_web::export::Labels,
    /// Set by [`run_headless`]: enables the `quit` method.
    pub headless: bool,
    /// Set when a client called `quit` (headless only).
    pub quit_requested: bool,
    /// Set when a client called `disconnect`: the host should switch AI
    /// control off (drop the [`ApiServer`]) once replies are flushed.
    pub disconnect_requested: bool,
    events: Vec<Event>,
    /// The server's starting permission has been given to the documents.
    /// Once only: after that the level is the person's to change, and a
    /// document opened later takes the level the others have (the host
    /// copies it), not the one the server started with.
    initialized: bool,
    /// Who has a transaction open, per document id.
    tx_owner: std::collections::HashMap<String, u64>,
    /// The revision each agent last read or wrote, per (document, agent):
    /// an agent may change a document only if nobody else has since.
    seen: std::collections::HashMap<(String, String), u64>,
    waits: Vec<dispatch::Wait>,
    deferred: std::collections::VecDeque<dispatch::Pending>,
    jobs: Vec<dispatch::Job>,
    saves: Vec<dispatch::SaveWait>,
}

impl Dispatcher {
    /// Call before dropping the server when AI control is switched off:
    /// rolls back a transaction an agent left open.
    pub fn shutdown(&mut self, editor: &mut zebra_core::editor::Editor) {
        self.shutdown_docs(&mut One(editor));
    }
    /// [`Dispatcher::shutdown`] for every open document.
    pub fn shutdown_docs(&mut self, docs: &mut dyn Documents) {
        for (id, _) in std::mem::take(&mut self.tx_owner) {
            if let Some(e) = docs.editor(&id) {
                if e.in_transaction() {
                    e.abort();
                }
            }
        }
        self.waits.clear();
        self.deferred.clear();
        // A save already running finishes (the host polls it); nobody is
        // left to tell.
        self.saves.clear();
    }
    pub fn new(renderer: Arc<zebra_render::Renderer>) -> Self {
        Dispatcher {
            renderer,
            web_labels: Default::default(),
            headless: false,
            quit_requested: false,
            disconnect_requested: false,
            events: vec![],
            initialized: false,
            tx_owner: Default::default(),
            seen: Default::default(),
            waits: vec![],
            deferred: Default::default(),
            jobs: vec![],
            saves: vec![],
        }
    }
    /// Apply all queued requests, push events to subscribers. Returns true if
    /// anything was handled (the UI should repaint).
    ///
    /// Drains `editor.drain_events()`; get them with [`Dispatcher::take_events`].
    pub fn pump(&mut self, server: &ApiServer, editor: &mut zebra_core::Editor) -> bool {
        self.pump_impl(server, &mut One(editor))
    }
    /// [`Dispatcher::pump`] for a host with several open documents: a call
    /// naming a `document` goes to that one, every other call to the front one.
    pub fn pump_docs(&mut self, server: &ApiServer, docs: &mut dyn Documents) -> bool {
        self.pump_impl(server, docs)
    }
    /// Every editor event drained since the last call (human and agent).
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
    /// True while a `wait` or deferred call is pending (the server's waker
    /// keeps firing meanwhile; hosts normally don't need this).
    pub fn has_pending(&self) -> bool {
        !self.waits.is_empty() || !self.deferred.is_empty()
    }
}

/// `zebra --headless`: no window, same API. Blocks until killed or `quit`.
pub fn run_headless(opts: ApiOptions, open: Option<PathBuf>) -> anyhow::Result<()> {
    // Only while the person's AI control switch is on (a person switched it
    // on once, and nobody switched it off since).
    if let Some(why) = discovery::ai_switch().flag_refusal() {
        anyhow::bail!("{why}");
    }
    headless::run(opts, open)
}

pub use headless::HeadlessHandle;
