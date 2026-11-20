//! The spatial index gives the same answers as the brute-force functions
//! on `Document`, on random decks (rotated and scaled elements, nested and
//! equal-sized frames, connectors) through edits, failed batches, undo and
//! redo.
use serde_json::{json, Value};
use zebra_core::*;

const H: Origin = Origin::Human;

fn cmd(j: Value) -> Command {
    serde_json::from_value(j).unwrap()
}

/// xorshift: deterministic, no dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn f(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (self.next() % 1_000_000) as f64 / 1_000_000.0 * (hi - lo)
    }
    /// Coarse values, so equal sizes and shared edges turn up.
    fn coarse(&mut self, lo: f64, hi: f64) -> f64 {
        (self.f(lo, hi) / 50.0).round() * 50.0
    }
}

fn pick(ed: &Editor, rng: &mut Rng) -> Option<ElementId> {
    let z = &ed.doc.z_order;
    (!z.is_empty()).then(|| z[rng.below(z.len())].clone())
}

/// A new frame's place, brute force: behind the lowest element it holds.
fn brute_frame_pos(doc: &Document, b: &Rect) -> usize {
    doc.z_order.iter().position(|o| doc.get(o).is_some_and(|x| frame_holds(b, &x.bounds()))).unwrap_or(doc.z_order.len())
}

fn random_command(ed: &Editor, rng: &mut Rng) -> Value {
    let id = pick(ed, rng);
    let some = |rng: &mut Rng| -> Vec<ElementId> {
        let mut v: Vec<ElementId> = (0..1 + rng.below(3)).filter_map(|_| pick(ed, rng)).collect();
        v.dedup();
        v
    };
    let (x, y) = (rng.coarse(-3000.0, 3000.0), rng.coarse(-3000.0, 3000.0));
    let rot = if rng.below(3) == 0 { rng.f(-180.0, 180.0) } else { 0.0 };
    match rng.below(16) {
        0..=2 => {
            let s = rng.coarse(100.0, 4000.0);
            let style = ["bracket", "circle", "rectangle", "invisible"][rng.below(4)];
            let h = rng.coarse(100.0, 3000.0);
            json!({"op":"add_frame","style":style,"x":x,"y":y,"w":s,"h":h,"rotation":rot})
        }
        3..=5 => {
            let shape = ["rectangle", "ellipse", "line", "arrow"][rng.below(4)];
            let (w, h) = (rng.coarse(10.0, 800.0), rng.coarse(10.0, 600.0));
            json!({"op":"add_shape","shape":shape,"x":x,"y":y,"w":w,"h":h,"rotation":rot})
        }
        6 => json!({"op":"add_text","text":"Words here","x":x,"y":y}),
        7 => match (pick(ed, rng), pick(ed, rng)) {
            (Some(a), Some(b)) if a != b => json!({"op":"add_connector","from":{"element":a},"to":{"element":b}}),
            _ => json!({"op":"add_shape","shape":"ellipse","x":x,"y":y,"w":100,"h":100}),
        },
        8 => json!({"op":"move","ids":some(rng),"dx":rng.coarse(-1500.0, 1500.0),"dy":rng.coarse(-1500.0, 1500.0),"with_contents":rng.below(2) == 0}),
        9 => json!({"op":"rotate","ids":some(rng),"degrees":rng.f(-90.0, 90.0)}),
        10 => json!({"op":"scale","ids":some(rng),"factor":rng.f(0.3, 3.0)}),
        11 => json!({"op":"delete","ids":some(rng)}),
        12 => {
            let order = ["front", "back", "forward", "backward"][rng.below(4)];
            json!({"op":"z_order","ids":some(rng),"order":order})
        }
        // Exact copies on top: equal areas tie.
        13 => json!({"op":"duplicate","ids":some(rng),"dx":0,"dy":0}),
        14 => match id {
            Some(id) if ed.doc.get(&id).is_some_and(|e| e.is_frame()) => {
                let style = ["bracket", "circle", "rectangle"][rng.below(3)];
                json!({"op":"update","id":id,"patch":{"frame_style":style}})
            }
            Some(id) => json!({"op":"update","id":id,"patch":{"w":rng.coarse(10.0, 900.0),"x":x}}),
            None => json!({"op":"add_text","text":"x","x":x,"y":y}),
        },
        _ => json!({"op":"move","ids":["no-such-id"],"dx":1,"dy":1}),
    }
}

fn check(ed: &Editor, rng: &mut Rng, ctx: &str) {
    let d = &ed.doc;
    let parents = ed.parent_frames();
    for id in d.elements.keys() {
        let want = d.parent_frame(id);
        assert_eq!(ed.parent_frame(id), want, "{ctx}: parent_frame({id})");
        assert_eq!(parents.get(id.as_str()).map(|s| s.to_string()), want, "{ctx}: parent_frames()[{id}]");
        if d.elements[id].is_frame() {
            assert_eq!(ed.frame_contents(id), d.frame_contents(id), "{ctx}: frame_contents({id})");
            let e = &d.elements[id];
            assert_eq!(ed.own_label(e), e.own_label(d), "{ctx}: own_label({id})");
        }
    }
    let ordered: Vec<&str> = d.ordered().map(|e| e.id.as_str()).collect();
    for _ in 0..40 {
        // Points near elements (edges, corners), and anywhere.
        let p = match pick(ed, rng).and_then(|id| d.get(&id)) {
            Some(e) if rng.below(4) != 0 => {
                let c = e.corners()[rng.below(4)];
                Vec2::new(c.x + rng.f(-20.0, 20.0), c.y + rng.f(-20.0, 20.0))
            }
            _ => Vec2::new(rng.f(-6000.0, 6000.0), rng.f(-6000.0, 6000.0)),
        };
        let tol = [0.0, 4.0, 60.0][rng.below(3)];
        assert_eq!(ed.hit_test(p, tol).map(|e| &e.id), d.hit_test(p, tol).map(|e| &e.id), "{ctx}: hit_test({p:?}, {tol})");
        // Culling: everything that reaches into a rectangle, back to front.
        let r = Rect::from_center(p, rng.f(1.0, 5000.0), rng.f(1.0, 5000.0));
        let got: Vec<&str> = ed.elements_in(r).iter().map(|e| e.id.as_str()).collect();
        let mut it = ordered.iter();
        assert!(got.iter().all(|g| it.any(|o| o == g)), "{ctx}: elements_in is not back to front");
        for e in d.ordered() {
            let pad = spatial::draw_overhang(e);
            if e.bounds().inflate(pad).intersects(&r) {
                assert!(got.contains(&e.id.as_str()), "{ctx}: elements_in({r:?}) misses {}", e.id);
            }
        }
    }
}

#[test]
fn index_answers_match_brute_force_on_random_decks() {
    for seed in 1..=12u64 {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0x2545_F491_4F6C_DD1D));
        let mut ed = Editor::default();
        for step in 0..160 {
            let ctx = format!("seed {seed} step {step}");
            match rng.below(12) {
                0 => {
                    let _ = ed.undo(H);
                }
                1 => {
                    let _ = ed.redo(H);
                }
                2 => {
                    // A batch: several commands, the last of which may fail
                    // and roll the whole batch back.
                    let n = 2 + rng.below(5);
                    let cmds: Vec<Command> = (0..n).map(|_| cmd(random_command(&ed, &mut rng))).collect();
                    let _ = ed.apply_batch(cmds, None, H);
                }
                _ => {
                    let c = random_command(&ed, &mut rng);
                    let new_frame = (c["op"] == "add_frame").then(|| {
                        // A circle frame gets a square box.
                        let (w, h) = (c["w"].as_f64().unwrap(), c["h"].as_f64().unwrap());
                        let (w, h) = if c["style"] == "circle" { (w.max(h), w.max(h)) } else { (w, h) };
                        let probe = Element {
                            id: String::new(),
                            transform: Transform { x: c["x"].as_f64().unwrap(), y: c["y"].as_f64().unwrap(), scale: 1.0, rotation: c["rotation"].as_f64().unwrap() },
                            size: Size { w, h },
                            locked: false,
                            name: String::new(),
                            placeholder: false,
                            group: None,
                            kind: ElementKind::Frame(FrameData { style: FrameStyle::Bracket, color: None }),
                        };
                        brute_frame_pos(&ed.doc, &probe.bounds())
                    });
                    if let Ok(out) = ed.apply(cmd(c), H) {
                        if let Some(want) = new_frame {
                            let id = &out.created[0];
                            assert_eq!(ed.doc.z_order.iter().position(|z| z == id), Some(want), "{ctx}: new frame's place in the draw order");
                        }
                    }
                }
            }
            if step % 4 == 0 {
                check(&ed, &mut rng, &ctx);
            }
        }
        check(&ed, &mut rng, &format!("seed {seed} end"));
        // A transaction that is cancelled puts everything back.
        ed.begin("drag", H).unwrap();
        if let Some(id) = pick(&ed, &mut rng) {
            let _ = ed.apply(cmd(json!({"op":"move","ids":[id],"dx":500,"dy":0})), H);
        }
        ed.abort();
        check(&ed, &mut rng, &format!("seed {seed} after abort"));
        // Loading rebuilds it.
        let doc = ed.doc.clone();
        let mut fresh = Editor::default();
        fresh.load(doc, None);
        check(&fresh, &mut rng, &format!("seed {seed} loaded"));
    }
}

/// A host that changes the document behind the editor's back gets the
/// brute-force answers until it calls `reindex` (or the next edit).
#[test]
fn a_document_changed_behind_the_editors_back_is_not_trusted() {
    let mut ed = Editor::default();
    ed.apply(cmd(json!({"op":"add_frame","id":"f","x":0,"y":0,"w":1000,"h":700})), H).unwrap();
    ed.apply(cmd(json!({"op":"add_shape","id":"s","shape":"rectangle","x":9000,"y":0,"w":100,"h":100})), H).unwrap();
    let mut e = ed.doc.elements["s"].clone();
    e.id = "inside".into();
    e.transform.x = 0.0;
    ed.doc.elements.insert("inside".into(), e);
    ed.doc.z_order.push("inside".into());
    assert_eq!(ed.frame_contents("f"), vec!["inside".to_string()]);
    assert_eq!(ed.parent_frame("inside").as_deref(), Some("f"));
    assert_eq!(ed.hit_test(Vec2::new(0.0, 0.0), 1.0).map(|e| e.id.as_str()), Some("inside"));
    ed.reindex();
    assert_eq!(ed.frame_contents("f"), vec!["inside".to_string()]);
}

/// Zoomable decks span huge size ranges: a 10^7 nest and far-off
/// elements still answer right.
#[test]
fn deep_zoom_nests_and_far_coordinates() {
    let mut ed = Editor::default();
    let mut s = 1.0e7;
    for i in 0..8 {
        ed.apply(cmd(json!({"op":"add_frame","id":format!("f{i}"),"x":0,"y":0,"w":s,"h":s * 0.7})), H).unwrap();
        s /= 10.0;
    }
    ed.apply(cmd(json!({"op":"add_shape","id":"dot","shape":"ellipse","x":0,"y":0,"w":0.5,"h":0.5})), H).unwrap();
    ed.apply(cmd(json!({"op":"add_shape","id":"far","shape":"ellipse","x":1.0e12,"y":-3.0e11,"w":10,"h":10})), H).unwrap();
    assert_eq!(ed.parent_frame("dot").as_deref(), Some("f7"));
    assert_eq!(ed.parent_frame("f3").as_deref(), Some("f2"));
    assert_eq!(ed.parent_frame("far"), None);
    assert_eq!(ed.frame_contents("f6"), ed.doc.frame_contents("f6"));
    assert_eq!(ed.hit_test(Vec2::new(1.0e12, -3.0e11), 1.0).map(|e| e.id.as_str()), Some("far"));
    // The frames went behind what they enclose: a nest drawn biggest first.
    let pos = |id: &str| ed.doc.z_order.iter().position(|z| z == id).unwrap();
    assert!(pos("f0") < pos("f7"));
}

/// The ownership rule's edge tolerance reaches the index too: an element
/// a hair wider than its frame (rounding on a rotated frame) is still
/// found there, as brute force finds it.
#[test]
fn edge_tolerance_matches_brute_force() {
    let mut ed = Editor::default();
    ed.apply(cmd(json!({"op":"add_frame","id":"f","x":0,"y":0,"w":1024,"h":2048})), H).unwrap();
    // As wide as the frame plus a rounding error, much less tall.
    let w = 1024.0 * (1.0 + 1e-10);
    ed.apply(cmd(json!({"op":"add_shape","id":"s","shape":"rectangle","x":0,"y":0,"w":w,"h":100})), H).unwrap();
    ed.apply(cmd(json!({"op":"add_shape","id":"out","shape":"rectangle","x":0,"y":0,"w":1024.5,"h":100})), H).unwrap();
    assert_eq!(ed.doc.parent_frame("s").as_deref(), Some("f"));
    assert_eq!(ed.parent_frame("s").as_deref(), Some("f"));
    assert_eq!(ed.parent_frames().get("s").copied(), Some("f"));
    assert_eq!(ed.doc.parent_frame("out"), None);
    assert_eq!(ed.parent_frame("out"), None);
    assert_eq!(ed.frame_contents("f"), vec!["s".to_string()]);
    assert_eq!(ed.frame_contents("f"), ed.doc.frame_contents("f"));
}
