//! Test-drive bug O6: an SVG file couldn't be inserted as a picture
//! ("unrecognised media format"). Keynote takes SVG pictures; so does
//! Zebra now, by path and as bytes, drawn like a symbol.

use base64::Engine;
use serde_json::json;
use zebra_core::*;

const SVG: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<!-- made by hand -->
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><rect width="200" height="100" fill="#3070c0"/></svg>"##;

fn cmd(v: serde_json::Value) -> Command {
    serde_json::from_value(v).unwrap()
}

fn picture(ed: &Editor) -> &Asset {
    let e = ed.doc.elements.values().next().expect("a picture");
    let ElementKind::Image(i) = &e.kind else { panic!("not a picture: {:?}", e.kind) };
    &ed.doc.assets[&i.asset]
}

#[test]
fn an_svg_file_comes_in_as_a_picture() {
    let dir = std::env::temp_dir().join(format!("zebra-svg-pictures-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("logo.svg");
    std::fs::write(&path, SVG).unwrap();
    let mut ed = Editor::default();
    ed.apply(cmd(json!({"op": "add_image", "path": path.to_string_lossy(), "x": 0, "y": 0})), Origin::Human).unwrap();
    let a = picture(&ed);
    assert_eq!(a.ext, "svg");
    assert_eq!(a.width, 2 * a.height, "its shape from the viewBox");
    // As bytes, with a byte-order mark.
    let mut ed = Editor::default();
    let bytes = [b"\xef\xbb\xbf".as_slice(), SVG.as_bytes()].concat();
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    ed.apply(cmd(json!({"op": "add_image", "data_base64": b64, "x": 0, "y": 0})), Origin::Human).unwrap();
    assert_eq!(picture(&ed).ext, "svg");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_files_that_are_svgs_are_taken() {
    use zebra_core::symbols::starts_as_svg;
    assert!(starts_as_svg(SVG.as_bytes()));
    assert!(starts_as_svg(b"  <svg/>"));
    assert!(starts_as_svg(b"<svg\nviewBox='0 0 1 1'></svg>"));
    // An HTML page with an SVG in it, a tag that only starts like one,
    // and text that isn't XML at all are not pictures.
    for not in [&b"<html><body><svg/></body></html>"[..], b"<svgfoo/>", b"hello <svg>", b"", b"<!-- never closed <svg>"] {
        assert!(!starts_as_svg(not), "{:?}", String::from_utf8_lossy(not));
        let b64 = base64::engine::general_purpose::STANDARD.encode(not);
        let r = Editor::default().apply(cmd(json!({"op": "add_image", "data_base64": b64, "x": 0, "y": 0})), Origin::Human);
        assert!(r.is_err(), "{:?}", String::from_utf8_lossy(not));
    }
}
