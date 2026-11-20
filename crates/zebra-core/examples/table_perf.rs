//! How long table edits take, and what their undo steps hold.
//! `cargo run --release -p zebra-core --example table_perf`
use std::time::{Duration, Instant};

use serde_json::json;
use zebra_core::*;

fn csv(rows: usize, cols: usize, row_totals: bool) -> String {
    let last = table::column_name(cols - 2);
    let mut out = String::new();
    for r in 0..rows {
        let line: Vec<String> = (0..cols)
            .map(|c| match (r, c) {
                (0, c) => format!("Col {c}"),
                (r, c) if r + 1 == rows => format!("=SUM({l}2:{l}{n})", l = table::column_name(c), n = rows - 1),
                (r, c) if row_totals && c + 1 == cols => format!("=SUM(A{n}:{last}{n})", n = r + 1),
                (r, c) if c == 0 => format!("Item {r}"),
                (r, c) => ((r * 7 + c * 13) % 1000).to_string(),
            })
            .collect();
        out.push_str(&line.join("\t"));
        out.push('\n');
    }
    out
}

fn time<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let v = f();
    (v, t.elapsed())
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn run(rows: usize, cols: usize, row_totals: bool) {
    println!("\n{rows} x {cols} ({} cells){}", rows * cols, if row_totals { ", a formula in every row" } else { ", totals in the footer" });
    let text = csv(rows, cols, row_totals);
    let mut ed = Editor::default();
    let (grid, parse) = time(|| table::parse_delimited(&text).unwrap());
    drop(grid);
    let (_, paste) = time(|| ed.apply(serde_json::from_value(json!({"op": "add_table", "id": "b1", "csv": text, "footer_rows": 1})).unwrap(), Origin::Human).unwrap());
    println!("  paste {} KB: parse {}, add (parse, work out, measure, undo step) {}, step {} KB", text.len() / 1024, ms(parse), ms(paste), ed.history().last().unwrap().approx_bytes() / 1024);
    for (what, c) in [
        ("edit a cell", json!({"op": "set_cells", "id": "b1", "at": "C50", "values": [[12345]]})),
        ("insert a row at the top", json!({"op": "insert_rows", "id": "b1", "index": 1})),
        ("sort", json!({"op": "sort_table", "id": "b1", "column": 3, "descending": true})),
        ("delete a row at the top", json!({"op": "delete_rows", "id": "b1", "index": 1})),
        ("insert a column", json!({"op": "insert_columns", "id": "b1", "index": 1})),
    ] {
        let (res, d) = time(|| ed.apply(serde_json::from_value(c).unwrap(), Origin::Human));
        match res {
            Ok(_) => println!("  {what}: {}, undo step {} bytes", ms(d), ed.history().last().unwrap().approx_bytes()),
            Err(e) => println!("  {what}: refused in {} ({})", ms(d), e.message),
        }
    }
    let (_, u) = time(|| ed.undo(Origin::Human).unwrap());
    let (_, r) = time(|| ed.redo(Origin::Human).unwrap());
    println!("  undo {}, redo {}", ms(u), ms(r));
    let (json, save) = time(|| io::to_json(&ed.doc));
    let (doc, read) = time(|| io::from_json(&json).unwrap());
    let (p, prep) = time(|| Editor::prepare(doc, &editor::EstimateMeasure, &|_| {}));
    drop(p);
    println!("  open: document.json {} KB, write {}, read {}, prepare (work out, measure) {}", json.len() / 1024, ms(save), ms(read), ms(prep));
}

fn main() {
    run(1000, 20, false);
    run(1000, 20, true);
    run(5000, 20, false);
    run(4990, 20, false);
    run(4999, 20, true);
    run(1000, 100, false);
    run(500, 200, true);
}
