//! Charts in the editor: commands, links to tables, undo, files, limits
//! and hostile input.
use serde_json::json;
use zebra_core::chart::{ChartData, ChartLook, ChartType, SeriesIn};
use zebra_core::*;

fn cmd(j: serde_json::Value) -> Command {
    serde_json::from_value(j).unwrap()
}

fn run(ed: &mut Editor, j: serde_json::Value) -> ZResult<Outcome> {
    ed.apply(cmd(j), Origin::Human)
}

fn chart(ed: &Editor, id: &str) -> ChartData {
    match &ed.doc.get(id).unwrap().kind {
        ElementKind::Chart(c) => c.clone(),
        _ => panic!("{id} is not a chart"),
    }
}

fn label(ed: &Editor) -> String {
    let l = ed.undo_label().unwrap_or_default();
    l.strip_suffix(" (You)").unwrap_or(&l).to_string()
}

fn values(ed: &Editor, id: &str) -> Vec<Vec<f64>> {
    chart(ed, id).plot(&ed.doc).series.into_iter().map(|s| s.values).collect()
}

/// A table b1 with a footer row of totals.
fn sales() -> Editor {
    let mut ed = Editor::default();
    run(
        &mut ed,
        json!({"op": "add_table", "id": "b1", "footer_rows": 1, "cells": [
            ["Region", "Q2", "Q3"],
            ["North", 1240, 1410],
            ["South", 980, "=B3+70"],
            ["East", 760, 702],
            ["Total", "=SUM(B2:B4)", "=SUM(C2:C4)"]
        ]}),
    )
    .unwrap();
    ed
}

#[test]
fn a_new_chart_has_sample_data_and_one_undo_step() {
    let mut ed = Editor::default();
    let out = run(&mut ed, json!({"op": "add_chart"})).unwrap();
    let id = &out.created[0];
    assert!(id.starts_with('h'), "{id}");
    let c = chart(&ed, id);
    assert_eq!(c.chart_type, ChartType::Column);
    assert_eq!(c.look, ChartLook::Flat);
    let p = c.plot(&ed.doc);
    assert_eq!(p.categories, ["North", "South", "East", "West"]);
    assert_eq!(p.series.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Q2", "Q3"]);
    assert_eq!(label(&ed), "Add chart");
    ed.undo(Origin::Human).unwrap();
    assert!(ed.doc.get(id).is_none());
    ed.redo(Origin::Human).unwrap();
    assert!(ed.doc.get(id).is_some());
    // Every type starts with data it can draw.
    for t in ChartType::ALL {
        let id = run(&mut ed, json!({"op": "add_chart", "chart_type": t.key()})).unwrap().created[0].clone();
        let p = chart(&ed, &id).plot(&ed.doc);
        assert!(!p.series.is_empty() && p.series.iter().all(|s| s.values.iter().any(|v| v.is_finite())), "{t:?}: {p:?}");
    }
}

#[test]
fn charts_are_format_3_and_older_builds_refuse_them() {
    assert_eq!(FORMAT_VERSION, 3);
    let mut ed = sales();
    assert_eq!(io::required_version(&ed.doc), 2);
    let id = run(&mut ed, json!({"op": "add_chart", "title": "Sales"})).unwrap().created[0].clone();
    assert_eq!(io::required_version(&ed.doc), 3);
    let json = io::to_json(&ed.doc);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["format_version"], 3);
    let back = io::from_json(&json).unwrap();
    assert_eq!(back.elements[&id], ed.doc.elements[&id]);
    // A build that knows format 2 refuses it as newer.
    let mut future = v.clone();
    future["format_version"] = json!(FORMAT_VERSION + 1);
    assert_eq!(io::from_json(&future.to_string()).unwrap_err().code, "bad_file");
    ed.apply(cmd(json!({"op": "delete", "ids": [id]})), Origin::Human).unwrap();
    assert_eq!(io::required_version(&ed.doc), 2);
    // Defaults stay out of the file.
    let mut ed = Editor::default();
    let id = run(&mut ed, json!({"op": "add_chart"})).unwrap().created[0].clone();
    let v = serde_json::to_value(&ed.doc.elements[&id].kind).unwrap();
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    keys.sort();
    assert_eq!(keys, ["chart_type", "data", "type"], "{v}");
}

#[test]
fn data_reads_by_columns_or_rows_with_gaps_for_text() {
    let mut ed = Editor::default();
    let id = run(&mut ed, json!({"op": "add_chart", "data": [["", "A", "B"], ["x", 1, "n/a"], ["y", "=2*3", 4]]})).unwrap().created[0].clone();
    let v = values(&ed, &id);
    assert_eq!(v[0], [1.0, 6.0], "a formula's value");
    assert!(v[1][0].is_nan(), "text is a gap");
    run(&mut ed, json!({"op": "format_chart", "id": id, "series_in": "rows"})).unwrap();
    let p = chart(&ed, &id).plot(&ed.doc);
    assert_eq!(p.categories, ["A", "B"]);
    assert_eq!(p.series[1].name, "y");
    assert_eq!(p.series[1].values, [6.0, 4.0]);
    // csv works as for tables.
    let id = run(&mut ed, json!({"op": "add_chart", "chart_type": "pie", "csv": ",Share\nOne,3\nTwo,5\n"})).unwrap().created[0].clone();
    assert_eq!(values(&ed, &id), [[3.0, 5.0]]);
}

#[test]
fn set_chart_data_types_grows_and_cuts() {
    let mut ed = Editor::default();
    let id = run(&mut ed, json!({"op": "add_chart"})).unwrap().created[0].clone();
    run(&mut ed, json!({"op": "set_chart_data", "id": id, "at": "B2", "values": [[2000]]})).unwrap();
    assert_eq!(values(&ed, &id)[0][0], 2000.0);
    assert_eq!(label(&ed), "Edit chart data");
    // Past the edge: grows.
    run(&mut ed, json!({"op": "set_chart_data", "id": id, "at": "D1", "values": [["Q4"], [5]]})).unwrap();
    assert_eq!(chart(&ed, &id).data.n_cols(), 4);
    assert_eq!(chart(&ed, &id).plot(&ed.doc).series[2].name, "Q4");
    // Size set: cut.
    run(&mut ed, json!({"op": "set_chart_data", "id": id, "rows": 3, "columns": 2})).unwrap();
    let p = chart(&ed, &id).plot(&ed.doc);
    assert_eq!((p.series.len(), p.categories.len()), (1, 2));
    // Replaced.
    run(&mut ed, json!({"op": "set_chart_data", "id": id, "data": [["", "S"], ["a", 1], ["b", 2], ["c", 3]]})).unwrap();
    assert_eq!(values(&ed, &id), [[1.0, 2.0, 3.0]]);
    // Limits and nonsense refused, nothing changed.
    let before = chart(&ed, &id);
    for bad in [
        json!({"op": "set_chart_data", "id": id, "rows": 0}),
        json!({"op": "set_chart_data", "id": id, "rows": 10_001, "columns": 2}),
        json!({"op": "set_chart_data", "id": id, "at": "nope", "values": [[1]]}),
        json!({"op": "set_chart_data", "id": id, "values": []}),
        json!({"op": "set_chart_data", "id": id}),
        json!({"op": "set_chart_data", "id": id, "values": [[1]], "data": [[1]]}),
        json!({"op": "set_chart_data", "id": id, "at": "ZZZ9999", "values": [[1]]}),
    ] {
        assert!(run(&mut ed, bad.clone()).is_err(), "{bad}");
        assert_eq!(chart(&ed, &id), before, "{bad}");
    }
    assert!(run(&mut ed, json!({"op": "set_chart_data", "id": "b99", "values": [[1]]})).is_err());
}

#[test]
fn format_chart_sets_and_checks_every_option() {
    let mut ed = Editor::default();
    let id = run(&mut ed, json!({"op": "add_chart"})).unwrap().created[0].clone();
    run(&mut ed, json!({"op": "format_chart", "id": id, "chart_type": "bar"})).unwrap();
    assert_eq!(label(&ed), "Change chart type");
    run(&mut ed, json!({"op": "format_chart", "id": id, "look": "tints"})).unwrap();
    assert_eq!(label(&ed), "Change chart style");
    run(
        &mut ed,
        json!({"op": "format_chart", "id": id, "title": "Sales", "legend": false, "border": true, "font_size": 1.5,
               "value_labels": true, "gridlines": false, "gap": 0.2, "gap_sets": 1.0, "hole": 0.6,
               "axis_min": 0, "axis_max": 2000, "axis_step": 500, "number": {"type": "currency", "decimals": 0},
               "x_title": "Region", "y_title": "£k",
               "series": [{"index": 1, "color": "theme:shape4", "labels": true, "line_width": 3, "markers": true, "smooth": true, "as_line": true}]}),
    )
    .unwrap();
    assert_eq!(label(&ed), "Format chart");
    let c = chart(&ed, &id);
    assert_eq!((c.title.as_str(), c.legend, c.border, c.font_size), ("Sales", false, true, 1.5));
    assert_eq!((c.axis_min, c.axis_max, c.axis_step), (Some(0.0), Some(2000.0), Some(500.0)));
    assert_eq!(c.series.len(), 2);
    assert!(c.series[0].is_empty());
    assert_eq!(c.series[1].color.as_deref(), Some("theme:shape4"));
    // "auto" and "" reset.
    run(&mut ed, json!({"op": "format_chart", "id": id, "axis_max": "auto", "series": [{"index": 1, "color": "", "line_width": 0, "labels": null}]})).unwrap();
    let c = chart(&ed, &id);
    assert_eq!(c.axis_max, None);
    assert_eq!(c.series[1].color, None);
    assert_eq!(c.series[1].line_width, None);
    let before = chart(&ed, &id);
    for bad in [
        json!({"op": "format_chart", "id": id, "font_size": 0}),
        json!({"op": "format_chart", "id": id, "gap": 2}),
        json!({"op": "format_chart", "id": id, "hole": 1}),
        json!({"op": "format_chart", "id": id, "axis_min": 5000, "axis_max": 10}),
        json!({"op": "format_chart", "id": id, "axis_step": -1}),
        json!({"op": "format_chart", "id": id, "series": [{"index": 0, "color": "chartreuse"}]}),
        json!({"op": "format_chart", "id": id, "series": [{"index": 64}]}),
        json!({"op": "format_chart", "id": id, "series": [{"index": 0, "line_width": 1000}]}),
        json!({"op": "format_chart", "id": id, "number": {"decimals": 99}}),
        json!({"op": "format_chart", "id": id, "totals": [5000]}),
    ] {
        assert!(run(&mut ed, bad.clone()).is_err(), "{bad}");
        assert_eq!(chart(&ed, &id), before, "{bad}");
    }
    assert!(serde_json::from_value::<Command>(json!({"op": "format_chart", "id": id, "chart_type": "3d_pie"})).is_err());
    // Not a chart.
    let t = run(&mut ed, json!({"op": "add_table"})).unwrap().created[0].clone();
    let e = run(&mut ed, json!({"op": "format_chart", "id": t, "legend": false})).unwrap_err();
    assert!(e.message.contains("not a chart"), "{}", e.message);
}

#[test]
fn a_new_waterfall_ticks_its_last_point_as_the_total() {
    let mut ed = Editor::default();
    let id = run(&mut ed, json!({"op": "add_chart", "chart_type": "waterfall"})).unwrap().created[0].clone();
    let c = chart(&ed, &id);
    assert_eq!(c.totals, [4]);
    let id = run(&mut ed, json!({"op": "add_chart"})).unwrap().created[0].clone();
    run(&mut ed, json!({"op": "format_chart", "id": id, "chart_type": "waterfall"})).unwrap();
    assert_eq!(chart(&ed, &id).totals, [3]);
}

#[test]
fn a_linked_chart_follows_its_table() {
    let mut ed = sales();
    let id = run(&mut ed, json!({"op": "add_chart", "from_table": "b1", "link": true})).unwrap().created[0].clone();
    let c = chart(&ed, &id);
    assert_eq!(c.linked_table(), Some("b1"));
    let p = c.plot(&ed.doc);
    // Footer rows left out; header row and the text column name things.
    assert_eq!(p.categories, ["North", "South", "East"]);
    assert_eq!(p.series[1].name, "Q3");
    assert_eq!(p.series[1].values, [1410.0, 1050.0, 702.0]);
    // Typing in the table changes what's drawn; the chart is reported changed.
    let out = run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "B3", "values": [[1000]]})).unwrap();
    assert!(out.changed.contains(&id), "{out:?}");
    assert_eq!(values(&ed, &id)[1], [1410.0, 1070.0, 702.0], "the formula followed");
    // Its own data can't be typed into while linked.
    let e = run(&mut ed, json!({"op": "set_chart_data", "id": id, "values": [[1]]})).unwrap_err();
    assert_eq!(e.code, "linked");
    // A row added to the table body shows.
    run(&mut ed, json!({"op": "insert_rows", "id": "b1", "index": 4})).unwrap();
    run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "A5", "values": [["West", 1105, 1230]]})).unwrap();
    assert_eq!(chart(&ed, &id).plot(&ed.doc).categories, ["North", "South", "East", "West"]);
}

#[test]
fn a_linked_range_moves_with_its_cells() {
    let mut ed = sales();
    let id = run(&mut ed, json!({"op": "add_chart", "from_table": "b1", "range": "A1:B4", "link": true})).unwrap().created[0].clone();
    let range = |ed: &Editor| chart(ed, &id).link.unwrap().range.unwrap().a1();
    run(&mut ed, json!({"op": "insert_rows", "id": "b1", "index": 0})).unwrap();
    assert_eq!(range(&ed), "A2:B5");
    run(&mut ed, json!({"op": "insert_columns", "id": "b1", "index": 1})).unwrap();
    assert_eq!(range(&ed), "A2:C5", "a column inside it widens it");
    assert_eq!(values(&ed, &id)[1], [1240.0, 980.0, 760.0]);
    // One undo step for the table edit and the moved range together.
    ed.undo(Origin::Human).unwrap();
    assert_eq!(range(&ed), "A2:B5");
    ed.undo(Origin::Human).unwrap();
    assert_eq!(range(&ed), "A1:B4");
    // Deleting every cell it drew: the chart keeps the values it showed.
    run(&mut ed, json!({"op": "delete_columns", "id": "b1", "index": 0, "count": 2})).unwrap();
    let c = chart(&ed, &id);
    assert!(c.link.is_none());
    assert_eq!(values(&ed, &id), [[1240.0, 980.0, 760.0]]);
    ed.undo(Origin::Human).unwrap();
    assert_eq!(range(&ed), "A1:B4");
}

#[test]
fn deleting_the_table_keeps_the_values_and_undo_relinks() {
    let mut ed = sales();
    let id = run(&mut ed, json!({"op": "add_chart", "from_table": "b1", "link": true})).unwrap().created[0].clone();
    let before = values(&ed, &id);
    let out = run(&mut ed, json!({"op": "delete", "ids": ["b1"]})).unwrap();
    assert!(out.changed.contains(&id));
    let c = chart(&ed, &id);
    assert!(c.link.is_none());
    assert_eq!(values(&ed, &id), before);
    assert_eq!(c.plot(&ed.doc).categories, ["North", "South", "East"]);
    assert!(c.data.cells.iter().flatten().all(|c| !c.is_formula()), "formulas became their values");
    ed.undo(Origin::Human).unwrap();
    assert_eq!(chart(&ed, &id).linked_table(), Some("b1"));
    // Deleting both together is fine too.
    run(&mut ed, json!({"op": "delete", "ids": ["b1", id]})).unwrap();
    assert!(ed.doc.elements.is_empty());
}

#[test]
fn copies_without_the_table_keep_values_and_with_it_follow_the_copy() {
    let mut ed = sales();
    let id = run(&mut ed, json!({"op": "add_chart", "from_table": "b1", "link": true})).unwrap().created[0].clone();
    let before = values(&ed, &id);
    // Copy the chart alone to another deck.
    let clip = clip::Clip::of(&ed, std::slice::from_ref(&id));
    let mut other = Editor::default();
    let out = other.paste_clip(&clip, 0.0, 0.0, None, Origin::Human).unwrap();
    let pasted = &out.created[0];
    assert!(chart(&other, pasted).link.is_none());
    assert_eq!(values(&other, pasted), before);
    // The chart and its table together: the copy follows the copied table.
    let clip = clip::Clip::of(&ed, &["b1".to_string(), id.clone()]);
    let out = other.paste_clip(&clip, 0.0, 0.0, None, Origin::Human).unwrap();
    let (t, c) = (&out.created[0], &out.created[1]);
    assert_eq!(chart(&other, c).linked_table(), Some(t.as_str()));
    // Duplicate with the table, and without.
    let out = run(&mut ed, json!({"op": "duplicate", "ids": ["b1", id]})).unwrap();
    let (t2, c2) = (&out.created[0], &out.created[1]);
    assert_eq!(chart(&ed, c2).linked_table(), Some(t2.as_str()));
    let out = run(&mut ed, json!({"op": "duplicate", "ids": [id]})).unwrap();
    assert_eq!(chart(&ed, &out.created[0]).linked_table(), Some("b1"));
}

#[test]
fn link_and_unlink() {
    let mut ed = sales();
    let id = run(&mut ed, json!({"op": "add_chart"})).unwrap().created[0].clone();
    let own = chart(&ed, &id).data.clone();
    run(&mut ed, json!({"op": "link_chart", "id": id, "table": "b1", "range": "A1:B4"})).unwrap();
    assert_eq!(label(&ed), "Link to table");
    assert_eq!(values(&ed, &id), [[1240.0, 980.0, 760.0]]);
    run(&mut ed, json!({"op": "unlink_chart", "id": id})).unwrap();
    assert_eq!(label(&ed), "Unlink chart");
    assert!(chart(&ed, &id).link.is_none());
    assert_eq!(values(&ed, &id), [[1240.0, 980.0, 760.0]], "kept what it showed");
    ed.undo(Origin::Human).unwrap();
    ed.undo(Origin::Human).unwrap();
    assert_eq!(chart(&ed, &id).data, own);
    for bad in [
        json!({"op": "link_chart", "id": id, "table": "b9"}),
        json!({"op": "link_chart", "id": id, "table": id}),
        json!({"op": "link_chart", "id": id, "table": "b1", "range": "Z90:Z99"}),
        json!({"op": "unlink_chart", "id": id}),
        json!({"op": "add_chart", "from_table": id}),
        json!({"op": "add_chart", "link": true}),
        json!({"op": "add_chart", "from_table": "b1", "data": [[1]]}),
    ] {
        assert!(run(&mut ed, bad.clone()).is_err(), "{bad}");
    }
    // From a table without link: a copy of its cells.
    let c = run(&mut ed, json!({"op": "add_chart", "from_table": "b1", "range": "A1:C4"})).unwrap().created[0].clone();
    assert!(chart(&ed, &c).link.is_none());
    assert_eq!(values(&ed, &c)[1], [1410.0, 1050.0, 702.0]);
}

#[test]
fn a_table_without_names_gets_numbered_categories_and_series() {
    let mut ed = Editor::default();
    run(&mut ed, json!({"op": "add_table", "id": "b1", "header_rows": 0, "cells": [[1, 2], [3, 4], [5, 6]]})).unwrap();
    let id = run(&mut ed, json!({"op": "add_chart", "from_table": "b1", "link": true})).unwrap().created[0].clone();
    let p = chart(&ed, &id).plot(&ed.doc);
    assert_eq!(p.categories, ["1", "2", "3"]);
    assert_eq!(p.series[0].name, "Series 1");
    assert_eq!(p.series[1].values, [2.0, 4.0, 6.0]);
    // Unlinked: the names come along, so it draws the same.
    run(&mut ed, json!({"op": "unlink_chart", "id": id})).unwrap();
    let q = chart(&ed, &id).plot(&ed.doc);
    assert_eq!(q.series.iter().map(|s| s.values.clone()).collect::<Vec<_>>(), p.series.iter().map(|s| s.values.clone()).collect::<Vec<_>>());
    assert_eq!(q.categories, p.categories);
}

#[test]
fn a_link_to_a_huge_table_draws_the_first_part() {
    let mut ed = Editor::default();
    // 5,001 rows x 20 columns: 100,020 cells is over a table's limit, so 4,999.
    let rows: Vec<Vec<serde_json::Value>> = (0..4999).map(|r| (0..20).map(|c| json!(r * 20 + c)).collect()).collect();
    run(&mut ed, json!({"op": "add_table", "id": "b1", "header_rows": 0, "cells": rows})).unwrap();
    let id = run(&mut ed, json!({"op": "add_chart", "from_table": "b1", "link": true})).unwrap().created[0].clone();
    let t = std::time::Instant::now();
    let p = chart(&ed, &id).plot(&ed.doc);
    assert!(t.elapsed().as_millis() < 2000);
    assert!(p.cut);
    let cells = (p.categories.len() + 1) * (p.series.len() + 1);
    assert!(cells <= chart::MAX_VALUES, "{cells}");
    assert!(p.series.len() <= chart::MAX_SERIES);
    // It says what it left out.
    assert_eq!((p.all_categories, p.all_series), (4999, 20));
    let q = describe::query(&ed, Query::Chart { id: id.clone() }).unwrap();
    assert_eq!(q["drawn"]["categories"], json!(p.categories.len()));
    assert_eq!(q["drawn"]["of_categories"], json!(4999));
    let d = describe::describe(&ed);
    assert!(d.contains("…and"), "{d}");
    // Unlinking keeps all of it: a chart's data holds what a table can.
    run(&mut ed, json!({"op": "unlink_chart", "id": id})).unwrap();
    let c = chart(&ed, &id);
    assert!(c.check().is_ok());
    assert_eq!((c.data.n_rows(), c.data.n_cols()), (4999, 20));
    assert!(c.plot(&ed.doc).cut);
    // Own data takes what a table takes (10,100 cells is fine)...
    let big: Vec<Vec<i32>> = vec![vec![1; 101]; 100];
    run(&mut ed, json!({"op": "add_chart", "data": big})).unwrap();
    // ...and over a table's limit is refused, with the table's reason.
    let huge: Vec<Vec<i32>> = vec![vec![1; 257]; 2];
    let err = run(&mut ed, json!({"op": "add_chart", "data": huge})).unwrap_err();
    assert_eq!(err.code, "too_big");
    assert!(err.message.contains("256 columns"), "{}", err.message);
}

#[test]
fn the_chart_query_and_describe() {
    let mut ed = sales();
    let id = run(&mut ed, json!({"op": "add_chart", "chart_type": "line", "title": "Sales by Region", "from_table": "b1", "link": true})).unwrap().created[0].clone();
    let q = ed.query(Query::Chart { id: id.clone() }).unwrap();
    assert_eq!(q["chart_type"], "line");
    assert_eq!(q["chart_type_code"], "CT6");
    assert_eq!(q["source"]["table"], "b1");
    assert_eq!(q["source"]["range"], "A1:C4");
    assert_eq!(q["categories"], json!(["North", "South", "East"]));
    assert_eq!(q["series"][0]["values"], json!([1240.0, 980.0, 760.0]));
    let own = run(&mut ed, json!({"op": "add_chart", "data": [["", "S"], ["a", "=1+1"], ["b", "x"]]})).unwrap().created[0].clone();
    let q = ed.query(Query::Chart { id: own.clone() }).unwrap();
    assert_eq!(q["formulas"]["B2"], "=1+1");
    assert_eq!(q["series"][0]["values"], json!([2.0, null]));
    assert!(ed.query(Query::Chart { id: "b1".into() }).is_err());
    let d = describe::describe(&ed);
    assert!(d.contains(&format!("{id} chart")) || d.contains(&id), "{d}");
    assert!(d.contains("line title=\"Sales by Region\" 2 series x 3 categories linked to b1"), "{d}");
    assert!(d.contains("Q2: 1240, 980, 760"), "{d}");
    // Found by its words.
    let found = ed.query(Query::Elements { kind: Some("chart".into()), text: Some("south".into()), within: None, visible: false }).unwrap();
    assert_eq!(found["elements"].as_array().unwrap().len(), 1);
}

#[test]
fn locked_charts_refuse_changes() {
    let mut ed = Editor::default();
    let id = run(&mut ed, json!({"op": "add_chart"})).unwrap().created[0].clone();
    run(&mut ed, json!({"op": "update", "id": id, "patch": {"locked": true}})).unwrap();
    assert!(run(&mut ed, json!({"op": "format_chart", "id": id, "legend": false})).is_err());
    assert!(run(&mut ed, json!({"op": "set_chart_data", "id": id, "values": [[1]]})).is_err());
}

fn file_with_chart(kind: serde_json::Value) -> String {
    json!({
        "format_version": 3, "title": "t", "theme": "classic", "next_id": 5, "z_order": ["h1", "b1"], "path": [],
        "elements": {
            "h1": {"id": "h1", "transform": {"x": 0, "y": 0, "scale": 1, "rotation": 0}, "size": {"w": 640, "h": 440}, "kind": kind},
            "b1": {"id": "b1", "transform": {"x": 0, "y": 0, "scale": 1, "rotation": 0}, "size": {"w": 10, "h": 10}, "kind": {"type": "table", "cells": [["", "a"], ["x", "1"]], "columns": [], "rows": []}}
        }
    })
    .to_string()
}

#[test]
fn hostile_chart_files_load_repaired() {
    let d = json!({"cells": [["", "a"], ["x", "1"]], "columns": [], "rows": []});
    let cases = [
        json!({"type": "chart", "chart_type": "hologram", "look": "glitter", "data": d}),
        json!({"type": "chart", "data": "everything", "series_in": 7, "link": {"table": 5}}),
        json!({"type": "chart"}),
        json!({"type": "chart", "data": d, "link": {"table": "h1"}}),
        json!({"type": "chart", "data": d, "link": {"table": "b404"}}),
        json!({"type": "chart", "data": d, "link": {"table": "b1", "range": "C9:Z99"}}),
        json!({"type": "chart", "data": d, "font_size": null, "gap": "wide", "hole": 1e308, "axis_min": 9, "axis_max": 1, "axis_step": -0.0, "totals": [-1, "x", 1e20, 2]}),
        json!({"type": "chart", "data": d, "series": [{"color": "no"}, {"line_width": -1}, 5, "s", {"color": "theme:shape9"}], "number": "money"}),
        json!({"type": "chart", "data": d, "series": "all", "title": 5, "legend": "yes"}),
        json!({"type": "chart", "data": {"cells": [[{"input": "=1/0", "value": {"error": 5}}]], "columns": 1}}),
    ];
    for k in cases {
        let json = file_with_chart(k.clone());
        let doc = io::from_json(&json).unwrap_or_else(|e| panic!("{k}: {}", e.message));
        let mut ed = Editor::default();
        ed.load(doc, None);
        let e = ed.doc.get("h1").unwrap_or_else(|| panic!("{k}: chart dropped"));
        check::element(e).unwrap_or_else(|err| panic!("{k}: repaired chart still broken: {}", err.message));
        let ElementKind::Chart(c) = &e.kind else { panic!() };
        if let Some(l) = &c.link {
            assert_eq!(l.table, "b1", "{k}");
            assert!(l.range.is_none(), "{k}: clipped away");
        }
        let _ = c.plot(&ed.doc);
        let _ = describe::describe(&ed);
    }
    // A million cells in a chart's data: cut to a table's limit.
    let row: Vec<&str> = vec!["1"; 1000];
    let cells: Vec<Vec<&str>> = vec![row; 1000];
    let json = file_with_chart(json!({"type": "chart", "data": {"cells": cells, "columns": [], "rows": []}}));
    let doc = io::from_json(&json).unwrap();
    let ElementKind::Chart(c) = &doc.elements["h1"].kind else { panic!() };
    assert!(c.data.n_rows() * c.data.n_cols() <= zebra_core::table::MAX_CELLS);
    assert!(c.check().is_ok());
}

#[test]
fn every_type_and_hostile_data_extract_without_panicking() {
    let nasty = [
        json!([[""]]),
        json!([["only text", "x"], ["y", "z"]]),
        json!([["", "a"], ["x", 1e308], ["y", -1e308], ["z", 5e-324]]),
        json!([["", "a", "b", "c"], ["x", "=1/0", "=NA()", "#REF!"]]),
        json!([["", "a"], ["x", 0], ["y", 0]]),
        json!([["", "a"], ["x", -5], ["y", -1]]),
        json!([["🇬🇧🇬🇧🇬🇧".repeat(100), "e\u{301}".repeat(500)], ["\u{202e}rtl", 3]]),
    ];
    for t in ChartType::ALL {
        for data in &nasty {
            let mut ed = Editor::default();
            let id = run(&mut ed, json!({"op": "add_chart", "chart_type": t.key(), "data": data})).unwrap().created[0].clone();
            let p = chart(&ed, &id).plot(&ed.doc);
            for s in &p.series {
                assert!(s.name.chars().count() < 2000);
                assert!(chart::label(&s.name).graphemes_ok());
            }
            for si in [SeriesIn::Rows, SeriesIn::Columns] {
                run(&mut ed, json!({"op": "format_chart", "id": id, "series_in": si})).unwrap();
                let _ = chart(&ed, &id).plot(&ed.doc);
            }
        }
    }
}

trait GraphemesOk {
    fn graphemes_ok(&self) -> bool;
}
impl GraphemesOk for String {
    fn graphemes_ok(&self) -> bool {
        use unicode_segmentation::UnicodeSegmentation;
        self.graphemes(true).count() <= chart::MAX_LABEL
    }
}
