//! The theme editor (Prezi Classic's "Customize current theme"): colours,
//! fonts (any installed font, embedded in the file), a background picture
//! and a 3D background of up to three layers. Changes show live on the
//! canvas; each burst of edits is one undo step.
use egui::{Color32, RichText, Ui};
use zebra_core::theme::{BackgroundLayer, MAX_LAYERS};
use zebra_core::{Command, Origin, Theme};

use crate::app::ZebraApp;
use crate::style;

const H: Origin = Origin::Human;
/// Seconds without a change before a burst of edits becomes one undo step.
const COMMIT_AFTER: f64 = 0.8;

pub struct ThemeEditor {
    /// The theme being edited (a custom theme of this document).
    pub draft: Theme,
    families: Vec<String>,
    font_query: [String; 3],
    /// When the last change was applied; the open transaction commits after a pause.
    last_change: Option<f64>,
    status: Option<String>,
}

impl ThemeEditor {
    /// Start editing the presentation's theme. A built-in theme is copied to
    /// a new custom theme first.
    pub fn open(app: &ZebraApp) -> Self {
        let mut draft = app.editor.theme();
        if Theme::is_builtin(&draft.id) {
            let mut n = 1;
            while app.editor.doc.themes.iter().any(|t| t.id == format!("custom-{n}")) {
                n += 1;
            }
            draft.id = format!("custom-{n}");
            draft.name = format!("{} (custom)", draft.name);
        }
        let mut families = app.editor.measure.font_families();
        for b in ["Sans", "Serif", "Mono", "Hand"].iter().rev() {
            families.insert(0, b.to_string());
        }
        ThemeEditor { draft, families, font_query: Default::default(), last_change: None, status: None }
    }
}

fn hex_to_c32(h: &str) -> Color32 {
    zebra_core::theme::parse_hex(h).map(style::c32).unwrap_or(Color32::from_rgb(255, 0, 255))
}

fn c32_to_hex(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

/// Show the editor window if open.
pub fn window(app: &mut ZebraApp, ctx: &egui::Context) {
    let Some(mut te) = app.theme_ed.take() else { return };
    let now = app.now();
    let mut open = true;
    let mut changed = false;
    let mut close = false;
    egui::Window::new("Theme")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(320.0)
        .default_pos(ctx.content_rect().right_top() + egui::vec2(-360.0, 70.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height((ctx.content_rect().height() - 200.0).min(560.0)).show(ui, |ui| {
                changed |= editor_ui(app, ui, &mut te);
            });
            footer(app, ui, &mut te, &mut close);
        });
    if changed {
        apply_draft(app, &mut te, now);
    }
    // Commit a burst of edits as one undo step once things settle.
    if let Some(t) = te.last_change {
        if now - t > COMMIT_AFTER {
            if app.editor.human_gesture_active() {
                app.editor.commit();
            }
            te.last_change = None;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
    if open && !close {
        app.theme_ed = Some(te);
    } else if app.editor.human_gesture_active() {
        app.editor.commit();
    }
}

fn apply_draft(app: &mut ZebraApp, te: &mut ThemeEditor, now: f64) {
    if !app.editor.in_transaction() {
        let _ = app.editor.begin("Edit theme", H);
    }
    let r = app.editor.apply(Command::SaveTheme { theme: te.draft.clone(), apply: true }, H);
    if app.report(r).is_some() {
        // Embedded fonts were filled in by the editor: keep the draft in step.
        if let Some(t) = app.editor.doc.themes.iter().find(|t| t.id == te.draft.id) {
            te.draft.fonts = t.fonts.clone();
        }
    }
    te.last_change = Some(now);
}

/// Returns true when the draft changed.
fn editor_ui(app: &mut ZebraApp, ui: &mut Ui, te: &mut ThemeEditor) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Name");
        changed |= ui.text_edit_singleline(&mut te.draft.name).lost_focus();
    });
    ui.horizontal(|ui| {
        ui.label("Start from");
        ui.menu_button("Choose a theme…", |ui| {
            let all: Vec<Theme> = Theme::builtins().into_iter().chain(app.editor.doc.themes.iter().cloned()).collect();
            for t in all {
                if t.id != te.draft.id && ui.selectable_label(false, &t.name).clicked() {
                    let (id, name) = (te.draft.id.clone(), te.draft.name.clone());
                    te.draft = t;
                    te.draft.id = id;
                    te.draft.name = name;
                    changed = true;
                }
            }
        });
    });

    section(ui, "Colours");
    egui::Grid::new("theme-colours").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
        let d = &mut te.draft;
        let mut rows: Vec<(&str, &mut String)> = vec![
            ("Background", &mut d.background),
            ("Title", &mut d.title.color),
            ("Subtitle", &mut d.subtitle.color),
            ("Body text", &mut d.body.color),
            ("Frames", &mut d.frame),
            ("Frame fill", &mut d.frame_fill),
            ("Accent", &mut d.accent),
        ];
        for (i, s) in d.shapes.iter_mut().enumerate() {
            rows.push((["Colour 1", "Colour 2", "Colour 3", "Colour 4"][i], s));
        }
        for (i, (label, hex)) in rows.into_iter().enumerate() {
            let mut c = hex_to_c32(hex);
            ui.label(label);
            if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).changed() {
                *hex = c32_to_hex(c);
                changed = true;
            }
            if i % 2 == 1 {
                ui.end_row();
            }
        }
    });

    section(ui, "Fonts");
    let families = te.families.clone();
    for (i, label) in ["Title", "Subtitle", "Body"].into_iter().enumerate() {
        let st = match i {
            0 => &mut te.draft.title,
            1 => &mut te.draft.subtitle,
            _ => &mut te.draft.body,
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).size(12.0)).on_hover_text("Any installed font. It's embedded in the file when you save, so it shows on other Macs too.");
            let shown = match st.font.as_str() {
                "Sans" => "Sans (Inter)".to_string(),
                "Serif" => "Serif (Lora)".to_string(),
                "Mono" => "Mono (JetBrains Mono)".to_string(),
                "Hand" => "Handwriting (Caveat)".to_string(),
                f => f.to_string(),
            };
            egui::ComboBox::from_id_salt(("font", i)).selected_text(shown).width(170.0).height(320.0).show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut te.font_query[i]).hint_text("Search fonts"));
                let q = te.font_query[i].to_lowercase();
                for f in families.iter().filter(|f| q.is_empty() || f.to_lowercase().contains(&q)).take(400) {
                    if ui.selectable_label(st.font == *f, f).clicked() {
                        st.font = f.clone();
                        changed = true;
                    }
                }
            });
            let r = ui.add(egui::DragValue::new(&mut st.size).range(8.0..=400.0).speed(0.5).suffix(" pt"));
            changed |= r.drag_stopped() || (r.changed() && !r.dragged());
        });
    }
    if !te.draft.fonts.is_empty() {
        let names: Vec<&str> = te.draft.fonts.keys().map(|s| s.as_str()).collect();
        ui.label(RichText::new(format!("Embedded in the file: {}", names.join(", "))).size(11.0).weak());
    }

    section(ui, "Background");
    ui.horizontal(|ui| {
        ui.label("Picture");
        if ui.button(if te.draft.background_image.is_some() { "Replace…" } else { "Choose…" }).clicked() {
            if let Some(key) = pick_image(app) {
                te.draft.background_image = Some(key);
                changed = true;
            }
        }
        if te.draft.background_image.is_some() && ui.button("Remove").clicked() {
            te.draft.background_image = None;
            changed = true;
        }
    });

    ui.add_space(4.0);
    ui.label(RichText::new("3D background").font(style::bold(12.0)))
        .on_hover_text("Up to three pictures behind the canvas that move slower than it as you zoom and pan, for depth.");
    let mut remove = None;
    for (i, l) in te.draft.layers.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("Layer {}", i + 1));
            let r = ui.add(egui::Slider::new(&mut l.depth, 0.0..=1.0).show_value(false).text("depth")).on_hover_text("Left: far away, barely moves. Right: moves with the canvas.");
            changed |= r.drag_stopped() || (r.changed() && !r.dragged());
            let r = ui.add(egui::Slider::new(&mut l.opacity, 0.1..=1.0).show_value(false).text("opacity"));
            changed |= r.drag_stopped() || (r.changed() && !r.dragged());
            if ui.small_button("✕").on_hover_text("Remove layer").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        te.draft.layers.remove(i);
        changed = true;
    }
    if te.draft.layers.len() < MAX_LAYERS && ui.button("Add Layer…").clicked() {
        if let Some(asset) = pick_image(app) {
            // New layers come nearer than the ones behind them.
            let depth = 0.15 + 0.25 * te.draft.layers.len() as f64;
            te.draft.layers.push(BackgroundLayer { asset, depth, opacity: 1.0 });
            changed = true;
        }
    }

    changed
}

/// Save as Theme… and Done, under the scrolling part.
fn footer(app: &mut ZebraApp, ui: &mut Ui, te: &mut ThemeEditor, close: &mut bool) {
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Save as Theme…").on_hover_text("Keep this theme for other presentations").clicked() {
            te.status = Some(match crate::library::themes_dir().map(|d| crate::library::save_theme(&app.editor.doc, &te.draft, &d)) {
                Some(Ok(p)) => format!("Saved “{}” to My Themes.", p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()),
                Some(Err(e)) => format!("Couldn't save the theme: {}", e.message),
                None => "Couldn't find the themes folder.".into(),
            });
        }
        if ui.button("Done").clicked() {
            *close = true;
        }
    });
    if let Some(s) = &te.status {
        ui.label(RichText::new(s).size(11.0).weak());
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).font(style::bold(12.0)));
    ui.add_space(2.0);
}

/// Ask for a picture and store it as a document asset; returns its key.
fn pick_image(app: &mut ZebraApp) -> Option<String> {
    let p = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "webp"]).pick_file()?;
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) => {
            app.toast(format!("Couldn't read {}: {e}", p.display()));
            return None;
        }
    };
    let r = app.editor.add_asset(bytes);
    app.report(r)
}

/// "My Themes" entries for the theme menu: apply a saved theme file.
pub fn apply_library_theme(app: &mut ZebraApp, from: &zebra_core::Document, t: &Theme) {
    app.editor.import_assets(from, t.asset_keys());
    app.apply(Command::SaveTheme { theme: t.clone(), apply: true });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        for h in ["#102030", "#ffffff", "#00000080"] {
            assert_eq!(c32_to_hex(hex_to_c32(h)), h);
        }
    }
}
