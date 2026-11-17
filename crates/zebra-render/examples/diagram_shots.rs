//! Renders every diagram (Insert › Diagram) to PNG, for checking by eye:
//!   cargo run --release -p zebra-render --example diagram_shots -- <out dir> [theme]
use zebra_core::{Command, Document, Editor, Origin};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().unwrap_or_else(|| "out".into()));
    let theme = args.next();
    std::fs::create_dir_all(&out).unwrap();
    let r = zebra_render::Renderer::new();
    for d in zebra_core::diagrams::ALL {
        let mut ed = Editor::new(Document::default());
        ed.measure = r.measure();
        if let Some(t) = &theme {
            ed.apply(Command::SetTheme { theme: t.clone() }, Origin::Human).unwrap();
        }
        ed.apply(Command::AddDiagram { diagram: d.key.into(), x: 0.0, y: 0.0, width: Some(1600.0) }, Origin::Human).unwrap();
        for e in ed.doc.elements.values_mut() {
            e.placeholder = false;
        }
        ed.view.viewport = zebra_core::Vec2::new(960.0, 600.0);
        let png = r.capture(&ed, &zebra_render::CaptureTarget::Overview, 960, 600).unwrap();
        std::fs::write(out.join(format!("{}.png", d.key)), png).unwrap();
    }
}
