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
mod server;

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

/// A connected (authenticated) client, for the agent status pill.
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
pub struct Dispatcher {
    pub renderer: Arc<zebra_render::Renderer>,
    /// Set by [`run_headless`]: enables the `quit` method.
    pub headless: bool,
    /// Set when a client called `quit` (headless only).
    pub quit_requested: bool,
    events: Vec<Event>,
    initialized: bool,
    tx_owner: Option<u64>,
    waits: Vec<dispatch::Wait>,
    deferred: std::collections::VecDeque<dispatch::Pending>,
}

impl Dispatcher {
    pub fn new(renderer: Arc<zebra_render::Renderer>) -> Self {
        Dispatcher {
            renderer,
            headless: false,
            quit_requested: false,
            events: vec![],
            initialized: false,
            tx_owner: None,
            waits: vec![],
            deferred: Default::default(),
        }
    }
    /// Apply all queued requests, push events to subscribers. Returns true if
    /// anything was handled (the UI should repaint).
    ///
    /// Drains `editor.drain_events()`; get them with [`Dispatcher::take_events`].
    pub fn pump(&mut self, server: &ApiServer, editor: &mut zebra_core::Editor) -> bool {
        self.pump_impl(server, editor)
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
    headless::run(opts, open)
}

pub use headless::HeadlessHandle;
