//! Files the system asks Zebra to open: a double-click in Finder, a drop on
//! the Dock icon, Open With, Spotlight. On macOS they arrive through the app
//! delegate (`application:openURLs:`, see `mac::install_open_hook`), often
//! before the first window exists, so they wait here. The launch takes the
//! first one in place of the template chooser ([`launch_file`]); the shell
//! turns the rest, and any that come later, into `Action::OpenPath`
//! ([`take`]).
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Paths waiting to be opened, oldest first.
#[derive(Default)]
pub struct OpenQueue {
    paths: Mutex<Vec<PathBuf>>,
}

impl OpenQueue {
    pub const fn new() -> Self {
        OpenQueue { paths: Mutex::new(Vec::new()) }
    }

    pub fn push(&self, paths: impl IntoIterator<Item = PathBuf>) {
        if let Ok(mut q) = self.paths.lock() {
            q.extend(paths);
        }
    }

    /// Everything waiting, oldest first.
    pub fn take(&self) -> Vec<PathBuf> {
        self.paths.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
    }

    /// The file a launch opens: the one on the command line, else the first
    /// one the system asked for (the rest stay queued for the shell).
    pub fn launch_file(&self, cli: Option<PathBuf>) -> Option<PathBuf> {
        if cli.is_some() {
            return cli;
        }
        let mut q = self.paths.lock().ok()?;
        (!q.is_empty()).then(|| q.remove(0))
    }
}

static QUEUE: OpenQueue = OpenQueue::new();
static WAKE: OnceLock<egui::Context> = OnceLock::new();

/// Queue files to open and wake the app to open them.
pub fn push(paths: impl IntoIterator<Item = PathBuf>) {
    QUEUE.push(paths);
    if let Some(c) = WAKE.get() {
        c.request_repaint();
    }
}

/// Files waiting to be opened.
pub fn take() -> Vec<PathBuf> {
    QUEUE.take()
}

/// See [`OpenQueue::launch_file`].
pub fn launch_file(cli: Option<PathBuf>) -> Option<PathBuf> {
    QUEUE.launch_file(cli)
}

/// The context to wake when files arrive (call once the app exists).
pub fn set_wake(ctx: &egui::Context) {
    let _ = WAKE.set(ctx.clone());
    if QUEUE.paths.lock().is_ok_and(|q| !q.is_empty()) {
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_wait_in_order_and_drain_once() {
        let q = OpenQueue::new();
        q.push([PathBuf::from("/a.zebra")]);
        q.push([PathBuf::from("/b.zebra"), PathBuf::from("/c.zebra")]);
        assert_eq!(q.take(), vec![PathBuf::from("/a.zebra"), PathBuf::from("/b.zebra"), PathBuf::from("/c.zebra")]);
        assert!(q.take().is_empty());
    }

    /// A double-click that launches Zebra: the file is the launch file (no
    /// template chooser), and a second file waits for the shell.
    #[test]
    fn a_launch_takes_the_first_file_the_system_sent() {
        let q = OpenQueue::new();
        assert_eq!(q.launch_file(None), None, "nothing sent: a plain launch");
        q.push([PathBuf::from("/a.zebra"), PathBuf::from("/b.zebra")]);
        assert_eq!(q.launch_file(None), Some(PathBuf::from("/a.zebra")));
        assert_eq!(q.take(), vec![PathBuf::from("/b.zebra")]);
    }

    #[test]
    fn the_command_line_file_comes_first() {
        let q = OpenQueue::new();
        q.push([PathBuf::from("/a.zebra")]);
        assert_eq!(q.launch_file(Some("/cli.zebra".into())), Some(PathBuf::from("/cli.zebra")));
        assert_eq!(q.take(), vec![PathBuf::from("/a.zebra")], "still opened, in a window of its own");
    }
}
