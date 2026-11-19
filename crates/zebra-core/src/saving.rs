//! Saving on a worker thread.
//!
//! Writing a 200,000-element deck takes seconds (serialising, deflating,
//! writing every asset); done on the main thread it froze the window.
//! [`Editor::start_save`] writes a snapshot of the document on a worker;
//! the host polls with [`Editor::poll_save`] (each frame, or each pump of
//! the API), which marks the document saved at the revision the snapshot
//! was taken (edits made meanwhile keep it edited) and points its assets
//! at the new file. One save per document at a time: a second waits.
//!
//! Whoever started a save takes its result with [`Editor::take_saved`]:
//! the app shows a failure to the person, the API replies to the agent.
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::editor::{Editor, Origin};
use crate::error::{ZError, ZResult};
use crate::model::AssetBytes;

/// A save in flight.
pub(crate) struct SaveJob {
    path: PathBuf,
    by: Origin,
    revision: u64,
    /// `Editor::loads` then: a document loaded meanwhile isn't this one.
    loads: u64,
    rx: Receiver<ZResult<Vec<(String, AssetBytes)>>>,
}

/// A finished save, for whoever started it.
#[derive(Debug, Clone)]
pub struct Saved {
    pub path: PathBuf,
    pub by: Origin,
    pub result: ZResult<()>,
}

/// Finished saves nobody took are dropped after this many.
const KEEP_DONE: usize = 8;

impl Editor {
    /// Start writing the document to `path` on a worker thread. `wake` is
    /// called from the worker when it is done (to schedule a poll).
    /// Refused with `busy` while another save of this document runs.
    pub fn start_save(&mut self, path: PathBuf, by: Origin, wake: Box<dyn Fn() + Send>) -> ZResult<()> {
        if let Some(j) = &self.saving {
            return Err(ZError::new("busy", format!("{} is being saved", j.path.display())).hint("wait for that save to finish"));
        }
        // The snapshot: assets are shared (Arc or mapped), not copied.
        let doc = self.doc.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let to = path.clone();
        std::thread::Builder::new()
            .name("zebra-save".into())
            .spawn(move || {
                let r = crate::io::write_and_map(&doc, &to);
                drop(doc);
                let _ = tx.send(r);
                wake();
            })
            .map_err(|e| ZError::new("io", format!("cannot start saving: {e}")))?;
        self.saving = Some(SaveJob { path, by, revision: self.revision, loads: self.loads, rx });
        Ok(())
    }

    /// The file being saved to, while a save runs.
    pub fn saving(&self) -> Option<&Path> {
        self.saving.as_ref().map(|j| j.path.as_path())
    }

    /// Finish a save that is done (without waiting). True if one finished.
    pub fn poll_save(&mut self) -> bool {
        let r = match self.saving.as_ref().map(|j| j.rx.try_recv()) {
            None | Some(Err(TryRecvError::Empty)) => return false,
            Some(Ok(r)) => r,
            Some(Err(TryRecvError::Disconnected)) => Err(ZError::new("io", "the save stopped unexpectedly")),
        };
        self.finish_save(r);
        true
    }

    /// Wait for a save in flight to finish (quitting). True if one did.
    pub fn wait_save(&mut self) -> bool {
        let Some(j) = &self.saving else { return false };
        let r = j.rx.recv().unwrap_or_else(|_| Err(ZError::new("io", "the save stopped unexpectedly")));
        self.finish_save(r);
        true
    }

    fn finish_save(&mut self, r: ZResult<Vec<(String, AssetBytes)>>) {
        let Some(j) = self.saving.take() else { return };
        let result = r.map(|mapped| {
            if self.loads == j.loads {
                crate::io::adopt_mapped(&mut self.doc, mapped);
                self.mark_saved_at(j.revision, Some(j.path.clone()));
            }
        });
        self.saved.push(Saved { path: j.path, by: j.by, result });
        if self.saved.len() > KEEP_DONE {
            self.saved.remove(0);
        }
    }

    /// Take the finished saves `who` started.
    pub fn take_saved(&mut self, who: impl Fn(&Saved) -> bool) -> Vec<Saved> {
        let (mine, rest) = std::mem::take(&mut self.saved).into_iter().partition(|s| who(s));
        self.saved = rest;
        mine
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;

    fn add(ed: &mut Editor, x: f64) {
        let c: Command = serde_json::from_value(serde_json::json!({"op":"add_shape","shape":"rectangle","x":x,"y":0})).unwrap();
        ed.apply(c, Origin::Human).unwrap();
    }

    #[test]
    fn a_save_runs_aside_and_edits_made_meanwhile_stay_unsaved() {
        let dir = std::env::temp_dir().join(format!("zebra-saving-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.zebra");
        let mut ed = Editor::default();
        add(&mut ed, 0.0);
        ed.start_save(path.clone(), Origin::Human, Box::new(|| {})).unwrap();
        assert_eq!(ed.saving(), Some(path.as_path()));
        // A second save of the same document waits for the first.
        let e = ed.start_save(path.clone(), Origin::agent("a"), Box::new(|| {})).unwrap_err();
        assert_eq!(e.code, "busy");
        add(&mut ed, 100.0);
        assert!(ed.wait_save());
        assert!(ed.saving().is_none());
        let done = ed.take_saved(|s| s.by == Origin::Human);
        assert_eq!(done.len(), 1);
        assert!(done[0].result.is_ok());
        assert_eq!(ed.file_path.as_deref(), Some(path.as_path()));
        assert!(ed.dirty(), "the edit made during the save isn't in the file");
        assert_eq!(crate::io::load(&path).unwrap().elements.len(), 1, "the file has the snapshot");
        // Saved again, it is clean.
        ed.start_save(path.clone(), Origin::Human, Box::new(|| {})).unwrap();
        while !ed.poll_save() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!ed.dirty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_save_is_reported_to_whoever_started_it() {
        let mut ed = Editor::default();
        add(&mut ed, 0.0);
        let path = PathBuf::from("/nonexistent-dir-for-zebra/x.zebra");
        ed.start_save(path, Origin::agent("bot"), Box::new(|| {})).unwrap();
        ed.wait_save();
        assert!(ed.take_saved(|s| s.by == Origin::Human).is_empty());
        let done = ed.take_saved(|s| s.by == Origin::agent("bot"));
        assert_eq!(done.len(), 1);
        assert!(done[0].result.is_err());
        assert!(ed.dirty());
        assert!(ed.file_path.is_none());
    }
}
