//! The bundled `zebractl`: where it is, how to tell an agent about it, and
//! the optional one-click install into PATH (like VS Code's `code`).
//!
//! In `Zebra.app`, `zebractl` sits next to the app binary in
//! `Contents/MacOS/`, so an agent can always run it by its full path with
//! nothing installed. Installing a symlink in `/usr/local/bin` is only a
//! convenience, and is not offered in a sandboxed (App Store) build, which
//! can't write outside its container.
use std::path::PathBuf;

const LINK: &str = "/usr/local/bin/zebractl";

/// The bundled zebractl next to the running executable, if present.
pub fn bundled() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.join("zebractl");
    p.is_file().then_some(p)
}

/// True inside the macOS App Sandbox.
pub fn sandboxed() -> bool {
    std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some()
}

/// How an agent should invoke zebractl: the short name once installed,
/// else the bundled full path, else the bare name (dev builds on PATH).
pub fn zebractl_command() -> String {
    let installed = std::fs::canonicalize(LINK).ok();
    match bundled() {
        Some(b) if installed.as_ref() == std::fs::canonicalize(&b).ok().as_ref() => "zebractl".into(),
        Some(b) => quote(&b.display().to_string()),
        None => "zebractl".into(),
    }
}

fn quote(s: &str) -> String {
    if s.contains([' ', '\'', '"', '(', ')', '&']) {
        format!("'{}'", s.replace('\'', r"'\''"))
    } else {
        s.to_string()
    }
}

/// The text a person pastes into any agent's chat.
pub fn agent_prompt(ctl: &str) -> String {
    crate::t!("cli-agent-prompt", ctl = ctl)
}

/// `None` when installing isn't offered (no bundled binary, sandboxed, or
/// not macOS); otherwise whether the link already points at this bundle.
pub fn install_state() -> Option<bool> {
    if !cfg!(target_os = "macos") || sandboxed() {
        return None;
    }
    let b = bundled()?;
    Some(std::fs::canonicalize(LINK).ok() == std::fs::canonicalize(b).ok())
}

/// Symlink the bundled zebractl into /usr/local/bin, asking for an admin
/// password through the standard macOS prompt.
pub fn install() -> anyhow::Result<()> {
    let b = bundled().ok_or_else(|| anyhow::anyhow!(crate::t!("cli-error-not-bundled")))?;
    let script = install_script(&b.display().to_string(), &crate::t!("cli-install-prompt"));
    let out = std::process::Command::new("/usr/bin/osascript").arg("-e").arg(script).output()?;
    if out.status.success() {
        Ok(())
    } else {
        let e = String::from_utf8_lossy(&out.stderr);
        if e.contains("-128") {
            anyhow::bail!(crate::t!("cli-error-cancelled"));
        }
        anyhow::bail!("{}", e.trim())
    }
}

/// The AppleScript that makes the link. The path is an AppleScript
/// string, quoted for the shell by AppleScript itself (`quoted form of`):
/// quoting it by hand broke on an apostrophe in the app's path.
fn install_script(path: &str, prompt: &str) -> String {
    let lit = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    format!(
        "do shell script \"mkdir -p /usr/local/bin && ln -sf \" & quoted form of {} & \" {LINK}\" with administrator privileges with prompt {}",
        lit(path),
        lit(prompt)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_apostrophe_in_the_app_path_survives() {
        let s = install_script("/Users/k/Kartik's \"Apps\"/Zebra.app/Contents/MacOS/zebractl", "Zebra wants to install \"zebractl\"");
        assert!(s.contains(r#"quoted form of "/Users/k/Kartik's \"Apps\"/Zebra.app/Contents/MacOS/zebractl""#), "{s}");
        assert!(!s.contains(r"\'"), "no backslash-apostrophe, which AppleScript rejects: {s}");
        assert_eq!(quote("/Users/k/Kartik's Apps/zebractl"), r"'/Users/k/Kartik'\''s Apps/zebractl'");
    }

    #[test]
    fn quote_wraps_a_path_with_spaces() {
        assert_eq!(quote("/a b/c"), "'/a b/c'");
    }
}
