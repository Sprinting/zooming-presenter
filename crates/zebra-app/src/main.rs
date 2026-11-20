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
    /// Lower what a connected agent may do this run: observe or edit. It
    /// never raises it: the person's choice from the AI button is the most
    /// an agent gets.
    #[arg(long, value_name = "LEVEL")]
    agent_permission: Option<String>,
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
    /// Without it, Zebra opens where it was last (like Keynote).
    #[arg(long, hide = true)]
    size: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    zebra_app::keymap::load();
    // My Themes, My Templates and the sample, for agents too.
    zebra_api::library::install(Some(zebra_app::sample::commands));
    // Only a person raises permission (the AI button): the flag can only
    // lower it. `ApiOptions::permission` is the most the flag allows; the
    // app takes the lower of it and the person's saved choice.
    let cap = match cli.agent_permission.as_deref() {
        None | Some("full") => Permission::Full,
        Some("observe") => Permission::Observe,
        Some("edit") => Permission::Edit,
        Some(other) => anyhow::bail!("--agent-permission must be observe, edit or full (got '{other}')"),
    };
    let mut api_opts = ApiOptions { socket: cli.api_socket.clone(), permission: cap };
    // The first time is always a person: no flag turns AI control on until
    // someone has switched it on in Zebra's window once.
    for v in zebra_api::discovery::ignored_overrides() {
        eprintln!("zebra: {v} is ignored by this build (only developer builds use it)");
    }
    // New media is spilled to files beside the autosaves and mapped, not
    // kept on the heap (headless agents too).
    zebra_app::files::start_spilling();
    // The clip-art index takes a moment to read: do it before anyone asks.
    let _ = std::thread::Builder::new().name("zebra-symbols".into()).spawn(|| {
        let _ = zebra_core::symbols::library();
    });
    let refused = zebra_api::discovery::ai_switch().flag_refusal();
    if cli.headless {
        // No window and no person: only while the person's switch is on.
        if let Some(why) = refused {
            anyhow::bail!("{why}");
        }
        // No window, so no person to ask: the person's saved choice.
        api_opts.permission = zebra_app::app::launch_permission(zebra_app::app::saved_agent_permission(), cap);
        return zebra_api::run_headless(api_opts, cli.file);
    }
    let screenshot = match &cli.screenshot_after {
        Some(v) if v.len() == 2 => Some((v[0].parse::<f64>()?, PathBuf::from(&v[1]))),
        _ => None,
    };
    let asked = cli.size.as_deref().and_then(|s| s.split_once('x')).and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?)));
    // Windows go back where they were, unless a debug run asks for a size.
    let frames = (cli.size.is_none() && screenshot.is_none()).then(zebra_app::window_frames::Frames::load);
    let last = frames.as_ref().and_then(|f| f.last);
    let first = zebra_app::mac::first_window_size(zebra_app::mac::main_screen_room());
    let (w, h) = asked.or(last.map(|f| (f.size[0], f.size[1]))).unwrap_or((first.x, first.y));
    if let (true, Some(why)) = (cli.api, refused) {
        eprintln!("zebra: {why}");
    }
    let opts = AppOptions { open: cli.file, api: (cli.api && refused.is_none()).then_some(api_opts), screenshot, demo: cli.demo, as_launch: false };

    let background = cli.background || std::env::var_os("ZEBRA_BACKGROUND").is_some_and(|v| v != "0");
    let native = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: {
            let b = zebra_app::window_builder("Zebra").with_inner_size([w, h]).with_active(!background);
            // macOS keeps a title bar on screen if that screen is gone.
            match last {
                Some(f) if asked.is_none() => b.with_position(f.pos),
                _ => b,
            }
        },
        // Zebra keeps window frames itself (window_frames), per document.
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
    // Before launch finishes too: Finder sends the file it launched us for
    // while launch finishes, before any window exists.
    zebra_app::mac::install_open_hook();
    eframe::run_native(
        "Zebra",
        native,
        Box::new(move |cc| {
            // Launched by a double-click in Finder: open that file instead
            // of offering the template chooser.
            let mut opts = opts;
            opts.open = zebra_app::open_events::launch_file(opts.open.take());
            let app = ZebraApp::new(&cc.egui_ctx, cc.wgpu_render_state.clone(), cc.storage, opts);
            let mut shell = zebra_app::shell::Shell::new(app);
            shell.frames = frames;
            Ok(Box::new(shell))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
