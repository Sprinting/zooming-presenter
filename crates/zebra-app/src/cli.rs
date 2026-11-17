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
    format!(
        "Zebra (a zooming presentation app) is open on my Mac and I've allowed AI control. \
         Run `{ctl} --help` to learn how to drive it, then `{ctl} describe` to see what's on the canvas."
    )
}

/// An MCP server entry for apps that take a JSON config.
pub fn mcp_config(ctl: &str) -> String {
    let cmd = ctl.trim_matches('\'');
    format!("{{\n  \"mcpServers\": {{\n    \"zebra\": {{ \"command\": {}, \"args\": [\"mcp\"] }}\n  }}\n}}", serde_json::to_string(cmd).unwrap_or_default())
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
    let b = bundled().ok_or_else(|| anyhow::anyhow!("zebractl isn't inside this copy of Zebra"))?;
    let path = b.display().to_string().replace('\\', "\\\\").replace('"', "\\\"").replace('\'', "'\\''");
    let script = format!(
        "do shell script \"mkdir -p /usr/local/bin && ln -sf '{path}' {LINK}\" with administrator privileges with prompt \"Zebra wants to install the zebractl command so AI agents can use it.\""
    );
    let out = std::process::Command::new("/usr/bin/osascript").arg("-e").arg(script).output()?;
    if out.status.success() {
        Ok(())
    } else {
        let e = String::from_utf8_lossy(&out.stderr);
        if e.contains("-128") {
            anyhow::bail!("cancelled");
        }
        anyhow::bail!("{}", e.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mcp_config_is_json() {
        let v: serde_json::Value = serde_json::from_str(&mcp_config("'/Applications/My Zebra.app/Contents/MacOS/zebractl'")).unwrap();
        assert_eq!(v["mcpServers"]["zebra"]["command"], "/Applications/My Zebra.app/Contents/MacOS/zebractl");
        assert_eq!(quote("/a b/c"), "'/a b/c'");
    }
}
