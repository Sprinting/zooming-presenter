//! Zebra automation API: "drive Zebra like Playwright drives Chrome".
//!
//! CONTRACT used by `zebra-app` (keep these signatures stable):
//!
//! ```ignore
//! let server = ApiServer::start(ApiOptions::default(), waker)?;  // background thread(s)
//! // on the UI thread, every frame (and whenever `waker` fires):
//! dispatcher.pump(&server, &mut editor);                         // applies queued requests
//! ```
//! Headless: `zebra --headless --api` calls [`run_headless`].
//!
//! Wire protocol: JSON-RPC 2.0, one JSON object per line, over a Unix domain
//! socket (mode 0600). Discovery file per running instance (see `discovery`).

use std::path::PathBuf;
use std::sync::Arc;

pub use zebra_core::view::Permission;

#[derive(Debug, Clone)]
pub struct ApiOptions {
    /// Socket path; default: per-user temp dir + pid.
    pub socket: Option<PathBuf>,
    /// Initial agent permission.
    pub permission: Permission,
}

impl Default for ApiOptions {
    fn default() -> Self {
        ApiOptions { socket: None, permission: Permission::Edit }
    }
}

/// A connected client, for the agent status pill.
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub id: u64,
    pub name: String,
}

/// Wakes the UI thread when a request arrives (eframe: `ctx.request_repaint()`).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

pub struct ApiServer {
    // implemented by the API agent
}

impl ApiServer {
    pub fn start(_opts: ApiOptions, _waker: Waker) -> anyhow::Result<ApiServer> {
        anyhow::bail!("not implemented yet")
    }
    pub fn socket_path(&self) -> PathBuf {
        PathBuf::new()
    }
    pub fn clients(&self) -> Vec<ClientInfo> {
        vec![]
    }
}

/// Runs on the thread that owns the `Editor` (main thread in the app).
pub struct Dispatcher {
    pub renderer: Arc<zebra_render::Renderer>,
}

impl Dispatcher {
    pub fn new(renderer: Arc<zebra_render::Renderer>) -> Self {
        Dispatcher { renderer }
    }
    /// Apply all queued requests, push events to subscribers. Returns true if
    /// anything was handled (the UI should repaint).
    pub fn pump(&mut self, _server: &ApiServer, _editor: &mut zebra_core::Editor) -> bool {
        false
    }
}

/// `zebra --headless`: no window, same API. Blocks until killed or `quit`.
pub fn run_headless(_opts: ApiOptions, _open: Option<PathBuf>) -> anyhow::Result<()> {
    anyhow::bail!("not implemented yet")
}
