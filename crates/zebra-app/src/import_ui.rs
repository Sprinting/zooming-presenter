//! File › Import…: pick a PDF or PowerPoint file, choose the pages or
//! slides on a sheet, watch it come in (with Cancel), and read what came in
//! differently afterwards.
//!
//! The file is opened and read on a worker thread; the window keeps
//! drawing. The pages land in one undo step through
//! [`zebra_core::Editor::import_deck`].
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::{Align2, Color32, Pos2, RichText, Vec2};
use zebra_core::import::{ImportNote, ImportedDeck};
use zebra_core::{Origin, ViewCommand};
use zebra_import::{Control, ImportError, Reader};

use crate::app::ZebraApp;
use crate::style::{self, Palette};
use crate::widgets as w;

/// Where an import is: the sheet shows one of these.
pub enum Flow {
    /// Opening the file to count its pages.
    Reading { path: PathBuf, rx: Receiver<Result<Box<dyn Reader>, ImportError>> },
    /// Which pages? (`slides`: a PowerPoint file, counted in slides.)
    Choose { path: PathBuf, reader: Box<dyn Reader>, pages: usize, all: bool, from: f64, to: f64, slides: bool },
    /// Reading the pages.
    Running { path: PathBuf, rx: Receiver<Result<ImportedDeck, ImportError>>, ctl: Control, total: usize, slides: bool },
}

impl Flow {
    fn path(&self) -> &Path {
        match self {
            Flow::Reading { path, .. } | Flow::Choose { path, .. } | Flow::Running { path, .. } => path,
        }
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// File › Import…: the open panel, then the sheet.
pub fn choose_file(app: &mut ZebraApp) {
    if app.import.is_some() {
        return;
    }
    crate::pickers::ask(app, crate::pickers::Pick::Import);
}

/// Start importing `path`: count its pages off the UI thread.
pub fn start(app: &mut ZebraApp, path: PathBuf) {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = app.ctx.clone();
    let p = path.clone();
    let spawned = std::thread::Builder::new().name("zebra-import-open".into()).spawn(move || {
        let _ = tx.send(zebra_import::open(&p));
        ctx.request_repaint();
    });
    match spawned {
        Ok(_) => app.import = Some(Flow::Reading { path, rx }),
        Err(e) => failed(app, &path, ImportError::Read(e.to_string())),
    }
}

fn failed(app: &mut ZebraApp, path: &Path, e: ImportError) {
    eprintln!("zebra: import {}: {e}", path.display());
    let message = match e {
        ImportError::NotSupported => crate::t!("import-error-unsupported"),
        ImportError::Damaged("PowerPoint") => crate::t!("import-error-damaged-pptx"),
        ImportError::Damaged(_) => crate::t!("import-error-damaged"),
        ImportError::Encrypted => crate::t!("import-error-encrypted"),
        ImportError::OldPowerPoint => crate::t!("import-error-old-ppt"),
        ImportError::AllHidden => crate::t!("import-error-all-hidden"),
        ImportError::NoEngine(_) => crate::t!("import-error-no-engine"),
        ImportError::Read(_) => crate::t!("import-error-read"),
        ImportError::Cancelled => return,
    };
    crate::sheets::request(app, crate::sheets::SheetKind::Alert { title: crate::t!("import-failed-title", name = file_name(path)), message });
}

/// The lines of the summary sheet, one per kind of note.
pub fn note_lines(notes: &std::collections::BTreeMap<ImportNote, usize>) -> Vec<String> {
    notes
        .iter()
        .map(|(n, c)| {
            let c = *c as i64;
            match n {
                ImportNote::PageSkipped => crate::t!("import-note-skipped", count = c),
                ImportNote::HiddenSlide => crate::t!("import-note-hidden", count = c),
                ImportNote::Chart => crate::t!("import-note-chart", count = c),
                ImportNote::SmartArt => crate::t!("import-note-smartart", count = c),
                ImportNote::Table => crate::t!("import-note-table", count = c),
                ImportNote::Equation => crate::t!("import-note-equation", count = c),
                ImportNote::Media => crate::t!("import-note-media", count = c),
                ImportNote::AsPicture => crate::t!("import-note-as-picture", count = c),
                ImportNote::Shape => crate::t!("import-note-shape", count = c),
                ImportNote::Metafile => crate::t!("import-note-metafile", count = c),
                ImportNote::Picture => crate::t!("import-note-picture", count = c),
                ImportNote::Flip => crate::t!("import-note-flip", count = c),
                ImportNote::Gradient => crate::t!("import-note-gradient", count = c),
                ImportNote::Other => crate::t!("import-note-other", count = c),
            }
        })
        .collect()
}

/// "Imported 3 pages" / "Imported 3 slides".
pub fn done_title(deck: &ImportedDeck) -> String {
    let count = deck.pages.len() as i64;
    if deck.pages.first().is_some_and(|p| p.slide) {
        crate::t!("import-done-title-slides", count = count)
    } else {
        crate::t!("import-done-title", count = count)
    }
}

/// Put a read deck on the canvas (one undo step) and show what came in.
pub fn finish(app: &mut ZebraApp, path: &Path, deck: ImportedDeck) {
    let label = crate::t!("app-undo-import", name = file_name(path));
    let r = app.editor.import_deck(&deck, label, Origin::Human);
    let Some(out) = app.report(r) else { return };
    let frames: Vec<String> = out.created.iter().filter(|id| app.editor.doc.get(id).is_some_and(|e| e.is_frame())).cloned().collect();
    app.select(frames);
    app.view_cmd(ViewCommand::ZoomToSelection);
    let lines = note_lines(&deck.notes);
    if !lines.is_empty() {
        crate::sheets::request(
            app,
            crate::sheets::SheetKind::ImportNotes { title: done_title(&deck), message: lines.join("\n") },
        );
    }
}

/// Check the workers; called every frame while an import is under way.
fn poll(app: &mut ZebraApp) {
    let Some(flow) = app.import.take() else { return };
    app.import = match flow {
        Flow::Reading { path, rx } => match rx.try_recv() {
            Ok(Ok(reader)) => {
                let pages = reader.page_count();
                let slides = reader.kind() == "PowerPoint";
                Some(Flow::Choose { path, reader, pages, all: true, from: 1.0, to: pages as f64, slides })
            }
            Ok(Err(e)) => {
                failed(app, &path, e);
                None
            }
            Err(TryRecvError::Empty) => Some(Flow::Reading { path, rx }),
            Err(TryRecvError::Disconnected) => {
                failed(app, &path, ImportError::Read("worker stopped".into()));
                None
            }
        },
        Flow::Running { path, rx, ctl, total, slides } => match rx.try_recv() {
            Ok(Ok(deck)) => {
                finish(app, &path, deck);
                None
            }
            Ok(Err(e)) => {
                failed(app, &path, e);
                None
            }
            Err(TryRecvError::Empty) => Some(Flow::Running { path, rx, ctl, total, slides }),
            Err(TryRecvError::Disconnected) => {
                failed(app, &path, ImportError::Read("worker stopped".into()));
                None
            }
        },
        f @ Flow::Choose { .. } => Some(f),
    };
}

/// Read the chosen pages on a worker.
fn run(app: &mut ZebraApp, path: PathBuf, reader: Box<dyn Reader>, first: usize, last: usize) {
    let slides = reader.kind() == "PowerPoint";
    let mut opts = zebra_import::Options::all(app.editor.theme());
    opts.first = first;
    opts.last = last;
    opts.page_name = std::sync::Arc::new(|n| crate::t!("import-page-name", n = (n as i64)));
    opts.slide_name = std::sync::Arc::new(|n| crate::t!("import-slide-name", n = (n as i64)));
    opts.has_soundtrack = app.editor.doc.soundtrack.is_some();
    let ctl = Control::default();
    let (tx, rx) = std::sync::mpsc::channel();
    let (c, ctx) = (ctl.clone(), app.ctx.clone());
    let total = last + 1 - first;
    let spawned = std::thread::Builder::new().name("zebra-import".into()).spawn(move || {
        let _ = tx.send(reader.read(&opts, &c));
        ctx.request_repaint();
    });
    match spawned {
        Ok(_) => app.import = Some(Flow::Running { path, rx, ctl, total, slides }),
        Err(e) => failed(app, &path, ImportError::Read(e.to_string())),
    }
}

/// The import sheet, drawn in egui in the alert style of [`crate::sheets`]
/// (an NSAlert can't hold the page fields).
pub fn ui(app: &mut ZebraApp, ctx: &egui::Context) {
    poll(app);
    if app.import.is_none() {
        return;
    }
    let p = Palette::of(ctx);
    let content = ctx.content_rect();
    let top = content.top() + crate::mac::title_row(app) + crate::panels::TOOLBAR_ROW;
    let (block_id, sheet_id) = (app.uid("import-block"), app.uid("import-sheet"));
    let Some(flow) = app.import.as_mut() else { return };
    let name = file_name(flow.path());
    #[derive(PartialEq)]
    enum Do {
        Nothing,
        Import,
        Cancel,
    }
    let mut act = Do::Nothing;
    egui::Area::new(block_id).order(egui::Order::Foreground).fixed_pos(content.min).show(ctx, |ui| {
        let r = ui.allocate_rect(content, egui::Sense::click_and_drag());
        ui.painter().rect_filled(r.rect, 0.0, Color32::from_black_alpha(if p.dark { 40 } else { 14 }));
    });
    let width = 260.0;
    egui::Area::new(sheet_id)
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(content.center().x - width / 2.0 - 16.0, top + 2.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(p.panel)
                .stroke(egui::Stroke::new(0.6, if p.dark { Color32::from_gray(0x50) } else { Color32::from_black_alpha(45) }))
                .corner_radius(10)
                .shadow(ui.visuals().window_shadow)
                .inner_margin(egui::Margin { left: 16, right: 16, top: 18, bottom: 16 })
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.vertical_centered(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(54.0), egui::Sense::hover());
                        crate::canvas::draw_zebra_disc(ui.painter(), r.center(), 24.0);
                        ui.add_space(8.0);
                        let title = match flow {
                            Flow::Running { .. } => crate::t!("import-importing", name = name.as_str()),
                            _ => crate::t!("import-sheet-title", name = name.as_str()),
                        };
                        ui.add(egui::Label::new(RichText::new(title).font(style::semibold(13.0)).color(p.text)).wrap().halign(egui::Align::Center));
                        ui.add_space(4.0);
                    });
                    match flow {
                        Flow::Reading { .. } => {
                            ui.vertical_centered(|ui| {
                                ui.add(egui::Label::new(RichText::new(crate::t!("import-reading")).font(style::ui_font(11.0)).color(p.weak)));
                            });
                        }
                        Flow::Choose { pages, all, from, to, slides, .. } => {
                            let n = *pages as i64;
                            let (count, every) = if *slides {
                                (crate::t!("import-slide-count", count = n), crate::t!("import-slides-all"))
                            } else {
                                (crate::t!("import-page-count", count = n), crate::t!("import-pages-all"))
                            };
                            ui.vertical_centered(|ui| {
                                ui.add(egui::Label::new(RichText::new(count).font(style::ui_font(11.0)).color(p.text)));
                            });
                            ui.add_space(10.0);
                            if w::radio(ui, *all, &every).clicked() {
                                *all = true;
                            }
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                if w::radio(ui, !*all, &crate::t!("import-pages-from")).clicked() {
                                    *all = false;
                                }
                                let n = *pages as f64;
                                let opt = w::Num { min: 1.0, max: n, width: 62.0, ..Default::default() };
                                if w::number_field(ui, from, opt).changed() {
                                    *all = false;
                                    *to = to.max(*from);
                                }
                                ui.label(RichText::new(crate::t!("import-pages-to")).font(style::ui_font(13.0)).color(p.text));
                                if w::number_field(ui, to, opt).changed() {
                                    *all = false;
                                    *from = from.min(*to);
                                }
                            });
                        }
                        Flow::Running { ctl, total, slides, .. } => {
                            let done = ctl.done().min(*total);
                            ui.add_space(6.0);
                            let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), egui::Sense::hover());
                            ui.painter().rect_filled(r, 3.0, if p.dark { Color32::from_gray(0x48) } else { Color32::from_gray(0xdc) });
                            let f = if *total > 0 { done as f32 / *total as f32 } else { 0.0 };
                            ui.painter().rect_filled(egui::Rect::from_min_size(r.min, Vec2::new(r.width() * f, r.height())), 3.0, style::accent());
                            ui.add_space(4.0);
                            ui.vertical_centered(|ui| {
                                ui.add(egui::Label::new(RichText::new(if *slides {
                                    crate::t!("import-progress-slides", done = (done as i64), total = (*total as i64))
                                } else {
                                    crate::t!("import-progress", done = (done as i64), total = (*total as i64))
                                }).font(style::ui_font(11.0)).color(p.weak)));
                            });
                            ctx.request_repaint_after(std::time::Duration::from_millis(100));
                        }
                    }
                    ui.add_space(14.0);
                    let wdt = ui.available_width();
                    if matches!(flow, Flow::Choose { .. }) {
                        let label = crate::t!("sheet-import");
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(wdt, w::H_REGULAR + 2.0), egui::Sense::click());
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::DefaultButton, true, &label));
                        ui.painter().rect_filled(rect, w::RADIUS, style::accent());
                        ui.painter().text(rect.center(), Align2::CENTER_CENTER, &label, style::ui_font(13.0), w::on_accent());
                        if resp.clicked() {
                            act = Do::Import;
                        }
                        ui.add_space(2.0);
                    }
                    if w::push_button_ext(ui, &crate::t!("sheet-cancel"), w::Size::Regular, true, Some(wdt)).clicked() {
                        act = Do::Cancel;
                    }
                });
        });
    ctx.input(|i| {
        if i.key_pressed(egui::Key::Escape) {
            act = Do::Cancel;
        }
        if i.key_pressed(egui::Key::Enter) && act == Do::Nothing {
            act = Do::Import;
        }
    });
    match act {
        Do::Nothing => {}
        Do::Cancel => {
            if let Some(Flow::Running { ctl, .. }) = &app.import {
                // The worker stops at the next page and its answer is dropped.
                ctl.cancel();
            }
            app.import = None;
        }
        Do::Import => {
            if let Some(Flow::Choose { path, reader, pages, all, from, to, .. }) = app.import.take() {
                let (first, last) = if all { (1, pages) } else { (from.round().max(1.0) as usize, (to.round() as usize).clamp(1, pages)) };
                run(app, path, reader, first.min(last), last);
            }
        }
    }
}
