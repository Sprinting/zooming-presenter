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
    FontFamilyName, LineHeight, PositionedLayoutItem, StyleProperty,
};
use unicode_script::{Script as UScript, UnicodeScript};
use zebra_core::rich;
use zebra_core::{Align, ListKind, SpanFormat, TextData, Theme};

pub use parley::FontData;

/// Line height as a multiple of the font size.
pub const LINE_HEIGHT: f32 = 1.25;

#[allow(dead_code)]
struct Bundled {
    family: &'static str,
    files: &'static [&'static [u8]],
}

#[cfg(not(target_arch = "wasm32"))]
macro_rules! font {
    ($p:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/", $p)) as &'static [u8]
    };
}

/// The web player (wasm32) bundles no fonts: an exported page carries the
/// files its deck uses and registers them with [`Fonts::add_font`].
#[cfg(target_arch = "wasm32")]
const BUNDLED: &[Bundled] = &[];

/// Inter, Lora and JetBrains Mono are variable fonts (a weight axis), so
/// a style can be any weight from Thin or Regular to Black or Bold; Caveat
/// has a Regular and a Bold face.
#[cfg(not(target_arch = "wasm32"))]
const BUNDLED: &[Bundled] = &[
    Bundled {
        family: "Inter",
        files: &[font!("inter/InterVariable.ttf"), font!("inter/InterVariable-Italic.ttf")],
    },
    Bundled {
        family: "Lora",
        files: &[font!("lora/Lora-Variable.ttf"), font!("lora/Lora-Italic-Variable.ttf")],
    },
    Bundled {
        family: "JetBrains Mono",
        files: &[
            font!("jetbrains-mono/JetBrainsMono-Variable.ttf"),
            font!("jetbrains-mono/JetBrainsMono-Italic-Variable.ttf"),
        ],
    },
    Bundled {
        family: "Caveat",
        files: &[font!("caveat/Caveat-Regular.ttf"), font!("caveat/Caveat-Bold.ttf")],
    },
];

/// Names the bundled fonts' files give their family, where that isn't
/// the family Zebra knows them by ("Inter Variable" is Inter). A page's
/// fonts (cut from these files) register under the Zebra name too.
const FAMILY_ALIASES: &[(&str, &str)] = &[("Inter Variable", "Inter")];

/// The family name a font file gives itself (typographic, else legacy).
fn family_name_of(bytes: &[u8]) -> Option<String> {
    use skrifa::{string::StringId, MetadataProvider};
    let f = skrifa::FontRef::from_index(bytes, 0).ok()?;
    [StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME]
        .into_iter()
        .find_map(|id| f.localized_strings(id).english_or_first().map(|s| s.chars().collect::<String>()))
}

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
    /// The same location as axis settings in user units (e.g. `wght`
    /// 600): what a PDF cuts the font to.
    pub variations: Vec<([u8; 4], f32)>,
    /// Synthetic italic skew in degrees (fonts without an italic face).
    pub skew: Option<f32>,
    pub glyphs: Vec<PGlyph>,
    /// Index into [`TextLayout::formats`] when this run has its own
    /// formatting (colour, link...).
    pub format: Option<u32>,
}

/// An underline or strike-through line, in layout space.
#[derive(Debug, Clone, Copy)]
pub struct Decoration {
    pub x0: f32,
    pub x1: f32,
    /// Centre of the line.
    pub y: f32,
    pub thickness: f32,
    pub format: u32,
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
    /// The style's weight.
    weight: u16,
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
        Self::with(true)
    }

    /// No fonts at all until [`Fonts::add_font`]: what the web player
    /// starts with (tests of exported pages).
    #[doc(hidden)]
    pub fn bare() -> Self {
        Self::with(false)
    }

    fn with(bundled: bool) -> Self {
        // System fonts stay available as a fallback for scripts and symbols the
        // bundled families lack; the bundled families always win by name.
        let mut collection = Collection::new(CollectionOptions { shared: false, system_fonts: bundled });
        for b in BUNDLED.iter().filter(|_| bundled) {
            for data in b.files {
                let blob: Blob<u8> = Blob::new(Arc::new(*data));
                let named = parley::fontique::FontInfoOverride { family_name: Some(b.family), ..Default::default() };
                collection.register_fonts(blob, Some(named));
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

    /// Register a font file (TTF, OTF or a collection). Layouts made
    /// before it was added are dropped, as with a theme's embedded fonts.
    pub fn add_font(&self, bytes: Vec<u8>) {
        let alias = family_name_of(&bytes).and_then(|n| FAMILY_ALIASES.iter().find(|(a, _)| *a == n).map(|(_, z)| *z));
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let blob: Blob<u8> = Blob::new(Arc::new(bytes));
        let named = alias.map(|a| parley::fontique::FontInfoOverride { family_name: Some(a), ..Default::default() });
        g.font_cx.collection.register_fonts(blob, named);
        g.cache.clear();
        g.used.clear();
        g.fallbacks.clear();
    }

    /// The font files `doc`'s text is set in (bundled, embedded or system
    /// fallbacks), each once: what a page showing the deck must carry.
    pub fn files_used(&self, editor: &zebra_core::Editor) -> Vec<Blob<u8>> {
        let mut out: Vec<Blob<u8>> = vec![];
        for (font, _) in self.faces_used(editor) {
            if !out.iter().any(|b| b.id() == font.data.id()) {
                out.push(font.data.clone());
            }
        }
        out
    }

    /// True for the fonts that ship inside Zebra (open licences: a page
    /// may carry them whole).
    pub fn is_bundled(font: &FontData) -> bool {
        let p = font.data.as_ref().as_ptr();
        BUNDLED.iter().flat_map(|b| b.files.iter()).any(|f| f.as_ptr() == p)
    }

    /// The font faces `doc`'s text is set in, each with the glyphs it
    /// shows (for cutting fonts down to what a page needs).
    pub fn faces_used(&self, editor: &zebra_core::Editor) -> Vec<(FontData, std::collections::BTreeSet<u16>)> {
        let mut out: Vec<(FontData, std::collections::BTreeSet<u16>)> = vec![];
        for (_, faces) in self.faces_by_element(editor) {
            for (font, glyphs) in faces {
                match out.iter_mut().find(|(f, _)| f.data.id() == font.data.id() && f.index == font.index) {
                    Some((_, g)) => g.extend(glyphs),
                    None => out.push((font, glyphs)),
                }
            }
        }
        out
    }

    /// Per text element or connector label: the faces it is set in and the
    /// glyphs of each.
    pub fn faces_by_element(&self, editor: &zebra_core::Editor) -> Vec<(zebra_core::ElementId, Vec<(FontData, std::collections::BTreeSet<u16>)>)> {
        use zebra_core::ElementKind;
        self.register_doc_fonts(&editor.doc);
        let theme = editor.theme();
        let faces = |l: &TextLayout| {
            let mut out: Vec<(FontData, std::collections::BTreeSet<u16>)> = vec![];
            for run in &l.runs {
                let glyphs = run.glyphs.iter().filter_map(|g| u16::try_from(g.id).ok());
                match out.iter_mut().find(|(f, _)| f.data.id() == run.font.data.id() && f.index == run.font.index) {
                    Some((_, g)) => g.extend(glyphs),
                    None => out.push((run.font.clone(), glyphs.collect())),
                }
            }
            out
        };
        let mut out = vec![];
        for id in &editor.doc.z_order {
            let Some(e) = editor.doc.elements.get(id) else { continue };
            match &e.kind {
                ElementKind::Text(t) if !t.text.is_empty() => out.push((e.id.clone(), faces(&self.layout(t, &theme, wrap_for(t, e.size.w))))),
                ElementKind::Connector(c) if !c.label.is_empty() => {
                    let t = TextData { text: c.label.clone(), style: zebra_core::TextStyle::Body, ..Default::default() };
                    out.push((e.id.clone(), faces(&self.layout(&t, &theme, None))));
                }
                _ => {}
            }
        }
        out
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
            weight: style.weight,
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
        // parley panics on what it can't hold (more than 65,535 styles in a
        // paragraph, audit of 20 Nov C5), and this runs on the UI thread:
        // a text it can't lay out is laid out plain, else left empty.
        let size = style.size as f32;
        let l = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| do_layout(&mut g, &key, size)))
            .or_else(|_| {
                let plain = Key { spans: vec![], ..key.clone() };
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| do_layout(&mut g, &plain, size)))
            })
            .unwrap_or_else(|_| TextLayout {
                text: Arc::from(""),
                width: 0.0,
                height: 0.0,
                runs: vec![],
                formats: vec![],
                decorations: vec![],
                links: vec![],
            });
        let l = Arc::new(l);
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

    /// The upright faces of a family, lightest first, one per weight (see
    /// [`zebra_core::TextMeasure::font_faces`]). A variable font lists its
    /// named instances; Zebra's own fonts list the usual weights their
    /// weight axis covers, without names (the app names them).
    pub fn faces(&self, doc: Option<&zebra_core::Document>, family: &str) -> Vec<zebra_core::FontFace> {
        use parley::fontique::{FontStyle as FStyle, FontWidth};
        use skrifa::MetadataProvider;
        use zebra_core::FontFace;
        if let Some(d) = doc {
            self.register_doc_fonts(d);
        }
        let fam = family_for(family);
        let ours = BUNDLED.iter().any(|b| b.family == fam);
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let FontContext { collection, source_cache } = &mut g.font_cx;
        let Some(info) = collection.family_by_name(fam) else { return vec![] };
        let all = info.fonts();
        let upright: Vec<_> = all.iter().filter(|f| f.style() == FStyle::Normal).collect();
        let upright = if upright.is_empty() { all.iter().collect() } else { upright };
        let normal: Vec<_> = upright.iter().copied().filter(|f| f.width() == FontWidth::NORMAL).collect();
        let faces = if normal.is_empty() { upright } else { normal };
        let mut out: Vec<FontFace> = vec![];
        for f in faces {
            let name_of = |blob: &Blob<u8>, id: skrifa::string::StringId| -> Option<String> {
                let font = skrifa::FontRef::from_index(blob.as_ref(), f.index()).ok()?;
                font.localized_strings(id).english_or_first().map(|s| s.chars().collect::<String>())
            };
            match f.axes().iter().find(|a| a.tag.to_be_bytes() == *b"wght") {
                Some(axis) => {
                    let mut named = vec![];
                    if !ours && let Some(blob) = f.load(Some(source_cache)) {
                        if let Ok(font) = skrifa::FontRef::from_index(blob.as_ref(), f.index()) {
                            let axes = font.axes();
                            let wi = axes.iter().position(|a| a.tag().to_be_bytes() == *b"wght");
                            for inst in font.named_instances().iter() {
                                let Some(w) = wi.and_then(|i| inst.user_coords().nth(i)) else { continue };
                                // Upright, default-width instances only.
                                let other_axes_default = inst.user_coords().zip(axes.iter()).enumerate().all(|(i, (v, a))| {
                                    Some(i) == wi || !matches!(&a.tag().to_be_bytes(), b"ital" | b"slnt" | b"wdth") || v == a.default_value()
                                });
                                if !other_axes_default {
                                    continue;
                                }
                                named.push(FontFace { weight: w.round().clamp(1.0, 1000.0) as u16, name: name_of(&blob, inst.subfamily_name_id()) });
                            }
                        }
                    }
                    if named.is_empty() {
                        for (w, _) in zebra_core::theme::WEIGHT_NAMES {
                            if axis.min <= w as f32 && w as f32 <= axis.max {
                                named.push(FontFace { weight: w, name: None });
                            }
                        }
                    }
                    out.extend(named);
                }
                None => {
                    let name = if ours {
                        None
                    } else {
                        f.load(Some(source_cache)).and_then(|blob| {
                            name_of(&blob, skrifa::string::StringId::TYPOGRAPHIC_SUBFAMILY_NAME).or_else(|| name_of(&blob, skrifa::string::StringId::SUBFAMILY_NAME))
                        })
                    };
                    out.push(FontFace { weight: f.weight().value().round().clamp(1.0, 1000.0) as u16, name });
                }
            }
        }
        out.sort_by_key(|f| f.weight);
        out.dedup_by_key(|f| f.weight);
        out
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

/// Spaces, controls, joiners, variation selectors and tags: drawn with no
/// glyph of their own, so a font lacking them is no reason to fall back.
fn needs_no_glyph(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(c,
            '\u{00AD}' | '\u{034F}' | '\u{FEFF}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2060}'..='\u{2064}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{E0000}'..='\u{E0FFF}')
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

/// Clusters a layout drew with .notdef (glyph 0), with the families that
/// have their characters: what to add to those ranges' family lists.
fn tofu_fixes(
    layout: &parley::Layout<[u8; 4]>,
    text: &str,
    family: &str,
    font_cx: &mut FontContext,
    cache: &mut HashMap<(char, Option<UScript>), Option<String>>,
) -> Vec<(Range<usize>, Vec<String>)> {
    let mut bad: Vec<Range<usize>> = vec![];
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(gr) = item else {
                continue;
            };
            for c in gr.run().visual_clusters() {
                if c.glyphs().any(|g| g.id == 0) {
                    bad.push(c.text_range());
                }
            }
        }
    }
    if bad.is_empty() {
        return vec![];
    }
    bad.sort_by_key(|r| (r.start, r.end));
    bad.dedup();
    let mut prefer: Vec<UScript> = text
        .chars()
        .map(|c| c.script())
        .filter(|s| real_script(*s))
        .collect();
    prefer.dedup();
    let mut out = vec![];
    for r in bad {
        let mut fams: Vec<String> = vec![];
        for ch in text[r.clone()].chars().filter(|c| !needs_no_glyph(*c)) {
            if let Some(f) = fallback_family(font_cx, cache, ch, &prefer)
                && f != family
                && !fams.contains(&f)
            {
                fams.push(f);
            }
        }
        if !fams.is_empty() {
            out.push((r, fams));
        }
    }
    out
}

const BULLETS: [&str; 3] = ["\u{2022}", "\u{25E6}", "\u{25AA}"];

/// Lays out each paragraph on its own (so each can have its own indent,
/// list marker and alignment) and stacks them.
/// Style changes pushed to parley for one paragraph, under its 65,535.
const MAX_STYLE_PUSHES: usize = 60_000;
/// Spans a text can format: a span's brush holds its index + 1 in a u32
/// (it held a u16, so spans past 65,534 lost their formatting: B14).
const MAX_BRUSH_SPANS: usize = u32::MAX as usize - 1;

fn do_layout(g: &mut Inner, key: &Key, size: f32) -> TextLayout {
    let t = data_of(key);
    let Inner { font_cx, layout_cx, fallbacks, .. } = g;
    let wrap = key.wrap_bits.map(|b| f64::from_bits(b) as f32);
    let paras: Vec<&str> = t.text.split('\n').collect();
    let formats: Vec<SpanFormat> = t.spans.iter().map(|s| s.format.clone()).collect();
    let spans_sorted = t.spans.windows(2).all(|w| w[0].end <= w[1].start);

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

    // The box's weight: the style's, or Bold if the box is (see
    // `bold_weight`: Bold in a heavy style keeps the style's weight).
    let base_weight = zebra_core::theme::bold_weight(key.weight, key.bold.then_some(true));

    // `extra`: byte ranges to set in the theme family followed by
    // fallback families (see `tofu_fixes`).
    let build = |font_cx: &mut FontContext,
                 layout_cx: &mut LayoutContext<[u8; 4]>,
                 p: &P,
                 extra: &[(Range<usize>, Vec<String>)]| {
        let mut b = layout_cx.ranged_builder(font_cx, &p.text, 1.0, true);
        b.push_default(StyleProperty::FontFamily(FontFamily::named(&key.family)));
        b.push_default(StyleProperty::FontSize(size));
        b.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(LINE_HEIGHT)));
        if base_weight != 400 {
            b.push_default(StyleProperty::FontWeight(FontWeight::new(base_weight as f32)));
        }
        if key.italic {
            b.push_default(StyleProperty::FontStyle(FontStyle::Italic));
        }
        let pchars = p.text[p.marker_len..].chars().count();
        // parley holds at most 65,535 styles in a paragraph, and a span's
        // brush holds its index in a u32: past either, the characters keep
        // the box's own style. Sorted spans (normalized, as open and every
        // edit leave them) start at the first that reaches this paragraph.
        let first = if spans_sorted { t.spans.partition_point(|s| s.end <= p.char0) } else { 0 };
        let mut pushes = 0;
        for (si, s) in t.spans.iter().enumerate().skip(first).take(MAX_BRUSH_SPANS.saturating_sub(first)) {
            if spans_sorted && s.start >= p.char0 + pchars {
                break;
            }
            let (a, z) = (s.start.max(p.char0), s.end.min(p.char0 + pchars));
            if a >= z {
                continue;
            }
            // What this span pushes: its brush and each property it sets.
            let f = &s.format;
            let need = 1 + [f.bold.is_some(), f.italic.is_some(), f.size.is_some()].iter().filter(|x| **x).count();
            if pushes + need > MAX_STYLE_PUSHES {
                break;
            }
            pushes += need;
            let body = &p.text[p.marker_len..];
            let r = p.marker_len + rich::byte_at(body, a - p.char0)..p.marker_len + rich::byte_at(body, z - p.char0);
            if let Some(v) = f.bold {
                let w = zebra_core::theme::bold_weight(key.weight, Some(v));
                b.push(StyleProperty::FontWeight(FontWeight::new(w as f32)), r.clone());
            }
            if let Some(v) = f.italic {
                b.push(StyleProperty::FontStyle(if v { FontStyle::Italic } else { FontStyle::Normal }), r.clone());
            }
            if let Some(k) = f.size {
                b.push(StyleProperty::FontSize(size * k as f32), r.clone());
            }
            let n = (si + 1) as u32;
            b.push(StyleProperty::Brush(n.to_le_bytes()), r);
        }
        for (r, fams) in extra.iter().take(MAX_STYLE_PUSHES.saturating_sub(pushes)) {
            let list: Vec<FontFamilyName> =
                std::iter::once(&key.family).chain(fams).map(|f| FontFamilyName::named(f)).collect();
            b.push(StyleProperty::FontFamily(FontFamily::List(list.into())), r.clone());
        }
        let mut layout = b.build(&p.text);
        if p.marker_len > 0 {
            // Hanging indent: wrapped lines line up after the marker.
            let marker = &p.text[..p.marker_len];
            let mut bb = layout_cx.ranged_builder(font_cx, marker, 1.0, true);
            bb.push_default(StyleProperty::FontFamily(FontFamily::named(&key.family)));
            bb.push_default(StyleProperty::FontSize(size));
            if base_weight != 400 {
                bb.push_default(StyleProperty::FontWeight(FontWeight::new(base_weight as f32)));
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
        let width = wrap.map(|w| (w - p.indent_x).max(size));
        let mut l = build(font_cx, layout_cx, p, &[]);
        l.set_alignment(alignment(p.align), AlignmentOptions::default());
        l.break_all_lines(width);
        // A character the run's fonts lack: lay out again with a font
        // that has it added for just that cluster.
        let fixes = tofu_fixes(&l, &p.text, &key.family, font_cx, fallbacks);
        if !fixes.is_empty() {
            l = build(font_cx, layout_cx, p, &fixes);
            l.set_alignment(alignment(p.align), AlignmentOptions::default());
            l.break_all_lines(width);
        }
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
                let fi = u32::from_le_bytes(brush);
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
                    variations: synthesis.variation_settings().iter().map(|(t, v)| (t.to_be_bytes(), *v)).collect(),
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
    fn font_faces(&self, doc: &zebra_core::Document, family: &str) -> Vec<zebra_core::FontFace> {
        self.fonts.faces(Some(doc), family)
    }
}

/// The wrap width a text element is laid out with when drawn.
pub fn wrap_for(t: &TextData, size_w: f64) -> Option<f64> {
    if t.fixed_width { Some(size_w) } else { None }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    /// C5 of the 20 Nov audit: more than 65,535 formatted spans in a text
    /// panicked parley ("too many styles") on the UI thread. Past what
    /// parley holds the characters keep the box's style; the rest is laid
    /// out as before.
    #[test]
    fn a_text_with_70000_spans_lays_out() {
        let fonts = Fonts::new();
        let theme = Theme::by_id("classic").unwrap();
        let n = 70_000;
        let mut t = TextData { text: "ab".repeat(n), ..Default::default() };
        t.spans = (0..n)
            .map(|i| zebra_core::model::TextSpan {
                start: 2 * i,
                end: 2 * i + 1,
                format: SpanFormat { bold: Some(true), italic: Some(i % 2 == 0), ..Default::default() },
            })
            .collect();
        let l = fonts.layout(&t, &theme, Some(2000.0));
        assert!(!l.runs.is_empty() && l.height > 0.0);
        // Many paragraphs, each formatted, still lay out in one pass.
        let mut t = TextData { text: vec!["ab"; n].join("\n"), ..Default::default() };
        t.spans = (0..n).map(|i| zebra_core::model::TextSpan { start: 3 * i, end: 3 * i + 1, format: SpanFormat { bold: Some(true), ..Default::default() } }).collect();
        let at = std::time::Instant::now();
        let l = fonts.layout(&t, &theme, None);
        assert!(l.height > 0.0);
        assert!(at.elapsed().as_secs() < 60, "took {:?}", at.elapsed());
    }

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

/// A bundled TrueType font's bytes (tests of font cutting).
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn bundled_font_bytes_for_tests() -> &'static [u8] {
    BUNDLED[0].files[0]
}
