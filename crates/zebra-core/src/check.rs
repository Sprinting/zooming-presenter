//! Invariants of a document, checked in two places:
//!
//! * after every command ([`element`], [`steps`]): a command whose result
//!   breaks one is refused and rolled back, so a bad number (an overflow to
//!   infinity, NaN) or a bad text range never gets into the document, the
//!   undo history or a saved file;
//! * when a file is opened ([`repair`], [`repair_json`]): a file written by
//!   an older build, another program or a text editor is brought back
//!   inside them instead of crashing the app later.
use std::collections::{HashMap, HashSet};

use crate::error::{ZError, ZResult};
use crate::model::*;

/// The canvas is this big in every direction, and nothing on it is bigger
/// or scaled further than this. Far beyond any real deck (the camera's
/// own zoom range is 1e-6 to 1e6), and far enough from the ends of f64
/// that sums, products and areas of in-range numbers stay finite and
/// keep their precision.
pub const WORLD_LIMIT: f64 = 1e15;

fn fin(v: f64) -> bool {
    v.is_finite()
}

/// A coordinate or length on the canvas.
fn world(v: f64) -> bool {
    v.abs() <= WORLD_LIMIT
}

/// Line widths and corner radii may always be this big (local units),
/// whatever the element's size: the inspector offers up to 200, and a
/// shape shrunk below its outline's width must stay editable.
pub const LINE_FLOOR: f64 = 200.0;

/// Most a shape's outline width or corner radius may be: its larger side
/// (but at least [`LINE_FLOOR`]). Absurd widths made renderers allocate
/// gigabytes or panic (hostile-input audit C3).
pub fn shape_line_limit(size: &Size) -> f64 {
    size.w.max(size.h).max(LINE_FLOOR)
}

/// Most a connector's or ink stroke's width may be: the diagonal of the
/// element and of its points (but at least [`LINE_FLOOR`]).
fn line_limit<'a>(size: &Size, pts: impl Iterator<Item = (f64, f64)>) -> f64 {
    let (mut x0, mut y0, mut x1, mut y1) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (x, y) in pts {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    size.w.hypot(size.h).max((x1 - x0).hypot(y1 - y0)).max(LINE_FLOOR)
}

fn connector_limit(size: &Size, d: &ConnectorData) -> f64 {
    line_limit(size, [(d.from.x, d.from.y), (d.to.x, d.to.y)].into_iter())
}

fn ink_limit(size: &Size, d: &InkData) -> f64 {
    line_limit(size, d.points.iter().map(|p| (p.x, p.y)))
}

/// A key the editor made (`content_key`): safe as a file name. Anything
/// with a path separator or a dot could leave the media cache folder.
pub fn safe_asset_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 128 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Every number of `e` is finite and in range, and its text ranges fit
/// its text.
pub fn element(e: &Element) -> ZResult<()> {
    let bad = |what: &str| Err(ZError::invalid(format!("'{}': {what} is not a usable number", e.id)).hint("coordinates, sizes and scales stay within ±1e15"));
    let t = &e.transform;
    if !(world(t.x) && world(t.y)) {
        return bad("position");
    }
    if !fin(t.rotation) {
        return bad("rotation");
    }
    if !(t.scale >= 1.0 / WORLD_LIMIT && t.scale <= WORLD_LIMIT) {
        return bad("scale");
    }
    if !(world(e.size.w) && world(e.size.h) && e.size.w >= 0.0 && e.size.h >= 0.0) {
        return bad("size");
    }
    if !(world(e.size.w * t.scale) && world(e.size.h * t.scale)) {
        return bad("size on the canvas");
    }
    match &e.kind {
        ElementKind::Text(d) => {
            let n = d.text.chars().count();
            if d.spans.iter().any(|s| s.start >= s.end || s.end > n || s.format.size.is_some_and(|z| !(fin(z) && z > 0.0))) {
                return Err(ZError::invalid(format!("'{}': a text span is outside the text", e.id)));
            }
            if d.paras.iter().any(|p| p.indent > 8) {
                return Err(ZError::invalid(format!("'{}': paragraph indent is 0 to 8", e.id)));
            }
        }
        ElementKind::Shape(d) => {
            if !(fin(d.stroke_width) && d.stroke_width >= 0.0 && fin(d.corner_radius) && d.corner_radius >= 0.0) {
                return bad("stroke width or corner radius");
            }
            let limit = shape_line_limit(&e.size);
            if d.stroke_width > limit || d.corner_radius > limit {
                return Err(ZError::invalid(format!("'{}': stroke width and corner radius are at most {limit}", e.id))
                    .hint("at most the shape's larger side, or 200"));
            }
        }
        ElementKind::Ink(d) => {
            if !(world(d.width) && d.width > 0.0) || d.points.iter().any(|p| !(world(p.x) && world(p.y))) {
                return bad("an ink point or width");
            }
            let limit = ink_limit(&e.size, d);
            if d.width > limit {
                return Err(ZError::invalid(format!("'{}': ink width is at most {limit}", e.id)).hint("at most the drawing's diagonal, or 200"));
            }
        }
        ElementKind::Icon(d) => {
            if !(fin(d.stroke_width) && d.stroke_width > 0.0) {
                return bad("stroke width");
            }
        }
        ElementKind::Connector(d) => {
            if !(world(d.width) && d.width > 0.0) || [&d.from, &d.to].iter().any(|p| !(world(p.x) && world(p.y) && fin(p.dir.x) && fin(p.dir.y))) {
                return bad("a connector end or width");
            }
            let limit = connector_limit(&e.size, d);
            if d.width > limit {
                return Err(ZError::invalid(format!("'{}': connector width is at most {limit}", e.id)).hint("at most the connector's length, or 200"));
            }
        }
        ElementKind::Image(d) => {
            if let Some(c) = &d.crop {
                if !(fin(c.x) && fin(c.y) && fin(c.w) && fin(c.h) && c.w > 0.0 && c.h > 0.0 && c.x >= 0.0 && c.y >= 0.0 && c.x + c.w <= 1.0 + 1e-9 && c.y + c.h <= 1.0 + 1e-9) {
                    return bad("crop");
                }
            }
        }
        ElementKind::Video(d) => {
            let length = crate::media::plausible_duration(d.duration) == d.duration;
            if !(fin(d.start) && d.start >= 0.0 && length && d.end.is_none_or(|x| fin(x) && x > d.start)) {
                return bad("start, end or duration");
            }
        }
        ElementKind::Table(t) => {
            if let Err(what) = t.check() {
                return Err(ZError::invalid(format!("'{}': {what}", e.id)));
            }
        }
        ElementKind::Chart(c) => {
            if let Err(what) = c.check() {
                return Err(ZError::invalid(format!("'{}': {what}", e.id)));
            }
        }
        ElementKind::Frame(_) => {}
    }
    Ok(())
}

/// The path and the play settings hold only usable numbers.
pub fn steps(doc: &Document) -> ZResult<()> {
    for (i, s) in doc.path.iter().enumerate() {
        if let StepTarget::View { view: v } = &s.target {
            if !(world(v.cx) && world(v.cy) && fin(v.rotation) && world(v.width) && world(v.height) && v.width > 0.0 && v.height > 0.0) {
                return Err(ZError::invalid(format!("step {i}: the view needs finite numbers and a width and height > 0")));
            }
        }
        if s.duration.is_some_and(|d| !(fin(d) && d >= 0.0)) || s.dwell.is_some_and(|d| !(fin(d) && d > 0.0)) {
            return Err(ZError::invalid(format!("step {i}: duration and dwell are seconds")));
        }
    }
    if doc.play.autoplay.is_some_and(|d| !(fin(d) && d > 0.0)) {
        return Err(ZError::invalid("autoplay is a number of seconds"));
    }
    if doc.soundtrack.as_ref().is_some_and(|s| !fin(s.volume)) {
        return Err(ZError::invalid("volume is a number from 0 to 1"));
    }
    Ok(())
}

/// Before parsing: a number the document needs that was saved as `null`
/// (what JSON makes of infinity and NaN) becomes a plain default, so the
/// file opens. [`repair`] then tidies the element.
pub fn repair_json(v: &mut serde_json::Value) {
    let Some(els) = v.get_mut("elements").and_then(|e| e.as_object_mut()) else { return };
    for e in els.values_mut() {
        if let Some(k) = e.get_mut("kind").filter(|k| k.get("type").and_then(|t| t.as_str()) == Some("table")) {
            repair_table_json(k);
        }
        if let Some(k) = e.get_mut("kind").filter(|k| k.get("type").and_then(|t| t.as_str()) == Some("chart")) {
            repair_chart_json(k);
        }
        for (part, key, default) in [("transform", "x", 0.0), ("transform", "y", 0.0), ("transform", "rotation", 0.0), ("transform", "scale", 1.0), ("size", "w", 100.0), ("size", "h", 100.0)] {
            if let Some(n) = e.get_mut(part).and_then(|p| p.get_mut(key)) {
                if n.is_null() {
                    *n = serde_json::json!(default);
                }
            }
        }
    }
}

/// A table as read from a file, made readable: anything that isn't what
/// a table holds is replaced by a default or dropped. Nothing is cut,
/// whatever its size (its memory was checked as the file was read).
/// [`repair`] then makes it rectangular.
fn repair_table_json(k: &mut serde_json::Value) {
    use crate::table::{Cell, CellRange, Column, Gridlines, RangeFormat, Row, TableStyle};
    use serde::Deserialize;
    use serde_json::{json, Value};
    let Some(o) = k.as_object_mut() else { return };
    let ok = |v: &Value, f: fn(&Value) -> bool| f(v);
    // Cells: rows of cells.
    let mut cells = match o.remove("cells") {
        Some(Value::Array(rows)) => rows,
        _ => vec![],
    };
    // A cell with a value or format that doesn't read keeps its input.
    let fix_cell = |c: &mut Value| {
        if Cell::deserialize(&*c).is_err() {
            let input = c.get("input").and_then(|x| x.as_str()).map(str::to_string);
            *c = json!(input.unwrap_or_default());
        }
    };
    for row in cells.iter_mut() {
        // Version 4's filled cells only: {"n": columns, "c": [[column,
        // cell], ...], "k": rows}.
        if let Some(o) = row.as_object_mut() {
            if !o.get("n").is_some_and(|n| n.is_u64()) {
                o.insert("n".into(), json!(0));
            }
            if o.get("k").is_some_and(|k| !k.is_u64()) {
                o.remove("k");
            }
            let mut pairs = match o.remove("c") {
                Some(Value::Array(v)) => v,
                _ => vec![],
            };
            // A pair that isn't [column, cell] has nowhere to go.
            pairs.retain(|p| p.as_array().is_some_and(|a| a.len() == 2 && a[0].is_u64()));
            for p in pairs.iter_mut() {
                if let Some(c) = p.get_mut(1) {
                    fix_cell(c);
                }
            }
            o.insert("c".into(), Value::Array(pairs));
            o.retain(|k, _| matches!(k.as_str(), "n" | "c" | "k"));
            continue;
        }
        if !row.is_array() {
            *row = json!([]);
        }
        if let Some(r) = row.as_array_mut() {
            for c in r.iter_mut() {
                fix_cell(c);
            }
        }
    }
    if cells.is_empty() {
        cells.push(json!([]));
    }
    o.insert("cells".into(), Value::Array(cells));
    // Columns and rows: each readable, or a default ([`repair`] sizes them).
    let list = |o: &mut serde_json::Map<String, Value>, key: &str, good: fn(&Value) -> bool, default: Value| {
        let mut v = match o.remove(key) {
            Some(Value::Array(v)) => v,
            _ => vec![],
        };
        for x in v.iter_mut() {
            if !good(x) {
                *x = default.clone();
            }
        }
        o.insert(key.into(), Value::Array(v));
    };
    list(o, "columns", |x| Column::deserialize(x).is_ok(), json!({"width": 100}));
    // Rows: a row, or a run of alike rows `[count, row]` (version 4).
    list(o, "rows", |x| Row::deserialize(x).is_ok() || <(u64, Row)>::deserialize(x).is_ok(), json!({"height": 48}));
    // Lists whose bad entries go.
    for (key, good) in [("merges", (|x: &Value| CellRange::deserialize(x).is_ok()) as fn(&Value) -> bool), ("cell_formats", |x: &Value| RangeFormat::deserialize(x).is_ok())] {
        if let Some(v) = o.get_mut(key) {
            match v.as_array_mut() {
                Some(a) => a.retain(|x| good(x)),
                None => *v = json!([]),
            }
        }
    }
    // Settings that don't read go back to their defaults.
    let checks: [(&str, fn(&Value) -> bool); 8] = [
        ("header_rows", |x| x.as_u64().is_some_and(|n| n <= 255)),
        ("header_columns", |x| x.as_u64().is_some_and(|n| n <= 255)),
        ("footer_rows", |x| x.as_u64().is_some_and(|n| n <= 255)),
        ("style", |x| TableStyle::deserialize(x).is_ok()),
        ("font_size", |x| x.as_f64().is_some()),
        ("banded", |x| x.is_null() || x.is_boolean()),
        ("gridlines", |x| x.is_null() || Gridlines::deserialize(x).is_ok()),
        ("show_name", |x| x.is_boolean()),
    ];
    for (key, good) in checks {
        if o.get(key).is_some_and(|x| !ok(x, good)) {
            o.remove(key);
        }
    }
}

/// A chart as read from a file, made readable: a type or look this build
/// doesn't know becomes the default, its data is read as a table's is,
/// and any setting that doesn't read goes back to its default. [`repair`]
/// then brings the values in range and cuts the data to the limits.
fn repair_chart_json(k: &mut serde_json::Value) {
    use crate::chart::{ChartLink, ChartLook, ChartType, SeriesFormat, SeriesIn};
    use crate::table::DataFormat;
    use serde::Deserialize;
    use serde_json::{json, Value};
    let Some(o) = k.as_object_mut() else { return };
    match o.get_mut("data") {
        Some(d) if d.is_object() => repair_table_json(d),
        _ => {
            o.insert("data".into(), json!({"cells": [[""]], "columns": [], "rows": []}));
        }
    }
    let checks: [(&str, fn(&Value) -> bool); 20] = [
        ("chart_type", |x| ChartType::deserialize(x).is_ok()),
        ("look", |x| ChartLook::deserialize(x).is_ok()),
        ("series_in", |x| SeriesIn::deserialize(x).is_ok()),
        ("link", |x| x.is_null() || ChartLink::deserialize(x).is_ok()),
        ("number", |x| DataFormat::deserialize(x).is_ok()),
        ("title", |x| x.is_string()),
        ("x_title", |x| x.is_string()),
        ("y_title", |x| x.is_string()),
        ("legend", |x| x.is_boolean()),
        ("border", |x| x.is_boolean()),
        ("value_labels", |x| x.is_boolean()),
        ("gridlines", |x| x.is_boolean()),
        ("font_size", |x| x.as_f64().is_some()),
        ("gap", |x| x.as_f64().is_some()),
        ("gap_sets", |x| x.as_f64().is_some()),
        ("hole", |x| x.as_f64().is_some()),
        ("axis_min", |x| x.is_null() || x.as_f64().is_some()),
        ("axis_max", |x| x.is_null() || x.as_f64().is_some()),
        ("axis_step", |x| x.is_null() || x.as_f64().is_some()),
        ("totals", |x| x.is_array()),
    ];
    for (key, good) in checks {
        if o.get(key).is_some_and(|x| !good(x)) {
            o.remove(key);
        }
    }
    if let Some(Value::Array(t)) = o.get_mut("totals") {
        t.retain(|x| x.as_u64().is_some_and(|n| n < crate::chart::MAX_CATEGORIES as u64));
        t.truncate(crate::chart::MAX_CATEGORIES);
    }
    match o.get_mut("series") {
        Some(Value::Array(list)) => {
            list.truncate(crate::chart::MAX_SERIES);
            for x in list.iter_mut() {
                if SeriesFormat::deserialize(&*x).is_err() {
                    *x = json!({});
                }
            }
        }
        Some(_) => {
            o.remove("series");
        }
        None => {}
    }
}

/// Bring a just-parsed document inside the invariants. Elements that
/// can't be mended are dropped. Returns how many things were changed.
pub fn repair(doc: &mut Document) -> usize {
    let mut fixed = 0;
    let mut drop = vec![];
    for (id, e) in doc.elements.iter_mut() {
        // The map key is the element's id: code that collects `e.id` looks
        // it up by key again.
        if e.id != *id {
            e.id = id.clone();
            fixed += 1;
        }
        if element(e).is_ok() {
            continue;
        }
        fixed += 1;
        let t = &mut e.transform;
        if !world(t.x) {
            t.x = 0.0;
        }
        if !world(t.y) {
            t.y = 0.0;
        }
        if !fin(t.rotation) {
            t.rotation = 0.0;
        }
        if !(t.scale >= 1.0 / WORLD_LIMIT && t.scale <= WORLD_LIMIT) {
            t.scale = 1.0;
        }
        if !(world(e.size.w) && e.size.w >= 0.0) {
            e.size.w = 100.0;
        }
        if !(world(e.size.h) && e.size.h >= 0.0) {
            e.size.h = 100.0;
        }
        if !(world(e.size.w * t.scale) && world(e.size.h * t.scale)) {
            t.scale = 1.0;
        }
        match &mut e.kind {
            ElementKind::Text(d) => {
                for s in &mut d.spans {
                    if s.format.size.is_some_and(|z| !(fin(z) && z > 0.0)) {
                        s.format.size = None;
                    }
                }
                crate::rich::normalize(d);
            }
            ElementKind::Shape(d) => {
                if !(fin(d.stroke_width) && d.stroke_width >= 0.0) {
                    d.stroke_width = 4.0;
                }
                if !(fin(d.corner_radius) && d.corner_radius >= 0.0) {
                    d.corner_radius = 0.0;
                }
                let limit = shape_line_limit(&e.size);
                d.stroke_width = d.stroke_width.min(limit);
                // Drawn no rounder than half the shorter side anyway.
                if d.corner_radius > limit {
                    d.corner_radius = (e.size.w.min(e.size.h) / 2.0).min(limit);
                }
            }
            ElementKind::Ink(d) => {
                d.points.retain(|p| world(p.x) && world(p.y));
                if !(world(d.width) && d.width > 0.0) {
                    d.width = 4.0;
                }
                d.width = d.width.min(ink_limit(&e.size, d));
            }
            ElementKind::Icon(d) => d.stroke_width = 2.0,
            ElementKind::Connector(d) => {
                if !(world(d.width) && d.width > 0.0) {
                    d.width = 3.0;
                }
                for p in [&mut d.from, &mut d.to] {
                    if !(world(p.x) && world(p.y)) {
                        (p.x, p.y) = (0.0, 0.0);
                    }
                    if !(fin(p.dir.x) && fin(p.dir.y)) {
                        p.dir = crate::geom::Vec2::ZERO;
                    }
                }
                d.width = d.width.min(connector_limit(&e.size, d));
            }
            ElementKind::Image(d) => {
                d.crop = d.crop.filter(|c| fin(c.x) && fin(c.y) && fin(c.w) && fin(c.h)).map(|c| c.clamped()).filter(|c| !c.is_full());
            }
            ElementKind::Video(d) => {
                if !(fin(d.start) && d.start >= 0.0) {
                    d.start = 0.0;
                }
                // A length no video has (from a lying header) is unknown.
                d.duration = crate::media::plausible_duration(d.duration);
                d.end = d.end.filter(|x| fin(*x) && *x > d.start);
            }
            ElementKind::Table(t) => {
                // Sizes in a table that lost them come back at the usual
                // body text size; measuring afterwards fits the rest.
                t.repair(24.0);
            }
            ElementKind::Chart(c) => {
                c.repair(24.0);
            }
            ElementKind::Frame(_) => {}
        }
        if element(e).is_err() {
            drop.push(id.clone());
        }
    }
    for id in &drop {
        doc.elements.remove(id);
    }
    // A chart linked to a table that isn't there (or to anything else)
    // draws its own data; a range outside its table is clipped to it.
    let tables: HashMap<ElementId, (usize, usize)> = doc
        .elements
        .iter()
        .filter_map(|(k, e)| match &e.kind {
            ElementKind::Table(t) => Some((k.clone(), (t.n_rows(), t.n_cols()))),
            _ => None,
        })
        .collect();
    for e in doc.elements.values_mut() {
        if let ElementKind::Chart(c) = &mut e.kind {
            let Some(l) = &mut c.link else { continue };
            match tables.get(&l.table) {
                None => {
                    c.link = None;
                    fixed += 1;
                }
                Some(&(rows, cols)) => {
                    if let Some(r) = l.range {
                        let clipped = r.clip(rows, cols);
                        if clipped != Some(r) {
                            l.range = clipped;
                            fixed += 1;
                        }
                    }
                }
            }
        }
    }
    // Draw order: every element once.
    let before = doc.z_order.len();
    let mut seen = HashSet::new();
    let els = &doc.elements;
    doc.z_order.retain(|id| els.contains_key(id) && seen.insert(id.clone()));
    fixed += before - doc.z_order.len();
    let missing: Vec<ElementId> = doc.elements.keys().filter(|k| !seen.contains(*k)).cloned().collect();
    doc.z_order.extend(missing);
    // Path steps with unusable numbers go; timings fall back to the defaults.
    let before = doc.path.len();
    doc.path.retain(|s| match &s.target {
        StepTarget::View { view: v } => world(v.cx) && world(v.cy) && fin(v.rotation) && world(v.width) && world(v.height) && v.width > 0.0 && v.height > 0.0,
        _ => true,
    });
    fixed += before - doc.path.len();
    for s in &mut doc.path {
        s.duration = s.duration.filter(|d| fin(*d) && *d >= 0.0);
        s.dwell = s.dwell.filter(|d| fin(*d) && *d > 0.0);
        s.builds.retain(|b| els.contains_key(b));
    }
    // An absurd id counter (u64::MAX overflowed, hostile-input audit L4)
    // counts on from the largest number in use instead. A lower one is
    // kept: new ids skip the ones taken, and a sound file reopens as it
    // was saved.
    if doc.next_id > u64::MAX / 2 {
        doc.next_id = doc.elements.keys().filter_map(|k| k.trim_start_matches(|c: char| !c.is_ascii_digit()).parse::<u64>().ok()).max().map_or(1, |m| m.saturating_add(1));
        fixed += 1;
    }
    // A custom theme without an id can't be chosen; one with a built-in's id
    // would hide that built-in.
    let before = doc.themes.len();
    doc.themes.retain(|t| !t.id.trim().is_empty() && !crate::theme::Theme::is_builtin(&t.id));
    fixed += before - doc.themes.len();
    // Custom theme weights a file can hold (u16) but save_theme refuses:
    // the nearest weight in range. The renderer draws the nearest face.
    for t in &mut doc.themes {
        if t.name.trim().is_empty() {
            t.name = t.id.clone();
            fixed += 1;
        }
        for s in [&mut t.title, &mut t.subtitle, &mut t.body] {
            let d = crate::theme::TextStyleDef::default();
            if !(fin(s.size) && s.size > 0.0) {
                s.size = d.size;
                fixed += 1;
            }
            if s.font.trim().is_empty() {
                s.font = d.font;
                fixed += 1;
            }
            let w = s.weight.clamp(1, 1000);
            if w != s.weight {
                s.weight = w;
                fixed += 1;
            }
        }
    }
    doc.play.autoplay = doc.play.autoplay.filter(|d| fin(*d) && *d > 0.0);
    if let Some(s) = &mut doc.soundtrack {
        if !fin(s.volume) {
            s.volume = 0.6;
        }
    }
    fixed
}
