//! Pages read from another program's file (File › Import…, the API's
//! `import`): the pages laid out to the right of what is already on the
//! canvas inside one frame, with a path step per page, as one undo step.
//!
//! The readers live in `zebra-import`; this is the part every format
//! shares, so a PDF and a PowerPoint file land on the canvas the same way.
use std::collections::BTreeMap;

use serde::Serialize;

use crate::clip::Clip;
use crate::command::Command;
use crate::editor::{Editor, Outcome};
use crate::error::{ZError, ZResult};
use crate::geom::Transform;
use crate::model::{Align, Asset, AssetBytes, Element, ElementKind, FrameData, FrameStyle, MediaKind, Size, Soundtrack, StepTarget};
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
    /// The slides (1-based, ascending) a note is about, for the notes that
    /// name them: slides left out ([`ImportNote::PageSkipped`] from a
    /// PowerPoint file), [`ImportNote::Simplified`].
    /// The note's count is then the number of slides.
    pub slides: BTreeMap<ImportNote, Vec<usize>>,
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
    /// SmartArt graphics were left out: neither a Zebra diagram, the
    /// shapes PowerPoint saved of them nor a list of their items could be
    /// made (their data is missing or unreadable).
    SmartArt,
    /// Tables were left out.
    Table,
    /// Charts of a type Zebra doesn't have (stock, surface, pie of pie,
    /// percent bar…) came in as the nearest type it has.
    ChartNearest,
    /// Tables whose cells had text of different sizes came in with one
    /// size (a Zebra table has one text size).
    TableText,
    /// Tables whose style couldn't be read came in with the look their
    /// cells set themselves, in a plain style.
    TableStyle,
    /// Equations with no picture of them in the file were left out.
    Equation,
    /// Videos and sounds that could not be taken out of the file, or that
    /// won't play on a Mac (WMV, AVI…), came in as their poster pictures.
    Media,
    /// Embedded objects (and charts and equations PowerPoint saves a
    /// picture of beside them) came in as that picture.
    AsPicture,
    /// Shapes Zebra has no match for (custom shapes, curved connectors…)
    /// were left out; their text came in.
    Shape,
    /// EMF and WMF pictures that could not be drawn were left out.
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
    /// Formatting past [`MAX_IMPORT_SPANS`] style changes in a box was
    /// dropped (the rest takes the box's own style).
    Simplified,
    /// Slide layouts, masters or themes that couldn't be read were left out
    /// (their slides came in without their design).
    Design,
    /// PDF pages that couldn't be drawn (too much memory or time, or
    /// PDFium couldn't open them) came in as grey placeholders.
    Placeholder,
}

/// Most formatted spans an imported text box keeps (the text renderer
/// lays out at most 60,000 style changes in a paragraph).
pub const MAX_IMPORT_SPANS: usize = 60_000;

impl ImportNote {
    pub const ALL: [ImportNote; 20] = [
        ImportNote::PageSkipped,
        ImportNote::HiddenSlide,
        ImportNote::Chart,
        ImportNote::SmartArt,
        ImportNote::Table,
        ImportNote::ChartNearest,
        ImportNote::TableText,
        ImportNote::TableStyle,
        ImportNote::Equation,
        ImportNote::Media,
        ImportNote::AsPicture,
        ImportNote::Shape,
        ImportNote::Metafile,
        ImportNote::Picture,
        ImportNote::Flip,
        ImportNote::Gradient,
        ImportNote::Other,
        ImportNote::Simplified,
        ImportNote::Design,
        ImportNote::Placeholder,
    ];

    /// English, for the API and the undo history of agents.
    pub fn english(self, n: usize) -> String {
        let (one, many) = match self {
            ImportNote::Placeholder => ("page that couldn't be drawn shown as a placeholder", "pages that couldn't be drawn shown as placeholders"),
            ImportNote::PageSkipped => ("unreadable page left out", "unreadable pages left out"),
            ImportNote::HiddenSlide => ("hidden slide left out", "hidden slides left out"),
            ImportNote::Chart => ("chart left out", "charts left out"),
            ImportNote::SmartArt => ("SmartArt graphic left out", "SmartArt graphics left out"),
            ImportNote::Table => ("table left out", "tables left out"),
            ImportNote::ChartNearest => ("chart shown as the nearest type Zebra has", "charts shown as the nearest type Zebra has"),
            ImportNote::TableText => ("table with text of several sizes given one size", "tables with text of several sizes given one size"),
            ImportNote::TableStyle => ("table whose style couldn't be read shown plainly", "tables whose style couldn't be read shown plainly"),
            ImportNote::Equation => ("equation left out", "equations left out"),
            ImportNote::Media => ("video or sound imported as its poster picture", "videos and sounds imported as their poster pictures"),
            ImportNote::AsPicture => ("embedded object imported as a picture", "embedded objects imported as pictures"),
            ImportNote::Shape => ("unsupported shape left out (its text kept)", "unsupported shapes left out (their text kept)"),
            ImportNote::Metafile => ("EMF or WMF picture left out", "EMF or WMF pictures left out"),
            ImportNote::Picture => ("picture in an unsupported format left out", "pictures in unsupported formats left out"),
            ImportNote::Flip => ("flipped object shown unflipped", "flipped objects shown unflipped"),
            ImportNote::Gradient => ("gradient made a solid colour", "gradients made solid colours"),
            ImportNote::Other => ("other object left out", "other objects left out"),
            ImportNote::Simplified => ("slide with formatting simplified", "slides with formatting simplified"),
            ImportNote::Design => ("slide design that couldn't be read", "slide designs that couldn't be read"),
        };
        format!("{n} {}", if n == 1 { one } else { many })
    }

    /// [`Self::english`], naming the slides when the note has them:
    /// "2 unreadable slides left out (2, 5–9)".
    pub fn english_with(self, n: usize, slides: &[usize]) -> String {
        if slides.is_empty() {
            return self.english(n);
        }
        let head = match self {
            ImportNote::PageSkipped => format!("{n} {}", if n == 1 { "unreadable slide left out" } else { "unreadable slides left out" }),
            _ => self.english(n),
        };
        let (groups, more) = slide_groups(slides);
        let more = if more > 0 { format!(" and {more} more") } else { String::new() };
        format!("{head} ({}{more})", groups.join(", "))
    }
}

/// How many groups of slide numbers a note lists before "and N more".
pub const SLIDE_GROUPS: usize = 8;

/// Slide numbers as a note lists them: runs of consecutive numbers as
/// ranges ("5–9"), at most [`SLIDE_GROUPS`] groups, and how many numbers
/// are left after them.
pub fn slide_groups(slides: &[usize]) -> (Vec<String>, usize) {
    let mut runs: Vec<(usize, usize)> = vec![];
    for &n in slides {
        match runs.last_mut() {
            Some(r) if n == r.1 + 1 => r.1 = n,
            Some(r) if n <= r.1 => {}
            _ => runs.push((n, n)),
        }
    }
    let shown: Vec<String> = runs.iter().take(SLIDE_GROUPS).map(|&(a, b)| if a == b { a.to_string() } else { format!("{a}\u{2013}{b}") }).collect();
    let more = runs.iter().skip(SLIDE_GROUPS).map(|&(a, b)| b - a + 1).sum();
    (shown, more)
}

impl ImportedDeck {
    /// Note `n` about slide `slide` (1-based), once per slide.
    pub fn note_slide(&mut self, n: ImportNote, slide: usize) {
        let list = self.slides.entry(n).or_default();
        if let Err(at) = list.binary_search(&slide) {
            list.insert(at, slide);
        }
        self.notes.insert(n, list.len());
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
    let mut a = asset_of(AssetBytes::from(Vec::new()), &bytes)?;
    a.bytes = crate::media::spill(bytes, &key, &a.ext);
    Ok((key, a))
}

/// [`asset_from`] for bytes already in place (streamed to a spill file
/// with [`crate::media::spill_reader`]), with their key.
pub fn asset_from_bytes(bytes: AssetBytes) -> ZResult<Asset> {
    let probe = bytes.clone();
    asset_of(bytes, &probe)
}

/// What `data` is, as an asset holding `bytes` (the same bytes, or an
/// empty stand-in to be filled).
fn asset_of(bytes: AssetBytes, data: &[u8]) -> ZResult<Asset> {
    let (ext, w, h) = if is_pdf(data) {
        ("pdf".to_string(), 0, 0)
    } else if data.starts_with(b"<svg") {
        let (w, h) = crate::symbols::svg_pixels(data);
        ("svg".to_string(), w, h)
    } else {
        crate::editor::image_info(data)?
    };
    Ok(Asset { ext, bytes, width: w, height: h })
}

/// A video or sound asset (MP4, MOV, M4V; MP3, M4A, AAC, WAV) and its
/// content key, for [`ImportedDeck::assets`], with what its header says
/// (kind, size, length). Other formats are refused: they won't play.
pub fn av_asset_from(bytes: Vec<u8>) -> ZResult<(String, Asset, crate::media::MediaInfo)> {
    let key = crate::editor::content_key(&bytes);
    let (mut a, info) = av_asset_from_bytes(AssetBytes::from(Vec::new()), &bytes)?;
    a.bytes = crate::media::spill(bytes, &key, &info.ext);
    Ok((key, a, info))
}

/// [`av_asset_from`] for bytes already in place (`data` is what they hold).
pub fn av_asset_from_bytes(bytes: AssetBytes, data: &[u8]) -> ZResult<(Asset, crate::media::MediaInfo)> {
    let info = crate::media::info(data)?;
    if !matches!(info.kind, MediaKind::Video | MediaKind::Audio) {
        return Err(crate::error::ZError::invalid("not a video or a sound"));
    }
    Ok((Asset { ext: info.ext.clone(), bytes, width: info.width, height: info.height }, info))
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
    /// on its own. Objects grouped on a page stay grouped, locked ones
    /// locked. A page's sound becomes its step's voice-over; the deck's
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
                    // The page's own size: what fills the whole slide (its
                    // background, a full-bleed picture) is exactly as big,
                    // and is held by the frame and moves with it
                    // ([`crate::model::frame_holds`]).
                    size: Size { w: p.w, h: p.h },
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
        // Sounds go in through the commands that set them, by key: they are
        // among the assets the paste adds (they used to go as base64, the
        // whole sound encoded and decoded again in memory).
        let has = |key: &str| deck.assets.contains_key(key).then(|| key.to_string());
        // A paste adds unlocked copies: lock what the page has locked.
        let locked: Vec<String> = clip.elements.iter().filter(|e| e.locked).map(|e| e.id.clone()).collect();
        let first_step = self.doc.path.len();
        let music = deck.soundtrack.as_ref().filter(|_| self.doc.soundtrack.is_none());
        self.paste_clip_then(&clip, 0.0, 0.0, Some(label), origin, |ids| {
            let mut cmds: Vec<Command> = steps
                .iter()
                .map(|(f, notes, _)| Command::AddStep { target: StepTarget::Element { id: ids[f.as_str()].clone() }, index: None, builds: vec![], notes: notes.clone() })
                .collect();
            for (i, (_, _, voice)) in steps.iter().enumerate() {
                if let Some(asset) = voice.as_deref().and_then(has) {
                    cmds.push(Command::SetVoiceover { index: first_step + i, data_base64: String::new(), path: None, asset: Some(asset) });
                }
            }
            if let Some(t) = music {
                if let Some(asset) = has(&t.asset) {
                    cmds.push(Command::SetSoundtrack { data_base64: String::new(), path: None, looping: Some(t.looping), volume: Some(t.volume), name: Some(t.name.clone()), asset: Some(asset) });
                }
            }
            for id in &locked {
                cmds.push(Command::Update { id: ids[id.as_str()].clone(), patch: crate::command::ElementPatch { locked: Some(true), ..Default::default() } });
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
