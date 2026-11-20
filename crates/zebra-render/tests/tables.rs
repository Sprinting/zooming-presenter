//! Tables drawn through the display list: styles, only what's on screen,
//! far away, `###`, errors, and how long a frame takes.
mod common;

use std::time::Instant;

use common::*;
use serde_json::json;
use zebra_core::table::TableStyle;
use zebra_core::*;
use zebra_render::display::Item;
use zebra_render::{CaptureTarget, Renderer, SceneOptions};

fn run(ed: &mut Editor, j: serde_json::Value) {
    ed.apply(serde_json::from_value(j).unwrap(), Origin::Human).unwrap();
}

fn editor(r: &Renderer, theme: &str) -> Editor {
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.doc.theme = theme.into();
    ed.view.viewport = Vec2::new(800.0, 500.0);
    ed
}

fn sales(ed: &mut Editor, style: &str) {
    run(
        ed,
        json!({"op": "add_table", "id": "b1", "footer_rows": 1, "style": style, "scale": 1.0, "cells": [
            ["Region", "Q2", "Q3", "Change"],
            ["North", "£1,240", "£1,410", "13.7%"],
            ["South", "£980", "£1,050", "7.1%"],
            ["East", "£760", "£702", "-7.6%"],
            ["West", "£1,105", "£1,230", "11.3%"],
            ["Total", "=SUM(B2:B5)", "=SUM(C2:C5)", "=C6/B6-1"]
        ]}),
    );
}

/// `rows` x `cols` of numbers under a header, totals in a footer.
fn big(ed: &mut Editor, rows: usize, cols: usize) {
    let text: String = (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| match (r, c) {
                    (0, c) => format!("Col {c}"),
                    (r, c) if r + 1 == rows => format!("=SUM({l}2:{l}{n})", l = table::column_name(c), n = rows - 1),
                    (r, 0) => format!("Item {r}"),
                    (r, c) => ((r * 7 + c * 13) % 1000).to_string(),
                })
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n");
    run(ed, json!({"op": "add_table", "id": "b1", "csv": text, "footer_rows": 1, "scale": 1.0}));
}

fn texts(list: &zebra_render::display::DisplayList) -> Vec<String> {
    list.items
        .iter()
        .filter_map(|i| match i {
            Item::Text { layout, .. } => Some(layout.text.to_string()),
            _ => None,
        })
        .collect()
}

fn camera_on(ed: &Editor, zoom: f64, dx: f64, dy: f64) -> Camera {
    let e = ed.doc.get("b1").unwrap();
    let (w, h) = e.world_size();
    Camera { cx: e.transform.x - w / 2.0 + dx, cy: e.transform.y - h / 2.0 + dy, zoom, rotation: 0.0 }
}

#[test]
fn every_style_draws_in_classic_and_night() {
    let r = Renderer::new();
    let shots = std::env::var("ZEBRA_SHOTS").ok().map(std::path::PathBuf::from);
    for theme in ["classic", "night"] {
        let th = Theme::resolve(theme);
        for s in TableStyle::ALL {
            let mut ed = editor(&r, theme);
            sales(&mut ed, s.id());
            let png = r.capture(&ed, &CaptureTarget::Element { id: "b1".into() }, 720, 400).unwrap();
            let name = format!("style_{theme}_{}.png", s.id());
            std::fs::write(out_dir().join(&name), &png).unwrap();
            if let Some(d) = &shots {
                let _ = std::fs::create_dir_all(d);
                let _ = std::fs::write(d.join(&name), &png);
            }
            let img = image::load_from_memory(&png).unwrap().to_rgba8();
            let bg = th.color(&th.background);
            let ink = img.pixels().filter(|p| p.0.iter().zip(bg).any(|(a, b)| (*a as i32 - b as i32).abs() > 24)).count();
            assert!(ink > 3000, "{theme} {}: looks empty ({ink} pixels)", s.id());
            // Every cell's text is there, the totals worked out.
            let cam = camera_on(&ed, 1.0, 360.0, 160.0);
            let list = r.build_display_list(&ed, &cam, [720.0, 400.0], 1.0, &SceneOptions::default());
            let t = texts(&list);
            assert!(t.iter().any(|x| x == "Region") && t.iter().any(|x| x.contains("4,085")), "{theme} {}: {t:?}", s.id());
        }
    }
}

#[test]
fn a_number_too_wide_shows_hashes_and_an_error_its_text() {
    let r = Renderer::new();
    let mut ed = editor(&r, "classic");
    run(&mut ed, json!({"op": "add_table", "id": "b1", "scale": 1.0, "cells": [["A", "B"], ["123456789012345", "=1/0"]]}));
    run(&mut ed, json!({"op": "set_column", "id": "b1", "column": 0, "width": 40}));
    let cam = camera_on(&ed, 1.0, 200.0, 50.0);
    let list = r.build_display_list(&ed, &cam, [400.0, 200.0], 1.0, &SceneOptions::default());
    let t = texts(&list);
    assert!(t.contains(&"###".to_string()), "{t:?}");
    assert!(!t.iter().any(|x| x.contains("1234567")), "{t:?}");
    assert!(t.contains(&"#DIV/0!".to_string()), "{t:?}");
}

#[test]
fn only_rows_on_screen_are_laid_out_and_far_away_is_bars() {
    let r = Renderer::new();
    let mut ed = editor(&r, "classic");
    big(&mut ed, 5000, 20);
    let opts = SceneOptions::default();
    let vp = [1280.0, 800.0];
    let mut report = String::new();

    // Close up on the top-left corner: a screenful of cells.
    let cam = camera_on(&ed, 1.0, 640.0, 400.0);
    let t0 = Instant::now();
    let list = r.build_display_list(&ed, &cam, vp, 2.0, &opts);
    let first = t0.elapsed();
    let t0 = Instant::now();
    let list2 = r.build_display_list(&ed, &cam, vp, 2.0, &opts);
    let again = t0.elapsed();
    let n_text = texts(&list).len();
    assert!(n_text > 50 && n_text < 1000, "close up laid out {n_text} texts");
    assert_eq!(texts(&list2).len(), n_text);
    report += &format!("close up: {n_text} texts, {} items, first frame {first:?}, next {again:?}\n", list.items.len());

    // Middle of the table: only its rows.
    let e = ed.doc.get("b1").unwrap();
    let cam = Camera { cx: e.transform.x - e.world_size().0 / 2.0 + 640.0, cy: e.transform.y, zoom: 1.0, rotation: 0.0 };
    let list = r.build_display_list(&ed, &cam, vp, 2.0, &opts);
    let t = texts(&list);
    assert!(t.iter().any(|x| x.starts_with("Item 25")), "middle rows: {:?}", &t[..t.len().min(10)]);
    assert!(!t.iter().any(|x| x == "Item 3"), "a row far above the screen was laid out");

    // Rows a few pixels high: no text laid out, bars instead.
    let (w, h) = e.world_size();
    let zoom = 3.0 / (h / 5000.0);
    let cam = Camera { cx: e.transform.x - w / 2.0 + 400.0 / zoom, cy: e.transform.y - h / 2.0 + 400.0 / zoom, zoom, rotation: 0.0 };
    let t0 = Instant::now();
    let list = r.build_display_list(&ed, &cam, vp, 1.0, &opts);
    let took = t0.elapsed();
    assert_eq!(texts(&list).len(), 0, "text laid out at 3 px a row");
    assert!(list.items.len() > 100, "bars drawn: {}", list.items.len());
    report += &format!("3 px rows: {} items, {took:?}\n", list.items.len());

    // The whole table on screen: a few blocks.
    let zoom = 700.0 / h;
    let cam = Camera { cx: e.transform.x, cy: e.transform.y, zoom, rotation: 0.0 };
    let t0 = Instant::now();
    let list = r.build_display_list(&ed, &cam, vp, 1.0, &opts);
    let took = t0.elapsed();
    assert_eq!(texts(&list).len(), 0);
    assert!(list.items.len() < 200, "whole table drew {} items", list.items.len());
    report += &format!("whole table: {} items, {took:?}\n", list.items.len());
    std::fs::write(out_dir().join("table_frames.txt"), &report).unwrap();
    eprintln!("{report}");
}

#[test]
fn hostile_tables_draw_without_panicking() {
    let r = Renderer::new();
    let mut ed = editor(&r, "classic");
    run(&mut ed, json!({"op": "add_table", "id": "b1", "rows": 3, "columns": 3}));
    let cam = camera_on(&ed, 1.0, 100.0, 50.0);
    // Columns of no width, a huge font, rotated and at extreme zooms.
    run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "A1", "values": [["x".repeat(5000), "=1/0", "-12"]]}));
    run(&mut ed, json!({"op": "set_column", "id": "b1", "column": 1, "width": 0.01}));
    for zoom in [1e-9, 1e-3, 1.0, 1e3, 1e9] {
        let _ = r.build_display_list(&ed, &Camera { zoom, ..cam }, [400.0, 300.0], 2.0, &SceneOptions::default());
        let _ = r.build_display_list(&ed, &Camera { zoom, rotation: 0.7, ..cam }, [400.0, 300.0], 2.0, &SceneOptions::default());
    }
    let _ = r.capture(&ed, &CaptureTarget::Element { id: "b1".into() }, 200, 100).unwrap();
}
