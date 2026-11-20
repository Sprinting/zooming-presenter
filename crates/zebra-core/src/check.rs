//! Invariants of a document, checked in two places:
//!
//! * after every command ([`element`], [`steps`]): a command whose result
//!   breaks one is refused and rolled back, so a bad number (an overflow to
//!   infinity, NaN) or a bad text range never gets into the document, the
//!   undo history or a saved file;
//! * when a file is opened ([`repair`], [`repair_json`]): a file written by
//!   an older build, another program or a text editor is brought back
//!   inside them instead of crashing the app later.
use std::collections::HashSet;

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
        }
        ElementKind::Ink(d) => {
            if !(world(d.width) && d.width > 0.0) || d.points.iter().any(|p| !(world(p.x) && world(p.y))) {
                return bad("an ink point or width");
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
        }
        ElementKind::Image(d) => {
            if let Some(c) = &d.crop {
                if !(fin(c.x) && fin(c.y) && fin(c.w) && fin(c.h) && c.w > 0.0 && c.h > 0.0 && c.x >= 0.0 && c.y >= 0.0 && c.x + c.w <= 1.0 + 1e-9 && c.y + c.h <= 1.0 + 1e-9) {
                    return bad("crop");
                }
            }
        }
        ElementKind::Video(d) => {
            if !(fin(d.start) && d.start >= 0.0 && fin(d.duration) && d.end.is_none_or(|x| fin(x) && x > d.start)) {
                return bad("start, end or duration");
            }
        }
        ElementKind::Table(t) => {
            if let Err(what) = t.check() {
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
        for (part, key, default) in [("transform", "x", 0.0), ("transform", "y", 0.0), ("transform", "rotation", 0.0), ("transform", "scale", 1.0), ("size", "w", 100.0), ("size", "h", 100.0)] {
            if let Some(n) = e.get_mut(part).and_then(|p| p.get_mut(key)) {
                if n.is_null() {
                    *n = serde_json::json!(default);
                }
            }
        }
    }
}

/// Bring a just-parsed document inside the invariants. Elements that
/// can't be mended are dropped. Returns how many things were changed.
pub fn repair(doc: &mut Document) -> usize {
    let mut fixed = 0;
    let mut drop = vec![];
    for (id, e) in doc.elements.iter_mut() {
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
            }
            ElementKind::Ink(d) => {
                d.points.retain(|p| world(p.x) && world(p.y));
                if !(world(d.width) && d.width > 0.0) {
                    d.width = 4.0;
                }
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
            }
            ElementKind::Image(d) => {
                d.crop = d.crop.filter(|c| fin(c.x) && fin(c.y) && fin(c.w) && fin(c.h)).map(|c| c.clamped()).filter(|c| !c.is_full());
            }
            ElementKind::Video(d) => {
                if !(fin(d.start) && d.start >= 0.0) {
                    d.start = 0.0;
                }
                if !fin(d.duration) {
                    d.duration = 0.0;
                }
                d.end = d.end.filter(|x| fin(*x) && *x > d.start);
            }
            ElementKind::Table(t) => {
                // Sizes in a table that lost them come back at the usual
                // body text size; measuring afterwards fits the rest.
                t.repair(24.0);
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
    doc.play.autoplay = doc.play.autoplay.filter(|d| fin(*d) && *d > 0.0);
    if let Some(s) = &mut doc.soundtrack {
        if !fin(s.volume) {
            s.volume = 0.6;
        }
    }
    fixed
}
