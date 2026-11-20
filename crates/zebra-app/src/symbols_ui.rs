//! The Symbols tab of the Symbols & Shapes window: clip-art sets, search,
//! and tiles drawn from each symbol's SVG on a worker thread.
//!
//! Tiles are drawn when they scroll into view and kept as textures while
//! they are used; a tile not shown for [`zebra_core::symbols::IDLE`] is
//! dropped, like the SVGs behind it.
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

use egui::{Color32, ColorImage, Rect, RichText, Sense, TextureHandle, TextureOptions, Vec2 as EVec2};
use zebra_core::symbols::{library, Library, SymbolRef, Tint};
use zebra_core::{Command, ElementPatch, ViewCommand};

use crate::app::ZebraApp;
use crate::shape_library::Library as Window;
use crate::style::Palette;

/// Tile size in points, and the pixel size tiles are drawn at.
const TILE: f32 = 52.0;
const TILE_PX: u32 = 96;
const COLS: usize = 6;
/// Search results shown at most (searching every set).
const LIMIT: usize = 3000;

enum Tile {
    Pending,
    Ready(TextureHandle),
    Failed,
}

/// Tiles and the search state of the Symbols tab.
pub struct Symbols {
    /// None: every set.
    pub set: Option<u16>,
    tiles: HashMap<(SymbolRef, [u8; 4]), (Tile, Instant)>,
    tx: Option<Sender<(SymbolRef, [u8; 4])>>,
    rx: Option<Receiver<((SymbolRef, [u8; 4]), Option<ColorImage>)>>,
    /// Last search: (query, set) and its hits.
    results: Option<((String, Option<u16>), Vec<SymbolRef>)>,
    last_trim: Option<Instant>,
    /// The grid was drawn since `after_frame` last looked.
    shown: bool,
}

impl Default for Symbols {
    fn default() -> Self {
        Symbols { set: None, tiles: HashMap::new(), tx: None, rx: None, results: None, last_trim: None, shown: false }
    }
}

impl Symbols {
    fn start(&mut self, ctx: &egui::Context) {
        if self.tx.is_some() {
            return;
        }
        let (jtx, jrx) = channel::<(SymbolRef, [u8; 4])>();
        let (rtx, rrx) = channel();
        let wake = ctx.clone();
        let _ = std::thread::Builder::new().name("zebra-symbol-tiles".into()).spawn(move || {
            while let Ok(job) = jrx.recv() {
                let img = library().and_then(|lib| draw_tile(lib, job.0, job.1));
                if rtx.send((job, img)).is_err() {
                    return;
                }
                wake.request_repaint();
            }
        });
        self.tx = Some(jtx);
        self.rx = Some(rrx);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else { return };
        while let Ok((k, img)) = rx.try_recv() {
            let tile = match img {
                Some(img) => Tile::Ready(ctx.load_texture(format!("symbol-{}-{}", k.0.set, k.0.index), img, TextureOptions::LINEAR)),
                None => Tile::Failed,
            };
            self.tiles.insert(k, (tile, Instant::now()));
        }
        // Drop tiles not shown for a while (checked once a minute).
        let now = Instant::now();
        if self.last_trim.is_none_or(|t| now.duration_since(t).as_secs() >= 60) {
            self.last_trim = Some(now);
            self.tiles.retain(|_, (t, used)| matches!(t, Tile::Pending) || now.duration_since(*used) < zebra_core::symbols::IDLE);
            if let Some(lib) = library() {
                lib.trim();
            }
        }
    }

    /// Tiles held now (for tests and the `perf` query).
    pub fn tiles(&self) -> usize {
        self.tiles.len()
    }

    /// Call once a frame, after the panels: when the grid was not drawn
    /// (the panel closed, or another tab is showing), its tiles, search
    /// results and the library's SVGs are freed. The grid is the only
    /// place that trims them, so a closed panel used to hold them for good.
    pub fn after_frame(&mut self, ctx: &egui::Context) {
        let _ = ctx;
        if std::mem::take(&mut self.shown) || (self.tiles.is_empty() && self.results.is_none()) {
            return;
        }
        if let Some(rx) = &self.rx {
            while rx.try_recv().is_ok() {}
        }
        self.tiles.clear();
        self.results = None;
        self.last_trim = None;
        if let Some(lib) = library() {
            lib.clear();
        }
    }

    fn tile(&mut self, r: SymbolRef, ink: [u8; 4]) -> Option<&TextureHandle> {
        let now = Instant::now();
        let e = self.tiles.entry((r, ink)).or_insert_with(|| {
            if let Some(tx) = &self.tx {
                let _ = tx.send((r, ink));
            }
            (Tile::Pending, now)
        });
        e.1 = now;
        match &e.0 {
            Tile::Ready(t) => Some(t),
            _ => None,
        }
    }

    fn failed(&self, r: SymbolRef, ink: [u8; 4]) -> bool {
        matches!(self.tiles.get(&(r, ink)), Some((Tile::Failed, _)))
    }

    /// Tiles held now (for tests).
    pub fn tiles_held(&self) -> usize {
        self.tiles.len()
    }
}

fn draw_tile(lib: &Library, r: SymbolRef, ink: [u8; 4]) -> Option<ColorImage> {
    let svg = lib.svg(r).ok()?;
    let tint = lib.set(r.set).map(|s| s.tint).unwrap_or_default();
    let style = zebra_render::svg::SvgStyle {
        color: (tint != Tint::None).then_some(ink),
        color2: (tint == Tint::Two).then_some([47, 136, 255, 255]),
        sticker: false,
    };
    let (w, h) = zebra_core::symbols::svg_size(&svg);
    let k = TILE_PX as f64 / w.max(h);
    let (pw, ph) = (((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1));
    let px = zebra_render::svg::rasterize(&svg, &style, pw, ph)?;
    Some(ColorImage::from_rgba_premultiplied([pw as usize, ph as usize], &px))
}

/// Name of a set group, for the set menu.
fn group_name(g: &str) -> String {
    match g {
        "emoji" => crate::t!("symbols-group-emoji"),
        "colour" => crate::t!("symbols-group-colour"),
        "drawn" => crate::t!("symbols-group-drawn"),
        "people" => crate::t!("symbols-group-people"),
        "mono" => crate::t!("symbols-group-mono"),
        _ => crate::t!("symbols-group-classic"),
    }
}

const GROUPS: [&str; 6] = ["emoji", "colour", "drawn", "people", "mono", "classic"];

/// Why there are no symbols: not installed, or a pack this build can't read.
pub fn not_installed() -> String {
    if zebra_core::symbols::library_error().is_some() { crate::t!("symbols-old-pack") } else { crate::t!("symbols-not-installed") }
}

/// The Symbols tab: set menu, then the grid. Clicking a tile inserts the
/// symbol in the middle of the view.
pub fn grid(app: &mut ZebraApp, win: &mut Window, ui: &mut egui::Ui, p: &Palette) {
    let Some(lib) = library() else {
        ui.label(RichText::new(not_installed()).color(p.weak));
        return;
    };
    let mut sym = std::mem::take(&mut app.symbols);
    sym.start(ui.ctx());
    sym.shown = true;
    sym.poll(ui.ctx());

    let all = crate::t!("symbols-all-sets");
    let current = sym.set.and_then(|s| lib.set(s)).map(|s| s.name.clone()).unwrap_or_else(|| all.clone());
    egui::ComboBox::from_id_salt("symbol-set").selected_text(current).width(ui.available_width()).height(420.0).show_ui(ui, |ui| {
        ui.selectable_value(&mut sym.set, None, &all);
        for g in GROUPS {
            ui.separator();
            ui.label(RichText::new(group_name(g)).size(11.0).color(p.weak));
            for (i, s) in lib.sets().enumerate() {
                if s.group == g {
                    ui.selectable_value(&mut sym.set, Some(i as u16), &s.name);
                }
            }
        }
    });
    ui.add_space(4.0);

    let key = (win.query.trim().to_lowercase(), sym.set);
    if sym.results.as_ref().is_none_or(|(k, _)| *k != key) {
        let hits = match (key.0.is_empty(), sym.set) {
            (true, Some(s)) => lib.all_in(s).collect(),
            (true, None) => vec![],
            (false, s) => lib.search(&key.0, s.map(|s| vec![s]).as_deref(), LIMIT),
        };
        sym.results = Some((key.clone(), hits));
    }
    let hits = sym.results.as_ref().map(|r| r.1.clone()).unwrap_or_default();
    if hits.is_empty() {
        let msg = if key.0.is_empty() { crate::t!("symbols-pick-a-set", count = lib.total()) } else { crate::t!("symbols-none-match") };
        ui.label(RichText::new(msg).color(p.weak));
        app.symbols = sym;
        return;
    }
    let ink = {
        let c = ui.visuals().text_color();
        [c.r(), c.g(), c.b(), 255]
    };
    let rows = hits.len().div_ceil(COLS);
    let mut pick = None;
    egui::ScrollArea::vertical().max_height(330.0).show_rows(ui, TILE + 4.0, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for &r in hits.iter().skip(row * COLS).take(COLS) {
                    let (rect, resp) = ui.allocate_exact_size(EVec2::splat(TILE), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 6.0, p.hover);
                    }
                    let name = display_name(lib.name(r));
                    let tip = match (sym.set, lib.set(r.set)) {
                        (None, Some(s)) => format!("{name} · {}", s.name),
                        _ => name.to_string(),
                    };
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tip));
                    if let Some(t) = sym.tile(r, ink) {
                        let inner = rect.shrink(8.0);
                        let sz = t.size_vec2();
                        let k = (inner.width() / sz.x).min(inner.height() / sz.y);
                        let at = Rect::from_center_size(inner.center(), sz * k);
                        ui.painter().image(t.id(), at, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    } else if sym.failed(r, ink) {
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(14.0), p.weak);
                    }
                    if resp.on_hover_text(tip).clicked() {
                        pick = Some(r);
                    }
                }
            });
        }
    });
    ui.add_space(2.0);
    let count = if hits.len() >= LIMIT { crate::t!("symbols-count-many", count = LIMIT) } else { crate::t!("symbols-count", count = hits.len()) };
    ui.label(RichText::new(count).size(11.0).color(p.weak));
    app.symbols = sym;
    if let Some(r) = pick {
        insert(app, win, lib, r);
    }
}

fn insert(app: &mut ZebraApp, win: &mut Window, lib: &Library, r: SymbolRef) {
    let Some(set) = lib.set(r.set) else { return };
    let (set, name) = (set.key.clone(), lib.name(r).to_string());
    let zoom = app.editor.view.camera.zoom;
    let (x, y) = win.drop_point(app);
    let out = app.apply(Command::AddSymbol { set, name, x, y, width: Some(140.0 / zoom), color: None, color2: None, sticker: false, id: None });
    if let Some(o) = out {
        win.inserted += 1;
        app.view_cmd(ViewCommand::Select { ids: o.created });
    }
}

/// Inspector rows for a symbol: where it's from, its colours and the
/// sticker look.
pub fn inspector(insp: &mut crate::inspector::Insp, ui: &mut egui::Ui, look: &zebra_core::SymbolLook) {
    use crate::widgets as w;
    let set_name = library().and_then(|l| l.set_index(&look.set).and_then(|i| l.set(i)).map(|s| s.name.clone())).unwrap_or_else(|| look.set.clone());
    let alt = match &insp.el.kind {
        zebra_core::ElementKind::Image(i) => i.alt.clone(),
        _ => String::new(),
    };
    w::section(ui, &set_name, |ui| {
        w::caption(ui, look.name.as_str());
        let mut alt = alt;
        w::row(ui, &crate::t!("inspector-description"), |ui| {
            let r = w::text_field(ui, &mut alt, ui.available_width(), &crate::t!("inspector-alt-text-hint"));
            insp.track(&r);
            if r.changed() {
                insp.set(ElementPatch { alt: Some(alt.clone()), ..Default::default() });
            }
        });
        let mut st = look.sticker;
        if ui.checkbox(&mut st, crate::t!("symbols-sticker")).on_hover_text(crate::t!("symbols-sticker-tip")).changed() {
            insp.set(ElementPatch { sticker: Some(st), ..Default::default() });
        }
    });
    let slots = [
        ("theme:body", crate::t!("inspector-swatch-text")),
        ("theme:title", crate::t!("inspector-swatch-title")),
        ("theme:accent", crate::t!("inspector-swatch-accent")),
        ("theme:shape1", crate::t!("inspector-swatch-colour-1")),
        ("theme:shape2", crate::t!("inspector-swatch-colour-2")),
        ("theme:shape3", crate::t!("inspector-swatch-colour-3")),
        ("theme:shape4", crate::t!("inspector-swatch-colour-4")),
    ];
    let (dark, white) = (crate::t!("symbols-swatch-dark"), crate::t!("symbols-swatch-white"));
    let mut sw = crate::inspector::theme_swatches(insp, &slots);
    sw.push((Color32::from_rgb(0x1d, 0x1d, 0x1f), "#1d1d1f", dark.as_str()));
    sw.push((Color32::WHITE, "#ffffff", white.as_str()));
    if let Some(c) = &look.color {
        let title = if look.color2.is_some() { crate::t!("symbols-outline-colour") } else { crate::t!("inspector-section-colour") };
        w::section(ui, &title, |ui| {
            if let Some(k) = crate::inspector::colour_picker(ui, "symbol-colour", c, Color32::GRAY, &sw) {
                insp.set(ElementPatch { color: Some(k), ..Default::default() });
            }
        });
    }
    if let Some(c) = &look.color2 {
        w::section(ui, &crate::t!("symbols-fill-colour"), |ui| {
            if let Some(k) = crate::inspector::colour_picker(ui, "symbol-colour-2", c, Color32::GRAY, &sw) {
                insp.set(ElementPatch { color2: Some(k), ..Default::default() });
            }
        });
    }
}

/// A symbol's name as people read it. Pack names are upstream file names
/// ("add-circle-20", "zoom_out"): hyphens and underscores become spaces, a
/// trailing size or index number goes, and the first letter is a capital.
pub fn display_name(raw: &str) -> String {
    let spaced = raw.replace(['-', '_'], " ");
    let mut words: Vec<&str> = spaced.split_whitespace().collect();
    if words.len() > 1 && words.last().is_some_and(|w| w.chars().all(|c| c.is_ascii_digit())) {
        words.pop();
    }
    let joined = words.join(" ");
    let mut c = joined.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => raw.to_string(),
    }
}

#[cfg(test)]
mod name_tests {
    use super::display_name;

    #[test]
    fn pack_names_read_as_words() {
        assert_eq!(display_name("add-circle-20"), "Add circle");
        assert_eq!(display_name("2nd-place-medal"), "2nd place medal");
        assert_eq!(display_name("micah 120"), "Micah");
        assert_eq!(display_name("zoom_out"), "Zoom out");
        assert_eq!(display_name("1"), "1");
        assert_eq!(display_name("a-b-off"), "A b off");
    }
}
