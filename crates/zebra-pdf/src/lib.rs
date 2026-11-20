//! PDF pages read and drawn in a separate process, `zebra-pdf-helper`.
//!
//! PDFium is a large C++ library fed files from anywhere. A crafted
//! 3.5 KB PDF made it allocate 12 GB and abort the whole app (a C++
//! `bad_alloc` can't be caught from Rust). So Zebra never runs PDFium in
//! its own process: every PDF goes through a helper ([`Pool`]) that the
//! app watches and kills when it uses more than [`limits::BUDGET`] of
//! memory or runs past a deadline, or when the person cancels. A page
//! that kills a helper is given up on for the session (its picture
//! shows instead); a PDF that kills it [`limits::KILLS_PER_PDF`] times,
//! or while opening, is given up on whole. New helpers start on demand,
//! after a growing wait when they keep dying, so one file can't make the
//! app start processes without end.
//!
//! The PDF isn't copied for the helper: it gets the file the app has
//! mapped (passed over the socket) and PDFium reads what it needs from
//! it. The helper is untrusted: everything it says is checked
//! ([`wire`]).
//!
//! On the Mac the helper is `Zebra.app/Contents/MacOS/zebra-pdf-helper`;
//! in the App Store build it inherits the app's sandbox (see
//! scripts/bundle-macos.sh).
pub mod limits;
#[doc(hidden)]
pub mod samples;
pub mod sys;
pub mod wire;
mod client;

pub use client::{helper_path, Lane, PdfError, Pool, Preview, Stats};
pub use limits::Limits;
