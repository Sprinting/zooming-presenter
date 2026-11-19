//! Bundled fonts and parley text layout.
//!
//! One layout function serves measuring (editor box sizes), drawing (GPU and
//! CPU) and PDF export, so all of them agree to the pixel. Layouts are cached
//! by (text data, theme font + size, wrap width).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use parley::fontique::{
    Blob, Collection, CollectionOptions, FallbackKey, FamilyId, GenericFamily, QueryFamily, QueryFont, QueryStatus, Script,
    SourceCache,
};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontStyle, FontWeight, IndentOptions, LayoutContext,
    LineHeight, PositionedLayoutItem, StyleProperty,
};
use unicode_script::{Script as UScript, UnicodeScript};
use zebra_core::rich;
use zebra_core::{Align, ListKind, SpanFormat, TextData, Theme};

pub use parley::FontData;

/// Line height as a multiple of the font size.
pub const LINE_HEIGHT: f32 = 1.25;

struct Bundled {
    family: &'static str,
    files: &'static [&'static [u8]],
}

macro_rules! font {
    ($p:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/", $p)) as &'static [u8]
    };
}

const BUNDLED: &[Bundled] = &[
    Bundled {
        family: "Inter",
        files: &[
            font!("inter/Inter-Regular.ttf"),
            font!("inter/Inter-Bold.ttf"),
            font!("inter/Inter-Italic.ttf"),
            font!("inter/Inter-BoldItalic.ttf"),
        ],
    },
    Bundled {
        family: "Lora",
        files: &[
            font!("lora/Lora-Regular.ttf"),
            font!("lora/Lora-Bold.ttf"),
            font!("lora/Lora-Italic.ttf"),
            font!("lora/Lora-BoldItalic.ttf"),
        ],
    },
    Bundled {
        family: "JetBrains Mono",
        files: &[
            font!("jetbrains-mono/JetBrainsMono-Regular.ttf"),
            font!("jetbrains-mono/JetBrainsMono-Bold.ttf"),
            font!("jetbrains-mono/JetBrainsMono-Italic.ttf"),
            font!("jetbrains-mono/JetBrainsMono-BoldItalic.ttf"),
        ],
    },
    Bundled {
        family: "Caveat",
        files: &[font!("caveat/Caveat-Regular.ttf"), font!("caveat/Caveat-Bold.ttf")],
    },
];

/// Map a theme family ("Sans", "Serif", "Mono", "Hand") to a bundled family.
/// Any other name is passed through (a system font, if installed).
pub fn family_for(theme_font: &str) -> &str {
    match theme_font {
        "Sans" | "sans" | "sans-serif" => "Inter",
        "Serif" | "serif" => "Lora",
        "Mono" | "mono" | "monospace" => "JetBrains Mono",
        "Hand" | "hand" | "handwriting" => "Caveat",
        other => other,
    }
}

/// One positioned glyph, in layout space (font size = theme size, origin at
/// the text box's top-left corner).
#[derive(Debug, Clone, Copy)]
pub struct PGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
    /// Byte range of the source cluster in [`TextLayout::text`].
    pub start: u32,
    pub end: u32,
}

/// A run of glyphs sharing one font face and size.
#[derive(Debug, Clone)]
pub struct GlyphRunData {
    pub font: FontData,
    pub font_size: f32,
    /// Variable font coordinates (empty for static fonts).
    pub coords: Vec<i16>,
    /// Synthetic italic skew in degrees (fonts without an italic face).
    pub skew: Option<f32>,
    pub glyphs: Vec<PGlyph>,
    /// Index into [`TextLayout::formats`] when this run has its own
    /// formatting (colour, link...).
    pub format: Option<u16>,
}

/// An underline or strike-through line, in layout space.
#[derive(Debug, Clone, Copy)]
pub struct Decoration {
    pub x0: f32,
    pub x1: f32,
    /// Centre of the line.
    pub y: f32,
    pub thickness: f32,
    pub format: u16,
}

/// A clickable link area, in layout space.
#[derive(Debug, Clone)]
pub struct LinkArea {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub url: String,
}

/// A finished layout: what drawing and measuring share.
#[derive(Debug, Clone)]
pub struct TextLayout {
    /// The text that was laid out (with bullets inserted).
    pub text: Arc<str>,
    pub width: f64,
    pub height: f64,
    pub runs: Vec<GlyphRunData>,
    /// Character formats used by runs and decorations (the text's spans).
    pub formats: Vec<SpanFormat>,
    pub decorations: Vec<Decoration>,
    pub links: Vec<LinkArea>,
}

impl TextLayout {
    /// The link under a point in layout space, if any.
    pub fn link_at(&self, x: f32, y: f32) -> Option<&str> {
        self.links.iter().find(|l| x >= l.x0 && x <= l.x1 && y >= l.y0 && y <= l.y1).map(|l| l.url.as_str())
    }
}

/// A span's layout-relevant formatting, hashable.
#[derive(Clone, PartialEq, Eq, Hash)]
struct SpanKey {
    start: usize,
    end: usize,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    color: Option<String>,
    link: Option<String>,
    size_bits: Option<u64>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    text: String,
    family: String,
    size_bits: u64,
    bold: bool,
    italic: bool,
    align: u8,
    bullets: bool,
    wrap_bits: Option<u64>,
    spans: Vec<SpanKey>,
    /// (indent, list, align) per paragraph entry.
    paras: Vec<(u8, u8, u8)>,
}

struct Inner {
    /// Embedded theme fonts already registered (asset keys).
    registered: std::collections::HashSet<String>,
    font_cx: FontContext,
    layout_cx: LayoutContext<[u8; 4]>,
    cache: HashMap<Key, Arc<TextLayout>>,
    /// Keys touched since the last trim, to evict layouts no longer drawn.
    generation: u64,
    used: HashMap<Key, u64>,
    /// The generation the cache was last trimmed in.
    swept: Option<u64>,
    /// Layouts computed (cache misses), for tests.
    misses: u64,
    /// Per (character, preferred script): the family to fall back to when
    /// the text's own fonts lack it. None = no font has it.
    fallbacks: HashMap<(char, Option<UScript>), Option<String>>,
}

/// Font database + layout cache. Cheap to clone (shared).
#[derive(Clone)]
pub struct Fonts {
    inner: Arc<Mutex<Inner>>,
}

const CACHE_SOFT_LIMIT: usize = 4096;
/// With no frames to count (headless measuring), the cache is cut back
/// once it reaches this.
const CACHE_HARD_LIMIT: usize = 4 * CACHE_SOFT_LIMIT;

impl Fonts {
    pub fn new() -> Self {
        // System fonts stay available as a fallback for scripts and symbols the
        // bundled families lack; the bundled families always win by name.
        let mut collection = Collection::new(CollectionOptions { shared: false, system_fonts: true });
        for b in BUNDLED {
            for data in b.files {
                let blob: Blob<u8> = Blob::new(Arc::new(*data));
                collection.register_fonts(blob, None);
            }
        }
        let font_cx = FontContext { collection, source_cache: SourceCache::default() };
        Fonts {
            inner: Arc::new(Mutex::new(Inner {
                registered: Default::default(),
                font_cx,
                layout_cx: LayoutContext::new(),
                cache: HashMap::new(),
                generation: 0,
                used: HashMap::new(),
                swept: None,
                misses: 0,
                fallbacks: HashMap::new(),
            })),
        }
    }

    pub fn measurer(&self) -> Box<dyn zebra_core::TextMeasure> {
        Box::new(ParleyMeasure { fonts: self.clone() })
    }

    /// Lay out `t` with the theme's style. `wrap` None = no wrapping.
    pub fn layout(&self, t: &TextData, theme: &Theme, wrap: Option<f64>) -> Arc<TextLayout> {
        let style = theme.text_style(t.style);
        let key = Key {
            text: t.text.clone(),
            family: family_for(&style.font).to_string(),
            size_bits: style.size.to_bits(),
            bold: t.bold,
            italic: t.italic,
            align: t.align as u8,
            bullets: t.bullets,
            wrap_bits: wrap.filter(|w| w.is_finite() && *w > 0.0).map(|w| w.to_bits()),
            spans: t
                .spans
                .iter()
                .map(|s| SpanKey {
                    start: s.start,
                    end: s.end,
                    bold: s.format.bold,
                    italic: s.format.italic,
                    underline: s.format.underline,
                    strike: s.format.strike,
                    color: s.format.color.clone(),
                    link: s.format.link.clone(),
                    size_bits: s.format.size.map(f64::to_bits),
                })
                .collect(),
            paras: t
                .paras
                .iter()
                .map(|p| (p.indent, p.list.map(|l| l as u8 + 1).unwrap_or(0), p.align.map(|a| a as u8 + 1).unwrap_or(0)))
                .collect(),
        };
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let gen_now = g.generation;
        g.used.insert(key.clone(), gen_now);
        if let Some(l) = g.cache.get(&key) {
            return l.clone();
        }
        let l = Arc::new(do_layout(&mut g, &key, style.size as f32));
        g.misses += 1;
        if g.cache.len() >= CACHE_HARD_LIMIT {
            // Nobody is marking frames: start a new generation ourselves.
            g.generation += 1;
        }
        if g.cache.len() >= CACHE_SOFT_LIMIT && g.swept != Some(g.generation) {
            // Once per generation (frame), drop everything not used in it
            // or the one before. A deck that draws more texts than the
            // soft limit keeps them all: trimming on every miss, and
            // counting each trim as a frame, threw out layouts the same
            // frame was about to ask for again.
            let keep = g.generation.saturating_sub(1);
            let Inner { cache, used, .. } = &mut *g;
            used.retain(|_, v| *v >= keep);
            cache.retain(|k, _| used.contains_key(k));
            g.swept = Some(g.generation);
        }
        g.cache.insert(key, l.clone());
        l
    }

    /// (family, faces registered under that name, files bundled): for tests.
    pub(crate) fn bundled_report(&self) -> Vec<(&'static str, usize, usize)> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        BUNDLED
            .iter()
            .map(|b| {
                let n = g.font_cx.collection.family_by_name(b.family).map(|f| f.fonts().len()).unwrap_or(0);
                (b.family, n, b.files.len())
            })
            .collect()
    }

    /// Register the fonts a document's theme embeds, once each, so text
    /// set in them renders on a machine where they aren't installed.
    pub fn register_doc_fonts(&self, doc: &zebra_core::Document) {
        if doc.themes.iter().all(|t| t.fonts.is_empty()) {
            return;
        }
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut added = false;
        for key in doc.themes.iter().flat_map(|t| t.fonts.values().flatten()) {
            if g.registered.contains(key) {
                continue;
            }
            if let Some(a) = doc.assets.get(key) {
                // Shares the (possibly memory-mapped) bytes; no copy.
                let blob: Blob<u8> = Blob::new(Arc::new(a.bytes.clone()));
                g.font_cx.collection.register_fonts(blob, None);
                added = true;
            }
            g.registered.insert(key.clone());
        }
        if added {
            // Layouts made with a fallback font are stale now.
            g.cache.clear();
            g.used.clear();
            g.fallbacks.clear();
        }
    }

    /// Installed font families (system and bundled), sorted, without
    /// hidden system families (names starting with '.').
    pub fn families(&self) -> Vec<String> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<String> = g.font_cx.collection.family_names().filter(|n| !n.starts_with('.')).map(|n| n.to_string()).collect();
        v.sort_by_key(|a| a.to_lowercase());
        v.dedup();
        v
    }

    /// Files of an installed (system) family: (bytes, extension). Bundled
    /// families return nothing; they ship with the app.
    pub fn family_files(&self, family: &str) -> Vec<(Vec<u8>, String)> {
        use parley::fontique::SourceKind;
        if BUNDLED.iter().any(|b| b.family == family) || family_for(family) != family {
            return vec![];
        }
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(fam) = g.font_cx.collection.family_by_name(family) else { return vec![] };
        let mut paths = vec![];
        for f in fam.fonts() {
            if let SourceKind::Path(p) = &f.source().kind {
                let p = p.to_path_buf();
                if !paths.contains(&p) {
                    paths.push(p);
                }
            }
        }
        paths
            .into_iter()
            .filter_map(|p| {
                let ext = p.extension()?.to_string_lossy().to_lowercase();
                let ext = if matches!(ext.as_str(), "ttf" | "otf" | "ttc") { ext } else { "ttf".into() };
                Some((std::fs::read(&p).ok()?, ext))
            })
            .collect()
    }

    /// Whether any installed or bundled font has a glyph for `c` (tests
    /// use it to skip characters this machine can't show at all).
    #[doc(hidden)]
    pub fn has_glyph_somewhere(&self, c: char) -> bool {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Inner { font_cx, fallbacks, .. } = &mut *g;
        fallback_family(font_cx, fallbacks, c, &[]).is_some()
    }

    /// Cached layouts (for the perf query).
    pub fn cached_layouts(&self) -> usize {
        self.inner.lock().map(|g| g.cache.len()).unwrap_or(0)
    }

    /// Layouts computed so far (cache misses).
    #[doc(hidden)]
    pub fn layout_misses(&self) -> u64 {
        self.inner.lock().map(|g| g.misses).unwrap_or(0)
    }

    /// Mark a frame boundary for cache eviction.
    pub fn next_generation(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.generation += 1;
        }
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

/// Rebuild the `TextData` a key was made from (layout only needs these).
fn data_of(key: &Key) -> TextData {
    let align = |a: u8| match a {
        x if x == Align::Center as u8 => Align::Center,
        x if x == Align::Right as u8 => Align::Right,
        _ => Align::Left,
    };
    TextData {
        text: key.text.clone(),
        bold: key.bold,
        italic: key.italic,
        align: align(key.align),
        bullets: key.bullets,
        spans: key
            .spans
            .iter()
            .map(|s| zebra_core::TextSpan {
                start: s.start,
                end: s.end,
                format: SpanFormat {
                    bold: s.bold,
                    italic: s.italic,
                    underline: s.underline,
                    strike: s.strike,
                    color: s.color.clone(),
                    link: s.link.clone(),
                    size: s.size_bits.map(f64::from_bits),
                },
            })
            .collect(),
        paras: key
            .paras
            .iter()
            .map(|&(indent, list, al)| zebra_core::ParaFormat {
                indent,
                list: match list {
                    1 => Some(ListKind::None),
                    2 => Some(ListKind::Bullet),
                    3 => Some(ListKind::Number),
                    _ => None,
                },
                align: (al > 0).then(|| align(al - 1)),
            })
            .collect(),
        ..Default::default()
    }
}

fn real_script(s: UScript) -> bool {
    !matches!(s, UScript::Common | UScript::Inherited | UScript::Unknown)
}

/// A family with a glyph for `c`, for when the text's own fonts and the
/// fallbacks of the script parley gave its run both lack it.
///
/// parley picks fallback fonts by the script of the run, and a character
/// with no script of its own (Common, e.g. the danda "।", or Inherited, a
/// combining mark) takes the script of the text before it. So "Hello।"
/// only tried Latin fallbacks and drew the danda as .notdef. Here the
/// character's own Script_Extensions decide (the danda: Devanagari,
/// Bengali...), preferring a script in `prefer` (the paragraph's), then
/// emoji and the generic families, then every installed family.
/// Cached per character, misses too.
fn fallback_family(
    font_cx: &mut FontContext,
    cache: &mut HashMap<(char, Option<UScript>), Option<String>>,
    c: char,
    prefer: &[UScript],
) -> Option<String> {
    let mut scripts: Vec<UScript> = c
        .script_extension()
        .iter()
        .filter(|s| real_script(*s))
        .collect();
    let preferred = prefer.iter().copied().find(|s| scripts.contains(s));
    if let Some(f) = cache.get(&(c, preferred)) {
        return f.clone();
    }
    if let Some(p) = preferred {
        scripts.retain(|s| *s != p);
        scripts.insert(0, p);
    }
    let FontContext {
        collection,
        source_cache,
    } = font_cx;
    let generic = [
        GenericFamily::Emoji,
        GenericFamily::SansSerif,
        GenericFamily::SystemUi,
        GenericFamily::Serif,
        GenericFamily::Monospace,
        GenericFamily::Math,
    ];
    let mut id = scripts
        .iter()
        .find_map(|s| covering(collection, source_cache, c, vec![], Some(*s)));
    if id.is_none() {
        id = covering(
            collection,
            source_cache,
            c,
            generic.iter().map(|g| QueryFamily::Generic(*g)).collect(),
            None,
        );
    }
    if id.is_none() {
        let all: Vec<QueryFamily<'static>> = collection.family_ids().map(QueryFamily::Id).collect();
        id = covering(collection, source_cache, c, all, None);
    }
    let name = id.and_then(|id| collection.family_name(id).map(str::to_string));
    cache.insert((c, preferred), name.clone());
    name
}

/// The first family in `families` (then the fallbacks of `script`) whose
/// font has a glyph for `c`.
fn covering(
    collection: &mut Collection,
    source_cache: &mut SourceCache,
    c: char,
    families: Vec<QueryFamily<'static>>,
    script: Option<UScript>,
) -> Option<FamilyId> {
    let mut q = collection.query(source_cache);
    q.set_families(families);
    if let Some(s) = script {
        let b = s.short_name().as_bytes();
        q.set_fallbacks(FallbackKey::new(
            Script::from_bytes([b[0], b[1], b[2], b[3]]),
            None,
        ));
    }
    let mut found: Option<FamilyId> = None;
    q.matches_with(|f: &QueryFont| {
        if f.charmap().and_then(|m| m.map(c)).is_some_and(|g| g != 0) {
            found = Some(f.family.0);
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    found
}

const BULLETS: [&str; 3] = ["\u{2022}", "\u{25E6}", "\u{25AA}"];

/// Lays out each paragraph on its own (so each can have its own indent,
/// list marker and alignment) and stacks them.
fn do_layout(g: &mut Inner, key: &Key, size: f32) -> TextLayout {
    let t = data_of(key);
    let Inner { font_cx, layout_cx, .. } = g;
    let wrap = key.wrap_bits.map(|b| f64::from_bits(b) as f32);
    let paras: Vec<&str> = t.text.split('\n').collect();
    let formats: Vec<SpanFormat> = t.spans.iter().map(|s| s.format.clone()).collect();

    // Paragraph text with its list marker, and where the text starts.
    struct P {
        text: String,
        marker_len: usize,
        indent_x: f32,
        align: Align,
        char0: usize,
    }
    let mut ps = vec![];
    let mut numbers = [0usize; 9];
    let mut char0 = 0;
    for (i, p) in paras.iter().enumerate() {
        let (list, indent, align) = rich::para(&t, i);
        let lvl = indent.min(8) as usize;
        for n in numbers.iter_mut().skip(lvl + 1) {
            *n = 0;
        }
        let marker = match list {
            ListKind::Bullet => {
                numbers[lvl] = 0;
                format!("{}\u{2002}", BULLETS[lvl % 3])
            }
            ListKind::Number => {
                numbers[lvl] += 1;
                format!("{}.\u{2002}", numbers[lvl])
            }
            ListKind::None => {
                numbers[lvl] = 0;
                String::new()
            }
        };
        ps.push(P { marker_len: marker.len(), text: format!("{marker}{p}"), indent_x: indent as f32 * size * 1.25, align, char0 });
        char0 += p.chars().count() + 1;
    }

    let build = |font_cx: &mut FontContext, layout_cx: &mut LayoutContext<[u8; 4]>, p: &P| {
        let mut b = layout_cx.ranged_builder(font_cx, &p.text, 1.0, true);
        b.push_default(StyleProperty::FontFamily(FontFamily::named(&key.family)));
        b.push_default(StyleProperty::FontSize(size));
        b.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(LINE_HEIGHT)));
        if key.bold {
            b.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
        }
        if key.italic {
            b.push_default(StyleProperty::FontStyle(FontStyle::Italic));
        }
        let pchars = p.text[p.marker_len..].chars().count();
        for (si, s) in t.spans.iter().enumerate() {
            let (a, z) = (s.start.max(p.char0), s.end.min(p.char0 + pchars));
            if a >= z {
                continue;
            }
            let body = &p.text[p.marker_len..];
            let r = p.marker_len + rich::byte_at(body, a - p.char0)..p.marker_len + rich::byte_at(body, z - p.char0);
            let f = &s.format;
            if let Some(v) = f.bold {
                b.push(StyleProperty::FontWeight(if v { FontWeight::BOLD } else { FontWeight::NORMAL }), r.clone());
            }
            if let Some(v) = f.italic {
                b.push(StyleProperty::FontStyle(if v { FontStyle::Italic } else { FontStyle::Normal }), r.clone());
            }
            if let Some(k) = f.size {
                b.push(StyleProperty::FontSize(size * k as f32), r.clone());
            }
            let n = (si + 1) as u16;
            b.push(StyleProperty::Brush([n as u8, (n >> 8) as u8, 0, 0]), r);
        }
        let mut layout = b.build(&p.text);
        if p.marker_len > 0 {
            // Hanging indent: wrapped lines line up after the marker.
            let marker = &p.text[..p.marker_len];
            let mut bb = layout_cx.ranged_builder(font_cx, marker, 1.0, true);
            bb.push_default(StyleProperty::FontFamily(FontFamily::named(&key.family)));
            bb.push_default(StyleProperty::FontSize(size));
            if key.bold {
                bb.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
            }
            let mut bl = bb.build(marker);
            bl.break_all_lines(None);
            layout.set_text_indent(bl.full_width(), IndentOptions { each_line: true, hanging: true });
        }
        layout
    };
    let alignment = |a: Align| match a {
        Align::Center => Alignment::Center,
        Align::Right => Alignment::Right,
        Align::Left => Alignment::Left,
    };

    let mut layouts = vec![];
    for p in &ps {
        let mut l = build(font_cx, layout_cx, p);
        l.set_alignment(alignment(p.align), AlignmentOptions::default());
        l.break_all_lines(wrap.map(|w| (w - p.indent_x).max(size)));
        layouts.push(l);
    }
    // Without wrapping, align paragraphs within the widest one.
    let natural = ps.iter().zip(&layouts).map(|(p, l)| p.indent_x + l.width()).fold(0.0f32, f32::max);
    if wrap.is_none() {
        for (p, l) in ps.iter().zip(layouts.iter_mut()) {
            if p.align != Align::Left {
                l.break_all_lines(Some((natural - p.indent_x).max(1.0)));
            }
        }
    }

    let line_h = size * LINE_HEIGHT;
    let mut runs = Vec::new();
    let mut decorations = Vec::new();
    let mut links = Vec::new();
    let mut y0 = 0.0f32;
    let mut byte0 = 0usize;
    for (p, layout) in ps.iter().zip(&layouts) {
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(gr) = item else { continue };
                let run = gr.run();
                let synthesis = run.synthesis();
                let ranges: Vec<Range<usize>> = run
                    .visual_clusters()
                    .flat_map(|c| {
                        let r = c.text_range();
                        std::iter::repeat_n(r, c.glyphs().count())
                    })
                    .collect();
                let whole = run.text_range();
                let glyphs: Vec<PGlyph> = gr
                    .positioned_glyphs()
                    .enumerate()
                    .map(|(i, pg)| {
                        let r = ranges.get(i).cloned().unwrap_or_else(|| whole.clone());
                        PGlyph {
                            id: pg.id,
                            x: pg.x + p.indent_x,
                            y: pg.y + y0,
                            advance: pg.advance,
                            start: (r.start + byte0) as u32,
                            end: (r.end + byte0) as u32,
                        }
                    })
                    .collect();
                if glyphs.is_empty() {
                    continue;
                }
                let brush = gr.style().brush;
                let fi = u16::from(brush[0]) | (u16::from(brush[1]) << 8);
                let format = (fi > 0).then(|| fi - 1);
                if let Some(f) = format.and_then(|i| formats.get(i as usize).map(|f| (i, f))) {
                    let (i, f) = f;
                    let fs = run.font_size();
                    let (x0, x1) = (gr.offset() + p.indent_x, gr.offset() + gr.advance() + p.indent_x);
                    let base = gr.baseline() + y0;
                    if f.underline == Some(true) || (f.link.is_some() && f.underline != Some(false)) {
                        decorations.push(Decoration { x0, x1, y: base + fs * 0.11, thickness: (fs * 0.06).max(0.5), format: i });
                    }
                    if f.strike == Some(true) {
                        decorations.push(Decoration { x0, x1, y: base - fs * 0.28, thickness: (fs * 0.06).max(0.5), format: i });
                    }
                    if let Some(url) = &f.link {
                        links.push(LinkArea { x0, x1, y0: base - fs * 0.95, y1: base + fs * 0.3, url: url.clone() });
                    }
                }
                runs.push(GlyphRunData {
                    font: run.font().clone(),
                    font_size: run.font_size(),
                    coords: run.normalized_coords().iter().map(|c| c.to_bits()).collect(),
                    skew: synthesis.skew(),
                    glyphs,
                    format,
                });
            }
        }
        y0 += layout.height().max(line_h);
        byte0 += p.text.len() + 1;
    }

    let height = (y0 as f64).max(line_h as f64);
    let width = match wrap {
        Some(w) => w as f64,
        None => (natural as f64).max(size as f64 * 0.3),
    };
    let text: String = ps.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n");
    TextLayout { text: text.into(), width, height, runs, formats, decorations, links }
}

/// The measurer installed in `Editor::measure`.
struct ParleyMeasure {
    fonts: Fonts,
}

impl zebra_core::TextMeasure for ParleyMeasure {
    fn measure(&self, text: &TextData, theme: &Theme, wrap_width: Option<f64>) -> (f64, f64) {
        let l = self.fonts.layout(text, theme, wrap_width);
        (l.width, l.height)
    }
    fn prepare(&self, doc: &zebra_core::Document) {
        self.fonts.register_doc_fonts(doc);
    }
    fn font_files(&self, family: &str) -> Vec<(Vec<u8>, String)> {
        self.fonts.family_files(family)
    }
    fn font_families(&self) -> Vec<String> {
        self.fonts.families()
    }
}

/// The wrap width a text element is laid out with when drawn.
pub fn wrap_for(t: &TextData, size_w: f64) -> Option<f64> {
    if t.fixed_width { Some(size_w) } else { None }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    /// W8: above the soft limit every miss counted as a frame and threw
    /// out nearly the whole cache, so each text was laid out again on
    /// every frame.
    #[test]
    fn a_deck_with_more_texts_than_the_soft_limit_is_laid_out_once() {
        let fonts = Fonts::new();
        let theme = Theme::by_id("classic").unwrap();
        let n = CACHE_SOFT_LIMIT + 300;
        let texts: Vec<TextData> = (0..n).map(|i| TextData { text: format!("t{i}"), ..Default::default() }).collect();
        let frame = |fonts: &Fonts| {
            fonts.next_generation();
            for t in &texts {
                fonts.layout(t, &theme, None);
            }
        };
        frame(&fonts);
        assert_eq!(fonts.layout_misses(), n as u64);
        frame(&fonts);
        frame(&fonts);
        assert_eq!(fonts.layout_misses(), n as u64, "frames two and three lay nothing out again");
        assert_eq!(fonts.cached_layouts(), n);
        // Texts no longer drawn do go, once the cache is over the limit.
        for g in 0..3 {
            fonts.next_generation();
            for i in 0..CACHE_SOFT_LIMIT + 10 {
                fonts.layout(&TextData { text: format!("other{g}-{i}"), ..Default::default() }, &theme, None);
            }
        }
        assert!(fonts.cached_layouts() < 3 * CACHE_SOFT_LIMIT, "{}", fonts.cached_layouts());
        // And with nobody marking frames (headless measuring) it stays bounded.
        for i in 0..3 * CACHE_HARD_LIMIT {
            fonts.layout(&TextData { text: format!("m{i}"), ..Default::default() }, &theme, None);
        }
        assert!(fonts.cached_layouts() <= 2 * CACHE_HARD_LIMIT, "{}", fonts.cached_layouts());
    }
}
