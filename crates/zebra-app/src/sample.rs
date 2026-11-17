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
        kind: ElementKind::Shape(ShapeData { shape, fill: fill.map(Into::into), stroke: stroke.map(Into::into), stroke_width: 6.0 }),
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
    text(&mut ed, "Why zoom?", -1150.0, 40.0, TextStyle::Title, 1.0, 0.0, false);
    let b1 = text(&mut ed, "Show the big picture first", -1150.0, 200.0, TextStyle::Body, 1.4, 0.0, true);
    let b2 = text(&mut ed, "Then dive into the details", -1150.0, 290.0, TextStyle::Body, 1.4, 0.0, true);
    let b3 = text(&mut ed, "Zoom out again to connect them", -1150.0, 380.0, TextStyle::Body, 1.4, 0.0, true);

    ed.apply(Command::Align { ids: vec![b1.clone(), b2.clone(), b3.clone()], edge: zebra_core::command::AlignEdge::Left }, H).unwrap();

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
            // Debug: AI popover open (off), or with AI control switched on.
            "aipanel" => app.open_ai_panel = true,
            "aion" => {
                app.set_ai_control(true);
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
                let _ = app.editor.apply(Command::AddText { text: "Added by the agent".into(), x: 0.0, y: -250.0, style: TextStyle::Subtitle, scale: Some(1.2), place: None, id: None }, agent.clone());
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
            "shortcuts" => app.show_shortcuts = true,
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
            "dark" => ctx.set_theme(egui::ThemePreference::Dark),
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
                let _ = app.editor.apply(Command::UpdateStep { index: 1, target: None, builds: None, notes: Some(notes.into()), duration: None, dwell: None }, H);
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
