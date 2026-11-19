//! Screen atlas: drives the real app (egui_kittest with the wgpu renderer,
//! so the canvas is drawn by vello as in the app) through its menus,
//! toolbar menus, windows, inspector tabs and modes, and writes one PNG per
//! state plus `pages.tsv` (file, section, caption).
//!
//! cargo run -p zebra-app --example screen_atlas -- <out dir>
use std::path::PathBuf;
use std::time::Duration;

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use zebra_app::actions::Action;
use zebra_app::{AppOptions, ZebraApp};
use zebra_core::{Command, Document, Editor, Origin, Tool, ViewCommand};

type H = Harness<'static, ZebraApp>;
/// A scenario can correct its caption to what is really on screen.
static CAPTION: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
const HUMAN: Origin = Origin::Human;

struct Atlas {
    dir: PathBuf,
    rows: Vec<String>,
    section: String,
    seen: std::collections::HashSet<u64>,
}

fn harness(dark: bool) -> H {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(0.05)
        .with_os(egui::os::OperatingSystem::Mac)
        .with_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light })
        .allow_missing_glyphs()
        .with_max_steps(400)
        .wgpu()
        .build_eframe(move |cc| {
            cc.egui_ctx.set_embed_viewports(true);
            ZebraApp::new(&cc.egui_ctx, cc.wgpu_render_state.clone(), None, AppOptions::default())
        });
    h.ctx.set_embed_viewports(true);
    if dark {
        h.ctx.set_theme(egui::Theme::Dark);
    }
    // A launch offers the template chooser; the pages that show it open it
    // themselves.
    h.state_mut().welcome = None;
    settle(&mut h, 4);
    h
}

fn settle(h: &mut H, n: usize) {
    for _ in 0..n {
        h.step();
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Let real time pass (camera flights run on the wall clock).
fn wait(h: &mut H, secs: f32) {
    let n = (secs / 0.05) as usize;
    for _ in 0..n {
        h.step();
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn double_click(h: &mut H, p: Pos2) {
    h.event(Event::PointerMoved(p));
    h.step();
    for _ in 0..2 {
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.step();
    }
    settle(h, 6);
}

/// Toolbar items share names with the menu bar; the toolbar's is the last.
fn toolbar_pos(h: &H, label: &str) -> Option<Pos2> {
    h.query_all_by_label(label).map(|n| n.rect()).find(|r| r.top() > 25.0 && r.bottom() < 80.0).map(|r| r.center())
}

/// The base deck: a blank document with two framed steps, a title, body
/// text, a shape, an arrow and a symbol.
fn base_doc(app: &ZebraApp) -> Document {
    let mut ed = Editor::default();
    ed.measure = app.renderer.measure();
    let cmds = serde_json::json!([
        {"op":"add_frame","id":"one","x":0,"y":0,"w":900,"h":900,"style":"circle","name":"One","add_to_path":true},
        {"op":"add_frame","id":"two","x":1400,"y":0,"w":1100,"h":750,"style":"bracket","name":"Two","add_to_path":true},
        {"op":"add_text","id":"title","text":"Hello","style":"title","place":{"relative_to":"one","side":"inside"}},
        {"op":"add_text","id":"body","text":"A second idea\nwith two lines","style":"body","place":{"relative_to":"two","side":"inside"}},
        {"op":"add_shape","id":"box","shape":"rectangle","x":-150,"y":800,"w":420,"h":240,"fill":"theme:accent"},
        {"op":"add_shape","id":"arrow","shape":"arrow","x":650,"y":0,"w":300,"h":60,"fill":"theme:accent"},
        {"op":"add_symbol","id":"rocket","set":"noto","name":"rocket","x":1250,"y":700,"width":260}
    ]);
    for c in cmds.as_array().unwrap() {
        let c: Command = serde_json::from_value(c.clone()).unwrap();
        if let Err(e) = ed.apply(c, HUMAN) {
            eprintln!("base doc: {}", e.message);
        }
    }
    ed.doc
}

fn load_base(h: &mut H) {
    let doc = base_doc(h.state());
    let app = h.state_mut();
    app.editor.load(doc, None);
    app.after_load();
    let _ = app.editor.view(ViewCommand::FitAll { animate: false }, HUMAN);
    settle(h, 6);
}

fn fresh(dark: bool, blank: bool) -> H {
    let mut h = harness(dark);
    if blank {
        let app = h.state_mut();
        app.editor.load(Document::default(), None);
        app.after_load();
        settle(&mut h, 4);
    } else {
        load_base(&mut h);
    }
    h
}

fn perform(h: &mut H, a: Action) {
    let ctx = h.ctx.clone();
    h.state_mut().perform(&ctx, a);
    settle(h, 6);
}

fn select(h: &mut H, ids: &[&str]) {
    h.state_mut().select(ids.iter().map(|s| s.to_string()).collect());
    settle(h, 4);
}

fn click_label(h: &mut H, label: &str) -> bool {
    let found = h.query_all_by_label(label).next().map(|n| n.rect().center());
    match found {
        Some(p) => {
            click_at(h, p, PointerButton::Primary);
            true
        }
        None => {
            eprintln!("  no widget labelled {label:?}");
            false
        }
    }
}

fn hover_label(h: &mut H, label: &str) -> bool {
    let found = h.query_all_by_label(label).next().map(|n| n.rect().center());
    match found {
        Some(p) => {
            h.event(Event::PointerMoved(p));
            settle(h, 6);
            true
        }
        None => {
            eprintln!("  no widget labelled {label:?}");
            false
        }
    }
}

fn click_at(h: &mut H, p: Pos2, button: PointerButton) {
    h.event(Event::PointerMoved(p));
    h.step();
    h.event(Event::PointerButton { pos: p, button, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: p, button, pressed: false, modifiers: Modifiers::NONE });
    settle(h, 6);
}

/// Screen position of an element's centre.
fn screen_of(h: &H, id: &str) -> Pos2 {
    let app = h.state();
    let e = app.editor.doc.get(id).expect("element");
    let c = e.transform.center();
    let cam = &app.editor.view.camera;
    let r = app.canvas.rect;
    let s = cam.world_to_screen(c, zebra_core::Vec2::new(r.width() as f64, r.height() as f64));
    Pos2::new(r.min.x + s.x as f32, r.min.y + s.y as f32)
}

impl Atlas {
    fn shot(&mut self, h: &mut H, caption: &str) {
        settle(h, 8);
        let img = match h.render() {
            Ok(i) => i,
            Err(e) => {
                eprintln!("render failed for {caption}: {e}");
                return;
            }
        };
        use std::hash::{Hash, Hasher};
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        img.as_raw().hash(&mut hs);
        if !self.seen.insert(hs.finish()) {
            eprintln!("same as an earlier page, skipped: {caption}");
            return;
        }
        let name = format!("{:03}.png", self.rows.len() + 1);
        img.save(self.dir.join(&name)).unwrap();
        self.rows.push(format!("{name}\t{}\t{caption}", self.section));
        eprintln!("{name} {} / {caption}", self.section);
    }

    fn section(&mut self, s: &str) {
        self.section = s.to_string();
    }

    /// Run one scenario in its own app, surviving a panic.
    fn run(&mut self, caption: &str, dark: bool, blank: bool, f: impl FnOnce(&mut H)) {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut h = fresh(dark, blank);
            f(&mut h);
            h
        }));
        let fixed = CAPTION.lock().unwrap().take();
        match r {
            Ok(mut h) => self.shot(&mut h, fixed.as_deref().unwrap_or(caption)),
            Err(_) => eprintln!("PANIC in {caption}"),
        }
    }
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "atlas".into()));
    std::fs::create_dir_all(&dir).unwrap();
    let only = std::env::args().nth(2);
    let mut a = Atlas { dir: dir.clone(), rows: vec![], section: String::new(), seen: Default::default() };
    let want = |s: &str| only.as_deref().is_none_or(|o| s.contains(o));

    if only.as_deref() == Some("dump") {
        let h = fresh(false, false);
        fn walk(n: egui_kittest::Node<'_>, d: usize) {
            let ak = n.accesskit_node();
            let r = ak.bounding_box().map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32))).unwrap_or(egui::Rect::NOTHING);
            if r.is_finite() && r.top() < 60.0 {
                eprintln!("{:indent$}{:?} {:?} {:?} {:?}", "", n.accesskit_node().role(), n.accesskit_node().label(), n.accesskit_node().value(), r, indent = d);
            }
            for c in n.children() {
                walk(c, d + 1);
            }
        }
        walk(h.root(), 0);
        return;
    }
    if want("start") {
        a.section("Starting out");
        a.run("A new, empty document", false, true, |_| {});
        a.run("The template chooser (File › New from Template…)", false, true, |h| perform(h, Action::ShowTemplates));
        a.run("The test deck used below: two framed steps, a title, body text, a box, an arrow and a symbol", false, false, |_| {});
        a.run("The same deck in dark mode", true, false, |_| {});
        a.run("Sidebar and inspector hidden", false, false, |h| {
            perform(h, Action::ToggleSidebar);
            perform(h, Action::ToggleProperties);
        });
    }

    if want("menus") {
        a.section("Menu bar (drawn by Zebra on Linux; native menus on the Mac)");
        let menus: Vec<(String, Vec<String>)> = zebra_app::menus::spec()
            .into_iter()
            .map(|(t, items)| {
                let subs = items
                    .iter()
                    .filter_map(|i| match i {
                        zebra_app::menus::Item::Sub(s, _) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                (t, subs)
            })
            .collect();
        for (title, subs) in &menus {
            for sel in [false, true] {
                let t = title.clone();
                let cap = if sel { format!("{title} menu, with the box selected") } else { format!("{title} menu, nothing selected") };
                a.run(&cap, false, false, move |h| {
                    if sel {
                        select(h, &["box"]);
                    }
                    click_label(h, &t);
                });
            }
            for s in subs {
                let (t, s2) = (title.clone(), s.clone());
                a.run(&format!("{title} › {s}"), false, false, move |h| {
                    select(h, &["title"]);
                    click_label(h, &t);
                    hover_label(h, &s2);
                });
            }
        }
    }

    if want("toolbar") {
        a.section("Toolbar menus");
        let labels = [
            zebra_app::t!("toolbar-view"),
            zebra_app::t!("toolbar-zoom"),
            zebra_app::t!("toolbar-frame"),
            zebra_app::t!("toolbar-shape"),
            zebra_app::t!("toolbar-line"),
            zebra_app::t!("toolbar-media"),
            zebra_app::t!("toolbar-theme"),
            zebra_app::t!("toolbar-export"),
        ];
        for label in labels {
            a.run(&format!("{label} toolbar menu"), false, false, move |h| {
                let p = if label == zebra_app::t!("toolbar-zoom") { h.query_all_by_label_contains(&label).last().map(|n| n.rect().center()) } else { toolbar_pos(h, &label) };
                if let Some(p) = p {
                    click_at(h, p, PointerButton::Primary);
                }
            });
        }
        let play = zebra_app::t!("toolbar-play");
        a.run("Play button, right-click menu", false, false, move |h| {
            let p = toolbar_pos(h, &play);
            if let Some(p) = p {
                click_at(h, p, PointerButton::Secondary);
            }
        });
        let ai = zebra_app::t!("ai-toolbar");
        a.run("AI popover (AI control off)", false, false, move |h| {
            match toolbar_pos(h, &ai) {
                Some(p) => click_at(h, p, PointerButton::Primary),
                None => h.state_mut().open_ai_panel = true,
            }
        });
    }

    if want("toolbar") {
        let ai = zebra_app::t!("ai-toolbar");
        a.run("AI popover (AI control on)", false, false, move |h| {
            h.state_mut().set_ai_control(true);
            settle(h, 3);
            match toolbar_pos(h, &ai) {
                Some(p) => click_at(h, p, PointerButton::Primary),
                None => h.state_mut().open_ai_panel = true,
            }
        });
        a.run("The narrowest window (760 wide)", false, false, |h| {
            h.set_size(egui::vec2(760.0, 600.0));
            settle(h, 4);
        });
        a.run("The narrowest window (760 wide), a box selected", false, false, |h| {
            h.set_size(egui::vec2(760.0, 600.0));
            select(h, &["box"]);
            settle(h, 4);
        });
        a.run("The narrowest window (760 by 480), the template chooser", false, true, |h| {
            h.set_size(egui::vec2(760.0, 480.0));
            perform(h, Action::ShowTemplates);
        });
    }

    if want("canvas") {
        a.section("Selecting things");
        for (id, what) in [("title", "title text"), ("body", "body text"), ("box", "box"), ("arrow", "arrow"), ("rocket", "symbol"), ("one", "circle frame"), ("two", "bracket frame")] {
            let kind = {
                let h = fresh(false, false);
                h.state().editor.doc.get(id).map(|e| e.kind.clone())
            };
            for tab in zebra_app::inspector::tabs_for(kind.as_ref(), false) {
                let cap = format!("The {what} selected, inspector {} tab", tab.label());
                a.run(&cap, false, false, move |h| {
                    select(h, &[id]);
                    h.state_mut().inspector_tab = tab;
                    settle(h, 4);
                });
            }
        }
        a.run("Two things selected (box and arrow)", false, false, |h| select(h, &["box", "arrow"]));
        a.run("Everything selected (Edit › Select All)", false, false, |h| perform(h, Action::SelectAll));
        a.run("Box and arrow grouped, group selected", false, false, |h| {
            select(h, &["box", "arrow"]);
            perform(h, Action::Group);
        });
        a.run("The box locked", false, false, |h| {
            select(h, &["box"]);
            perform(h, Action::ToggleLock);
        });
        a.run("Right-click on the box", false, false, |h| {
            let p = screen_of(h, "box");
            click_at(h, p, PointerButton::Secondary);
        });
        a.run("Right-click on empty canvas", false, false, |h| {
            let r = h.state().canvas.rect;
            click_at(h, Pos2::new(r.right() - 60.0, r.bottom() - 60.0), PointerButton::Secondary);
        });
        a.run("Right-click on a step in the path sidebar", false, false, |h| {
            let r = h.state().canvas.rect;
            click_at(h, Pos2::new(r.left() - 70.0, r.top() + 60.0), PointerButton::Secondary);
        });

        a.section("Writing and drawing");
        a.run("Double-click on the canvas: a new text box", false, false, |h| {
            let r = h.state().canvas.rect;
            double_click(h, Pos2::new(r.left() + 160.0, r.bottom() - 120.0));
            h.event(Event::Text("Typing here".into()));
            settle(h, 4);
        });
        a.run("Editing the title text (double-click)", false, false, |h| {
            let p = screen_of(h, "title");
            double_click(h, p);
        });
        for (tool, what) in [(Tool::Pen, "Draw")] {
            a.run(&format!("{what} tool chosen"), false, false, move |h| perform(h, Action::SetTool(tool)));
        }
        a.run("Frame tool chosen", false, false, |h| perform(h, Action::FrameTool));
        a.run("Drawing a rectangle (mid-drag)", false, false, |h| {
            perform(h, Action::SetTool(Tool::Rectangle));
            let r = h.state().canvas.rect;
            let (p, q) = (Pos2::new(r.left() + 80.0, r.top() + 80.0), Pos2::new(r.left() + 260.0, r.top() + 200.0));
            h.event(Event::PointerMoved(p));
            h.step();
            h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            for i in 1..=8 {
                h.event(Event::PointerMoved(p + (q - p) * (i as f32 / 8.0)));
                h.step();
            }
        });

        a.section("The path");
        a.run("Edit Path mode", false, false, |h| perform(h, Action::TogglePathEdit));
        a.run("Edit Path mode, step 2 chosen", false, false, |h| {
            perform(h, Action::TogglePathEdit);
            h.state_mut().selected_step = Some(1);
            let _ = h.state_mut().editor.view(ViewCommand::GotoStep { step: 1 }, HUMAN);
        });
        a.run("Clear Path asks first", false, false, |h| perform(h, Action::ClearPath));
        a.run("An empty path", false, true, |h| {
            let _ = h.state_mut().editor.apply(serde_json::from_value(serde_json::json!({"op":"add_text","text":"Only text","x":0,"y":0,"style":"title"})).unwrap(), HUMAN);
            settle(h, 4);
        });
    }

    if want("sidebar") {
        a.section("The path sidebar (spec 08)");
        // A third step uses the first frame again: its rows light alone.
        fn three(h: &mut H) {
            let _ = h.state_mut().editor.apply(serde_json::from_value(serde_json::json!({"op":"add_step","target":{"kind":"element","id":"one"}})).unwrap(), HUMAN);
            let _ = h.state_mut().editor.view(ViewCommand::FitAll { animate: false }, HUMAN);
            settle(h, 6);
        }
        fn row(h: &H, i: usize) -> Pos2 {
            h.state().step_rows[i].center()
        }
        fn gap(h: &H, k: usize) -> Pos2 {
            let rows = &h.state().step_rows;
            Pos2::new(rows[k].min.x + h.state().step_number_w / 2.0 + 1.0, (rows[k - 1].max.y + rows[k].min.y) / 2.0)
        }
        fn empty(h: &H) -> Pos2 {
            let r = h.state().canvas.rect;
            Pos2::new(r.center().x, r.max.y - 40.0)
        }
        for dark in [false, true] {
            let mode = if dark { ", dark" } else { "" };
            a.run(&format!("Step 3 clicked: the list has the keys, its row in the accent (step 1 uses the same frame and stays plain){mode}"), dark, false, |h| {
                three(h);
                let r = row(h, 2);
                click_at(h, r, PointerButton::Primary);
                wait(h, 1.2);
            });
            a.run(&format!("Then empty canvas clicked: the keys go back to the canvas, the row turns grey{mode}"), dark, false, |h| {
                three(h);
                let r = row(h, 2);
                click_at(h, r, PointerButton::Primary);
                wait(h, 1.2);
                let _ = h.state_mut().editor.view(ViewCommand::FitAll { animate: false }, HUMAN);
                settle(h, 4);
                let e = empty(h);
                click_at(h, e, PointerButton::Primary);
            });
            a.run(&format!("Frame One selected on the canvas: both its steps show, grey (the canvas has the keys){mode}"), dark, false, |h| {
                three(h);
                select(h, &["one"]);
            });
            a.run(&format!("Hovering the gap between steps 1 and 2: an insertion line with a +{mode}"), dark, false, |h| {
                three(h);
                let g = gap(h, 1);
                h.event(Event::PointerMoved(g));
                settle(h, 4);
            });
            a.run(&format!("After the +: an empty step waits for a frame{mode}"), dark, false, |h| {
                three(h);
                let g = gap(h, 1);
                click_at(h, g, PointerButton::Primary);
                let e = empty(h);
                h.event(Event::PointerMoved(e));
                settle(h, 4);
            });
            a.run(&format!("Waiting for a frame, the pointer on frame Two: outlined, pointing hand{mode}"), dark, false, |h| {
                three(h);
                let g = gap(h, 1);
                click_at(h, g, PointerButton::Primary);
                let t = screen_of(h, "two");
                h.event(Event::PointerMoved(t));
                settle(h, 4);
            });
        }
        a.run("Right-click on a step: Add Step After This One", false, false, |h| {
            three(h);
            let r = row(h, 0);
            click_at(h, r, PointerButton::Secondary);
        });
        a.run("Steps 1 to 3 selected with ⇧-click", false, false, |h| {
            three(h);
            let r = row(h, 0);
            click_at(h, r, PointerButton::Primary);
            let r = row(h, 2);
            h.event(Event::ModifiersChanged(Modifiers::SHIFT));
            click_at(h, r, PointerButton::Primary);
            h.event(Event::ModifiersChanged(Modifiers::NONE));
            settle(h, 4);
        });
        a.run("A path of 1,200 steps, at its end: four-digit numbers fit their column", false, true, |h| {
            let cmds: Vec<Command> = (0..1200)
                .map(|i| serde_json::from_value(serde_json::json!({"op":"add_frame","x": (i % 40) as f64 * 1000.0,"y": (i / 40) as f64 * 800.0,"w":800,"h":600,"add_to_path":true})).unwrap())
                .collect();
            let _ = h.state_mut().editor.apply_batch(cmds, None, HUMAN);
            settle(h, 4);
            let ctx = h.ctx.clone();
            h.state_mut().perform(&ctx, Action::Deselect);
            zebra_app::sidebar::select_step(h.state_mut(), 1197);
            wait(h, 1.5);
        });
    }

    if want("windows") {
        a.section("Windows and panels");
        let wins: Vec<(&str, Action)> = vec![
            ("Command palette (⌘K)", Action::CommandPalette),
            ("Keyboard shortcuts", Action::ShowShortcuts),
            ("Symbols & Shapes: Shapes tab", Action::ShapeLibrary),
            ("Insert › Symbols & Shapes", Action::InsertSymbols),
            ("Insert › Layout", Action::InsertLayout),
            ("Insert › Diagram", Action::InsertDiagram),
            ("Theme editor", Action::ThemeEditor),
            ("Playback options", Action::PlaybackOptions),
            ("Agent Activity panel", Action::ToggleActivity),
            ("About Zebra", Action::About),
            ("Help › Acknowledgements", Action::Acknowledgements),
        ];
        for (cap, act) in wins {
            a.run(cap, false, false, move |h| perform(h, act));
        }
        a.run("Symbols & Shapes: Icons tab", false, false, |h| {
            perform(h, Action::ShapeLibrary);
            h.state_mut().library.tab = zebra_app::shape_library::Tab::Icons;
        });
        a.run("Symbols tab, searching \"star\"", false, false, |h| {
            perform(h, Action::InsertSymbols);
            h.state_mut().library.query = "star".into();
        });
        a.run("Command palette, typed \"fra\"", false, false, |h| {
            h.state_mut().palette = Some(("fra".into(), 0));
        });
        for pane in [zebra_app::settings::Pane::General, zebra_app::settings::Pane::Canvas, zebra_app::settings::Pane::Presenting, zebra_app::settings::Pane::Keyboard, zebra_app::settings::Pane::Agents] {
            a.run(&format!("Settings › {}", pane.label()), false, false, move |h| {
                h.state_mut().show_settings = true;
                h.state_mut().settings_pane = pane;
            });
        }
        a.run("Settings › Keyboard, recording new keys for Lock", false, false, |h| {
            let app = h.state_mut();
            app.show_settings = true;
            app.settings_pane = zebra_app::settings::Pane::Keyboard;
            app.key_capture = Some(zebra_app::keymap::Capture { action: Action::ToggleLock, got: None });
        });
        a.run("Settings in dark mode", true, false, |h| {
            h.state_mut().show_settings = true;
        });
        a.run("Insert › Diagram in dark mode", true, false, |h| perform(h, Action::InsertDiagram));
    }

    if want("big") {
        a.section("Big decks");
        a.run("Opening a big file: progress in the window", false, false, |h| {
            h.state_mut().opening = Some(zebra_app::opening::Opening::stalled(std::path::Path::new("/tmp/A rather long deck name of 200,000 elements.zebra"), 0.4));
        });
        a.run("Zoomed far out: text too small to read is drawn as faint bars; the title, still big enough, as text", false, true, |h| {
            let mut cmds = vec![serde_json::json!({"op":"add_text","text":"Quarterly review","style":"title","x":7000,"y":-1200,"width":4000})];
            for i in 0..200 {
                let (x, y) = ((i % 20) as f64 * 800.0, (i / 20) as f64 * 500.0);
                cmds.push(serde_json::json!({"op":"add_text","text":"A line of body text\nand a shorter one","style":"body","x":x,"y":y}));
            }
            let cmds: Vec<Command> = cmds.into_iter().map(|c| serde_json::from_value(c).unwrap()).collect();
            let app = h.state_mut();
            app.editor.apply_batch(cmds, None, HUMAN).unwrap();
            let _ = app.editor.view(ViewCommand::FitAll { animate: false }, HUMAN);
            settle(h, 6);
        });
    }

    if want("present") {
        a.section("Presenting");
        a.run("Playing, step 1", false, false, |h| {
            perform(h, Action::PresentStart);
            wait(h, 1.0);
            let _ = h.state_mut().editor.view(ViewCommand::GotoStep { step: 0 }, HUMAN);
            wait(h, 3.0);
        });
        a.run("Playing, step 2", false, false, |h| {
            perform(h, Action::PresentStart);
            wait(h, 1.0);
            let _ = h.state_mut().editor.view(ViewCommand::GotoStep { step: 1 }, HUMAN);
            wait(h, 3.0);
        });
        a.run("Playing, mouse moved (controls shown)", false, false, |h| {
            perform(h, Action::PresentStart);
            wait(h, 3.0);
            h.event(Event::PointerMoved(Pos2::new(700.0, 820.0)));
            settle(h, 6);
        });
        a.run("Playing, overview (Home or O)", false, false, |h| {
            perform(h, Action::PresentStart);
            let _ = h.state_mut().editor.view(ViewCommand::GotoStep { step: 1 }, HUMAN);
            wait(h, 2.0);
            let _ = h.state_mut().editor.view(ViewCommand::Overview, HUMAN);
            wait(h, 3.0);
        });
        a.run("Playing, screen blanked (B)", false, false, |h| {
            perform(h, Action::PresentStart);
            wait(h, 2.0);
            let _ = h.state_mut().editor.view(ViewCommand::Blank { blank: Some(zebra_core::view::Blank::Black) }, HUMAN);
            settle(h, 6);
        });
        a.run("Presenter view", false, false, |h| {
            perform(h, Action::PresentStart);
            perform(h, Action::TogglePresenterView);
            wait(h, 3.0);
        });
    }

    if want("more") {
        a.section("Themes");
        for t in zebra_core::Theme::ids() {
            if t == "classic" {
                continue;
            }
            let t2 = t.clone();
            a.run(&format!("Theme: {t}"), false, false, move |h| perform(h, Action::SetTheme(t2)));
        }
        a.section("Pictures, video and agents (the sample media deck)");
        for (demo, cap) in [("media", "A picture, a cropped circle picture and a video"), ("mediacrop", "Cropping a picture"), ("mediaplay", "A video playing")] {
            a.run(cap, false, false, move |h| {
                let ctx = h.ctx.clone();
                zebra_app::sample::demo(h.state_mut(), &ctx, &format!("embed,{demo}"));
                wait(h, 1.5);
            });
        }
        a.run("Rich text being edited (bold, italic, link, lists)", false, false, |h| {
            let ctx = h.ctx.clone();
            zebra_app::sample::demo(h.state_mut(), &ctx, "embed,richtext");
            settle(h, 6);
        });
        a.run("An agent's change, with the Activity panel", false, false, |h| {
            let ctx = h.ctx.clone();
            zebra_app::sample::demo(h.state_mut(), &ctx, "embed,agent");
            wait(h, 1.0);
        });
        a.run("A diagram just inserted", false, false, |h| {
            let ctx = h.ctx.clone();
            zebra_app::sample::demo(h.state_mut(), &ctx, "embed,empty,diagram=process");
            settle(h, 6);
        });
    }

    std::fs::write(dir.join("pages.tsv"), a.rows.join("\n") + "\n").unwrap();
    eprintln!("{} pages", a.rows.len());
}
