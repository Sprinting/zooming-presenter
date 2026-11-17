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

/// `<data_dir>/Zebra` (or `$ZEBRA_DATA_DIR`).
pub fn app_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ZEBRA_DATA_DIR") {
        return PathBuf::from(d);
    }
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("Zebra")
}

pub fn instances_dir() -> PathBuf {
    app_dir().join("instances")
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
    let mut out = vec![];
    let Ok(rd) = fs::read_dir(instances_dir()) else { return out };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(mut inst) = serde_json::from_str::<Instance>(&text) else { continue };
        inst.file = path.clone();
        if !pid_alive(inst.pid) || !inst.socket.exists() {
            let _ = fs::remove_file(&path);
            continue;
        }
        out.push(inst);
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
            "no running Zebra instance with the API enabled (looked in {}). Start one with `zebra --api` (GUI) or `zebra --headless --api`, or use `zebractl mcp --spawn-headless`",
            instances_dir().display()
        )
    })
}

/// Default socket path: `$TMPDIR/zebra-<uid>/<name>.sock` (per-user, 0700
/// parent). Falls back to `/tmp/zebra-<uid>/` if that would exceed the
/// 104-byte `sun_path` limit on macOS.
pub fn default_socket_path(name: &str) -> std::io::Result<PathBuf> {
    // SAFETY: getuid never fails.
    let uid = unsafe { libc::getuid() };
    let mut dir = std::env::temp_dir().join(format!("zebra-{uid}"));
    if dir.join(format!("{name}.sock")).as_os_str().len() > 100 {
        dir = PathBuf::from(format!("/tmp/zebra-{uid}"));
    }
    ensure_private_dir(&dir)?;
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
