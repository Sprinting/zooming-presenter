//! File › Import…: other programs' files as Zebra pages.
//!
//! A [`Reader`] turns a file into a [`zebra_core::import::ImportedDeck`];
//! [`zebra_core::Editor::import_deck`] puts the deck on the canvas (one
//! frame per page, a path through them, one undo step). Readers run off
//! the UI thread: they report pages done and stop when asked
//! ([`Control`]).
//!
//! A PDF's pages come in as pictures drawn from the PDF (see [`pdf`]); a
//! PowerPoint file's slides come in as Zebra's own text, shapes and
//! pictures (see [`pptx`]).
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub use zebra_core::import::{ImportNote, ImportedDeck};
use zebra_core::Theme;

pub mod pdf;
pub mod pptx;
mod pptx_xml;
mod pptx_media;
mod pptx_package;

/// The file types File › Import… offers (lower case, no dot).
pub const EXTENSIONS: &[&str] = &["pdf", "pptx"];

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
    /// A PowerPoint 97–2003 file (.ppt), which Zebra can't read.
    #[error("this is an old PowerPoint file (.ppt): save it as .pptx in PowerPoint or Keynote first")]
    OldPowerPoint,
    /// Every slide asked for is hidden.
    #[error("every slide chosen is hidden")]
    AllHidden,
    /// Every slide asked for that isn't hidden is over a limit (more than
    /// 8 MB of XML, or more shapes than can be read).
    #[error("every slide chosen is too big to import")]
    TooBig,
    /// The PDF engine is missing from the app (a broken install).
    #[error("the PDF reader is missing from Zebra ({0})")]
    NoEngine(String),
    #[error("cannot read the file: {0}")]
    Read(String),
    #[error("the import was cancelled")]
    Cancelled,
    /// A PowerPoint deck while beta features are off (importing PowerPoint
    /// is one, see `zebra_core::beta`).
    #[error("importing PowerPoint is a beta feature, and beta features are off")]
    BetaOff,
    /// The PDF needs more memory or time than Zebra gives it to open it
    /// (its helper was stopped while opening it). Pages that need too
    /// much come in as placeholders instead.
    #[error("the PDF needs too much memory or time to read")]
    TooDemanding,
    /// An agent asked for every page of a PDF with more than
    /// [`pdf::ASK_ABOVE_PAGES`] without saying which (`to`), as the
    /// import sheet asks a person; `pages`: how many the file has.
    #[error("the file has {pages} pages; say which to import")]
    NeedRange { pages: usize },
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
    /// Count a page done (readers call this; so can tests).
    pub fn page_done(&self) {
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
    /// The frame name for slide `n` (1-based) when it has no title.
    pub slide_name: Arc<dyn Fn(usize) -> String + Send + Sync>,
    /// The document has background music already: music a file plays
    /// across its slides stays a picture (noted) rather than replace it.
    pub has_soundtrack: bool,
}

impl Options {
    pub fn all(theme: Theme) -> Options {
        Options { first: 1, last: usize::MAX, theme, page_name: Arc::new(|n| format!("Page {n}")), slide_name: Arc::new(|n| format!("Slide {n}")), has_soundtrack: false }
    }
}

/// An opened file, ready to import.
pub trait Reader: Send {
    /// "PDF", "PowerPoint".
    fn kind(&self) -> &'static str;
    fn page_count(&self) -> usize;
    fn read(&self, opts: &Options, ctl: &Control) -> Result<ImportedDeck, ImportError>;
}

/// Open `path` with the reader its contents call for. The file is
/// memory-mapped, not read: a PowerPoint file's video or a PDF of any size
/// is streamed out of it, so importing costs disk (one copy of each
/// medium, in the document's own spill files), not memory.
pub fn open(path: &Path) -> Result<Box<dyn Reader>, ImportError> {
    open_with(path, &Control::default())
}

/// [`open`], stopped early when `ctl` is cancelled (counting a big PDF's
/// pages can take a while).
pub fn open_with(path: &Path, ctl: &Control) -> Result<Box<dyn Reader>, ImportError> {
    let map = zebra_core::io::map_document(path).map_err(|e| ImportError::Read(e.to_string()))?;
    let title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    open_source_with(map.into(), title, ctl)
}

/// Open a file's bytes; `title` names the import (the file name without
/// its extension).
pub fn open_bytes(bytes: Vec<u8>, title: String) -> Result<Box<dyn Reader>, ImportError> {
    open_source(bytes.into(), title)
}

/// Open a file's bytes, in memory or mapped.
pub fn open_source(bytes: zebra_core::AssetBytes, title: String) -> Result<Box<dyn Reader>, ImportError> {
    open_source_with(bytes, title, &Control::default())
}

fn open_source_with(bytes: zebra_core::AssetBytes, title: String, ctl: &Control) -> Result<Box<dyn Reader>, ImportError> {
    if zebra_core::import::is_pdf(&bytes) {
        return Ok(Box::new(pdf::PdfReader::new(bytes, title, ctl)?));
    }
    if pptx::is_pptx(&bytes) {
        return Ok(Box::new(pptx::PptxReader::new(bytes, title)?));
    }
    // Old PowerPoint files and password-protected .pptx files are both
    // OLE compound files.
    Err(pptx::compound_file_error(&bytes).unwrap_or(ImportError::NotSupported))
}

/// The pages `opts` asks for, as 0-based indices into `count` pages.
pub(crate) fn page_range(opts: &Options, count: usize) -> std::ops::Range<usize> {
    let first = opts.first.max(1).min(count.max(1)) - 1;
    let last = opts.last.max(1).min(count);
    first..last.max(first)
}
