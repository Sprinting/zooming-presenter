//! View › Show Document Warnings, as Keynote's Warnings window: what
//! opening or importing the document couldn't bring in, one line each,
//! selectable and with Copy so the lines can be pasted elsewhere. It opens
//! by itself when an open adds warnings; the person can close it and open
//! it again from the View menu at any time.
use egui::RichText;
use zebra_core::warnings::Warning;

use crate::app::ZebraApp;
use crate::style::{self, Palette};

/// One warning in the person's language.
pub fn line(w: &Warning) -> String {
    match w {
        Warning::AssetNotRead { kind, disk_full, .. } => {
            use zebra_core::warnings::MediaWord;
            let kind = match kind {
                MediaWord::Picture => "picture",
                MediaWord::Video => "video",
                MediaWord::Sound => "sound",
                MediaWord::Font => "font",
                MediaWord::Other => "other",
            };
            if *disk_full { crate::t!("warning-media-disk-full", kind = kind) } else { crate::t!("warning-media-unreadable", kind = kind) }
        }
        Warning::Import { file, note, count, slides } => {
            let notes = std::collections::BTreeMap::from([(*note, *count)]);
            let named = std::collections::BTreeMap::from([(*note, slides.clone())]);
            let text = crate::import_ui::note_lines(&notes, &named).join(" ");
            crate::t!("warning-import", file = file.as_str(), text = text)
        }
    }
}

/// Every warning, one per line (what Copy puts on the clipboard).
pub fn text(app: &ZebraApp) -> String {
    app.editor.doc.warnings.iter().map(line).collect::<Vec<_>>().join("\n")
}

pub fn window(app: &mut ZebraApp, ctx: &egui::Context) {
    if !app.show_warnings {
        return;
    }
    let p = Palette::of(ctx);
    let mut open = true;
    let mut copy = false;
    egui::Window::new(crate::t!("warnings-title"))
        .id(app.uid("document-warnings"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([460.0, 280.0])
        .frame(style::float_frame(&p).inner_margin(12))
        .show(ctx, |ui| {
            if app.editor.doc.warnings.is_empty() {
                ui.label(RichText::new(crate::t!("warnings-none")).color(p.weak));
                return;
            }
            egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(360.0).show(ui, |ui| {
                for w in app.editor.doc.warnings.iter() {
                    ui.add(egui::Label::new(RichText::new(line(w)).color(p.text)).selectable(true).wrap());
                    ui.add_space(4.0);
                }
            });
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::widgets::push_button(ui, &crate::t!("warnings-copy")).clicked() {
                    copy = true;
                }
            });
        });
    if copy {
        let t = text(app);
        ctx.copy_text(t);
    }
    if !open {
        app.show_warnings = false;
    }
}
