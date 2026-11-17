//! Instance discovery, like Chrome's `DevToolsActivePort`.
//!
//! Each running instance with the API enabled writes
//! `<data_dir>/Zebra/instances/<pid>.json` (mode 0600):
//! `{"pid", "socket", "token", "version", "api_version", "started_at"}`.
//! `<data_dir>` is `~/Library/Application Support` on macOS
//! (`$XDG_DATA_HOME` / `~/.local/share` elsewhere); `ZEBRA_DATA_DIR`
//! overrides the whole `<data_dir>/Zebra` part (tests use this).
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Instance {
    pub pid: u32,
    pub socket: PathBuf,
    pub token: String,
    pub version: String,
    #[serde(default)]
    pub api_version: u32,
    /// Unix seconds.
    pub started_at: u64,
    #[serde(default)]
    pub headless: bool,
    /// The discovery file itself (not serialised).
    #[serde(skip)]
    pub file: PathBuf,
}

/// App Group shared by the sandboxed app and its bundled `zebractl` in a
/// Mac App Store build (both must carry this `application-groups`
/// entitlement). PLACEHOLDER: the real id is `<TEAMID>.<bundle id>`, set at
/// signing time. Unsandboxed builds don't need it.
pub const APP_GROUP_ID: &str = "ZEBRATEAM.app.zebra";
/// Bundle identifier (placeholder until the product is named).
pub const BUNDLE_ID: &str = "app.zebra.Zebra";

/// The App Group container, if this process can use it (macOS only; it
/// exists once a process with the entitlement has run).
pub fn group_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("ZEBRA_GROUP_DIR") {
        return Some(PathBuf::from(d));
    }
    if !cfg!(target_os = "macos") {
        return None;
    }
    let d = dirs::home_dir()?.join("Library/Group Containers").join(APP_GROUP_ID);
    d.is_dir().then_some(d)
}

/// Where this process writes: `$ZEBRA_DATA_DIR`, else the App Group
/// container (sandboxed builds), else `<data_dir>/Zebra`.
pub fn app_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ZEBRA_DATA_DIR") {
        return PathBuf::from(d);
    }
    if let Some(g) = group_dir() {
        return g.join("Zebra");
    }
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("Zebra")
}

/// Written the first time a person switches on AI control in Zebra's own
/// window. Until it exists, launch flags (`--api`, `--headless`, debug
/// demos) can't switch AI control on: the first time is always a person.
pub fn human_grant_file() -> PathBuf {
    app_dir().join("ai-control-granted")
}

/// Has a person ever switched on AI control in Zebra's window?
pub fn human_has_granted() -> bool {
    human_grant_file().exists()
}

/// Record that a person switched on AI control. Only Zebra's own UI calls
/// this; no API method or launch flag reaches it.
pub fn record_human_grant() -> std::io::Result<()> {
    fs::create_dir_all(app_dir())?;
    fs::write(human_grant_file(), b"")
}

pub fn instances_dir() -> PathBuf {
    app_dir().join("instances")
}

/// Every place a running Zebra may have announced itself: ours, the App
/// Group, the unsandboxed data dir, and the sandboxed app's own container.
pub fn search_dirs() -> Vec<PathBuf> {
    if std::env::var_os("ZEBRA_DATA_DIR").is_some() {
        return vec![instances_dir()];
    }
    let mut v = vec![instances_dir()];
    if let Some(g) = group_dir() {
        v.push(g.join("Zebra/instances"));
    }
    if let Some(d) = dirs::data_dir() {
        v.push(d.join("Zebra/instances"));
    }
    if cfg!(target_os = "macos") {
        if let Some(h) = dirs::home_dir() {
            v.push(h.join("Library/Containers").join(BUNDLE_ID).join("Data/Library/Application Support/Zebra/instances"));
        }
    }
    v.dedup();
    v
}

/// True if a process with this pid exists (works on macOS and Linux).
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // SAFETY: kill with signal 0 only checks for existence/permission.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    Ok(())
}

/// Write the discovery file; returns its path.
pub fn write(inst: &Instance, file_name: &str) -> std::io::Result<PathBuf> {
    let dir = instances_dir();
    ensure_private_dir(&dir)?;
    let path = dir.join(file_name);
    let tmp = dir.join(format!(".{file_name}.tmp"));
    {
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(serde_json::to_string_pretty(inst).unwrap().as_bytes())?;
        f.sync_all().ok();
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// All live instances, newest first. Stale files (dead pid or missing
/// socket) are deleted as a side effect.
pub fn list() -> Vec<Instance> {
    let mut out: Vec<Instance> = vec![];
    for entry in search_dirs().iter().filter_map(|d| fs::read_dir(d).ok()).flat_map(|rd| rd.flatten()) {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(mut inst) = serde_json::from_str::<Instance>(&text) else { continue };
        inst.file = path.clone();
        if !pid_alive(inst.pid) || !inst.socket.exists() {
            let _ = fs::remove_file(&path);
            if !pid_alive(inst.pid) {
                // A killed or crashed instance leaves its socket behind.
                // Only ever delete an actual socket, whatever the stale
                // file says.
                use std::os::unix::fs::FileTypeExt;
                if fs::symlink_metadata(&inst.socket).is_ok_and(|m| m.file_type().is_socket()) {
                    let _ = fs::remove_file(&inst.socket);
                }
            }
            continue;
        }
        if !out.iter().any(|o| o.socket == inst.socket) {
            out.push(inst);
        }
    }
    out.sort_by(|a, b| b.started_at.cmp(&a.started_at).then(b.pid.cmp(&a.pid)));
    out
}

/// Find an instance: by pid, by socket path, or the newest live one.
pub fn find(pid: Option<u32>, socket: Option<&Path>) -> anyhow::Result<Instance> {
    let all = list();
    if let Some(s) = socket {
        return all.into_iter().find(|i| i.socket == s).ok_or_else(|| {
            anyhow::anyhow!("no discovery file for socket {} (pass --token or set ZEBRA_TOKEN)", s.display())
        });
    }
    if let Some(p) = pid {
        return all.into_iter().find(|i| i.pid == p).ok_or_else(|| anyhow::anyhow!("no running Zebra instance with pid {p}"));
    }
    all.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "Zebra isn't accepting AI control right now. Either Zebra isn't open, or \"Let AI agents control Zebra\" is switched off. Ask the person to open Zebra and switch it on (the AI button in the toolbar); an agent cannot turn it on. Developers: `zebra --headless --api` runs a private instance. (Looked in: {})",
            search_dirs().iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
        )
    })
}

/// Default socket path: `$TMPDIR/zebra-<uid>/<name>.sock` (per-user, 0700
/// parent). Falls back to `/tmp/zebra-<uid>/` if that would exceed the
/// 104-byte `sun_path` limit on macOS.
pub fn default_socket_path(name: &str) -> std::io::Result<PathBuf> {
    // SAFETY: getuid never fails.
    let uid = unsafe { libc::getuid() };
    let mut dir = match group_dir() {
        // Sandboxed builds: the App Group is the only place both the app and
        // the bundled zebractl can reach.
        Some(g) if std::env::var_os("ZEBRA_DATA_DIR").is_none() => g.join("s"),
        _ => std::env::temp_dir().join(format!("zebra-{uid}")),
    };
    if dir.join(format!("{name}.sock")).as_os_str().len() > 100 {
        dir = PathBuf::from(format!("/tmp/zebra-{uid}"));
    }
    ensure_private_dir(&dir)?;
    sweep_dead_sockets(&dir);
    // Refuse a directory someone else owns.
    let meta = fs::metadata(&dir)?;
    use std::os::unix::fs::MetadataExt;
    if meta.uid() != uid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} is owned by another user", dir.display()),
        ));
    }
    Ok(dir.join(format!("{name}.sock")))
}

/// Remove `<pid>.sock` / `<pid>-<n>.sock` files whose process is gone
/// (killed or crashed instances never ran their cleanup).
pub fn sweep_dead_sockets(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "sock") {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let Ok(pid) = stem.split('-').next().unwrap_or("").parse::<u32>() else { continue };
        if !pid_alive(pid) {
            let _ = fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod sweep_tests {
    #[test]
    fn dead_sockets_are_swept_live_ones_kept() {
        let dir = std::env::temp_dir().join(format!("zebra-sweep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let me = std::process::id();
        for name in [format!("{me}.sock"), "2147483000-1.sock".into(), "notes.txt".into()] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        super::sweep_dead_sockets(&dir);
        assert!(dir.join(format!("{me}.sock")).exists(), "live process keeps its socket");
        assert!(!dir.join("2147483000-1.sock").exists(), "dead process's socket is removed");
        assert!(dir.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
