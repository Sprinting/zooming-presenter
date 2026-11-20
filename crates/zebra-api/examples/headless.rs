//! Minimal stand-in for `zebra --headless --api`: same flags, no GUI deps.
//! `cargo run -p zebra-api --example headless -- --headless --api [--agent-permission observe] [--api-socket P] [file]`
//! Agents get `edit`; the flag only lowers it (only a person raises it, in
//! the app's AI button).
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
                if args.next().as_deref() == Some("observe") {
                    opts.permission = Permission::Observe;
                }
            }
            other => file = Some(other.into()),
        }
    }
    zebra_api::run_headless(opts, file)
}
