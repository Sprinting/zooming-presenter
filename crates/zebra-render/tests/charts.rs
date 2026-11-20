//! Charts drawn through the display list: every type in every look and
//! theme, contrast, far away, the cache, and hostile data.
mod common;

use common::*;
use kurbo::Shape;
use serde_json::json;
use zebra_core::chart::{ChartLook, ChartType};
use zebra_core::table_style::{contrast, luminance};
use zebra_core::*;
use zebra_render::display::{DisplayList, Item};
use zebra_render::{CaptureTarget, Renderer, SceneOptions};

fn run(ed: &mut Editor, j: serde_json::Value) -> Outcome {
    ed.apply(serde_json::from_value(j).unwrap(), Origin::Human).unwrap()
}

fn editor(r: &Renderer, theme: &str) -> Editor {
    let mut ed = Editor::default();
    ed.measure = r.measure();
    ed.doc.theme = theme.into();
    ed.view.viewport = Vec2::new(800.0, 500.0);
    ed
}

fn shot(name: &str, png: &[u8]) {
    std::fs::write(out_dir().join(name), png).unwrap();
    if let Ok(d) = std::env::var("ZEBRA_SHOTS") {
        let d = std::path::PathBuf::from(d);
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::write(d.join(name), png);
    }
}

fn camera_on(ed: &Editor, id: &str, zoom: f64) -> Camera {
    let e = ed.doc.get(id).unwrap();
    Camera { cx: e.transform.x, cy: e.transform.y, zoom, rotation: 0.0 }
}

fn list(r: &Renderer, ed: &Editor, id: &str, zoom: f64) -> DisplayList {
    r.build_display_list(ed, &camera_on(ed, id, zoom), [800.0, 600.0], 1.0, &SceneOptions::default())
}

fn texts(l: &DisplayList) -> Vec<String> {
    l.items
        .iter()
        .filter_map(|i| match i {
            Item::Text { layout, .. } => Some(layout.text.to_string()),
            _ => None,
        })
        .collect()
}

fn ink(png: &[u8], bg: [u8; 4]) -> usize {
    let img = image::load_from_memory(png).unwrap().to_rgba8();
    img.pixels().filter(|p| p.0.iter().zip(bg).take(3).any(|(a, b)| (*a as i32 - b as i32).abs() > 24)).count()
}

/// Every type in Flat, and every look on a column chart, in each theme:
/// something is drawn, the labels are there, no stand-in colour leaks.
#[test]
fn every_type_and_look_draws_in_every_theme() {
    let r = Renderer::new();
    let mut empty = vec![];
    for th in Theme::builtins() {
        let bg = th.color(&th.background);
        let mut cases: Vec<(ChartType, ChartLook)> = ChartType::ALL.iter().map(|t| (*t, ChartLook::Flat)).collect();
        cases.extend(ChartLook::ALL.iter().map(|l| (ChartType::Column, *l)));
        for t in ChartType::ALL {
            for l in ChartLook::ALL {
                if !cases.contains(&(*t, *l)) && th.id == "classic" {
                    cases.push((*t, *l));
                }
            }
        }
        for (t, l) in cases {
            let mut ed = editor(&r, &th.id);
            let id = run(&mut ed, json!({"op": "add_chart", "chart_type": t.key(), "look": l.key()})).created[0].clone();
            let png = r.capture(&ed, &CaptureTarget::Element { id: id.clone() }, 640, 440).unwrap();
            if l == ChartLook::Flat || t == ChartType::Column {
                shot(&format!("chart_{}_{}_{}.png", th.id, t.code(), l.code()), &png);
            }
            let n = ink(&png, bg);
            if n < if t == ChartType::Scatter { 1200 } else { 2500 } {
                empty.push(format!("{} {t:?} {l:?}: looks empty ({n})", th.id));
            }
            let lst = list(&r, &ed, &id, 1.0);
            let tx = texts(&lst);
            match t {
                ChartType::Scatter | ChartType::Bubble => assert!(tx.iter().any(|s| s == "Q2"), "{} {t:?}: {tx:?}", th.id),
                ChartType::Waterfall => assert!(tx.iter().any(|s| s == "Sales"), "{} {t:?}: {tx:?}", th.id),
                _ => assert!(tx.iter().any(|s| s == "North"), "{} {t:?} {l:?}: {tx:?}", th.id),
            }
            for i in &lst.items {
                let c = match i {
                    Item::Fill { color, .. } | Item::Stroke { color, .. } | Item::Text { color, .. } => *color,
                    _ => continue,
                };
                assert!(!(matches!(c[0], 1..=4) && c[2] == 0 && c[3] == 255 && (c[0] != 1 || c[1] < 64)), "stand-in colour {c:?} leaked");
            }
        }
    }
    assert!(empty.is_empty(), "{empty:#?}");
}

/// Text reads on the chart: 7:1 for the default chart, 4.5:1 for the
/// others; bars at 4.5:1 and 3:1.
#[test]
fn text_and_marks_meet_their_contrast() {
    let r = Renderer::new();
    for th in Theme::builtins() {
        let under = th.color(&th.background);
        for (t, l, text_ratio, mark_ratio) in [(ChartType::Column, ChartLook::Flat, 7.0, 4.5), (ChartType::Bar, ChartLook::Flat, 4.5, 3.0), (ChartType::Line, ChartLook::Tints, 4.5, 1.0)] {
            let mut ed = editor(&r, &th.id);
            let id = run(&mut ed, json!({"op": "add_chart", "chart_type": t.key(), "look": l.key()})).created[0].clone();
            let lst = list(&r, &ed, &id, 1.0);
            let mut marks = 0;
            for i in &lst.items {
                match i {
                    Item::Text { color, .. } => {
                        let c = contrast(*color, under);
                        assert!(c >= text_ratio - 1e-6, "{} {t:?}: text {color:?} at {c:.2}", th.id);
                    }
                    Item::Fill { color, path } if color[3] == 255 && path.bounding_box().area() > 50.0 => {
                        let c = contrast(*color, under);
                        if c < 1.05 {
                            continue; // the ground
                        }
                        marks += 1;
                        assert!(c >= mark_ratio - 1e-6, "{} {t:?}: mark {color:?} at {c:.2} (lum {:.3})", th.id, luminance(*color));
                    }
                    _ => {}
                }
            }
            assert!(marks > 0 || t == ChartType::Line, "{} {t:?}: no marks", th.id);
        }
    }
}

#[test]
fn far_away_charts_drop_their_text_then_become_a_block() {
    let r = Renderer::new();
    let mut ed = editor(&r, "classic");
    let id = run(&mut ed, json!({"op": "add_chart"})).created[0].clone();
    let near = list(&r, &ed, &id, 1.0);
    assert!(!texts(&near).is_empty());
    // 440 high at 0.08: 35 px, no text.
    let mid = list(&r, &ed, &id, 0.08);
    assert!(texts(&mid).is_empty());
    assert!(mid.items.len() > 4);
    let far = list(&r, &ed, &id, 0.005);
    assert!(far.items.len() <= 2, "{}", far.items.len());
}

#[test]
fn the_layout_is_cached_until_the_chart_or_its_table_changes() {
    let r = Renderer::new();
    let mut ed = editor(&r, "classic");
    run(&mut ed, json!({"op": "add_table", "id": "b1", "cells": [["", "Q2"], ["North", 1], ["South", 2]]}));
    let id = run(&mut ed, json!({"op": "add_chart", "from_table": "b1", "link": true, "x": 2000})).created[0].clone();
    let t0 = r.chart_layouts();
    list(&r, &ed, &id, 1.0);
    list(&r, &ed, &id, 1.0);
    assert_eq!(r.chart_layouts() - t0, 1, "the second frame reuses the layout");
    run(&mut ed, json!({"op": "set_cells", "id": "b1", "at": "B2", "values": [[5000]]}));
    let l = list(&r, &ed, &id, 1.0);
    assert_eq!(r.chart_layouts() - t0, 2);
    assert!(texts(&l).iter().any(|t| t.contains("5,000") || t.contains("6,000")), "{:?}", texts(&l));
    // Many charts: the cache stays bounded.
    for i in 0..300 {
        run(&mut ed, json!({"op": "add_chart", "x": i * 700, "y": 3000}));
    }
    for id in ed.doc.z_order.clone() {
        if id.starts_with('h') {
            list(&r, &ed, &id, 1.0);
        }
    }
    assert!(r.charts_cached() <= 256, "{}", r.charts_cached());
}

#[test]
fn hostile_data_draws_without_panicking() {
    let r = Renderer::new();
    let long = "🇬🇧".repeat(5000);
    let nasty = [
        json!([[""]]),
        json!([["", "a"], ["x", "text"]]),
        json!([["", "a", "b"], ["x", 1e308, -1e308], ["y", 5e-324, 0]]),
        json!([["", "a"], ["x", "=1/0"]]),
        json!([["", "a"], ["x", 0], ["y", 0]]),
        json!([["", "a"], ["x", -5], ["y", -1]]),
        json!([[long.clone(), long.clone()], [long.clone(), 1]]),
    ];
    for t in ChartType::ALL {
        for data in &nasty {
            let mut ed = editor(&r, "night");
            let id = run(&mut ed, json!({"op": "add_chart", "chart_type": t.key(), "data": data, "title": long})).created[0].clone();
            list(&r, &ed, &id, 1.0);
            for (w, h) in [(0.001, 0.001), (1e6, 1.0), (1.0, 1e6)] {
                run(&mut ed, json!({"op": "resize", "id": id, "width": w, "height": h}));
                list(&r, &ed, &id, 1.0);
            }
        }
    }
    // A thousand series, cut to 64.
    let mut ed = editor(&r, "classic");
    let row: Vec<serde_json::Value> = (0..1000).map(|i| json!(i)).collect();
    let id = run(&mut ed, json!({"op": "add_chart", "data": [row.clone(), row]})).created[0].clone();
    let t0 = std::time::Instant::now();
    list(&r, &ed, &id, 1.0);
    assert!(t0.elapsed().as_secs_f64() < 5.0);
}

#[test]
fn a_link_to_a_huge_table_lays_out_quickly() {
    let r = Renderer::new();
    let mut ed = editor(&r, "classic");
    let csv: String = (0..4999).map(|r| (0..20).map(|c| ((r * 7 + c * 13) % 1000).to_string()).collect::<Vec<_>>().join("\t")).collect::<Vec<_>>().join("\n");
    run(&mut ed, json!({"op": "add_table", "id": "b1", "csv": csv, "header_rows": 0}));
    let id = run(&mut ed, json!({"op": "add_chart", "chart_type": "line", "from_table": "b1", "link": true, "x": 50000})).created[0].clone();
    let t0 = std::time::Instant::now();
    let l = list(&r, &ed, &id, 1.0);
    let took = t0.elapsed();
    eprintln!("100k-cell link: first frame {took:?}, {} items", l.items.len());
    assert!(took.as_secs_f64() < if cfg!(debug_assertions) { 10.0 } else { 0.5 }, "{took:?}");
}
