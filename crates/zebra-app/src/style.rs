//! Chrome look: classic macOS (Big Sur to Sonoma, before Liquid Glass),
//! drawn entirely in egui. Semantic colours, the system accent, SF Pro in the
//! chrome when the Mac has it (Inter elsewhere), AppKit control metrics.
//! Controls themselves live in `widgets.rs`.
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Default accent (macOS system blue). The live accent is [`accent()`],
/// which follows the system accent colour on macOS.
pub const ACCENT: Color32 = Color32::from_rgb(0x00, 0x7a, 0xff);
/// Reserved agent colour. Never used for anything human; does not follow
/// the system accent (it is an identity colour).
pub const AGENT: Color32 = Color32::from_rgb(0x8b, 0x5c, 0xf6);
/// Snapping guides.
pub const GUIDE: Color32 = Color32::from_rgb(0xff, 0x3d, 0x7f);
/// Path line and badges (Prezi path was a dark slate line with round badges).
pub const PATH: Color32 = Color32::from_rgb(0x2c, 0x3e, 0x50);
/// Kept for compatibility; the Play button now uses the accent.
pub const PRESENT: Color32 = ACCENT;

pub const SANS: &str = "Sans";
pub const SANS_BOLD: &str = "SansBold";
/// Chrome font, semibold (Apple's 590 weight on SF Pro; Inter SemiBold-ish
/// via Inter Bold elsewhere). Section headers and the window title use it.
pub const UI_SEMIBOLD: &str = "UiSemibold";

static ACCENT_RGBA: AtomicU32 = AtomicU32::new(0);

/// The live accent colour: the macOS system accent (Blue, Purple, Pink,
/// Red, Orange, Yellow, Green, Graphite) when known, else system blue.
pub fn accent() -> Color32 {
    let v = ACCENT_RGBA.load(Ordering::Relaxed);
    if v == 0 {
        ACCENT
    } else {
        let [r, g, b, _] = v.to_be_bytes();
        Color32::from_rgb(r, g, b)
    }
}

/// Set the accent (from `controlAccentColor` on macOS, or a test).
pub fn set_accent(c: Color32) {
    ACCENT_RGBA.store(u32::from_be_bytes([c.r(), c.g(), c.b(), 0xff]), Ordering::Relaxed);
}

/// Semantic chrome colours, hand-matched to AppKit's Aqua and Dark Aqua
/// (Big Sur to Sonoma), resolved to opaque sRGB over their usual
/// backgrounds. Names follow NSColor where one exists.
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Inspector and dialog background (windowBackgroundColor).
    pub panel: Color32,
    /// Unified title bar + toolbar.
    pub toolbar: Color32,
    /// Opaque source-list sidebar (no vibrancy, by decision).
    pub sidebar: Color32,
    pub canvas_chrome: Color32,
    /// separatorColor.
    pub border: Color32,
    /// labelColor.
    pub text: Color32,
    /// secondaryLabelColor (kept at AA contrast on every panel).
    pub weak: Color32,
    /// tertiaryLabelColor: disabled text and glyphs.
    pub disabled: Color32,
    /// Hover/pressed tint for borderless items (toolbar, list rows).
    pub hover: Color32,
    pub pressed: Color32,
    /// controlBackgroundColor: text fields, lists, popover content.
    pub field: Color32,
    /// Push button / segmented bezel fill.
    pub control: Color32,
    pub control_pressed: Color32,
    /// Bezel outline for buttons and fields.
    pub control_edge: Color32,
    /// Selected segment of a segmented control (not the accent: AppKit
    /// draws the selected segment as a lighter/raised plate).
    pub segment_on: Color32,
    pub float: Color32,
}

impl Palette {
    pub fn of(ctx: &egui::Context) -> Palette {
        Self::for_dark(ctx.global_style().visuals.dark_mode)
    }
    pub fn for_dark(dark: bool) -> Palette {
        let g = Color32::from_gray;
        if dark {
            Palette {
                dark,
                panel: g(0x2b),
                toolbar: g(0x36),
                sidebar: g(0x28),
                canvas_chrome: g(0x1e),
                border: g(0x1c),
                text: g(0xdf),
                weak: g(0x9d),
                disabled: g(0x6a),
                hover: Color32::from_white_alpha(18),
                pressed: Color32::from_white_alpha(34),
                field: g(0x1f),
                control: g(0x5a),
                control_pressed: g(0x70),
                control_edge: g(0x48),
                segment_on: g(0x6c),
                float: g(0x2e),
            }
        } else {
            Palette {
                dark,
                panel: g(0xf5),
                toolbar: g(0xea),
                sidebar: g(0xe8),
                canvas_chrome: g(0xd8),
                border: g(0xd0),
                text: g(0x26),
                weak: g(0x6b),
                disabled: g(0xa8),
                hover: Color32::from_black_alpha(14),
                pressed: Color32::from_black_alpha(30),
                field: Color32::WHITE,
                control: Color32::WHITE,
                control_pressed: g(0xe2),
                control_edge: g(0xc4),
                segment_on: Color32::WHITE,
                float: Color32::WHITE,
            }
        }
    }
}

pub fn float_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.float)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(4))
        .shadow(Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(if p.dark { 90 } else { 30 }) })
}

macro_rules! font {
    ($p:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/", $p))
    };
}

/// Fonts: Inter for the chrome; theme families (Sans/Serif/Hand/Mono) for the
/// in-place text editor so typing looks like the result.
pub fn install_fonts(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        defs.font_data.insert(name.to_string(), Arc::new(FontData::from_static(bytes)));
    };
    add("inter", font!("inter/Inter-Regular.ttf"));
    add("inter-bold", font!("inter/Inter-Bold.ttf"));
    add("inter-italic", font!("inter/Inter-Italic.ttf"));
    add("inter-bolditalic", font!("inter/Inter-BoldItalic.ttf"));
    add("lora", font!("lora/Lora-Regular.ttf"));
    add("lora-bold", font!("lora/Lora-Bold.ttf"));
    add("lora-italic", font!("lora/Lora-Italic.ttf"));
    add("lora-bolditalic", font!("lora/Lora-BoldItalic.ttf"));
    add("caveat", font!("caveat/Caveat-Regular.ttf"));
    add("caveat-bold", font!("caveat/Caveat-Bold.ttf"));
    add("jbm", font!("jetbrains-mono/JetBrainsMono-Regular.ttf"));
    add("jbm-bold", font!("jetbrains-mono/JetBrainsMono-Bold.ttf"));
    add("jbm-italic", font!("jetbrains-mono/JetBrainsMono-Italic.ttf"));
    add("jbm-bolditalic", font!("jetbrains-mono/JetBrainsMono-BoldItalic.ttf"));

    // Inter is the first fallback everywhere: SF Pro and the theme fonts
    // lack some of the symbols the chrome uses (⇤ ↵ ⌫ ⎋ …), and egui's own
    // fonts have none of them, so without it they drew as empty boxes.
    let mut fallbacks: Vec<String> = vec!["inter".into()];
    fallbacks.extend(defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    let fam = |defs: &mut FontDefinitions, name: &str, primary: &str| {
        let mut v = vec![primary.to_string()];
        v.extend(fallbacks.iter().filter(|f| *f != primary).cloned());
        defs.families.insert(FontFamily::Name(name.into()), v);
    };
    // Chrome font: SF Pro (the system font, read from the Mac at runtime,
    // never bundled) at Regular and Semibold through its weight axis. Inter
    // everywhere else, and if the system font can't be read.
    let (ui_regular, ui_semibold) = match system_ui_font() {
        Some(blob) => {
            let mk = |w: f32| FontData {
                font: blob.clone(),
                index: 0,
                tweak: egui::epaint::text::FontTweak {
                    coords: egui::epaint::text::VariationCoords::new([(b"wght", w)]),
                    ..Default::default()
                },
            };
            defs.font_data.insert("sf".into(), Arc::new(mk(400.0)));
            defs.font_data.insert("sf-semibold".into(), Arc::new(mk(590.0)));
            ("sf", "sf-semibold")
        }
        None => ("inter", "inter-bold"),
    };
    fam(&mut defs, SANS, ui_regular);
    fam(&mut defs, SANS_BOLD, ui_semibold);
    fam(&mut defs, UI_SEMIBOLD, ui_semibold);
    for (base, file) in [("Sans", "inter"), ("Serif", "lora"), ("Hand", "caveat"), ("Mono", "jbm")] {
        for (suffix, fsuffix) in [("", ""), ("-b", "-bold"), ("-i", "-italic"), ("-bi", "-bolditalic")] {
            let f = format!("{file}{fsuffix}");
            let f = if defs.font_data.contains_key(&f) { f } else { format!("{file}{}", if suffix.contains('b') { "-bold" } else { "" }) };
            let f = if defs.font_data.contains_key(&f) { f } else { file.to_string() };
            fam(&mut defs, &format!("doc:{base}{suffix}"), &f);
        }
    }
    let prop = defs.families.get_mut(&FontFamily::Proportional).unwrap();
    prop.insert(0, ui_regular.into());
    if ui_regular != "inter" {
        prop.insert(1, "inter".into());
    }
    ctx.set_fonts(defs);
}

/// SF Pro from the running Mac (`SFNS.ttf`, a variable font). `None` off
/// macOS, or when `ZEBRA_NO_SYSTEM_FONT` is set (screenshot comparisons).
fn system_ui_font() -> Option<egui::epaint::text::Blob> {
    if !cfg!(target_os = "macos") || std::env::var_os("ZEBRA_NO_SYSTEM_FONT").is_some() {
        return None;
    }
    let bytes = std::fs::read("/System/Library/Fonts/SFNS.ttf").ok()?;
    Some(std::sync::Arc::new(bytes) as egui::epaint::text::Blob)
}

/// egui family for a theme font name + bold/italic.
pub fn doc_family(font: &str, bold: bool, italic: bool) -> FontFamily {
    let base = match font {
        "Serif" | "Hand" | "Mono" => font,
        _ => "Sans",
    };
    let suffix = match (bold, italic) {
        (false, false) => "",
        (true, false) => "-b",
        (false, true) => "-i",
        (true, true) => "-bi",
    };
    FontFamily::Name(format!("doc:{base}{suffix}").into())
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SANS_BOLD.into()))
}

/// Chrome semibold (section headers, window title, default buttons).
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(UI_SEMIBOLD.into()))
}

/// Chrome regular at a size (13 controls, 11 small labels, 10 toolbar labels).
pub fn ui_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn apply_visuals(ctx: &egui::Context) {
    ctx.options_mut(|o| {
        o.zoom_with_keyboard = false;
        // Follow the system appearance; light when the platform doesn't say.
        o.fallback_theme = egui::Theme::Light;
    });
    for dark in [false, true] {
        let p = Palette::for_dark(dark);
        let acc = accent();
        let theme = if dark { egui::Theme::Dark } else { egui::Theme::Light };
        ctx.style_mut_of(theme, |s| {
            let v = &mut s.visuals;
            v.panel_fill = p.panel;
            v.window_fill = p.panel;
            v.window_stroke = Stroke::new(1.0, if dark { Color32::from_gray(0x50) } else { Color32::from_black_alpha(40) });
            v.window_corner_radius = CornerRadius::same(10);
            // NSMenu: 6 px radius, 5 px inset, opaque in this style.
            v.menu_corner_radius = CornerRadius::same(6);
            v.extreme_bg_color = p.field;
            v.text_edit_bg_color = Some(p.field);
            v.faint_bg_color = if dark { Color32::from_white_alpha(6) } else { Color32::from_black_alpha(8) };
            v.selection.bg_fill = acc.gamma_multiply(if dark { 0.55 } else { 0.30 });
            v.selection.stroke = Stroke::new(1.0, acc);
            v.hyperlink_color = acc;
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
            v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
            v.widgets.noninteractive.bg_fill = p.panel;
            for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
                w.corner_radius = CornerRadius::same(5);
                w.expansion = 0.0;
            }
            // Fallback egui widgets (anything not yet on the Mac kit) get a
            // push-button bezel rather than web-style grey chips.
            v.widgets.inactive.weak_bg_fill = p.control;
            v.widgets.inactive.bg_fill = p.control;
            v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.control_edge);
            v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
            v.widgets.hovered.weak_bg_fill = p.control;
            v.widgets.hovered.bg_fill = p.control;
            v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.control_edge);
            v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
            v.widgets.active.weak_bg_fill = p.control_pressed;
            v.widgets.active.bg_fill = p.control_pressed;
            v.widgets.active.bg_stroke = Stroke::new(1.0, p.control_edge);
            v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
            v.widgets.open.weak_bg_fill = p.control_pressed;
            v.widgets.open.bg_fill = p.control_pressed;
            v.widgets.open.bg_stroke = Stroke::new(1.0, p.control_edge);
            v.widgets.open.fg_stroke = Stroke::new(1.0, p.text);
            v.weak_text_color = Some(p.weak);
            v.popup_shadow = Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(if dark { 110 } else { 40 }) };
            v.window_shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if dark { 120 } else { 55 }) };
            v.striped = true;
            v.handle_shape = egui::style::HandleShape::Circle;
            v.slider_trailing_fill = true;
            s.spacing.button_padding = egui::vec2(9.0, 2.0);
            s.spacing.item_spacing = egui::vec2(6.0, 6.0);
            // AppKit regular control height.
            s.spacing.interact_size.y = 22.0;
            s.spacing.menu_margin = Margin::same(5);
            s.spacing.window_margin = Margin::same(16);
            s.spacing.scroll = egui::style::ScrollStyle::floating();
            s.spacing.icon_width = 14.0;
            s.spacing.icon_width_inner = 8.0;
            // Short ease on hover/press fills; AppKit has almost none.
            s.animation_time = 0.08;
            s.text_styles.insert(egui::TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
            s.text_styles.insert(egui::TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
            s.text_styles.insert(egui::TextStyle::Small, FontId::new(11.0, FontFamily::Proportional));
            s.text_styles.insert(egui::TextStyle::Heading, FontId::new(13.0, FontFamily::Name(UI_SEMIBOLD.into())));
            s.text_styles.insert(egui::TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace));
            s.visuals.override_text_color = None;
        });
    }
}

pub fn c32(rgba: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3])
}
