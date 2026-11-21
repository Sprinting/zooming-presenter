//! Tables as big as Numbers' (1,000,000 x 1,000) whose empty cells cost
//! nothing: made, edited, saved, reopened, and hostile files of the
//! sparse form (plan-21nov/27).
use serde_json::json;
use std::time::Instant;
use zebra_core::table::{self, CellValue, TableData};
use zebra_core::*;

fn run(ed: &mut Editor, j: serde_json::Value) -> ZResult<Outcome> {
    ed.apply(serde_json::from_value(j).unwrap(), Origin::Human)
}

fn table(ed: &Editor, id: &str) -> TableData {
    match &ed.doc.get(id).unwrap().kind {
        ElementKind::Table(t) => t.clone(),
        _ => panic!("{id} is not a table"),
    }
}

fn input(t: &TableData, r: usize, c: usize) -> &str {
    t.cells[r].get(c).map_or("", |c| c.input.as_str())
}

/// Scattered cells: (row, column, input), fewer in the biggest tables
/// (each edit there takes about 0.1 s in a release build, more in tests).
fn scattered(rows: usize) -> Vec<(usize, usize, String)> {
    let n = if rows > 100_000 { 30 } else { 200 };
    let mut v: Vec<(usize, usize, String)> = (0..n).map(|i| ((i * 7_919) % rows, (i * 37) % 1000, format!("{}", i * 3))).collect();
    v.push((rows - 1, 999, "corner".into()));
    v.push((rows / 2, 0, "7".into()));
    v.push((rows - 2, 0, "5".into()));
    v.push((0, 0, "Title".into()));
    v
}

/// Makes a `rows` x 1,000 table, fills scattered cells, adds a formula,
/// saves and reopens it: (seconds each step took, saved bytes).
fn round_trip(rows: usize) -> (Vec<(&'static str, f64)>, usize) {
    let mut times = vec![];
    let mut t0 = Instant::now();
    let mut lap = |name: &'static str, times: &mut Vec<(&'static str, f64)>| {
        times.push((name, t0.elapsed().as_secs_f64()));
        t0 = Instant::now();
    };
    let mut ed = Editor::default();
    run(&mut ed, json!({"op": "add_table", "id": "t", "rows": rows, "columns": 1000, "header_rows": 0})).unwrap();
    lap("make", &mut times);
    for (r, c, v) in scattered(rows) {
        run(&mut ed, json!({"op": "set_cells", "id": "t", "at": table::cell_name(r, c), "values": [[v]]})).unwrap();
    }
    lap("edits", &mut times);
    // A sum down the whole of column A.
    run(&mut ed, json!({"op": "set_cells", "id": "t", "at": "B1", "values": [[format!("=SUM(A2:A{rows})")]]})).unwrap();
    lap("formula", &mut times);
    let t = table(&ed, "t");
    assert_eq!((t.n_rows(), t.n_cols()), (rows, 1000));
    let want: f64 = scattered(rows).iter().filter(|(r, c, _)| *c == 0 && *r > 0).map(|(_, _, v)| v.parse::<f64>().unwrap()).sum();
    assert_eq!(want, 12.0);
    assert_eq!(t.cells[0].get(1).unwrap().value, Some(CellValue::Number(want)));
    let bytes = io::save_bytes(&ed.doc).unwrap();
    lap("save", &mut times);
    let back = io::load_bytes(&bytes).unwrap();
    lap("open", &mut times);
    let mut ed2 = Editor::default();
    ed2.load(back, None);
    let t2 = table(&ed2, "t");
    assert_eq!(t2, t, "reopens as it was");
    assert_eq!(input(&t2, rows - 1, 999), "corner");
    assert_eq!(input(&t2, 0, 0), "Title");
    assert_eq!(input(&t2, rows / 2 + 1, 500), "");
    (times, bytes.len())
}

#[test]
fn a_full_size_table_with_scattered_cells_is_made_saved_and_reopened() {
    for rows in [10_000, 100_000, table::MAX_ROWS] {
        let (times, size) = round_trip(rows);
        eprintln!("{rows} x 1000: {times:?}, {size} bytes saved");
        assert!(size < 64 << 10, "{rows} rows of mostly empty cells save small: {size} bytes");
    }
}

#[test]
fn a_full_size_table_edits_and_undoes() {
    let mut ed = Editor::default();
    let rows = table::MAX_ROWS;
    run(&mut ed, json!({"op": "add_table", "id": "t", "rows": rows, "columns": 1000})).unwrap();
    run(&mut ed, json!({"op": "set_cells", "id": "t", "at": "ALL1000000", "values": [["last"]]})).unwrap();
    run(&mut ed, json!({"op": "set_cells", "id": "t", "at": "C5", "values": [[5], [3], [9]]})).unwrap();
    // Full: Numbers refuses a row more; nothing changes.
    let full = table(&ed, "t");
    let e = run(&mut ed, json!({"op": "insert_rows", "id": "t", "index": 4, "count": 1})).unwrap_err();
    assert_eq!(e.code, "too_big");
    assert_eq!(table(&ed, "t"), full);
    // Rows out and in, columns in: cells move with them.
    run(&mut ed, json!({"op": "delete_rows", "id": "t", "index": 0, "count": 2})).unwrap();
    assert_eq!(input(&table(&ed, "t"), 2, 2), "5");
    run(&mut ed, json!({"op": "insert_rows", "id": "t", "index": 0, "count": 2})).unwrap();
    run(&mut ed, json!({"op": "delete_columns", "id": "t", "index": 0, "count": 1})).unwrap();
    run(&mut ed, json!({"op": "insert_columns", "id": "t", "index": 0, "count": 1})).unwrap();
    let t = table(&ed, "t");
    assert_eq!((t.n_rows(), t.n_cols()), (rows, 1000));
    assert_eq!(input(&t, 4, 2), "5");
    assert_eq!(input(&t, rows - 1, 999), "last");
    // Merging joins what it covers into the first cell, as Numbers does.
    run(&mut ed, json!({"op": "merge_cells", "id": "t", "range": "C5:C7"})).unwrap();
    let t = table(&ed, "t");
    assert_eq!((input(&t, 4, 2), input(&t, 5, 2)), ("5\n3\n9", ""));
    // Sorting a million rows: empty cells go last.
    run(&mut ed, json!({"op": "unmerge_cells", "id": "t", "range": "C5:C7"})).unwrap();
    run(&mut ed, json!({"op": "set_cells", "id": "t", "at": "D10", "values": [[2], [1]]})).unwrap();
    let t0 = Instant::now();
    run(&mut ed, json!({"op": "sort_table", "id": "t", "column": 3})).unwrap();
    eprintln!("sort 1,000,000 rows: {:?}", t0.elapsed());
    let t = table(&ed, "t");
    // (Taking out the first two rows took the header row with it.)
    let hr = t.header_rows as usize;
    assert_eq!((input(&t, hr, 3), input(&t, hr + 1, 3), input(&t, hr + 2, 3)), ("1", "2", ""), "filled cells first");
    // Undo all the way back to the empty table.
    while ed.can_undo() {
        ed.undo(Origin::Human).unwrap();
    }
    assert!(ed.doc.get("t").is_none());
}

/// A document holding one table whose `kind` is `t`.
fn file(t: serde_json::Value) -> String {
    json!({
        "format_version": 4, "title": "t", "theme": "classic", "next_id": 2, "z_order": ["t"], "path": [],
        "elements": {"t": {"id": "t", "transform": {"x": 0, "y": 0, "scale": 1, "rotation": 0}, "size": {"w": 10, "h": 10}, "kind": t}}
    })
    .to_string()
}

fn opened(t: serde_json::Value) -> ZResult<TableData> {
    let doc = io::from_json(&file(t))?;
    match &doc.get("t").unwrap().kind {
        ElementKind::Table(t) => Ok(t.clone()),
        _ => panic!("not a table"),
    }
}

#[test]
fn the_sparse_form_reads_back_and_older_forms_still_open() {
    // Runs of empty rows, sparse rows, dense rows and runs of row heights.
    let t = opened(json!({"type": "table",
        "cells": [{"n": 3, "c": [[2, "x"]]}, {"n": 3, "k": 999998}, ["a", "", "b"]],
        "columns": [], "rows": [[999999, {"height": 30}], {"height": 50}]}))
    .unwrap();
    assert_eq!((t.n_rows(), t.n_cols()), (1_000_000, 3));
    assert_eq!((input(&t, 0, 2), input(&t, 999_999, 0), input(&t, 999_999, 2)), ("x", "a", "b"));
    assert_eq!((t.rows[5].height, t.rows[999_999].height), (30.0, 50.0));
    // Version 2 and 3 files (dense) still open.
    let old = r#"{"format_version":2,"title":"t","theme":"classic","next_id":2,"z_order":["t"],"path":[],
        "elements":{"t":{"id":"t","transform":{"x":0,"y":0,"scale":1,"rotation":0},"size":{"w":10,"h":10},
        "kind":{"type":"table","cells":[["a","b"],["c",""]],"columns":[],"rows":[]}}}}"#;
    let doc = io::from_json(old).unwrap();
    let ElementKind::Table(t) = &doc.get("t").unwrap().kind else { panic!() };
    assert_eq!((input(t, 1, 0), t.n_cols()), ("c", 2));
    // What's written: a mostly full row dense, a mostly empty one sparse,
    // a run of empty rows once.
    let mut ed = Editor::default();
    ed.load(doc, None);
    run(&mut ed, json!({"op": "insert_rows", "id": "t", "index": 2, "count": 500})).unwrap();
    run(&mut ed, json!({"op": "insert_columns", "id": "t", "index": 2, "count": 8})).unwrap();
    run(&mut ed, json!({"op": "set_cells", "id": "t", "at": "J300", "values": [["far"]]})).unwrap();
    let v: serde_json::Value = serde_json::from_str(&io::to_json(&ed.doc)).unwrap();
    let cells = &v["elements"]["t"]["kind"]["cells"];
    assert_eq!(cells[0], json!({"n": 10, "c": [[0, "a"], [1, "b"]]}));
    assert_eq!(cells[2], json!({"n": 10, "k": 297}));
    assert_eq!(cells[3], json!({"n": 10, "c": [[9, "far"]]}));
    assert_eq!(v["elements"]["t"]["kind"]["rows"].as_array().unwrap().len(), 1, "one run of alike rows");
}

#[test]
fn hostile_sparse_files() {
    // Claims bigger than memory are refused, as an oversized file is.
    for t in [
        json!({"type": "table", "cells": [{"n": 1u64 << 50}], "columns": [], "rows": []}),
        json!({"type": "table", "cells": [{"n": 2, "c": [[1u64 << 50, "x"]]}], "columns": [], "rows": []}),
        json!({"type": "table", "cells": [{"n": 2, "k": 1u64 << 50}], "columns": [], "rows": []}),
        json!({"type": "table", "cells": [["a"]], "columns": [], "rows": [[1u64 << 50, {"height": 30}]]}),
    ] {
        let e = opened(t.clone()).unwrap_err();
        assert!(e.message.contains("memory"), "{t}: {}", e.message);
    }
    // Past Numbers' size but within memory: kept whole (B2).
    let t = opened(json!({"type": "table", "cells": [{"n": 1500, "c": [[1499, "z"]]}, {"n": 1, "k": 1_200_000}], "columns": [], "rows": []})).unwrap();
    assert_eq!((t.n_rows(), t.n_cols()), (1_200_001, 1500));
    assert_eq!(input(&t, 0, 1499), "z");
    // A cell past its row's length widens the table; columns out of order
    // are sorted, and a column given twice keeps the later cell.
    let t = opened(json!({"type": "table", "cells": [{"n": 2, "c": [[5, "far"], [0, "first"], [0, "second"]]}, {"n": 2}], "columns": [], "rows": []})).unwrap();
    assert_eq!(t.n_cols(), 6);
    assert_eq!((input(&t, 0, 0), input(&t, 0, 5)), ("second", "far"));
    assert_eq!(t.cells[1].len(), 6, "rows are evened out");
    // Junk in the sparse form is repaired, not fatal: bad n, bad k, pairs
    // that aren't [column, cell], cells that don't read, unknown keys.
    let t = opened(json!({"type": "table", "cells": [
        {"n": "lots", "k": -4, "c": [[1, "ok"], ["x", "no"], [2], 7, [3, {"input": "kept", "value": {"bogus": 1}}]], "evil": true},
        {"n": 4, "c": "nope"},
        {},
        ["dense", 5, null]
    ], "columns": [], "rows": [[2, {"height": "x"}], "junk"]}))
    .unwrap();
    assert_eq!(t.n_rows(), 4);
    assert_eq!((input(&t, 0, 1), input(&t, 0, 3)), ("ok", "kept"));
    assert_eq!(input(&t, 3, 0), "dense");
    assert_eq!(t.check(), Ok(()));
}

#[test]
fn the_cell_count_bounds_filled_cells_not_the_grid() {
    // A full-size table is tiny in memory: its rows and columns only.
    assert!(table::fits_limits(table::MAX_ROWS, table::MAX_COLUMNS));
    let t = TableData::new(table::MAX_ROWS, table::MAX_COLUMNS, 20.0);
    assert_eq!((t.n_rows(), t.n_cols()), (table::MAX_ROWS, table::MAX_COLUMNS));
    assert_eq!(t.cells.iter().map(|r| r.filled_count()).sum::<usize>(), 0);
}

/// Edits in a full-size table, timed (release builds: `cargo test
/// --release -- --ignored edit_speed --nocapture`).
#[test]
#[ignore]
fn edit_speed() {
    let mut ed = Editor::default();
    run(&mut ed, json!({"op": "add_table", "id": "t", "rows": table::MAX_ROWS, "columns": 1000})).unwrap();
    let t0 = Instant::now();
    for i in 0..50 {
        run(&mut ed, json!({"op": "set_cells", "id": "t", "at": table::cell_name(i * 997, i), "values": [[i]]})).unwrap();
    }
    eprintln!("one edit at 1,000,000 rows: {:?}", t0.elapsed() / 50);
}
