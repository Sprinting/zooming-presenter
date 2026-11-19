//! Timings of whole-deck work on a big deck, without the app.
//!
//!   cargo run --release -p zebra-core --example big_deck -- 10000
//!
//! Builds the stress-test deck (cells of frame + title + body + ellipse,
//! every 5th frame on the path) in batches of 2,000 commands, then times
//! the agent queries, a move of everything, its undo, and a reload.
use std::time::Instant;

use serde_json::json;
use zebra_core::command::{Command, Query};
use zebra_core::{Editor, Origin};

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(10_000);
    let skip_queries = std::env::args().any(|a| a == "--skip-queries");
    let h = Origin::agent("bench");
    let mut ed = Editor::default();
    let cmd = |j: serde_json::Value| -> Command { serde_json::from_value(j).unwrap() };
    let (mut cmds, mut made, mut gx, mut batches) = (vec![], 0, 0usize, vec![]);
    let t0 = Instant::now();
    while made < n {
        let (x, y) = ((gx % 100) as f64 * 1200.0, (gx / 100) as f64 * 900.0);
        cmds.push(cmd(json!({"op":"add_frame","x":x,"y":y,"w":1000,"h":700,"add_to_path":gx % 5 == 0})));
        cmds.push(cmd(json!({"op":"add_text","text":format!("Section {gx}"),"x":x,"y":y - 250.0,"style":"title","scale":0.6})));
        cmds.push(cmd(json!({"op":"add_text","text":"First point\nSecond point\nThird point","x":x,"y":y,"style":"body"})));
        cmds.push(cmd(json!({"op":"add_shape","shape":"ellipse","x":x + 350.0,"y":y + 200.0,"w":120,"h":120})));
        made += 4;
        gx += 1;
        if cmds.len() >= 2000 || made >= n {
            let t = Instant::now();
            ed.apply_batch(std::mem::take(&mut cmds), Some("batch".into()), h.clone()).unwrap();
            batches.push(t.elapsed().as_secs_f64());
        }
    }
    println!("built {} elements in {:.2} s; 2,000-command batch first {:.0} ms, last {:.0} ms", ed.doc.elements.len(), t0.elapsed().as_secs_f64(), batches[0] * 1e3, batches[batches.len() - 1] * 1e3);
    let time = |label: &str, f: &mut dyn FnMut()| {
        let t = Instant::now();
        f();
        println!("{label}: {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    };
    if !skip_queries {
        time("describe", &mut || {
            ed.query(Query::Describe).unwrap();
        });
        time("query elements", &mut || {
            ed.query(serde_json::from_value(json!({"q":"elements"})).unwrap()).unwrap();
        });
    }
    let ids: Vec<String> = ed.doc.elements.keys().cloned().collect();
    time("move all", &mut || {
        ed.apply(cmd(json!({"op":"move","ids":ids,"dx":10,"dy":10})), h.clone()).unwrap();
    });
    time("undo", &mut || {
        ed.undo(h.clone()).unwrap();
    });
    time("add frame", &mut || {
        ed.apply(cmd(json!({"op":"add_frame","x":-5000,"y":-5000,"w":1000,"h":700})), h.clone()).unwrap();
    });
    let doc = ed.doc.clone();
    time("load", &mut || ed.load(doc.clone(), None));
}
