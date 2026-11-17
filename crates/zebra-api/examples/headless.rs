//! Minimal stand-in for `zebra --headless --api`: same flags, no GUI deps.
//! `cargo run -p zebra-api --example headless -- --headless --api [--agent-permission full] [--api-socket P] [file]`
use zebra_api::{ApiOptions, Permission};

fn main() -> anyhow::Result<()> {
    let mut opts = ApiOptions::default();
    let mut file = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--headless" | "--api" => {}
            "--api-socket" => opts.socket = args.next().map(Into::into),
            "--agent-permission" => {
                opts.permission = match args.next().as_deref() {
                    Some("observe") => Permission::Observe,
                    Some("full") => Permission::Full,
                    _ => Permission::Edit,
                }
            }
            other => file = Some(other.into()),
        }
    }
    zebra_api::run_headless(opts, file)
}
