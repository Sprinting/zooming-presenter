//! zebractl: drive a running Zebra from the command line.
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};
use zebra_api::client::{Client, ClientError};

#[derive(Parser)]
#[command(name = "zebractl", version, about = "Control Zebra, a zooming presentation app, from the command line",
    long_about = "Control Zebra, a zooming presentation app, from the command line.\n\n\
Zebra must be open, and the person using it must have switched on \"Allow AI control\" (the AI button in Zebra's toolbar). Agents cannot switch it on; `zebractl disconnect` switches it off. If you have to start Zebra yourself, start it without taking the person's focus: `open -g -a Zebra --args --background`. Everything you do shows up live in the person's window with your --agent name, and each change can be undone by them.\n\n\
NAME YOURSELF: pass --agent <your name> on every command (or set ZEBRA_AGENT_NAME once). Without it you share the name \"zebractl\" with every other unnamed agent.\n\n\
READ BEFORE YOU CHANGE: if the person or another agent changed the document since you last read it (`describe`, `query`), edits fail with stale_revision. Read again, then retry.\n\n\
HOW ZEBRA WORKS:\n  One infinite canvas. Elements (frames, text, shapes, images) have a centre x,y in world units (y points down), a scale and a rotation in degrees.\n  Frames (bracket, circle, rectangle, invisible) group whatever lies inside them and are the usual path stops.\n  The path is the ordered list of camera stops used when presenting; the camera flies, zooms and rotates between them.\n  Ids are short and stable (f1, t2, s3); read them with `describe` first.\n\n\
START HERE:\n  zebractl describe                     outline of the canvas, ids and path\n  zebractl add frame --x 0 --y 0 --name Intro --path\n  zebractl add text \"Hello\" --inside f1 --style title\n  zebractl apply '[{\"op\":\"auto_path\"},{\"op\":\"set_theme\",\"theme\":\"night\"}]'\n  zebractl screenshot --step 0 -o step0.png   check your work (doesn't move the person's view)\n  zebractl commands                     every command with its JSON schema")]
struct Cli {
    #[command(flatten)]
    conn: Conn,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct Conn {
    /// Connect to the instance with this pid.
    #[arg(long, global = true)]
    instance: Option<u32>,
    /// Connect to this socket path.
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    /// Auth token (default: from the discovery file; env ZEBRA_TOKEN).
    #[arg(long, global = true, env = "ZEBRA_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// The open document to act on, by id or name (see `zebractl
    /// documents`). Default: the one in front.
    #[arg(long, global = true)]
    document: Option<String>,
    /// Who you are: the name the person sees on everything you do
    /// (history, status bar, your cursor), and what keeps your work apart
    /// from other agents'. Use your own name, the same on every command.
    /// Also read from ZEBRA_AGENT_NAME.
    #[arg(long, global = true, env = "ZEBRA_AGENT_NAME")]
    agent: Option<String>,
}

/// Used when an agent gives no name: every unnamed agent shares it.
const SHARED_NAME: &str = "zebractl";

impl Conn {
    fn name(&self) -> String {
        self.agent.clone().filter(|a| !a.trim().is_empty()).unwrap_or_else(|| SHARED_NAME.to_string())
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Mode, selection, camera, revision, path length...
    State,
    /// Text outline of the presentation (ids, text, positions, path).
    Describe,
    /// Memory and speed: heap, cache sizes, frame times, repaints per second.
    Perf {
        /// Print JSON instead of a short table.
        #[arg(long)]
        json: bool,
    },
    /// Run a query: '{"q":"elements","kind":"frame"}' or just 'path'.
    Query { query: String },
    /// Apply a command, or an array of commands as one undo step.
    Apply {
        /// JSON command or array (omit with -f).
        json: Option<String>,
        /// Read JSON from a file ('-' = stdin).
        #[arg(short, long)]
        file: Option<PathBuf>,
        /// Name for this change: one undo step with this label, shown in
        /// the person's history and status bar.
        #[arg(long)]
        label: Option<String>,
    },
    /// Run a view command: '{"op":"zoom_to","id":"f1"}'.
    View { json: String },
    /// Add an element.
    Add {
        #[command(subcommand)]
        what: Add,
    },
    /// Move elements by a world delta (frames carry what they hold, except
    /// anything locked). --frame-only moves a frame without its contents.
    Move {
        ids: Vec<String>,
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        dx: f64,
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        dy: f64,
        #[arg(long)]
        frame_only: bool,
    },
    /// Delete elements.
    Delete { ids: Vec<String> },
    /// Select elements (your own selection unless you have 'full' permission).
    /// --frame-only selects one frame on its own, as Option-clicking it
    /// does: moving, resizing or turning it in Zebra leaves its contents
    /// (needs 'full').
    Select {
        ids: Vec<String>,
        #[arg(long)]
        frame_only: bool,
    },
    /// Set text of an element.
    SetText { id: String, text: String },
    /// A table's cells (what each shows, formulas as typed), columns,
    /// merges and settings. --range A1:D10 limits the cells.
    Table {
        id: String,
        #[arg(long)]
        range: Option<String>,
    },
    /// Fill cells of a table from AT (e.g. B2) with tab- or
    /// comma-separated rows read from FILE ("-" for stdin). The table
    /// grows to take them. Formulas start with =.
    SetCells { id: String, at: String, file: PathBuf },
    /// Take characters START..END (counted in characters, END not
    /// included) out of text ID into a new box of their own, as dragging
    /// selected words out of a box does. The new box's centre is --x,--y;
    /// without them it goes just below the old box. --copy leaves the old
    /// box as it was. Prints the new box's id.
    Split {
        id: String,
        start: usize,
        end: usize,
        #[arg(long, allow_hyphen_values = true, requires = "y")]
        x: Option<f64>,
        #[arg(long, allow_hyphen_values = true, requires = "x")]
        y: Option<f64>,
        #[arg(long)]
        copy: bool,
    },
    /// Undo the last change (yours or the person's)
    Undo,
    /// Redo
    Redo,
    /// Start presenting (at a 0-based step): `present 2` or `present --step 2`.
    Present {
        step: Option<usize>,
        #[arg(long = "step", value_name = "STEP", conflicts_with = "step")]
        step_flag: Option<usize>,
    },
    /// Next step/build while presenting.
    Next,
    /// Previous step/build while presenting.
    Prev,
    /// Leave present mode.
    Stop,
    /// Show the person a one-line status in Zebra's status bar
    /// ("Rewriting the intro"). No text clears it.
    Status { text: Vec<String> },
    /// Render a PNG without moving the human's camera.
    Screenshot {
        #[arg(long, conflicts_with_all = ["element", "overview", "viewport"])]
        step: Option<usize>,
        /// With --step: how many of the step's builds are showing (0 = none
        /// yet). Default: all of them.
        #[arg(long, requires = "step")]
        build: Option<usize>,
        #[arg(long)]
        element: Option<String>,
        #[arg(long)]
        overview: bool,
        /// The human's window.
        #[arg(long)]
        viewport: bool,
        #[arg(short, long, default_value = "zebra.png")]
        output: PathBuf,
        /// Default: the person's window size (up to 1600 wide), so a step
        /// looks as it will when presenting. Give one side to keep that shape.
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
    },
    /// Export pdf | png | json | zebra | html (a web page; a folder when its media is big).
    Export {
        format: String,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
    },
    /// Open a .zebra (or .json) file (needs 'full' permission).
    Open {
        path: PathBuf,
        /// Discard unsaved changes.
        #[arg(long)]
        force: bool,
    },
    /// Save (needs 'full' permission).
    Save { path: Option<PathBuf> },
    /// File › Save as Template: into the person's My Templates (needs
    /// 'full' permission). Default name: the document's title.
    SaveTemplate { name: Option<String> },
    /// Print events as JSON lines.
    Events {
        /// Keep printing forever.
        #[arg(long)]
        follow: bool,
        /// Stop after N events (default 1 without --follow).
        #[arg(long)]
        count: Option<usize>,
        /// Only these events (comma separated).
        #[arg(long, value_delimiter = ',')]
        filter: Vec<String>,
    },
    /// Wait for: idle | step N | revision N | mode MODE.
    Wait {
        what: String,
        value: Option<String>,
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
    },
    /// Send a raw JSON-RPC message (id/jsonrpc added if missing); prints the response.
    Raw { json: String },
    /// The command catalogue.
    Commands {
        /// Markdown table (for docs).
        #[arg(long)]
        markdown: bool,
        /// Full JSON (schemas and examples).
        #[arg(long)]
        json: bool,
    },
    /// List running instances.
    Instances,
    /// List the documents open in Zebra (id, name, which is in front).
    Documents,
    /// Open a new, empty document in a window of its own; prints its id.
    /// Pass `--document <id>` on later commands to work in it.
    New,
    /// Close the `--document` you name. Refused with unsaved changes
    /// unless --discard (needs 'full' permission; the changes are lost).
    Close {
        #[arg(long)]
        discard: bool,
    },
    /// Copy elements from `--document` (default: the front one) into
    /// another open document, with their groups, frame contents and
    /// pictures. Doesn't touch the clipboard.
    CopyTo {
        /// The document to copy into (id or name).
        to: String,
        /// Element ids.
        #[arg(required = true)]
        ids: Vec<String>,
        /// Centre the copies here (world x).
        #[arg(long, requires = "y")]
        x: Option<f64>,
        #[arg(long, requires = "x")]
        y: Option<f64>,
    },
    /// File › Import…: a PDF's pages as pictures, or a PowerPoint (.pptx)
    /// file's slides as text, shapes and pictures, each page or slide in a
    /// frame inside one frame, with a path step each, right of what is on
    /// the canvas, as one undo step.
    Import {
        /// The .pdf or .pptx file.
        file: PathBuf,
        /// First page or slide (1-based; default 1).
        #[arg(long)]
        from: Option<u64>,
        /// Last page or slide (default: the last). A PDF of more than 200
        /// pages needs it; at most 2,000 pages are imported at a time.
        #[arg(long)]
        to: Option<u64>,
    },
    /// Internal: the per-agent background relay that holds this agent's one
    /// connection to Zebra (started by the first command).
    #[command(name = "__relay", hide = true)]
    Relay {
        #[arg(long)]
        zebra_socket: PathBuf,
        #[arg(long)]
        relay_socket: PathBuf,
    },
    /// Switch AI control off in Zebra (only the person can switch it back on)
    Disconnect,
}

#[derive(Subcommand)]
enum Add {
    /// Add a text box (x,y = centre).
    Text {
        text: String,
        #[arg(long, allow_hyphen_values = true)]
        x: Option<f64>,
        #[arg(long, allow_hyphen_values = true)]
        y: Option<f64>,
        /// title | subtitle | body
        #[arg(long, default_value = "body")]
        style: String,
        #[arg(long)]
        scale: Option<f64>,
        /// Centre it inside this frame, sized to fit.
        #[arg(long)]
        inside: Option<String>,
        #[arg(long)]
        id: Option<String>,
    },
    /// Add a frame.
    Frame {
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        x: f64,
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        y: f64,
        #[arg(long, default_value_t = 800.0)]
        w: f64,
        #[arg(long, default_value_t = 600.0)]
        h: f64,
        /// bracket | circle | rectangle | invisible
        #[arg(long, default_value = "bracket")]
        style: String,
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        rotation: f64,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        id: Option<String>,
        /// Append to the path.
        #[arg(long)]
        path: bool,
    },
    /// Add an image from a file.
    Image {
        file: PathBuf,
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        x: f64,
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        y: f64,
        /// World width (height follows aspect ratio).
        #[arg(long)]
        width: Option<f64>,
        #[arg(long)]
        alt: Option<String>,
    },
    /// Add a web video from an https link, as Insert › Web Video… does:
    /// MP4, MOV or M4V files and HLS streams play from the link; YouTube
    /// and Vimeo links become a video card. Zebra checks the link first
    /// (up to 10 s) and says why when it doesn't play. Only the link and a
    /// poster are kept in the deck.
    WebVideo {
        url: String,
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        x: f64,
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        y: f64,
        /// World width (height follows the video's shape).
        #[arg(long)]
        width: Option<f64>,
        #[arg(long)]
        alt: Option<String>,
        /// Seconds into the video to start (default: the link's own t=).
        #[arg(long)]
        start: Option<f64>,
        /// Seconds into the video to stop.
        #[arg(long)]
        end: Option<f64>,
    },
    /// Add a table: empty (--rows, --columns) or filled from a CSV or
    /// tab-separated file (--csv FILE, "-" for stdin). Style ts1..ts11
    /// (default ts4, Ruled).
    Table {
        #[arg(long)]
        rows: Option<usize>,
        #[arg(long)]
        columns: Option<usize>,
        #[arg(long)]
        csv: Option<PathBuf>,
        #[arg(long)]
        style: Option<String>,
        #[arg(long)]
        header_rows: Option<u8>,
        #[arg(long)]
        footer_rows: Option<u8>,
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        x: f64,
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        y: f64,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        id: Option<String>,
    },
    /// Add a shape: rectangle | ellipse | triangle | line | arrow.
    Shape {
        shape: String,
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        x: f64,
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        y: f64,
        #[arg(long, default_value_t = 200.0)]
        w: f64,
        #[arg(long, default_value_t = 200.0)]
        h: f64,
        #[arg(long)]
        fill: Option<String>,
    },
}

/// Cells to send: a file (or stdin), refused unread when it is bigger
/// than a table can take.
fn read_cells(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let cap = zebra_core::table::MAX_PASTE_BYTES as u64;
    let mut text = String::new();
    let read = if path.as_os_str() == "-" {
        std::io::stdin().take(cap + 1).read_to_string(&mut text)
    } else {
        std::fs::File::open(path).and_then(|f| f.take(cap + 1).read_to_string(&mut text))
    };
    read.with_context(|| path.display().to_string())?;
    if text.len() as u64 > cap {
        bail!("{}", zebra_core::table::Limit::Bytes.message());
    }
    Ok(text)
}

fn main() {
    let cli = Cli::parse();
    for v in zebra_api::discovery::ignored_overrides() {
        eprintln!("zebractl: {v} is ignored by this build (only developer builds use it)");
    }
    if let Err(e) = run(cli) {
        eprintln!("zebractl: {e:#}");
        std::process::exit(1);
    }
}

fn abs(p: &Path) -> Result<PathBuf> {
    if p.is_absolute() {
        return Ok(p.to_path_buf());
    }
    Ok(std::env::current_dir()?.join(p))
}

fn connect(c: &Conn) -> Result<Client> {
    Client::discover(c.instance, c.socket.as_deref(), c.token.as_deref(), &c.name())
}

/// Connect through this agent's relay, starting it if needed (see
/// `zebra_api::relay`). Falls back to a direct connection.
/// `ZEBRACTL_DIRECT=1` skips the relay.
fn connect_agent(c: &Conn, direct: bool) -> Result<Client> {
    if direct || std::env::var_os("ZEBRACTL_DIRECT").is_some() {
        return connect(c);
    }
    let (sock, tok) = match (&c.socket, &c.token) {
        (Some(s), Some(t)) => (s.clone(), t.clone()),
        _ => {
            let inst = zebra_api::discovery::find(c.instance, c.socket.as_deref())?;
            (inst.socket, c.token.clone().unwrap_or(inst.token))
        }
    };
    let rs = zebra_api::relay::socket_for(&sock, &c.name());
    if let Ok(cl) = zebra_api::relay::connect(&rs, &c.name(), &tok) {
        return Ok(cl);
    }
    if let Ok(exe) = std::env::current_exe() {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("__relay")
            .arg("--zebra-socket")
            .arg(&sock)
            .arg("--relay-socket")
            .arg(&rs)
            .arg("--agent")
            .arg(&c.name())
            .env("ZEBRA_TOKEN", &tok)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // Its own session: outlives this command and the agent's shell.
        // SAFETY: setsid is async-signal-safe.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        if cmd.spawn().is_ok() {
            for _ in 0..40 {
                if let Ok(cl) = zebra_api::relay::connect(&rs, &c.name(), &tok) {
                    return Ok(cl);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
    Client::connect_hello(&sock, &c.name(), &tok).map_err(|e| anyhow!("{e}"))
}

fn print(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

/// `--document`, sent with every call.
static DOCUMENT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

fn call(cl: &mut Client, method: &str, mut params: Value) -> Result<Value> {
    if let (Some(Some(d)), Some(o)) = (DOCUMENT.get(), params.as_object_mut()) {
        o.insert("document".into(), json!(d));
    }
    cl.call(method, params).map_err(|e| match e {
        ClientError::Rpc(e) => anyhow!("{e}"),
        ClientError::Io(e) => anyhow!("{e}"),
    })
}

fn parse_json(s: &str, what: &str) -> Result<Value> {
    serde_json::from_str(s).with_context(|| format!("{what} is not valid JSON: {s}"))
}

fn run(cli: Cli) -> Result<()> {
    let _ = DOCUMENT.set(cli.conn.document.clone());
    // Commands that don't need a connection.
    match &cli.cmd {
        Cmd::Commands { markdown, json } => {
            if *markdown {
                print!("{}", zebra_api::catalogue::catalogue_markdown());
            } else if *json {
                print(&zebra_api::catalogue::catalogue_json());
            } else {
                print!("{}", zebra_api::catalogue::catalogue_text());
            }
            return Ok(());
        }
        Cmd::Instances => {
            let list: Vec<Value> = zebra_api::discovery::list()
                .into_iter()
                .map(|i| json!({"pid": i.pid, "socket": i.socket, "version": i.version, "started_at": i.started_at}))
                .collect();
            print(&json!(list));
            return Ok(());
        }
        Cmd::Relay { zebra_socket, relay_socket } => {
            let token = cli.conn.token.clone().unwrap_or_default();
            return zebra_api::relay::run(zebra_socket, &token, &cli.conn.name(), relay_socket);
        }
        _ => {}
    }
    if cli.conn.agent.as_deref().is_none_or(|a| a.trim().is_empty()) {
        eprintln!(
            "zebractl: you have no name, so you act as \"{SHARED_NAME}\", shared with every other unnamed agent. Pass --agent <your name> on every command, or set ZEBRA_AGENT_NAME."
        );
    }

    // Event streams hold their own connection; everything else goes
    // through the agent's relay so Zebra sees one steady connection.
    let direct = matches!(cli.cmd, Cmd::Events { .. });
    let mut cl = connect_agent(&cli.conn, direct)?;
    let c = &mut cl;
    let view = |c: &mut Client, cmd: Value| call(c, "view", json!({ "command": cmd }));
    match cli.cmd {
        Cmd::State => print(&call(c, "state", json!({}))?),
        Cmd::Documents => print(&call(c, "documents", json!({}))?),
        Cmd::New => print(&call(c, "new_document", json!({}))?),
        Cmd::Close { discard } => {
            if cli.conn.document.is_none() {
                bail!("say which document to close: zebractl --document <id or name> close");
            }
            print(&call(c, "close_document", json!({ "discard": discard }))?)
        }
        Cmd::CopyTo { to, ids, x, y } => {
            let mut p = json!({ "to": to, "ids": ids });
            if let (Some(x), Some(y)) = (x, y) {
                p["at"] = json!({ "x": x, "y": y });
            }
            print(&call(c, "copy_to", p)?)
        }
        Cmd::Import { file, from, to } => {
            let mut p = json!({ "path": abs(&file)? });
            if let Some(n) = from {
                p["from"] = json!(n);
            }
            if let Some(n) = to {
                p["to"] = json!(n);
            }
            print(&call(c, "import", p)?)
        }
        Cmd::Describe => {
            let v = call(c, "describe", json!({}))?;
            match v.get("outline").and_then(|o| o.as_str()) {
                Some(s) => print!("{s}"),
                None => print(&v),
            }
        }
        Cmd::Perf { json } => {
            let v = call(c, "query", json!({ "query": { "q": "perf" } }))?;
            if json {
                print(&v);
            } else {
                print_perf(&v);
            }
        }
        Cmd::Query { query } => {
            let q = if query.trim_start().starts_with('{') { parse_json(&query, "query")? } else { json!({ "q": query }) };
            print(&call(c, "query", json!({ "query": q }))?);
        }
        Cmd::Apply { json: j, file, label } => {
            let text = match (j, file) {
                (Some(j), None) => j,
                (None, Some(f)) if f.as_os_str() == "-" => std::io::read_to_string(std::io::stdin())?,
                (None, Some(f)) => std::fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?,
                _ => bail!("give the command JSON as an argument or with -f FILE"),
            };
            let mut v = parse_json(&text, "command")?;
            absolutize_image_paths(&mut v)?;
            let r = if v.is_array() {
                let mut p = json!({ "commands": v });
                if let Some(l) = label {
                    p["label"] = json!(l);
                }
                call(c, "batch", p)?
            } else {
                let mut p = json!({ "command": v });
                if let Some(l) = label {
                    p["label"] = json!(l);
                }
                call(c, "apply", p)?
            };
            print(&r);
        }
        Cmd::View { json: j } => print(&view(c, parse_json(&j, "view command")?)?),
        Cmd::Add { what } => {
            let cmd = match what {
                Add::Text { text, x, y, style, scale, inside, id } => {
                    let mut m = json!({ "op": "add_text", "text": text, "style": style });
                    if let Some(x) = x { m["x"] = json!(x) }
                    if let Some(y) = y { m["y"] = json!(y) }
                    if let Some(s) = scale { m["scale"] = json!(s) }
                    if let Some(f) = inside { m["place"] = json!({ "relative_to": f, "side": "inside" }) }
                    if let Some(i) = id { m["id"] = json!(i) }
                    m
                }
                Add::Frame { x, y, w, h, style, rotation, name, id, path } => {
                    let mut m = json!({ "op": "add_frame", "x": x, "y": y, "w": w, "h": h, "style": style, "rotation": rotation, "add_to_path": path });
                    if let Some(n) = name { m["name"] = json!(n) }
                    if let Some(i) = id { m["id"] = json!(i) }
                    m
                }
                Add::Image { file, x, y, width, alt } => {
                    let mut m = json!({ "op": "add_image", "path": abs(&file)?, "x": x, "y": y });
                    if let Some(w) = width { m["width"] = json!(w) }
                    if let Some(a) = alt { m["alt"] = json!(a) }
                    m
                }
                Add::WebVideo { url, x, y, width, alt, start, end } => {
                    let mut m = json!({ "op": "add_web_video", "url": url, "x": x, "y": y });
                    if let Some(w) = width { m["width"] = json!(w) }
                    if let Some(a) = alt { m["alt"] = json!(a) }
                    if let Some(s) = start { m["start"] = json!(s) }
                    if let Some(e) = end { m["end"] = json!(e) }
                    m
                }
                Add::Table { rows, columns, csv, style, header_rows, footer_rows, x, y, name, id } => {
                    let mut m = json!({ "op": "add_table", "x": x, "y": y });
                    if let Some(v) = rows { m["rows"] = json!(v) }
                    if let Some(v) = columns { m["columns"] = json!(v) }
                    if let Some(f) = csv { m["csv"] = json!(read_cells(&f)?) }
                    if let Some(v) = style { m["style"] = json!(v) }
                    if let Some(v) = header_rows { m["header_rows"] = json!(v) }
                    if let Some(v) = footer_rows { m["footer_rows"] = json!(v) }
                    if let Some(v) = name { m["name"] = json!(v) }
                    if let Some(v) = id { m["id"] = json!(v) }
                    m
                }
                Add::Shape { shape, x, y, w, h, fill } => {
                    let mut m = json!({ "op": "add_shape", "shape": shape, "x": x, "y": y, "w": w, "h": h });
                    if let Some(f) = fill { m["fill"] = json!(f) }
                    m
                }
            };
            print(&call(c, "apply", json!({ "command": cmd }))?);
        }
        Cmd::Move { ids, dx, dy, frame_only } => {
            let mut cmd = json!({ "op": "move", "ids": ids, "dx": dx, "dy": dy });
            if frame_only {
                cmd["with_contents"] = json!(false);
            }
            print(&call(c, "apply", json!({ "command": cmd }))?)
        }
        Cmd::Delete { ids } => print(&call(c, "apply", json!({ "command": { "op": "delete", "ids": ids } }))?),
        Cmd::Select { ids, frame_only } => {
            let mut cmd = json!({ "op": "select", "ids": ids });
            if frame_only {
                cmd["frame_only"] = json!(true);
            }
            print(&view(c, cmd)?)
        }
        Cmd::Split { id, start, end, x, y, copy } => {
            let mut cmd = json!({ "op": "split_text", "id": id, "start": start, "end": end });
            if let (Some(x), Some(y)) = (x, y) {
                cmd["x"] = json!(x);
                cmd["y"] = json!(y);
            }
            if copy {
                cmd["copy"] = json!(true);
            }
            print(&call(c, "apply", json!({ "command": cmd }))?)
        }
        Cmd::Table { id, range } => {
            let mut q = json!({ "q": "table", "id": id });
            if let Some(r) = range {
                q["range"] = json!(r);
            }
            print(&call(c, "query", json!({ "query": q }))?)
        }
        Cmd::SetCells { id, at, file } => {
            let text = read_cells(&file)?;
            let rows = zebra_core::table::parse_delimited(&text).map_err(|l| anyhow!(l.message()))?;
            print(&call(c, "apply", json!({ "command": { "op": "set_cells", "id": id, "at": at, "values": rows } }))?)
        }
        Cmd::SetText { id, text } => print(&call(c, "apply", json!({ "command": { "op": "update", "id": id, "patch": { "text": text } } }))?),
        Cmd::Undo => print(&call(c, "undo", json!({}))?),
        Cmd::Redo => print(&call(c, "redo", json!({}))?),
        Cmd::Present { step, step_flag } => print(&view(c, json!({ "op": "present", "step": step.or(step_flag) }))?),
        Cmd::Next => print(&view(c, json!({ "op": "next" }))?),
        Cmd::Prev => print(&view(c, json!({ "op": "prev" }))?),
        Cmd::Stop => print(&view(c, json!({ "op": "exit_present" }))?),
        Cmd::Status { text } => print(&view(c, json!({ "op": "set_status", "text": text.join(" ") }))?),
        Cmd::Disconnect => print(&c.call("disconnect", json!({}))?),
        Cmd::Screenshot { step, build, element, overview, viewport, output, width, height } => {
            let target = if let Some(s) = step {
                json!({ "step": s, "build": build })
            } else if let Some(e) = element {
                json!({ "element": e })
            } else if overview {
                json!("overview")
            } else if viewport {
                json!("viewport")
            } else {
                Value::Null
            };
            let out = abs(&output)?;
            print(&call(c, "capture", json!({ "target": target, "width": width, "height": height, "path": out }))?);
        }
        Cmd::Export { format, output, width, height } => {
            let out = abs(&output)?;
            print(&call(c, "export", json!({ "format": format, "path": out, "width": width, "height": height }))?);
        }
        Cmd::Open { path, force } => print(&call(c, "open", json!({ "path": abs(&path)?, "force": force }))?),
        Cmd::Save { path } => {
            let p = path.map(|p| abs(&p)).transpose()?;
            print(&call(c, "save", json!({ "path": p }))?);
        }
        Cmd::SaveTemplate { name } => print(&call(c, "save_template", json!({ "name": name }))?),
        Cmd::Events { follow, count, filter } => {
            let p = if filter.is_empty() { json!({}) } else { json!({ "events": filter }) };
            call(c, "subscribe", p)?;
            let limit = if follow { count } else { Some(count.unwrap_or(1)) };
            let mut n = 0;
            let stdout = std::io::stdout();
            loop {
                if limit.is_some_and(|l| n >= l) {
                    break;
                }
                match c.next_event(None).map_err(|e| anyhow!("{e}"))? {
                    Some(ev) => {
                        let mut out = stdout.lock();
                        writeln!(out, "{ev}")?;
                        out.flush()?;
                        n += 1;
                    }
                    None => continue,
                }
            }
        }
        Cmd::Wait { what, value, timeout_ms } => {
            let until = match (what.as_str(), value) {
                ("idle", _) => json!("idle"),
                ("step", Some(v)) => json!({ "step": v.parse::<u64>().context("step must be a number")? }),
                ("revision", Some(v)) => json!({ "revision": v.parse::<u64>().context("revision must be a number")? }),
                ("mode", Some(v)) => json!({ "mode": v }),
                _ => bail!("usage: zebractl wait idle | step N | revision N | mode edit|path_edit|present"),
            };
            print(&call(c, "wait", json!({ "until": until, "timeout_ms": timeout_ms }))?);
        }
        Cmd::Raw { json: j } => {
            let mut v = parse_json(&j, "message")?;
            if !v.is_object() {
                bail!("a raw message is a JSON object, like '{{\"method\":\"state\"}}'");
            }
            if v.get("id").is_none() {
                v["id"] = json!(1000);
            }
            let r = c.raw(v).map_err(|e| anyhow!("{e}"))?;
            print(&r);
        }
        Cmd::Commands { .. } | Cmd::Instances | Cmd::Relay { .. } => unreachable!(),
    }
    Ok(())
}

/// The commands that name a file to read.
const FILE_OPS: [&str; 6] = ["add_image", "add_video", "replace_image", "set_poster", "set_soundtrack", "set_voiceover"];

/// Make the `path` of every command that reads a file absolute (relative
/// to our working directory: the app's is somewhere else). Only
/// `add_image` used to be handled; the others were refused by the app.
fn absolutize_image_paths(v: &mut Value) -> Result<()> {
    absolutize_paths_in(v, &std::env::current_dir()?);
    Ok(())
}

fn absolutize_paths_in(v: &mut Value, cwd: &Path) {
    match v {
        Value::Array(a) => {
            for x in a {
                absolutize_paths_in(x, cwd);
            }
        }
        Value::Object(o) => {
            if o.get("op").and_then(|x| x.as_str()).is_some_and(|op| FILE_OPS.contains(&op)) {
                if let Some(Value::String(p)) = o.get("path") {
                    if !Path::new(p).is_absolute() {
                        let ap = cwd.join(p);
                        o.insert("path".into(), json!(ap));
                    }
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn every_file_command_gets_an_absolute_path() {
        let mut v = json!([
            { "op": "add_image", "path": "a.png" },
            { "op": "add_video", "path": "clips/b.mov" },
            { "op": "replace_image", "id": "e1", "path": "c.png" },
            { "op": "set_poster", "id": "e2", "path": "d.png" },
            { "op": "set_soundtrack", "path": "e.mp3" },
            { "op": "set_voiceover", "index": 0, "path": "/abs/f.m4a" },
            { "op": "add_text", "text": "path", "path": "not a file" }
        ]);
        absolutize_paths_in(&mut v, Path::new("/work/deck"));
        let got: Vec<&str> = v.as_array().unwrap().iter().map(|c| c["path"].as_str().unwrap()).collect();
        assert_eq!(got, ["/work/deck/a.png", "/work/deck/clips/b.mov", "/work/deck/c.png", "/work/deck/d.png", "/work/deck/e.mp3", "/abs/f.m4a", "not a file"]);
    }
}

/// `zebractl perf` as a short human-readable table.
fn print_perf(v: &Value) {
    let n = |k: &str| v.get(k).filter(|x| !x.is_null()).map(|x| x.to_string()).unwrap_or_else(|| "n/a".into());
    println!("heap            {} MB (peak {} MB)", n("heap_mb"), n("heap_peak_mb"));
    println!("frame time      p50 {} ms, p95 {} ms ({} frames)", n("frame_ms_p50"), n("frame_ms_p95"), n("frames_sampled"));
    println!("repaints        {} in the last second, {} in the last 10 s", n("repaints_last_second"), n("repaints_last_10s"));
    if let Some(c) = v.get("caches").and_then(|c| c.as_object()) {
        for (k, x) in c {
            println!("cache {k:<10} {x}");
        }
    }
    if let Some(d) = v.get("document") {
        println!("document        {d}");
    }
}
