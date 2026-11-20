//! PowerPoint (.pptx) slides as Zebra's own objects, through BetterOffice's
//! PresentationML parser.
//!
//! Each slide becomes a frame (marked as a slide, so it shows deck-style
//! when presenting) holding:
//! - its background, as a locked rectangle (or picture) at the back;
//! - the master's and layout's own objects, then the slide's;
//! - text boxes and placeholders as Zebra text: the source's paragraphs
//!   and line breaks, bullets and numbering by level, alignment, and runs'
//!   bold, italic, underline, strike-through, colour, links and sizes
//!   (strike-through read from the part's XML, which BetterOffice's model
//!   leaves it out of). Fonts are never
//!   copied: every box takes a style of the document's theme by its size
//!   (sizes are ranked deck-wide into title, subtitle and body, as the PDF
//!   prototype did) and keeps its size on the slide through its scale;
//!   sizes within a box become relative spans;
//! - pictures (PNG, JPEG, GIF, SVG; EMF and WMF drawn as SVG through
//!   BetterOffice's metafile player) and the basic shapes Zebra has, with
//!   their fill, line and text; groups stay groups;
//! - videos as Zebra videos (their pictures as posters); sounds as the
//!   slide's voice-over, or the document's music when they play across
//!   slides;
//! - SmartArt as the Zebra diagram for its layout, holding its items'
//!   words; a layout Zebra has none for as the shapes PowerPoint saved of
//!   it (grouped); one saved without them as a bulleted list of its items.
//!
//! What has no Zebra match comes in as the picture the file keeps of it,
//! when it keeps one, or is left out; either way it is counted in the
//! deck's notes ([`ImportNote`]).
use std::collections::{BTreeMap, BTreeSet, HashMap};

use zebra_core::diagrams::Item;
use zebra_core::templates::Piece;
use ooxml_drawingml::{resolve_color_value_to_hex_with_theme, style_fill, style_outline, ColorValue, ShapeFill, ShapeOutline, Theme as DmlTheme, ThemeFormatScheme};
use pptx_parse::{
    Bullet, GraphicFrameData, ParagraphProperties, Picture, PictureCrop, Placeholder, PptxPackage, Relationship, RunProperties, Shape, ShapeNode, ShapeTransform, Slide,
    SlideLayout, SlideMaster, SourceShape, TargetMode, TextAutofit, TextBody, TextCaps,
};
use crate::pptx_media::MediaSource;
use crate::pptx_package::{Package, Read};
use zebra_core::import::{asset_from, BoxAnchor, ImportNote, ImportedDeck, ImportedItem, ImportedPage, SourceBox};
use zebra_core::{
    Align, AssetBytes, Crop, Dash, Element, ElementKind, ImageData, ImageMask, InkData, ListKind, MediaKind, ParaFormat, ShapeData, ShapeKind, Size, Soundtrack, SpanFormat, TextData, TextSpan, TextStyle, Transform, VideoData,
};

use crate::pptx_xml::{part_info, smart_art, MediaRef, PartInfo, ShapeInfo, SmartArt, StandIn, StyleKind, TextStrike};
use crate::{Control, ImportError, Options, Reader};

/// Long side of a slide on the canvas, world units (the same as a PDF's
/// largest page, [`crate::pdf::PAGE_UNITS`]).
pub const SLIDE_UNITS: f64 = crate::pdf::PAGE_UNITS;
const EMU_PER_PT: f64 = 12_700.0;
/// PowerPoint's default slide, 13.333 x 7.5 inches.
const DEFAULT_SLIDE: (i64, i64) = (12_192_000, 6_858_000);
/// SmartArt drawings read inside one another at most (PowerPoint never
/// nests them; a drawing that names itself is cut off sooner).
const MAX_SMARTART_DEPTH: usize = 4;

/// Is this a PowerPoint file (a zip holding `ppt/presentation.xml`)?
/// Reads the zip's directory, not the whole file; a zip too damaged to
/// read counts when its first or last 64 KB name the part (it is then
/// reported damaged, not "not supported").
pub fn is_pptx(bytes: &[u8]) -> bool {
    const NAME: &[u8] = b"ppt/presentation.xml";
    if !bytes.starts_with(b"PK\x03\x04") {
        return false;
    }
    if let Ok(z) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) {
        return z.index_for_name("ppt/presentation.xml").is_some();
    }
    let edge = 64 * 1024;
    let has = |b: &[u8]| b.windows(NAME.len()).any(|w| w == NAME);
    has(&bytes[..bytes.len().min(edge)]) || has(&bytes[bytes.len().saturating_sub(edge)..])
}

/// What an OLE compound file is, when it is one Zebra knows: an old
/// PowerPoint file, or a .pptx locked with a password.
pub fn compound_file_error(bytes: &[u8]) -> Option<ImportError> {
    const OLE: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    if !bytes.starts_with(OLE) {
        return None;
    }
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
    let has = |s: &str| {
        let n = utf16(s);
        bytes.windows(n.len()).any(|w| w == n.as_slice())
    };
    if has("EncryptedPackage") {
        Some(ImportError::Encrypted)
    } else if has("PowerPoint Document") {
        Some(ImportError::OldPowerPoint)
    } else {
        None
    }
}

pub struct PptxReader {
    /// The file, read a batch of slides at a time; its media read from
    /// the file as it is needed.
    package: Package,
    title: String,
}

impl PptxReader {
    /// `bytes`: the file, mapped (or in memory). Only its directory and
    /// slide list are read here; slides are read by [`Reader::read`].
    pub fn new(bytes: AssetBytes, title: String) -> Result<PptxReader, ImportError> {
        let package = Package::open(MediaSource::open(bytes)?)?;
        Ok(PptxReader { package, title })
    }
}

impl Reader for PptxReader {
    fn kind(&self) -> &'static str {
        "PowerPoint"
    }
    fn page_count(&self) -> usize {
        self.package.slide_count()
    }
    fn read(&self, opts: &Options, ctl: &Control) -> Result<ImportedDeck, ImportError> {
        let mut deck = ImportedDeck { title: self.title.clone(), ..Default::default() };
        let mut pending = vec![];
        let mut media = BTreeMap::new();
        let mut counted = BTreeSet::new();
        // The slide size, from the first batch (it is the presentation's,
        // the same in every batch).
        let mut k_size: Option<(f64, (i64, i64))> = None;
        let mut cancelled = false;
        let mut skipped = 0usize;
        self.package.read(crate::page_range(opts, self.package.slide_count()), |batch| {
            if ctl.cancelled() {
                cancelled = true;
                return false;
            }
            let (pkg, slides) = match batch {
                Read::Skipped(i) => {
                    deck.note_slide(ImportNote::PageSkipped, i + 1);
                    skipped += 1;
                    ctl.page_done();
                    return true;
                }
                Read::Parsed(pkg, slides) => (pkg, slides),
            };
            let pkg = &*pkg;
            let (k, (sw, sh)) = *k_size.get_or_insert_with(|| {
                let (sw, sh) = match (pkg.presentation.width_emu, pkg.presentation.height_emu) {
                    (w, h) if w > 0 && h > 0 => (w, h),
                    _ => DEFAULT_SLIDE,
                };
                (SLIDE_UNITS / sw.max(sh) as f64, (sw, sh))
            });
            let index = Index::new(pkg);
            let mut infos = BTreeMap::new();
            for (slide, &i) in pkg.slides.iter().zip(&slides) {
                if ctl.cancelled() {
                    cancelled = true;
                    return false;
                }
                if slide.hidden == Some(true) {
                    *deck.notes.entry(ImportNote::HiddenSlide).or_default() += 1;
                    ctl.page_done();
                    continue;
                }
                let (layout, master) = slide_parts(pkg, slide);
                let parts = [Some(pkg.presentation.part_path.as_str()), Some(slide.part_path.as_str()), layout.map(|l| l.part_path.as_str()), master.map(|m| m.part_path.as_str())];
                for part in parts.into_iter().flatten() {
                    if !infos.contains_key(part) {
                        infos.insert(part.to_string(), part_info(pkg, part));
                    }
                }
                let mut r = SlideReader::new(pkg, self.package.media(), &index, slide, k, (sw, sh), &mut deck, &mut media, &mut counted, &infos, &opts.theme);
                r.has_soundtrack = opts.has_soundtrack;
                r.read();
                let SlideReader { items, pending: texts, voiceover, .. } = r;
                let page = deck.pages.len();
                pending.extend(texts.into_iter().map(|t| (page, t)));
                let name = slide_title(slide).unwrap_or_else(|| (opts.slide_name)(i + 1));
                deck.pages.push(ImportedPage { w: sw as f64 * k, h: sh as f64 * k, name, items, notes: slide.notes.trim().to_string(), slide: true, voiceover });
                ctl.page_done();
                // The slide's own part info isn't needed again.
                infos.remove(&slide.part_path);
            }
            true
        });
        if cancelled || ctl.cancelled() {
            return Err(ImportError::Cancelled);
        }
        let designs = self.package.broken_designs();
        if designs > 0 {
            deck.notes.insert(ImportNote::Design, designs);
        }
        if deck.pages.is_empty() {
            // Nothing could be read, or everything chosen is hidden.
            return Err(if skipped > 0 { ImportError::Damaged("PowerPoint") } else { ImportError::AllHidden });
        }
        rank_text(&mut deck, &pending, opts, k_size.map_or(1.0, |k| k.0));
        Ok(deck)
    }
}

/// A batch's relationships and media parts, by id and path (looked up for
/// every run's link and every picture).
struct Index<'p> {
    rels: HashMap<&'p str, HashMap<&'p str, &'p Relationship>>,
    media: HashMap<&'p str, &'p pptx_parse::MediaPart>,
}

impl<'p> Index<'p> {
    fn new(pkg: &'p PptxPackage) -> Index<'p> {
        let rels = pkg.relationships.iter().map(|(part, list)| (part.as_str(), list.iter().rev().map(|r| (r.id.as_str(), r)).collect())).collect();
        let media = pkg.media.iter().rev().map(|m| (m.part_path.as_str(), m)).collect();
        Index { rels, media }
    }
    /// Relationship `id` of part `part` (the first with that id).
    fn rel(&self, part: &str, id: &str) -> Option<&'p Relationship> {
        self.rels.get(part)?.get(id).copied()
    }
}

/// A slide's layout and master.
fn slide_parts<'p>(pkg: &'p PptxPackage, slide: &Slide) -> (Option<&'p SlideLayout>, Option<&'p SlideMaster>) {
    let layout = slide.layout_part_path.as_deref().and_then(|p| pkg.layouts.iter().find(|l| l.part_path == p)).or_else(|| pkg.layouts.first());
    let master = pptx_parse::master_for_layout(pkg, layout).or_else(|| pkg.masters.first());
    (layout, master)
}

/// A text box read from a slide, waiting for the deck-wide size ranking.
struct PendingText {
    item: usize,
    /// The size most of its characters have, points.
    base_pt: f64,
    chars: usize,
    /// Wrap width, world units (0 when the text doesn't wrap).
    wrap: f64,
}

/// The slide's title placeholder's text, on one line.
fn slide_title(slide: &Slide) -> Option<String> {
    fn find(nodes: &[ShapeNode]) -> Option<&TextBody> {
        nodes.iter().find_map(|n| match n {
            ShapeNode::Shape(s) if s.base.placeholder.as_ref().is_some_and(|p| matches!(p.placeholder_type.as_deref(), Some("title" | "ctrTitle"))) => s.text.as_ref(),
            ShapeNode::Group(g) => find(&g.children),
            _ => None,
        })
    }
    let body = find(&slide.shapes)?;
    let text = body.paragraphs.iter().map(|p| p.runs.iter().map(|r| if r.line_break { " " } else { r.text.as_str() }).collect::<String>()).collect::<Vec<_>>().join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

/// Rank the deck's text sizes into the theme's styles: the commonest size
/// (by characters) is body, sizes at least 1.6 times it are titles, sizes
/// in between subtitles. Each box keeps its size through its scale.
fn rank_text(deck: &mut ImportedDeck, pending: &[(usize, PendingText)], opts: &Options, k: f64) {
    let mut hist: BTreeMap<i64, usize> = BTreeMap::new();
    for (_, t) in pending {
        *hist.entry((t.base_pt * 2.0).round() as i64).or_default() += t.chars;
    }
    let body = hist.iter().max_by_key(|(s, n)| (**n, -**s)).map(|(s, _)| *s as f64 / 2.0).unwrap_or(18.0).max(1.0);
    for (page, t) in pending {
        let Some(item) = deck.pages.get_mut(*page).and_then(|p| p.items.get_mut(t.item)) else { continue };
        let e = &mut item.element;
        let ElementKind::Text(data) = &mut e.kind else { continue };
        let style = if t.base_pt >= 1.6 * body {
            TextStyle::Title
        } else if t.base_pt >= 1.15 * body {
            TextStyle::Subtitle
        } else {
            TextStyle::Body
        };
        data.style = style;
        let theme_size = opts.theme.text_style(style).size.max(1.0);
        e.transform.scale = (t.base_pt * EMU_PER_PT * k / theme_size).max(1e-6);
        e.size.w = if data.fixed_width { t.wrap / e.transform.scale } else { 0.0 };
    }
}

/// Where a shape is on the slide: centre, size and turn in EMU and
/// degrees, its groups' moves, scales, flips and turns applied.
#[derive(Debug, Clone, Copy)]
struct Placed {
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
    rot: f64,
    flip_h: bool,
    flip_v: bool,
}

/// Place `t` through its groups `parents` (outermost first).
fn place(t: &ShapeTransform, parents: &[&ShapeTransform]) -> Placed {
    let mut p = Placed {
        cx: t.x as f64 + t.width as f64 / 2.0,
        cy: t.y as f64 + t.height as f64 / 2.0,
        w: t.width as f64,
        h: t.height as f64,
        rot: finite(t.rotation_deg),
        flip_h: t.flip_h,
        flip_v: t.flip_v,
    };
    for g in parents.iter().rev() {
        let (gx, gy, gw, gh) = (g.x as f64, g.y as f64, g.width as f64, g.height as f64);
        let chx = g.child_x.unwrap_or(g.x) as f64;
        let chy = g.child_y.unwrap_or(g.y) as f64;
        let chw = g.child_width.unwrap_or(g.width) as f64;
        let chh = g.child_height.unwrap_or(g.height) as f64;
        let sx = if chw > 0.0 { gw / chw } else { 1.0 };
        let sy = if chh > 0.0 { gh / chh } else { 1.0 };
        let mut cx = gx + (p.cx - chx) * sx;
        let mut cy = gy + (p.cy - chy) * sy;
        p.w *= sx;
        p.h *= sy;
        let (gcx, gcy) = (gx + gw / 2.0, gy + gh / 2.0);
        if g.flip_h {
            cx = 2.0 * gcx - cx;
            p.flip_h = !p.flip_h;
            p.rot = -p.rot;
        }
        if g.flip_v {
            cy = 2.0 * gcy - cy;
            p.flip_v = !p.flip_v;
            p.rot = -p.rot;
        }
        let a = finite(g.rotation_deg).to_radians();
        let (dx, dy) = (cx - gcx, cy - gcy);
        p.cx = gcx + dx * a.cos() - dy * a.sin();
        p.cy = gcy + dx * a.sin() + dy * a.cos();
        p.rot += finite(g.rotation_deg);
    }
    p.rot = p.rot.rem_euclid(360.0);
    p
}

fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// Whose objects are being read: the slide's own, or the master's and
/// layout's decorations behind them (their placeholders are not drawn).
#[derive(Clone, Copy, PartialEq)]
enum Role {
    Slide,
    Inherited,
}

/// The text bodies and styles a shape's text inherits from.
struct Cascade<'a> {
    primary: Option<&'a TextBody>,
    layout: Option<&'a TextBody>,
    master: Option<&'a TextBody>,
    placeholder: Option<&'a Placeholder>,
    style_color: Option<&'a ColorValue>,
    /// The strike-through `primary`, `layout` and `master` say (which
    /// the parser's model leaves out).
    strike: [Option<&'a TextStrike>; 3],
}

struct SlideReader<'a> {
    pkg: &'a PptxPackage,
    slide: &'a Slide,
    layout: Option<&'a SlideLayout>,
    master: Option<&'a SlideMaster>,
    theme: DmlTheme,
    scheme: ThemeFormatScheme,
    k: f64,
    size: (i64, i64),
    deck: &'a mut ImportedDeck,
    /// The file's media.
    src: &'a MediaSource,
    /// The batch's relationships and media by id and path.
    index: &'a Index<'a>,
    /// SmartArt drawings being read, innermost last (a drawing that names
    /// itself, or one inside it, is not read again).
    drawings: Vec<String>,
    /// Asset key of each media part read so far (None: it can't come in).
    media: &'a mut BTreeMap<String, Option<String>>,
    items: Vec<ImportedItem>,
    pending: Vec<PendingText>,
    groups: usize,
    /// The part whose objects are being read (for its links).
    part: &'a str,
    /// Masters, layouts and inherited backgrounds whose notes are counted
    /// already: what every slide shows of them is noted once per import.
    counted: &'a mut BTreeSet<String>,
    /// Reading something whose notes are counted already.
    quiet: bool,
    /// What the parser's model leaves out of each part, by part path.
    infos: &'a BTreeMap<String, PartInfo>,
    /// The document has background music already.
    has_soundtrack: bool,
    /// The slide's sound, for its step's voice-over.
    voiceover: Option<String>,
    /// The document's theme (diagrams are built in its colours).
    zebra: &'a zebra_core::Theme,
}

impl<'a> SlideReader<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        pkg: &'a PptxPackage,
        src: &'a MediaSource,
        index: &'a Index<'a>,
        slide: &'a Slide,
        k: f64,
        size: (i64, i64),
        deck: &'a mut ImportedDeck,
        media: &'a mut BTreeMap<String, Option<String>>,
        counted: &'a mut BTreeSet<String>,
        infos: &'a BTreeMap<String, PartInfo>,
        zebra: &'a zebra_core::Theme,
    ) -> Self {
        let (layout, master) = slide_parts(pkg, slide);
        let theme = pptx_parse::slide_theme(pkg, Some(&slide.part_path), layout.map(|l| l.part_path.as_str()));
        let scheme = master
            .and_then(|m| m.theme_part_path.as_deref())
            .and_then(|p| pkg.themes.iter().find(|t| t.part_path == p))
            .or_else(|| pkg.themes.first())
            .map(|t| t.format_scheme.clone())
            .unwrap_or_default();
        SlideReader { pkg, src, index, drawings: vec![], slide, layout, master, theme, scheme, k, size, deck, media, items: vec![], pending: vec![], groups: 0, part: &slide.part_path, counted, quiet: false, infos, has_soundtrack: false, voiceover: None, zebra }
    }

    fn note(&mut self, n: ImportNote) {
        if !self.quiet {
            *self.deck.notes.entry(n).or_default() += 1;
        }
    }

    /// Count notes from here on only if `key` (an inherited part) hasn't
    /// had its notes counted yet.
    fn count_once(&mut self, key: String) {
        self.quiet = !self.counted.insert(key);
    }

    fn read(&mut self) {
        self.background();
        self.quiet = false;
        let show = self.slide.show_master_shapes;
        if let (Some(m), true) = (self.master, show && self.layout.is_none_or(|l| l.show_master_shapes)) {
            self.part = &m.part_path;
            self.count_once(m.part_path.clone());
            self.walk(&m.shapes, None, &[], None, Role::Inherited);
        }
        if let (Some(l), true) = (self.layout, show) {
            self.part = &l.part_path;
            self.count_once(l.part_path.clone());
            self.walk(&l.shapes, None, &[], None, Role::Inherited);
        }
        let (pkg, slide) = (self.pkg, self.slide);
        self.part = &slide.part_path;
        self.quiet = false;
        let source = pkg.slide_source(&slide.part_path);
        self.walk(&slide.shapes, source.map(|s| s.shapes.as_slice()), &[], None, Role::Slide);
        if let Some(src) = source {
            for o in &src.omitted {
                if !o.hidden {
                    self.note(ImportNote::Other);
                }
            }
            count_equations(&src.shapes, &mut |n| self.note(n));
        }
    }

    /// The slide's background (its own, else its layout's, else its
    /// master's) as a rectangle or picture behind everything.
    fn background(&mut self) {
        let pkg_bg = |fill: &Option<ShapeFill>, pic: &Option<Box<pptx_parse::PictureFill>>, rf: &Option<ooxml_drawingml::StyleReference>| {
            (fill.is_some() || pic.is_some() || rf.is_some()).then(|| (fill.clone(), pic.clone(), rf.clone()))
        };
        // Whose background it is: an inherited one is noted once.
        let found = pkg_bg(&self.slide.background, &self.slide.background_picture, &self.slide.background_reference)
            .map(|b| (b, None))
            .or_else(|| self.layout.and_then(|l| Some((pkg_bg(&l.background, &l.background_picture, &l.background_reference)?, Some(&l.part_path)))))
            .or_else(|| self.master.and_then(|m| Some((pkg_bg(&m.background, &m.background_picture, &m.background_reference)?, Some(&m.part_path)))));
        let (found, owner) = match found {
            Some((b, owner)) => (Some(b), owner),
            None => (None, None),
        };
        if let Some(p) = owner {
            self.count_once(format!("{p}#background"));
        }
        let (w, h) = (self.size.0 as f64, self.size.1 as f64);
        let all = Placed { cx: w / 2.0, cy: h / 2.0, w, h, rot: 0.0, flip_h: false, flip_v: false };
        let (fill, pic, rf) = found.unwrap_or_default();
        let pictured = pic.as_ref().and_then(|p| p.media_part_path.clone()).is_some_and(|path| self.picture(&path, &PictureCrop::default(), "rect", all, String::new(), None));
        if !pictured {
            let fill = fill.or_else(|| rf.map(|r| style_fill(&self.scheme, &r, &self.theme)));
            let color = fill.as_ref().and_then(|f| self.fill_color(f)).or_else(|| self.hex(&scheme_color("background1")));
            let Some(c) = color else { return };
            self.push_shape(ShapeKind::Rectangle, all, Some(c), None, 0.0, 0.0, Dash::Solid, None);
        }
        // Locked, as a slide's background is in Keynote: clicking the
        // slide doesn't pick it up.
        if let Some(it) = self.items.last_mut() {
            it.element.locked = true;
        }
    }

    fn walk(&mut self, nodes: &'a [ShapeNode], source: Option<&'a [SourceShape]>, parents: &[&ShapeTransform], group: Option<&str>, role: Role) {
        for (i, node) in nodes.iter().enumerate() {
            let src = source.and_then(|s| s.get(i));
            match node {
                ShapeNode::Group(g) => {
                    if g.base.hidden {
                        continue;
                    }
                    let gid = match group {
                        Some(g) => g.to_string(),
                        None => {
                            self.groups += 1;
                            format!("g{}", self.groups)
                        }
                    };
                    let mut ps = parents.to_vec();
                    ps.push(&g.base.transform);
                    self.walk(&g.children, src.map(|s| s.children.as_slice()), &ps, Some(&gid), role);
                }
                ShapeNode::Shape(s) => {
                    if s.base.hidden || (role == Role::Inherited && s.base.placeholder.is_some()) {
                        continue;
                    }
                    self.shape(node, s, parents, group);
                }
                ShapeNode::Picture(p) => {
                    if p.base.hidden || (role == Role::Inherited && p.base.placeholder.is_some()) {
                        continue;
                    }
                    // A video or sound: a Zebra video, the slide's voice-over
                    // or the document's music; else (not in the file, or a
                    // format a Mac won't play) its poster picture, noted.
                    let media = self.shape_info(self.part, p.base.id).and_then(|i| i.media.as_ref());
                    if let Some(m) = media {
                        if self.media_node(p, m, parents, group) {
                            continue;
                        }
                    }
                    if media.is_some() || src.is_some_and(|s| s.media.is_some()) {
                        self.note(ImportNote::Media);
                    }
                    // A chart (or other object) the file keeps a picture of
                    // beside it comes in as that picture: say so.
                    let stand_in = self.shape_info(self.part, p.base.id).and_then(|i| i.stands_in);
                    if self.picture_node(p, &p.base.transform, parents, group) && stand_in.is_some() {
                        self.note(ImportNote::AsPicture);
                    }
                }
                ShapeNode::GraphicFrame(f) => {
                    if f.base.hidden || (role == Role::Inherited && f.base.placeholder.is_some()) {
                        continue;
                    }
                    match &f.data {
                        GraphicFrameData::Table(_) => self.note(ImportNote::Table),
                        GraphicFrameData::Chart { .. } => self.note(ImportNote::Chart),
                        GraphicFrameData::Diagram { relationship_ids, drawing_part_path } => {
                            let at = place(&f.base.transform, parents);
                            if !self.smart_art(at, &f.base.transform, relationship_ids, drawing_part_path.as_deref(), parents, group) {
                                self.note(ImportNote::SmartArt);
                            }
                        }
                        GraphicFrameData::Unknown { picture: Some(p), .. } => {
                            if self.picture_node(p, &f.base.transform, parents, group) {
                                self.note(ImportNote::AsPicture);
                            }
                        }
                        GraphicFrameData::Unknown { picture: None, .. } => self.note(ImportNote::Other),
                    }
                }
            }
        }
    }

    /// A SmartArt graphic over `at` (`t` its frame's own place): the Zebra
    /// diagram for its layout, holding its items' words; else the shapes
    /// PowerPoint saved of it, grouped; else its items as a bulleted list.
    /// False when none of these can be made.
    fn smart_art(&mut self, at: Placed, t: &ShapeTransform, rel_ids: &[String], drawing: Option<&str>, parents: &[&ShapeTransform], group: Option<&str>) -> bool {
        let data = self.smart_art_data(rel_ids);
        if let Some(d) = &data {
            let key = d.layout.as_deref().and_then(|l| diagram_for(l, &d.items));
            if let Some(pieces) = key.and_then(|k| zebra_core::diagrams::with_items(k, &d.items)) {
                let gid = self.group_for(group);
                self.pieces(&pieces, at, &gid);
                return true;
            }
        }
        // The drawing PowerPoint saved (not one the parser laid out itself
        // when there was none): its shapes sit in the frame's box.
        let pkg = self.pkg;
        let saved = drawing.and_then(|p| pkg.diagram_drawings.iter().find(|d| d.part_path == p)).filter(|d| d.synthesis_extent.is_none());
        // A drawing that holds a SmartArt naming the drawing itself (or one
        // around it) would be read forever: it is read once, and SmartArt
        // is never nested deeper than PowerPoint makes it.
        let saved = saved.filter(|d| self.drawings.len() < MAX_SMARTART_DEPTH && !self.drawings.contains(&d.part_path));
        if let Some(dd) = saved {
            let before = self.items.len();
            let gid = self.group_for(group);
            let frame = ShapeTransform { child_x: Some(0), child_y: Some(0), child_width: Some(t.width), child_height: Some(t.height), ..t.clone() };
            let mut ps = parents.to_vec();
            ps.push(&frame);
            let part = self.part;
            self.part = &dd.part_path;
            self.drawings.push(dd.part_path.clone());
            self.walk(&dd.shapes, None, &ps, Some(&gid), Role::Slide);
            self.drawings.pop();
            self.part = part;
            if self.items.len() > before {
                return true;
            }
        }
        data.is_some_and(|d| self.bullet_list(&d.items, at, group))
    }

    /// The SmartArt data model (and layout) the slide's relationships
    /// `rel_ids` name.
    fn smart_art_data(&self, rel_ids: &[String]) -> Option<SmartArt> {
        let rels = self.pkg.relationships.get(self.part)?;
        let target = |kind: &str| rels.iter().find(|r| rel_ids.contains(&r.id) && r.has_type(kind)).and_then(|r| r.resolved_target.as_deref());
        let data = self.pkg.part_bytes(target("/diagramData")?)?;
        let layout = target("/diagramLayout").and_then(|p| self.pkg.part_bytes(p));
        smart_art(data, layout)
    }

    /// A diagram's pieces (unit coordinates) fitted into `at`, centred,
    /// as one group.
    fn pieces(&mut self, pieces: &[Piece], at: Placed, gid: &str) {
        let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
        let mut grow = |x: f64, y: f64, hw: f64, hh: f64| {
            lo = (lo.0.min(x - hw), lo.1.min(y - hh));
            hi = (hi.0.max(x + hw), hi.1.max(y + hh));
        };
        for p in pieces {
            match p {
                Piece::Shape { x, y, w, h, rot, .. } | Piece::Frame { x, y, w, h, rot, .. } => {
                    let (s, c) = rot.to_radians().sin_cos();
                    grow(*x, *y, (w * c.abs() + h * s.abs()) / 2.0, (w * s.abs() + h * c.abs()) / 2.0);
                }
                Piece::Text { x, y, w, h, .. } => grow(*x, *y, w / 2.0, h / 2.0),
                Piece::Ink { points, .. } => points.iter().for_each(|q| grow(q[0], q[1], 0.0, 0.0)),
            }
        }
        if !(hi.0 > lo.0 && hi.1 > lo.1) {
            return;
        }
        let k = self.k;
        // World units per diagram unit, and where the diagram's (0, 0) goes.
        let u = (at.w / (hi.0 - lo.0)).min(at.h / (hi.1 - lo.1)) * k;
        let (ox, oy) = (at.cx * k - (lo.0 + hi.0) / 2.0 * u, at.cy * k - (lo.1 + hi.1) / 2.0 * u);
        let element = |transform: Transform, size: Size, kind: ElementKind| Element { id: String::new(), transform, size, locked: false, name: String::new(), placeholder: false, group: Some(gid.to_string()), kind };
        for p in pieces {
            let (element, fit) = match p {
                Piece::Frame { .. } => continue,
                Piece::Shape { shape, x, y, w, h, rot, fill, stroke, sw, dash } => {
                    let data = ShapeData { shape: *shape, fill: fill.clone(), stroke: stroke.clone(), stroke_width: (u * sw.unwrap_or(0.004)).max(1.0), dash: *dash, ..Default::default() };
                    (element(Transform { x: ox + x * u, y: oy + y * u, scale: 1.0, rotation: *rot }, Size { w: w * u, h: h * u }, ElementKind::Shape(data)), None)
                }
                Piece::Text { style, text, x, y, w, h, bullets, align, on, .. } => {
                    // Each line as tall as the piece allows; the import
                    // makes it smaller if the theme font runs wider.
                    let lines = text.lines().count().max(1) as f64;
                    let size = self.zebra.text_style(*style).size.max(1.0);
                    let scale = (h * u / lines / (size * 1.25)).max(1e-6);
                    let color = on.as_ref().map(|f| self.zebra.text_on(Some(f)).unwrap_or_else(|| "theme:body".into()));
                    let data = TextData { text: text.clone(), style: *style, bullets: *bullets, align: *align, color, fixed_width: false, ..Default::default() };
                    let (cx, cy, hw, hh) = (ox + x * u, oy + y * u, w * u / 2.0, h * u / 2.0);
                    let fit = SourceBox { x0: cx - hw, y0: cy - hh, x1: cx + hw, y1: cy + hh, anchor: BoxAnchor::Middle };
                    (element(Transform { x: cx, y: cy, scale, rotation: 0.0 }, Size { w: 0.0, h: 0.0 }, ElementKind::Text(data)), Some(fit))
                }
                Piece::Ink { points, width, color } => {
                    let pts: Vec<(f64, f64)> = points.iter().map(|q| (q[0] * u, q[1] * u)).collect();
                    let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |a, q| (a.0.min(q.0), a.1.max(q.0)));
                    let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |a, q| (a.0.min(q.1), a.1.max(q.1)));
                    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                    let data = InkData { points: pts.iter().map(|q| zebra_core::Vec2::new(q.0 - cx, q.1 - cy)).collect(), color: color.clone(), width: width * u, highlighter: false };
                    (element(Transform { x: ox + cx, y: oy + cy, scale: 1.0, rotation: 0.0 }, Size { w: x1 - x0, h: y1 - y0 }, ElementKind::Ink(data)), None)
                }
            };
            self.items.push(ImportedItem { element, fit });
        }
    }

    /// SmartArt items as a bulleted list over `at`, each level indented
    /// one more. False when they have no words.
    fn bullet_list(&mut self, items: &[Item], at: Placed, group: Option<&str>) -> bool {
        fn add(items: &[Item], depth: u8, out: &mut Vec<(u8, String)>) {
            for it in items {
                out.extend(it.text.lines().filter(|l| !l.trim().is_empty()).map(|l| (depth, l.trim().to_string())));
                add(&it.children, (depth + 1).min(8), out);
            }
        }
        let mut lines = vec![];
        add(items, 0, &mut lines);
        if lines.is_empty() {
            return false;
        }
        let text = lines.iter().map(|l| l.1.as_str()).collect::<Vec<_>>().join("\n");
        let chars = text.chars().count();
        let paras = lines.iter().map(|l| ParaFormat { indent: l.0, list: Some(ListKind::Bullet), align: None }).collect();
        let color = self.hex(&scheme_color("text1")).unwrap_or_else(|| "#000000".into()).to_ascii_lowercase();
        let mut data = TextData { text, style: TextStyle::Body, align: Align::Left, color: Some(color), fixed_width: true, paras, ..Default::default() };
        zebra_core::rich::normalize(&mut data);
        // The box, inset as a text box is; the text as big as fits it,
        // 18 point at most.
        let k = self.k;
        let (w, h) = ((at.w - 182_880.0).max(at.w * 0.5), (at.h - 91_440.0).max(at.h * 0.5));
        let pt = (h / EMU_PER_PT / (lines.len() as f64 * 1.25)).clamp(6.0, 18.0);
        let (bw, bh) = (w * k, h * k);
        let fit = SourceBox { x0: at.cx * k - bw / 2.0, y0: at.cy * k - bh / 2.0, x1: at.cx * k + bw / 2.0, y1: at.cy * k + bh / 2.0, anchor: BoxAnchor::Top };
        self.pending.push(PendingText { item: self.items.len(), base_pt: pt, chars, wrap: bw });
        self.items.push(ImportedItem {
            element: Element {
                id: String::new(),
                transform: Transform { x: at.cx * k, y: at.cy * k, scale: 1.0, rotation: at.rot },
                size: Size { w: bw, h: bh },
                locked: false,
                name: String::new(),
                placeholder: false,
                group: group.map(str::to_string),
                kind: ElementKind::Text(data),
            },
            fit: Some(fit),
        });
        true
    }

    /// The layout's and master's placeholders a slide placeholder fills.
    fn inherited(&self, ph: Option<&Placeholder>) -> (Option<&'a ShapeNode>, Option<&'a ShapeNode>) {
        let Some(ph) = ph else { return (None, None) };
        (self.layout.and_then(|l| find_placeholder(&l.shapes, ph)), self.master.and_then(|m| find_placeholder(&m.shapes, ph)))
    }

    fn shape(&mut self, node: &'a ShapeNode, s: &'a Shape, parents: &[&ShapeTransform], group: Option<&str>) {
        let (lay, mas) = self.inherited(s.base.placeholder.as_ref());
        // A placeholder without a place of its own sits where its layout's
        // (else its master's) does.
        let t = if s.base.transform.width == 0 && s.base.transform.height == 0 {
            [lay, mas].into_iter().flatten().map(node_transform).find(|t| t.width != 0 || t.height != 0).unwrap_or(&s.base.transform)
        } else {
            &s.base.transform
        };
        let mut at = place(t, parents);
        let nodes = [Some(node), lay, mas];
        // An equation comes in as the picture PowerPoint keeps of it (the
        // shape the parser reads), else is left out; either way, noted.
        let equation = self.shape_info(self.part, s.base.id).and_then(|i| i.stands_in) == Some(StandIn::Equation);
        // A shape filled with a picture is that picture.
        if let Some(path) = s.picture_fill.as_ref().and_then(|p| p.media_part_path.clone()) {
            let alt = s.base.description.clone().unwrap_or_default();
            let crop = s.picture_fill.as_ref().map(|p| p.crop.clone()).unwrap_or_default();
            let came = self.picture(&path, &crop, &s.geometry, at, alt, group);
            if equation {
                self.note(if came { ImportNote::AsPicture } else { ImportNote::Equation });
            }
        } else {
            if equation {
                self.note(ImportNote::Equation);
            }
            let custom = !s.has_preset_geometry && !s.paths.is_empty();
            let line = matches!(s.geometry.as_str(), "line" | "straightConnector1");
            let kind = (!custom && !line).then(|| shape_kind(&s.geometry)).flatten();
            let fill = if s.use_bg_fill {
                self.hex(&scheme_color("background1"))
            } else if kind.is_some() {
                self.resolved_fill(&nodes).and_then(|f| self.fill_color(&f))
            } else {
                // Only whether there is one (a shape left out has no colours to note).
                self.resolved_fill(&nodes).filter(|f| f.fill_type != "none").map(|_| String::new())
            };
            let outline = self.resolved_outline(&nodes);
            let stroke = outline.as_ref().and_then(|o| o.color.as_ref()).and_then(|c| self.hex(c));
            let width = outline.as_ref().and_then(|o| o.width).unwrap_or(12_700.0) * self.k;
            let dash = match outline.as_ref().and_then(|o| o.style.as_deref()) {
                Some("dot" | "sysDot") => Dash::Dotted,
                Some("solid") | None => Dash::Solid,
                Some(_) => Dash::Dashed,
            };
            if line && !custom {
                if let (Some(c), Some(o)) = (stroke, outline.as_ref()) {
                    self.line(at, o, c, width, dash, group);
                }
            } else if let Some(kind) = kind {
                if fill.is_some() || stroke.is_some() {
                    let radius = if kind == ShapeKind::RoundedRect { s.adjust_values.get("adj").copied().unwrap_or(16_667.0) / 100_000.0 * at.w.min(at.h) * self.k } else { 0.0 };
                    let kind = self.unflip(kind, &mut at);
                    let gid = s.text.as_ref().filter(|b| has_text(b)).map(|_| self.group_for(group));
                    let gid = gid.as_deref().or(group);
                    self.push_shape(kind, at, fill, stroke, width, radius, dash, gid);
                    if let Some(body) = &s.text {
                        self.text(s, body, (lay, mas), place(t, parents), gid);
                    }
                    return;
                }
            } else if fill.is_some() || stroke.is_some() {
                self.note(ImportNote::Shape);
            }
        }
        if let Some(body) = &s.text {
            self.text(s, body, (lay, mas), place(t, parents), group);
        }
    }

    /// A group for a shape and its text: the shape's own group, else a new one.
    fn group_for(&mut self, group: Option<&str>) -> String {
        match group {
            Some(g) => g.to_string(),
            None => {
                self.groups += 1;
                format!("g{}", self.groups)
            }
        }
    }

    /// Turn flips Zebra can't draw into turns where the shape looks the
    /// same; note the rest.
    fn unflip(&mut self, kind: ShapeKind, at: &mut Placed) -> ShapeKind {
        use ShapeKind as K;
        let mut kind = kind;
        if at.flip_h && at.flip_v {
            at.rot = (at.rot + 180.0).rem_euclid(360.0);
            at.flip_h = false;
            at.flip_v = false;
        }
        // Same either way round left to right / top to bottom.
        let even_h = matches!(kind, K::Rectangle | K::RoundedRect | K::Ellipse | K::Triangle | K::Diamond | K::Pentagon | K::Hexagon | K::Octagon | K::Trapezoid | K::Star | K::Heart | K::Plus | K::Ring | K::Cylinder | K::BlockArrowUp | K::BlockArrowDown | K::BlockArrowBoth);
        let even_v = matches!(kind, K::Rectangle | K::RoundedRect | K::Ellipse | K::Diamond | K::Hexagon | K::Octagon | K::Plus | K::Ring | K::Chevron | K::BlockArrowRight | K::BlockArrowLeft | K::BlockArrowBoth);
        if at.flip_h {
            at.flip_h = false;
            match kind {
                K::BlockArrowRight => kind = K::BlockArrowLeft,
                K::BlockArrowLeft => kind = K::BlockArrowRight,
                _ if even_h => {}
                _ if even_v => at.rot = (at.rot + 180.0).rem_euclid(360.0),
                _ => self.note(ImportNote::Flip),
            }
        }
        if at.flip_v {
            at.flip_v = false;
            match kind {
                K::BlockArrowUp => kind = K::BlockArrowDown,
                K::BlockArrowDown => kind = K::BlockArrowUp,
                _ if even_v => {}
                _ if even_h => at.rot = (at.rot + 180.0).rem_euclid(360.0),
                _ => self.note(ImportNote::Flip),
            }
        }
        kind
    }

    #[allow(clippy::too_many_arguments)]
    fn push_shape(&mut self, kind: ShapeKind, at: Placed, fill: Option<String>, stroke: Option<String>, width: f64, radius: f64, dash: Dash, group: Option<&str>) {
        let k = self.k;
        self.items.push(ImportedItem {
            element: Element {
                id: String::new(),
                transform: Transform { x: at.cx * k, y: at.cy * k, scale: 1.0, rotation: at.rot },
                size: Size { w: (at.w * k).max(0.5), h: (at.h * k).max(0.5) },
                locked: false,
                name: String::new(),
                placeholder: false,
                group: group.map(str::to_string),
                kind: ElementKind::Shape(ShapeData { shape: kind, fill, stroke, stroke_width: width.max(0.25), corner_radius: radius, dash }),
            },
            fit: None,
        });
    }

    /// A straight line or arrow, from its box's corner to the opposite one
    /// (which ones its flips say), arrowheads where the source has them.
    fn line(&mut self, at: Placed, o: &ShapeOutline, color: String, width: f64, dash: Dash, group: Option<&str>) {
        let head = |e: &Option<ooxml_drawingml::LineEnd>| e.as_ref().is_some_and(|e| e.end_type != "none");
        let (start, end) = (head(&o.head_end), head(&o.tail_end));
        let mut dx = if at.flip_h { -at.w } else { at.w };
        let mut dy = if at.flip_v { -at.h } else { at.h };
        let a = at.rot.to_radians();
        (dx, dy) = (dx * a.cos() - dy * a.sin(), dx * a.sin() + dy * a.cos());
        let kind = match (start, end) {
            (true, true) => ShapeKind::DoubleArrow,
            (false, true) => ShapeKind::Arrow,
            (true, false) => {
                (dx, dy) = (-dx, -dy);
                ShapeKind::Arrow
            }
            (false, false) => ShapeKind::Line,
        };
        let len = dx.hypot(dy);
        let k = self.k;
        self.items.push(ImportedItem {
            element: Element {
                id: String::new(),
                transform: Transform { x: at.cx * k, y: at.cy * k, scale: 1.0, rotation: dy.atan2(dx).to_degrees().rem_euclid(360.0) },
                size: Size { w: (len * k).max(0.5), h: (width * 4.0).max(1.0) },
                locked: false,
                name: String::new(),
                placeholder: false,
                group: group.map(str::to_string),
                kind: ElementKind::Shape(ShapeData { shape: kind, fill: None, stroke: Some(color), stroke_width: width.max(0.25), corner_radius: 0.0, dash }),
            },
            fit: None,
        });
    }

    /// A picture node (or the picture an embedded object keeps) placed by
    /// `t`. True when it came in.
    fn picture_node(&mut self, p: &Picture, t: &ShapeTransform, parents: &[&ShapeTransform], group: Option<&str>) -> bool {
        let Some(path) = p.media_part_path.clone() else {
            self.note(ImportNote::Picture);
            return false;
        };
        let alt = p.base.description.clone().filter(|d| !d.is_empty()).unwrap_or_default();
        self.picture(&path, &p.crop, &p.geometry, place(t, parents), alt, group)
    }

    /// The picture at media part `path` over `at`. True when it came in.
    fn picture(&mut self, path: &str, crop: &PictureCrop, geometry: &str, mut at: Placed, alt: String, group: Option<&str>) -> bool {
        let Some(key) = self.picture_asset(path) else { return false };
        self.unflip_picture(&mut at);
        let f = |v: i32| v.max(0) as f64 / 100_000.0;
        let (l, t, r, b) = (f(crop.left), f(crop.top), f(crop.right), f(crop.bottom));
        let crop = (l + r < 0.99 && t + b < 0.99 && (l + t + r + b) > 0.0).then(|| Crop { x: l, y: t, w: 1.0 - l - r, h: 1.0 - t - b }.clamped());
        let mask = match geometry {
            "ellipse" => ImageMask::Circle,
            "roundRect" => ImageMask::Rounded,
            _ => ImageMask::Rectangle,
        };
        let k = self.k;
        self.items.push(ImportedItem {
            element: Element {
                id: String::new(),
                transform: Transform { x: at.cx * k, y: at.cy * k, scale: 1.0, rotation: at.rot },
                size: Size { w: (at.w * k).max(0.5), h: (at.h * k).max(0.5) },
                locked: false,
                name: String::new(),
                placeholder: false,
                group: group.map(str::to_string),
                kind: ElementKind::Image(ImageData { asset: key, alt, crop, mask, symbol: None, page: None }),
            },
            fit: None,
        });
        true
    }

    /// The picture asset for media part `path` (read once per import), or
    /// None (noted the first time) when Zebra can't show it.
    fn picture_asset(&mut self, path: &str) -> Option<String> {
        match self.media.get(path) {
            Some(k) => k.clone(),
            None => {
                let key = self.media_asset(path);
                self.media.insert(path.to_string(), key.clone());
                key
            }
        }
    }

    /// Pictures can't flip: flipped both ways is turned half round; one
    /// way is shown unflipped (noted).
    fn unflip_picture(&mut self, at: &mut Placed) {
        if at.flip_h && at.flip_v {
            at.rot = (at.rot + 180.0).rem_euclid(360.0);
        } else if at.flip_h || at.flip_v {
            self.note(ImportNote::Flip);
        }
    }

    /// The video or sound picture `p` plays, taken out of the file: a
    /// video at the picture's place with the picture as its poster; a
    /// sound playing across slides as the document's music, one on its
    /// slide as the slide's voice-over. False when it can't come in (the
    /// file links to it, it is in a format a Mac won't play, or the music
    /// or voice-over is taken).
    fn media_node(&mut self, p: &Picture, m: &MediaRef, parents: &[&ShapeTransform], group: Option<&str>) -> bool {
        let found = m.rels.iter().filter_map(|id| self.index.rel(self.part, id)).filter_map(|r| r.resolved_target.as_deref()).find_map(|t| self.index.media.get(t).copied());
        let Some(part) = found else { return false };
        let Some((key, asset, info)) = self.src.av(&part.part_path) else { return false };
        let timing = self.infos.get(self.part).and_then(|i| i.timing.get(&p.base.id)).cloned().unwrap_or_default();
        match info.kind {
            MediaKind::Video => {
                let poster = p.media_part_path.clone().and_then(|path| self.picture_asset(&path));
                let mut at = place(&p.base.transform, parents);
                self.unflip_picture(&mut at);
                let start = m.trim.0 / 1000.0;
                let end = (m.trim.1 > 0.0 && info.duration > 0.0).then(|| info.duration - m.trim.1 / 1000.0).filter(|e| *e > start);
                let k = self.k;
                self.items.push(ImportedItem {
                    element: Element {
                        id: String::new(),
                        transform: Transform { x: at.cx * k, y: at.cy * k, scale: 1.0, rotation: at.rot },
                        size: Size { w: (at.w * k).max(0.5), h: (at.h * k).max(0.5) },
                        locked: false,
                        name: String::new(),
                        placeholder: false,
                        group: group.map(str::to_string),
                        kind: ElementKind::Video(VideoData {
                            asset: key.clone(),
                            link: None,
                            poster,
                            autoplay: timing.auto,
                            looping: timing.looping,
                            muted: timing.mute,
                            start: if start < info.duration || info.duration == 0.0 { start } else { 0.0 },
                            end,
                            duration: info.duration,
                            alt: p.base.description.clone().unwrap_or_default(),
                        }),
                    },
                    fit: None,
                });
            }
            MediaKind::Audio if timing.across => {
                if self.has_soundtrack || self.deck.soundtrack.is_some() {
                    return false;
                }
                let name = &p.base.name;
                let name = match name.rsplit_once('.') {
                    Some((stem, ext)) if (1..=4).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) => stem,
                    _ => name.as_str(),
                };
                // ECMA-376's default volume is half.
                self.deck.soundtrack = Some(Soundtrack { asset: key.clone(), looping: timing.looping, volume: timing.volume.unwrap_or(0.5), name: name.trim().to_string() });
            }
            MediaKind::Audio => {
                if self.voiceover.is_some() {
                    return false;
                }
                self.voiceover = Some(key.clone());
            }
            _ => return false,
        }
        self.deck.assets.entry(key).or_insert(asset);
        true
    }

    /// The asset for media part `path`, or None (noted) when Zebra can't
    /// show it.
    fn media_asset(&mut self, path: &str) -> Option<String> {
        let Some(part) = self.index.media.get(path).copied() else {
            self.note(ImportNote::Picture);
            return None;
        };
        let ct = part.content_type.to_ascii_lowercase();
        let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        let metafile = ct.contains("emf") || ct.contains("wmf") || matches!(ext.as_str(), "emf" | "wmf" | "emz" | "wmz");
        if !metafile && !ct.contains("svg") && ext != "svg" {
            // A plain picture: streamed out of the file, never whole in memory.
            return match self.src.picture(path) {
                Some((key, asset)) if asset.ext != "pdf" => {
                    self.deck.assets.entry(key.clone()).or_insert(asset);
                    Some(key)
                }
                _ => {
                    self.note(ImportNote::Picture);
                    None
                }
            };
        }
        let Some(mut bytes) = self.src.read(path, crate::pptx_media::MAX_CONVERTED) else {
            self.note(if metafile { ImportNote::Metafile } else { ImportNote::Picture });
            return None;
        };
        if metafile {
            // Drawn as SVG, as Keynote converts them.
            match metafile_svg(&bytes) {
                Some(svg) => bytes = svg,
                None => {
                    self.note(ImportNote::Metafile);
                    return None;
                }
            }
        }
        if metafile || ct.contains("svg") || ext == "svg" {
            // Zebra knows an SVG by its first tag.
            match bytes.windows(4).position(|w| w == b"<svg") {
                Some(i) => {
                    bytes.drain(..i);
                }
                None => {
                    self.note(ImportNote::Picture);
                    return None;
                }
            }
        }
        match asset_from(bytes) {
            Ok((key, asset)) if asset.ext != "pdf" => {
                self.deck.assets.entry(key.clone()).or_insert(asset);
                Some(key)
            }
            _ => {
                self.note(ImportNote::Picture);
                None
            }
        }
    }

    /// `body` as a Zebra text over its shape's box `at`, inset as the
    /// source insets it.
    fn text(&mut self, s: &'a Shape, body: &'a TextBody, inherited: (Option<&'a ShapeNode>, Option<&'a ShapeNode>), at: Placed, group: Option<&str>) {
        let c = Cascade {
            primary: Some(body),
            layout: inherited.0.and_then(node_text),
            master: inherited.1.and_then(node_text),
            placeholder: s.base.placeholder.as_ref(),
            style_color: s.style.as_ref().and_then(|st| st.font_color.as_ref()),
            strike: [
                self.text_strike(self.part, s.base.id),
                inherited.0.zip(self.layout).and_then(|(n, l)| self.text_strike(&l.part_path, n.id())),
                inherited.1.zip(self.master).and_then(|(n, m)| self.text_strike(&m.part_path, n.id())),
            ],
        };
        let bodies = [c.primary, c.layout, c.master];
        let pick = |f: &dyn Fn(&TextBody) -> Option<i64>| bodies.iter().flatten().find_map(|b| f(b));
        let il = pick(&|b| b.inset_left).unwrap_or(91_440) as f64;
        let ir = pick(&|b| b.inset_right).unwrap_or(91_440) as f64;
        let it = pick(&|b| b.inset_top).unwrap_or(45_720) as f64;
        let ib = pick(&|b| b.inset_bottom).unwrap_or(45_720) as f64;
        let anchor = match bodies.iter().flatten().find_map(|b| b.anchor.as_deref()) {
            Some("ctr") => BoxAnchor::Middle,
            Some("b") => BoxAnchor::Bottom,
            _ => BoxAnchor::Top,
        };
        let wraps = bodies.iter().flatten().find_map(|b| b.wrap).unwrap_or(true);
        let scale = match bodies.iter().flatten().find_map(|b| b.autofit.as_ref()) {
            Some(TextAutofit::Normal { font_scale: Some(f), .. }) if *f > 0.0 => *f,
            _ => 1.0,
        };
        let Some(built) = self.build_text(&c, scale) else { return };
        let k = self.k;
        let (w, h) = ((at.w - il - ir).max(at.w * 0.2), (at.h - it - ib).max(0.0));
        // The text box's centre, moved by uneven insets (turned with it).
        let (ox, oy) = ((il - ir) / 2.0, (it - ib) / 2.0);
        let a = at.rot.to_radians();
        let (cx, cy) = (at.cx + ox * a.cos() - oy * a.sin(), at.cy + ox * a.sin() + oy * a.cos());
        let (bw, bh) = (w * k, h * k);
        let fit = SourceBox { x0: cx * k - bw / 2.0, y0: cy * k - bh / 2.0, x1: cx * k + bw / 2.0, y1: cy * k + bh / 2.0, anchor };
        let (data, base_pt, chars) = built;
        let mut data = data;
        data.fixed_width = wraps;
        self.pending.push(PendingText { item: self.items.len(), base_pt, chars, wrap: if wraps { bw } else { 0.0 } });
        self.items.push(ImportedItem {
            element: Element {
                id: String::new(),
                transform: Transform { x: cx * k, y: cy * k, scale: 1.0, rotation: at.rot },
                size: Size { w: bw, h: bh },
                locked: false,
                name: String::new(),
                placeholder: false,
                group: group.map(str::to_string),
                kind: ElementKind::Text(data),
            },
            fit: Some(fit),
        });
    }

    /// The text, its base size in points (the size most characters have)
    /// and its length, or None when there is nothing to show.
    fn build_text(&self, c: &Cascade, font_scale: f64) -> Option<(TextData, f64, usize)> {
        struct Run {
            start: usize,
            end: usize,
            pt: f64,
            bold: bool,
            italic: bool,
            underline: bool,
            strike: bool,
            color: String,
            link: Option<String>,
        }
        let body = c.primary?;
        let mut text = String::new();
        let mut paras: Vec<(u8, ListKind, Align)> = vec![];
        let mut runs: Vec<Run> = vec![];
        let mut n = 0usize;
        let default_color = self.hex(&scheme_color("text1")).unwrap_or_else(|| "#000000".into());
        for (pi, p) in body.paragraphs.iter().enumerate() {
            let level = p.properties.level.min(8);
            let props = self.paragraph_properties(c, pi, level);
            let list = match &props.bullet {
                Some(Bullet::Character { .. }) => ListKind::Bullet,
                Some(Bullet::AutoNumber { .. }) => ListKind::Number,
                _ => ListKind::None,
            };
            let align = match props.alignment.as_deref() {
                Some("ctr") => Align::Center,
                Some("r") => Align::Right,
                _ => Align::Left,
            };
            if !paras.is_empty() {
                text.push('\n');
                n += 1;
            }
            let has_runs = p.runs.iter().any(|r| !r.line_break && !r.text.is_empty());
            paras.push((level as u8, if has_runs { list } else { ListKind::None }, align));
            let para_strike = self.strike_default(c, pi, level as usize);
            let run_strike = c.strike[0].and_then(|t| t.paras.get(pi));
            for (ri, r) in p.runs.iter().enumerate() {
                if r.line_break {
                    // A line break inside a paragraph: Zebra starts a new
                    // paragraph, without a bullet of its own.
                    text.push('\n');
                    n += 1;
                    paras.push((level as u8, ListKind::None, align));
                    continue;
                }
                let mut rp = props.default_run.clone().unwrap_or_default();
                merge_run(&mut rp, &r.properties);
                let mut t = r.text.replace(['\r', '\n', '\u{b}'], " ");
                if rp.caps == Some(TextCaps::All) {
                    t = t.to_uppercase();
                }
                let len = t.chars().count();
                if len == 0 {
                    continue;
                }
                let pt = rp.font_size_pt.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(18.0) * font_scale;
                let color = rp.color.as_ref().and_then(|c| self.hex(c)).unwrap_or_else(|| default_color.clone());
                let link = rp.hyperlink_relationship_id.as_deref().and_then(|id| self.link(id));
                runs.push(Run {
                    start: n,
                    end: n + len,
                    pt,
                    bold: rp.bold.unwrap_or(false),
                    italic: rp.italic.unwrap_or(false),
                    underline: rp.underline.as_deref().is_some_and(|u| u != "none"),
                    strike: run_strike.and_then(|p| p.1.get(ri).copied().flatten()).or(para_strike).unwrap_or(false),
                    color,
                    link,
                });
                text.push_str(&t);
                n += len;
            }
        }
        // Empty lines at the end take room and show nothing.
        while text.ends_with('\n') {
            text.pop();
            paras.pop();
        }
        if text.trim().is_empty() || runs.is_empty() {
            return None;
        }
        let chars = |r: &Run| r.end - r.start;
        let commonest = |key: &dyn Fn(&Run) -> String| -> String {
            let mut m: BTreeMap<String, usize> = BTreeMap::new();
            for r in &runs {
                *m.entry(key(r)).or_default() += chars(r);
            }
            m.into_iter().max_by_key(|(_, c)| *c).map(|(k, _)| k).unwrap_or_default()
        };
        let base_pt: f64 = commonest(&|r| format!("{:.1}", r.pt)).parse().unwrap_or(18.0);
        let color = commonest(&|r| r.color.clone());
        let visible = |r: &&Run| text.chars().skip(r.start).take(r.end - r.start).any(|c| !c.is_whitespace());
        let bold = runs.iter().filter(visible).all(|r| r.bold);
        let italic = runs.iter().filter(visible).all(|r| r.italic);
        let mut align_count: BTreeMap<u8, usize> = BTreeMap::new();
        for (_, _, a) in &paras {
            *align_count.entry(*a as u8).or_default() += 1;
        }
        let box_align = paras.iter().map(|p| p.2).max_by_key(|a| (align_count[&(*a as u8)], std::cmp::Reverse(*a as u8))).unwrap_or_default();
        let spans = runs
            .iter()
            .map(|r| TextSpan {
                start: r.start,
                end: r.end,
                format: SpanFormat {
                    bold: (r.bold != bold).then_some(r.bold),
                    italic: (r.italic != italic).then_some(r.italic),
                    underline: r.underline.then_some(true),
                    strike: r.strike.then_some(true),
                    color: (r.color != color).then(|| r.color.to_ascii_lowercase()),
                    link: r.link.clone(),
                    size: ((r.pt / base_pt - 1.0).abs() > 0.02).then_some(r.pt / base_pt),
                },
            })
            .collect();
        let paras = paras.iter().map(|(level, list, align)| ParaFormat { indent: *level, list: Some(*list), align: (*align != box_align).then_some(*align) }).collect();
        let mut data = TextData { text, style: TextStyle::Body, bold, italic, align: box_align, bullets: false, color: Some(color.to_ascii_lowercase()), fixed_width: true, spans, paras };
        zebra_core::rich::normalize(&mut data);
        Some((data, base_pt, n))
    }

    /// The strike-through part `part`'s shape `id` says (None when it
    /// says none).
    fn text_strike(&self, part: &str, id: u32) -> Option<&'a TextStrike> {
        self.shape_info(part, id)?.text.as_ref()
    }

    /// What the parser's model leaves out of part `part`'s shape `id`.
    fn shape_info(&self, part: &str, id: u32) -> Option<&'a ShapeInfo> {
        let infos: &'a BTreeMap<String, PartInfo> = self.infos;
        infos.get(part)?.shapes.get(&id)
    }

    /// Whether paragraph `index` at `level`'s runs strike through unless
    /// they say otherwise: the cascade of [`Self::paragraph_properties`].
    fn strike_default(&self, c: &Cascade, index: usize, level: usize) -> Option<bool> {
        let level = level.min(8);
        let mut out = None;
        let mut set = |v: Option<bool>| {
            if v.is_some() {
                out = v;
            }
        };
        if let Some(info) = self.master.and_then(|m| self.infos.get(&m.part_path)) {
            set(info.styles[style_kind(c.placeholder) as usize][level]);
        }
        if c.placeholder.is_none() {
            if let Some(info) = self.infos.get(&self.pkg.presentation.part_path) {
                set(info.default);
                set(info.styles[StyleKind::Other as usize][level]);
            }
        }
        for (t, own) in [(c.strike[2], false), (c.strike[1], false), (c.strike[0], true)] {
            let Some(t) = t else { continue };
            set(t.default);
            set(t.levels[level]);
            let p = if own { t.paras.get(index) } else { t.paras.get(index).or_else(|| t.paras.get(level)) };
            if let Some(p) = p {
                set(p.0);
            }
        }
        out
    }

    /// The URL a run's link relationship points at: web and mail links
    /// only ([`web_link`]). Links to other slides, and links that would run
    /// something (`javascript:`, `file:`…), are left out: the run's words
    /// stay, plain.
    fn link(&self, id: &str) -> Option<String> {
        let r = self.index.rel(self.part, id)?;
        if r.target_mode != TargetMode::External && !r.target.contains(':') {
            return None;
        }
        web_link(&r.target)
    }

    /// Paragraph `index` at `level`'s properties, from the master's text
    /// styles (or the presentation's defaults for text outside
    /// placeholders) through the master's, layout's and shape's own lists.
    fn paragraph_properties(&self, c: &Cascade, index: usize, level: u32) -> ParagraphProperties {
        let mut out = self.master.and_then(|m| master_style(m, c.placeholder, level)).cloned().unwrap_or_default();
        if c.placeholder.is_none() {
            let pres = &self.pkg.presentation;
            let lvl = pres.default_text_style.get(level as usize).or_else(|| pres.default_text_style.first());
            for src in pres.default_text_paragraph.as_deref().into_iter().chain(lvl) {
                merge_paragraph(&mut out, src);
            }
        }
        if let Some(col) = c.style_color {
            out.default_run.get_or_insert_with(RunProperties::default).color = Some(col.clone());
        }
        for (body, own) in [(c.master, false), (c.layout, false), (c.primary, true)] {
            let Some(body) = body else { continue };
            if let Some(src) = &body.default_list_style {
                merge_paragraph(&mut out, src);
            }
            if let Some(src) = body.list_style.get(level as usize) {
                merge_paragraph(&mut out, src);
            }
            let p = if own { body.paragraphs.get(index) } else { body.paragraphs.get(index).or_else(|| body.paragraphs.get(level as usize)) };
            if let Some(p) = p {
                merge_paragraph(&mut out, &p.properties);
            }
        }
        out
    }

    fn resolved_fill(&self, nodes: &[Option<&ShapeNode>]) -> Option<ShapeFill> {
        nodes
            .iter()
            .flatten()
            .find_map(|n| if node_style(n).is_some_and(|s| s.fill_disabled) { Some(ShapeFill::named("none")) } else { node_fill(n).cloned() })
            .or_else(|| {
                let r = nodes.iter().flatten().find_map(|n| node_style(n)?.fill.as_ref())?;
                Some(style_fill(&self.scheme, r, &self.theme))
            })
    }

    fn resolved_outline(&self, nodes: &[Option<&ShapeNode>]) -> Option<ShapeOutline> {
        let mut out = nodes.iter().flatten().find_map(|n| {
            let r = node_style(n)?.line.as_ref()?;
            Some(style_outline(&self.scheme, r, &self.theme).unwrap_or_default())
        });
        for n in nodes.iter().rev().flatten() {
            if node_style(n).is_some_and(|s| s.line_disabled) {
                out = Some(ShapeOutline::default());
            }
            if let Some(d) = node_outline(n) {
                let base = out.unwrap_or_default();
                out = Some(ShapeOutline {
                    width: d.width.or(base.width),
                    color: d.color.clone().or(base.color),
                    style: d.style.clone().or(base.style),
                    head_end: d.head_end.clone().or(base.head_end),
                    tail_end: d.tail_end.clone().or(base.tail_end),
                    ..d.clone()
                });
            }
        }
        out
    }

    /// A fill's colour: solid fills their own, gradients their first stop
    /// (noted); none, patterns and pictures none.
    fn fill_color(&mut self, f: &ShapeFill) -> Option<String> {
        match f.fill_type.as_str() {
            "solid" => f.color.as_ref().and_then(|c| self.hex(c)),
            "gradient" => {
                let c = f.gradient.as_ref().and_then(|g| g.stops.first()).map(|s| s.color.clone()).or_else(|| f.color.clone())?;
                self.note(ImportNote::Gradient);
                self.hex(&c)
            }
            _ => None,
        }
    }

    /// `#rrggbb` for a colour, theme colours resolved through the slide's
    /// colour map. Fully transparent colours are none.
    fn hex(&self, c: &ColorValue) -> Option<String> {
        if c.alpha.is_some_and(|a| a <= 0.0) {
            return None;
        }
        resolve_color_value_to_hex_with_theme(Some(c), Some(&self.theme)).map(|h| h.to_ascii_lowercase())
    }
}

/// `url` when it is a web or mail link (http, https or mailto, in any
/// case; its scheme is written in lower case), else None. Spaces round it
/// and tabs and line breaks inside are dropped first, as browsers drop
/// them (`java\tscript:` is `javascript:` to a browser).
pub fn web_link(url: &str) -> Option<String> {
    let url: String = url.trim_matches(|c: char| c.is_whitespace() || c.is_control()).chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
    let (scheme, rest) = url.split_once(':')?;
    let scheme = scheme.to_ascii_lowercase();
    match scheme.as_str() {
        "http" | "https" if rest.starts_with("//") && rest.len() > 2 => Some(format!("{scheme}:{rest}")),
        "mailto" if !rest.is_empty() => Some(format!("{scheme}:{rest}")),
        _ => None,
    }
}

/// An EMF or WMF picture (or a gzipped .emz or .wmz) as an SVG document;
/// None when it can't be drawn.
fn metafile_svg(bytes: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    /// A compressed metafile bigger than this is refused.
    const MAX: u64 = 64 << 20;
    let mut plain = vec![];
    let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
        flate2::read::GzDecoder::new(bytes).take(MAX + 1).read_to_end(&mut plain).ok()?;
        (plain.len() as u64 <= MAX).then_some(plain.as_slice())?
    } else {
        bytes
    };
    if !ooxml_metafile::is_metafile(bytes) {
        return None;
    }
    let svg = ooxml_metafile::to_svg(bytes).ok()?;
    (svg.width > 0.0 && svg.height > 0.0).then(|| svg.markup.into_bytes())
}

/// The Zebra diagram for SmartArt layout `layout` (the last part of its
/// id) holding `items`, when Zebra has one.
fn diagram_for(layout: &str, items: &[Item]) -> Option<&'static str> {
    Some(match layout {
        // Basic Block List (PowerPoint's default).
        "default" => "block_list",
        // Basic Process.
        "process1" => "process",
        // Basic Chevron Process, Closed Chevron Process.
        "chevron1" | "hChevron3" => "chevrons",
        // The cycles of nodes round a loop.
        "cycle1" | "cycle2" | "cycle3" | "cycle5" | "cycle6" => "cycle",
        // Basic Timeline.
        "hProcess11" => "timeline",
        "funnel1" => "funnel",
        // Basic Target.
        "target1" => "target",
        // Basic Venn.
        "venn1" => match items.len() {
            2 => "venn2",
            3 => "venn3",
            _ => return None,
        },
        // Hierarchy, Organization Chart; Horizontal Hierarchy.
        "hierarchy1" | "orgChart1" => "org_chart",
        "hierarchy2" => "tree",
        _ => return None,
    })
}

fn scheme_color(slot: &str) -> ColorValue {
    ColorValue { theme_color: Some(slot.to_string()), ..ColorValue::default() }
}

/// The Zebra shape for a DrawingML preset, where Zebra has one that looks
/// the same.
fn shape_kind(prst: &str) -> Option<ShapeKind> {
    use ShapeKind as K;
    Some(match prst {
        "rect" | "flowChartProcess" => K::Rectangle,
        "roundRect" | "flowChartAlternateProcess" => K::RoundedRect,
        "ellipse" | "flowChartConnector" => K::Ellipse,
        "triangle" => K::Triangle,
        "rtTriangle" => K::RightTriangle,
        "diamond" | "flowChartDecision" => K::Diamond,
        "pentagon" => K::Pentagon,
        "hexagon" => K::Hexagon,
        "octagon" => K::Octagon,
        "parallelogram" | "flowChartInputOutput" => K::Parallelogram,
        "trapezoid" => K::Trapezoid,
        "star5" => K::Star,
        "heart" => K::Heart,
        "plus" => K::Plus,
        "donut" => K::Ring,
        "cloud" => K::Cloud,
        "can" => K::Cylinder,
        "cube" => K::Cube,
        "chevron" => K::Chevron,
        "flowChartDocument" => K::Document,
        "rightArrow" => K::BlockArrowRight,
        "leftArrow" => K::BlockArrowLeft,
        "upArrow" => K::BlockArrowUp,
        "downArrow" => K::BlockArrowDown,
        "leftRightArrow" => K::BlockArrowBoth,
        _ => return None,
    })
}

fn has_text(b: &TextBody) -> bool {
    b.paragraphs.iter().any(|p| p.runs.iter().any(|r| !r.line_break && !r.text.trim().is_empty()))
}

fn node_transform(n: &ShapeNode) -> &ShapeTransform {
    match n {
        ShapeNode::Shape(s) => &s.base.transform,
        ShapeNode::Picture(p) => &p.base.transform,
        ShapeNode::GraphicFrame(f) => &f.base.transform,
        ShapeNode::Group(g) => &g.base.transform,
    }
}

fn node_text(n: &ShapeNode) -> Option<&TextBody> {
    match n {
        ShapeNode::Shape(s) => s.text.as_ref(),
        _ => None,
    }
}

fn node_style(n: &ShapeNode) -> Option<&pptx_parse::ShapeStyle> {
    match n {
        ShapeNode::Shape(s) => s.style.as_deref(),
        ShapeNode::Picture(p) => p.style.as_deref(),
        _ => None,
    }
}

fn node_fill(n: &ShapeNode) -> Option<&ShapeFill> {
    match n {
        ShapeNode::Shape(s) => s.fill.as_ref(),
        ShapeNode::Picture(p) => p.fill.as_ref(),
        _ => None,
    }
}

fn node_outline(n: &ShapeNode) -> Option<&ShapeOutline> {
    match n {
        ShapeNode::Shape(s) => s.outline.as_ref(),
        ShapeNode::Picture(p) => p.outline.as_ref(),
        _ => None,
    }
}

/// The first object in `nodes` (groups included) filling placeholder `ph`.
fn find_placeholder<'n>(nodes: &'n [ShapeNode], ph: &Placeholder) -> Option<&'n ShapeNode> {
    nodes.iter().find_map(|n| {
        let base = match n {
            ShapeNode::Shape(s) => &s.base,
            ShapeNode::Picture(p) => &p.base,
            ShapeNode::GraphicFrame(f) => &f.base,
            ShapeNode::Group(g) => return find_placeholder(&g.children, ph),
        };
        base.placeholder.as_ref().is_some_and(|p| p.matches(ph)).then_some(n)
    })
}

/// The master's text style a placeholder of this kind takes (text
/// outside placeholders: other).
fn style_kind(ph: Option<&Placeholder>) -> StyleKind {
    match ph.map(|p| p.placeholder_type.as_deref().unwrap_or("body")) {
        Some("title" | "ctrTitle") => StyleKind::Title,
        Some("body" | "obj" | "subTitle") => StyleKind::Body,
        _ => StyleKind::Other,
    }
}

/// The master's title, body or other text style at `level` for a
/// placeholder of this kind (text outside placeholders: other).
fn master_style<'m>(m: &'m SlideMaster, ph: Option<&Placeholder>, level: u32) -> Option<&'m ParagraphProperties> {
    let styles = match style_kind(ph) {
        StyleKind::Title => &m.text_styles.title,
        StyleKind::Body => &m.text_styles.body,
        StyleKind::Other => &m.text_styles.other,
    };
    styles.get(level as usize).or_else(|| styles.first())
}

fn merge_paragraph(t: &mut ParagraphProperties, s: &ParagraphProperties) {
    if s.alignment.is_some() {
        t.alignment.clone_from(&s.alignment);
    }
    if s.bullet.is_some() {
        t.bullet.clone_from(&s.bullet);
    }
    if let Some(r) = &s.default_run {
        merge_run(t.default_run.get_or_insert_with(RunProperties::default), r);
    }
}

fn merge_run(t: &mut RunProperties, s: &RunProperties) {
    if s.font_size_pt.is_some() {
        t.font_size_pt = s.font_size_pt;
    }
    if s.bold.is_some() {
        t.bold = s.bold;
    }
    if s.italic.is_some() {
        t.italic = s.italic;
    }
    if s.underline.is_some() {
        t.underline.clone_from(&s.underline);
    }
    if s.caps.is_some() {
        t.caps = s.caps;
    }
    if s.color.is_some() {
        t.color.clone_from(&s.color);
    }
    if s.hyperlink_relationship_id.is_some() {
        t.hyperlink_relationship_id.clone_from(&s.hyperlink_relationship_id);
    }
}

/// Count equations (paragraph content the parser leaves out) in `shapes`.
fn count_equations(shapes: &[SourceShape], note: &mut impl FnMut(ImportNote)) {
    for s in shapes {
        for o in &s.omitted_inlines {
            if o.element.rsplit(':').next() == Some("m") || o.element.ends_with("oMath") || o.element.ends_with("oMathPara") {
                note(ImportNote::Equation);
            }
        }
        count_equations(&s.children, note);
    }
}
