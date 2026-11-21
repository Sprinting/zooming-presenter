//! Agent perception: the text outline (`describe`) and structured queries.
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

/// `t` in double quotes for a one-line listing: only `"`, `\`, control
/// characters and bidi overrides are escaped, so every script reads as written (Rust's `{:?}`
/// escaped Devanagari's combining marks as `\u{94d}`).
pub fn quoted(t: &str) -> String {
    let mut out = String::with_capacity(t.len() + 2);
    out.push('"');
    for ch in t.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Bidi overrides would reorder the rest of the line.
            c if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
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
                let _ = write!(s, " {}", quoted(&txt));
            } else {
                // Rich text reads back as Markdown (format_text and format_paragraphs set it).
                let txt = truncate(&crate::rich::to_markdown(d).replace('\n', " / "), 90);
                let _ = write!(s, " md{}", quoted(&txt));
            }
        }
        ElementKind::Shape(d) => {
            let _ = write!(s, " {}", serde_json::to_value(d.shape).unwrap().as_str().unwrap());
        }
        ElementKind::Icon(i) => {
            let _ = write!(s, " {}", i.name);
        }
        ElementKind::Table(t) => {
            let _ = write!(s, " {}x{} {}", t.n_rows(), t.n_cols(), t.style.id());
            for (n, w) in [(t.header_rows, "header row"), (t.footer_rows, "footer row"), (t.header_columns, "header column")] {
                if n > 0 && !(w == "header row" && n == 1) {
                    let _ = write!(s, " {n} {w}{}", if n > 1 { "s" } else { "" });
                }
            }
            if t.header_rows == 0 {
                s.push_str(" no header row");
            }
            if !t.merges.is_empty() {
                let m: Vec<String> = t.merges.iter().map(|m| m.a1()).collect();
                let _ = write!(s, " merged {}", m.join(","));
            }
        }
        ElementKind::Chart(c) => {
            let _ = write!(s, " {}", c.chart_type.key());
            if c.look != crate::chart::ChartLook::Flat {
                let _ = write!(s, " {}", c.look.key());
            }
            if !c.title.is_empty() {
                let _ = write!(s, " title={}", quoted(&truncate(&c.title, 40)));
            }
            let p = c.plot(&ed.doc);
            let _ = write!(s, " {} series x {} categories", p.series.len(), p.categories.len());
            if p.cut {
                let _ = write!(s, " (of {} x {}: draws the first part)", p.all_series, p.all_categories);
            }
            if let Some(l) = &c.link {
                let _ = write!(s, " linked to {}", l.table);
                if let Some(r) = &l.range {
                    let _ = write!(s, " {}", r.a1());
                }
            }
            if c.series_in == crate::chart::SeriesIn::Rows {
                s.push_str(" series in rows");
            }
        }
        ElementKind::Connector(c) => {
            let end = |e: &crate::model::Endpoint| e.element.clone().unwrap_or_else(|| "point".into());
            let _ = write!(s, " {} -> {}", end(&c.from), end(&c.to));
            if !c.label.is_empty() {
                let _ = write!(s, " label={}", quoted(&truncate(&c.label, 30)));
            }
        }
        ElementKind::Image(i) => {
            if !i.alt.is_empty() {
                let _ = write!(s, " alt={}", quoted(&truncate(&i.alt, 40)));
            }
            if i.crop.is_some() {
                s.push_str(" cropped");
            }
            if i.mask != ImageMask::Rectangle {
                let _ = write!(s, " mask {}", serde_json::to_value(i.mask).unwrap().as_str().unwrap());
            }
        }
        ElementKind::Video(v) => {
            if let Some(l) = &v.link {
                let kind = serde_json::to_value(l.kind).ok().and_then(|k| k.as_str().map(str::to_string)).unwrap_or_default();
                let _ = write!(s, " {} link={}", if l.kind.is_card() { format!("{kind} card") } else { format!("web {kind}") }, quoted(&l.url));
            }
            if !v.alt.is_empty() {
                let _ = write!(s, " alt={}", quoted(&truncate(&v.alt, 40)));
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
        let _ = write!(s, " name={}", quoted(&e.name));
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
    // Text style weights, once a style isn't Regular (the `themes` query
    // always has them).
    let th = ed.theme();
    if [&th.title, &th.subtitle, &th.body].iter().any(|s| s.weight != crate::theme::REGULAR) {
        let one = |n: &str, s: &crate::theme::TextStyleDef| {
            let w = crate::theme::weight_name(s.weight).map(str::to_string).unwrap_or_else(|| s.weight.to_string());
            format!("{n} {} {w} {}", s.font, r(s.size))
        };
        let _ = writeln!(out, "Styles: {} · {} · {}", one("title", &th.title), one("subtitle", &th.subtitle), one("body", &th.body));
    }
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
                let _ = write!(out, " notes: {}", quoted(&truncate(&s.notes, 50)));
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
                if let ElementKind::Table(t) = &e.kind {
                    for line in table_lines(t, DESCRIBE_ROWS) {
                        let _ = writeln!(out, "{}{line}", "  ".repeat(depth + 2));
                    }
                }
                if let ElementKind::Chart(c) = &e.kind {
                    for line in chart_lines(&c.plot(&ed.doc), DESCRIBE_VALUES) {
                        let _ = writeln!(out, "{}{line}", "  ".repeat(depth + 2));
                    }
                }
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

/// Rows of a table `describe` lists; the `table` query has them all.
const DESCRIBE_ROWS: usize = 20;

/// A table as lines of text: row number, then each cell as it shows,
/// with a formula after its value (`£2,220,000 =SUM(B2:B3)`) and merged
/// cells shown once.
/// Values per series a chart's description shows.
pub const DESCRIBE_VALUES: usize = 20;

/// A chart's data as lines: the categories, then each series' values (the
/// first `max` of each, then how many more).
pub fn chart_lines(p: &crate::chart::Plot, max: usize) -> Vec<String> {
    let more = |n: usize| if n > max { format!(" …and {} more", n - max) } else { String::new() };
    let num = |v: f64| if v.is_finite() { crate::table::plain_number(v) } else { "-".into() };
    let mut out = vec![];
    if !p.categories.is_empty() {
        let cats: Vec<String> = p.categories.iter().take(max).map(|c| truncate(c, 24)).collect();
        out.push(format!("categories: {}{}", cats.join(", "), more(p.categories.len())));
    }
    for s in &p.series {
        let vals: Vec<String> = (0..s.values.len().min(max))
            .map(|i| match (s.x.get(i), s.sizes.get(i)) {
                (Some(x), Some(z)) => format!("({}, {}, {})", num(*x), num(s.values[i]), num(*z)),
                (Some(x), None) => format!("({}, {})", num(*x), num(s.values[i])),
                _ => num(s.values[i]),
            })
            .collect();
        out.push(format!("{}: {}{}", truncate(&s.name, 30), vals.join(", "), more(s.values.len())));
    }
    if p.cut {
        out.push(format!("(draws the first {} of {} series and {} of {} categories)", p.series.len(), p.all_series, p.categories.len(), p.all_categories));
    }
    out
}

pub fn table_lines(t: &crate::table::TableData, max_rows: usize) -> Vec<String> {
    let looks: Vec<_> = (0..t.n_cols()).map(|c| t.column_look(c)).collect();
    let mut out = vec![];
    let n = t.n_rows();
    for r in 0..n.min(max_rows) {
        let mut cells = vec![];
        for c in 0..t.n_cols() {
            if t.anchor(r, c) != (r, c) {
                cells.push("←".to_string());
                continue;
            }
            let shown = t.shown(r, c, looks[c].as_ref());
            let mut v = match shown.tick {
                Some(on) => (if on { "☑" } else { "☐" }).to_string(),
                None => truncate(&shown.text.replace('\n', " / "), 40),
            };
            let cell = &t.cells[r][c];
            if cell.is_formula() {
                v = if v.is_empty() { cell.input.clone() } else { format!("{v} {}", cell.input) };
            }
            cells.push(v);
        }
        out.push(format!("{}: {}", r + 1, cells.join(" | ")));
    }
    if n > max_rows {
        out.push(format!("… {} more rows (table query)", n - max_rows));
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
                // Document Warnings: what opening or importing couldn't
                // bring in (the person's View › Show Document Warnings).
                "warnings": d.warnings.iter().map(|w| w.english()).collect::<Vec<_>>(),
                "mode": v.mode,
                "tool": v.tool,
                "selection": v.selection,
                "frame_only": ed.frame_only().is_some(),
                "agent_selection": v.agent_selection,
                "camera": v.camera,
                "agent_camera": v.agent_camera,
                "viewport": [v.viewport.x, v.viewport.y],
                "animating": v.flight.is_some(),
                "present": v.present,
                "agent_paused": v.agent_paused,
                "agent_permission": v.agent_permission,
                "beta_features": v.beta_features,
                "follow_agent": v.follow_agent,
                "human_editing": ed.human_gesture_active(),
                "can_undo": ed.can_undo(),
                "can_redo": ed.can_redo(),
                "element_count": d.elements.len(),
                "path_length": d.path.len(),
            })
        }
        Query::Document => serde_json::to_value(d).unwrap(),
        Query::Table { id, range } => {
            let e = d.get(&id).ok_or_else(|| ZError::not_found(&id, d.elements.keys().cloned()))?;
            let ElementKind::Table(t) = &e.kind else {
                return Err(ZError::invalid(format!("'{id}' is a {}, not a table", e.kind.type_name())));
            };
            let whole = t.whole();
            let range = match range {
                Some(r) => r.clip(t.n_rows(), t.n_cols()).ok_or_else(|| ZError::invalid(format!("{} is outside the table", r.a1())))?,
                None => whole,
            };
            let mut formulas = serde_json::Map::new();
            let mut errors = serde_json::Map::new();
            for r in range.r0..=range.r1 {
                for c in range.c0..=range.c1 {
                    let cell = &t.cells[r][c];
                    if cell.is_formula() {
                        formulas.insert(crate::table::cell_name(r, c), json!(cell.input));
                    }
                    if let Some(crate::table::CellValue::Error { error }) = &cell.value {
                        errors.insert(crate::table::cell_name(r, c), json!(error));
                    }
                }
            }
            let columns: Vec<Value> = t
                .columns
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let mut v = serde_json::to_value(c).unwrap();
                    v.as_object_mut().unwrap().insert("letter".into(), json!(crate::table::column_name(i)));
                    v
                })
                .collect();
            json!({
                "id": id, "range": range.a1(), "rows": t.n_rows(), "columns": t.n_cols(),
                "style": t.style, "style_name": t.style.name(),
                "header_rows": t.header_rows, "header_columns": t.header_columns, "footer_rows": t.footer_rows,
                "shown": t.shown_rows(&range), "formulas": formulas, "errors": errors,
                "column_info": columns, "merges": t.merges, "cell_formats": t.cell_formats,
                "font_size": t.font_size, "banded": t.banded, "gridlines": t.gridlines, "show_name": t.show_name,
            })
        }
        Query::Chart { id } => {
            let e = d.get(&id).ok_or_else(|| ZError::not_found(&id, d.elements.keys().cloned()))?;
            let ElementKind::Chart(c) = &e.kind else {
                return Err(ZError::invalid(format!("'{id}' is a {}, not a chart", e.kind.type_name())));
            };
            let p = c.plot(d);
            let src = c.source(d);
            let series: Vec<Value> = p
                .series
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let num = |v: &f64| if v.is_finite() { json!(v) } else { Value::Null };
                    let mut o = json!({"index": i, "name": s.name, "values": s.values.iter().map(num).collect::<Vec<_>>()});
                    if !s.x.is_empty() {
                        o["x"] = json!(s.x.iter().map(num).collect::<Vec<_>>());
                    }
                    if !s.sizes.is_empty() {
                        o["sizes"] = json!(s.sizes.iter().map(num).collect::<Vec<_>>());
                    }
                    if let Some(f) = c.series.get(i).filter(|f| !f.is_empty()) {
                        o["format"] = serde_json::to_value(f).unwrap();
                    }
                    o
                })
                .collect();
            let mut v = serde_json::to_value(c).unwrap();
            let o = v.as_object_mut().unwrap();
            o.remove("data");
            o.remove("series");
            o.insert("id".into(), json!(id));
            o.insert("chart_type_code".into(), json!(c.chart_type.code()));
            // What is drawn, of how much: a chart draws at most
            // MAX_SERIES series, MAX_CATEGORIES categories, MAX_VALUES values.
            o.insert(
                "drawn".into(),
                json!({"series": p.series.len(), "categories": p.categories.len(), "of_series": p.all_series, "of_categories": p.all_categories, "cut": p.cut,
                       "limits": {"series": crate::chart::MAX_SERIES, "categories": crate::chart::MAX_CATEGORIES, "values": crate::chart::MAX_VALUES}}),
            );
            if c.axis_step.is_some() && c.step_ignored(&p) {
                o.insert("axis_step_ignored".into(), json!(format!("that step draws more than {} gridlines, so the axis steps automatically", crate::chart::MAX_AXIS_STEPS)));
            }
            o.insert("look_code".into(), json!(c.look.code()));
            o.insert(
                "source".into(),
                if src.linked { json!({"table": c.link.as_ref().map(|l| l.table.clone()), "range": src.range.a1()}) } else { json!({"own": true, "rows": c.data.n_rows(), "columns": c.data.n_cols()}) },
            );
            if !src.linked {
                o.insert("data".into(), json!(c.data.shown_rows(&c.data.whole())));
                let formulas: serde_json::Map<String, Value> = c
                    .data
                    .cells
                    .iter()
                    .enumerate()
                    .flat_map(|(r, row)| row.iter().enumerate().filter(|(_, x)| x.is_formula()).map(move |(cc, x)| (crate::table::cell_name(r, cc), json!(x.input))))
                    .collect();
                if !formulas.is_empty() {
                    o.insert("formulas".into(), Value::Object(formulas));
                }
            }
            o.insert("categories".into(), json!(p.categories));
            o.insert("series".into(), json!(series));
            if p.cut {
                o.insert("cut".into(), json!(true));
            }
            v
        }
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
                            ElementKind::Table(t) => t.cells.iter().flatten().map(|c| c.input.to_lowercase()).collect::<Vec<_>>().join("\n"),
                            ElementKind::Chart(c) => {
                                let p = c.plot(d);
                                std::iter::once(c.title.to_lowercase()).chain(p.categories.iter().chain(p.series.iter().map(|s| &s.name)).map(|s| s.to_lowercase())).collect::<Vec<_>>().join("\n")
                            }
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
        Query::Themes => {
            // My Themes: `set_theme` takes these ids too (it copies the
            // theme into the document, pictures and fonts included).
            let lib = crate::library::get().map(|l| l.themes()).unwrap_or_default();
            let library: Vec<Value> = lib.iter().filter(|(_, t)| !d.themes.iter().any(|c| c.id == t.id)).map(|(_, t)| json!({ "id": t.id, "name": t.name })).collect();
            json!({ "current": d.theme, "themes": Theme::builtins(), "custom": d.themes, "library": library })
        }
        Query::Templates => {
            // My Templates (`apply_template` takes `saved:<name>`) and the
            // sample presentation, where the host has them.
            let lib = crate::library::get();
            let saved: Vec<Value> = lib
                .as_ref()
                .map(|l| l.templates())
                .unwrap_or_default()
                .into_iter()
                .map(|n| json!({ "id": format!("{}{n}", crate::library::SAVED_PREFIX), "name": n }))
                .collect();
            let sample = lib.is_some_and(|l| l.sample().is_some());
            json!({ "templates": &*crate::templates::TEMPLATES, "layouts": &*crate::templates::LAYOUTS, "saved": saved, "sample": sample })
        }
        Query::Fonts { family: None } => json!({ "families": ed.measure.font_families(), "bundled": ["Sans", "Serif", "Mono", "Hand"] }),
        Query::Fonts { family: Some(f) } => {
            let faces = ed.measure.font_faces(&ed.doc, &f);
            let weights: Vec<Value> = faces
                .iter()
                .map(|x| json!({ "weight": x.weight, "name": x.name.as_deref().or(crate::theme::weight_name(x.weight)).map(str::to_string).unwrap_or_else(|| x.weight.to_string()) }))
                .collect();
            json!({ "family": f, "weights": weights })
        }
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
    })
}


#[cfg(test)]
mod tests {
    use super::quoted;

    #[test]
    fn quoted_text_reads_as_written_in_every_script() {
        assert_eq!(quoted("क्षत्रिय"), "\"क्षत्रिय\"");
        assert_eq!(quoted("👨\u{200d}👩\u{200d}👧"), "\"👨\u{200d}👩\u{200d}👧\"");
        assert_eq!(quoted("say \"hi\"\\now"), r#""say \"hi\"\\now""#);
        assert_eq!(quoted("a\nb\tc\u{7}"), r#""a\nb\tc\u{7}""#);
        assert_eq!(quoted("abc\u{202e}def"), r#""abc\u{202e}def""#);
    }
}
