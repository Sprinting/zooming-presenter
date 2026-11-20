//! The theme editor (Prezi Classic's "Customize current theme"): colours,
//! fonts (any installed font, embedded in the file), a background picture
//! and a 3D background of up to three layers. Changes show live on the
//! canvas; each burst of edits is one undo step.
use egui::{Color32, RichText, Ui};
use zebra_core::theme::{BackgroundLayer, MAX_LAYERS};
use zebra_core::{Command, Origin, Theme};

use crate::app::ZebraApp;
use crate::style;
use crate::widgets as w;

const H: Origin = Origin::Human;
/// Seconds without a change before a burst of edits becomes one undo step.
const COMMIT_AFTER: f64 = 0.8;
/// Which styles get a Weight pop-up next to their font: Title, Subtitle,
/// Body. (Kartik hasn't decided whether Subtitle and Body get one.)
const WEIGHT_POPUP: [bool; 3] = [true, true, true];
/// The Weight pop-up's width.
const WEIGHT_W: f32 = 104.0;

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
        .default_width(WIDTH)
        .default_pos(ctx.content_rect().right_top() + egui::vec2(-WIDTH - 40.0, 70.0))
        .show(ctx, |ui| {
            // A fixed width: the rows fill it, so an auto-sizing window
            // grew a little every frame.
            ui.set_width(WIDTH);
            egui::ScrollArea::vertical().auto_shrink([false, true]).max_height((ctx.content_rect().height() - 200.0).min(560.0)).show(ui, |ui| {
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

/// Content width of the theme editor.
const WIDTH: f32 = 440.0;

/// Returns true when the draft changed.
///
/// Laid out like the inspector: 11 pt labels right-aligned in one column,
/// the kit's controls after them (text field, pop-up buttons, colour
/// wells, number fields with steppers). It used to mix egui combo boxes,
/// drag values and buttons, with each row's controls at a different x.
fn editor_ui(app: &mut ZebraApp, ui: &mut Ui, te: &mut ThemeEditor) -> bool {
    let mut changed = false;
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
    w::row(ui, &crate::t!("theme-editor-name"), |ui| {
        let wdt = ui.available_width();
        changed |= w::text_field(ui, &mut te.draft.name, wdt, "").lost_focus();
    });
    w::row(ui, &crate::t!("theme-editor-start-from"), |ui| {
        let wdt = ui.available_width();
        w::popup_button(ui, "start-from", &crate::t!("theme-editor-choose-theme"), wdt, |ui| {
            let all: Vec<Theme> = Theme::builtins().into_iter().chain(app.editor.doc.themes.iter().cloned()).collect();
            for t in all {
                if t.id != te.draft.id && w::menu_item(ui, &crate::words::theme_name(&t), None).clicked() {
                    let (id, name) = (te.draft.id.clone(), te.draft.name.clone());
                    te.draft = t;
                    te.draft.id = id;
                    te.draft.name = name;
                    changed = true;
                }
            }
        });
    });

    let _ = section(ui, &crate::t!("theme-editor-section-colours"));
    {
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
        // Two label/well pairs to a line; each label in its own column.
        let mut it = rows.into_iter().enumerate().peekable();
        while it.peek().is_some() {
            ui.horizontal(|ui| {
                for _ in 0..2 {
                    let Some((i, (label, hex))) = it.next() else { break };
                    let p = style::Palette::of(ui.ctx());
                    let (r, _) = ui.allocate_exact_size(egui::vec2(w::LABEL_W, w::H_REGULAR), egui::Sense::hover());
                    ui.painter().text(egui::pos2(r.right() - 2.0, r.center().y), egui::Align2::RIGHT_CENTER, label, style::ui_font(11.0), p.weak);
                    if let Some(c) = w::color_well(ui, ("theme-colour", i), hex_to_c32(hex)) {
                        *hex = c32_to_hex(c);
                        changed = true;
                    }
                }
            });
        }
    }

    let _ = section(ui, &crate::t!("theme-editor-section-fonts"));
    let families = te.families.clone();
    let font_tip = crate::t!("theme-editor-font-tip");
    for (i, label) in [crate::t!("theme-editor-font-title"), crate::t!("theme-editor-font-subtitle"), crate::t!("theme-editor-font-body")].into_iter().enumerate() {
        let st = match i {
            0 => &mut te.draft.title,
            1 => &mut te.draft.subtitle,
            _ => &mut te.draft.body,
        };
        let query = &mut te.font_query[i];
        // Label, Font pop-up, Weight pop-up, size: one row per style.
        w::row(ui, &label, |ui| {
            let shown = match st.font.as_str() {
                "Sans" => crate::t!("theme-editor-font-sans-shown"),
                "Serif" => crate::t!("theme-editor-font-serif-shown"),
                "Mono" => crate::t!("theme-editor-font-mono-shown"),
                "Hand" => crate::t!("theme-editor-font-hand-shown"),
                f => f.to_string(),
            };
            // Room for the Weight pop-up, the size field, its stepper and
            // the gaps: a row that overflowed widened every row after it.
            let weight_w = if WEIGHT_POPUP[i] { WEIGHT_W + 6.0 } else { 0.0 };
            let wdt = (ui.available_width() - 112.0 - weight_w).max(100.0);
            w::popup_button_named(ui, ("font", i), Some(&label), &shown, wdt, |ui| {
                ui.add_space(2.0);
                let sw = ui.available_width().max(200.0);
                w::search_field(ui, query, sw, &crate::t!("theme-editor-search-fonts"));
                ui.add_space(4.0);
                let q = query.to_lowercase();
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for f in families.iter().filter(|f| q.is_empty() || f.to_lowercase().contains(&q)).take(400) {
                        if w::menu_check(ui, st.font == *f, &font_label(f), None).clicked() && st.font != *f {
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
            })
            .on_hover_text(&font_tip);
            if WEIGHT_POPUP[i] {
                let faces = app.font_faces(&st.font);
                let shown = crate::words::style_face_name(&faces, st.weight);
                w::popup_button_named(ui, ("weight", i), Some(&crate::t!("theme-editor-weight")), &shown, WEIGHT_W, |ui| {
                    for f in &faces {
                        if w::menu_check(ui, st.weight == f.weight, &crate::words::face_name(f), None).clicked() && st.weight != f.weight {
                            st.weight = f.weight;
                            changed = true;
                        }
                    }
                })
                .on_hover_text(crate::t!("theme-editor-weight"));
            }
            let mut size = st.size;
            if w::number_field(ui, &mut size, w::Num::pt().range(8.0, 400.0).step(1.0).decimals(1).width(78.0)).changed() {
                st.size = size;
                changed = true;
            }
        });
    }
    if !te.draft.fonts.is_empty() {
        let names: Vec<&str> = te.draft.fonts.keys().map(|s| s.as_str()).collect();
        w::caption(ui, crate::t!("theme-editor-embedded-fonts", fonts = names.join(", ")));
    }

    let _ = section(ui, &crate::t!("theme-editor-section-background"));
    w::row(ui, &crate::t!("theme-editor-picture"), |ui| {
        if w::push_button(ui, &if te.draft.background_image.is_some() { crate::t!("theme-editor-replace") } else { crate::t!("theme-editor-choose") }).clicked() {
            if let Some(key) = pick_image(app) {
                te.draft.background_image = Some(key);
                changed = true;
            }
        }
        if te.draft.background_image.is_some() && w::push_button(ui, &crate::t!("theme-editor-remove")).clicked() {
            te.draft.background_image = None;
            changed = true;
        }
    });

    section(ui, &crate::t!("theme-editor-3d-background")).on_hover_text(crate::t!("theme-editor-3d-background-tip"));
    let mut remove = None;
    for (i, l) in te.draft.layers.iter_mut().enumerate() {
        w::row(ui, &crate::t!("theme-editor-layer", number = (i + 1)), |ui| {
            // An icon, not the ✕ glyph: the UI font has no ✕ and drew a box.
            let tip = crate::t!("theme-editor-remove-layer");
            if w::icon_button(ui, crate::icons::Icon::Close, 20.0, true, false, &tip).on_hover_text(&tip).clicked() {
                remove = Some(i);
            }
        });
        let (depth, opacity) = (crate::t!("theme-editor-depth"), crate::t!("theme-editor-opacity"));
        w::row(ui, &depth, |ui| {
            let r = w::slider(ui, &mut l.depth, 0.0..=1.0, 180.0, &depth).on_hover_text(crate::t!("theme-editor-depth-tip"));
            changed |= r.drag_stopped() || (r.changed() && !r.dragged());
        });
        w::row(ui, &opacity, |ui| {
            let r = w::slider(ui, &mut l.opacity, 0.1..=1.0, 180.0, &opacity);
            changed |= r.drag_stopped() || (r.changed() && !r.dragged());
        });
    }
    if let Some(i) = remove {
        te.draft.layers.remove(i);
        changed = true;
    }
    if te.draft.layers.len() < MAX_LAYERS {
        w::row(ui, "", |ui| {
            if w::push_button(ui, &crate::t!("theme-editor-add-layer")).clicked() {
                if let Some(asset) = pick_image(app) {
                    // New layers come nearer than the ones behind them.
                    let depth = 0.15 + 0.25 * te.draft.layers.len() as f64;
                    te.draft.layers.push(BackgroundLayer { asset, depth, opacity: 1.0 });
                    changed = true;
                }
            }
        });
    }

    changed
}

/// Save as Theme… and Done, under the scrolling part: a Mac dialog button
/// row, the default button (Done) rightmost, 12 pt between the buttons.
fn footer(app: &mut ZebraApp, ui: &mut Ui, te: &mut ThemeEditor, close: &mut bool) {
    ui.add_space(8.0);
    w::separator(ui);
    ui.add_space(12.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        if w::default_button_w(ui, &crate::t!("theme-editor-done"), 80.0).clicked() {
            *close = true;
        }
        if w::push_button(ui, &crate::t!("theme-editor-save-as-theme")).on_hover_text(crate::t!("theme-editor-save-as-theme-tip")).clicked() {
            te.status = Some(match crate::library::themes_dir().map(|d| crate::library::save_theme(&app.editor.doc, &te.draft, &d)) {
                Some(Ok(p)) => crate::t!("theme-editor-saved", name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()),
                Some(Err(e)) => crate::t!("theme-editor-save-failed", error = e.message.as_str()),
                None => crate::t!("theme-editor-no-themes-folder"),
            });
        }
    });
    if let Some(s) = &te.status {
        w::caption(ui, s.clone());
    }
}

/// A group heading: 13 pt semibold, as in the inspector.
fn section(ui: &mut Ui, title: &str) -> egui::Response {
    let p = style::Palette::of(ui.ctx());
    ui.add_space(10.0);
    let r = ui.label(RichText::new(title).font(style::semibold(13.0)).color(p.text));
    ui.add_space(2.0);
    r
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
