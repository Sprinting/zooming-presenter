//! Hostile-input audit C3: absurd stroke and connector widths, from a file
//! or straight into the document, render (vello_cpu aborted on a 1e19 px
//! outline and panicked in its thread pool on a 1e15 px connector).
use zebra_core::*;
use zebra_render::{CaptureTarget, Renderer};

fn open(name: &str) -> Editor {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    Editor::new(io::from_json(&std::fs::read_to_string(path).unwrap()).unwrap())
}

fn render_all(ed: &Editor) {
    let r = Renderer::new();
    r.capture(ed, &CaptureTarget::Overview, 400, 300).unwrap();
    for i in 0..ed.doc.path.len() {
        r.capture(ed, &CaptureTarget::Step { index: i, build: None }, 400, 300).unwrap();
    }
}

#[test]
fn audit_documents_open_repaired_and_render() {
    let ed = open("stroke-width-abort-doc.json");
    let ElementKind::Shape(s) = &ed.doc.elements["s1"].kind else { panic!() };
    assert!(s.stroke_width <= zebra_core::check::shape_line_limit(&ed.doc.elements["s1"].size));
    render_all(&ed);

    let ed = open("connector-width-panic-doc.json");
    let ElementKind::Connector(c) = &ed.doc.elements["c1"].kind else { panic!() };
    assert!(c.width < 1e6, "{}", c.width);
    render_all(&ed);
}

/// The renderer copes on its own, should a width get past the checks.
#[test]
fn renderer_clamps_widths_the_document_did_not() {
    let mut ed = open("connector-width-panic-doc.json");
    if let ElementKind::Connector(c) = &mut ed.doc.elements.get_mut("c1").unwrap().kind {
        c.width = 1e15;
    }
    if let ElementKind::Shape(s) = &mut ed.doc.elements.get_mut("s1").unwrap().kind {
        s.stroke_width = 1e19;
    }
    if let ElementKind::Ink(n) = &mut ed.doc.elements.get_mut("n1").unwrap().kind {
        n.width = 1e15;
    }
    render_all(&ed);
}
