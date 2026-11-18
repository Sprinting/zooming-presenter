//! `zebra` — open the app. `zebra talk.zebra` opens a file.
//! `zebra --api` also lets AI agents drive this window (see docs/API.md);
//! `zebra --headless --api` runs with no window at all.
use std::path::PathBuf;

use clap::Parser;
use zebra_api::{ApiOptions, Permission};
use zebra_app::{AppOptions, ZebraApp};

/// Counts live heap bytes for the `perf` query (two relaxed atomics per
/// allocation).
#[global_allocator]
static ALLOC: zebra_core::perf::CountingAlloc = zebra_core::perf::CountingAlloc;

#[derive(Parser, Debug)]
#[command(name = "zebra", version, about = "Zebra — a zooming canvas for presentations")]
struct Cli {
    /// A .zebra file to open.
    file: Option<PathBuf>,
    /// Let AI agents drive this window over a local socket (off by default).
    #[arg(long)]
    api: bool,
    /// Socket path for the automation API (default: per-user temp dir).
    #[arg(long, value_name = "PATH")]
    api_socket: Option<PathBuf>,
    /// What a connected agent may do: observe, edit or full.
    #[arg(long, value_name = "LEVEL", default_value = "edit")]
    agent_permission: String,
    /// No window: only the API (implies --api).
    #[arg(long)]
    headless: bool,
    /// Debug: take a screenshot after SECS seconds, save it to PNG and quit.
    #[arg(long, num_args = 2, value_names = ["SECS", "PNG"], hide = true)]
    screenshot_after: Option<Vec<String>>,
    /// Debug: start in a demo state (sample,select,pathedit,present,agent,text).
    #[arg(long, hide = true)]
    demo: Option<String>,
    /// Open without taking focus (for agents that launch Zebra; also
    /// ZEBRA_BACKGROUND=1). On macOS: `open -g -a Zebra --args --background`.
    #[arg(long)]
    background: bool,
    /// Debug: window size WxH.
    #[arg(long, hide = true, default_value = "1440x900")]
    size: String,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    zebra_app::keymap::load();
    let permission = match cli.agent_permission.as_str() {
        "observe" => Permission::Observe,
        "full" => Permission::Full,
        "edit" => Permission::Edit,
        other => anyhow::bail!("--agent-permission must be observe, edit or full (got '{other}')"),
    };
    let api_opts = ApiOptions { socket: cli.api_socket.clone(), permission };
    // The first time is always a person: no flag turns AI control on until
    // someone has switched it on in Zebra's window once.
    for v in zebra_api::discovery::ignored_overrides() {
        eprintln!("zebra: {v} is ignored by this build (only developer builds use it)");
    }
    // The clip-art index takes a moment to read: do it before anyone asks.
    let _ = std::thread::Builder::new().name("zebra-symbols".into()).spawn(|| {
        let _ = zebra_core::symbols::library();
    });
    let granted = zebra_api::discovery::human_has_granted();
    const NOT_GRANTED: &str = "AI control stays off until a person switches it on in Zebra once (the AI button in the toolbar). After that, --api and --headless work.";
    if cli.headless {
        if !granted {
            anyhow::bail!("{NOT_GRANTED}");
        }
        return zebra_api::run_headless(api_opts, cli.file);
    }
    let screenshot = match &cli.screenshot_after {
        Some(v) if v.len() == 2 => Some((v[0].parse::<f64>()?, PathBuf::from(&v[1]))),
        _ => None,
    };
    let (w, h) = cli.size.split_once('x').and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))).unwrap_or((1440.0, 900.0));
    if cli.api && !granted {
        eprintln!("zebra: {NOT_GRANTED}");
    }
    let opts = AppOptions { open: cli.file, api: (cli.api && granted).then_some(api_opts), screenshot, demo: cli.demo };

    let background = cli.background || std::env::var_os("ZEBRA_BACKGROUND").is_some_and(|v| v != "0");
    let native = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: zebra_app::window_builder("Zebra").with_inner_size([w, h]).with_active(!background),
        // Each run starts at the requested size (debug screenshots need it).
        persist_window: false,
        // winit activates the app at launch unless told not to.
        #[cfg(target_os = "macos")]
        event_loop_builder: Some(Box::new(move |b| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            b.with_activate_ignoring_other_apps(!background);
            // Our menu bar is already in place (below); winit's would
            // replace it.
            b.with_default_menu(false);
        })),
        ..Default::default()
    };
    // Before launch finishes, or AppKit leaves the tab commands out of the
    // Window menu.
    #[cfg(target_os = "macos")]
    zebra_app::menu_mac::install_early();
    eframe::run_native(
        "Zebra",
        native,
        Box::new(move |cc| {
            let app = ZebraApp::new(&cc.egui_ctx, cc.wgpu_render_state.clone(), cc.storage, opts);
            Ok(Box::new(zebra_app::shell::Shell::new(app)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
