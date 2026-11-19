//! Help › Acknowledgements: who made the clip art, icons and fonts that
//! ship with Zebra, under which licences. CC BY sets need this credit.
use egui::RichText;

use crate::app::ZebraApp;
use crate::style::{self, Palette};

const FONTS: &[(&str, &str, &str)] = &[
    ("Inter", "The Inter Project Authors", include_str!("../../../assets/fonts/inter/OFL.txt")),
    ("Lora", "The Lora Project Authors", include_str!("../../../assets/fonts/lora/OFL.txt")),
    ("Caveat", "The Caveat Project Authors", include_str!("../../../assets/fonts/caveat/OFL.txt")),
    ("JetBrains Mono", "The JetBrains Mono Project Authors", include_str!("../../../assets/fonts/jetbrains-mono/OFL.txt")),
];

/// The licence files shipped beside PDFium (Zebra.app/Contents/Resources/
/// Licenses/PDFium), by file name. Empty outside the app bundle.
fn pdfium_licences() -> &'static [(String, String)] {
    static L: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    L.get_or_init(|| {
        let Some(dir) = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.join("Resources/Licenses/PDFium"))) else { return vec![] };
        let mut v: Vec<(String, String)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|f| Some((f.file_name().to_string_lossy().to_string(), std::fs::read_to_string(f.path()).ok()?)))
            .collect();
        v.sort();
        v
    })
}

fn entry(ui: &mut egui::Ui, p: &Palette, name: &str, by: &str, licence: &str, url: &str) {
    ui.label(RichText::new(name).font(style::semibold(12.5)).color(p.text));
    ui.label(RichText::new(crate::t!("credits-by", author = by, licence = licence)).size(11.5).color(p.weak));
    if !url.is_empty() {
        ui.add(egui::Label::new(RichText::new(url).size(11.0).color(p.weak)).selectable(true));
    }
    ui.add_space(6.0);
}

pub fn window(app: &mut ZebraApp, ctx: &egui::Context) {
    if !app.show_credits {
        return;
    }
    let p = Palette::of(ctx);
    let mut open = true;
    egui::Window::new(crate::t!("credits-title"))
        .id(app.uid("credits"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([440.0, 520.0])
        .frame(style::float_frame(&p).inner_margin(12))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new(crate::t!("credits-clip-art")).font(style::semibold(13.0)));
                ui.add_space(4.0);
                match zebra_core::symbols::library() {
                    Some(lib) => {
                        for s in lib.sets() {
                            entry(ui, &p, &s.name, &s.author, &s.licence, if s.licence_url.is_empty() { &s.author_url } else { &s.licence_url });
                        }
                    }
                    None => {
                        ui.label(RichText::new(crate::symbols_ui::not_installed()).size(11.5).color(p.weak));
                    }
                }
                ui.add_space(6.0);
                ui.label(RichText::new(crate::t!("credits-icons")).font(style::semibold(13.0)));
                ui.add_space(4.0);
                entry(ui, &p, "Lucide", "Lucide Contributors", "ISC", "https://lucide.dev");
                egui::CollapsingHeader::new(crate::t!("credits-licence-text")).id_salt("lucide-licence").show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(zebra_core::icons::LICENSE).size(10.5).monospace()).wrap());
                });
                ui.add_space(6.0);
                ui.label(RichText::new(crate::t!("credits-software")).font(style::semibold(13.0)));
                ui.add_space(4.0);
                // File › Import (PDF). The licences of the libraries built
                // into PDFium ship beside it (scripts/bundle-macos.sh).
                entry(ui, &p, "PDFium", "The PDFium Authors", "BSD 3-Clause", "https://pdfium.googlesource.com/pdfium");
                entry(ui, &p, "pdfium-binaries", "Benoît Blanchon", "MIT", "https://github.com/bblanchon/pdfium-binaries");
                entry(ui, &p, "pdfium-render", "Alastair Carey", "MIT / Apache 2.0", "https://github.com/ajrcarey/pdfium-render");
                for (name, text) in pdfium_licences() {
                    egui::CollapsingHeader::new(name.as_str()).id_salt(("pdfium-licence", name.as_str())).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(text).size(10.5).monospace()).wrap());
                    });
                }
                ui.add_space(6.0);
                ui.label(RichText::new(crate::t!("credits-fonts")).font(style::semibold(13.0)));
                ui.add_space(4.0);
                for (name, by, text) in FONTS {
                    entry(ui, &p, name, by, "SIL Open Font License 1.1", "");
                    egui::CollapsingHeader::new(crate::t!("credits-licence-text")).id_salt(("font-licence", *name)).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(*text).size(10.5).monospace()).wrap());
                    });
                }
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    app.show_credits = open;
}
