//! Agent perception: the text outline (`describe`), structured queries, lint.
use std::fmt::Write;

use serde_json::{json, Value};

use crate::command::Query;
use crate::editor::Editor;
use crate::error::{ZError, ZResult};
use crate::geom::Vec2;
use crate::model::*;
use crate::theme::Theme;

fn r(v: f64) -> f64 {
    if v.abs() >= 100.0 { v.round() } else { (v * 100.0).round() / 100.0 }
}

pub fn element_json(ed: &Editor, e: &Element) -> Value {
    element_json_in(ed, e, ed.parent_frame(&e.id).as_deref(), &ed.doc.steps_using(&e.id))
}

/// [`element_json`] with the parent frame and path steps worked out by
/// the caller (once for a whole list, not per element).
fn element_json_in(ed: &Editor, e: &Element, parent: Option<&str>, steps: &[usize]) -> Value {
    let b = e.bounds();
    let mut v = serde_json::to_value(e).unwrap();
    let o = v.as_object_mut().unwrap();
    o.insert("label".into(), json!(ed.label(e)));
    o.insert("bounds".into(), json!([r(b.x0), r(b.y0), r(b.x1), r(b.y1)]));
    o.insert("parent_frame".into(), json!(parent));
    o.insert("path_steps".into(), json!(steps));
    if e.is_frame() {
        o.insert("contents".into(), json!(ed.frame_contents(&e.id)));
    }
    v
}

/// Every element's path steps ([`Document::steps_using`]) in one pass.
fn steps_by_element(d: &Document) -> std::collections::HashMap<&str, Vec<usize>> {
    let mut m: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (i, s) in d.path.iter().enumerate() {
        let target = match &s.target {
            StepTarget::Element { id } => Some(id.as_str()),
            _ => None,
        };
        for id in target.into_iter().chain(s.builds.iter().map(|b| b.as_str())) {
            let v = m.entry(id).or_default();
            if v.last() != Some(&i) {
                v.push(i);
            }
        }
    }
    m
}

fn one_line(ed: &Editor, e: &Element, steps: &[usize]) -> String {
    let t = &e.transform;
    let (w, h) = e.world_size();
    let mut s = format!("{} {}", e.id, e.kind.type_name());
    match &e.kind {
        ElementKind::Frame(f) => {
            let _ = write!(s, " {}", serde_json::to_value(f.style).unwrap().as_str().unwrap());
        }
        ElementKind::Text(d) => {
            let _ = write!(s, " {}", serde_json::to_value(d.style).unwrap().as_str().unwrap());
            if d.spans.is_empty() && d.paras.is_empty() {
                let txt = truncate(&d.text.replace('\n', " / "), 60);
                let _ = write!(s, " {:?}", txt);
            } else {
                // Rich text reads back as Markdown (the same subset add_text accepts).
                let txt = truncate(&crate::rich::to_markdown(d).replace('\n', " / "), 90);
                let _ = write!(s, " md{:?}", txt);
            }
        }
        ElementKind::Shape(d) => {
            let _ = write!(s, " {}", serde_json::to_value(d.shape).unwrap().as_str().unwrap());
        }
        ElementKind::Icon(i) => {
            let _ = write!(s, " {}", i.name);
        }
        ElementKind::Connector(c) => {
            let end = |e: &crate::model::Endpoint| e.element.clone().unwrap_or_else(|| "point".into());
            let _ = write!(s, " {} -> {}", end(&c.from), end(&c.to));
            if !c.label.is_empty() {
                let _ = write!(s, " label={:?}", truncate(&c.label, 30));
            }
        }
        ElementKind::Image(i) => {
            if !i.alt.is_empty() {
                let _ = write!(s, " alt={:?}", truncate(&i.alt, 40));
            }
            if i.crop.is_some() {
                s.push_str(" cropped");
            }
            if i.mask != ImageMask::Rectangle {
                let _ = write!(s, " mask {}", serde_json::to_value(i.mask).unwrap().as_str().unwrap());
            }
        }
        ElementKind::Video(v) => {
            if !v.alt.is_empty() {
                let _ = write!(s, " alt={:?}", truncate(&v.alt, 40));
            }
            if v.duration > 0.0 {
                let _ = write!(s, " {}s", r(v.duration));
            }
            if v.start > 0.0 || v.end.is_some() {
                let end = v.end.map(|e| r(e).to_string()).unwrap_or_else(|| "end".into());
                let _ = write!(s, " trim {}–{end}", r(v.start));
            }
            for (on, w) in [(v.autoplay, "autoplay"), (v.looping, "loop"), (v.muted, "muted")] {
                if on {
                    let _ = write!(s, " {w}");
                }
            }
        }
        _ => {}
    }
    if !e.name.is_empty() {
        let _ = write!(s, " name={:?}", e.name);
    }
    let _ = write!(s, " @({},{}) {}x{}", r(t.x), r(t.y), r(w), r(h));
    if (t.scale - 1.0).abs() > 1e-9 {
        let _ = write!(s, " scale {}", r(t.scale));
    }
    if t.rotation.abs() > 1e-9 {
        let _ = write!(s, " rot {}°", r(t.rotation));
    }
    if e.locked {
        s.push_str(" locked");
    }
    if let Some(g) = &e.group {
        let _ = write!(s, " group={g}");
    }
    if !steps.is_empty() {
        let st: Vec<String> = steps.iter().map(|i| (i + 1).to_string()).collect();
        let _ = write!(s, " [step {}]", st.join(","));
    }
    if ed.view.selection.contains(&e.id) {
        s.push_str(" (human-selected)");
    }
    s
}

/// Compact outline, like a Playwright aria snapshot. Ids are stable.
pub fn describe(ed: &Editor) -> String {
    let d = &ed.doc;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\"{}\" · theme {} · rev {} · {} elements · {} path steps · mode {}",
        d.title,
        d.theme,
        ed.revision,
        d.elements.len(),
        d.path.len(),
        serde_json::to_value(ed.view.mode).unwrap().as_str().unwrap()
    );
    if let Some(t) = &d.soundtrack {
        let name = if t.name.is_empty() { "audio" } else { t.name.as_str() };
        let _ = writeln!(out, "Soundtrack: {name:?} volume {}{}", r(t.volume), if t.looping { " loop" } else { "" });
    }
    if !d.path.is_empty() {
        out.push_str("Path:\n");
        for (i, s) in d.path.iter().enumerate() {
            let tgt = match &s.target {
                StepTarget::Element { id } => format!("{id} \"{}\"", d.get(id).map(|e| ed.label(e)).unwrap_or_default()),
                StepTarget::View { view } => format!("view @({},{}) {}x{}", r(view.cx), r(view.cy), r(view.width), r(view.height)),
                StepTarget::Overview => "overview".into(),
            };
            let _ = write!(out, "  {}. {tgt}", i + 1);
            if !s.builds.is_empty() {
                let _ = write!(out, " builds: {}", s.builds.join(", "));
            }
            if !s.notes.is_empty() {
                let _ = write!(out, " notes: {:?}", truncate(&s.notes, 50));
            }
            if s.voiceover.is_some() {
                out.push_str(" voice-over");
            }
            out.push('\n');
        }
    }
    if d.elements.is_empty() {
        out.push_str("Canvas is empty.\n");
    } else {
        out.push_str("Canvas (back to front, children indented under their frame):\n");
        // Build a tree by smallest enclosing frame: every parent worked
        // out once, children listed back to front.
        let parents = ed.parent_frames();
        let mut children: std::collections::HashMap<Option<&str>, Vec<&str>> = std::collections::HashMap::new();
        for id in &d.z_order {
            children.entry(parents.get(id.as_str()).copied()).or_default().push(id);
        }
        let steps = steps_by_element(d);
        type Tree<'a> = std::collections::HashMap<Option<&'a str>, Vec<&'a str>>;
        fn walk(ed: &Editor, children: &Tree, steps: &std::collections::HashMap<&str, Vec<usize>>, parent: Option<&str>, depth: usize, out: &mut String) {
            for &id in children.get(&parent).map(|v| v.as_slice()).unwrap_or_default() {
                let e = &ed.doc.elements[id];
                let _ = writeln!(out, "{}{}", "  ".repeat(depth + 1), one_line(ed, e, steps.get(id).map(|v| v.as_slice()).unwrap_or_default()));
                if e.is_frame() {
                    walk(ed, children, steps, Some(id), depth + 1, out);
                }
            }
        }
        walk(ed, &children, &steps, None, 0, &mut out);
    }
    let c = ed.view.camera;
    let _ = writeln!(
        out,
        "Human view: centre ({},{}) zoom {} rot {}° viewport {}x{}",
        r(c.cx),
        r(c.cy),
        r(c.zoom),
        r(c.rotation),
        ed.view.viewport.x,
        ed.view.viewport.y
    );
    if !ed.view.selection.is_empty() {
        let _ = writeln!(out, "Human selection: {}", ed.view.selection.join(", "));
    }
    if ed.view.mode == crate::view::Mode::Present {
        let p = &ed.view.present;
        let _ = writeln!(out, "Presenting: step {:?} build {}", p.step.map(|s| s + 1), p.build);
    }
    out
}

pub fn query(ed: &Editor, q: Query) -> ZResult<Value> {
    let d = &ed.doc;
    Ok(match q {
        Query::Describe => json!({ "outline": describe(ed) }),
        Query::State => {
            let v = &ed.view;
            json!({
                "title": d.title,
                "theme": d.theme,
                "revision": ed.revision,
                "dirty": ed.dirty(),
                "file": ed.file_path.as_ref().map(|p| p.display().to_string()),
                "mode": v.mode,
                "tool": v.tool,
                "selection": v.selection,
                "agent_selection": v.agent_selection,
                "camera": v.camera,
                "agent_camera": v.agent_camera,
                "viewport": [v.viewport.x, v.viewport.y],
                "animating": v.flight.is_some(),
                "present": v.present,
                "agent_paused": v.agent_paused,
                "agent_permission": v.agent_permission,
                "follow_agent": v.follow_agent,
                "human_editing": ed.human_gesture_active(),
                "can_undo": ed.can_undo(),
                "can_redo": ed.can_redo(),
                "element_count": d.elements.len(),
                "path_length": d.path.len(),
            })
        }
        Query::Document => serde_json::to_value(d).unwrap(),
        Query::Element { id } => {
            let e = d.get(&id).ok_or_else(|| ZError::not_found(&id, d.elements.keys().cloned()))?;
            element_json(ed, e)
        }
        Query::Elements { kind, text, within, visible } => {
            let inside = match &within {
                Some(f) => {
                    if d.get(f).is_none() {
                        return Err(ZError::not_found(f, d.elements.keys().cloned()));
                    }
                    Some(ed.frame_contents(f).into_iter().collect::<std::collections::HashSet<_>>())
                }
                None => None,
            };
            let parents = ed.parent_frames();
            let steps = steps_by_element(d);
            let vb = ed.view.camera.visible_bounds(ed.view.viewport);
            let needle = text.map(|t| t.to_lowercase());
            let list: Vec<Value> = d
                .ordered()
                .filter(|e| kind.as_deref().is_none_or(|k| e.kind.type_name() == k))
                .filter(|e| inside.as_ref().is_none_or(|v| v.contains(&e.id)))
                .filter(|e| !visible || e.bounds().intersects(&vb))
                .filter(|e| {
                    needle.as_ref().is_none_or(|n| {
                        let hay = match &e.kind {
                            ElementKind::Text(t) => t.text.to_lowercase(),
                            ElementKind::Image(i) => i.alt.to_lowercase(),
                            ElementKind::Video(v) => v.alt.to_lowercase(),
                            _ => String::new(),
                        };
                        hay.contains(n.as_str()) || e.name.to_lowercase().contains(n.as_str())
                    })
                })
                .map(|e| element_json_in(ed, e, parents.get(e.id.as_str()).copied(), steps.get(e.id.as_str()).map(|v| v.as_slice()).unwrap_or_default()))
                .collect();
            json!({ "elements": list })
        }
        Query::HitTest { x, y } => {
            let tol = 4.0 / ed.view.camera.zoom;
            json!({ "id": ed.hit_test(Vec2::new(x, y), tol).map(|e| e.id.clone()) })
        }
        Query::HitTestScreen { x, y } => {
            let w = ed.view.camera.screen_to_world(Vec2::new(x, y), ed.view.viewport);
            let tol = 4.0 / ed.view.camera.zoom;
            json!({ "id": ed.hit_test(w, tol).map(|e| e.id.clone()), "world": [w.x, w.y] })
        }
        Query::Path => {
            let steps: Vec<Value> = d
                .path
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let label = match &s.target {
                        StepTarget::Element { id } => d.get(id).map(|e| ed.label(e)).unwrap_or_default(),
                        StepTarget::View { .. } => "view".into(),
                        StepTarget::Overview => "overview".into(),
                    };
                    json!({ "index": i, "label": label, "target": s.target, "builds": s.builds, "notes": s.notes,
                            "duration": s.duration, "dwell": s.dwell, "voiceover": s.voiceover, "camera": ed.camera_for_step(i) })
                })
                .collect();
            json!({ "steps": steps, "play": d.play })
        }
        Query::Themes => json!({ "current": d.theme, "themes": Theme::builtins(), "custom": d.themes }),
        Query::Templates => json!({ "templates": &*crate::templates::TEMPLATES, "layouts": &*crate::templates::LAYOUTS }),
        Query::Fonts => json!({ "families": ed.measure.font_families(), "bundled": ["Sans", "Serif", "Mono", "Hand"] }),
        Query::Perf => {
            let mut v = ed.perf.report(ed.view.now);
            let (mut mapped, mut in_ram) = (0usize, 0usize);
            for a in d.assets.values() {
                if a.bytes.is_mapped() { mapped += a.bytes.len() } else { in_ram += a.bytes.len() }
            }
            v["document"] = json!({
                "elements": d.elements.len(),
                "steps": d.path.len(),
                "history_kb": ed.history_bytes() / 1024,
                "assets_in_ram_kb": in_ram / 1024,
                "assets_mapped_kb": mapped / 1024,
            });
            v
        }
        Query::Icons { query, limit } => {
            let hits = crate::icons::search(&query, limit);
            let total = if query.trim().is_empty() { crate::icons::all().len() } else { hits.len() };
            json!({
                "total": total,
                "icons": hits.iter().map(|i| json!({ "name": i.name, "tags": i.tags })).collect::<Vec<_>>(),
                "licence": "Lucide icons, ISC licence",
            })
        }
        Query::Symbols { query, set, limit } => match crate::symbols::library() {
            None => match crate::symbols::library_error() {
                Some(e) => json!({ "installed": false, "error": e, "sets": [] }),
                None => json!({ "installed": false, "sets": [] }),
            },
            Some(lib) => {
                if query.trim().is_empty() && set.is_none() {
                    json!({ "installed": true, "total": lib.total(), "sets": lib.sets().map(|s| json!({
                        "set": s.key, "name": s.name, "group": s.group, "count": s.count, "licence": s.licence,
                        "colour": match s.tint { crate::symbols::Tint::Mono => "one", crate::symbols::Tint::Two => "two", crate::symbols::Tint::None => "own" },
                    })).collect::<Vec<_>>() })
                } else {
                    let only: Option<Vec<u16>> = set.as_deref().map(|k| lib.set_index(k).into_iter().collect());
                    let hits = lib.search(&query, only.as_deref(), limit);
                    json!({ "installed": true, "symbols": hits.iter().map(|r| json!({
                        "set": lib.set(r.set).map(|s| s.key.as_str()), "name": lib.name(*r), "keywords": lib.keywords(*r),
                    })).collect::<Vec<_>>() })
                }
            }
        },
        Query::Diagrams => json!(crate::diagrams::ALL.iter().map(|d| json!({ "diagram": d.key, "name": d.name, "description": d.description, "adds_to_path": d.path })).collect::<Vec<_>>()),
        Query::Shapes => json!(crate::model::ShapeKind::ALL
            .iter()
            .map(|k| json!({ "shape": k, "name": k.label(), "line": k.is_line() }))
            .collect::<Vec<_>>()),
        Query::History => {
            let h: Vec<Value> = ed
                .history()
                .iter()
                .rev()
                .take(100)
                .map(|h| json!({ "label": h.label, "by": h.origin.display(), "origin": h.origin, "revision": h.revision, "ids": h.ids }))
                .collect();
            json!({ "undo": h, "can_redo": ed.can_redo() })
        }
        Query::Lint => json!({ "issues": lint(ed) }),
    })
}

/// Where a text box's words are, not its box: a wrapped box is often
/// wider than its longest line, and the rest is empty and invisible.
fn ink_bounds(ed: &Editor, theme: &crate::theme::Theme, e: &Element) -> crate::geom::Rect {
    let ElementKind::Text(t) = &e.kind else { return e.bounds() };
    if !t.fixed_width {
        return e.bounds();
    }
    let natural = ed.measure.measure(t, theme, None).0;
    if !(natural > 0.0 && natural < e.size.w) {
        return e.bounds();
    }
    let spare = (e.size.w - natural) / 2.0;
    let off = match t.align {
        crate::model::Align::Left => -spare,
        crate::model::Align::Center => 0.0,
        crate::model::Align::Right => spare,
    };
    let mut ink = e.clone();
    ink.size.w = natural;
    let shift = crate::geom::Vec2::new(off * e.transform.scale, 0.0).rotate_deg(e.transform.rotation);
    ink.transform.x += shift.x;
    ink.transform.y += shift.y;
    ink.bounds()
}

/// The screen lint judges text size on: a 13- to 15-inch Mac laptop,
/// presenting full screen (points).
pub const LINT_SCREEN: crate::geom::Vec2 = crate::geom::Vec2 { x: 1440.0, y: 900.0 };

/// Layout lint: things a human would want fixed before presenting.
pub fn lint(ed: &Editor) -> Vec<Value> {
    let d = &ed.doc;
    let theme = ed.theme();
    let mut issues = vec![];
    let frames: Vec<&Element> = d.ordered().filter(|e| e.is_frame()).collect();
    // Lines and connectors run between things; they don't sit in a frame.
    let placed = |e: &Element| !e.is_frame() && !matches!(e.kind, ElementKind::Connector(_));
    for e in d.ordered().filter(|e| placed(e)) {
        let b = ink_bounds(ed, &theme, e);
        // The frame it mostly sits in: the most overlap, then the smallest.
        let home = frames
            .iter()
            .filter(|f| f.visual_bounds().area() > b.area())
            .map(|f| (*f, overlap(&f.visual_bounds(), &b)))
            .filter(|(_, a)| *a > 0.0)
            .max_by(|x, y| x.1.total_cmp(&y.1).then(y.0.visual_bounds().area().total_cmp(&x.0.visual_bounds().area())));
        if let Some((f, _)) = home {
            let fb = f.visual_bounds();
            if !fb.contains_rect(&b) {
                let sides: Vec<String> = [
                    ("left", fb.x0 - b.x0),
                    ("right", b.x1 - fb.x1),
                    ("top", fb.y0 - b.y0),
                    ("bottom", b.y1 - fb.y1),
                ]
                .iter()
                .filter(|(_, px)| *px > 0.5)
                .map(|(s, px)| format!("{px:.0} on the {s}"))
                .collect();
                issues.push(json!({"kind": "outside_frame", "id": e.id, "frame": f.id,
                    "message": format!("{} {} runs past the edge of frame {} by {}", e.kind.type_name(), e.id, f.id, sides.join(", "))}));
            }
        }
        // Text too small at the step that shows it.
        let ElementKind::Text(t) = &e.kind else { continue };
        let font_world = theme.text_style(t.style).size * e.transform.scale;
        // The tightest step that shows it: its own frame, a frame it mostly
        // sits in, or a step that reveals it as a build.
        let best = d
            .path
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let builds = s.builds.iter().any(|x| *x == e.id);
                match &s.target {
                    StepTarget::Element { id } => d
                        .get(id)
                        .filter(|f| f.id == e.id || builds || overlap(&f.visual_bounds(), &b) >= 0.5 * b.area())
                        .map(|f| (i, f.visual_bounds().area(), f)),
                    _ => None,
                }
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _, f)) = best {
            // On a typical laptop screen, presenting full screen, not in
            // whatever window the person happens to have open.
            let px = font_world * ed.camera_for_element_in(f, LINT_SCREEN).zoom;
            if px < 14.0 {
                issues.push(json!({"kind": "text_too_small", "id": e.id, "step": i,
                    "message": format!("text is {px:.0}pt tall at step {} presenting full screen on a {}x{} screen; aim for 18pt+", i + 1, LINT_SCREEN.x, LINT_SCREEN.y)}));
            }
        } else if !d.path.is_empty() && !d.path.iter().any(|s| matches!(s.target, StepTarget::Overview)) {
            issues.push(json!({"kind": "not_on_path", "id": e.id, "message": "text is not in any path step's frame and no step reveals it; the audience only sees it in the overview"}));
        }
    }
    // Overlapping elements.
    let items: Vec<&Element> = d.ordered().filter(|e| placed(e)).collect();
    // Each element is measured once, and only pairs whose boxes can touch
    // are looked at (sorted by left edge): comparing every pair, with a
    // text measurement for each, took minutes on a deck of thousands.
    let boxes: Vec<crate::geom::Rect> = items.iter().map(|e| ink_bounds(ed, &theme, e)).collect();
    let mut by_left: Vec<usize> = (0..items.len()).collect();
    by_left.sort_by(|a, b| boxes[*a].x0.total_cmp(&boxes[*b].x0));
    let mut pairs: Vec<(usize, usize)> = vec![];
    for (p, &i) in by_left.iter().enumerate() {
        for &j in &by_left[p + 1..] {
            if boxes[j].x0 >= boxes[i].x1 {
                break;
            }
            if overlap(&boxes[i], &boxes[j]) > 0.0 {
                pairs.push((i.min(j), i.max(j)));
            }
        }
    }
    // Reported in drawing order, as before.
    pairs.sort_unstable();
    {
        for (i, j) in pairs {
            let (a, b) = (&items[i], &items[j]);
            let (ba, bb) = (boxes[i], boxes[j]);
            let ix = overlap(&ba, &bb);
            if ix <= 0.0 {
                continue;
            }
            // A label on its shape: fine while it fits, worth saying when
            // it runs past the shape's edge.
            let label = match (&a.kind, &b.kind) {
                (ElementKind::Text(_), ElementKind::Shape(_)) => Some((a, b, ba, bb)),
                (ElementKind::Shape(_), ElementKind::Text(_)) => Some((b, a, bb, ba)),
                _ => None,
            };
            if let Some((t, s, tb, sb)) = label {
                if ix >= 0.5 * tb.area() {
                    if !sb.contains_rect(&tb) {
                        issues.push(json!({"kind": "label_overflow", "ids": [s.id, t.id],
                            "message": format!("text {} runs past the edge of shape {}", t.id, s.id)}));
                    }
                    continue;
                }
            }
            let frac = ix / ba.area().min(bb.area()).max(1e-9);
            // a small element fully on a big shape is probably intentional
            let size_ratio = ba.area().max(bb.area()) / ba.area().min(bb.area()).max(1e-9);
            if frac > 0.2 && size_ratio < 6.0 {
                issues.push(json!({"kind": "overlap", "ids": [a.id, b.id], "message": format!("{} and {} overlap ({:.0}%)", a.id, b.id, frac * 100.0)}));
            }
        }
    }
    // Steps pointing at empty frames.
    for (i, s) in d.path.iter().enumerate() {
        if let StepTarget::Element { id } = &s.target {
            if d.get(id).is_some_and(|e| e.is_frame()) && ed.frame_contents(id).is_empty() {
                issues.push(json!({"kind": "empty_step", "step": i, "id": id, "message": format!("step {} shows an empty frame", i + 1)}));
            }
        }
    }
    issues
}

/// Area of the intersection of two rectangles (0 when apart).
fn overlap(a: &crate::geom::Rect, b: &crate::geom::Rect) -> f64 {
    crate::geom::Rect { x0: a.x0.max(b.x0), y0: a.y0.max(b.y0), x1: a.x1.min(b.x1), y1: a.y1.min(b.y1) }.area()
}
