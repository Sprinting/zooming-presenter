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
/// Which styles get a Weight pop-up next to their font: Title, Subtitle,
/// Body. (Kartik hasn't decided whether Subtitle and Body get one.)
const WEIGHT_POPUP: [bool; 3] = [true, true, true];

pub struct ThemeEditor {
    /// The theme being edited (a custom theme of this document).
    pub draft: Theme,
    families: Vec<String>,
    font_query: [String; 3],
    /// When the last change was applied; the open transaction commits after a pause.
    last_change: Option<f64>,
    status: Option<String>,
    /// The document state the draft was last in step with.
    seen_rev: u64,
    seen_loads: u64,
}

impl ThemeEditor {
    /// Start editing the presentation's theme. A built-in theme is copied to
    /// a new custom theme first.
    pub fn open(app: &ZebraApp) -> Self {
        let draft = Self::draft_for(app);
        let mut families = app.editor.measure.font_families();
        for b in ["Sans", "Serif", "Mono", "Hand"].iter().rev() {
            families.insert(0, b.to_string());
        }
        ThemeEditor { draft, families, font_query: Default::default(), last_change: None, status: None, seen_rev: app.editor.revision, seen_loads: app.editor.loads }
    }

    /// The theme to edit: the presentation's, as it is now.
    fn draft_for(app: &ZebraApp) -> Theme {
        let mut draft = app.editor.theme();
        if Theme::is_builtin(&draft.id) {
            let base = crate::words::theme_name(&draft);
            let mut n = 1;
            while app.editor.doc.themes.iter().any(|t| t.id == format!("custom-{n}")) {
                n += 1;
            }
            draft.base = Some(draft.id.clone());
            draft.id = format!("custom-{n}");
            draft.name = crate::t!("theme-editor-custom-name", name = base.as_str());
        }
        draft
    }

    /// The document changed under the editor (Undo, an agent's theme edit,
    /// another theme picked): show what is there now. The old draft used to
    /// be written back over it on the next change.
    fn follow(&mut self, app: &ZebraApp) {
        if app.editor.revision != self.seen_rev {
            self.draft = Self::draft_for(app);
            self.font_query = Default::default();
            self.last_change = None;
            self.seen_rev = app.editor.revision;
        }
    }
}

/// The name shown for a font family: the four built-in families are
/// translated, installed fonts keep their own names.
fn font_label(f: &str) -> String {
    match f {
        "Sans" => crate::t!("theme-editor-font-sans"),
        "Serif" => crate::t!("theme-editor-font-serif"),
        "Mono" => crate::t!("theme-editor-font-mono"),
        "Hand" => crate::t!("theme-editor-font-hand"),
        f => f.to_string(),
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
    // Another file was opened in this window: the draft belonged to the old
    // one (its fonts and pictures too), so the editor closes.
    if te.seen_loads != app.editor.loads {
        return;
    }
    te.follow(app);
    let now = app.now();
    let mut open = true;
    let mut changed = false;
    let mut close = false;
    egui::Window::new(crate::t!("theme-editor-window-title"))
        .id(app.uid("theme-editor"))
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
                app.editor.commit_as(&crate::app::HUMAN);
            }
            te.last_change = None;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
    te.seen_rev = app.editor.revision;
    if open && !close {
        app.theme_ed = Some(te);
    } else if app.editor.human_gesture_active() {
        app.editor.commit_as(&crate::app::HUMAN);
    }
}

fn apply_draft(app: &mut ZebraApp, te: &mut ThemeEditor, now: f64) {
    if !app.editor.in_transaction() {
        let _ = app.editor.begin(crate::t!("theme-editor-undo-edit"), H);
    }
    let r = app.editor.apply(Command::SaveTheme { theme: te.draft.clone(), apply: true, to_library: false }, H);
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
        ui.label(crate::t!("theme-editor-name"));
        changed |= ui.text_edit_singleline(&mut te.draft.name).lost_focus();
    });
    ui.horizontal(|ui| {
        ui.label(crate::t!("theme-editor-start-from"));
        ui.menu_button(crate::t!("theme-editor-choose-theme"), |ui| {
            let all: Vec<Theme> = Theme::builtins().into_iter().chain(app.editor.doc.themes.iter().cloned()).collect();
            for t in all {
                if t.id != te.draft.id && ui.selectable_label(false, crate::words::theme_name(&t)).clicked() {
                    let (id, name) = (te.draft.id.clone(), te.draft.name.clone());
                    te.draft = t;
                    te.draft.id = id;
                    te.draft.name = name;
                    changed = true;
                }
            }
        });
    });

    section(ui, &crate::t!("theme-editor-section-colours"));
    egui::Grid::new("theme-colours").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
        let d = &mut te.draft;
        let mut rows: Vec<(String, &mut String)> = vec![
            (crate::t!("theme-editor-colour-background"), &mut d.background),
            (crate::t!("theme-editor-colour-title"), &mut d.title.color),
            (crate::t!("theme-editor-colour-subtitle"), &mut d.subtitle.color),
            (crate::t!("theme-editor-colour-body-text"), &mut d.body.color),
            (crate::t!("theme-editor-colour-frames"), &mut d.frame),
            (crate::t!("theme-editor-colour-frame-fill"), &mut d.frame_fill),
            (crate::t!("theme-editor-colour-accent"), &mut d.accent),
        ];
        let shape_names = [crate::t!("theme-editor-colour-1"), crate::t!("theme-editor-colour-2"), crate::t!("theme-editor-colour-3"), crate::t!("theme-editor-colour-4")];
        for (i, s) in d.shapes.iter_mut().enumerate() {
            rows.push((shape_names[i].clone(), s));
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

    section(ui, &crate::t!("theme-editor-section-fonts"));
    let families = te.families.clone();
    let font_tip = crate::t!("theme-editor-font-tip");
    let labels = [crate::t!("theme-editor-font-title"), crate::t!("theme-editor-font-subtitle"), crate::t!("theme-editor-font-body")];
    // Label, Font pop-up, Weight pop-up, size: one row per style, the
    // columns lined up.
    egui::Grid::new("theme-fonts").num_columns(4).spacing([6.0, 6.0]).show(ui, |ui| {
        for (i, label) in labels.into_iter().enumerate() {
            let st = match i {
                0 => &mut te.draft.title,
                1 => &mut te.draft.subtitle,
                _ => &mut te.draft.body,
            };
            ui.label(RichText::new(label).size(12.0)).on_hover_text(&font_tip);
            let shown = match st.font.as_str() {
                "Sans" => crate::t!("theme-editor-font-sans-shown"),
                "Serif" => crate::t!("theme-editor-font-serif-shown"),
                "Mono" => crate::t!("theme-editor-font-mono-shown"),
                "Hand" => crate::t!("theme-editor-font-hand-shown"),
                f => f.to_string(),
            };
            // One width for the three, so the rows line up; a long
            // installed font's name is cut short (its full name is in the
            // list).
            egui::ComboBox::from_id_salt(("font", i)).selected_text(shown).width(196.0).truncate().height(320.0).show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut te.font_query[i]).hint_text(crate::t!("theme-editor-search-fonts")));
                let q = te.font_query[i].to_lowercase();
                for f in families.iter().filter(|f| q.is_empty() || f.to_lowercase().contains(&q)).take(400) {
                    if ui.selectable_label(st.font == *f, font_label(f)).clicked() && st.font != *f {
                        st.font = f.clone();
                        // Keep the weight if the new font has it, else
                        // take its nearest face.
                        let have: Vec<u16> = app.font_faces(f).iter().map(|x| x.weight).collect();
                        if !have.is_empty() {
                            st.weight = zebra_core::theme::nearest_weight(&have, st.weight);
                        }
                        changed = true;
                    }
                }
            });
            if WEIGHT_POPUP[i] {
                let faces = app.font_faces(&st.font);
                let shown = crate::words::style_face_name(&faces, st.weight);
                egui::ComboBox::from_id_salt(("weight", i)).selected_text(shown).width(96.0).truncate().height(320.0).show_ui(ui, |ui| {
                    for f in &faces {
                        if ui.selectable_label(st.weight == f.weight, crate::words::face_name(f)).clicked() && st.weight != f.weight {
                            st.weight = f.weight;
                            changed = true;
                        }
                    }
                })
                .response
                .on_hover_text(crate::t!("theme-editor-weight"));
            } else {
                ui.label("");
            }
            let r = ui.add(egui::DragValue::new(&mut st.size).range(8.0..=400.0).speed(0.5).suffix(format!(" {}", crate::t!("theme-editor-unit-pt"))));
            changed |= r.drag_stopped() || (r.changed() && !r.dragged());
            ui.end_row();
        }
    });
    if !te.draft.fonts.is_empty() {
        let names: Vec<&str> = te.draft.fonts.keys().map(|s| s.as_str()).collect();
        ui.label(RichText::new(crate::t!("theme-editor-embedded-fonts", fonts = names.join(", "))).size(11.0).weak());
    }

    section(ui, &crate::t!("theme-editor-section-background"));
    ui.horizontal(|ui| {
        ui.label(crate::t!("theme-editor-picture"));
        if ui.button(if te.draft.background_image.is_some() { crate::t!("theme-editor-replace") } else { crate::t!("theme-editor-choose") }).clicked() {
            if let Some(key) = pick_image(app) {
                te.draft.background_image = Some(key);
                changed = true;
            }
        }
        if te.draft.background_image.is_some() && ui.button(crate::t!("theme-editor-remove")).clicked() {
            te.draft.background_image = None;
            changed = true;
        }
    });

    ui.add_space(4.0);
    ui.label(RichText::new(crate::t!("theme-editor-3d-background")).font(style::bold(12.0)))
        .on_hover_text(crate::t!("theme-editor-3d-background-tip"));
    let mut remove = None;
    for (i, l) in te.draft.layers.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(crate::t!("theme-editor-layer", number = (i + 1)));
            let r = ui.add(egui::Slider::new(&mut l.depth, 0.0..=1.0).show_value(false).text(crate::t!("theme-editor-depth"))).on_hover_text(crate::t!("theme-editor-depth-tip"));
            changed |= r.drag_stopped() || (r.changed() && !r.dragged());
            let r = ui.add(egui::Slider::new(&mut l.opacity, 0.1..=1.0).show_value(false).text(crate::t!("theme-editor-opacity")));
            changed |= r.drag_stopped() || (r.changed() && !r.dragged());
            if ui.small_button(crate::t!("theme-editor-remove-layer-button")).on_hover_text(crate::t!("theme-editor-remove-layer")).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        te.draft.layers.remove(i);
        changed = true;
    }
    if te.draft.layers.len() < MAX_LAYERS && ui.button(crate::t!("theme-editor-add-layer")).clicked() {
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
        if ui.button(crate::t!("theme-editor-save-as-theme")).on_hover_text(crate::t!("theme-editor-save-as-theme-tip")).clicked() {
            te.status = Some(match crate::library::themes_dir().map(|d| crate::library::save_theme(&app.editor.doc, &te.draft, &d)) {
                Some(Ok(p)) => crate::t!("theme-editor-saved", name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()),
                Some(Err(e)) => crate::t!("theme-editor-save-failed", error = e.message.as_str()),
                None => crate::t!("theme-editor-no-themes-folder"),
            });
        }
        if ui.button(crate::t!("theme-editor-done")).clicked() {
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
    let p = rfd::FileDialog::new().add_filter(crate::t!("app-file-filter-image"), &["png", "jpg", "jpeg", "webp"]).pick_file()?;
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) => {
            app.toast(crate::t!("theme-editor-read-failed", path = p.display().to_string(), error = e.to_string()));
            return None;
        }
    };
    let r = app.editor.add_asset(bytes);
    app.report(r)
}

/// "My Themes" entries for the theme menu: apply a saved theme file.
pub fn apply_library_theme(app: &mut ZebraApp, from: &zebra_core::Document, t: &Theme) {
    app.editor.import_assets(from, t.asset_keys());
    app.apply(Command::SaveTheme { theme: t.clone(), apply: true, to_library: false });
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
