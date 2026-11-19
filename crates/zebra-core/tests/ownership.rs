//! One rule says which frame an element belongs to (`frame_holds`), and
//! every reader agrees with what moves when the person drags a frame:
//! `parent_frame`, the describe outline, the element and elements queries,
//! lint, and the spatial index.
use std::collections::BTreeSet;

use serde_json::{json, Value};
use zebra_core::*;

const H: Origin = Origin::Human;

fn cmd(j: Value) -> Command {
    serde_json::from_value(j).unwrap()
}

fn run(ed: &mut Editor, j: Value) {
    ed.apply(cmd(j), H).unwrap();
}

/// What actually moves when the person drags `frame`: apply the same move
/// a drag does, see which elements changed position, then undo.
fn moved_by_drag(ed: &mut Editor, frame: &str) -> BTreeSet<String> {
    let before: Vec<(String, Vec2)> = ed.doc.elements.values().map(|e| (e.id.clone(), Vec2::new(e.transform.x, e.transform.y))).collect();
    run(ed, json!({"op":"move","ids":[frame],"dx":37,"dy":-11}));
    let moved = before
        .iter()
        .filter(|(id, p)| id != frame && ed.doc.elements[id].transform.x != p.x)
        .map(|(id, _)| id.clone())
        .collect();
    ed.undo(H).unwrap();
    moved
}

fn set(v: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    v.into_iter().collect()
}

/// Every reader of ownership agrees with the drag, for every frame.
fn assert_agrees(ed: &mut Editor) {
    let frames: Vec<String> = ed.doc.ordered().filter(|e| e.is_frame()).map(|e| e.id.clone()).collect();
    let outline = describe::describe(ed);
    for f in &frames {
        let moved = moved_by_drag(ed, f);
        assert_eq!(set(ed.frame_contents(f)), moved, "frame_contents({f}) vs drag");
        assert_eq!(set(ed.doc.frame_contents(f)), moved, "Document::frame_contents({f}) vs drag");
        let within = ed.query(Query::Elements { kind: None, text: None, within: Some(f.clone()), visible: false }).unwrap();
        let within = within["elements"].as_array().unwrap();
        assert_eq!(set(within.iter().map(|e| e["id"].as_str().unwrap().to_string())), moved, "elements within {f} vs drag");
        let el = ed.query(Query::Element { id: f.clone() }).unwrap();
        let contents: Vec<String> = serde_json::from_value(el["contents"].clone()).unwrap();
        assert_eq!(set(contents), moved, "element {f} contents vs drag");
    }
    let all = ed.query(Query::Elements { kind: None, text: None, within: None, visible: false }).unwrap();
    for e in all["elements"].as_array().unwrap() {
        let id = e["id"].as_str().unwrap();
        assert_eq!(e["parent_frame"].as_str().map(str::to_string), ed.parent_frame(id), "elements list parent_frame of {id}");
    }
    let ids: Vec<String> = ed.doc.z_order.clone();
    for id in &ids {
        let parent = ed.parent_frame(id);
        assert_eq!(parent, ed.doc.parent_frame(id), "index vs brute force for {id}");
        assert_eq!(ed.parent_frames().get(id.as_str()).map(|s| s.to_string()), parent, "parent_frames()[{id}]");
        let el = ed.query(Query::Element { id: id.clone() }).unwrap();
        assert_eq!(el["parent_frame"].as_str().map(str::to_string), parent, "element {id} parent_frame");
        if let Some(p) = &parent {
            assert!(moved_by_drag(ed, p).contains(id), "{id} doesn't move with its parent {p}");
            // In the outline it sits one level deeper than its parent.
            let depth = |x: &str| {
                let line = outline.lines().find(|l| l.trim_start().starts_with(&format!("{x} "))).unwrap_or_else(|| panic!("{x} not in outline:\n{outline}"));
                line.len() - line.trim_start().len()
            };
            assert_eq!(depth(id), depth(p) + 2, "{id} not under {p} in the outline:\n{outline}");
        }
    }
}

fn frame(ed: &mut Editor, id: &str, x: f64, y: f64, w: f64, h: f64, rotation: f64) {
    run(ed, json!({"op":"add_frame","id":id,"x":x,"y":y,"w":w,"h":h,"rotation":rotation,"style":"rectangle"}));
}

fn shape(ed: &mut Editor, id: &str, x: f64, y: f64, w: f64, h: f64) {
    run(ed, json!({"op":"add_shape","id":id,"shape":"rectangle","x":x,"y":y,"w":w,"h":h}));
}

fn outside_frame<'a>(issues: &'a [Value], id: &str) -> Option<&'a Value> {
    issues.iter().find(|i| i["kind"] == "outside_frame" && i["id"] == id)
}

#[test]
fn straddling_element_belongs_to_neither_frame() {
    let mut ed = Editor::default();
    frame(&mut ed, "a", 0.0, 0.0, 800.0, 600.0, 0.0);
    frame(&mut ed, "b", 900.0, 0.0, 800.0, 600.0, 0.0);
    // Mostly in a (x 300..500 against a's right edge at 400), partly in the gap.
    shape(&mut ed, "s", 400.0, 0.0, 200.0, 100.0);
    // Straddles both frames: x 350..950.
    shape(&mut ed, "both", 650.0, 200.0, 600.0, 50.0);
    assert_eq!(ed.parent_frame("s"), None);
    assert_eq!(ed.parent_frame("both"), None);
    assert!(moved_by_drag(&mut ed, "a").is_empty());
    assert_agrees(&mut ed);
    // Lint names the frame it mostly sits in and says it won't move with it.
    let issues = describe::lint(&ed);
    let i = outside_frame(&issues, "s").expect("straddler not linted");
    assert_eq!(i["frame"], "a");
    assert_eq!(i["moves_with_frame"], false);
    assert!(i["message"].as_str().unwrap().contains("doesn't move with a"), "{i}");
}

#[test]
fn nested_frames() {
    let mut ed = Editor::default();
    frame(&mut ed, "big", 0.0, 0.0, 4000.0, 3000.0, 0.0);
    frame(&mut ed, "mid", 0.0, 0.0, 1600.0, 1200.0, 0.0);
    frame(&mut ed, "small", 200.0, 100.0, 400.0, 300.0, 0.0);
    shape(&mut ed, "in_small", 200.0, 100.0, 100.0, 100.0);
    shape(&mut ed, "in_mid", -500.0, -400.0, 100.0, 100.0);
    shape(&mut ed, "in_big", 1500.0, 1000.0, 100.0, 100.0);
    // Inside mid, straddling small's left edge (small spans x 0..400),
    // mostly inside small.
    shape(&mut ed, "half_small", 60.0, 100.0, 160.0, 40.0);
    assert_eq!(ed.parent_frame("in_small").as_deref(), Some("small"));
    assert_eq!(ed.parent_frame("small").as_deref(), Some("mid"));
    assert_eq!(ed.parent_frame("mid").as_deref(), Some("big"));
    assert_eq!(ed.parent_frame("in_big").as_deref(), Some("big"));
    assert_eq!(ed.parent_frame("half_small").as_deref(), Some("mid"));
    assert_eq!(moved_by_drag(&mut ed, "big").len(), 6);
    assert_agrees(&mut ed);
    // It moves with mid but not with small, which it mostly sits in.
    let issues = describe::lint(&ed);
    let i = outside_frame(&issues, "half_small").expect("straddler of a nested frame not linted");
    assert_eq!(i["frame"], "small");
    assert_eq!(i["moves_with_frame"], false);
    assert!(outside_frame(&issues, "in_small").is_none());
}

#[test]
fn rotated_frame_holds_what_its_bounding_box_holds() {
    let mut ed = Editor::default();
    frame(&mut ed, "r", 0.0, 0.0, 1000.0, 400.0, 30.0);
    let vb = ed.doc.get("r").unwrap().visual_bounds();
    // In the corner of the box around the rotated frame, outside the frame
    // itself: dragging the frame takes it along, so it belongs to it.
    shape(&mut ed, "corner", vb.x0 + 30.0, vb.y0 + 30.0, 40.0, 40.0);
    shape(&mut ed, "centre", 0.0, 0.0, 100.0, 60.0);
    shape(&mut ed, "past", vb.x1, 0.0, 100.0, 60.0);
    assert_eq!(ed.parent_frame("corner").as_deref(), Some("r"));
    assert_eq!(ed.parent_frame("centre").as_deref(), Some("r"));
    assert_eq!(ed.parent_frame("past"), None);
    assert_agrees(&mut ed);
}

#[test]
fn element_exactly_on_an_edge_is_inside() {
    let mut ed = Editor::default();
    frame(&mut ed, "f", 0.0, 0.0, 800.0, 600.0, 0.0);
    // Flush with the left edge (x -400..-200) and with top and bottom.
    shape(&mut ed, "flush", -300.0, 0.0, 200.0, 600.0);
    // One unit past the right edge.
    shape(&mut ed, "over", 351.0, 0.0, 100.0, 100.0);
    // A frame turned a quarter: its box is 600 wide and 800 tall, and its
    // corners carry rounding. A shape exactly as wide as that box.
    frame(&mut ed, "q", 3000.0, 0.0, 800.0, 600.0, 90.0);
    let vb = ed.doc.get("q").unwrap().visual_bounds();
    shape(&mut ed, "across", 3000.0, 0.0, 600.0, 100.0);
    assert!((vb.width() - 600.0).abs() < 1e-6, "{vb:?}");
    assert_eq!(ed.parent_frame("flush").as_deref(), Some("f"));
    assert_eq!(ed.parent_frame("over"), None);
    assert_eq!(ed.parent_frame("across").as_deref(), Some("q"));
    // A frame the same size as another doesn't belong to it.
    frame(&mut ed, "twin", 0.0, 0.0, 800.0, 600.0, 0.0);
    assert_eq!(ed.parent_frame("twin"), None);
    assert_eq!(ed.parent_frame("f"), None);
    assert_agrees(&mut ed);
}

#[test]
fn the_rule_itself() {
    let f = Rect { x0: 0.0, y0: 0.0, x1: 100.0, y1: 50.0 };
    assert!(frame_holds(&f, &Rect { x0: 0.0, y0: 0.0, x1: 100.0, y1: 10.0 }), "flush with three edges");
    assert!(frame_holds(&f, &Rect { x0: -1e-8, y0: 0.0, x1: 50.0, y1: 10.0 }), "rounding past an edge");
    assert!(!frame_holds(&f, &Rect { x0: -0.01, y0: 0.0, x1: 50.0, y1: 10.0 }), "really past an edge");
    assert!(!frame_holds(&f, &f), "the same box");
}
