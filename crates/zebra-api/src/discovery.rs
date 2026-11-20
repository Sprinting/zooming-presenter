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

/// True in builds that honour the folder overrides `ZEBRA_DATA_DIR` and
/// `ZEBRA_GROUP_DIR`: debug builds (tests) and builds with the
/// `dev-overrides` feature (scripts/perf.sh). Never the app people install:
/// pointing Zebra at a made-up data folder would skip the person's first
/// switch-on of AI control (see [`human_grant_file`]).
pub const DEV_OVERRIDES: bool = cfg!(any(debug_assertions, feature = "dev-overrides"));

/// A folder override from the environment, in dev builds only.
pub(crate) fn dev_env(name: &str) -> Option<std::ffi::OsString> {
    if DEV_OVERRIDES { std::env::var_os(name) } else { None }
}

/// Folder overrides set in the environment that this build ignores, so
/// the binaries can say so instead of silently using the real folders.
pub fn ignored_overrides() -> Vec<&'static str> {
    if DEV_OVERRIDES {
        return vec![];
    }
    ["ZEBRA_DATA_DIR", "ZEBRA_GROUP_DIR"].into_iter().filter(|n| std::env::var_os(n).is_some()).collect()
}

/// The App Group container, if this process can use it (macOS only; it
/// exists once a process with the entitlement has run).
pub fn group_dir() -> Option<PathBuf> {
    if let Some(d) = dev_env("ZEBRA_GROUP_DIR") {
        return Some(PathBuf::from(d));
    }
    if !cfg!(target_os = "macos") {
        return None;
    }
    let d = dirs::home_dir()?.join("Library/Group Containers").join(APP_GROUP_ID);
    d.is_dir().then_some(d)
}

/// Where this process writes: `$ZEBRA_DATA_DIR` (dev builds only), else the App Group
/// container (sandboxed builds), else `<data_dir>/Zebra`.
pub fn app_dir() -> PathBuf {
    if let Some(d) = dev_env("ZEBRA_DATA_DIR") {
        return PathBuf::from(d);
    }
    if let Some(g) = group_dir() {
        return g.join("Zebra");
    }
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("Zebra")
}

/// The person's AI control switch, as Zebra keeps it between launches.
///
/// `granted`: a person has switched AI control on in Zebra's own window at
/// least once. Until then launch flags (`--api`, debug demos)
/// can't switch it on: the first time is always a person. `on`: the
/// person's last choice (None: not kept here; older files).
///
/// In a signed Mac build it lives in the login keychain, in an item only
/// Zebra's own code signature may read or change: another app or script
/// that tries gets a macOS prompt. The item is per signing certificate, so
/// a build signed with another certificate (Developer ID, App Store) starts
/// with AI control off and a person switches it on once, instead of macOS
/// asking about the old item. Elsewhere (and in tests) it is a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AiSwitch {
    pub granted: bool,
    pub on: Option<bool>,
}

impl AiSwitch {
    /// May a launch flag (`--api`) start AI control? Only
    /// once a person has switched it on, and not after the person or an
    /// agent switched it off since (`on == None` is an older grant that
    /// didn't keep the choice: it counts as on).
    pub fn lets_flags_start(&self) -> bool {
        self.granted && self.on != Some(false)
    }

    /// Why a launch flag can't start AI control now, for the person who
    /// ran it (None: it can).
    pub fn flag_refusal(&self) -> Option<&'static str> {
        if !self.granted {
            Some("AI control stays off until a person switches it on in Zebra once (the AI button in the toolbar). After that, --api works.")
        } else if self.on == Some(false) {
            Some("AI control was switched off, so --api stays off until a person switches it back on in Zebra (the AI button in the toolbar).")
        } else {
            None
        }
    }
}

/// The file that keeps the switch where the keychain isn't used.
pub fn human_grant_file() -> PathBuf {
    app_dir().join("ai-control-granted")
}

/// Where the switch is kept.
enum Store {
    File(PathBuf),
    #[cfg(target_os = "macos")]
    Keychain { account: String },
}

fn store() -> Store {
    #[cfg(target_os = "macos")]
    if dev_env("ZEBRA_DATA_DIR").is_none() {
        if let Some(signer) = keychain::signer() {
            return Store::Keychain { account: format!("signer-{signer}") };
        }
    }
    Store::File(human_grant_file())
}

fn parse(v: &[u8]) -> AiSwitch {
    match v {
        b"on" => AiSwitch { granted: true, on: Some(true) },
        b"off" => AiSwitch { granted: true, on: Some(false) },
        _ => AiSwitch { granted: true, on: None },
    }
}

/// The switch as last kept.
pub fn ai_switch() -> AiSwitch {
    match store() {
        Store::File(f) => fs::read(f).map(|v| parse(&v)).unwrap_or_default(),
        #[cfg(target_os = "macos")]
        Store::Keychain { account } => keychain::get(&account).map(|v| parse(&v)).unwrap_or_default(),
    }
}

/// Has a person ever switched on AI control in Zebra's window?
pub fn human_has_granted() -> bool {
    ai_switch().granted
}

/// Keep the switch's new state. Switching on is only ever the person in
/// Zebra's window (no API method or launch flag reaches this with `true`);
/// switching off is the person or an agent. Off before any grant keeps
/// nothing, so it can't count as one.
pub fn record_ai_switch(on: bool) -> std::io::Result<()> {
    if !on && !human_has_granted() {
        return Ok(());
    }
    let v: &[u8] = if on { b"on" } else { b"off" };
    match store() {
        Store::File(f) => {
            fs::create_dir_all(app_dir())?;
            fs::write(f, v)
        }
        #[cfg(target_os = "macos")]
        Store::Keychain { account } => keychain::set(&account, v),
    }
}

/// Record that a person switched on AI control (see [`record_ai_switch`]).
pub fn record_human_grant() -> std::io::Result<()> {
    record_ai_switch(true)
}

/// The keychain item and the signature it is tied to.
#[cfg(target_os = "macos")]
mod keychain {
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::data::{CFData, CFDataRef};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::{CFString, CFStringRef};
    use security_framework::os::macos::code_signing::{Flags, SecCode};
    use sha2::{Digest, Sha256};
    use std::ffi::c_void;

    const SERVICE: &str = concat!("app.zebra.Zebra", ".ai-control");
    const NOT_FOUND: i32 = -25300;
    /// kSecCSSigningInformation
    const SIGNING_INFORMATION: u32 = 1 << 1;

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecCodeCopySigningInformation(code: *const c_void, flags: u32, info: *mut CFDictionaryRef) -> i32;
        fn SecCertificateCopyData(cert: *const c_void) -> CFDataRef;
        static kSecCodeInfoCertificates: CFStringRef;
    }

    /// The first 16 hex digits of the SHA-256 of the certificate Zebra is
    /// signed with. None when it has none (ad hoc or unsigned): such a
    /// build changes signature every time, so the keychain would ask each
    /// rebuild; it keeps the file instead.
    pub fn signer() -> Option<String> {
        let code = SecCode::for_self(Flags::NONE).ok()?;
        let mut info: CFDictionaryRef = std::ptr::null();
        // SAFETY: a valid code object; `info` is returned retained.
        let status = unsafe { SecCodeCopySigningInformation(code.as_concrete_TypeRef() as *const c_void, SIGNING_INFORMATION, &mut info) };
        if status != 0 || info.is_null() {
            return None;
        }
        let info: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_create_rule(info) };
        let key = unsafe { CFString::wrap_under_get_rule(kSecCodeInfoCertificates) };
        let certs = info.find(&key)?;
        let certs: CFArray<CFType> = unsafe { CFArray::wrap_under_get_rule(certs.as_CFTypeRef() as CFArrayRef) };
        let leaf = certs.get(0)?;
        // SAFETY: the array holds SecCertificate objects.
        let data = unsafe { SecCertificateCopyData(leaf.as_CFTypeRef()) };
        if data.is_null() {
            return None;
        }
        let data = unsafe { CFData::wrap_under_create_rule(data) };
        let hash = Sha256::digest(data.bytes());
        Some(hash.iter().take(8).map(|b| format!("{b:02x}")).collect())
    }

    /// The item's value. None when there is none (no grant yet), or macOS
    /// refused (the person said no to a prompt).
    pub fn get(account: &str) -> Option<Vec<u8>> {
        match security_framework::passwords::get_generic_password(SERVICE, account) {
            Ok(v) => Some(v),
            Err(e) if e.code() == NOT_FOUND => None,
            Err(e) => {
                eprintln!("zebra: couldn't read the AI control switch from the keychain: {e}");
                None
            }
        }
    }

    /// Create or change the item. Made by Zebra, it trusts only Zebra's
    /// signature.
    pub fn set(account: &str, v: &[u8]) -> std::io::Result<()> {
        security_framework::passwords::set_generic_password(SERVICE, account, v).map_err(|e| std::io::Error::other(e.to_string()))
    }
}

pub fn instances_dir() -> PathBuf {
    app_dir().join("instances")
}

/// Every place a running Zebra may have announced itself: ours, the App
/// Group, the unsandboxed data dir, and the sandboxed app's own container.
pub fn search_dirs() -> Vec<PathBuf> {
    if dev_env("ZEBRA_DATA_DIR").is_some() {
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

/// A note that Zebra is open, written whether AI control is on or off.
/// It holds no socket and no token: it only lets `zebractl` tell "Zebra
/// isn't open" from "AI control is off". Removed when dropped (the app
/// quitting); a crashed app's note is ignored and swept by pid.
pub struct RunningNote(PathBuf);

impl RunningNote {
    pub fn write() -> std::io::Result<RunningNote> {
        let dir = instances_dir();
        ensure_private_dir(&dir)?;
        let pid = std::process::id();
        let path = dir.join(format!("{pid}.running"));
        let body = serde_json::json!({ "pid": pid, "version": env!("CARGO_PKG_VERSION") });
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
        f.write_all(body.to_string().as_bytes())?;
        Ok(RunningNote(path))
    }
}

impl Drop for RunningNote {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Pids of Zebra apps that are open (with AI control on or off).
pub fn running_apps() -> Vec<u32> {
    let mut out = vec![];
    for entry in search_dirs().iter().filter_map(|d| fs::read_dir(d).ok()).flat_map(|rd| rd.flatten()) {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "running") {
            continue;
        }
        let pid = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        if pid_alive(pid) {
            out.push(pid);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// What to tell an agent when no instance accepts AI control.
pub fn not_accepting_message() -> String {
    match running_apps().as_slice() {
        [] => "Zebra isn't open. Ask the person to open Zebra and switch on AI control (the AI button in Zebra's toolbar). An agent can't switch it on.".to_string(),
        [pid] => format!("Zebra is open (pid {pid}), but AI control is switched off. Ask the person to switch it on (the AI button in Zebra's toolbar). An agent can't switch it on."),
        pids => format!(
            "Zebra is open (pids {}), but AI control is switched off. Ask the person to switch it on (the AI button in Zebra's toolbar). An agent can't switch it on.",
            pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")
        ),
    }
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
    all.into_iter().next().ok_or_else(|| anyhow::anyhow!("{}", not_accepting_message()))
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
        Some(g) if dev_env("ZEBRA_DATA_DIR").is_none() => g.join("s"),
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

#[cfg(test)]
mod switch_tests {
    use super::AiSwitch;

    #[test]
    fn flags_start_only_a_switch_that_is_not_off() {
        let never = AiSwitch::default();
        assert!(!never.lets_flags_start() && never.flag_refusal().is_some());
        let older = AiSwitch { granted: true, on: None };
        assert!(older.lets_flags_start() && older.flag_refusal().is_none());
        let on = AiSwitch { granted: true, on: Some(true) };
        assert!(on.lets_flags_start());
        let off = AiSwitch { granted: true, on: Some(false) };
        assert!(!off.lets_flags_start());
        assert!(off.flag_refusal().unwrap().contains("switched off"));
    }
}
