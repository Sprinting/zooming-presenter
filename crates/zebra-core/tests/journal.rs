//! The editor records each change from the old values of what it touched
//! (its journal), not from a copy of the whole document. Every kind of
//! command must still undo and redo exactly. (Debug builds also check each
//! step against a whole-document diff inside the editor.)
use serde_json::json;
use zebra_core::*;

fn cmd(j: serde_json::Value) -> Command {
    serde_json::from_value(j).unwrap()
}

fn deck() -> Editor {
    let mut ed = Editor::default();
    ed.apply_batch(
        vec![
            cmd(json!({"op":"add_frame","id":"f1","x":0,"y":0,"w":800,"h":600,"add_to_path":true})),
            cmd(json!({"op":"add_text","id":"t1","text":"Hello world","style":"title","x":0,"y":-100})),
            cmd(json!({"op":"add_text","id":"t2","text":"One\nTwo","x":0,"y":100})),
            cmd(json!({"op":"add_shape","id":"s1","shape":"rectangle","x":300,"y":200,"w":100,"h":80})),
            cmd(json!({"op":"add_shape","id":"s2","shape":"ellipse","x":-300,"y":200,"w":100,"h":80})),
            cmd(json!({"op":"add_frame","id":"f2","x":2000,"y":0,"w":800,"h":600,"add_to_path":true})),
            cmd(json!({"op":"add_connector","id":"c1","from":{"element":"s1"},"to":{"element":"s2"}})),
        ],
        Some("build".into()),
        Origin::Human,
    )
    .unwrap();
    ed
}

/// Apply `c`, then check that undo gives back the document before it and
/// redo the one after.
fn round_trip(ed: &mut Editor, c: serde_json::Value) {
    let before = ed.doc.clone();
    let label = c.to_string();
    ed.apply(cmd(c), Origin::Human).unwrap_or_else(|e| panic!("{label}: {e:?}"));
    let after = ed.doc.clone();
    assert_ne!(after, before, "{label} changed nothing");
    ed.undo(Origin::Human).unwrap();
    assert_eq!(ed.doc, before, "undo of {label}");
    ed.redo(Origin::Human).unwrap();
    assert_eq!(ed.doc, after, "redo of {label}");
}

#[test]
fn every_kind_of_command_undoes_and_redoes_exactly() {
    let mut ed = deck();
    let template = zebra_core::templates::TEMPLATES[0].id.clone();
    let layout = zebra_core::templates::LAYOUTS[0].id.clone();
    let diagram = zebra_core::diagrams::ALL[0].key;
    for c in [
        json!({"op":"move","ids":["s1"],"dx":50,"dy":-20}),
        json!({"op":"move","ids":["f1"],"dx":10,"dy":10}),
        json!({"op":"scale","ids":["s2"],"factor":1.5}),
        json!({"op":"rotate","ids":["t2"],"degrees":30}),
        json!({"op":"resize","id":"s1","width":300}),
        json!({"op":"update","id":"t1","patch":{"text":"Changed","name":"Title"}}),
        json!({"op":"update","id":"s2","patch":{"name":"Oval","locked":true}}),
        json!({"op":"update","id":"s2","patch":{"locked":false,"name":""}}),
        json!({"op":"format_text","id":"t1","start":0,"end":3,"format":{"bold":true}}),
        json!({"op":"format_paragraphs","id":"t2","list":"bullet"}),
        json!({"op":"group","ids":["s1","s2"]}),
        json!({"op":"duplicate","ids":["s1"],"dx":20,"dy":20}),
        json!({"op":"ungroup","ids":["s1"]}),
        json!({"op":"z_order","ids":["s1"],"order":"back"}),
        json!({"op":"z_order","ids":["t2"],"order":"forward"}),
        json!({"op":"wrap_in_frame","ids":["s1","s2"],"add_to_path":true}),
        json!({"op":"arrange","ids":["s1","s2","t2"],"layout":"row"}),
        json!({"op":"align","ids":["s1","s2"],"edge":"left"}),
        json!({"op":"add_step","target":{"kind":"overview"}}),
        json!({"op":"move_step","from":0,"to":1}),
        json!({"op":"update_step","index":0,"notes":"Say hi","builds":["t1"]}),
        json!({"op":"remove_step","index":1}),
        json!({"op":"set_title","title":"Talk"}),
        json!({"op":"set_playback","autoplay":5,"loop":true}),
        json!({"op":"set_theme","theme":"night"}),
        json!({"op":"add_diagram","diagram":diagram,"x":5000,"y":0}),
        json!({"op":"add_layout","layout":layout,"x":-5000,"y":0,"add_to_path":true}),
        json!({"op":"delete","ids":["s2"]}),
        json!({"op":"auto_path","discard_notes":true}),
        json!({"op":"clear_path","discard_notes":true}),
        json!({"op":"apply_template","template":template,"replace":true}),
    ] {
        round_trip(&mut ed, c);
    }
}

#[test]
fn a_drag_is_one_step_and_cancelling_it_restores_everything() {
    let mut ed = deck();
    let before = ed.doc.clone();
    ed.begin("Move", Origin::Human).unwrap();
    for _ in 0..5 {
        ed.apply(cmd(json!({"op":"move","ids":["s1"],"dx":3,"dy":4})), Origin::Human).unwrap();
    }
    ed.apply(cmd(json!({"op":"add_shape","shape":"ellipse","x":0,"y":0})), Origin::Human).unwrap();
    ed.abort();
    assert_eq!(ed.doc, before, "abort");
    ed.begin("Move", Origin::Human).unwrap();
    for _ in 0..5 {
        ed.apply(cmd(json!({"op":"move","ids":["s1"],"dx":3,"dy":4})), Origin::Human).unwrap();
    }
    ed.commit();
    let after = ed.doc.clone();
    assert_eq!(ed.doc.get("s1").unwrap().transform.x, before.get("s1").unwrap().transform.x + 15.0);
    ed.undo(Origin::Human).unwrap();
    assert_eq!(ed.doc, before, "one undo undoes the whole drag");
    ed.redo(Origin::Human).unwrap();
    assert_eq!(ed.doc, after);
}

#[test]
fn a_failed_batch_leaves_nothing_behind() {
    let mut ed = deck();
    let before = ed.doc.clone();
    let r = ed.apply_batch(
        vec![
            cmd(json!({"op":"add_frame","x":0,"y":5000,"w":100,"h":100,"add_to_path":true})),
            cmd(json!({"op":"move","ids":["s1","f1"],"dx":1,"dy":1})),
            cmd(json!({"op":"set_title","title":"Gone"})),
            cmd(json!({"op":"z_order","ids":["s1"],"order":"front"})),
            cmd(json!({"op":"delete","ids":["s2"]})),
            cmd(json!({"op":"delete","ids":["nope"]})),
        ],
        None,
        Origin::Human,
    );
    assert!(r.is_err());
    assert_eq!(ed.doc, before);
    // And the editor is still in a state to take the next change.
    round_trip(&mut ed, json!({"op":"move","ids":["s1"],"dx":1,"dy":1}));
}

#[test]
fn moving_a_connected_shape_moves_the_connector_in_the_same_step() {
    let mut ed = deck();
    let before = ed.doc.clone();
    ed.apply(cmd(json!({"op":"move","ids":["s1"],"dx":500,"dy":0})), Origin::Human).unwrap();
    assert_ne!(ed.doc.get("c1"), before.get("c1"), "the connector follows");
    ed.undo(Origin::Human).unwrap();
    assert_eq!(ed.doc, before);
}
