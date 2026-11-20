//! Document Warnings, as Keynote's: problems met opening, importing or
//! exporting a document, which the person can review at any time (View >
//! Show Document Warnings) and copy. Nothing Zebra leaves out of a
//! document goes unsaid: it is a warning here.
//!
//! Kept for the session (not written to the file).

/// One problem met with the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// A picture, video, sound or font in the file that couldn't be read,
    /// so it shows as missing. `disk_full`: there wasn't room on the disk
    /// to unpack it.
    AssetNotRead { key: String, kind: MediaWord, disk_full: bool },
    /// What an import of `file` changed or left out: one line of the
    /// import's summary (`count` of `note`, on these `slides`).
    Import { file: String, note: crate::import::ImportNote, count: usize, slides: Vec<usize> },
}

impl Warning {
    /// The warning in English, for agents (the app has its own words).
    pub fn english(&self) -> String {
        match self {
            Warning::AssetNotRead { key, kind, disk_full: true } => format!("a {} (asset {key}) couldn't be unpacked: the disk is full; it shows as missing", kind.english()),
            Warning::AssetNotRead { key, kind, disk_full: false } => format!("a {} (asset {key}) couldn't be read; it shows as missing", kind.english()),
            Warning::Import { file, note, count, slides } => format!("{file}: {}", note.english_with(*count, slides)),
        }
    }
}

impl crate::model::Document {
    /// Keep what an import of `file` changed or left out as warnings.
    pub fn warn_import(&mut self, file: &str, deck: &crate::import::ImportedDeck) {
        for (note, count) in &deck.notes {
            let slides = deck.slides.get(note).cloned().unwrap_or_default();
            self.warnings.push(Warning::Import { file: file.to_string(), note: *note, count: *count, slides });
        }
    }
}

/// What a media file is, for the words of a warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaWord {
    Picture,
    Video,
    Sound,
    Font,
    Other,
}

impl MediaWord {
    /// From a file extension.
    pub fn of_ext(ext: &str) -> MediaWord {
        match ext.to_ascii_lowercase().as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "pdf" | "heic" | "tif" | "tiff" | "bmp" => MediaWord::Picture,
            "mp4" | "mov" | "m4v" => MediaWord::Video,
            "m4a" | "mp3" | "wav" | "aac" | "aif" | "aiff" => MediaWord::Sound,
            "ttf" | "otf" | "ttc" => MediaWord::Font,
            _ => MediaWord::Other,
        }
    }
    pub fn english(self) -> &'static str {
        match self {
            MediaWord::Picture => "picture",
            MediaWord::Video => "video",
            MediaWord::Sound => "sound",
            MediaWord::Font => "font",
            MediaWord::Other => "media file",
        }
    }
}

/// A document's warnings. Not part of its content: two documents that
/// differ only in their warnings are equal, and undo leaves them alone.
#[derive(Debug, Clone, Default)]
pub struct Warnings(pub Vec<Warning>);

impl PartialEq for Warnings {
    fn eq(&self, _: &Warnings) -> bool {
        true
    }
}

impl std::ops::Deref for Warnings {
    type Target = Vec<Warning>;
    fn deref(&self) -> &Vec<Warning> {
        &self.0
    }
}

impl std::ops::DerefMut for Warnings {
    fn deref_mut(&mut self) -> &mut Vec<Warning> {
        &mut self.0
    }
}
