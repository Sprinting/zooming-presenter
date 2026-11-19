//! File › Import…: other programs' files as Zebra pages.
//!
//! A [`Reader`] turns a file into a [`zebra_core::import::ImportedDeck`];
//! [`zebra_core::Editor::import_deck`] puts the deck on the canvas (one
//! frame per page, a path through them, one undo step). Readers run off
//! the UI thread: they report pages done and stop when asked
//! ([`Control`]).
//!
//! A PDF's pages come in as pictures drawn from the PDF (see [`pdf`]).
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub use zebra_core::import::{ImportNote, ImportedDeck};
use zebra_core::Theme;

pub mod pdf;

/// The file types File › Import… offers (lower case, no dot).
pub const EXTENSIONS: &[&str] = &["pdf"];

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ImportError {
    /// Not a file type Zebra can import.
    #[error("Zebra can't import this kind of file")]
    NotSupported,
    /// The file is damaged or not what its name says.
    #[error("the file is damaged or isn't a {0} file")]
    Damaged(&'static str),
    #[error("the file is protected with a password")]
    Encrypted,
    /// The PDF engine is missing from the app (a broken install).
    #[error("the PDF reader is missing from Zebra ({0})")]
    NoEngine(String),
    #[error("cannot read the file: {0}")]
    Read(String),
    #[error("the import was cancelled")]
    Cancelled,
}

/// Progress and cancel, shared with whoever started the import.
#[derive(Debug, Clone, Default)]
pub struct Control {
    done: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
}

impl Control {
    /// Pages read so far.
    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub(crate) fn page_done(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }
}

/// What to import.
#[derive(Clone)]
pub struct Options {
    /// First and last page, 1-based and inclusive. Clamped to the file.
    pub first: usize,
    pub last: usize,
    /// The document's theme (readers that bring in text rank its sizes
    /// into the theme's styles).
    pub theme: Theme,
    /// The frame name for page `n` (1-based), in the person's language.
    pub page_name: Arc<dyn Fn(usize) -> String + Send + Sync>,
}

impl Options {
    pub fn all(theme: Theme) -> Options {
        Options { first: 1, last: usize::MAX, theme, page_name: Arc::new(|n| format!("Page {n}")) }
    }
}

/// An opened file, ready to import.
pub trait Reader: Send {
    /// "PDF", "PowerPoint".
    fn kind(&self) -> &'static str;
    fn page_count(&self) -> usize;
    fn read(&self, opts: &Options, ctl: &Control) -> Result<ImportedDeck, ImportError>;
}

/// Open `path` with the reader its contents call for.
pub fn open(path: &Path) -> Result<Box<dyn Reader>, ImportError> {
    let bytes = std::fs::read(path).map_err(|e| ImportError::Read(e.to_string()))?;
    let title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    open_bytes(bytes, title)
}

/// Open a file's bytes; `title` names the import (the file name without
/// its extension).
pub fn open_bytes(bytes: Vec<u8>, title: String) -> Result<Box<dyn Reader>, ImportError> {
    if zebra_core::import::is_pdf(&bytes) {
        return Ok(Box::new(pdf::PdfReader::new(bytes, title)?));
    }
    Err(ImportError::NotSupported)
}

/// The pages `opts` asks for, as 0-based indices into `count` pages.
pub(crate) fn page_range(opts: &Options, count: usize) -> std::ops::Range<usize> {
    let first = opts.first.max(1).min(count.max(1)) - 1;
    let last = opts.last.max(1).min(count);
    first..last.max(first)
}
