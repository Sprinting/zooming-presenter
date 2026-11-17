//! The layout lint an agent runs before handing a deck back.
use serde_json::{json, Value};
use zebra_core::command::*;
use zebra_core::*;

fn cmd(j: Value) -> Command {
    serde_json::from_value(j).unwrap()
}

fn run(ed: &mut Editor, j: Value) {
    ed.apply(cmd(j), Origin::agent("test")).unwrap();
}

/// One frame on the path, with its centre at (cx, cy).
fn deck() -> (Editor, f64, f64) {
    let mut ed = Editor::default();
    run(&mut ed, json!({"op":"add_frame","id":"f1","x":0,"y":0,"w":1000,"h":600,"add_to_path":true}));
    let b = ed.doc.get("f1").unwrap().bounds();
    let (cx, cy) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
    (ed, cx, cy)
}

fn lint(ed: &Editor) -> Vec<Value> {
    zebra_core::describe::lint(ed)
}

fn kinds(issues: &[Value], id: &str) -> Vec<String> {
    issues
        .iter()
        .filter(|i| i["id"] == id || i["ids"].as_array().is_some_and(|a| a.iter().any(|x| x == id)))
        .map(|i| i["kind"].as_str().unwrap().to_string())
        .collect()
}

/// Move `id` so its right edge sits `by` past frame f1's right edge.
fn push_right(ed: &mut Editor, id: &str, by: f64) {
    let fx1 = ed.doc.get("f1").unwrap().bounds().x1;
    let x1 = ed.doc.get(id).unwrap().bounds().x1;
    run(ed, json!({"op":"move","ids":[id],"dx": fx1 - x1 + by,"dy":0}));
}

#[test]
fn a_shape_hanging_off_its_frame_is_flagged() {
    let (mut ed, cx, cy) = deck();
    run(&mut ed, json!({"op":"add_shape","id":"s1","shape":"rounded_rect","x":cx,"y":cy,"w":250,"h":130}));
    assert!(kinds(&lint(&ed), "s1").is_empty(), "inside its frame: no issue");
    push_right(&mut ed, "s1", 45.0);
    let issues = lint(&ed);
    let i = issues.iter().find(|i| i["kind"] == "outside_frame" && i["id"] == "s1").expect("shape off its frame is flagged");
    assert_eq!(i["frame"], "f1");
    let m = i["message"].as_str().unwrap();
    assert!(m.contains("45 on the right"), "{m}");
}

#[test]
fn text_spilling_its_frame_says_so_not_that_it_is_off_the_path() {
    let (mut ed, cx, cy) = deck();
    run(&mut ed, json!({"op":"add_text","id":"t1","text":"A subtitle that is a little too wide","x":cx,"y":cy,"style":"body"}));
    push_right(&mut ed, "t1", 25.0);
    let k = kinds(&lint(&ed), "t1");
    assert!(k.contains(&"outside_frame".to_string()), "{k:?}");
    assert!(!k.contains(&"not_on_path".to_string()), "it is on the path (f1 shows it): {k:?}");
}

#[test]
fn text_revealed_as_a_build_is_on_the_path() {
    let (mut ed, cx, cy) = deck();
    // Far outside every frame, but step 1 reveals it as a build.
    run(&mut ed, json!({"op":"add_text","id":"t1","text":"Aside","x":cx + 5000.0,"y":cy,"style":"body"}));
    assert!(kinds(&lint(&ed), "t1").contains(&"not_on_path".to_string()));
    run(&mut ed, json!({"op":"update_step","index":0,"builds":["t1"]}));
    let k = kinds(&lint(&ed), "t1");
    assert!(!k.contains(&"not_on_path".to_string()), "{k:?}");
}

#[test]
fn a_label_on_its_shape_is_not_an_overlap_until_it_spills() {
    let (mut ed, cx, cy) = deck();
    run(&mut ed, json!({"op":"add_shape","id":"s1","shape":"rounded_rect","x":cx,"y":cy,"w":300,"h":120}));
    run(&mut ed, json!({"op":"add_text","id":"t1","text":"Label","x":cx,"y":cy,"style":"body"}));
    // Centre the label on the shape whatever x/y mean for each kind.
    let (s, t) = (ed.doc.get("s1").unwrap().bounds(), ed.doc.get("t1").unwrap().bounds());
    run(&mut ed, json!({"op":"move","ids":["t1"],"dx": (s.x0 + s.x1 - t.x0 - t.x1) / 2.0,"dy": (s.y0 + s.y1 - t.y0 - t.y1) / 2.0}));
    assert!(ed.doc.get("s1").unwrap().bounds().contains_rect(&ed.doc.get("t1").unwrap().bounds()));
    assert!(kinds(&lint(&ed), "t1").is_empty(), "{:?}", lint(&ed));
    // Text grows past the shape's edge: say that, not "overlap 100%".
    run(&mut ed, json!({"op":"update","id":"t1","patch":{"text":"A label far too long for its box"}}));
    let k = kinds(&lint(&ed), "t1");
    assert!(k.contains(&"label_overflow".to_string()) && !k.contains(&"overlap".to_string()), "{k:?}");
}

#[test]
fn two_shapes_on_top_of_each_other_still_overlap() {
    let (mut ed, cx, cy) = deck();
    run(&mut ed, json!({"op":"add_shape","id":"s1","shape":"rectangle","x":cx,"y":cy,"w":200,"h":120}));
    run(&mut ed, json!({"op":"add_shape","id":"s2","shape":"rectangle","x":cx,"y":cy,"w":200,"h":120}));
    assert!(kinds(&lint(&ed), "s1").contains(&"overlap".to_string()));
}
