//! Symbols (Insert › Symbols & Shapes) and diagrams (Insert › Diagram).
//! The library is a small pack written to a temporary folder; this file is
//! its own test process, so `ZEBRA_SYMBOLS_DIR` only affects it.
use std::sync::Once;

use zebra_core::symbols::library;
use zebra_core::*;

const ROCKET: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 16"><rect width="32" height="16" fill="#e33"/></svg>"##;
const BULB: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" fill="currentColor"/></svg>"#;
const PARK: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48"><rect width="48" height="48" fill="#2F88FF" stroke="#000"/></svg>"##;

fn pack() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("zebra-symbols-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Pack format 2, each SVG its own zstd frame (no dictionary here;
        // tests/symbols_pack.rs reads a pack the real builder wrote).
        let set = |key: &str, items: &[(&str, &str, &str)]| {
            let (mut dat, mut idx) = (Vec::new(), String::new());
            for (name, kw, svg) in items {
                let z = ruzstd::encoding::compress_to_vec(svg.as_bytes(), ruzstd::encoding::CompressionLevel::Fastest);
                idx += &format!("{name}\t{kw}\t{}\t{}\n", dat.len(), z.len());
                dat.extend(z);
            }
            std::fs::write(dir.join(format!("{key}.dat")), dat).unwrap();
            std::fs::write(dir.join(format!("{key}.idx")), idx).unwrap();
        };
        set("emoji", &[("rocket", "space,launch", ROCKET), ("red box", "square", ROCKET)]);
        set("glyphs", &[("lightbulb", "idea", BULB), ("rocket-launch", "space", BULB)]);
        set("park", &[("rocket", "space", PARK)]);
        let sets = serde_json::json!({"format": 2, "sets": [
            {"id":"C1","key":"emoji","name":"Emoji","group":"emoji","tint":"","author":"A","licence":"MIT","count":2},
            {"id":"C2","key":"glyphs","name":"Glyphs","group":"mono","tint":"mono","author":"B","licence":"CC BY 4.0","credit":true,"count":2},
            {"id":"C3","key":"park","name":"Park","group":"colour","tint":"two","author":"C","licence":"Apache 2.0","count":1},
        ]});
        std::fs::write(dir.join("sets.json"), sets.to_string()).unwrap();
        // SAFETY: set once, before any thread reads the library.
        unsafe { std::env::set_var("ZEBRA_SYMBOLS_DIR", &dir) };
    });
}

fn cmd(j: serde_json::Value) -> Command {
    serde_json::from_value(j).unwrap()
}

#[test]
fn the_library_searches_names_then_keywords() {
    pack();
    let lib = library().expect("library");
    assert_eq!(lib.total(), 5);
    let names: Vec<&str> = lib.search("rocket", None, 10).into_iter().map(|r| lib.name(r)).collect();
    assert_eq!(names, ["rocket", "rocket", "rocket-launch"]);
    let space: Vec<&str> = lib.search("spa", Some(&[lib.set_index("glyphs").unwrap()]), 10).into_iter().map(|r| lib.name(r)).collect();
    assert_eq!(space, ["rocket-launch"]);
    assert!(lib.search("idea bulb", None, 10).is_empty(), "every word must match");
    let r = lib.find("glyphs", "lightbulb").unwrap();
    assert_eq!(&*lib.svg(r).unwrap(), BULB.as_bytes());
    assert!(lib.cached() >= 1);
}

#[test]
fn a_symbol_is_an_svg_picture_that_keeps_its_colour_settings() {
    pack();
    let mut ed = Editor::default();
    let out = ed.apply(cmd(serde_json::json!({"op":"add_symbol","set":"emoji","name":"rocket","x":10,"y":20,"width":300})), Origin::Human).unwrap();
    let e = ed.doc.get(&out.created[0]).unwrap().clone();
    let ElementKind::Image(img) = &e.kind else { panic!("{e:?}") };
    let a = &ed.doc.assets[&img.asset];
    assert_eq!((a.ext.as_str(), a.width, a.height), ("svg", 4096, 2048));
    assert!((e.bounds().width() - 300.0).abs() < 1e-6 && (e.bounds().height() - 150.0).abs() < 1e-6);
    let look = img.symbol.clone().unwrap();
    assert_eq!((look.set.as_str(), look.name.as_str(), look.color, look.sticker), ("emoji", "rocket", None, false));

    // Full-colour sets keep their colours; one-colour sets follow the theme.
    let err = ed.apply(cmd(serde_json::json!({"op":"add_symbol","set":"emoji","name":"rocket","color":"#ff0000"})), Origin::Human).unwrap_err();
    assert!(err.message.contains("own colours"), "{}", err.message);
    let out = ed.apply(cmd(serde_json::json!({"op":"add_symbol","set":"glyphs","name":"lightbulb","sticker":true})), Origin::Human).unwrap();
    let id = out.created[0].clone();
    let look = |ed: &Editor| match &ed.doc.get(&id).unwrap().kind {
        ElementKind::Image(i) => i.symbol.clone().unwrap(),
        _ => unreachable!(),
    };
    assert_eq!((look(&ed).color.as_deref(), look(&ed).color2, look(&ed).sticker), (Some("theme:body"), None, true));
    ed.apply(Command::Update { id: id.clone(), patch: ElementPatch { color: Some("theme:accent".into()), sticker: Some(false), ..Default::default() } }, Origin::Human).unwrap();
    assert_eq!((look(&ed).color.as_deref(), look(&ed).sticker), (Some("theme:accent"), false));
    assert!(ed.apply(Command::Update { id: id.clone(), patch: ElementPatch { color2: Some("#00ff00".into()), ..Default::default() } }, Origin::Human).is_err());

    // Two-colour sets take an outline and a fill colour.
    let out = ed.apply(cmd(serde_json::json!({"op":"add_symbol","set":"park","name":"rocket"})), Origin::Human).unwrap();
    let ElementKind::Image(i) = &ed.doc.get(&out.created[0]).unwrap().kind else { unreachable!() };
    let l = i.symbol.as_ref().unwrap();
    assert_eq!((l.color.as_deref(), l.color2.as_deref()), (Some("theme:body"), Some("theme:accent")));

    // Unknown names suggest near ones.
    let err = ed.apply(cmd(serde_json::json!({"op":"add_symbol","set":"glyphs","name":"rocket"})), Origin::Human).unwrap_err();
    assert_eq!(err.code, "not_found");
}

#[test]
fn a_saved_deck_draws_its_symbols_without_the_library() {
    pack();
    let mut ed = Editor::default();
    let out = ed.apply(cmd(serde_json::json!({"op":"add_symbol","set":"glyphs","name":"lightbulb","color":"#336699"})), Origin::Human).unwrap();
    let path = std::env::temp_dir().join(format!("zebra-symbol-deck-{}.zebra", std::process::id()));
    io::write_file(&ed.doc, &path).unwrap();
    let doc = io::load(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let e = doc.get(&out.created[0]).unwrap();
    let ElementKind::Image(img) = &e.kind else { unreachable!() };
    let a = &doc.assets[&img.asset];
    assert_eq!((a.ext.as_str(), a.width, a.height), ("svg", 4096, 4096));
    assert_eq!(&a.bytes[..], BULB.as_bytes());
    assert_eq!(img.symbol.as_ref().unwrap().color.as_deref(), Some("#336699"));

    // Copied into another document, it brings its SVG along.
    let clip = clip::Clip::of(&ed, &out.created);
    let mut other = Editor::default();
    other.paste_clip(&clip, 0.0, 0.0, None, Origin::Human).unwrap();
    assert!(other.doc.assets.contains_key(&img.asset));
}

#[test]
fn every_diagram_inserts_grouped_or_on_the_path() {
    for d in diagrams::ALL {
        let mut ed = Editor::default();
        let out = ed.apply(cmd(serde_json::json!({"op":"add_diagram","diagram":d.key,"x":100,"y":50,"width":1600})), Origin::Human).unwrap();
        assert!(out.created.len() >= 4, "{}", d.key);
        let els: Vec<&Element> = out.created.iter().map(|id| ed.doc.get(id).unwrap()).collect();
        let b = ed.bounds_of(&out.created);
        assert!(b.width() <= 1600.0 * 1.02 && b.height() <= 1600.0 * 0.72, "{} is {}x{}", d.key, b.width(), b.height());
        assert!(els.iter().any(|e| e.placeholder), "{} has placeholder text", d.key);
        if d.path {
            assert!(!ed.doc.path.is_empty(), "{}", d.key);
            assert!(els.iter().all(|e| e.group.is_none()));
        } else {
            assert!(ed.doc.path.is_empty(), "{}", d.key);
            let g = els[0].group.clone();
            assert!(g.is_some() && els.iter().all(|e| e.group == g), "{} is one group", d.key);
        }
        // One undo removes it.
        ed.undo(Origin::Human).unwrap();
        assert!(ed.doc.elements.is_empty() && ed.doc.path.is_empty(), "{}", d.key);
    }
    let err = Editor::default().apply(cmd(serde_json::json!({"op":"add_diagram","diagram":"nope"})), Origin::Human).unwrap_err();
    assert!(err.valid_values.iter().any(|v| v == "process"));
}

#[test]
fn agents_can_list_symbols_and_diagrams() {
    pack();
    let ed = Editor::default();
    let sets = describe::query(&ed, cmd_q(serde_json::json!({"q":"symbols"}))).unwrap();
    assert_eq!(sets["sets"].as_array().unwrap().len(), 3);
    assert_eq!(sets["sets"][1]["colour"], "one");
    let hits = describe::query(&ed, cmd_q(serde_json::json!({"q":"symbols","query":"rocket","set":"park"}))).unwrap();
    assert_eq!(hits["symbols"][0]["name"], "rocket");
    let d = describe::query(&ed, cmd_q(serde_json::json!({"q":"diagrams"}))).unwrap();
    assert_eq!(d.as_array().unwrap().len(), diagrams::ALL.len());
}

fn cmd_q(j: serde_json::Value) -> Query {
    serde_json::from_value(j).unwrap()
}

/// R13: replacing a symbol's picture leaves an ordinary picture, not one
/// still drawn with the symbol's tint and sticker settings.
#[test]
fn replacing_a_symbols_picture_drops_the_symbol_look() {
    pack();
    let mut ed = Editor::default();
    let out = ed.apply(cmd(serde_json::json!({"op":"add_symbol","set":"glyphs","name":"lightbulb","color":"#336699"})), Origin::Human).unwrap();
    let id = out.created[0].clone();
    let png = {
        use base64::Engine;
        let img = image::RgbImage::from_fn(8, 8, |x, y| image::Rgb([x as u8 * 30, y as u8 * 30, 1]));
        let mut out = std::io::Cursor::new(vec![]);
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        base64::engine::general_purpose::STANDARD.encode(out.into_inner())
    };
    ed.apply(cmd(serde_json::json!({"op":"replace_image","id":id,"data_base64":png})), Origin::Human).unwrap();
    let ElementKind::Image(img) = &ed.doc.get(&id).unwrap().kind else { panic!() };
    assert!(img.symbol.is_none());
}
