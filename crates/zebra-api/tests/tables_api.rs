//! Tables over the socket (spec 19a): every table command, the `table`
//! query, describe, strict fields, limits and undo.
use std::path::PathBuf;
use std::sync::Once;

use serde_json::{Value, json};
use zebra_api::client::{Client, ClientError};
use zebra_api::{ApiOptions, HeadlessHandle, Permission};

fn start() -> HeadlessHandle {
    static INIT: Once = Once::new();
    let dir: PathBuf = std::env::temp_dir().join(format!("zebra-api-tables-{}", std::process::id()));
    INIT.call_once(|| {
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once before any server starts in this test binary.
        unsafe { std::env::set_var("ZEBRA_DATA_DIR", dir.join("data")) };
    });
    HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Edit }, None).expect("instance starts")
}

fn apply(c: &mut Client, cmd: Value) -> Value {
    c.call("apply", json!({ "command": cmd })).unwrap_or_else(|e| panic!("{cmd}: {e:?}"))
}

fn table(c: &mut Client, id: &str) -> Value {
    c.call("query", json!({"query": {"q": "table", "id": id}})).unwrap()
}

fn refused(r: Result<Value, ClientError>) -> zebra_api::RpcError {
    match r {
        Err(ClientError::Rpc(e)) => e,
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

fn sales(c: &mut Client) {
    apply(
        c,
        json!({"op": "add_table", "id": "b1", "footer_rows": 1, "name": "Sales", "cells": [
            ["Region", "Q2", "Q3"],
            ["North", "£1,240", 1410],
            ["South", "£980", 1050],
            ["East", "£760", 702],
            ["Total", "=SUM(B2:B4)", "=SUM(C2:C4)"]
        ]}),
    );
}

#[test]
fn every_table_command_works_and_undoes() {
    let h = start();
    let mut c = Client::connect_hello(&h.socket, "tables", &h.token).unwrap();
    sales(&mut c);
    let t = table(&mut c, "b1");
    assert_eq!(t["shown"][4][1], "£2,980", "{t}");
    assert_eq!(t["formulas"]["B5"], "=SUM(B2:B4)");
    assert_eq!(t["style"], "ts4");

    apply(&mut c, json!({"op": "set_cells", "id": "b1", "at": "B2", "values": [["£2,000"]]}));
    assert_eq!(table(&mut c, "b1")["shown"][4][1], "£3,740");

    apply(&mut c, json!({"op": "insert_rows", "id": "b1", "index": 4, "count": 1}));
    let t = table(&mut c, "b1");
    assert_eq!(t["rows"], 6);
    assert_eq!(t["formulas"]["B6"], "=SUM(B2:B5)", "a row added at the end of the body joins the total");
    apply(&mut c, json!({"op": "set_cells", "id": "b1", "at": "A5", "values": [["West", 100, 200]]}));
    assert_eq!(table(&mut c, "b1")["shown"][5][2], "3362", "plain numbers, no separators");
    apply(&mut c, json!({"op": "delete_rows", "id": "b1", "index": 4}));
    assert_eq!(table(&mut c, "b1")["formulas"]["B5"], "=SUM(B2:B4)");

    apply(&mut c, json!({"op": "insert_columns", "id": "b1", "index": 1, "count": 1}));
    assert_eq!(table(&mut c, "b1")["formulas"]["C5"], "=SUM(C2:C4)");
    apply(&mut c, json!({"op": "delete_columns", "id": "b1", "index": 1}));
    apply(&mut c, json!({"op": "move_rows", "id": "b1", "from": 3, "to": 1}));
    assert_eq!(table(&mut c, "b1")["shown"][1][0], "East");
    apply(&mut c, json!({"op": "move_columns", "id": "b1", "from": 2, "to": 1}));
    assert_eq!(table(&mut c, "b1")["shown"][0][1], "Q3");
    apply(&mut c, json!({"op": "merge_cells", "id": "b1", "range": "A1:C1"}));
    assert_eq!(table(&mut c, "b1")["merges"][0], "A1:C1");
    apply(&mut c, json!({"op": "unmerge_cells", "id": "b1", "range": "A1"}));
    assert_eq!(table(&mut c, "b1")["merges"].as_array().unwrap().len(), 0);
    apply(&mut c, json!({"op": "format_cells", "id": "b1", "range": "B2:B4", "format": {"number": {"type": "currency", "decimals": 2}, "bold": true}}));
    let t = table(&mut c, "b1");
    assert!(t["shown"][1][1].as_str().unwrap().ends_with(".00"), "{t}");
    apply(&mut c, json!({"op": "format_cells", "id": "b1", "range": "B2:B4", "clear": true}));
    apply(&mut c, json!({"op": "set_column", "id": "b1", "column": 2, "width": 200, "format": {"type": "number", "decimals": 1}}));
    assert_eq!(table(&mut c, "b1")["column_info"][2]["width"], 200.0);
    apply(&mut c, json!({"op": "set_row", "id": "b1", "row": 0, "height": 80}));
    apply(&mut c, json!({"op": "sort_table", "id": "b1", "column": 0}));
    let t = table(&mut c, "b1");
    assert_eq!([&t["shown"][1][0], &t["shown"][2][0], &t["shown"][3][0], &t["shown"][4][0]], ["East", "North", "South", "Total"], "body sorted, footer stays");
    // Labelled, so it is its own undo step (an agent's quick edits merge).
    c.call("batch", json!({"label": "Restyle", "commands": [{"op": "update", "id": "b1", "patch": {"table_style": "ts2", "header_columns": 1, "banded": false, "gridlines": {"horizontal": true, "vertical": true, "outline": false}, "font_size": 1.25, "show_name": true}}]})).unwrap();
    let t = table(&mut c, "b1");
    assert_eq!((t["style"].as_str(), t["header_columns"].as_u64(), t["show_name"].as_bool()), (Some("ts2"), Some(1), Some(true)));

    // Describe and elements say what the table is.
    let d = c.call("describe", json!({})).unwrap();
    let text = d.to_string();
    assert!(text.contains("Sales") && text.contains("b1"), "{text}");
    let els = c.call("query", json!({"query": {"q": "elements", "kind": "table"}})).unwrap();
    assert!(els.to_string().contains("b1"), "{els}");

    // Undo takes back the restyle alone.
    c.call("undo", json!({})).unwrap();
    assert_eq!(table(&mut c, "b1")["style"], "ts4");
    h.stop().unwrap();
}

#[test]
fn a_batch_of_table_edits_is_one_undo_step() {
    let h = start();
    let mut c = Client::connect_hello(&h.socket, "tables", &h.token).unwrap();
    sales(&mut c);
    c.call("batch", json!({"label": "Fill in", "commands": [
        {"op": "set_cells", "id": "b1", "at": "A2", "values": [["N"]]},
        {"op": "insert_rows", "id": "b1", "index": 1},
        {"op": "sort_table", "id": "b1", "column": 1, "descending": true}
    ]}))
    .unwrap();
    c.call("undo", json!({})).unwrap();
    let t = table(&mut c, "b1");
    assert_eq!((t["rows"].as_u64(), t["shown"][1][0].as_str()), (Some(5), Some("North")));
    h.stop().unwrap();
}

#[test]
fn table_commands_refuse_unknown_fields_bad_input_and_the_too_big() {
    let h = start();
    let mut c = Client::connect_hello(&h.socket, "tables", &h.token).unwrap();
    sales(&mut c);
    let rev = c.call("state", json!({})).unwrap()["revision"].clone();
    let e = refused(c.call("apply", json!({"command": {"op": "set_cells", "id": "b1", "at": "A1", "values": [["x"]], "colour": "red"}})));
    assert_eq!(e.kind(), "invalid_params", "{e}");
    assert!(e.message.contains("unknown field `colour`"), "{e}");
    let e = refused(c.call("apply", json!({"command": {"op": "format_cells", "id": "b1", "range": "A1", "format": {"colour": "#ff0000"}}})));
    assert!(e.message.contains("colour"), "{e}");
    let e = refused(c.call("apply", json!({"command": {"op": "format_cells", "id": "b1", "range": "A1", "format": {"fill": "not a colour"}}})));
    assert_eq!(e.kind(), "invalid_params", "{e}");
    let e = refused(c.call("apply", json!({"command": {"op": "set_cells", "id": "b1", "at": "nowhere", "values": [["x"]]}})));
    assert_eq!(e.kind(), "invalid_params", "{e}");
    let e = refused(c.call("apply", json!({"command": {"op": "delete_rows", "id": "b1", "index": 9}})));
    assert_eq!(e.kind(), "invalid_params", "{e}");
    let e = refused(c.call("apply", json!({"command": {"op": "insert_columns", "id": "b1", "index": 0, "count": 1000}})));
    assert_eq!(e.data["code"].as_str().unwrap_or(e.kind()), "too_big", "{e:?}");
    let long = "x".repeat(zebra_core::table::MAX_INPUT + 1);
    let e = refused(c.call("apply", json!({"command": {"op": "set_cells", "id": "b1", "at": "A1", "values": [[long]]}})));
    assert!(e.message.contains("characters"), "{e}");
    let e = refused(c.call("query", json!({"query": {"q": "table", "id": "nope"}})));
    assert_eq!(e.kind(), "not_found", "{e}");
    assert_eq!(c.call("state", json!({})).unwrap()["revision"], rev, "nothing changed");
    h.stop().unwrap();
}

#[test]
fn csv_tables_and_functions_that_reach_out() {
    let h = start();
    let mut c = Client::connect_hello(&h.socket, "tables", &h.token).unwrap();
    apply(&mut c, json!({"op": "add_table", "id": "b2", "csv": "Item,Price\nTea,2.5\n\"Cake, lemon\",3\n"}));
    let t = table(&mut c, "b2");
    assert_eq!(t["shown"][2][0], "Cake, lemon");
    apply(&mut c, json!({"op": "set_cells", "id": "b2", "at": "C1", "values": [["=WEBSERVICE(\"http://example.com\")", "=INFO(\"system\")", "=HYPERLINK(\"http://example.com\",\"site\")"]]}));
    let t = table(&mut c, "b2");
    assert_eq!(t["errors"]["C1"], "#NAME?");
    assert_eq!(t["errors"]["D1"], "#NAME?");
    assert_eq!(t["shown"][0][4], "site", "HYPERLINK shows its text only");
    h.stop().unwrap();
}
