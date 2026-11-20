//! One rule says which frame an element belongs to (`frame_holds`), and
//! every reader agrees with what moves when the person drags a frame:
//! `parent_frame`, the describe outline, the element and elements queries,
//! and the spatial index.
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

fn shape_kind(ed: &mut Editor, id: &str, kind: &str, x: f64, y: f64, w: f64, h: f64, rotation: f64) {
    run(ed, json!({"op":"add_shape","id":id,"shape":kind,"x":x,"y":y,"w":w,"h":h,"rotation":rotation}));
}

fn shape(ed: &mut Editor, id: &str, x: f64, y: f64, w: f64, h: f64) {
    run(ed, json!({"op":"add_shape","id":id,"shape":"rectangle","x":x,"y":y,"w":w,"h":h}));
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
}

#[test]
fn turned_frame_holds_what_lies_in_its_turned_outline() {
    let mut ed = Editor::default();
    frame(&mut ed, "r", 0.0, 0.0, 1000.0, 400.0, 30.0);
    let vb = ed.doc.get("r").unwrap().visual_bounds();
    // In the corner of the box around the turned frame, outside the frame
    // itself: not held (until 18d the rule took that box, and it was).
    shape(&mut ed, "corner", vb.x0 + 30.0, vb.y0 + 30.0, 40.0, 40.0);
    shape(&mut ed, "centre", 0.0, 0.0, 100.0, 60.0);
    shape(&mut ed, "past", vb.x1, 0.0, 100.0, 60.0);
    // Near the frame's far end, 450 along its long axis: turned with the
    // frame it fits (its corners reach 495 of 500); square to the page its
    // corners reach past the end.
    let at = Vec2::new(450.0, 0.0).rotate_deg(30.0);
    run(&mut ed, json!({"op":"add_shape","id":"along","shape":"rectangle","x":at.x,"y":at.y,"w":90,"h":90,"rotation":30}));
    shape(&mut ed, "square", at.x, at.y, 90.0, 90.0);
    assert_eq!(ed.parent_frame("corner"), None);
    assert_eq!(ed.parent_frame("centre").as_deref(), Some("r"));
    assert_eq!(ed.parent_frame("past"), None);
    assert_eq!(ed.parent_frame("along").as_deref(), Some("r"));
    assert_eq!(ed.parent_frame("square"), None);
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

fn el<'a>(ed: &'a Editor, id: &str) -> &'a Element {
    ed.doc.get(id).unwrap()
}

#[test]
fn the_rule_itself() {
    let mut ed = Editor::default();
    frame(&mut ed, "f", 50.0, 25.0, 100.0, 50.0, 0.0);
    shape(&mut ed, "flush", 50.0, 5.0, 100.0, 10.0);
    // Rounding past an edge (1e-8 of a frame 150 across) still counts.
    run(&mut ed, json!({"op":"add_shape","id":"hair","shape":"rectangle","x":25.0 - 0.5e-8,"y":5,"w":50.0 + 1e-8,"h":10}));
    shape(&mut ed, "past", 24.99, 5.0, 50.0, 10.0);
    let d = &ed.doc;
    assert!(frame_holds(d, el(&ed, "f"), el(&ed, "flush")), "flush with three edges");
    assert!(frame_holds(d, el(&ed, "f"), el(&ed, "hair")), "rounding past an edge");
    assert!(!frame_holds(d, el(&ed, "f"), el(&ed, "past")), "really past an edge");
    assert!(!frame_holds(d, el(&ed, "f"), el(&ed, "f")), "itself");
    assert!(!frame_holds(d, el(&ed, "flush"), el(&ed, "f")), "only frames hold");
}

#[test]
fn circle_frame_holds_what_lies_in_its_circle() {
    let mut ed = Editor::default();
    run(&mut ed, json!({"op":"add_frame","id":"c","x":0,"y":0,"w":1000,"h":1000,"style":"circle"}));
    // In the corner of the frame's box, outside its circle (corners 622
    // from the centre, the circle's radius is 500).
    shape(&mut ed, "corner", 420.0, 420.0, 40.0, 40.0);
    // Near the circle on its axis: corners 470.4 from the centre.
    shape(&mut ed, "rim", 450.0, 0.0, 40.0, 40.0);
    shape(&mut ed, "centre", 0.0, 0.0, 300.0, 300.0);
    assert_eq!(ed.parent_frame("corner"), None);
    assert_eq!(ed.parent_frame("rim").as_deref(), Some("c"));
    assert_eq!(ed.parent_frame("centre").as_deref(), Some("c"));
    // A wide frame switched to a circle keeps its box and draws the
    // circle on its short side.
    frame(&mut ed, "wide", 5000.0, 0.0, 1000.0, 600.0, 0.0);
    run(&mut ed, json!({"op":"update","id":"wide","patch":{"frame_style":"circle"}}));
    shape(&mut ed, "in_box_only", 5400.0, 0.0, 40.0, 40.0);
    shape(&mut ed, "in_circle", 5000.0, 250.0, 40.0, 40.0);
    assert_eq!(ed.parent_frame("in_box_only"), None);
    assert_eq!(ed.parent_frame("in_circle").as_deref(), Some("wide"));
    assert_agrees(&mut ed);
}

#[test]
fn bracket_and_invisible_frames_hold_their_whole_box() {
    for style in ["bracket", "invisible"] {
        let mut ed = Editor::default();
        run(&mut ed, json!({"op":"add_frame","id":"f","x":0,"y":0,"w":800,"h":600,"style":style}));
        // By an edge where a bracket draws nothing, inside the box.
        shape(&mut ed, "top", 0.0, -280.0, 100.0, 40.0);
        shape(&mut ed, "over", 0.0, -290.0, 100.0, 40.0);
        assert_eq!(ed.parent_frame("top").as_deref(), Some("f"), "{style}");
        assert_eq!(ed.parent_frame("over"), None, "{style}");
        assert_agrees(&mut ed);
    }
}

#[test]
fn lines_and_arrows_count_by_their_end_and_bend_points() {
    let mut ed = Editor::default();
    frame(&mut ed, "f", 0.0, 0.0, 800.0, 600.0, 0.0);
    // A line's box is taller than the frame; its ends are well inside.
    shape_kind(&mut ed, "line", "line", 0.0, 0.0, 700.0, 800.0, 0.0);
    // Turned 45°: the corners of its box reach out of the frame, its ends
    // (±247 each way) don't.
    shape_kind(&mut ed, "diag", "arrow", 0.0, 0.0, 700.0, 300.0, 45.0);
    shape_kind(&mut ed, "long", "line", 0.0, 0.0, 820.0, 10.0, 0.0);
    // A connector by its ends and the corners of its elbow.
    run(&mut ed, json!({"op":"add_connector","id":"elbow","from":{"x":-350,"y":-250},"to":{"x":350,"y":250},"route":"elbow"}));
    run(&mut ed, json!({"op":"add_connector","id":"out","from":{"x":-350,"y":-250},"to":{"x":450,"y":250}}));
    assert_eq!(ed.parent_frame("line").as_deref(), Some("f"));
    assert_eq!(ed.parent_frame("diag").as_deref(), Some("f"));
    assert_eq!(ed.parent_frame("long"), None);
    assert_eq!(el(&ed, "elbow").held_points().len(), 4, "two ends, two bends");
    assert_eq!(ed.parent_frame("elbow").as_deref(), Some("f"));
    assert_eq!(ed.parent_frame("out"), None);
    assert_agrees(&mut ed);
}

#[test]
fn a_group_counts_as_a_whole() {
    let mut ed = Editor::default();
    frame(&mut ed, "f", 0.0, 0.0, 800.0, 600.0, 0.0);
    shape(&mut ed, "in", 0.0, 0.0, 100.0, 100.0);
    shape(&mut ed, "out", 1000.0, 0.0, 100.0, 100.0);
    shape(&mut ed, "also_in", 200.0, 0.0, 100.0, 100.0);
    run(&mut ed, json!({"op":"group","ids":["in","out"]}));
    assert_eq!(ed.parent_frame("in"), None, "one member is out: the group stays");
    assert_eq!(ed.parent_frame("out"), None);
    assert_eq!(ed.frame_contents("f"), vec!["also_in".to_string()]);
    assert!(!frame_holds(&ed.doc, el(&ed, "f"), el(&ed, "in")));
    assert_agrees(&mut ed);
    // All in: the group is held.
    run(&mut ed, json!({"op":"move","ids":["out"],"dx":-800,"dy":0}));
    assert_eq!(ed.parent_frame("in").as_deref(), Some("f"));
    assert_eq!(ed.parent_frame("out").as_deref(), Some("f"));
    assert_agrees(&mut ed);
    // Ungrouped, each counts by itself.
    run(&mut ed, json!({"op":"move","ids":["out"],"dx":800,"dy":0}));
    run(&mut ed, json!({"op":"ungroup","ids":["in"]}));
    assert_eq!(ed.parent_frame("in").as_deref(), Some("f"));
    assert_eq!(ed.parent_frame("out"), None);
    assert_agrees(&mut ed);
}

#[test]
fn nested_frames_the_smallest_holder_wins() {
    let mut ed = Editor::default();
    frame(&mut ed, "outer", 0.0, 0.0, 3000.0, 3000.0, 0.0);
    run(&mut ed, json!({"op":"add_frame","id":"round","x":0,"y":0,"w":1000,"h":1000,"style":"circle"}));
    // In the round frame's box, outside its circle: the outer frame's.
    shape(&mut ed, "box_corner", 440.0, 440.0, 40.0, 40.0);
    shape(&mut ed, "middle", 0.0, 0.0, 100.0, 100.0);
    // Two frames overlap; both hold this: the smaller one has it.
    frame(&mut ed, "a", 1000.0, 1000.0, 800.0, 800.0, 0.0);
    frame(&mut ed, "b", 1100.0, 1000.0, 700.0, 700.0, 0.0);
    shape(&mut ed, "both", 1100.0, 1000.0, 100.0, 100.0);
    // A rectangle frame and a circle frame on the same box: the circle
    // lies inside the rectangle and is smaller, so it is held.
    frame(&mut ed, "sq", -1000.0, -1000.0, 600.0, 600.0, 0.0);
    run(&mut ed, json!({"op":"add_frame","id":"disc","x":-1000,"y":-1000,"w":600,"h":600,"style":"circle"}));
    assert_eq!(ed.parent_frame("box_corner").as_deref(), Some("outer"));
    assert_eq!(ed.parent_frame("middle").as_deref(), Some("round"));
    assert_eq!(ed.parent_frame("round").as_deref(), Some("outer"));
    assert_eq!(ed.parent_frame("both").as_deref(), Some("b"));
    assert_eq!(ed.parent_frame("disc").as_deref(), Some("sq"));
    assert_eq!(ed.parent_frame("sq").as_deref(), Some("outer"));
    assert_agrees(&mut ed);
}

#[test]
fn something_exactly_the_size_of_its_frame_is_held() {
    let mut ed = Editor::default();
    frame(&mut ed, "f", 0.0, 0.0, 800.0, 600.0, 0.0);
    shape(&mut ed, "page", 0.0, 0.0, 800.0, 600.0);
    // A page turned with its frame (the PDF rotate case): its corners
    // carry rounding.
    frame(&mut ed, "q", 3000.0, 0.0, 800.0, 600.0, 90.0);
    run(&mut ed, json!({"op":"add_shape","id":"turned","shape":"rectangle","x":3000,"y":0,"w":800,"h":600,"rotation":90}));
    frame(&mut ed, "t", 6000.0, 0.0, 800.0, 600.0, 37.0);
    run(&mut ed, json!({"op":"add_shape","id":"slanted","shape":"rectangle","x":6000,"y":0,"w":800,"h":600,"rotation":37}));
    assert_eq!(ed.parent_frame("page").as_deref(), Some("f"));
    assert_eq!(ed.parent_frame("turned").as_deref(), Some("q"));
    assert_eq!(ed.parent_frame("slanted").as_deref(), Some("t"));
    // Two frames the same size don't hold each other.
    frame(&mut ed, "twin", 0.0, 0.0, 800.0, 600.0, 0.0);
    assert_eq!(ed.parent_frame("twin"), None);
    assert_eq!(ed.parent_frame("f"), None);
    assert_agrees(&mut ed);
}

#[test]
fn locked_objects_are_held_but_never_move() {
    let mut ed = Editor::default();
    frame(&mut ed, "f", 0.0, 0.0, 800.0, 600.0, 0.0);
    shape(&mut ed, "bg", 0.0, 0.0, 700.0, 500.0);
    shape(&mut ed, "s", 100.0, 0.0, 100.0, 100.0);
    run(&mut ed, json!({"op":"update","id":"bg","patch":{"locked":true}}));
    assert_eq!(ed.parent_frame("bg").as_deref(), Some("f"), "it still belongs to the frame");
    assert_eq!(ed.carried(&["f".to_string()]), vec!["f".to_string(), "s".to_string()]);
    assert_eq!(moved_by_drag(&mut ed, "f"), set(["s".to_string()]));
    let before = el(&ed, "bg").transform;
    run(&mut ed, json!({"op":"rotate","ids":["f"],"degrees":20}));
    run(&mut ed, json!({"op":"scale","ids":["f"],"factor":1.5}));
    assert_eq!(el(&ed, "bg").transform, before);
}

#[test]
fn what_a_frame_carries_is_fixed_when_the_drag_starts() {
    let mut ed = Editor::default();
    frame(&mut ed, "f", 0.0, 0.0, 800.0, 600.0, 0.0);
    shape(&mut ed, "mine", 0.0, 0.0, 100.0, 100.0);
    // In the way of the drag.
    shape(&mut ed, "passer", 1000.0, 0.0, 100.0, 100.0);
    // A drag: what moves is worked out once, then moved step by step.
    let carried = ed.carried(&["f".to_string()]);
    assert_eq!(carried, vec!["f".to_string(), "mine".to_string()]);
    for _ in 0..4 {
        run(&mut ed, json!({"op":"move","ids":carried,"dx":500,"dy":0,"with_contents":false}));
    }
    assert_eq!(el(&ed, "f").transform.x, 2000.0);
    assert_eq!(el(&ed, "mine").transform.x, 2000.0);
    assert_eq!(el(&ed, "passer").transform.x, 1000.0, "the frame passed over it and left it");
    // A turn the same way: the frame swings past things and leaves them.
    frame(&mut ed, "g", 0.0, 5000.0, 1000.0, 200.0, 0.0);
    shape(&mut ed, "near", 0.0, 5300.0, 50.0, 50.0);
    let carried = ed.carried(&["g".to_string()]);
    for _ in 0..6 {
        run(&mut ed, json!({"op":"rotate","ids":carried,"degrees":15,"about":[0, 5000],"with_contents":false}));
    }
    assert_eq!(el(&ed, "near").transform.y, 5300.0);
    assert_eq!(ed.parent_frame("near").as_deref(), Some("g"), "now inside the turned frame");
}
