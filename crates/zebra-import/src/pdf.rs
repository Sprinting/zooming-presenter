//! PDF pages through PDFium: text, rectangles, ovals, lines and pictures
//! come in as Zebra objects; other vector art as SVG pictures; what Zebra
//! can't draw (equations, gradients, clipping, transparency effects) as
//! pictures of just those objects, rendered by PDFium ("patches").
//!
//! PDFium is a separate library, loaded at run time by full path from the
//! app bundle and never from the system's search paths.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use pdfium_render::prelude::*;
use zebra_core::import::{asset_from, ImportNote, ImportedDeck, ImportedItem, ImportedPage, SourceBox};
use zebra_core::{Element, ElementKind, ImageData, ShapeData, ShapeKind, Size, Transform};

use crate::text::{self, Block, Container, Run};
use crate::{Control, ImportError, Options, Reader};

/// Where PDFium is: inside the app (Zebra.app/Contents/Frameworks on the
/// Mac, beside the program elsewhere). Debug builds and tests also take
/// `ZEBRA_PDFIUM` (the library file); see scripts/fetch-pdfium.sh.
pub fn library_path() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("ZEBRA_PDFIUM").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = Pdfium::pdfium_platform_library_name();
    let p = if cfg!(target_os = "macos") { dir.parent()?.join("Frameworks").join(name) } else { dir.join(name) };
    p.is_file().then_some(p)
}

/// The loaded PDFium (bound once per process).
pub fn pdfium() -> Result<&'static Pdfium, ImportError> {
    static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();
    PDFIUM
        .get_or_init(|| {
            let p = library_path().ok_or_else(|| "libpdfium not found".to_string())?;
            Pdfium::bind_to_library(&p).map(Pdfium::new).map_err(|e| format!("{e}"))
        })
        .as_ref()
        .map_err(|e| ImportError::NoEngine(e.clone()))
}

/// Can PDFs be imported here (is PDFium in place and loadable)?
pub fn available() -> bool {
    pdfium().is_ok()
}

fn load_error(e: PdfiumError) -> ImportError {
    match e {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError | PdfiumInternalError::SecurityError) => ImportError::Encrypted,
        _ => ImportError::Damaged("PDF"),
    }
}

pub struct PdfReader {
    bytes: Vec<u8>,
    title: String,
    pages: usize,
}

impl PdfReader {
    pub fn new(bytes: Vec<u8>, title: String) -> Result<PdfReader, ImportError> {
        let lib = pdfium()?;
        let pages = {
            let doc = lib.load_pdf_from_byte_slice(&bytes, None).map_err(load_error)?;
            doc.pages().len() as usize
        };
        if pages == 0 {
            return Err(ImportError::Damaged("PDF"));
        }
        Ok(PdfReader { bytes, title, pages })
    }
}

impl Reader for PdfReader {
    fn kind(&self) -> &'static str {
        "PDF"
    }
    fn page_count(&self) -> usize {
        self.pages
    }
    fn read(&self, opts: &Options, ctl: &Control) -> Result<ImportedDeck, ImportError> {
        let lib = pdfium()?;
        let doc = lib.load_pdf_from_byte_slice(&self.bytes, None).map_err(load_error)?;
        let mut deck = ImportedDeck { title: self.title.clone(), ..Default::default() };
        let mut walked = vec![];
        for i in crate::page_range(opts, self.pages) {
            if ctl.cancelled() {
                return Err(ImportError::Cancelled);
            }
            // A page PDFium can't open is left out, not the whole file.
            let walked_page = match doc.pages().get(i as i32) {
                Ok(mut page) => walk_page(&doc, &mut page, i, &mut deck),
                Err(_) => {
                    *deck.notes.entry(ImportNote::ObjectSkipped).or_default() += 1;
                    None
                }
            };
            if let Some(w) = walked_page {
                walked.push(w);
            }
            ctl.page_done();
        }
        if walked.is_empty() {
            return Err(ImportError::Damaged("PDF"));
        }
        finish(&mut deck, walked, opts);
        Ok(deck)
    }
}

// ----------------------------------------------------------------------
// Walking a page

/// Affine matrix, PDF row-vector convention (p' = p * M).
#[derive(Clone, Copy, Debug)]
struct M {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl M {
    const ID: M = M { a: 1., b: 0., c: 0., d: 1., e: 0., f: 0. };
    fn from(p: &PdfMatrix) -> M {
        M { a: p.a() as f64, b: p.b() as f64, c: p.c() as f64, d: p.d() as f64, e: p.e() as f64, f: p.f() as f64 }
    }
    /// self, then `o`.
    fn then(&self, o: &M) -> M {
        M {
            a: self.a * o.a + self.b * o.c,
            b: self.a * o.b + self.b * o.d,
            c: self.c * o.a + self.d * o.c,
            d: self.c * o.b + self.d * o.d,
            e: self.e * o.a + self.f * o.c + o.e,
            f: self.e * o.b + self.f * o.d + o.f,
        }
    }
    fn ap(&self, x: f64, y: f64) -> (f64, f64) {
        (x * self.a + y * self.c + self.e, x * self.b + y * self.d + self.f)
    }
    fn det(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }
    fn rot_deg(&self) -> f64 {
        self.b.atan2(self.a).to_degrees()
    }
    fn skewed(&self) -> bool {
        let sx = self.a.hypot(self.b);
        let sy = self.c.hypot(self.d);
        sx > 0. && sy > 0. && ((self.a * self.c + self.b * self.d) / (sx * sy)).abs() > 0.01
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Seg {
    M(f64, f64),
    L(f64, f64),
    C(f64, f64, f64, f64, f64, f64),
    Z,
}

/// Something on the page, in page points with y down.
enum Item {
    Shape { data: ShapeData, cx: f64, cy: f64, w: f64, h: f64, rot: f64 },
    Picture { asset: String, cx: f64, cy: f64, w: f64, h: f64, rot: f64, alt: String },
    Text(Block),
}

struct Walked {
    w: f64,
    h: f64,
    index: usize,
    /// (z, item)
    items: Vec<(f64, Item)>,
}

/// A part of the page that comes in as a picture of itself.
#[derive(Clone)]
struct Patch {
    /// Page points, y down.
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    z: f64,
    notes: Vec<ImportNote>,
}

struct Page<'p> {
    /// Page size in points.
    w: f64,
    h: f64,
    runs: Vec<Run>,
    items: Vec<(f64, Item)>,
    patches: Vec<Patch>,
    containers: Vec<Container>,
    /// Forms whose objects were walked one by one (by `z`).
    recursed: std::collections::HashSet<i64>,
    deck: &'p mut ImportedDeck,
    z: f64,
}

fn note(deck: &mut ImportedDeck, n: ImportNote) {
    *deck.notes.entry(n).or_default() += 1;
}

fn hex(c: &PdfColor) -> String {
    if c.alpha() == 255 {
        format!("#{:02x}{:02x}{:02x}", c.red(), c.green(), c.blue())
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.red(), c.green(), c.blue(), c.alpha())
    }
}

fn quad(q: &PdfQuadPoints, m: &M) -> Vec<(f64, f64)> {
    [(q.x1(), q.y1()), (q.x2(), q.y2()), (q.x3(), q.y3()), (q.x4(), q.y4())]
        .iter()
        .map(|(x, y)| m.ap(x.value as f64, y.value as f64))
        .collect()
}

fn bbox(p: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    (
        p.iter().map(|q| q.0).fold(f64::MAX, f64::min),
        p.iter().map(|q| q.0).fold(f64::MIN, f64::max),
        p.iter().map(|q| q.1).fold(f64::MAX, f64::min),
        p.iter().map(|q| q.1).fold(f64::MIN, f64::max),
    )
}

/// Equation text: maths letters and operators (PowerPoint's and Word's
/// equations), or TeX's maths fonts. A font called "Cambria Math" alone
/// says nothing: PowerPoint writes ordinary Cambria text with that name.
fn is_math(text: &str, font: &str) -> bool {
    let n = font.to_ascii_lowercase();
    ["cmmi", "cmsy", "cmex", "msbm", "msam", "mtextra", "mt extra"].iter().any(|p| n.contains(p))
        || text.chars().any(|c| {
            matches!(c as u32, 0x1D400..=0x1D7FF | 0x2200..=0x22FF | 0x2A00..=0x2AFF | 0x27C0..=0x27EF | 0x2980..=0x29FF | 0x2061..=0x2064)
                // Greek in a maths font.
                || (n.contains("math") && matches!(c as u32, 0x0391..=0x03C9))
        })
}

fn walk_page(doc: &PdfDocument, page: &mut PdfPage, index: usize, deck: &mut ImportedDeck) -> Option<Walked> {
    let (w, h) = (page.width().value as f64, page.height().value as f64);
    if !(w > 1.0 && h > 1.0 && w.is_finite() && h.is_finite()) {
        note(deck, ImportNote::ObjectSkipped);
        return None;
    }
    let mut pg = Page { w, h, runs: vec![], items: vec![], patches: vec![], containers: vec![], recursed: Default::default(), deck, z: 0.0 };
    let n = page.objects().len();
    for i in 0..n {
        if let Ok(mut obj) = page.objects().get(i) {
            pg.walk(doc, &mut obj, M::ID, true);
        }
    }
    // A line with maths in it comes in as a picture, whole.
    let (math, lines) = text::split_math(text::lines(std::mem::take(&mut pg.runs)));
    let mut wake = std::collections::HashSet::new();
    for l in &math {
        let pad = 0.75;
        let (x0, y0) = (l.iter().map(|r| r.x0).fold(f64::MAX, f64::min) - pad, l.iter().map(|r| r.y0).fold(f64::MAX, f64::min) - pad);
        let (x1, y1) = (l.iter().map(|r| r.x1).fold(f64::MIN, f64::max) + pad, l.iter().map(|r| r.y1).fold(f64::MIN, f64::max) + pad);
        let z = l.iter().map(|r| r.z).fold(f64::MAX, f64::min);
        pg.patches.push(Patch { x0: x0.max(0.0), y0: y0.max(0.0), x1: x1.min(w), y1: y1.min(h), z, notes: vec![ImportNote::EquationAsPicture] });
        wake.extend(l.iter().map(|r| r.z as i64));
    }
    if !wake.is_empty() {
        let mut z = 0i64;
        for i in 0..n {
            if let Ok(mut obj) = page.objects().get(i) {
                reactivate(&mut obj, &mut z, &wake, &pg.recursed);
            }
        }
    }
    for b in text::boxes(lines, &pg.containers, w) {
        pg.items.push((b.z, Item::Text(b)));
    }
    let patches = merge(std::mem::take(&mut pg.patches));
    for p in &patches {
        for n in &p.notes {
            note(pg.deck, *n);
        }
    }
    let mut items = std::mem::take(&mut pg.items);
    if !patches.is_empty() {
        render_patches(page, &patches, pg.deck, &mut items);
    }
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    // PDF pages are white: say so, unless the page paints its own background.
    let covers = |it: &Item| matches!(it, Item::Shape { data, w: sw, h: sh, .. } if data.fill.as_deref().is_some_and(|f| f.len() == 7) && *sw >= w - 1.0 && *sh >= h - 1.0);
    if !items.first().is_some_and(|(_, it)| covers(it)) {
        let data = ShapeData { shape: ShapeKind::Rectangle, fill: Some("#ffffff".into()), stroke: None, stroke_width: 0.0, ..Default::default() };
        items.insert(0, (f64::MIN, Item::Shape { data, cx: w / 2.0, cy: h / 2.0, w, h, rot: 0.0 }));
    }
    Some(Walked { w, h, index, items })
}

impl Page<'_> {
    /// Page space (y up) to page points y down.
    fn down(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (x, self.h - y)
    }

    fn patch(&mut self, pts: &[(f64, f64)], pad: f64, n: ImportNote) {
        let (x0, x1, y0, y1) = bbox(pts);
        let (x0, y0, x1, y1) = (x0 - pad, self.h - y1 - pad, x1 + pad, self.h - y0 + pad);
        // Only what's on the page.
        let (x0, y0, x1, y1) = (x0.max(0.0), y0.max(0.0), x1.min(self.w), y1.min(self.h));
        if x1 - x0 < 0.5 || y1 - y0 < 0.5 {
            return;
        }
        self.patches.push(Patch { x0, y0, x1, y1, z: self.z, notes: vec![n] });
    }

    /// Object bounds in page space (y up), cut to its clip path's box.
    fn visible_bounds(&self, obj: &PdfPageObject, parent: &M) -> Option<Vec<(f64, f64)>> {
        let q = obj.bounds().ok()?;
        let mut b = bbox(&quad(&q, parent));
        if let Some(cp) = obj.get_clip_path() {
            for sub in cp.iter() {
                let pts: Vec<(f64, f64)> = sub.iter().map(|s| parent.ap(s.x().value as f64, s.y().value as f64)).collect();
                if pts.is_empty() {
                    continue;
                }
                let c = bbox(&pts);
                b = (b.0.max(c.0), b.1.min(c.1), b.2.max(c.2), b.3.min(c.3));
            }
        }
        (b.0 < b.1 && b.2 < b.3).then(|| vec![(b.0, b.2), (b.1, b.3)])
    }

    /// Does a clip path cut into what the object shows?
    fn clip_cuts(&self, obj: &PdfPageObject, parent: &M) -> bool {
        let Some(cp) = obj.get_clip_path() else { return false };
        let Ok(q) = obj.bounds() else { return false };
        let ob = bbox(&quad(&q, parent));
        for sub in cp.iter() {
            let pts: Vec<(f64, f64)> = sub.iter().map(|s| parent.ap(s.x().value as f64, s.y().value as f64)).collect();
            if pts.is_empty() {
                continue;
            }
            let cb = bbox(&pts);
            // A clip that holds the whole page holds the object too.
            if cb.0 <= 1. && cb.1 >= self.w - 1. && cb.2 <= 1. && cb.3 >= self.h - 1. {
                continue;
            }
            let tol = 1.0;
            if !(cb.0 <= ob.0 + tol && cb.1 >= ob.1 - tol && cb.2 <= ob.2 + tol && cb.3 >= ob.3 - tol) {
                return true;
            }
            if sub.iter().any(|s| s.segment_type() == PdfPathSegmentType::BezierTo) {
                return true;
            }
        }
        false
    }

    /// Walk one object. What maps is collected and switched off on the
    /// page (so a later render of the page shows only the patches); what
    /// doesn't stays on and is recorded as a patch.
    fn walk(&mut self, doc: &PdfDocument, obj: &mut PdfPageObject, parent: M, top: bool) {
        self.z += 1.0;
        let native = self.read(doc, obj, parent, top);
        if native {
            let _ = obj.set_inactive();
        }
    }

    /// True when the object came in natively (or is invisible).
    fn read(&mut self, doc: &PdfDocument, obj: &mut PdfPageObject, parent: M, _top: bool) -> bool {
        let pad = 0.75;
        // Transparency (soft masks, blend modes, group alpha) Zebra can't
        // draw: a picture. Plain alpha colours are fine (below).
        let effect = obj.has_transparency();
        match obj {
            PdfPageObject::Text(t) => {
                let Ok(tm) = t.matrix() else { return false };
                let m = M::from(&tm).then(&parent);
                let txt = t.text();
                let mode = t.render_mode();
                if mode == PdfPageTextRenderMode::Invisible {
                    if !txt.trim().is_empty() {
                        note(self.deck, ImportNote::InvisibleTextSkipped);
                    }
                    return true;
                }
                if txt.trim().is_empty() {
                    return true;
                }
                let font = t.font();
                let name = font.name();
                let fam = font.family();
                let Ok(q) = t.bounds() else { return false };
                let pts = quad(&q, &parent);
                if m.skewed() || !matches!(mode, PdfPageTextRenderMode::FilledUnstroked | PdfPageTextRenderMode::FilledThenStroked) {
                    self.patch(&pts, pad, ImportNote::OtherAsPicture);
                    return false;
                }
                let lname = name.to_lowercase();
                let bold = lname.contains("bold")
                    || lname.contains("black")
                    || lname.contains("heavy")
                    || font.weight().map(|w| matches!(w, PdfFontWeight::Weight700Bold | PdfFontWeight::Weight800 | PdfFontWeight::Weight900)).unwrap_or(false)
                    || font.is_bold_reenforced()
                    || mode == PdfPageTextRenderMode::FilledThenStroked;
                let italic = lname.contains("italic") || lname.contains("oblique") || font.is_italic();
                let size = t.unscaled_font_size().value as f64 * m.c.hypot(m.d);
                let (x0, x1, y0, y1) = bbox(&pts);
                let color = t.fill_color().map(|c| hex(&c)).unwrap_or_else(|_| "#000000".into());
                let rot = m.rot_deg();
                let th = rot.to_radians();
                let txt_for_math = txt.clone();
                self.runs.push(Run {
                    text: txt,
                    x0,
                    x1,
                    y0: self.h - y1,
                    y1: self.h - y0,
                    u: m.e * th.cos() + m.f * th.sin(),
                    v: -m.e * th.sin() + m.f * th.cos(),
                    size,
                    bold,
                    italic,
                    color,
                    rot,
                    symbolic: font.is_symbolic(),
                    z: self.z,
                    math: is_math(&txt_for_math, &name) || is_math("", &fam),
                });
                true
            }
            PdfPageObject::Path(p) => {
                let Ok(pm) = p.matrix() else { return false };
                let m = M::from(&pm).then(&parent);
                let fill = p.fill_mode().ok().filter(|f| *f != PdfPathFillMode::None);
                let stroked = p.is_stroked().unwrap_or(false);
                if fill.is_none() && !stroked {
                    return true;
                }
                let fillc = fill.and_then(|_| p.fill_color().ok()).map(|c| hex(&c));
                let strokec = if stroked { p.stroke_color().ok().map(|c| hex(&c)) } else { None };
                let sw = p.stroke_width().map(|w| w.value as f64).unwrap_or(1.0) * m.a.hypot(m.b);
                let clipped = self.clip_cuts(obj, &parent);
                if clipped || effect {
                    let Some(pts) = self.visible_bounds(obj, &parent) else { return true };
                    self.patch(&pts, pad + sw / 2.0, if effect { ImportNote::EffectAsPicture } else { ImportNote::ClippedAsPicture });
                    return false;
                }
                let PdfPageObject::Path(p) = obj else { unreachable!() };
                let segs = path_segs(p, &m);
                self.path_item(&segs, fillc, strokec, sw, fill == Some(PdfPathFillMode::EvenOdd));
                true
            }
            PdfPageObject::Image(im) => {
                let Ok(imm) = im.matrix() else { return false };
                let m = M::from(&imm).then(&parent);
                let c = [m.ap(0., 0.), m.ap(1., 0.), m.ap(1., 1.), m.ap(0., 1.)];
                if m.skewed() || m.det() < 0.0 || m.a < -1e-6 || m.d < -1e-6 {
                    // Mirrored, flipped or skewed: Zebra has no flip.
                    self.patch(&c, pad, ImportNote::OtherAsPicture);
                    return false;
                }
                let clipped = self.clip_cuts(obj, &parent);
                if clipped {
                    let Some(pts) = self.visible_bounds(obj, &parent) else { return true };
                    self.patch(&pts, pad, ImportNote::ClippedAsPicture);
                    return false;
                }
                let PdfPageObject::Image(im) = obj else { unreachable!() };
                let img = match im.get_processed_image(doc).or_else(|_| im.get_raw_image()) {
                    Ok(i) => i,
                    Err(_) => {
                        self.patch(&c, pad, ImportNote::OtherAsPicture);
                        return false;
                    }
                };
                let mut png = Vec::new();
                if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
                    self.patch(&c, pad, ImportNote::OtherAsPicture);
                    return false;
                }
                let Ok((key, asset)) = asset_from(png) else { return false };
                self.deck.assets.entry(key.clone()).or_insert(asset);
                let wpt = (c[1].0 - c[0].0).hypot(c[1].1 - c[0].1);
                let hpt = (c[3].0 - c[0].0).hypot(c[3].1 - c[0].1);
                let (cx, cy) = self.down(((c[0].0 + c[2].0) / 2., (c[0].1 + c[2].1) / 2.));
                let alt = String::new();
                self.items.push((self.z, Item::Picture { asset: key, cx, cy, w: wpt, h: hpt, rot: -m.rot_deg(), alt }));
                true
            }
            PdfPageObject::Shading(_) => {
                if let Some(pts) = self.visible_bounds(obj, &parent) {
                    self.patch(&pts, 0.0, ImportNote::GradientAsPicture);
                }
                false
            }
            PdfPageObject::XObjectForm(f) => {
                let Ok(fm) = f.matrix() else { return false };
                let m = M::from(&fm).then(&parent);
                if effect || self.clip_cuts(obj, &parent) {
                    // The group's look depends on all of it together.
                    let Some(pts) = self.visible_bounds(obj, &parent) else { return true };
                    self.patch(&pts, pad, if effect { ImportNote::EffectAsPicture } else { ImportNote::ClippedAsPicture });
                    return false;
                }
                let PdfPageObject::XObjectForm(f) = obj else { unreachable!() };
                self.recursed.insert(self.z as i64);
                for i in 0..f.len() {
                    if let Ok(mut child) = f.get(i) {
                        self.walk(doc, &mut child, m, false);
                    }
                }
                // Children are switched off one by one; the form stays on.
                false
            }
            PdfPageObject::Unsupported(_) => {
                if let Some(pts) = self.visible_bounds(obj, &parent) {
                    self.patch(&pts, pad, ImportNote::OtherAsPicture);
                }
                false
            }
        }
    }

    fn shape(&mut self, kind: ShapeKind, fill: &Option<String>, stroke: &Option<String>, sw: f64, (cx, cy): (f64, f64), w: f64, h: f64, rot: f64, radius: f64) {
        let data = ShapeData { shape: kind, fill: fill.clone(), stroke: stroke.clone(), stroke_width: if stroke.is_some() { sw } else { 0.0 }, corner_radius: radius, ..Default::default() };
        self.items.push((self.z, Item::Shape { data, cx, cy: self.h - cy, w, h, rot }));
    }

    fn path_item(&mut self, segs: &[Seg], fill: Option<String>, stroke: Option<String>, sw: f64, evenodd: bool) {
        let pts: Vec<(f64, f64)> = segs
            .iter()
            .flat_map(|s| match *s {
                Seg::M(x, y) | Seg::L(x, y) => vec![(x, y)],
                Seg::C(a, b, c, d, e, f) => vec![(a, b), (c, d), (e, f)],
                Seg::Z => vec![],
            })
            .collect();
        if pts.is_empty() {
            return;
        }
        let (x0, x1, y0, y1) = bbox(&pts);
        let nm = segs.iter().filter(|s| matches!(s, Seg::M(..))).count();
        let nl = segs.iter().filter(|s| matches!(s, Seg::L(..))).count();
        let nc = segs.iter().filter(|s| matches!(s, Seg::C(..))).count();
        let closed = segs.contains(&Seg::Z);
        let centre = ((x0 + x1) / 2., (y0 + y1) / 2.);
        // A straight line.
        if nm == 1 && nl == 1 && nc == 0 && fill.is_none() {
            if let (Seg::M(ax, ay), Seg::L(bx, by)) = (segs[0], segs[1]) {
                let len = (bx - ax).hypot(by - ay);
                let rot = -(by - ay).atan2(bx - ax).to_degrees();
                self.shape(ShapeKind::Line, &None, &stroke, sw, ((ax + bx) / 2., (ay + by) / 2.), len, 0.0, rot, 0.0);
                return;
            }
        }
        // A rectangle: one subpath of right angles.
        if nm == 1 && nc == 0 && (3..=5).contains(&nl) && (closed || nl >= 4) {
            let mut v: Vec<(f64, f64)> = segs.iter().filter_map(|s| match *s { Seg::M(x, y) | Seg::L(x, y) => Some((x, y)), _ => None }).collect();
            if v.len() == 5 && (v[0].0 - v[4].0).abs() < 0.01 && (v[0].1 - v[4].1).abs() < 0.01 {
                v.pop();
            }
            if v.len() == 4 && right_angles(&v) {
                let (w, h) = (dist(v[0], v[1]), dist(v[1], v[2]));
                let rot = -(v[1].1 - v[0].1).atan2(v[1].0 - v[0].0).to_degrees();
                let (w, h, rot) = norm_rect(w, h, rot);
                if rot == 0.0 {
                    self.containers.push((x0, self.h - y1, x1, self.h - y0));
                }
                self.shape(ShapeKind::Rectangle, &fill, &stroke, sw, centre, w, h, rot, 0.0);
                return;
            }
        }
        // An oval: four curves.
        if nm == 1 && nc == 4 && nl == 0 {
            self.containers.push((x0, self.h - y1, x1, self.h - y0));
            self.shape(ShapeKind::Ellipse, &fill, &stroke, sw, centre, x1 - x0, y1 - y0, 0.0, 0.0);
            return;
        }
        // A rounded rectangle: four lines and four curves (PowerPoint's).
        if nm == 1 && nc == 4 && (3..=5).contains(&nl) {
            let r = match segs[0] {
                Seg::M(sx, sy) => ((sx - x0).abs().min((x1 - sx).abs())).max((sy - y0).abs().min((y1 - sy).abs())),
                _ => 0.0,
            };
            self.containers.push((x0, self.h - y1, x1, self.h - y0));
            self.shape(ShapeKind::RoundedRect, &fill, &stroke, sw, centre, x1 - x0, y1 - y0, 0.0, r);
            return;
        }
        // Anything else: an SVG picture, one object (not node by node).
        let pad = sw.max(0.5);
        let (bx0, bx1, by0, by1) = (x0 - pad, x1 + pad, y0 - pad, y1 + pad);
        let (vw, vh) = (bx1 - bx0, by1 - by0);
        let mut d = String::new();
        use std::fmt::Write;
        for s in segs {
            let _ = match *s {
                Seg::M(x, y) => write!(d, "M{:.2} {:.2}", x - bx0, by1 - y),
                Seg::L(x, y) => write!(d, "L{:.2} {:.2}", x - bx0, by1 - y),
                Seg::C(a, b, c, e, f, g) => write!(d, "C{:.2} {:.2} {:.2} {:.2} {:.2} {:.2}", a - bx0, by1 - b, c - bx0, by1 - e, f - bx0, by1 - g),
                Seg::Z => write!(d, "Z"),
            };
        }
        let paint = |c: &str| if c.len() == 9 { format!("{}\" {{}}-opacity=\"{:.3}", &c[..7], u8::from_str_radix(&c[7..], 16).unwrap_or(255) as f64 / 255.) } else { c.to_string() };
        let fill_attr = match &fill {
            Some(c) => paint(c).replace("{}", "fill"),
            None => "none".into(),
        };
        let stroke_attr = match &stroke {
            Some(c) => format!(" stroke=\"{}\" stroke-width=\"{:.2}\" stroke-linejoin=\"round\"", paint(c).replace("{}", "stroke"), sw),
            None => String::new(),
        };
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{vw:.2}\" height=\"{vh:.2}\" viewBox=\"0 0 {vw:.2} {vh:.2}\"><path d=\"{d}\" fill=\"{fill_attr}\" fill-rule=\"{}\"{stroke_attr}/></svg>",
            if evenodd { "evenodd" } else { "nonzero" }
        );
        let Ok((key, asset)) = asset_from(svg.into_bytes()) else { return };
        self.deck.assets.entry(key.clone()).or_insert(asset);
        let (cx, cy) = self.down(((bx0 + bx1) / 2., (by0 + by1) / 2.));
        self.items.push((self.z, Item::Picture { asset: key, cx, cy, w: vw, h: vh, rot: 0.0, alt: String::new() }));
    }
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn right_angles(v: &[(f64, f64)]) -> bool {
    (0..4).all(|i| {
        let (a, b, c) = (v[i], v[(i + 1) % 4], v[(i + 2) % 4]);
        let (ux, uy, wx, wy) = (b.0 - a.0, b.1 - a.1, c.0 - b.0, c.1 - b.1);
        let n = ux.hypot(uy) * wx.hypot(wy);
        n > 0. && ((ux * wx + uy * wy) / n).abs() < 0.02
    })
}

fn norm_rect(w: f64, h: f64, rot: f64) -> (f64, f64, f64) {
    let r = ((rot % 180.) + 180.) % 180.;
    if (r - 90.).abs() < 0.5 {
        (h, w, 0.0)
    } else if !(0.5..=179.5).contains(&r) {
        (w, h, 0.0)
    } else {
        (w, h, rot)
    }
}

fn path_segs(p: &PdfPagePathObject, m: &M) -> Vec<Seg> {
    let segs = p.segments();
    let raw: Vec<_> = segs.iter().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < raw.len() {
        let s = &raw[i];
        let (x, y) = m.ap(s.x().value as f64, s.y().value as f64);
        match s.segment_type() {
            PdfPathSegmentType::MoveTo => out.push(Seg::M(x, y)),
            PdfPathSegmentType::LineTo => out.push(Seg::L(x, y)),
            PdfPathSegmentType::BezierTo if i + 2 < raw.len() => {
                let b = m.ap(raw[i + 1].x().value as f64, raw[i + 1].y().value as f64);
                let c = m.ap(raw[i + 2].x().value as f64, raw[i + 2].y().value as f64);
                out.push(Seg::C(x, y, b.0, b.1, c.0, c.1));
                if raw[i + 2].is_close() {
                    out.push(Seg::Z);
                }
                i += 3;
                continue;
            }
            _ => {}
        }
        if s.is_close() {
            out.push(Seg::Z);
        }
        i += 1;
    }
    out
}

/// Patches that touch become one picture.
fn merge(mut ps: Vec<Patch>) -> Vec<Patch> {
    let touch = |a: &Patch, b: &Patch| a.x0 <= b.x1 + 1.0 && b.x0 <= a.x1 + 1.0 && a.y0 <= b.y1 + 1.0 && b.y0 <= a.y1 + 1.0;
    let mut changed = true;
    while changed {
        changed = false;
        let mut out: Vec<Patch> = vec![];
        for p in ps {
            if let Some(q) = out.iter_mut().find(|q| touch(q, &p)) {
                q.x0 = q.x0.min(p.x0);
                q.y0 = q.y0.min(p.y0);
                q.x1 = q.x1.max(p.x1);
                q.y1 = q.y1.max(p.y1);
                q.z = q.z.min(p.z);
                for n in p.notes {
                    if !q.notes.contains(&n) {
                        q.notes.push(n);
                    }
                }
                changed = true;
            } else {
                out.push(p);
            }
        }
        ps = out;
    }
    ps
}

/// Pixels per point for patches (216 dpi), less for very big ones.
const PATCH_PX: f64 = 3.0;
const PATCH_MAX: f64 = 4096.0;

/// Render the page once with everything that came in natively switched
/// off, on a transparent background, and cut the patches out of it.
fn render_patches(page: &PdfPage, patches: &[Patch], deck: &mut ImportedDeck, items: &mut Vec<(f64, Item)>) {
    let (w, h) = (page.width().value as f64, page.height().value as f64);
    let s = PATCH_PX.min(PATCH_MAX / w.max(h));
    let (bw, bh) = ((w * s).round().max(1.0) as i32, (h * s).round().max(1.0) as i32);
    let config = PdfRenderConfig::new()
        .set_target_width(bw)
        .set_target_height(bh)
        .set_format(PdfBitmapFormat::BGRA)
        .clear_before_rendering(true)
        .set_clear_color(PdfColor::new(255, 255, 255, 0))
        .render_annotations(false)
        .render_form_data(false);
    let Ok(img) = page.render_with_config(&config).and_then(|b| b.as_image()) else {
        note(deck, ImportNote::ObjectSkipped);
        return;
    };
    let img = img.to_rgba8();
    for p in patches {
        let (px0, py0) = ((p.x0 * s).floor().max(0.0) as u32, (p.y0 * s).floor().max(0.0) as u32);
        let (px1, py1) = (((p.x1 * s).ceil() as u32).min(img.width()), ((p.y1 * s).ceil() as u32).min(img.height()));
        if px1 <= px0 || py1 <= py0 {
            continue;
        }
        let cut = image::imageops::crop_imm(&img, px0, py0, px1 - px0, py1 - py0).to_image();
        // Nothing drawn there after all (a clip hid it).
        if cut.pixels().all(|p| p.0[3] == 0) {
            continue;
        }
        let mut png = Vec::new();
        if image::DynamicImage::ImageRgba8(cut).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
            continue;
        }
        let Ok((key, asset)) = asset_from(png) else { continue };
        deck.assets.entry(key.clone()).or_insert(asset);
        let (x0, y0, x1, y1) = (px0 as f64 / s, py0 as f64 / s, px1 as f64 / s, py1 as f64 / s);
        items.push((p.z, Item::Picture { asset: key, cx: (x0 + x1) / 2.0, cy: (y0 + y1) / 2.0, w: x1 - x0, h: y1 - y0, rot: 0.0, alt: String::new() }));
    }
}

/// Switch equation text back on for the patch render: the same walk as
/// [`Page::walk`], counting objects the same way.
fn reactivate(obj: &mut PdfPageObject, z: &mut i64, wake: &std::collections::HashSet<i64>, recursed: &std::collections::HashSet<i64>) {
    *z += 1;
    if wake.contains(z) {
        let _ = obj.set_active();
    }
    if recursed.contains(z) {
        if let PdfPageObject::XObjectForm(f) = obj {
            for i in 0..f.len() {
                if let Ok(mut child) = f.get(i) {
                    reactivate(&mut child, z, wake, recursed);
                }
            }
        }
    }
}

// ----------------------------------------------------------------------
// Pages to Zebra

/// Scale the walked pages into world units and rank text sizes: the
/// deck's body text comes out at the theme's body size, scale 1.
fn finish(deck: &mut ImportedDeck, walked: Vec<Walked>, opts: &Options) {
    let blocks: Vec<&Block> = walked.iter().flat_map(|w| w.items.iter()).filter_map(|(_, it)| if let Item::Text(b) = it { Some(b) } else { None }).collect();
    let body = text::body_size(&blocks);
    let theme = &opts.theme;
    // World units per point.
    let k = (theme.body.size / body.max(1.0)).clamp(1.0, 4.0);
    for wp in walked {
        let mut page = ImportedPage { w: wp.w * k, h: wp.h * k, name: (opts.page_name)(wp.index + 1), ..Default::default() };
        for (_, it) in wp.items {
            let (element, fit) = match it {
                Item::Shape { mut data, cx, cy, w, h, rot } => {
                    data.stroke_width *= k;
                    data.corner_radius *= k;
                    (element(cx * k, cy * k, 1.0, rot, w * k, h * k, ElementKind::Shape(data)), None)
                }
                Item::Picture { asset, cx, cy, w, h, rot, alt } => {
                    (element(cx * k, cy * k, 1.0, rot, w * k, h * k, ElementKind::Image(ImageData { asset, alt, crop: None, mask: Default::default(), symbol: None })), None)
                }
                Item::Text(mut b) => {
                    let style = text::style_for(b.size, body);
                    b.data.style = style;
                    let scale = b.size * k / text::style_size(theme, style).max(1.0);
                    let fit = SourceBox { x0: b.x0 * k, y0: b.y0 * k, x1: b.x1 * k, y1: b.y1 * k };
                    let (cx, cy) = ((fit.x0 + fit.x1) / 2.0, (fit.y0 + fit.y1) / 2.0);
                    let e = element(cx, cy, scale, b.rotation, (b.x1 - b.x0) * k / scale, (b.y1 - b.y0) * k / scale, ElementKind::Text(b.data));
                    (e, Some(fit))
                }
            };
            page.items.push(ImportedItem { element, fit });
        }
        deck.pages.push(page);
    }
}

fn element(x: f64, y: f64, scale: f64, rotation: f64, w: f64, h: f64, kind: ElementKind) -> Element {
    Element {
        id: String::new(),
        transform: Transform { x, y, scale, rotation },
        size: Size { w: w.max(0.0), h: h.max(0.0) },
        locked: false,
        name: String::new(),
        placeholder: false,
        group: None,
        kind,
    }
}

/// How many of each note, in a stable order (for the summary and tests).
pub fn note_list(deck: &ImportedDeck) -> Vec<(ImportNote, usize)> {
    let m: &BTreeMap<ImportNote, usize> = &deck.notes;
    m.iter().map(|(k, v)| (*k, *v)).collect()
}

