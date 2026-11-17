//! Bundled fonts and parley text layout.
//!
//! One layout function serves measuring (editor box sizes), drawing (GPU and
//! CPU) and PDF export, so all of them agree to the pixel. Layouts are cached
//! by (text data, theme font + size, wrap width).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontStyle, FontWeight, IndentOptions, LayoutContext,
    LineHeight, PositionedLayoutItem, StyleProperty,
};
use zebra_core::{Align, TextData, Theme};

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
}

/// A finished layout: what drawing and measuring share.
#[derive(Debug, Clone)]
pub struct TextLayout {
    /// The text that was laid out (with bullets inserted).
    pub text: Arc<str>,
    pub width: f64,
    pub height: f64,
    pub runs: Vec<GlyphRunData>,
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
}

/// Font database + layout cache. Cheap to clone (shared).
#[derive(Clone)]
pub struct Fonts {
    inner: Arc<Mutex<Inner>>,
}

const CACHE_SOFT_LIMIT: usize = 4096;

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
        };
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let gen_now = g.generation;
        g.used.insert(key.clone(), gen_now);
        if let Some(l) = g.cache.get(&key) {
            return l.clone();
        }
        let l = Arc::new(do_layout(&mut g, &key, style.size as f32));
        if g.cache.len() >= CACHE_SOFT_LIMIT {
            // Drop everything not used in the current or previous generation.
            let keep = g.generation.saturating_sub(1);
            let Inner { cache, used, .. } = &mut *g;
            used.retain(|_, v| *v >= keep);
            cache.retain(|k, _| used.contains_key(k));
            g.generation += 1;
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

    /// Cached layouts (for the perf query).
    pub fn cached_layouts(&self) -> usize {
        self.inner.lock().map(|g| g.cache.len()).unwrap_or(0)
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

fn do_layout(g: &mut Inner, key: &Key, size: f32) -> TextLayout {
    const BULLET: &str = "\u{2022}\u{2002}";
    let text: String = if key.bullets {
        key.text.split('\n').map(|p| format!("{BULLET}{p}")).collect::<Vec<_>>().join("\n")
    } else {
        key.text.clone()
    };
    let Inner { font_cx, layout_cx, .. } = g;
    let mut b = layout_cx.ranged_builder(font_cx, &text, 1.0, true);
    b.push_default(StyleProperty::FontFamily(FontFamily::named(&key.family)));
    b.push_default(StyleProperty::FontSize(size));
    b.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(LINE_HEIGHT)));
    if key.bold {
        b.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
    }
    if key.italic {
        b.push_default(StyleProperty::FontStyle(FontStyle::Italic));
    }
    let mut layout = b.build(&text);

    if key.bullets {
        // Hanging indent: wrapped lines line up after the bullet.
        let mut bb = layout_cx.ranged_builder(font_cx, BULLET, 1.0, true);
        bb.push_default(StyleProperty::FontFamily(FontFamily::named(&key.family)));
        bb.push_default(StyleProperty::FontSize(size));
        if key.bold {
            bb.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
        }
        let mut bl = bb.build(BULLET);
        bl.break_all_lines(None);
        let indent = bl.full_width();
        layout.set_text_indent(indent, IndentOptions { each_line: true, hanging: true });
    }
    let alignment = match key.align {
        x if x == Align::Center as u8 => Alignment::Center,
        x if x == Align::Right as u8 => Alignment::Right,
        _ => Alignment::Left,
    };
    layout.set_alignment(alignment, AlignmentOptions::default());
    let wrap = key.wrap_bits.map(|b| f64::from_bits(b) as f32);
    layout.break_all_lines(wrap);

    let mut runs = Vec::new();
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
                    PGlyph { id: pg.id, x: pg.x, y: pg.y, advance: pg.advance, start: r.start as u32, end: r.end as u32 }
                })
                .collect();
            if glyphs.is_empty() {
                continue;
            }
            runs.push(GlyphRunData {
                font: run.font().clone(),
                font_size: run.font_size(),
                coords: run.normalized_coords().iter().map(|c| c.to_bits()).collect(),
                skew: synthesis.skew(),
                glyphs,
            });
        }
    }

    let line_h = size as f64 * LINE_HEIGHT as f64;
    let height = (layout.height() as f64).max(line_h);
    let natural = layout.width() as f64;
    let width = match wrap {
        Some(w) => w as f64,
        None => natural.max(size as f64 * 0.3),
    };
    TextLayout { text: text.into(), width, height, runs }
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
