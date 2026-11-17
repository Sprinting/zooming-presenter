//! "File ▸ New from Sample": a small presentation built entirely through
//! zebra-core commands (exactly what an agent could do), plus debug demos.
use zebra_core::{
    Command, Document, Editor, ElementId, ElementKind, FrameData, FrameStyle, NewElement, Origin, ShapeData, ShapeKind,
    StepTarget, TextData, TextStyle, ViewCommand,
};

use crate::app::ZebraApp;

const H: Origin = Origin::Human;

fn text(ed: &mut Editor, s: &str, x: f64, y: f64, style: TextStyle, scale: f64, rot: f64, bullets: bool) -> ElementId {
    let el = NewElement {
        id: None,
        x,
        y,
        scale,
        rotation: rot,
        w: None,
        h: None,
        name: String::new(),
        kind: ElementKind::Text(TextData { text: s.into(), style, bullets, ..Default::default() }),
    };
    ed.apply(Command::AddElement { element: el }, H).expect("sample text").created[0].clone()
}

fn frame(ed: &mut Editor, style: FrameStyle, name: &str, x: f64, y: f64, w: f64, h: f64, rot: f64) -> ElementId {
    let el = NewElement {
        id: None,
        x,
        y,
        scale: 1.0,
        rotation: rot,
        w: Some(w),
        h: Some(h),
        name: name.into(),
        kind: ElementKind::Frame(FrameData { style, color: None }),
    };
    ed.apply(Command::AddElement { element: el }, H).expect("sample frame").created[0].clone()
}

fn shape(ed: &mut Editor, shape: ShapeKind, x: f64, y: f64, w: f64, h: f64, rot: f64, fill: Option<&str>, stroke: Option<&str>) -> ElementId {
    let el = NewElement {
        id: None,
        x,
        y,
        scale: 1.0,
        rotation: rot,
        w: Some(w),
        h: Some(h),
        name: String::new(),
        kind: ElementKind::Shape(ShapeData { shape, fill: fill.map(Into::into), stroke: stroke.map(Into::into), stroke_width: 6.0, ..Default::default() }),
    };
    ed.apply(Command::AddElement { element: el }, H).expect("sample shape").created[0].clone()
}

pub fn build(renderer: &zebra_render::Renderer) -> Document {
    let mut ed = Editor::default();
    ed.measure = renderer.measure();
    ed.apply(Command::SetTitle { title: "Thinking in Zoom".into() }, H).unwrap();

    // Overview title
    text(&mut ed, "Thinking in Zoom", 0.0, -560.0, TextStyle::Title, 2.2, 0.0, false);
    text(&mut ed, "a tiny tour of a zooming canvas", 0.0, -400.0, TextStyle::Subtitle, 1.6, 0.0, false);

    // 1. Bracket frame with fade-in bullets
    let f1 = frame(&mut ed, FrameStyle::Bracket, "Why zoom?", -1150.0, 250.0, 900.0, 620.0, 0.0);
    let t1 = text(&mut ed, "Why zoom?", -1150.0, 40.0, TextStyle::Title, 1.0, 0.0, false);
    let b1 = text(&mut ed, "Show the big picture first", -1150.0, 200.0, TextStyle::Body, 1.4, 0.0, true);
    let b2 = text(&mut ed, "Then dive into the details", -1150.0, 290.0, TextStyle::Body, 1.4, 0.0, true);
    let b3 = text(&mut ed, "Zoom out again to connect them", -1150.0, 380.0, TextStyle::Body, 1.4, 0.0, true);

    // Title and bullets share one left edge (critique 28).
    ed.apply(Command::Align { ids: vec![t1, b1.clone(), b2.clone(), b3.clone()], edge: zebra_core::command::AlignEdge::Left }, H).unwrap();

    // 2. Circle frame with a tiny hidden detail
    let f2 = frame(&mut ed, FrameStyle::Circle, "Circles", 0.0, 300.0, 720.0, 720.0, 0.0);
    text(&mut ed, "Frames come in four styles", 0.0, 170.0, TextStyle::Subtitle, 1.0, 0.0, false);
    text(&mut ed, "brackets · circles · rectangles · invisible", 0.0, 260.0, TextStyle::Body, 1.0, 0.0, false);
    shape(&mut ed, ShapeKind::Ellipse, 0.0, 450.0, 64.0, 64.0, 0.0, None, Some("theme:accent"));
    let detail = frame(&mut ed, FrameStyle::Invisible, "A secret", 0.0, 450.0, 26.0, 17.0, 0.0);
    text(&mut ed, "Look closer…", 0.0, 400.0, TextStyle::Body, 0.6, 0.0, false);
    text(&mut ed, "hidden in plain sight!", 0.0, 448.0, TextStyle::Body, 0.05, 0.0, false);

    // 3. Rotated rectangle frame
    let f3 = frame(&mut ed, FrameStyle::Rectangle, "A new angle", 1150.0, 260.0, 860.0, 560.0, -12.0);
    text(&mut ed, "A new angle", 1150.0 + 13.0, 120.0, TextStyle::Title, 0.9, -12.0, false);
    text(&mut ed, "Rotate a frame and the camera\nrolls to meet it when presenting.", 1150.0 - 10.0, 330.0, TextStyle::Body, 1.2, -12.0, false);

    // Arrows linking the ideas
    shape(&mut ed, ShapeKind::Arrow, -560.0, 280.0, 140.0, 40.0, 0.0, None, Some("theme:shape2"));
    shape(&mut ed, ShapeKind::Arrow, 560.0, 280.0, 140.0, 40.0, -6.0, None, Some("theme:shape2"));

    // Path: overview → 1 (+3 fades) → circle → detail → rotated → overview
    let step = |target| Command::AddStep { target, index: None, builds: vec![], notes: String::new() };
    ed.apply(step(StepTarget::Overview), H).unwrap();
    ed.apply(Command::AddStep { target: StepTarget::Element { id: f1 }, index: None, builds: vec![b1, b2, b3], notes: "Click to reveal each point.".into() }, H).unwrap();
    ed.apply(step(StepTarget::Element { id: f2 }), H).unwrap();
    ed.apply(step(StepTarget::Element { id: detail }), H).unwrap();
    ed.apply(step(StepTarget::Element { id: f3 }), H).unwrap();
    ed.apply(step(StepTarget::Overview), H).unwrap();
    ed.doc
}

/// Debug demo states for screenshots: sample, pathedit, present, agent, text.
pub fn demo(app: &mut ZebraApp, ctx: &egui::Context, which: &str) {
    let doc = build(&app.renderer);
    app.editor.load(doc, None);
    app.after_load();
    for part in which.split(',') {
        match part.trim() {
            "sample" => {}
            // Debug: every chrome icon, large and at toolbar size.
            "icons" => app.show_icon_sheet = true,
            // Debug: the Settings window, a sheet, the inspector on text.
            "settings" => app.show_settings = true,
            "sheet" => crate::sheets::request(app, crate::sheets::SheetKind::ClearPath),
            "dark" => ctx.set_theme(egui::Theme::Dark),
            // Debug: media deck (picture, cropped circle, playing video),
            // optionally in crop mode.
            "media" | "mediacrop" | "mediaplay" => media_demo(app, part.trim()),
            // Debug: AI popover open (off), or with AI control switched on.
            "aipanel" => app.open_ai_panel = true,
            "aion" => {
                // Debug demo, but still only after a person has switched AI
                // control on once (see discovery::human_has_granted).
                if zebra_api::discovery::human_has_granted() {
                    app.set_ai_control(true);
                }
                app.open_ai_panel = true;
            }
            "select" => {
                let found = app.editor.doc.ordered().find(|e| e.name == "A new angle").map(|e| e.id.clone());
                if let Some(id) = found {
                    app.select(vec![id]);
                }
            }
            "pathedit" => {
                let _ = app.editor.view(ViewCommand::SetMode { mode: zebra_core::Mode::PathEdit }, H);
                app.selected_step = Some(1);
            }
            "present" => {
                let _ = app.editor.view(ViewCommand::Present { step: Some(1) }, H);
                let _ = app.editor.view(ViewCommand::Next, H);
                // Keep the step counter visible for screenshots.
                app.last_pointer_move = 1e9;
            }
            "detail" => {
                let _ = app.editor.view(ViewCommand::Present { step: Some(3) }, H);
            }
            "agent" => {
                let agent = Origin::agent("Claude");
                app.demo_agent = Some("Claude".into());
                let _ = app.editor.apply(Command::AddText { text: "Added by the agent".into(), x: 0.0, y: -250.0, style: TextStyle::Subtitle, scale: Some(1.2), place: None, id: None, markdown: false, on: None }, agent.clone());
                let found = app.editor.doc.ordered().find(|e| e.name == "Why zoom?").map(|e| e.id.clone());
                if let Some(c) = found {
                    let _ = app.editor.apply(Command::Update { id: c.clone(), patch: zebra_core::ElementPatch { frame_style: Some(FrameStyle::Bracket), ..Default::default() } }, agent.clone());
                    let _ = app.editor.view(ViewCommand::Select { ids: vec![c] }, agent.clone());
                }
                app.show_activity = true;
            }
            "text" => {
                let p = app.canvas.rect.center();
                crate::canvas::begin_new_text(app, p);
                if let Some(t) = &mut app.canvas.text {
                    t.buf = "Hello, canvas".into();
                }
            }
            "richtext" => {
                let p = app.canvas.rect.center();
                crate::canvas::begin_new_text(app, p);
                if let Some(t) = &mut app.canvas.text {
                    t.rich = zebra_core::rich::from_markdown("Why **zebras** have *stripes*\n\n- flies get [confused](https://example.com)\n  - really ~~maybe~~\n1. first\n2. second");
                    t.buf = t.rich.text.clone();
                }
            }
            "shortcuts" => app.show_shortcuts = true,
            "library" | "iconlib" => {
                app.library.toggle();
                if part.trim() == "iconlib" {
                    app.library.tab = crate::shape_library::Tab::Icons;
                    app.library.query = "chart".into();
                }
            }
            "connectors" => {
                app.editor.load(Document::default(), None);
                app.after_load();
                let add = |app: &mut ZebraApp, j: serde_json::Value| {
                    let _ = app.editor.apply(serde_json::from_value(j).unwrap(), H);
                };
                add(app, serde_json::json!({"op":"add_shape","id":"idea","shape":"rounded_rect","x":-420,"y":-120,"w":220,"h":120,"fill":"theme:shape1"}));
                add(app, serde_json::json!({"op":"add_shape","id":"test","shape":"diamond","x":0,"y":-120,"w":200,"h":150,"fill":"theme:shape2"}));
                add(app, serde_json::json!({"op":"add_shape","id":"ship","shape":"ellipse","x":420,"y":-120,"w":180,"h":180,"fill":"theme:shape3"}));
                add(app, serde_json::json!({"op":"add_shape","id":"note","shape":"sticky_note","x":0,"y":230,"w":200,"h":200}));
                add(app, serde_json::json!({"op":"add_connector","from":{"element":"idea"},"to":{"element":"test"},"label":"try it"}));
                add(app, serde_json::json!({"op":"add_connector","from":{"element":"test"},"to":{"element":"ship"},"route":"curved","end":"open"}));
                add(app, serde_json::json!({"op":"add_connector","from":{"element":"test"},"to":{"element":"note"},"route":"elbow","start":"dot","dash":"dashed"}));
                add(app, serde_json::json!({"op":"add_connector","from":{"element":"note"},"to":{"element":"idea"},"route":"elbow","width":4}));
                let _ = app.editor.view(ViewCommand::FitAll { animate: false }, H);
                let id = app.editor.doc.ordered().find(|e| matches!(e.kind, ElementKind::Connector(_))).map(|e| e.id.clone());
                if let Some(id) = id {
                    app.select(vec![id]);
                }
            }
            "whiteboard" => {
                // Debug: sticky notes and shapes with text typed on them.
                app.editor.load(Document::default(), None);
                app.after_load();
                let add = |app: &mut ZebraApp, j: serde_json::Value| {
                    let _ = app.editor.apply(serde_json::from_value(j).unwrap(), H);
                };
                let notes = [("n1", -330.0, -170.0, "Interview five teachers", "#ffe98a"), ("n2", -10.0, -190.0, "Prototype the quiz flow", "#b8f0c8"), ("n3", -340.0, 160.0, "Price under $5 a month?", "#ffc4d6"), ("n4", 0.0, 140.0, "Ship the beta on Friday", "#bfe0ff")];
                for (id, x, y, text, fill) in notes {
                    add(app, serde_json::json!({"op":"add_shape","id":id,"shape":"sticky_note","x":x,"y":y,"w":260,"h":260,"fill":fill}));
                    add(app, serde_json::json!({"op":"add_text","text":text,"on":id}));
                }
                add(app, serde_json::json!({"op":"add_shape","id":"goal","shape":"rounded_rect","x":420,"y":-20,"w":300,"h":160,"fill":"theme:shape1"}));
                add(app, serde_json::json!({"op":"add_text","text":"**Launch** in March","markdown":true,"on":"goal"}));
                add(app, serde_json::json!({"op":"add_connector","from":{"element":"n4"},"to":{"element":"goal"},"route":"curved"}));
                let _ = app.editor.view(ViewCommand::FitAll { animate: false }, H);
                app.select(vec!["n2".into()]);
            }
            "shapes" => {
                // Gallery of every shape plus a row of icons.
                app.editor.load(Document::default(), None);
                app.after_load();
                let fills = ["theme:shape1", "theme:shape2", "theme:shape3", "theme:shape4"];
                for (i, &k) in ShapeKind::ALL.iter().enumerate() {
                    let (col, row) = ((i % 8) as f64, (i / 8) as f64);
                    let (w, h) = if k.is_line() { (150.0, 20.0) } else { (150.0, 110.0) };
                    let _ = app.editor.apply(
                        Command::AddShape { shape: k, x: col * 190.0, y: row * 160.0, w, h, rotation: 0.0, fill: Some(fills[i % 4].into()).filter(|_| !k.is_line()), stroke: None, id: None },
                        H,
                    );
                }
                for (i, n) in ["lightbulb", "rocket", "chart-line", "users", "target", "calendar", "map-pin", "heart-handshake"].iter().enumerate() {
                    let _ = app.editor.apply(Command::AddIcon { name: n.to_string(), x: i as f64 * 190.0, y: 5.0 * 160.0, size: 100.0, color: None, id: None }, H);
                }
                let _ = app.editor.view(ViewCommand::FitAll { animate: false }, H);
            }
            "palette" => app.palette = Some(("fra".into(), 0)),
            "seltext" => {
                let found = app.editor.doc.ordered().find(|e| matches!(&e.kind, ElementKind::Text(t) if t.text == "Why zoom?")).map(|e| e.id.clone());
                if let Some(id) = found {
                    app.select(vec![id]);
                }
            }
            "edittext" => {
                let found = app.editor.doc.ordered().find(|e| matches!(&e.kind, ElementKind::Text(t) if t.text == "Why zoom?")).map(|e| e.id.clone());
                if let Some(id) = found {
                    crate::canvas::begin_edit_existing(app, id);
                }
            }
            t if t.starts_with("theme=") => {
                let _ = app.editor.apply(Command::SetTheme { theme: t[6..].into() }, H);
            }
            // Stream 4 demos: template chooser, layout gallery, theme editor,
            // presenter view (embedded so a screenshot shows it), 3D background.
            "welcome" => app.welcome = Some(crate::welcome::Welcome::new()),
            "layouts" => app.show_layouts = true,
            "themeeditor" => app.theme_ed = Some(crate::theme_editor::ThemeEditor::open(app)),
            "playback" => app.stage.show_playback = true,
            t if t.starts_with("template=") => crate::welcome::new_from_template(app, &t[9..]),
            t if t.starts_with("layout=") => {
                app.editor.view.camera = app.editor.fit_all_camera();
                crate::welcome::insert_layout(app, &t[7..]);
                let ids = app.selection();
                let b = app.editor.bounds_of(&ids);
                app.editor.view.camera = zebra_core::Camera::fit_rect(&b, app.editor.view.viewport, 0.1);
            }
            "notes" => {
                let notes = "Open with the question: why do slides feel flat?\n\nPause. Let them answer.\n\nThen zoom in: one idea leads into the next.";
                let _ = app.editor.apply(Command::UpdateStep { index: 1, target: None, builds: None, notes: Some(notes.into()), duration: None, dwell: None, hide_builds: None }, H);
                app.selected_step = Some(1);
            }
            "presenter" => {
                ctx.set_embed_viewports(true);
                app.stage.presenter_view = true;
                let _ = app.editor.apply(Command::SetPlayback { autoplay: Some(10.0), looping: Some(true) }, H);
                let _ = app.editor.view(ViewCommand::Present { step: Some(1) }, H);
                app.last_pointer_move = 1e9;
            }
            "parallax" => demo_parallax(app),
            t if t.starts_with("zoom=") => {
                if let Ok(k) = t[5..].parse::<f64>() {
                    let _ = app.editor.view(ViewCommand::Zoom { factor: k, anchor: None }, H);
                    app.editor.view.flight = None;
                }
            }
            "empty" => {
                app.editor.load(Document::default(), None);
                app.after_load();
            }
            _ => {}
        }
    }
}

/// Debug demo: a custom theme with a two-layer 3D background made of
/// generated pictures (a starry sky far away, hills nearer).
fn demo_parallax(app: &mut ZebraApp) {
    let png = |img: image::RgbaImage| {
        let mut b = std::io::Cursor::new(vec![]);
        img.write_to(&mut b, image::ImageFormat::Png).ok();
        b.into_inner()
    };
    let (w, h) = (640u32, 400u32);
    let sky = image::RgbaImage::from_fn(w, h, |x, y| {
        let t = y as f32 / h as f32;
        let star = (x.wrapping_mul(7919) ^ y.wrapping_mul(104_729)) % 211 == 0;
        if star {
            image::Rgba([255, 255, 255, 255])
        } else {
            image::Rgba([(20.0 + 40.0 * t) as u8, (30.0 + 50.0 * t) as u8, (70.0 + 90.0 * t) as u8, 255])
        }
    });
    let hills = image::RgbaImage::from_fn(w, h, |x, y| {
        let fx = x as f32 / w as f32 * std::f32::consts::TAU;
        let top = h as f32 * (0.62 + 0.08 * (fx * 1.5).sin() + 0.04 * (fx * 4.0).cos());
        if (y as f32) > top { image::Rgba([40, 90, 70, 230]) } else { image::Rgba([0, 0, 0, 0]) }
    });
    let (Some(a), Some(b)) = (app.editor.add_asset(png(sky)).ok(), app.editor.add_asset(png(hills)).ok()) else { return };
    let mut t = zebra_core::Theme::resolve("night");
    t.id = "night-3d".into();
    t.name = "Night 3D".into();
    t.layers = vec![
        zebra_core::BackgroundLayer { asset: a, depth: 0.1, opacity: 1.0 },
        zebra_core::BackgroundLayer { asset: b, depth: 0.45, opacity: 1.0 },
    ];
    let _ = app.editor.apply(Command::SaveTheme { theme: t, apply: true }, H);
}

// ----------------------------------------------------------------------
// Media demo deck (screenshots and manual testing)

fn demo_picture(w: u32, h: u32, hue: f32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        // Sky to sea, with a sun.
        let sun = ((fx - 0.7).powi(2) + (fy - 0.3).powi(2)).sqrt() < 0.12;
        if sun {
            return image::Rgba([255, 214, 120, 255]);
        }
        let horizon = fy > 0.55 + 0.04 * (fx * 12.0).sin();
        let (r, g, b) = if horizon { (20.0 + 40.0 * fy, 70.0 + 60.0 * fy, 120.0 + 60.0 * fy) } else { (250.0 - 120.0 * fy, 150.0 + 60.0 * fy * hue, 120.0 + 120.0 * fy) };
        image::Rgba([r as u8, g as u8, b as u8, 255])
    });
    let mut v = vec![];
    let _ = img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png);
    v
}

pub fn media_demo(app: &mut ZebraApp, which: &str) {
    use base64::Engine;
    use zebra_core::Origin;
    let h = Origin::Human;
    app.editor.load(zebra_core::Document::default(), None);
    app.after_load();
    let ed = &mut app.editor;
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let cmd = |j: serde_json::Value| -> Command { serde_json::from_value(j).expect("demo command") };
    let _ = ed.apply(cmd(serde_json::json!({"op":"add_frame","x":0,"y":0,"w":2400,"h":1300,"style":"bracket","name":"Media","add_to_path":true})), h.clone());
    let _ = ed.apply(cmd(serde_json::json!({"op":"add_text","text":"Pictures, video and sound","x":0,"y":-520,"style":"title","scale":1.6})), h.clone());
    let pic = demo_picture(1200, 800, 1.0);
    let a = ed.apply(cmd(serde_json::json!({"op":"add_image","data_base64":b64(&pic),"x":-760,"y":80,"width":640,"alt":"Sunset"})), h.clone()).ok().and_then(|o| o.created.first().cloned());
    let c = ed.apply(cmd(serde_json::json!({"op":"add_image","data_base64":b64(&demo_picture(800, 800, 0.4)),"x":0,"y":80,"width":520,"alt":"Sun"})), h.clone()).ok().and_then(|o| o.created.first().cloned());
    if let Some(c) = &c {
        let _ = ed.apply(cmd(serde_json::json!({"op":"update","id":c,"patch":{"crop":{"x":0.45,"y":0.1,"w":0.5,"h":0.5},"mask":"circle"}})), h.clone());
        let _ = ed.apply(cmd(serde_json::json!({"op":"update","id":c,"patch":{"x":0,"y":80}})), h.clone());
    }
    // A video file (header only; the fake backend plays it).
    let dir = zebra_core::media::cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("demo-clip.mp4");
    let _ = std::fs::write(&path, zebra_core::media::fixtures::tiny_mp4(1280, 720, 20, true));
    let poster = app.media.backend.poster_png(&path, 2.0, 640).ok();
    let v = app
        .editor
        .apply(
            Command::AddVideo {
                data_base64: String::new(),
                path: Some(path.to_string_lossy().to_string()),
                x: 760.0,
                y: 80.0,
                width: Some(640.0),
                alt: "Product demo".into(),
                poster_base64: poster.map(|p| b64(&p)),
                autoplay: true,
                looping: true,
                muted: false,
                id: None,
            },
            h.clone(),
        )
        .ok()
        .and_then(|o| o.created.first().cloned());
    let _ = app.editor.view(zebra_core::ViewCommand::FitAll { animate: false }, h.clone());
    match which {
        "mediacrop" => {
            if let Some(a) = a {
                let _ = app.editor.apply(cmd(serde_json::json!({"op":"update","id":a,"patch":{"crop":{"x":0.2,"y":0.1,"w":0.6,"h":0.7}}})), h.clone());
                crate::media_ui::begin_crop(app, &a);
            }
        }
        "mediaplay" => {
            if let Some(v) = v {
                app.select(vec![v.clone()]);
                crate::media_ui::play(app, &v);
            }
        }
        _ => {}
    }
}
