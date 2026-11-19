//! Text runs placed on a page, grouped into lines and text boxes, and the
//! deck's text sizes ranked into the theme's styles.
//!
//! The source's line breaks are kept: every line becomes its own
//! paragraph (a wrapped bullet's later lines are indented one level, with
//! no bullet), so nothing reflows in the theme's font.
use std::collections::BTreeMap;

use zebra_core::{Align, ListKind, ParaFormat, SpanFormat, TextData, TextSpan, TextStyle, Theme};

/// One run of text as placed on the page. Page points, y down.
#[derive(Debug, Clone)]
pub struct Run {
    pub text: String,
    /// Ink box.
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    /// Along the baseline and across it (y up), in the run's own rotation:
    /// runs of one line share `v`.
    pub u: f64,
    pub v: f64,
    /// Font size in points as drawn.
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    /// `#rrggbb` or `#rrggbbaa`.
    pub color: String,
    /// Degrees, counter-clockwise (PDF).
    pub rot: f64,
    /// Font flagged symbolic (Symbol, Wingdings): single characters are bullets.
    pub symbolic: bool,
    /// Draw order on the page.
    pub z: f64,
    /// Part of an equation (maths characters): its line comes in as a picture.
    pub math: bool,
}

impl Run {
    fn len_along(&self) -> f64 {
        let t = self.rot.to_radians();
        (self.x1 - self.x0) * t.cos().abs() + (self.y1 - self.y0) * t.sin().abs()
    }
}

/// A text box made from lines that belong together. Page points, y down.
#[derive(Debug, Clone)]
pub struct Block {
    pub data: TextData,
    /// The box's main size in points (most characters).
    pub size: f64,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    /// Degrees, clockwise (Zebra).
    pub rotation: f64,
    pub z: f64,
}

/// A box drawn on the page (rectangle, rounded rectangle, ellipse): lines
/// inside different ones are never one text box.
pub type Container = (f64, f64, f64, f64);

fn is_bullet_char(t: &str, symbolic: bool) -> bool {
    let mut c = t.chars();
    let (Some(ch), None) = (c.next(), c.next()) else { return false };
    symbolic && !ch.is_alphanumeric()
        || "•◦▪▫‣⁃–-»●○■□➢➤✓✔▶►§Øüqnl".contains(ch)
        || ('\u{e000}'..='\u{f8ff}').contains(&ch)
}

/// Lines, then boxes. `page_w` helps tell a centred single line.
pub fn blocks(runs: Vec<Run>, containers: &[Container], page_w: f64) -> Vec<Block> {
    boxes(lines(runs), containers, page_w)
}

/// Runs on one baseline, close along it, left to right; top line first.
pub fn lines(mut runs: Vec<Run>) -> Vec<Vec<Run>> {
    runs.retain(|r| !r.text.trim().is_empty() && r.size > 0.0 && r.size.is_finite());
    // The same text drawn twice in the same place (PowerPoint's bullets,
    // shadows, fake bold) is one run.
    let mut kept: Vec<Run> = Vec::with_capacity(runs.len());
    for r in runs {
        let near = 0.15 * r.size;
        // Drawn again on top: the copy on top is the one that shows.
        match kept.iter_mut().find(|k| k.text.trim() == r.text.trim() && (k.x0 - r.x0).abs() < near && (k.y1 - r.y1).abs() < near && (k.size - r.size).abs() < near) {
            Some(k) => k.z = k.z.max(r.z),
            None => kept.push(r),
        }
    }
    let mut runs = kept;
    // 1. Lines: runs on one baseline, close along it.
    runs.sort_by(|a, b| b.v.total_cmp(&a.v).then(a.u.total_cmp(&b.u)));
    let mut lines: Vec<Vec<Run>> = vec![];
    for r in runs {
        let joined = lines.iter_mut().rev().take(6).find(|l| {
            let last = l.last().unwrap();
            let size = last.size.min(r.size);
            (last.v - r.v).abs() < 0.3 * size
                && (last.rot - r.rot).abs() < 0.5
                && r.u - (last.u + last.len_along()) < 1.5 * last.size.max(r.size)
                && r.u > last.u - 0.5
        });
        match joined {
            Some(l) => {
                l.push(r);
                l.sort_by(|a, b| a.u.total_cmp(&b.u));
            }
            None => lines.push(vec![r]),
        }
    }
    lines.sort_by(|a, b| b[0].v.total_cmp(&a[0].v));
    lines
}

/// Lines one under the other, in the same column and drawn box, become
/// one text box.
/// Lines that are (part of) an equation: maths characters, symbol-font
/// characters past the bullet, and the lines touching those (sub- and
/// superscripts, fraction bars' numerators and denominators).
pub fn split_math(lines: Vec<Vec<Run>>) -> (Vec<Vec<Run>>, Vec<Vec<Run>>) {
    let pua = |c: char| ('\u{e000}'..='\u{f8ff}').contains(&c);
    // A symbol-font character starting a line of words is a bullet.
    let bullet = |l: &Vec<Run>| l.len() > 1 && l[0].text.trim().chars().count() == 1 && l[1..].iter().flat_map(|r| r.text.chars()).filter(|c| c.is_alphabetic()).count() >= 3;
    let mathy = |l: &Vec<Run>| l.iter().enumerate().any(|(i, r)| r.math || (r.text.chars().any(pua) && !(i == 0 && bullet(l))));
    let (mut math, mut rest): (Vec<_>, Vec<_>) = lines.into_iter().partition(mathy);
    let ext = |l: &Vec<Run>, pad: f64| {
        let s = l.iter().map(|r| r.size).fold(0.0, f64::max) * pad;
        (
            l.iter().map(|r| r.x0).fold(f64::MAX, f64::min) - s,
            l.iter().map(|r| r.y0).fold(f64::MAX, f64::min) - s,
            l.iter().map(|r| r.x1).fold(f64::MIN, f64::max) + s,
            l.iter().map(|r| r.y1).fold(f64::MIN, f64::max) + s,
        )
    };
    loop {
        let boxes: Vec<_> = math.iter().map(|l| ext(l, 0.2)).collect();
        let (near, far): (Vec<_>, Vec<_>) = rest.into_iter().partition(|l| {
            let (x0, y0, x1, y1) = ext(l, 0.0);
            boxes.iter().any(|b| x0 < b.2 && b.0 < x1 && y0 < b.3 && b.1 < y1)
        });
        rest = far;
        if near.is_empty() {
            break;
        }
        math.extend(near);
    }
    (math, rest)
}

pub fn boxes(lines: Vec<Vec<Run>>, containers: &[Container], page_w: f64) -> Vec<Block> {
    let ext = |l: &[Run]| {
        (
            l.iter().map(|r| r.x0).fold(f64::MAX, f64::min),
            l.iter().map(|r| r.y0).fold(f64::MAX, f64::min),
            l.iter().map(|r| r.x1).fold(f64::MIN, f64::max),
            l.iter().map(|r| r.y1).fold(f64::MIN, f64::max),
        )
    };
    let lsize = |l: &[Run]| l.iter().map(|r| r.size).fold(0.0, f64::max);
    let container = |l: &[Run]| {
        let (x0, y0, x1, y1) = ext(l);
        containers
            .iter()
            .enumerate()
            .filter(|(_, c)| c.0 <= x0 + 1.0 && c.1 <= y0 + 1.0 && c.2 >= x1 - 1.0 && c.3 >= y1 - 1.0)
            .min_by(|a, b| ((a.1.2 - a.1.0) * (a.1.3 - a.1.1)).total_cmp(&((b.1.2 - b.1.0) * (b.1.3 - b.1.1))))
            .map(|(i, _)| i)
    };
    let mut groups: Vec<Vec<Vec<Run>>> = vec![];
    for l in lines {
        let (lx0, _, lx1, _) = ext(&l);
        let sz = lsize(&l);
        let cont = container(&l);
        let found = groups.iter_mut().rev().find(|g| {
            let prev = g.last().unwrap();
            let (px0, _, px1, _) = ext(prev);
            let psz = lsize(prev);
            let gap = prev[0].v - l[0].v;
            let same_col = (lx0 - px0).abs() < 3.0 * sz || ((lx0 + lx1) / 2.0 - (px0 + px1) / 2.0).abs() < sz;
            (prev[0].rot - l[0].rot).abs() < 0.5
                && gap > 0.0
                && gap < 1.9 * psz.max(sz)
                && same_col
                && lx0 < px1
                && lx1 > px0
                && sz / psz < 1.6
                && psz / sz < 1.6
                && container(prev) == cont
        });
        match found {
            Some(g) => g.push(l),
            None => groups.push(vec![l]),
        }
    }
    groups.into_iter().map(|g| block(g, page_w)).collect()
}

fn block(lines: Vec<Vec<Run>>, page_w: f64) -> Block {
    let all: Vec<&Run> = lines.iter().flatten().collect();
    let x0 = all.iter().map(|r| r.x0).fold(f64::MAX, f64::min);
    let x1 = all.iter().map(|r| r.x1).fold(f64::MIN, f64::max);
    let y0 = all.iter().map(|r| r.y0).fold(f64::MAX, f64::min);
    let y1 = all.iter().map(|r| r.y1).fold(f64::MIN, f64::max);
    // The box's size and colour: those of most characters.
    let mut sizes: BTreeMap<i64, usize> = BTreeMap::new();
    let mut colors: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &all {
        let n = r.text.chars().count();
        *sizes.entry((r.size * 10.0).round() as i64).or_default() += n;
        *colors.entry(r.color.as_str()).or_default() += n;
    }
    let base = *sizes.iter().max_by_key(|(_, n)| **n).unwrap().0 as f64 / 10.0;
    let color = colors.iter().max_by_key(|(_, n)| **n).unwrap().0.to_string();
    let all_bold = all.iter().all(|r| r.bold);
    let all_italic = all.iter().all(|r| r.italic);

    let line_x0 = |l: &Vec<Run>| l[0].x0;
    // A bullet as its own run, or at the start of the line's first run
    // ("•\u{2002}Text").
    let inline_bullet = |l: &Vec<Run>| {
        let t = l[0].text.trim_start();
        let mut c = t.chars();
        match (c.next(), c.next()) {
            (Some(b), Some(ws)) if ws.is_whitespace() && is_bullet_char(&b.to_string(), false) && c.as_str().trim().chars().count() > 0 => Some(t.len() - c.as_str().len()),
            _ => None,
        }
    };
    let is_bullet = |l: &Vec<Run>| (l.len() > 1 && is_bullet_char(l[0].text.trim(), l[0].symbolic)) || inline_bullet(l).is_some();
    let mut bullet_xs: Vec<f64> = lines.iter().filter(|l| is_bullet(l)).map(line_x0).collect();
    bullet_xs.sort_by(f64::total_cmp);
    bullet_xs.dedup_by(|a, b| (*a - *b).abs() < 4.0);

    // Alignment: centred when line centres agree but left edges don't.
    let lefts: Vec<f64> = lines.iter().map(line_x0).collect();
    let centres: Vec<f64> = lines.iter().map(|l| (l[0].x0 + l.last().unwrap().x1) / 2.0).collect();
    let rights: Vec<f64> = lines.iter().map(|l| l.last().unwrap().x1).collect();
    let spread = |v: &[f64]| v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min);
    let any_bullets = !bullet_xs.is_empty();
    let align = if any_bullets {
        Align::Left
    } else if lines.len() > 1 && spread(&centres) < 0.5 * base && spread(&lefts) > base {
        Align::Center
    } else if lines.len() > 1 && spread(&rights) < 0.5 * base && spread(&lefts) > base {
        Align::Right
    } else if lines.len() == 1 && page_w > 0.0 && ((x0 + x1) / 2.0 - page_w / 2.0).abs() < page_w * 0.01 && x0 > page_w * 0.1 {
        Align::Center
    } else {
        Align::Left
    };

    let mut text = String::new();
    let mut spans: Vec<TextSpan> = vec![];
    let mut paras: Vec<ParaFormat> = vec![];
    // The level of the bullet a later line continues, if any.
    let mut last_bullet: Option<(u8, f64)> = None;
    for (li, l) in lines.iter().enumerate() {
        if li > 0 {
            text.push('\n');
        }
        let bullet = is_bullet(l);
        let mut para = ParaFormat::default();
        if bullet {
            let level = bullet_xs.iter().position(|x| (x - l[0].x0).abs() < 4.0).unwrap_or(0).min(8) as u8;
            para.indent = level;
            para.list = Some(ListKind::Bullet);
            let text_x = if inline_bullet(l).is_some() && !(l.len() > 1 && is_bullet_char(l[0].text.trim(), l[0].symbolic)) { l[0].x0 + base } else { l[1].x0 };
            last_bullet = Some((level, text_x));
        } else if let Some((level, text_x)) = last_bullet {
            if (l[0].x0 - text_x).abs() < 0.6 * base {
                // A wrapped bullet's next line.
                para.indent = (level + 1).min(8);
            } else {
                last_bullet = None;
            }
        }
        paras.push(para);
        let mut prev_x1: Option<f64> = None;
        let mut first = true;
        let own_run = l.len() > 1 && is_bullet_char(l[0].text.trim(), l[0].symbolic);
        for (ri, r) in l.iter().enumerate() {
            if ri == 0 && bullet && own_run {
                continue;
            }
            let mut t = r.text.replace(['\r', '\n'], "");
            if ri == 0 && bullet {
                let cut = inline_bullet(l).unwrap_or(0);
                t = r.text.trim_start()[cut..].replace(['\r', '\n'], "");
            }
            if first {
                t = t.trim_start().to_string();
            }
            if let Some(px) = prev_x1 {
                if r.x0 - px > 0.15 * r.size && !text.ends_with(' ') && !t.starts_with(' ') {
                    text.push(' ');
                }
            }
            let start = text.chars().count();
            text.push_str(&t);
            let end = text.chars().count();
            let mut f = SpanFormat::default();
            if r.bold != all_bold {
                f.bold = Some(r.bold);
            }
            if r.italic != all_italic {
                f.italic = Some(r.italic);
            }
            if r.color != color {
                f.color = Some(r.color.clone());
            }
            if (r.size - base).abs() > 0.2 {
                f.size = Some(r.size / base);
            }
            if f != SpanFormat::default() && end > start {
                spans.push(TextSpan { start, end, format: f });
            }
            prev_x1 = Some(r.x1);
            first = false;
        }
        // Trailing spaces only.
        let trimmed = text.trim_end_matches(' ').len();
        text.truncate(trimmed);
    }
    let n = text.chars().count();
    let mut merged: Vec<TextSpan> = vec![];
    for mut s in spans {
        s.end = s.end.min(n);
        if s.start >= s.end {
            continue;
        }
        match merged.last_mut() {
            // Neighbours with the same formatting (spaces between them too).
            Some(p) if p.format == s.format && text.chars().skip(p.end).take(s.start - p.end).all(|c| c == ' ') => p.end = s.end,
            _ => merged.push(s),
        }
    }
    let data = TextData {
        text,
        style: TextStyle::Body,
        bold: all_bold,
        italic: all_italic,
        align,
        bullets: false,
        color: Some(color),
        fixed_width: false,
        spans: merged,
        paras: if paras.iter().any(|p| *p != ParaFormat::default()) { paras } else { vec![] },
    };
    let z = all.iter().map(|r| r.z).fold(f64::MAX, f64::min);
    Block { data, size: base, x0, y0, x1, y1, rotation: -lines[0][0].rot, z }
}

/// The deck's body size: the size of most characters (half points).
pub fn body_size(blocks: &[&Block]) -> f64 {
    let mut hist: BTreeMap<i64, usize> = BTreeMap::new();
    for b in blocks {
        *hist.entry((b.size * 2.0).round() as i64).or_default() += b.data.text.chars().count();
    }
    hist.iter().max_by_key(|(_, n)| **n).map(|(k, _)| *k as f64 / 2.0).unwrap_or(18.0)
}

/// The theme style for a box of `size` points in a deck whose body is
/// `body` points: well above body is a title, a little above a subtitle.
pub fn style_for(size: f64, body: f64) -> TextStyle {
    if size >= 1.6 * body {
        TextStyle::Title
    } else if size >= 1.15 * body {
        TextStyle::Subtitle
    } else {
        TextStyle::Body
    }
}

pub fn style_size(theme: &Theme, s: TextStyle) -> f64 {
    match s {
        TextStyle::Title => theme.title.size,
        TextStyle::Subtitle => theme.subtitle.size,
        TextStyle::Body => theme.body.size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, x: f64, baseline: f64, size: f64) -> Run {
        let w = text.chars().count() as f64 * size * 0.5;
        Run {
            text: text.into(),
            x0: x,
            y0: baseline - size * 0.75,
            x1: x + w,
            y1: baseline + size * 0.2,
            u: x,
            v: -baseline,
            size,
            bold: false,
            italic: false,
            color: "#000000".into(),
            rot: 0.0,
            symbolic: false,
            z: 0.0,
            math: false,
        }
    }

    #[test]
    fn line_breaks_are_kept_and_bullets_found() {
        let runs = vec![
            run("•", 50.0, 100.0, 20.0),
            run("First point that", 70.0, 100.0, 20.0),
            run("wraps here", 70.0, 124.0, 20.0),
            run("•", 50.0, 150.0, 20.0),
            run("Second", 70.0, 150.0, 20.0),
        ];
        let b = blocks(runs, &[], 720.0);
        assert_eq!(b.len(), 1);
        let t = &b[0].data;
        assert_eq!(t.text, "First point that\nwraps here\nSecond");
        assert_eq!(t.paras[0].list, Some(ListKind::Bullet));
        assert_eq!(t.paras[1].list, None);
        assert_eq!(t.paras[1].indent, 1);
        assert_eq!(t.paras[2].list, Some(ListKind::Bullet));
    }

    #[test]
    fn lines_of_different_sizes_close_together_stay_apart() {
        // An author line and a smaller line under it: two lines, never one.
        let runs = vec![run("Jeremy L. Thompson", 200.0, 200.0, 20.0), run("Advisor:", 260.0, 214.0, 13.0)];
        let b = blocks(runs, &[], 720.0);
        let texts: Vec<&str> = b.iter().map(|b| b.data.text.as_str()).collect();
        assert!(texts.iter().all(|t| !t.contains("Thompson Advisor")), "{texts:?}");
    }

    #[test]
    fn text_in_different_boxes_is_not_merged() {
        let runs = vec![run("Component", 50.0, 100.0, 10.0), run("1", 70.0, 112.0, 10.0), run("Component", 50.0, 130.0, 10.0), run("2", 70.0, 142.0, 10.0)];
        let boxes = [(40.0, 85.0, 120.0, 118.0), (40.0, 120.0, 120.0, 150.0)];
        let b = blocks(runs, &boxes, 720.0);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].data.text, "Component\n1");
    }

    #[test]
    fn sizes_rank_into_styles() {
        assert_eq!(style_for(40.0, 20.0), TextStyle::Title);
        assert_eq!(style_for(24.0, 20.0), TextStyle::Subtitle);
        assert_eq!(style_for(20.0, 20.0), TextStyle::Body);
        assert_eq!(style_for(12.0, 20.0), TextStyle::Body);
    }

    #[test]
    fn mixed_formatting_becomes_spans() {
        let mut bold = run("bold", 120.0, 100.0, 20.0);
        bold.bold = true;
        let mut red = run("red", 170.0, 100.0, 30.0);
        red.color = "#ff0000".into();
        let b = blocks(vec![run("plain", 50.0, 100.0, 20.0), bold, red, run("again", 220.0, 100.0, 20.0)], &[], 720.0);
        let t = &b[0].data;
        assert_eq!(t.text, "plain bold red again");
        assert!(t.spans.iter().any(|s| s.format.bold == Some(true) && s.start == 6 && s.end == 10), "{:?}", t.spans);
        assert!(t.spans.iter().any(|s| s.format.color.as_deref() == Some("#ff0000") && s.format.size == Some(1.5)));
        assert!(!t.bold);
    }
}
