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
use crate::model::{Align, Asset, Element, ElementKind, FrameData, FrameStyle, Size, StepTarget};
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
}

/// Something that came in differently from the source (the summary sheet
/// after an import lists these).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportNote {
    /// Pages that could not be read were left out.
    PageSkipped,
}

impl ImportNote {
    pub const ALL: [ImportNote; 1] = [ImportNote::PageSkipped];

    /// English, for the API and the undo history of agents.
    pub fn english(self, n: usize) -> String {
        let (one, many) = match self {
            ImportNote::PageSkipped => ("unreadable page left out", "unreadable pages left out"),
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
    /// page's name; a page of several objects is framed on its own.
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
            let single = p.items.len() == 1;
            if !single {
                let fid = tmp("f");
                clip.elements.push(Element {
                    id: fid.clone(),
                    transform: Transform { x: ox + p.w / 2.0, y: oy + p.h / 2.0, scale: 1.0, rotation: 0.0 },
                    size: Size { w: p.w, h: p.h },
                    locked: false,
                    name: p.name.clone(),
                    placeholder: false,
                    group: None,
                    kind: ElementKind::Frame(FrameData { style: FrameStyle::Rectangle, color: None, slide: false }),
                });
                steps.push((fid, p.notes.clone()));
            }
            for it in &p.items {
                let mut e = it.element.clone();
                e.id = tmp(e.kind.id_prefix());
                e.group = None;
                if let (Some(b), ElementKind::Text(_)) = (it.fit, &e.kind) {
                    fit_text(self, &theme, &mut e, b);
                }
                e.transform.x += ox;
                e.transform.y += oy;
                if single {
                    e.name = p.name.clone();
                    steps.push((e.id.clone(), p.notes.clone()));
                }
                clip.elements.push(e);
            }
        }
        self.paste_clip_then(&clip, 0.0, 0.0, Some(label), origin, |ids| {
            steps
                .iter()
                .map(|(f, notes)| Command::AddStep { target: StepTarget::Element { id: ids[f.as_str()].clone() }, index: None, builds: vec![], notes: notes.clone() })
                .collect()
        })
    }
}

/// Measure `e` in the theme font and place it over its source box `b`:
/// the same top, the same left, centre or right edge, never wider than the
/// source by more than a little (it gets smaller instead).
fn fit_text(ed: &Editor, theme: &crate::Theme, e: &mut Element, b: SourceBox) {
    let ElementKind::Text(t) = &e.kind else { return };
    let align = t.align;
    crate::editor::measure_text(&*ed.measure, theme, e);
    let src_w = (b.x1 - b.x0).max(1.0);
    let w = e.size.w * e.transform.scale;
    // The theme font may run wider than the source's: a little is fine,
    // more and the text is made smaller so lines stay where they were.
    if w > src_w * 1.04 && w > 0.0 {
        e.transform.scale *= src_w * 1.04 / w;
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
    e.transform.y = (b.y0 + b.y1) / 2.0;
}
