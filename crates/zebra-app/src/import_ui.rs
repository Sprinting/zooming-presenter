//! File › Import…: pick a PDF or PowerPoint file, choose the pages or
//! slides on a sheet, watch it come in (with Cancel), and read what came in
//! differently afterwards.
//!
//! The file is opened and read on a worker thread; the window keeps
//! drawing. The pages land in one undo step through
//! [`zebra_core::Editor::import_deck`].
use std::collections::BTreeMap;
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
    /// Opening the file to count its pages (`ctl` cancels).
    Reading { path: PathBuf, rx: Receiver<Result<Box<dyn Reader>, ImportError>>, ctl: Control },
    /// Which pages? (`slides`: a PowerPoint file, counted in slides.
    /// `many`: a PDF of more than [`zebra_import::pdf::ASK_ABOVE_PAGES`]
    /// pages, asked about with a range filled in and no "All pages".)
    Choose { path: PathBuf, reader: Box<dyn Reader>, pages: usize, all: bool, from: f64, to: f64, slides: bool, many: bool },
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
    let ctl = Control::default();
    let c = ctl.clone();
    let spawned = std::thread::Builder::new().name("zebra-import-open".into()).spawn(move || {
        let _ = tx.send(zebra_import::open_with(&p, &c));
        ctx.request_repaint();
    });
    match spawned {
        Ok(_) => app.import = Some(Flow::Reading { path, rx, ctl }),
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
        ImportError::TooBig => crate::t!("import-error-too-big"),
        ImportError::NoEngine(_) => crate::t!("import-error-no-engine"),
        ImportError::Read(_) => crate::t!("import-error-read"),
        ImportError::TooDemanding => crate::t!("import-error-too-demanding"),
        // Only the API asks for a range this way; the sheet asks itself.
        ImportError::NeedRange { .. } => crate::t!("import-error-read"),
        ImportError::Cancelled => return,
        ImportError::BetaOff => {
            crate::beta_ui::offer(app, zebra_core::beta::BetaFeature::PptxImport, crate::beta_ui::Pending::Import(path.to_path_buf()));
            return;
        }
    };
    crate::sheets::request(app, crate::sheets::SheetKind::Alert { title: crate::t!("import-failed-title", name = file_name(path)), message });
}

/// The lines of the summary sheet, one per kind of note. `slides`: the
/// slides each note is about, for the notes that name them.
pub fn note_lines(notes: &BTreeMap<ImportNote, usize>, slides: &BTreeMap<ImportNote, Vec<usize>>) -> Vec<String> {
    notes
        .iter()
        .map(|(n, c)| {
            let c = *c as i64;
            let named = slides.get(n).filter(|s| !s.is_empty()).map(|s| {
                let (groups, more) = zebra_core::import::slide_groups(s);
                let list = groups.join(", ");
                if more > 0 { crate::t!("import-note-slides-more", list = list, more = (more as i64)) } else { list }
            });
            match (n, named) {
                (ImportNote::PageSkipped, Some(s)) => crate::t!("import-note-skipped-slides", count = c, slides = s),
                (ImportNote::Simplified, s) => crate::t!("import-note-simplified", count = c, slides = s.unwrap_or_default()),
                (ImportNote::Design, _) => crate::t!("import-note-design"),
                (ImportNote::PageSkipped, None) => crate::t!("import-note-skipped", count = c),
                (ImportNote::HiddenSlide, _) => crate::t!("import-note-hidden", count = c),
                (ImportNote::Chart, _) => crate::t!("import-note-chart", count = c),
                (ImportNote::SmartArt, _) => crate::t!("import-note-smartart", count = c),
                (ImportNote::Table, _) => crate::t!("import-note-table", count = c),
                (ImportNote::ChartNearest, _) => crate::t!("import-note-chart-nearest", count = c),
                (ImportNote::TableText, _) => crate::t!("import-note-table-text", count = c),
                (ImportNote::TableStyle, _) => crate::t!("import-note-table-style", count = c),
                (ImportNote::Equation, _) => crate::t!("import-note-equation", count = c),
                (ImportNote::Media, _) => crate::t!("import-note-media", count = c),
                (ImportNote::AsPicture, _) => crate::t!("import-note-as-picture", count = c),
                (ImportNote::Shape, _) => crate::t!("import-note-shape", count = c),
                (ImportNote::Metafile, _) => crate::t!("import-note-metafile", count = c),
                (ImportNote::Picture, _) => crate::t!("import-note-picture", count = c),
                (ImportNote::Flip, _) => crate::t!("import-note-flip", count = c),
                (ImportNote::Gradient, _) => crate::t!("import-note-gradient", count = c),
                (ImportNote::Other, _) => crate::t!("import-note-other", count = c),
                (ImportNote::Placeholder, _) => crate::t!("import-note-placeholder", count = c),
                (ImportNote::Notes, Some(s)) => crate::t!("import-note-notes", count = c, slides = s),
                (ImportNote::Notes, None) => crate::t!("import-note-notes-count", count = c),
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
    app.editor.doc.warn_import(&file_name(path), &deck);
    let frames: Vec<String> = out.created.iter().filter(|id| app.editor.doc.get(id).is_some_and(|e| e.is_frame())).cloned().collect();
    app.select(frames);
    app.view_cmd(ViewCommand::ZoomToSelection);
    let lines = note_lines(&deck.notes, &deck.slides);
    if !lines.is_empty() {
        crate::sheets::request(
            app,
            crate::sheets::SheetKind::ImportNotes { title: done_title(&deck), message: lines.join("\n") },
        );
    }
}

/// The page sheet for a file of `pages` pages: every page chosen, or for a
/// big PDF the first [`zebra_import::pdf::ASK_ABOVE_PAGES`] (M1: it asks
/// before importing, the range filled in).
pub fn choose(path: PathBuf, reader: Box<dyn Reader>, pages: usize, slides: bool) -> Flow {
    let many = !slides && pages > zebra_import::pdf::ASK_ABOVE_PAGES;
    let to = if many { zebra_import::pdf::ASK_ABOVE_PAGES } else { pages };
    Flow::Choose { path, reader, pages, all: !many, from: 1.0, to: to as f64, slides, many }
}

/// The pages a sheet would import (1-based, inclusive). Any number of
/// pages can be imported at once, as Preview opens any number.
fn chosen(pages: usize, all: bool, from: f64, to: f64) -> (usize, usize) {
    let (first, last) = if all { (1, pages) } else { (from.round().max(1.0) as usize, (to.round().max(1.0) as usize).min(pages.max(1))) };
    (first.min(last), last)
}

/// `n` with its thousands grouped, as the person's numbers are written:
/// "30,000".
pub fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Check the workers; called every frame while an import is under way.
fn poll(app: &mut ZebraApp) {
    let Some(flow) = app.import.take() else { return };
    app.import = match flow {
        Flow::Reading { path, rx, ctl } => match rx.try_recv() {
            Ok(Ok(reader)) => {
                let pages = reader.page_count();
                let slides = reader.kind() == "PowerPoint";
                // Importing PowerPoint is a beta feature: offered, and
                // started again from the file on Turn On.
                if slides && app.editor.check_beta(zebra_core::beta::BetaFeature::PptxImport).is_err() {
                    crate::beta_ui::offer(app, zebra_core::beta::BetaFeature::PptxImport, crate::beta_ui::Pending::Import(path));
                    return;
                }
                Some(choose(path, reader, pages, slides))
            }
            Ok(Err(e)) => {
                failed(app, &path, e);
                None
            }
            Err(TryRecvError::Empty) => Some(Flow::Reading { path, rx, ctl }),
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
                        Flow::Choose { pages, all, from, to, slides, many, .. } => {
                            let n = *pages as i64;
                            let opt = w::Num { min: 1.0, max: *pages as f64, width: 62.0, ..Default::default() };
                            if *many {
                                // M1: "This PDF has 30,000 pages." / "Import
                                // pages [1] to [200]", the range editable.
                                ui.vertical_centered(|ui| {
                                    let line = crate::t!("import-many-pages", pages = group(*pages));
                                    ui.add(egui::Label::new(RichText::new(line).font(style::ui_font(11.0)).color(p.text)).wrap().halign(egui::Align::Center));
                                });
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(crate::t!("import-many-range")).font(style::ui_font(13.0)).color(p.text));
                                    if w::number_field(ui, from, opt).changed() {
                                        *to = to.max(*from);
                                    }
                                    ui.label(RichText::new(crate::t!("import-pages-to")).font(style::ui_font(13.0)).color(p.text));
                                    if w::number_field(ui, to, opt).changed() {
                                        *from = from.min(*to);
                                    }
                                });
                            } else {
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
            if let Some(Flow::Running { ctl, .. } | Flow::Reading { ctl, .. }) = &app.import {
                // The PDF helper is killed at once (within 25 ms), however
                // deep it is in a page; the worker's answer is dropped.
                ctl.cancel();
            }
            app.import = None;
        }
        Do::Import => {
            if let Some(Flow::Choose { path, reader, pages, all, from, to, .. }) = app.import.take() {
                let (first, last) = chosen(pages, all, from, to);
                run(app, path, reader, first, last);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All of an import's notes go in one summary; skipped slides and
    /// simplified formatting name their slides.
    #[test]
    fn the_summary_names_slides() {
        let notes = BTreeMap::from([(ImportNote::PageSkipped, 3), (ImportNote::Chart, 1), (ImportNote::Simplified, 2), (ImportNote::Design, 2)]);
        let slides = BTreeMap::from([(ImportNote::PageSkipped, vec![2, 5, 6]), (ImportNote::Simplified, vec![1, 3])]);
        assert_eq!(
            note_lines(&notes, &slides),
            [
                "Slides 2, 5\u{2013}6 couldn't be read and were left out.",
                "1 chart was left out.",
                "Some formatting on slides 1, 3 was simplified.",
                "Some slide designs couldn't be read.",
            ]
        );
        // Many: 8 groups, then how many more.
        let many: Vec<usize> = (1..=20).map(|i| i * 2).collect();
        let lines = note_lines(&BTreeMap::from([(ImportNote::PageSkipped, 20)]), &BTreeMap::from([(ImportNote::PageSkipped, many)]));
        assert_eq!(lines, ["Slides 2, 4, 6, 8, 10, 12, 14, 16 and 12 more couldn't be read and were left out."]);
        // A PDF's pages are counted, not named.
        let lines = note_lines(&BTreeMap::from([(ImportNote::PageSkipped, 2)]), &BTreeMap::new());
        assert_eq!(lines, ["2 pages couldn't be read and were left out."]);
    }
}
