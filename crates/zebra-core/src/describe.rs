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
    let b = e.bounds();
    let mut v = serde_json::to_value(e).unwrap();
    let o = v.as_object_mut().unwrap();
    o.insert("label".into(), json!(e.label(&ed.doc)));
    o.insert("bounds".into(), json!([r(b.x0), r(b.y0), r(b.x1), r(b.y1)]));
    o.insert("parent_frame".into(), json!(ed.doc.parent_frame(&e.id)));
    o.insert("path_steps".into(), json!(ed.doc.steps_using(&e.id)));
    if e.is_frame() {
        o.insert("contents".into(), json!(ed.doc.frame_contents(&e.id)));
    }
    v
}

fn one_line(ed: &Editor, e: &Element) -> String {
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
        ElementKind::Image(i) if !i.alt.is_empty() => {
            let _ = write!(s, " alt={:?}", truncate(&i.alt, 40));
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
    let steps = ed.doc.steps_using(&e.id);
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
    if !d.path.is_empty() {
        out.push_str("Path:\n");
        for (i, s) in d.path.iter().enumerate() {
            let tgt = match &s.target {
                StepTarget::Element { id } => format!("{id} \"{}\"", d.get(id).map(|e| e.label(d)).unwrap_or_default()),
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
            out.push('\n');
        }
    }
    if d.elements.is_empty() {
        out.push_str("Canvas is empty.\n");
    } else {
        out.push_str("Canvas (back to front, children indented under their frame):\n");
        // Build a tree by smallest enclosing frame.
        let parents: std::collections::HashMap<&str, Option<String>> =
            d.z_order.iter().map(|id| (id.as_str(), d.parent_frame(id))).collect();
        fn walk(ed: &Editor, parents: &std::collections::HashMap<&str, Option<String>>, parent: Option<&str>, depth: usize, out: &mut String) {
            for id in &ed.doc.z_order {
                if parents.get(id.as_str()).map(|p| p.as_deref()) == Some(parent) {
                    let e = &ed.doc.elements[id];
                    let _ = writeln!(out, "{}{}", "  ".repeat(depth + 1), one_line(ed, e));
                    if e.is_frame() {
                        walk(ed, parents, Some(id), depth + 1, out);
                    }
                }
            }
        }
        walk(ed, &parents, None, 0, &mut out);
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
                    Some(d.frame_contents(f))
                }
                None => None,
            };
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
                            _ => String::new(),
                        };
                        hay.contains(n.as_str()) || e.name.to_lowercase().contains(n.as_str())
                    })
                })
                .map(|e| element_json(ed, e))
                .collect();
            json!({ "elements": list })
        }
        Query::HitTest { x, y } => {
            let tol = 4.0 / ed.view.camera.zoom;
            json!({ "id": d.hit_test(Vec2::new(x, y), tol).map(|e| e.id.clone()) })
        }
        Query::HitTestScreen { x, y } => {
            let w = ed.view.camera.screen_to_world(Vec2::new(x, y), ed.view.viewport);
            let tol = 4.0 / ed.view.camera.zoom;
            json!({ "id": d.hit_test(w, tol).map(|e| e.id.clone()), "world": [w.x, w.y] })
        }
        Query::Path => {
            let steps: Vec<Value> = d
                .path
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let label = match &s.target {
                        StepTarget::Element { id } => d.get(id).map(|e| e.label(d)).unwrap_or_default(),
                        StepTarget::View { .. } => "view".into(),
                        StepTarget::Overview => "overview".into(),
                    };
                    json!({ "index": i, "label": label, "target": s.target, "builds": s.builds, "notes": s.notes,
                            "duration": s.duration, "camera": ed.camera_for_step(i) })
                })
                .collect();
            json!({ "steps": steps })
        }
        Query::Themes => json!({ "current": d.theme, "themes": Theme::builtins() }),
        Query::Icons { query, limit } => {
            let hits = crate::icons::search(&query, limit);
            let total = if query.trim().is_empty() { crate::icons::all().len() } else { hits.len() };
            json!({
                "total": total,
                "icons": hits.iter().map(|i| json!({ "name": i.name, "tags": i.tags })).collect::<Vec<_>>(),
                "licence": "Lucide icons, ISC licence",
            })
        }
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

/// Layout lint: things a human would want fixed before presenting.
pub fn lint(ed: &Editor) -> Vec<Value> {
    let d = &ed.doc;
    let theme = ed.theme();
    let mut issues = vec![];
    // Text too small at the step that shows it.
    for e in d.ordered() {
        let ElementKind::Text(t) = &e.kind else { continue };
        let font_world = theme.text_style(t.style).size * e.transform.scale;
        // the step whose target most tightly contains this text
        let best = d
            .path
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match &s.target {
                StepTarget::Element { id } => d.get(id).filter(|f| f.bounds().contains_rect(&e.bounds()) || f.id == e.id).map(|f| (i, f.bounds().area())),
                _ => None,
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = best {
            let px = font_world * ed.camera_for_step(i).zoom;
            if px < 14.0 {
                issues.push(json!({"kind": "text_too_small", "id": e.id, "step": i,
                    "message": format!("text is {px:.0}px tall at step {} (viewport {}x{}); aim for 18px+", i + 1, ed.view.viewport.x, ed.view.viewport.y)}));
            }
        } else if !d.path.is_empty() && !d.path.iter().any(|s| matches!(s.target, StepTarget::Overview)) {
            issues.push(json!({"kind": "not_on_path", "id": e.id, "message": "text is not inside any path step; the audience only sees it in the overview"}));
        }
        if let Some(pf) = d.parent_frame(&e.id) {
            let _ = pf;
        }
    }
    // Overlapping non-frame elements.
    let items: Vec<&Element> = d.ordered().filter(|e| !e.is_frame()).collect();
    for (i, a) in items.iter().enumerate() {
        for b in &items[i + 1..] {
            let (ba, bb) = (a.bounds(), b.bounds());
            if !ba.intersects(&bb) {
                continue;
            }
            let ix = crate::geom::Rect { x0: ba.x0.max(bb.x0), y0: ba.y0.max(bb.y0), x1: ba.x1.min(bb.x1), y1: ba.y1.min(bb.y1) };
            let frac = ix.area() / ba.area().min(bb.area()).max(1e-9);
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
            if d.get(id).is_some_and(|e| e.is_frame()) && d.frame_contents(id).is_empty() {
                issues.push(json!({"kind": "empty_step", "step": i, "id": id, "message": format!("step {} shows an empty frame", i + 1)}));
            }
        }
    }
    issues
}
