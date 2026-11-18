//! The template chooser shown at launch (and from File ▸ New from
//! Template…), and the layout gallery (Insert ▸ Layout…).
//!
//! Thumbnails are real renders of each template and layout, made once on
//! a worker thread with the CPU renderer.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

use egui::{Align2, Color32, ColorImage, FontId, Rect, RichText, Sense, TextureHandle, TextureOptions, Ui, Vec2 as EVec2};
use zebra_core::templates::{LAYOUTS, TEMPLATES};
use zebra_core::{Command, Document, Editor, Origin};

use crate::actions::Action;
use crate::app::ZebraApp;
use crate::style::{self, Palette, ACCENT};

const H: Origin = Origin::Human;
const TW: u32 = 320;
const TH: u32 = 200;

/// Rendered previews, keyed by "t:<template>" or "l:<layout>".
pub struct Previews {
    rx: Receiver<(String, ColorImage)>,
    pub textures: HashMap<String, TextureHandle>,
    ctx: egui::Context,
}

impl Previews {
    pub fn start(ctx: &egui::Context, renderer: std::sync::Arc<zebra_render::Renderer>) -> Self {
        let (tx, rx) = channel();
        let wake = ctx.clone();
        std::thread::Builder::new()
            .name("zebra-previews".into())
            .spawn(move || {
                let jobs = TEMPLATES
                    .iter()
                    .map(|t| (format!("t:{}", t.id), Command::ApplyTemplate { template: t.id.clone(), replace: false }))
                    .chain(LAYOUTS.iter().map(|l| (format!("l:{}", l.id), Command::AddLayout { layout: l.id.clone(), x: 0.0, y: 0.0, width: Some(1600.0), add_to_path: false })))
                    .chain(zebra_core::diagrams::ALL.iter().map(|d| (format!("d:{}", d.key), Command::AddDiagram { diagram: d.key.to_string(), x: 0.0, y: 0.0, width: Some(1600.0) })));
                for (key, cmd) in jobs {
                    let mut ed = Editor::new(Document::default());
                    ed.measure = renderer.measure();
                    if ed.apply(cmd, H).is_err() {
                        continue;
                    }
                    // Show placeholder text at full strength in the preview.
                    for e in ed.doc.elements.values_mut() {
                        e.placeholder = false;
                    }
                    ed.view.viewport = zebra_core::Vec2::new(TW as f64, TH as f64);
                    let Ok(png) = renderer.capture(&ed, &zebra_render::CaptureTarget::Overview, TW, TH) else { continue };
                    let Ok(img) = image::load_from_memory(&png) else { continue };
                    let im = img.to_rgba8();
                    let ci = ColorImage::from_rgba_unmultiplied([im.width() as usize, im.height() as usize], im.as_raw());
                    if tx.send((key, ci)).is_err() {
                        break;
                    }
                    wake.request_repaint();
                }
            })
            .expect("spawn preview thread");
        Previews { rx, textures: HashMap::new(), ctx: ctx.clone() }
    }

    fn poll(&mut self) {
        while let Ok((k, img)) = self.rx.try_recv() {
            let t = self.ctx.load_texture(format!("preview-{k}"), img, TextureOptions::LINEAR);
            self.textures.insert(k, t);
        }
    }
}

/// State of the template chooser.
pub struct Welcome {
    selected: Selection,
    mine: Vec<(PathBuf, String)>,
}

#[derive(Clone, PartialEq)]
enum Selection {
    Template(&'static str),
    Mine(PathBuf),
}

impl Welcome {
    pub fn new() -> Self {
        Welcome { selected: Selection::Template("big_picture"), mine: crate::library::list_templates() }
    }
}

impl Default for Welcome {
    fn default() -> Self {
        Self::new()
    }
}

fn previews(app: &mut ZebraApp, ctx: &egui::Context) {
    if app.previews.is_none() {
        app.previews = Some(Previews::start(ctx, app.renderer.clone()));
    }
    if let Some(p) = &mut app.previews {
        p.poll();
    }
}

fn card(ui: &mut Ui, tex: Option<&TextureHandle>, name: &str, selected: bool, p: &Palette) -> egui::Response {
    let w = 150.0;
    let (r, resp) = ui.allocate_exact_size(EVec2::new(w, w / 1.6 + 26.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, name));
    let img = Rect::from_min_size(r.min, EVec2::new(w, w / 1.6));
    let painter = ui.painter();
    match tex {
        Some(t) => {
            painter.image(t.id(), img, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            painter.rect_filled(img, 4.0, p.hover);
        }
    }
    let stroke = if selected { egui::Stroke::new(3.0, ACCENT) } else { egui::Stroke::new(1.0, p.border) };
    painter.rect_stroke(img, 4.0, stroke, egui::StrokeKind::Outside);
    painter.text(egui::pos2(img.center().x, img.bottom() + 13.0), Align2::CENTER_CENTER, name, FontId::proportional(12.5), if selected { ACCENT } else { p.text });
    resp
}

/// The template chooser (a sheet over the window).
pub fn window(app: &mut ZebraApp, ctx: &egui::Context) {
    if app.welcome.is_none() {
        return;
    }
    previews(app, ctx);
    let p = Palette::of(ctx);
    let mut choose: Option<Selection> = None;
    let mut cancel = false;
    egui::Modal::new(app.uid("welcome")).show(ctx, |ui| {
        ui.set_width(860.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(crate::t!("welcome-title")).font(style::bold(18.0)));
        });
        ui.add_space(8.0);
        ui.horizontal_top(|ui| {
            // Templates.
            ui.vertical(|ui| {
                ui.set_width(640.0);
                let welcome = app.welcome.as_mut().unwrap();
                let tex = app.previews.as_ref().map(|p| &p.textures);
                egui::Grid::new("templates").spacing([10.0, 10.0]).show(ui, |ui| {
                    for (i, t) in TEMPLATES.iter().enumerate() {
                        let sel = Selection::Template(t.id.as_str());
                        let r = card(ui, tex.and_then(|m| m.get(&format!("t:{}", t.id))), &crate::words::template_name(t), welcome.selected == sel, &p).on_hover_text(crate::words::template_description(t));
                        if r.clicked() {
                            welcome.selected = sel.clone();
                        }
                        if r.double_clicked() {
                            choose = Some(sel);
                        }
                        if i % 4 == 3 {
                            ui.end_row();
                        }
                    }
                });
                if !welcome.mine.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(crate::t!("welcome-my-templates")).font(style::bold(13.0)));
                    ui.horizontal_wrapped(|ui| {
                        for (path, name) in welcome.mine.clone() {
                            let sel = Selection::Mine(path);
                            let r = ui.selectable_label(welcome.selected == sel, &name);
                            if r.clicked() {
                                welcome.selected = sel.clone();
                            }
                            if r.double_clicked() {
                                choose = Some(sel);
                            }
                        }
                    });
                }
            });
            ui.add_space(12.0);
            // Recent files.
            ui.vertical(|ui| {
                ui.set_width(180.0);
                ui.label(RichText::new(crate::t!("welcome-recent")).font(style::bold(13.0)));
                ui.add_space(4.0);
                if app.recent.is_empty() {
                    ui.label(RichText::new(crate::t!("welcome-recent-empty")).size(12.0).color(p.weak));
                }
                for path in app.recent.clone().into_iter().take(10) {
                    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    if ui.selectable_label(false, &name).on_hover_text(path.display().to_string()).clicked() {
                        app.queued.push(Action::OpenPath(path));
                        cancel = true;
                    }
                }
            });
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button(crate::t!("welcome-open")).clicked() {
                app.queued.push(Action::Open);
                cancel = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let create = egui::Button::new(RichText::new(crate::t!("welcome-create")).color(Color32::WHITE)).fill(ACCENT).min_size(EVec2::new(84.0, 24.0));
                if ui.add(create).clicked() {
                    choose = app.welcome.as_ref().map(|w| w.selected.clone());
                }
                if ui.button(crate::t!("welcome-cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
        });
    });
    if let Some(sel) = choose {
        app.queued.push(match sel {
            Selection::Template(id) => Action::NewFromTemplate(id.to_string()),
            Selection::Mine(p) => Action::NewFromTemplateFile(p),
        });
        cancel = true;
    }
    if cancel {
        app.welcome = None;
    }
}

/// Start a new presentation from a built-in template.
pub fn new_from_template(app: &mut ZebraApp, id: &str) {
    let mut ed = Editor::new(Document::default());
    ed.measure = app.renderer.measure();
    let r = ed.apply(Command::ApplyTemplate { template: id.into(), replace: false }, H);
    if app.report(r).is_none() {
        return;
    }
    let mut doc = ed.doc;
    doc.title = TEMPLATES.iter().find(|t| t.id == id).filter(|t| t.id != "blank").map(crate::words::template_name).unwrap_or_else(|| crate::t!("app-untitled"));
    app.editor.load(doc, None);
    app.after_load();
}

/// Start a new, untitled presentation from a saved template file.
pub fn new_from_template_file(app: &mut ZebraApp, path: &std::path::Path) {
    match zebra_core::io::load(path) {
        Ok(doc) => {
            app.editor.load(doc, None);
            app.after_load();
        }
        Err(e) => app.toast(crate::t!("welcome-toast-template-open-failed", error = e.message.as_str())),
    }
}

/// File ▸ Save as Template…
pub fn save_as_template(app: &mut ZebraApp) {
    let Some(dir) = crate::library::templates_dir() else {
        app.toast(crate::t!("welcome-toast-templates-folder-missing"));
        return;
    };
    let name = app.editor.doc.title.clone();
    match crate::library::save_template(&app.editor.doc, &name, &dir) {
        Ok(p) => {
            let saved = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(name);
            app.toast(crate::t!("welcome-toast-template-saved", name = saved.as_str()))
        }
        Err(e) => app.toast(crate::t!("welcome-toast-template-save-failed", error = e.message.as_str())),
    }
}

/// The layout gallery: click a layout to insert it in the middle of the view.
pub fn layout_gallery(app: &mut ZebraApp, ctx: &egui::Context) {
    if !app.show_layouts {
        return;
    }
    previews(app, ctx);
    let p = Palette::of(ctx);
    let mut open = true;
    let mut pick: Option<&'static str> = None;
    egui::Window::new(crate::t!("layouts-title")).open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, EVec2::ZERO).show(ctx, |ui| {
        ui.label(RichText::new(crate::t!("layouts-caption")).size(12.0).color(p.weak));
        ui.add_space(6.0);
        let tex = app.previews.as_ref().map(|p| &p.textures);
        egui::Grid::new("layouts").spacing([10.0, 10.0]).show(ui, |ui| {
            for (i, l) in LAYOUTS.iter().enumerate() {
                if card(ui, tex.and_then(|m| m.get(&format!("l:{}", l.id))), &crate::words::layout_name(l), false, &p).on_hover_text(crate::words::layout_description(l)).clicked() {
                    pick = Some(l.id.as_str());
                }
                if i % 5 == 4 {
                    ui.end_row();
                }
            }
        });
    });
    if let Some(id) = pick {
        insert_layout(app, id);
        open = false;
    }
    app.show_layouts = open;
}

/// The diagram gallery (Insert › Diagram…): click one to insert it in the
/// middle of the view.
pub fn diagram_gallery(app: &mut ZebraApp, ctx: &egui::Context) {
    if !app.show_diagrams {
        return;
    }
    previews(app, ctx);
    let p = Palette::of(ctx);
    let mut open = true;
    let mut pick: Option<&'static str> = None;
    egui::Window::new(crate::t!("diagrams-title")).open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, EVec2::ZERO).show(ctx, |ui| {
        ui.label(RichText::new(crate::t!("diagrams-caption")).size(12.0).color(p.weak));
        ui.add_space(6.0);
        let tex = app.previews.as_ref().map(|p| &p.textures);
        let h = (ctx.content_rect().height() - 160.0).max(240.0);
        egui::ScrollArea::vertical().max_height(h).min_scrolled_height(h.min(600.0)).show(ui, |ui| {
            egui::Grid::new("diagrams").spacing([10.0, 10.0]).show(ui, |ui| {
                for (i, d) in zebra_core::diagrams::ALL.iter().enumerate() {
                    if card(ui, tex.and_then(|m| m.get(&format!("d:{}", d.key))), &crate::words::diagram_name(d), false, &p).on_hover_text(crate::words::diagram_description(d)).clicked() {
                        pick = Some(d.key);
                    }
                    if i % 5 == 4 {
                        ui.end_row();
                    }
                }
            });
        });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    if let Some(key) = pick {
        insert_diagram(app, key);
        open = false;
    }
    app.show_diagrams = open;
}

/// Insert a diagram centred in the view, sized to 80% of its width, and
/// select it.
pub fn insert_diagram(app: &mut ZebraApp, key: &str) {
    let c = app.canvas_center_world();
    let width = app.editor.view.viewport.x / app.editor.view.camera.zoom * 0.8;
    let r = app.editor.apply(Command::AddDiagram { diagram: key.into(), x: c.x, y: c.y, width: Some(width) }, H);
    if let Some(out) = app.report(r) {
        // A grouped diagram: select it all. A zooming one: its outer frame.
        let frames: Vec<_> = out.created.iter().filter(|i| app.editor.doc.get(i).is_some_and(|e| e.is_frame())).cloned().collect();
        match frames.first() {
            Some(f) => app.select(vec![f.clone()]),
            None => app.select(out.created.clone()),
        }
    }
}

/// Insert a layout centred in the view, sized to 80% of its width.
pub fn insert_layout(app: &mut ZebraApp, id: &str) {
    let c = app.canvas_center_world();
    let width = app.editor.view.viewport.x / app.editor.view.camera.zoom * 0.8;
    let r = app.editor.apply(Command::AddLayout { layout: id.into(), x: c.x, y: c.y, width: Some(width), add_to_path: true }, H);
    if let Some(out) = app.report(r) {
        // Select the outermost frame so it can be moved right away.
        let frames: Vec<_> = out.created.iter().filter(|i| app.editor.doc.get(i).is_some_and(|e| e.is_frame())).cloned().collect();
        if let Some(f) = frames.first() {
            app.select(vec![f.clone()]);
        }
    }
}
