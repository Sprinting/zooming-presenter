//! Pages read from another program's file (File › Import…, the API's
//! `import`): the pages laid out to the right of what is already on the
//! canvas inside one frame, with a path step per page, as one undo step.
//!
//! The readers live in `zebra-import`; this is the part every format
//! shares, so a PDF and a PowerPoint file land on the canvas the same way.
use std::collections::BTreeMap;

use base64::Engine as _;
use serde::Serialize;

use crate::clip::Clip;
use crate::command::Command;
use crate::editor::{Editor, Outcome};
use crate::error::{ZError, ZResult};
use crate::geom::Transform;
use crate::model::{Align, Asset, Element, ElementKind, FrameData, FrameStyle, MediaKind, Size, Soundtrack, StepTarget};
use crate::Origin;

/// What a reader made of a file.
#[derive(Debug, Clone, Default)]
pub struct ImportedDeck {
    /// The file's name without its extension.
    pub title: String,
    pub pages: Vec<ImportedPage>,
    /// Pictures the pages show, by content key ([`asset_from`]).
    pub assets: BTreeMap<String, Asset>,
    /// What could not come in as it was, and how often.
    pub notes: BTreeMap<ImportNote, usize>,
    /// Music the file plays across its pages (an asset in `assets`); it
    /// becomes the document's background music unless it has some.
    pub soundtrack: Option<Soundtrack>,
}

/// One page or slide. Coordinates are world units with the page's top-left
/// corner at (0, 0).
#[derive(Debug, Clone, Default)]
pub struct ImportedPage {
    pub w: f64,
    pub h: f64,
    /// The frame's name ("Page 3", a slide title).
    pub name: String,
    /// Back to front.
    pub items: Vec<ImportedItem>,
    /// Speaker notes for the page's path step.
    pub notes: String,
    /// A PowerPoint slide: always framed (even when it holds one object),
    /// and its frame shows deck-style when presenting ([`FrameData::slide`]).
    pub slide: bool,
    /// Sound the page plays (an asset in [`ImportedDeck::assets`]): its
    /// path step's voice-over.
    pub voiceover: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImportedItem {
    pub element: Element,
    /// Text only: the box the text took up in the source. The theme font
    /// is not the source's, so the text is measured again and kept at the
    /// source's edge (left, centre or right, by its alignment), made smaller
    /// when it would run wider than the source did.
    pub fit: Option<SourceBox>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    /// Where the text sits in the box from top to bottom.
    pub anchor: BoxAnchor,
}

/// Where text sits in its [`SourceBox`] from top to bottom. A PDF's box is
/// the text's own ink, so its text is centred on it; a PowerPoint text box
/// can be taller than its text and says where the text goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxAnchor {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// Something that came in differently from the source (the summary sheet
/// after an import lists these).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportNote {
    /// Pages that could not be read were left out.
    PageSkipped,
    /// Hidden slides were left out.
    HiddenSlide,
    /// Charts with no picture of them in the file were left out.
    Chart,
    /// SmartArt graphics were left out.
    SmartArt,
    /// Tables were left out.
    Table,
    /// Equations were left out.
    Equation,
    /// Videos and sounds that could not be taken out of the file, or that
    /// won't play on a Mac (WMV, AVI…), came in as their poster pictures.
    Media,
    /// Embedded objects came in as the picture the file keeps of them.
    AsPicture,
    /// Shapes Zebra has no match for (custom shapes, curved connectors…)
    /// were left out; their text came in.
    Shape,
    /// EMF and WMF pictures with no other picture beside them were left out.
    Metafile,
    /// Pictures in other formats (TIFF, BMP…) or that could not be read
    /// were left out.
    Picture,
    /// Flipped objects Zebra can't flip are shown unflipped.
    Flip,
    /// Gradients came in as their first colour.
    Gradient,
    /// Other objects (ink, add-in content…) were left out.
    Other,
}

impl ImportNote {
    pub const ALL: [ImportNote; 14] = [
        ImportNote::PageSkipped,
        ImportNote::HiddenSlide,
        ImportNote::Chart,
        ImportNote::SmartArt,
        ImportNote::Table,
        ImportNote::Equation,
        ImportNote::Media,
        ImportNote::AsPicture,
        ImportNote::Shape,
        ImportNote::Metafile,
        ImportNote::Picture,
        ImportNote::Flip,
        ImportNote::Gradient,
        ImportNote::Other,
    ];

    /// English, for the API and the undo history of agents.
    pub fn english(self, n: usize) -> String {
        let (one, many) = match self {
            ImportNote::PageSkipped => ("unreadable page left out", "unreadable pages left out"),
            ImportNote::HiddenSlide => ("hidden slide left out", "hidden slides left out"),
            ImportNote::Chart => ("chart left out", "charts left out"),
            ImportNote::SmartArt => ("SmartArt graphic left out", "SmartArt graphics left out"),
            ImportNote::Table => ("table left out", "tables left out"),
            ImportNote::Equation => ("equation left out", "equations left out"),
            ImportNote::Media => ("video or sound imported as its poster picture", "videos and sounds imported as their poster pictures"),
            ImportNote::AsPicture => ("embedded object imported as a picture", "embedded objects imported as pictures"),
            ImportNote::Shape => ("unsupported shape left out (its text kept)", "unsupported shapes left out (their text kept)"),
            ImportNote::Metafile => ("EMF or WMF picture left out", "EMF or WMF pictures left out"),
            ImportNote::Picture => ("picture in an unsupported format left out", "pictures in unsupported formats left out"),
            ImportNote::Flip => ("flipped object shown unflipped", "flipped objects shown unflipped"),
            ImportNote::Gradient => ("gradient made a solid colour", "gradients made solid colours"),
            ImportNote::Other => ("other object left out", "other objects left out"),
        };
        format!("{n} {}", if n == 1 { one } else { many })
    }
}

/// Is this a PDF? (PDF allows junk before its header, within 1 KB.)
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.len() >= 5 && bytes[..bytes.len().min(1024)].windows(5).any(|w| w == b"%PDF-")
}

/// A picture asset (PNG, JPEG, WebP, GIF or SVG), or the PDF that pages
/// are drawn from, and its content key, for [`ImportedDeck::assets`].
/// The bytes go through [`crate::media::spill`], like other new media,
/// so an imported file's pictures and PDF need not sit on the heap.
pub fn asset_from(bytes: Vec<u8>) -> ZResult<(String, Asset)> {
    let key = crate::editor::content_key(&bytes);
    let (ext, w, h) = if is_pdf(&bytes) {
        ("pdf".to_string(), 0, 0)
    } else if bytes.starts_with(b"<svg") {
        let (w, h) = crate::symbols::svg_pixels(&bytes);
        ("svg".to_string(), w, h)
    } else {
        crate::editor::image_info(&bytes)?
    };
    let bytes = crate::media::spill(bytes, &key);
    Ok((key, Asset { ext, bytes, width: w, height: h }))
}

/// A video or sound asset (MP4, MOV, M4V; MP3, M4A, AAC, WAV) and its
/// content key, for [`ImportedDeck::assets`], with what its header says
/// (kind, size, length). Other formats are refused: they won't play.
pub fn av_asset_from(bytes: Vec<u8>) -> ZResult<(String, Asset, crate::media::MediaInfo)> {
    let info = crate::media::info(&bytes)?;
    if !matches!(info.kind, MediaKind::Video | MediaKind::Audio) {
        return Err(crate::error::ZError::invalid("not a video or a sound"));
    }
    let key = crate::editor::content_key(&bytes);
    let bytes = crate::media::spill(bytes, &key);
    Ok((key, Asset { ext: info.ext.clone(), bytes, width: info.width, height: info.height }, info))
}

/// Pages per row on the canvas for `n` pages: a block about as wide as tall.
pub fn columns(n: usize) -> usize {
    (n as f64).sqrt().ceil().max(1.0) as usize
}

impl Editor {
    /// Add the pages of `deck` as one undo step named `label`: the pages in
    /// rows to the right of everything already on the canvas, one rectangle
    /// frame (named after the file) around them all, and a path step per
    /// page appended in page order (carrying the page's notes). A page that
    /// is a single picture (every PDF page) is its own step and carries the
    /// page's name; a page of several objects, and every slide, is framed
    /// on its own. Objects grouped on a page stay grouped. A page's sound becomes its step's voice-over; the deck's
    /// music the document's background music, unless it has some.
    pub fn import_deck(&mut self, deck: &ImportedDeck, label: String, origin: Origin) -> ZResult<Outcome> {
        if deck.pages.is_empty() {
            return Err(ZError::invalid("the file has no pages to import"));
        }
        let cw = deck.pages.iter().map(|p| p.w).fold(0.0, f64::max);
        let ch = deck.pages.iter().map(|p| p.h).fold(0.0, f64::max);
        if !(cw > 0.0 && ch > 0.0 && cw.is_finite() && ch.is_finite()) {
            return Err(ZError::invalid("the pages have no size"));
        }
        let gap = cw.max(ch) * 0.12;
        // Right of what's there, top edges level with its top.
        let (left, top) = match self.doc.bounds() {
            Some(b) => (b.x1 + gap * 1.5, b.y0),
            None => (-(cw + gap) * (columns(deck.pages.len()) as f64 - 1.0) / 2.0 - cw / 2.0, -ch / 2.0),
        };
        let cols = columns(deck.pages.len());
        let rows = deck.pages.len().div_ceil(cols);
        let theme = self.theme();
        let mut clip = Clip { elements: vec![], assets: deck.assets.clone() };
        let mut n = 0u64;
        let mut tmp = |prefix: &str| {
            n += 1;
            format!("import-{prefix}{n}")
        };
        // One frame around every page, first so it sits behind them. It
        // holds the pages ([`crate::model::frame_holds`]: they are smaller),
        // so moving, turning or resizing it carries the whole file.
        let pad = gap / 2.0;
        let (gw, gh) = (cols as f64 * (cw + gap) - gap, rows as f64 * (ch + gap) - gap);
        clip.elements.push(Element {
            id: tmp("f"),
            transform: Transform { x: left + gw / 2.0, y: top + gh / 2.0, scale: 1.0, rotation: 0.0 },
            size: Size { w: gw + 2.0 * pad, h: gh + 2.0 * pad },
            locked: false,
            name: deck.title.clone(),
            placeholder: false,
            group: None,
            kind: ElementKind::Frame(FrameData { style: FrameStyle::Rectangle, color: None, slide: false }),
        });
        // What each page's path step shows: a page that is one picture is
        // its own step; a page of several objects gets a frame of its own.
        let mut steps = vec![];
        for (i, p) in deck.pages.iter().enumerate() {
            let (col, row) = ((i % cols) as f64, (i / cols) as f64);
            // Centred in its cell, so pages of different sizes line up.
            let ox = left + col * (cw + gap) + (cw - p.w) / 2.0;
            let oy = top + row * (ch + gap) + (ch - p.h) / 2.0;
            let single = p.items.len() == 1 && !p.slide;
            if !single {
                let fid = tmp("f");
                clip.elements.push(Element {
                    id: fid.clone(),
                    transform: Transform { x: ox + p.w / 2.0, y: oy + p.h / 2.0, scale: 1.0, rotation: 0.0 },
                    // A hair bigger than the page, so what fills the whole
                    // slide (its background, a full-bleed picture) is held
                    // by the frame and moves with it (`frame_holds`).
                    size: Size { w: p.w * (1.0 + 1e-5), h: p.h * (1.0 + 1e-5) },
                    locked: false,
                    name: p.name.clone(),
                    placeholder: false,
                    group: None,
                    kind: ElementKind::Frame(FrameData { style: FrameStyle::Rectangle, color: None, slide: p.slide }),
                });
                steps.push((fid, p.notes.clone(), p.voiceover.clone()));
            }
            for it in &p.items {
                let mut e = it.element.clone();
                e.id = tmp(e.kind.id_prefix());
                // Groups are per page: the same name on two pages is two groups.
                e.group = e.group.take().map(|g| format!("import-p{i}-{g}"));
                if let (Some(b), ElementKind::Text(_)) = (it.fit, &e.kind) {
                    fit_text(self, &theme, &mut e, b);
                }
                e.transform.x += ox;
                e.transform.y += oy;
                if single {
                    e.name = p.name.clone();
                    steps.push((e.id.clone(), p.notes.clone(), p.voiceover.clone()));
                }
                clip.elements.push(e);
            }
        }
        // Sounds go in through the commands that set them, which take the
        // bytes; they are the assets the paste adds, so stored once.
        let b64 = |key: &str| deck.assets.get(key).map(|a| base64::engine::general_purpose::STANDARD.encode(&a.bytes[..]));
        let first_step = self.doc.path.len();
        let music = deck.soundtrack.as_ref().filter(|_| self.doc.soundtrack.is_none());
        self.paste_clip_then(&clip, 0.0, 0.0, Some(label), origin, |ids| {
            let mut cmds: Vec<Command> = steps
                .iter()
                .map(|(f, notes, _)| Command::AddStep { target: StepTarget::Element { id: ids[f.as_str()].clone() }, index: None, builds: vec![], notes: notes.clone() })
                .collect();
            for (i, (_, _, voice)) in steps.iter().enumerate() {
                if let Some(data) = voice.as_deref().and_then(b64) {
                    cmds.push(Command::SetVoiceover { index: first_step + i, data_base64: data, path: None });
                }
            }
            if let Some(t) = music {
                if let Some(data) = b64(&t.asset) {
                    cmds.push(Command::SetSoundtrack { data_base64: data, path: None, looping: Some(t.looping), volume: Some(t.volume), name: Some(t.name.clone()) });
                }
            }
            cmds
        })
    }
}

/// Measure `e` in the theme font and place it over its source box `b`:
/// the same left, centre or right edge, at the top, middle or bottom of the
/// box as its anchor says, never wider than the source by more than a
/// little (it gets smaller instead).
fn fit_text(ed: &Editor, theme: &crate::Theme, e: &mut Element, b: SourceBox) {
    let ElementKind::Text(t) = &e.kind else { return };
    let align = t.align;
    // Text that sizes itself aligns its paragraphs within its widest line;
    // the source aligned them within its box.
    let mixed = !t.fixed_width && t.paras.iter().any(|p| p.align.is_some_and(|a| a != align));
    crate::editor::measure_text(&*ed.measure, theme, e);
    let src_w = (b.x1 - b.x0).max(1.0);
    let w = e.size.w * e.transform.scale;
    // The theme font may run wider than the source's: a little is fine,
    // more and the text is made smaller so lines stay where they were.
    if w > src_w * 1.04 && w > 0.0 {
        e.transform.scale *= src_w * 1.04 / w;
    }
    if mixed {
        // As wide as the box (or its widest line, a hair over the box), so
        // a centred line is centred on the box and a right-aligned one
        // ends at its right edge. Nothing rewraps: every line fits.
        e.size.w = (src_w / e.transform.scale).max(e.size.w * 1.001);
        if let ElementKind::Text(t) = &mut e.kind {
            t.fixed_width = true;
        }
        crate::editor::measure_text(&*ed.measure, theme, e);
    }
    if e.transform.rotation.abs() > 0.5 {
        // Turned text keeps its centre.
        return;
    }
    let w = e.size.w * e.transform.scale;
    e.transform.x = match align {
        Align::Left => b.x0 + w / 2.0,
        Align::Center => (b.x0 + b.x1) / 2.0,
        Align::Right => b.x1 - w / 2.0,
    };
    // Text sits a little lower in its line box than the source's ink box
    // starts; centre the line box on the source box instead of top-aligning.
    let h = e.size.h * e.transform.scale;
    e.transform.y = match b.anchor {
        BoxAnchor::Top => b.y0 + h / 2.0,
        BoxAnchor::Middle => (b.y0 + b.y1) / 2.0,
        BoxAnchor::Bottom => b.y1 - h / 2.0,
    };
}
