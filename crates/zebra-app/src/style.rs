//! Chrome look: fonts, colours, egui visuals. Light, quiet, floating panels
//! (tldraw/Keynote-ish) with a few Prezi-era nods (brackets, zebra stripes).
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke};
use std::sync::Arc;

/// Human selection colour (system-blue-like).
pub const ACCENT: Color32 = Color32::from_rgb(0x1f, 0x7a, 0xf0);
/// Reserved agent colour. Never used for anything human.
pub const AGENT: Color32 = Color32::from_rgb(0x8b, 0x5c, 0xf6);
/// Snapping guides.
pub const GUIDE: Color32 = Color32::from_rgb(0xff, 0x3d, 0x7f);
/// Path line and badges (Prezi path was a dark slate line with round badges).
pub const PATH: Color32 = Color32::from_rgb(0x2c, 0x3e, 0x50);
/// The Present button.
pub const PRESENT: Color32 = Color32::from_rgb(0x1f, 0x7a, 0xf0);

pub const SANS: &str = "Sans";
pub const SANS_BOLD: &str = "SansBold";

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub panel: Color32,
    pub canvas_chrome: Color32,
    pub border: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub hover: Color32,
    pub float: Color32,
}

impl Palette {
    pub fn of(ctx: &egui::Context) -> Palette {
        Self::for_dark(ctx.global_style().visuals.dark_mode)
    }
    pub fn for_dark(dark: bool) -> Palette {
        if dark {
            Palette {
                dark,
                panel: Color32::from_rgb(0x23, 0x24, 0x28),
                canvas_chrome: Color32::from_rgb(0x1a, 0x1b, 0x1e),
                border: Color32::from_rgb(0x3a, 0x3b, 0x40),
                text: Color32::from_rgb(0xec, 0xec, 0xee),
                weak: Color32::from_rgb(0x9a, 0x9c, 0xa3),
                hover: Color32::from_rgb(0x34, 0x35, 0x3b),
                float: Color32::from_rgb(0x2b, 0x2c, 0x31),
            }
        } else {
            Palette {
                dark,
                panel: Color32::from_rgb(0xfb, 0xfb, 0xfc),
                canvas_chrome: Color32::from_rgb(0xf3, 0xf3, 0xf5),
                border: Color32::from_rgb(0xe3, 0xe4, 0xe8),
                text: Color32::from_rgb(0x1d, 0x1f, 0x24),
                weak: Color32::from_rgb(0x6e, 0x72, 0x7b),
                hover: Color32::from_rgb(0xee, 0xef, 0xf2),
                float: Color32::WHITE,
            }
        }
    }
}

pub fn float_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.float)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(6))
        .shadow(Shadow { offset: [0, 2], blur: 12, spread: 0, color: Color32::from_black_alpha(if p.dark { 90 } else { 28 }) })
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

    let fallbacks: Vec<String> = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let fam = |defs: &mut FontDefinitions, name: &str, primary: &str| {
        let mut v = vec![primary.to_string()];
        v.extend(fallbacks.iter().cloned());
        defs.families.insert(FontFamily::Name(name.into()), v);
    };
    fam(&mut defs, SANS, "inter");
    fam(&mut defs, SANS_BOLD, "inter-bold");
    for (base, file) in [("Sans", "inter"), ("Serif", "lora"), ("Hand", "caveat"), ("Mono", "jbm")] {
        for (suffix, fsuffix) in [("", ""), ("-b", "-bold"), ("-i", "-italic"), ("-bi", "-bolditalic")] {
            let f = format!("{file}{fsuffix}");
            let f = if defs.font_data.contains_key(&f) { f } else { format!("{file}{}", if suffix.contains('b') { "-bold" } else { "" }) };
            let f = if defs.font_data.contains_key(&f) { f } else { file.to_string() };
            fam(&mut defs, &format!("doc:{base}{suffix}"), &f);
        }
    }
    defs.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "inter".into());
    ctx.set_fonts(defs);
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

pub fn apply_visuals(ctx: &egui::Context) {
    ctx.options_mut(|o| {
        o.zoom_with_keyboard = false;
        // Follow the system appearance; light when the platform doesn't say.
        o.fallback_theme = egui::Theme::Light;
    });
    for dark in [false, true] {
        let p = Palette::for_dark(dark);
        let theme = if dark { egui::Theme::Dark } else { egui::Theme::Light };
        ctx.style_mut_of(theme, |s| {
            let v = &mut s.visuals;
            v.panel_fill = p.panel;
            v.window_fill = p.float;
            v.window_stroke = Stroke::new(1.0, p.border);
            v.window_corner_radius = CornerRadius::same(12);
            v.menu_corner_radius = CornerRadius::same(8);
            v.extreme_bg_color = if dark { Color32::from_rgb(0x18, 0x19, 0x1c) } else { Color32::from_rgb(0xf1, 0xf2, 0xf5) };
            v.faint_bg_color = p.hover;
            v.selection.bg_fill = ACCENT.gamma_multiply(if dark { 0.45 } else { 0.22 });
            v.selection.stroke = Stroke::new(1.0, ACCENT);
            v.hyperlink_color = ACCENT;
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
            v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
            for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
                w.corner_radius = CornerRadius::same(6);
            }
            v.widgets.inactive.weak_bg_fill = if dark { Color32::from_rgb(0x30, 0x31, 0x36) } else { Color32::from_rgb(0xf0, 0xf1, 0xf4) };
            v.widgets.inactive.bg_fill = p.hover;
            v.widgets.inactive.bg_stroke = Stroke::NONE;
            v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
            v.widgets.hovered.weak_bg_fill = if dark { Color32::from_rgb(0x3a, 0x3b, 0x41) } else { Color32::from_rgb(0xe6, 0xe7, 0xeb) };
            v.widgets.hovered.bg_fill = p.hover;
            v.widgets.hovered.bg_stroke = Stroke::NONE;
            v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
            v.widgets.active.weak_bg_fill = p.border;
            v.widgets.active.bg_fill = p.border;
            v.widgets.active.bg_stroke = Stroke::NONE;
            v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
            v.widgets.open.bg_fill = p.hover;
            v.widgets.open.fg_stroke = Stroke::new(1.0, p.text);
            v.weak_text_color = Some(p.weak);
            v.widgets.open.weak_bg_fill = p.hover;
            v.popup_shadow = Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if dark { 100 } else { 30 }) };
            v.window_shadow = v.popup_shadow;
            s.spacing.button_padding = egui::vec2(8.0, 4.0);
            s.spacing.item_spacing = egui::vec2(6.0, 6.0);
            s.spacing.interact_size.y = 26.0;
            s.spacing.menu_margin = Margin::same(6);
            s.spacing.window_margin = Margin::same(10);
            // Short ease on hover/press fills so the chrome feels less twitchy.
            s.animation_time = 0.12;
            s.text_styles.insert(egui::TextStyle::Body, FontId::new(13.5, FontFamily::Proportional));
            s.text_styles.insert(egui::TextStyle::Button, FontId::new(13.5, FontFamily::Proportional));
            s.text_styles.insert(egui::TextStyle::Small, FontId::new(11.5, FontFamily::Proportional));
            s.text_styles.insert(egui::TextStyle::Heading, FontId::new(17.0, FontFamily::Name(SANS_BOLD.into())));
        });
    }
}

pub fn c32(rgba: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3])
}
