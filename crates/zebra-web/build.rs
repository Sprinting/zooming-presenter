//! Builds the web player: this crate for wasm32-unknown-unknown, release,
//! gzip-compressed into OUT_DIR/player.wasm.gz, which the native exporter
//! embeds (`export::PLAYER_WASM_GZ`). The app never needs a wasm toolchain
//! at run time.
//!
//! * `ZEBRA_WEB_PLAYER=/path/player.wasm` uses a player built elsewhere.
//! * `ZEBRA_WEB_PLAYER=none` skips it (export then reports it missing).
//! * The inner build has its own target dir (`<target>/web-player`), so it
//!   never waits on the outer build's lock and stays cached between builds.
//! * Without the wasm32 target installed, a warning, and no player;
//!   `ZEBRA_REQUIRE_WEB_PLAYER=1` (release bundles) makes that an error.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("player.wasm.gz");
    println!("cargo:rerun-if-env-changed=ZEBRA_WEB_PLAYER");
    println!("cargo:rerun-if-env-changed=ZEBRA_REQUIRE_WEB_PLAYER");
    // Building the player itself: nothing to embed.
    if std::env::var("TARGET").is_ok_and(|t| t.starts_with("wasm32")) {
        std::fs::write(&out, b"").unwrap();
        return;
    }
    let required = std::env::var("ZEBRA_REQUIRE_WEB_PLAYER").is_ok_and(|v| v == "1");
    let wasm = match std::env::var("ZEBRA_WEB_PLAYER") {
        Ok(v) if v == "none" => Err("ZEBRA_WEB_PLAYER=none".to_string()),
        Ok(v) => {
            println!("cargo:rerun-if-changed={v}");
            std::fs::read(&v).map_err(|e| format!("{v}: {e}"))
        }
        Err(_) => build_player(),
    };
    let gz = match wasm {
        Ok(w) => {
            let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            e.write_all(&w).unwrap();
            e.finish().unwrap()
        }
        Err(why) => {
            if required {
                panic!("web player not built: {why}");
            }
            println!("cargo:warning=web player not built ({why}); File > Export > Web Page will be unavailable");
            vec![]
        }
    };
    std::fs::write(&out, gz).unwrap();
}

/// The outermost target directory (the one holding CACHEDIR.TAG).
fn target_root() -> PathBuf {
    if let Ok(t) = std::env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(t);
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    out.ancestors().find(|a| a.join("CACHEDIR.TAG").exists()).map(Path::to_path_buf).unwrap_or_else(|| out.join("web-target"))
}

fn build_player() -> Result<Vec<u8>, String> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../..");
    for dir in ["crates/zebra-web/src", "crates/zebra-web/web", "crates/zebra-render/src", "crates/zebra-core/src", "crates/zebra-core/templates", "Cargo.lock", ".cargo/config.toml"] {
        println!("cargo:rerun-if-changed={}", root.join(dir).display());
    }
    println!("cargo:rerun-if-changed=build.rs");
    let target_dir = target_root().join("web-player");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(&root)
        .args(["build", "--release", "--lib", "-p", "zebra-web", "--target", "wasm32-unknown-unknown"])
        .arg("--target-dir")
        .arg(&target_dir);
    // Flags meant for the outer (native) build must not reach the player.
    for v in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET", "CARGO_TARGET_DIR", "RUSTC_WORKSPACE_WRAPPER"] {
        cmd.env_remove(v);
    }
    let status = cmd.status().map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err("the wasm32 build failed (is the target installed? rustup target add wasm32-unknown-unknown)".into());
    }
    let wasm = target_dir.join("wasm32-unknown-unknown/release/zebra_web.wasm");
    std::fs::read(&wasm).map_err(|e| format!("{}: {e}", wasm.display()))
}
