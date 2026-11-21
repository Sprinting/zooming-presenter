//! Tables in the editor: commands, formulas, undo, files, limits and
//! hostile input.
use serde_json::json;
use zebra_core::table::{CellValue, TableData};
use zebra_core::*;

fn cmd(j: serde_json::Value) -> Command {
    serde_json::from_value(j).unwrap()
}

fn run(ed: &mut Editor, j: serde_json::Value) -> ZResult<Outcome> {
    ed.apply(cmd(j), Origin::Human)
}

fn table(ed: &Editor, id: &str) -> TableData {
    match &ed.doc.get(id).unwrap().kind {
        ElementKind::Table(t) => t.clone(),
        _ => panic!("{id} is not a table"),
    }
}

fn sales() -> Editor {
    let mut ed = Editor::default();
    run(
        &mut ed,
        json!({"op": "add_table", "id": "b1", "footer_rows": 1, "cells": [
            ["Region", "Q2", "Q3"],
            ["North", "£1,240,000", 980],
            ["South", "£980,000", 1200],
            ["East", "£410,000", 300],
            ["Total", "=SUM(B2:B4)", "=SUM(C2:C4)"]
        ]}),
    )
    .unwrap();
    ed
}

/// A `rows` x `cols` table of numbers under a header row, with a footer
/// row of column totals; with `row_totals`, the last column adds up each
/// row (a formula in every row).
fn big(rows: usize, cols: usize, row_totals: bool) -> Editor {
    let mut ed = Editor::default();
    let last = table::column_name(cols - 2);
    let cells: Vec<Vec<serde_json::Value>> = (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| match (r, c) {
                    (0, c) => json!(format!("Col {c}")),
                    (r, c) if r + 1 == rows => json!(format!("=SUM({l}2:{l}{n})", l = table::column_name(c), n = rows - 1)),
                    (r, c) if row_totals && c + 1 == cols => json!(format!("=SUM(A{n}:{last}{n})", n = r + 1)),
                    (r, c) => json!((r * 7 + c * 13) % 1000),
                })
                .collect()
        })
        .collect();
    run(&mut ed, json!({"op": "add_table", "id": "b1", "cells": cells, "footer_rows": 1})).unwrap();
    ed
}

#[test]
fn a_table_works_out_its_formulas_in_the_same_undo_step() {
    let mut ed = sales();
    let t = table(&ed, "b1");
    assert_eq!(t.cells[4][1].value, Some(CellValue::Number(2_630_000.0)));
    assert_eq!(t.shown(4, 1, t.column_look(1).as_ref()).text, "£2,630,000");
    assert_eq!(ed.doc.get("b1").unwrap().id, "b1");
    // Typing into a cell works the total out again; one undo puts both back.
    run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "B2", "values": [["£1,000,000"]]})).unwrap();
    assert_eq!(table(&ed, "b1").cells[4][1].value, Some(CellValue::Number(2_390_000.0)));
    ed.undo(Origin::Human).unwrap();
    let t = table(&ed, "b1");
    assert_eq!(t.cells[1][1].input, "£1,240,000");
    assert_eq!(t.cells[4][1].value, Some(CellValue::Number(2_630_000.0)));
    ed.redo(Origin::Human).unwrap();
    assert_eq!(table(&ed, "b1").cells[4][1].value, Some(CellValue::Number(2_390_000.0)));
}

#[test]
fn rows_and_columns_in_and_out_keep_formulas_pointing_at_their_cells() {
    let mut ed = sales();
    run(&mut ed, json!({"op": "insert_rows", "id": "b1", "index": 2})).unwrap();
    assert_eq!(table(&ed, "b1").cells[5][1].input, "=SUM(B2:B5)");
    run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "A3", "values": [["West", 100, 1]]})).unwrap();
    assert_eq!(table(&ed, "b1").cells[5][1].value, Some(CellValue::Number(2_630_100.0)));
    run(&mut ed, json!({"op": "delete_rows", "id": "b1", "index": 1})).unwrap();
    let t = table(&ed, "b1");
    assert_eq!(t.cells[4][1].input, "=SUM(B2:B4)");
    assert_eq!(t.cells[4][1].value, Some(CellValue::Number(1_390_100.0)));
    run(&mut ed, json!({"op": "insert_columns", "id": "b1", "index": 1})).unwrap();
    assert_eq!(table(&ed, "b1").cells[4][2].input, "=SUM(C2:C4)");
    run(&mut ed, json!({"op": "delete_columns", "id": "b1", "index": 1})).unwrap();
    assert_eq!(table(&ed, "b1").cells[4][1].input, "=SUM(B2:B4)");
    for _ in 0..5 {
        ed.undo(Origin::Human).unwrap();
    }
    assert_eq!(table(&ed, "b1"), table(&sales(), "b1"));
}

#[test]
fn merges_and_sorting() {
    let mut ed = sales();
    run(&mut ed, json!({"op": "sort_table", "id": "b1", "column": 1})).unwrap();
    let t = table(&ed, "b1");
    let names: Vec<&str> = t.cells.iter().map(|r| r[0].input.as_str()).collect();
    assert_eq!(names, ["Region", "East", "South", "North", "Total"], "by amount, header and footer stay");
    run(&mut ed, json!({"op": "sort_table", "id": "b1", "column": 0, "descending": true})).unwrap();
    let names: Vec<String> = table(&ed, "b1").cells.iter().map(|r| r[0].input.clone()).collect();
    assert_eq!(names, ["Region", "South", "North", "East", "Total"]);
    run(&mut ed, json!({"op": "merge_cells", "id": "b1", "range": "A2:A3"})).unwrap();
    let e = run(&mut ed, json!({"op": "sort_table", "id": "b1", "column": 1})).unwrap_err();
    assert!(e.message.contains("merged"), "{}", e.message);
    run(&mut ed, json!({"op": "unmerge_cells", "id": "b1", "range": "A2"})).unwrap();
    assert!(table(&ed, "b1").merges.is_empty());
    assert_eq!(table(&ed, "b1").cells[1][0].input, "South\nNorth", "merging keeps both texts");
}

#[test]
fn formats_columns_and_settings() {
    let mut ed = sales();
    run(&mut ed, json!({"op": "set_column", "id": "b1", "column": 2, "width": 300, "format": {"type": "currency", "currency": "$"}})).unwrap();
    let t = table(&ed, "b1");
    assert_eq!(t.shown(1, 2, None).text, "$980");
    assert_eq!(t.columns[2].width, 300.0);
    run(&mut ed, json!({"op": "format_cells", "id": "b1", "range": "A1:C1", "format": {"fill": "theme:shape5", "bold": false}})).unwrap();
    assert!(!table(&ed, "b1").bold_at(0, 1));
    run(&mut ed, json!({"op": "format_cells", "id": "b1", "range": "B1", "clear": true})).unwrap();
    assert!(table(&ed, "b1").bold_at(0, 1));
    assert!(!table(&ed, "b1").bold_at(0, 0));
    let bad = run(&mut ed, json!({"op": "format_cells", "id": "b1", "range": "A1", "format": {"fill": "javascript:alert(1)"}})).unwrap_err();
    assert!(bad.message.contains("colour"));
    run(&mut ed, json!({"op": "update", "id": "b1", "patch": {"table_style": "ts2", "header_columns": 1, "banded": false}})).unwrap();
    let t = table(&ed, "b1");
    assert_eq!((t.style, t.header_columns, t.banded), (table::TableStyle::Banded, 1, Some(false)));
    let e = run(&mut ed, json!({"op": "update", "id": "b1", "patch": {"header_rows": 5, "footer_rows": 5}})).unwrap_err();
    assert!(e.message.contains("too few"), "{}", e.message);
    let w = ed.doc.get("b1").unwrap().size.w;
    run(&mut ed, json!({"op": "resize", "id": "b1", "width": w * 2.0})).unwrap();
    assert!((ed.doc.get("b1").unwrap().size.w - w * 2.0).abs() < 1e-6);
}

#[test]
fn set_cells_grows_the_table_above_the_footer() {
    let mut ed = sales();
    // From the last body row down: two new rows go in above the footer.
    run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "A4", "values": [["East", 410, 300], ["West", 5, 5], ["Far", 1, 1]]})).unwrap();
    let t = table(&ed, "b1");
    assert_eq!(t.n_rows(), 7);
    assert_eq!(t.cells[6][0].input, "Total");
    assert_eq!(t.cells[6][1].input, "=SUM(B2:B6)", "the total takes the new rows in");
}

#[test]
fn files_round_trip_and_save_the_lowest_version_they_need() {
    let ed = sales();
    let json = io::to_json(&ed.doc);
    assert!(json.starts_with("{\n  \"format_version\": 4,"));
    let back = io::from_json(&json).unwrap();
    let mut ed2 = Editor::default();
    ed2.load(back, None);
    assert_eq!(table(&ed2, "b1"), table(&ed, "b1"));
    // No tables, no new colours: version 1, which older builds open.
    let mut plain = Editor::default();
    run(&mut plain, json!({"op": "add_text", "text": "Hi"})).unwrap();
    assert_eq!(required_version(&plain.doc), 1);
    assert!(io::to_json(&plain.doc).starts_with("{\n  \"format_version\": 1,"));
    // The fifth shape colour needs 2.
    run(&mut plain, json!({"op": "add_shape", "shape": "rectangle", "fill": "theme:shape5"})).unwrap();
    assert_eq!(required_version(&plain.doc), 2);
    // A version from the future is refused with a message.
    let future = json.replacen("\"format_version\": 4", &format!("\"format_version\": {}", FORMAT_VERSION + 1), 1);
    let e = io::from_json(&future).unwrap_err();
    assert_eq!(e.code, "bad_file");
    assert!(e.message.contains("newer than this app"), "{}", e.message);
}

#[test]
fn a_version_1_build_refuses_a_file_with_a_table() {
    // master's reader (format 1) refuses any file whose version is above
    // its own; a table makes the file version 2.
    let json = io::to_json(&sales().doc);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v["format_version"].as_u64().unwrap() > 1);
}

/// What each edit's undo step holds on a 1,000 x 20 table, and how long
/// the edit takes.
fn undo_sizes(row_totals: bool) -> (Vec<(&'static str, usize, std::time::Duration)>, Editor) {
    let mut ed = big(1000, 20, row_totals);
    let mut sizes = vec![];
    for (what, c) in [
        ("cell edit", json!({"op": "set_cells", "id": "b1", "at": "C500", "values": [[12345]]})),
        ("insert at top", json!({"op": "insert_rows", "id": "b1", "index": 1})),
        ("sort", json!({"op": "sort_table", "id": "b1", "column": 3, "descending": true})),
        ("delete at top", json!({"op": "delete_rows", "id": "b1", "index": 1})),
        ("insert column", json!({"op": "insert_columns", "id": "b1", "index": 0})),
    ] {
        let t0 = std::time::Instant::now();
        run(&mut ed, c).unwrap();
        let took = t0.elapsed();
        sizes.push((what, ed.history().last().unwrap().approx_bytes(), took));
    }
    (sizes, ed)
}

#[test]
fn undo_of_big_tables_stores_what_changed_not_the_table() {
    // A whole 1,000 x 20 table is about 2 MB in memory, 1.5 MB as JSON;
    // an undo step used to hold it twice.
    let (sizes, mut ed) = undo_sizes(false);
    eprintln!("undo bytes on 1,000 x 20, totals in the footer: {sizes:?}");
    let limit = |what: &str| match what {
        "cell edit" => 4_000,     // the cell and the totals that changed with it
        "insert at top" => 8_000, // one empty row, the totals' new ranges
        "sort" => 20_000,         // a permutation: 4 bytes a row, each way
        "delete at top" => 8_000,
        _ => 12_000,              // a column of empty cells
    };
    for (what, bytes, _) in &sizes {
        assert!(*bytes < limit(what), "{what}: {bytes} bytes");
    }
    // They undo and redo exactly.
    let after = table(&ed, "b1");
    for _ in 0..5 {
        ed.undo(Origin::Human).unwrap();
    }
    assert_eq!(table(&ed, "b1"), table(&big(1000, 20, false), "b1"));
    for _ in 0..5 {
        ed.redo(Origin::Human).unwrap();
    }
    assert_eq!(table(&ed, "b1"), after);
    // A formula in every row: inserting a row or sorting rewrites each
    // formula's references, so each one is stored; still well under the table.
    let (sizes, mut ed) = undo_sizes(true);
    eprintln!("undo bytes on 1,000 x 20, a formula in every row: {sizes:?}");
    for (what, bytes, _) in &sizes {
        assert!(*bytes < 400_000, "{what}: {bytes} bytes");
    }
    for _ in 0..5 {
        ed.undo(Origin::Human).unwrap();
    }
    assert_eq!(table(&ed, "b1"), table(&big(1000, 20, true), "b1"));
}

#[test]
fn deleting_a_table_drops_its_formula_sheet() {
    let mut ed = sales();
    assert_eq!(ed.formula_sheets(), 1);
    run(&mut ed, json!({"op": "delete", "ids": ["b1"]})).unwrap();
    assert_eq!(ed.formula_sheets(), 0);
    ed.undo(Origin::Human).unwrap();
    // Back with its values; the sheet is built again when it is next edited.
    assert_eq!(table(&ed, "b1").cells[4][1].value, Some(CellValue::Number(2_630_000.0)));
    run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "B4", "values": [["£0"]]})).unwrap();
    assert_eq!(table(&ed, "b1").cells[4][1].value, Some(CellValue::Number(2_220_000.0)));
    assert_eq!(ed.formula_sheets(), 1);
    // Undoing the table's creation drops it too.
    let mut ed = sales();
    ed.undo(Origin::Human).unwrap();
    assert_eq!(ed.formula_sheets(), 0);
}

#[test]
fn a_formula_cycle_shows_an_error() {
    let mut ed = Editor::default();
    run(&mut ed, json!({"op": "add_table", "id": "b1", "header_rows": 0, "cells": [["=B1+1", "=A1+1"], ["=A2", "5"]]})).unwrap();
    let t = table(&ed, "b1");
    for (r, c) in [(0, 0), (0, 1), (1, 0)] {
        assert!(matches!(&t.cells[r][c].value, Some(CellValue::Error { .. })), "{r},{c}: {:?}", t.cells[r][c].value);
    }
    let s = t.shown(1, 0, None);
    assert!(s.error.is_some(), "{s:?}");
}

#[test]
fn a_long_chain_of_formulas_works_out_without_overflowing() {
    // Each cell adds one to the one above: deep dependencies.
    let n = 5000;
    let cells: Vec<Vec<String>> = (0..n).map(|r| vec![if r == 0 { "1".into() } else { format!("=A{r}+1") }]).collect();
    let mut ed = Editor::default();
    let t0 = std::time::Instant::now();
    // A test thread has a small stack; the app's main thread a larger one.
    run(&mut ed, json!({"op": "add_table", "id": "b1", "header_rows": 0, "cells": cells})).unwrap();
    eprintln!("5,000-deep chain: {:?}", t0.elapsed());
    assert_eq!(table(&ed, "b1").cells[n - 1][0].value, Some(CellValue::Number(n as f64)));
}

#[test]
fn functions_that_reach_out_are_not_run() {
    let mut ed = Editor::default();
    run(
        &mut ed,
        json!({"op": "add_table", "id": "b1", "header_rows": 0, "cells": [
            ["=INFO(\"system\")", "=WEBSERVICE(\"http://example.com\")", "=IMPORTDATA(\"http://x\")", "=A1&\"\""],
            ["=HYPERLINK(\"https://example.com\",\"Site\")", "=\"INFO(\"", "=_xlfn.WEBSERVICE(\"x\")", "=RAND()<2"]
        ]}),
    )
    .unwrap();
    let t = table(&ed, "b1");
    for (r, c) in [(0, 0), (0, 1), (0, 2), (1, 2)] {
        assert_eq!(t.cells[r][c].value, Some(CellValue::Error { error: "#NAME?".into() }), "{r},{c}");
    }
    assert!(matches!(&t.cells[0][3].value, Some(CellValue::Error { .. })), "cells using it see an error");
    // HYPERLINK shows its text; nothing in Zebra follows it.
    assert_eq!(t.cells[1][0].value, Some(CellValue::Text("Site".into())));
    assert_eq!(t.cells[1][1].value, Some(CellValue::Text("INFO(".into())), "a name inside a string is just text");
    assert_eq!(t.cells[1][3].value, Some(CellValue::Bool(true)));
}

#[test]
fn over_the_limits_is_refused_with_a_message_and_changes_nothing() {
    let mut ed = sales();
    let before = ed.doc.clone();
    let long = "x".repeat(table::MAX_INPUT + 1);
    let e = run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "A2", "values": [[long]]})).unwrap_err();
    assert_eq!(e.code, "too_big");
    assert!(e.message.contains("32767"), "{}", e.message);
    let formula = format!("={}", "1+".repeat(4100)) + "1";
    let e = run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "A2", "values": [[formula]]})).unwrap_err();
    assert!(e.message.contains("formula"), "{}", e.message);
    // Numbers' limits: 1,000,000 rows and 1,000 columns.
    let e = run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "A1000001", "values": [[1]]})).unwrap_err();
    assert_eq!(e.code, "too_big");
    let e = run(&mut ed, json!({"op": "insert_columns", "id": "b1", "index": 0, "count": 1000})).unwrap_err();
    assert_eq!(e.code, "too_big");
    let e = run(&mut ed, json!({"op": "insert_rows", "id": "b1", "index": 0, "count": usize::MAX})).unwrap_err();
    assert_eq!(e.code, "too_big");
    // Empty cells take no memory (as in Numbers): a full-size table is
    // made, and only its rows and columns cost anything.
    let mut big = Editor::default();
    run(&mut big, json!({"op": "add_table", "id": "t", "rows": table::MAX_ROWS, "columns": table::MAX_COLUMNS})).unwrap();
    assert_eq!(table(&big, "t").n_rows(), table::MAX_ROWS);
    assert!(!table::fits_memory(usize::MAX / 2, 1000));
    assert_eq!(table::Limit::of_size(usize::MAX / 2, 1000), Some(table::Limit::Rows));
    let e = run(&mut ed, json!({"op": "delete_rows", "id": "b1", "index": usize::MAX, "count": 2})).unwrap_err();
    assert_eq!(e.code, "invalid_params");
    assert_eq!(ed.doc, before);
}

#[test]
fn huge_pastes_are_refused_before_they_become_a_grid() {
    use table::{parse_delimited, Limit};
    // No size limit on the text (as in Numbers): 30 MB of cells read.
    let big = "12345678\t".repeat(999).to_string() + "1\n";
    let big = big.repeat(3_000);
    assert!(big.len() > 25 << 20);
    let grid = parse_delimited(&big).unwrap();
    assert_eq!((grid.len(), grid[0].len()), (3_000, 1_000));
    // Past Numbers' row and column limits: stops at the first one over.
    let many = "1\t".repeat(2_000);
    assert_eq!(parse_delimited(&many), Err(Limit::Columns));
    let tall = "1\n".repeat(1_000_001);
    assert_eq!(parse_delimited(&tall), Err(Limit::Rows));
    // Filled cells must fit in memory: the count is checked as fields
    // are read (no Mac in the cloud runs out at 3,000,000).
    assert!(table::max_cells() > 3_000_000);
    let long_field = format!("a\t{}", "z".repeat(table::MAX_INPUT + 2));
    assert_eq!(parse_delimited(&long_field), Err(Limit::Input));
    assert_eq!(parse_delimited("").unwrap(), Vec::<Vec<String>>::new());
    let mut ed = Editor::default();
    let e = run(&mut ed, json!({"op": "add_table", "csv": many})).unwrap_err();
    assert_eq!(e.code, "too_big");
}

#[test]
fn empty_and_degenerate_tables() {
    let mut ed = Editor::default();
    run(&mut ed, json!({"op": "add_table", "id": "b1"})).unwrap();
    assert_eq!((table(&ed, "b1").n_rows(), table(&ed, "b1").n_cols()), (4, 4));
    run(&mut ed, json!({"op": "add_table", "id": "b2", "cells": []})).unwrap();
    assert_eq!((table(&ed, "b2").n_rows(), table(&ed, "b2").n_cols()), (1, 1));
    run(&mut ed, json!({"op": "add_table", "id": "b3", "csv": ""})).unwrap();
    let e = run(&mut ed, json!({"op": "delete_rows", "id": "b2", "index": 0})).unwrap_err();
    assert!(e.message.contains("at least one row"));
    let e = run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "ZZZZ1", "values": [[1]]})).unwrap_err();
    assert_eq!(e.code, "invalid_params");
    let e = run(&mut ed, json!({"op": "set_column", "id": "b1", "column": 0, "width": -5})).unwrap_err();
    assert_eq!(e.code, "invalid_params");
    let e = run(&mut ed, json!({"op": "set_cells", "id": "b1", "values": []})).unwrap_err();
    assert_eq!(e.code, "invalid_params");
    let e = run(&mut ed, json!({"op": "set_cells", "id": "nope", "values": [[1]]})).unwrap_err();
    assert_eq!(e.code, "not_found");
    run(&mut ed, json!({"op": "add_text", "id": "t1", "text": "x"})).unwrap();
    let e = run(&mut ed, json!({"op": "set_cells", "id": "t1", "values": [[1]]})).unwrap_err();
    assert!(e.message.contains("not a table"));
}

#[test]
fn the_table_query_and_describe() {
    let ed = sales();
    let q = ed.query(Query::Table { id: "b1".into(), range: None }).unwrap();
    assert_eq!(q["shown"][4][1], "£2,630,000");
    assert_eq!(q["formulas"]["B5"], "=SUM(B2:B4)");
    let d = describe::describe(&ed);
    assert!(d.contains("b1 table 5x3 ts4"), "{d}");
    assert!(d.contains("5: Total | £2,630,000 =SUM(B2:B4)"), "{d}");
    assert!(ed.query(Query::Table { id: "b1".into(), range: Some(table::CellRange::parse("Z99").unwrap()) }).is_err());
}

/// A table element's JSON with `kind` replaced.
fn file_with_table(kind: serde_json::Value) -> String {
    json!({
        "format_version": 2, "title": "t", "theme": "classic", "next_id": 5, "z_order": ["b1"], "path": [],
        "elements": {"b1": {"id": "b1", "transform": {"x": 0, "y": 0, "scale": 1, "rotation": 0}, "size": {"w": 10, "h": 10}, "kind": kind}}
    })
    .to_string()
}

fn load_ok_or_refused(json: &str) {
    match io::from_json(json) {
        Ok(doc) => {
            let mut ed = Editor::default();
            ed.load(doc, None);
            for e in ed.doc.elements.values() {
                check::element(e).unwrap_or_else(|err| panic!("repaired table still broken: {}", err.message));
            }
        }
        Err(e) => assert_eq!(e.code, "bad_file"),
    }
}

#[test]
fn hostile_table_files_load_repaired_or_are_refused() {
    let cases = [
        json!({"type": "table", "cells": [["a", "b", "c"], ["d"], [], "row", 5, null], "columns": [{"width": 1}], "rows": []}),
        json!({"type": "table", "cells": [["a"]], "columns": [{"width": null}, {"width": -5}, {"width": 1e308}, "x"], "rows": [{"height": "tall"}, {"height": 1e300}]}),
        json!({"type": "table", "cells": [["a", "b"], ["c", "d"]], "columns": [], "rows": [], "merges": ["A1:Z99", "A1:B2", "B2:B2", "nonsense", 7], "header_rows": 300, "footer_rows": 200, "header_columns": -1}),
        json!({"type": "table", "cells": [[{"input": 5, "value": {"x": 1}}, {"input": "=1/0", "value": {"error": 5}}, {"value": 1}, [[["deep"]]]]], "columns": [], "rows": []}),
        json!({"type": "table", "cells": "everything", "columns": 7, "rows": {"a": 1}, "style": "ts99", "font_size": "big", "gridlines": 3, "cell_formats": [{"range": "A1", "fill": 5}, {"range": "Q1:A9", "bold": true}]}),
        json!({"type": "table", "cells": [[{"input": "x", "spans": [{"start": 5, "end": 1}]}]], "columns": [{"width": 100, "type": "nope"}], "rows": [{"height": 1, "fit": -4}]}),
        json!({"type": "table"}),
        json!({"type": "table", "cells": [[]], "columns": [], "rows": [], "font_size": 0}),
    ];
    for k in cases {
        load_ok_or_refused(&file_with_table(k));
    }
}

#[test]
fn a_file_with_millions_of_cells_keeps_every_one() {
    // 2,000 x 1,000 = 2,000,000 cells, ragged, and a row and a cell past
    // Numbers' limits and Excel's cell length: nothing is cut (B2).
    let row: Vec<&str> = vec![""; 1000];
    let mut cells: Vec<Vec<&str>> = vec![row; 2000];
    cells[3] = vec!["x"; 5];
    let wide: Vec<&str> = vec!["w"; 1200];
    cells[4] = wide;
    let long = "z".repeat(table::MAX_INPUT + 10);
    cells[5][0] = &long;
    let json = file_with_table(json!({"type": "table", "cells": cells, "columns": [], "rows": []}));
    let doc = io::from_json(&json).unwrap();
    let ElementKind::Table(t) = &doc.elements["b1"].kind else { panic!() };
    assert_eq!((t.n_rows(), t.n_cols()), (2000, 1200));
    assert_eq!(t.cells[4][1199].input, "w");
    assert_eq!(t.cells[5][0].input.len(), table::MAX_INPUT + 10);
    // Saved and opened again, still whole.
    let back = io::load_bytes(&io::save_bytes(&doc).unwrap()).unwrap();
    let ElementKind::Table(b) = &back.elements["b1"].kind else { panic!() };
    assert_eq!((b.n_rows(), b.n_cols()), (2000, 1200));
    // Still editable: typing in it works, making it wider is refused.
    let mut ed = Editor::new(back);
    run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "A1", "values": [["hi"]]})).unwrap();
    assert_eq!(table(&ed, "b1").cells[0][0].input, "hi");
    run(&mut ed, json!({"op": "delete_rows", "id": "b1", "index": 0, "count": 10})).unwrap();
    assert!(run(&mut ed, json!({"op": "insert_columns", "id": "b1", "index": 0})).is_err());
    assert_eq!(table(&ed, "b1").n_cols(), 1200);
}

/// A tiny deterministic random source (no dependency).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[(self.next() % v.len() as u64) as usize]
    }
}

fn mutate(v: &mut serde_json::Value, rng: &mut Rng, depth: usize) {
    let wild = [json!(null), json!(-1), json!(1e308), json!(-1e308), json!(0), json!("x"), json!([]), json!({}), json!(true), json!(4294967296u64), json!("=1/0"), json!([["a"]]), json!({"input": 3})];
    match v {
        serde_json::Value::Object(m) if !m.is_empty() => {
            let keys: Vec<String> = m.keys().cloned().collect();
            let k = rng.pick(&keys).clone();
            if depth > 3 || rng.next() % 3 == 0 {
                m.insert(k, rng.pick(&wild).clone());
            } else if let Some(x) = m.get_mut(&k) {
                mutate(x, rng, depth + 1);
            }
        }
        serde_json::Value::Array(a) if !a.is_empty() => {
            let i = (rng.next() % a.len() as u64) as usize;
            match rng.next() % 4 {
                0 => a[i] = rng.pick(&wild).clone(),
                1 => {
                    a.remove(i);
                }
                2 => a.push(rng.pick(&wild).clone()),
                _ => mutate(&mut a[i], rng, depth + 1),
            }
        }
        _ => *v = rng.pick(&wild).clone(),
    }
}

#[test]
fn fuzzed_table_files_never_panic() {
    let base = serde_json::to_value(&sales().doc.elements["b1"].kind).unwrap();
    let mut rng = Rng(0x2545F4914F6CDD1D);
    for _ in 0..600 {
        let mut k = base.clone();
        for _ in 0..(1 + rng.next() % 4) {
            mutate(&mut k, &mut rng, 0);
        }
        if let Some(o) = k.as_object_mut() {
            o.insert("type".into(), json!("table"));
        }
        load_ok_or_refused(&file_with_table(k));
    }
}

/// Reshaping a table and its formula sheet together (the fast path) ends
/// where a sheet built afresh from the table does, over a run of random
/// edits, undos and redos.
#[test]
fn the_kept_formula_sheet_agrees_with_a_fresh_one() {
    let mut ed = big(30, 6, true);
    run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "B3", "values": [["=B2*2", "£5", "=A3+B3"], ["=SUM(B2:B3)", "13.7%", "x"]]})).unwrap();
    let mut rng = Rng(0x5eed_1234_abcd);
    for step in 0..120 {
        let t = table(&ed, "b1");
        let (n, c) = (t.n_rows(), t.n_cols());
        let pick = |rng: &mut Rng, k: usize| (rng.next() as usize) % k.max(1);
        let op = match pick(&mut rng, 9) {
            0 => json!({"op": "insert_rows", "id": "b1", "index": pick(&mut rng, n + 1), "count": 1 + pick(&mut rng, 2)}),
            1 if n > 4 => json!({"op": "delete_rows", "id": "b1", "index": 1 + pick(&mut rng, n - 2), "count": 1}),
            2 => json!({"op": "insert_columns", "id": "b1", "index": pick(&mut rng, c + 1), "count": 1}),
            3 if c > 3 => json!({"op": "delete_columns", "id": "b1", "index": pick(&mut rng, c - 1), "count": 1}),
            4 => json!({"op": "move_rows", "id": "b1", "from": 1 + pick(&mut rng, n - 2), "count": 1, "to": 1 + pick(&mut rng, n - 2)}),
            5 => json!({"op": "move_columns", "id": "b1", "from": pick(&mut rng, c), "count": 1, "to": pick(&mut rng, c)}),
            6 => json!({"op": "sort_table", "id": "b1", "column": pick(&mut rng, c), "descending": rng.next() % 2 == 0}),
            7 => {
                let _ = ed.undo(Origin::Human);
                continue;
            }
            _ => json!({"op": "set_cells", "id": "b1", "at": table::cell_name(1 + pick(&mut rng, n - 2), pick(&mut rng, c)), "values": [[(rng.next() % 500) as i64]]}),
        };
        let _ = run(&mut ed, op);
        let kept = table(&ed, "b1");
        let mut fresh = kept.clone();
        for row in fresh.cells.iter_mut() {
            for (_, cell) in row.filled_mut() {
                cell.value = None;
                cell.format = None;
            }
        }
        calc::Calc::new().recalc("x", &mut fresh);
        for r in 0..kept.n_rows() {
            for c in 0..kept.n_cols() {
                assert_eq!(kept.cells[r][c].value, fresh.cells[r][c].value, "step {step}: {} = {:?}", table::cell_name(r, c), kept.cells[r][c].input);
            }
        }
    }
}

#[test]
fn a_leading_apostrophe_keeps_text_as_text() {
    let mut ed = Editor::default();
    run(&mut ed, json!({"op": "add_table", "id": "b1", "cells": [["a", "b"], ["x", "=A2"]]})).unwrap();
    run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "A2", "values": [["'=SUM(A1)"]]})).unwrap();
    let t = table(&ed, "b1");
    assert!(!t.cells[1][0].is_formula());
    assert_eq!(t.shown(1, 0, None).text, "=SUM(A1)");
    // A formula that reads it sees the text without the apostrophe.
    assert_eq!(t.cells[1][1].value, Some(CellValue::Text("=SUM(A1)".into())));
    // A number kept as text keeps its zeros.
    run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "A2", "values": [["'0042"]]})).unwrap();
    assert_eq!(table(&ed, "b1").shown(1, 0, None).text, "0042");
    // An apostrophe alone shows nothing.
    run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "A2", "values": [["'"]]})).unwrap();
    assert_eq!(table(&ed, "b1").shown(1, 0, None).text, "");
}
