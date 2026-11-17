//! Rich text: formatting runs over a plain string, the way attributed
//! strings work (NSAttributedString, Pages, Keynote).
//!
//! * `TextData::text` stays plain text, so search, describe and old files
//!   keep working.
//! * `TextData::spans` holds character formatting for ranges, in characters
//!   (Unicode scalar values), sorted and non-overlapping after
//!   [`normalize`].
//! * `TextData::paras` holds per-paragraph indent, list kind and alignment.
//!
//! Editing the text keeps formatting attached to the characters it was on
//! ([`edit_text`]): the change is located as a common prefix and suffix and
//! the runs and paragraph entries are shifted around it. Text typed right
//! after a formatted run takes that run's formatting, as in every word
//! processor.
use crate::model::{Align, ListKind, ParaFormat, SpanFormat, TextData, TextSpan};

fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Merge `top` over `base`: set fields in `top` win; empty strings clear.
fn overlay(base: &SpanFormat, top: &SpanFormat) -> SpanFormat {
    fn pick<T: Clone>(b: &Option<T>, t: &Option<T>) -> Option<T> {
        t.clone().or_else(|| b.clone())
    }
    let mut f = SpanFormat {
        bold: pick(&base.bold, &top.bold),
        italic: pick(&base.italic, &top.italic),
        underline: pick(&base.underline, &top.underline),
        strike: pick(&base.strike, &top.strike),
        color: pick(&base.color, &top.color),
        link: pick(&base.link, &top.link),
        size: pick(&base.size, &top.size),
    };
    if f.color.as_deref() == Some("") {
        f.color = None;
    }
    if f.link.as_deref() == Some("") {
        f.link = None;
    }
    if f.size.is_some_and(|s| !s.is_finite() || s <= 0.0 || (s - 1.0).abs() < 1e-9) {
        f.size = None;
    }
    f
}

fn is_plain(f: &SpanFormat) -> bool {
    *f == SpanFormat::default()
}

/// Sort, clip to the text, resolve overlaps (later spans win), drop empty
/// and no-op spans, merge equal neighbours. Also trims `paras` to the
/// paragraph count and drops trailing default entries.
pub fn normalize(t: &mut TextData) {
    let n = char_len(&t.text);
    let mut cuts: Vec<usize> = vec![0, n];
    for s in &t.spans {
        cuts.push(s.start.min(n));
        cuts.push(s.end.min(n));
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut out: Vec<TextSpan> = vec![];
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b {
            continue;
        }
        let mut f = SpanFormat::default();
        for s in &t.spans {
            if s.start.min(n) <= a && s.end.min(n) >= b {
                f = overlay(&f, &s.format);
            }
        }
        if is_plain(&f) {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == a && last.format == f => last.end = b,
            _ => out.push(TextSpan { start: a, end: b, format: f }),
        }
    }
    t.spans = out;

    let paras = t.text.split('\n').count();
    t.paras.truncate(paras);
    for p in &mut t.paras {
        p.indent = p.indent.min(8);
    }
    while t.paras.last().is_some_and(|p| *p == ParaFormat::default()) {
        t.paras.pop();
    }
}

/// Apply `format` to characters `start..end` (set fields override, the rest
/// keep what each character already had).
pub fn apply_format(t: &mut TextData, start: usize, end: usize, format: &SpanFormat) {
    let n = char_len(&t.text);
    let (start, end) = (start.min(n), end.min(n));
    if start >= end {
        return;
    }
    // Split existing spans at the edges, then overlay inside the range.
    t.spans.push(TextSpan { start, end, format: SpanFormat::default() });
    normalize_cuts(t, start, end);
    for s in &mut t.spans {
        if s.start >= start && s.end <= end {
            s.format = overlay(&s.format, format);
        }
    }
    // Characters in the range with no span yet.
    let mut gaps = vec![];
    let mut at = start;
    let mut covered: Vec<(usize, usize)> = t.spans.iter().filter(|s| s.start >= start && s.end <= end).map(|s| (s.start, s.end)).collect();
    covered.sort_unstable();
    for (a, b) in covered {
        if a > at {
            gaps.push((at, a));
        }
        at = at.max(b);
    }
    if at < end {
        gaps.push((at, end));
    }
    for (a, b) in gaps {
        t.spans.push(TextSpan { start: a, end: b, format: overlay(&SpanFormat::default(), format) });
    }
    normalize(t);
}

/// Split spans so none crosses `a` or `b` (keeps formatting identical).
fn normalize_cuts(t: &mut TextData, a: usize, b: usize) {
    let mut out = vec![];
    for s in t.spans.drain(..) {
        let mut pieces = vec![(s.start, s.end)];
        for cut in [a, b] {
            pieces = pieces
                .into_iter()
                .flat_map(|(x, y)| if x < cut && cut < y { vec![(x, cut), (cut, y)] } else { vec![(x, y)] })
                .collect();
        }
        for (x, y) in pieces {
            if x < y {
                out.push(TextSpan { start: x, end: y, format: s.format.clone() });
            }
        }
    }
    t.spans = out;
}

/// The formatting every character in `start..end` shares (fields that
/// differ come back `None`). For toolbar state such as the B button.
pub fn common_format(t: &TextData, start: usize, end: usize) -> SpanFormat {
    let n = char_len(&t.text);
    let (start, end) = (start.min(n), end.min(n.max(start)));
    let at = |i: usize| -> SpanFormat {
        t.spans.iter().find(|s| s.start <= i && i < s.end).map(|s| s.format.clone()).unwrap_or_default()
    };
    let range: Vec<usize> = if start < end { (start..end).collect() } else { vec![start.saturating_sub(1)] };
    let mut it = range.into_iter().map(at);
    let Some(mut acc) = it.next() else { return SpanFormat::default() };
    for f in it {
        if acc.bold != f.bold {
            acc.bold = None;
        }
        if acc.italic != f.italic {
            acc.italic = None;
        }
        if acc.underline != f.underline {
            acc.underline = None;
        }
        if acc.strike != f.strike {
            acc.strike = None;
        }
        if acc.color != f.color {
            acc.color = None;
        }
        if acc.link != f.link {
            acc.link = None;
        }
        if acc.size != f.size {
            acc.size = None;
        }
    }
    acc
}

/// Replace the text, keeping formatting on the characters that survive.
pub fn edit_text(t: &mut TextData, new_text: &str) {
    if t.text == new_text {
        return;
    }
    let old: Vec<char> = t.text.chars().collect();
    let new: Vec<char> = new_text.chars().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old.iter().rev().zip(new.iter().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
    let old_mid_end = old.len() - suffix;
    let new_mid_end = new.len() - suffix;
    let inserted = new_mid_end - prefix;
    let removed = old_mid_end - prefix;

    // Character runs.
    let map = |i: usize, is_end: bool| -> usize {
        if i < prefix || (i == prefix && !is_end) {
            i
        } else if i >= old_mid_end {
            i - removed + inserted
        } else {
            // Inside the replaced text: snap to its edges.
            if is_end { new_mid_end } else { prefix }
        }
    };
    for s in &mut t.spans {
        // Typing right after a formatted run extends it.
        let extends = removed == 0 && s.end == prefix && s.start < prefix;
        s.start = map(s.start, false);
        s.end = if extends { prefix + inserted } else { map(s.end, true) };
    }

    // Paragraphs: newlines removed and inserted around the edit point.
    let para_at = old[..prefix].iter().filter(|c| **c == '\n').count();
    let removed_nl = old[prefix..old_mid_end].iter().filter(|c| **c == '\n').count();
    let inserted_nl = new[prefix..new_mid_end].iter().filter(|c| **c == '\n').count();
    if (removed_nl > 0 || inserted_nl > 0) && !t.paras.is_empty() {
        let here = t.paras.get(para_at).cloned().unwrap_or_default();
        let from = (para_at + 1).min(t.paras.len());
        let to = (para_at + 1 + removed_nl).min(t.paras.len());
        t.paras.splice(from..to, std::iter::repeat_n(here, inserted_nl));
    }

    t.text = new_text.to_string();
    normalize(t);
}

/// Change paragraphs `first..=last` (paragraph indices). `indent_by` adds to
/// the indent level (clamped 0..8).
pub fn format_paragraphs(t: &mut TextData, first: usize, last: usize, list: Option<ListKind>, indent: Option<u8>, indent_by: i32, align: Option<Align>) {
    let count = t.text.split('\n').count();
    let last = last.min(count.saturating_sub(1));
    if t.paras.len() <= last {
        t.paras.resize(last + 1, ParaFormat::default());
    }
    for p in &mut t.paras[first.min(last)..=last] {
        if let Some(l) = list {
            p.list = Some(l);
        }
        if let Some(i) = indent {
            p.indent = i;
        }
        p.indent = (p.indent as i32 + indent_by).clamp(0, 8) as u8;
        if let Some(a) = align {
            p.align = Some(a);
        }
    }
    normalize(t);
}

/// Paragraph index containing character `at`.
pub fn paragraph_of(t: &TextData, at: usize) -> usize {
    t.text.chars().take(at).filter(|c| *c == '\n').count()
}

/// The effective format of paragraph `i`.
pub fn para(t: &TextData, i: usize) -> (ListKind, u8, Align) {
    let p = t.paras.get(i).cloned().unwrap_or_default();
    let list = p.list.unwrap_or(if t.bullets { ListKind::Bullet } else { ListKind::None });
    (list, p.indent, p.align.unwrap_or(t.align))
}

/// Character offset → byte offset in `s`.
pub fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map(|(b, _)| b).unwrap_or(s.len())
}

/// Build a text from a small Markdown subset: `**bold**`, `*italic*`,
/// `~~strike~~`, `[text](url)`, `- ` / `* ` bullets, `1. ` numbered items,
/// nested lists by indentation. Uses `pulldown-cmark`.
pub fn from_markdown(md: &str) -> TextData {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut t = TextData::default();
    let mut text = String::new();
    let mut n = 0usize; // chars written
    let mut stack: Vec<(usize, SpanFormat)> = vec![];
    let mut lists: Vec<ListKind> = vec![];
    let mut spans = vec![];
    let mut paras: Vec<ParaFormat> = vec![];
    let mut para_open = false;
    let start_para = |text: &mut String, n: &mut usize, paras: &mut Vec<ParaFormat>, lists: &Vec<ListKind>, open: &mut bool| {
        if *open {
            return;
        }
        if !text.is_empty() {
            text.push('\n');
            *n += 1;
        }
        let list = lists.last().copied();
        paras.push(ParaFormat { indent: lists.len().saturating_sub(1) as u8, list: Some(list.unwrap_or(ListKind::None)), align: None });
        *open = true;
    };
    for ev in Parser::new_ext(md, Options::ENABLE_STRIKETHROUGH) {
        match ev {
            Event::Start(Tag::List(first)) => {
                lists.push(if first.is_some() { ListKind::Number } else { ListKind::Bullet });
                para_open = false;
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                para_open = false;
            }
            Event::Start(Tag::Item) | Event::Start(Tag::Paragraph) | Event::Start(Tag::Heading { .. }) => {
                start_para(&mut text, &mut n, &mut paras, &lists, &mut para_open);
            }
            Event::End(TagEnd::Item) | Event::End(TagEnd::Paragraph) | Event::End(TagEnd::Heading(_)) => para_open = false,
            Event::Start(Tag::Strong) => stack.push((n, SpanFormat { bold: Some(true), ..Default::default() })),
            Event::Start(Tag::Emphasis) => stack.push((n, SpanFormat { italic: Some(true), ..Default::default() })),
            Event::Start(Tag::Strikethrough) => stack.push((n, SpanFormat { strike: Some(true), ..Default::default() })),
            Event::Start(Tag::Link { dest_url, .. }) => stack.push((n, SpanFormat { link: Some(dest_url.to_string()), ..Default::default() })),
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link) => {
                if let Some((s, f)) = stack.pop() {
                    spans.push(TextSpan { start: s, end: n, format: f });
                }
            }
            Event::Text(s) | Event::Code(s) => {
                start_para(&mut text, &mut n, &mut paras, &lists, &mut para_open);
                text.push_str(&s);
                n += char_len(&s);
            }
            Event::SoftBreak | Event::HardBreak => {
                text.push('\n');
                n += 1;
                let last = paras.last().cloned().unwrap_or_default();
                paras.push(ParaFormat { list: Some(ListKind::None), ..last });
            }
            _ => {}
        }
    }
    t.text = text;
    t.spans = spans;
    t.paras = paras;
    if t.paras.iter().all(|p| p.indent == 0 && p.list.is_none_or(|l| l == ListKind::None) && p.align.is_none()) {
        t.paras.clear();
    }
    normalize(&mut t);
    t
}

/// The text as Markdown (inverse of [`from_markdown`] for what it covers).
/// Used by `describe` so agents can read formatting back.
pub fn to_markdown(t: &TextData) -> String {
    let chars: Vec<char> = t.text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut para_i = 0;
    let mut numbers: Vec<usize> = vec![0; 9];
    let mut line_start = true;
    while i <= chars.len() {
        if line_start {
            let (list, indent, _) = para(t, para_i);
            for _ in 0..indent {
                out.push_str("  ");
            }
            match list {
                ListKind::Bullet => out.push_str("- "),
                ListKind::Number => {
                    numbers[indent as usize] += 1;
                    out.push_str(&format!("{}. ", numbers[indent as usize]));
                }
                ListKind::None => {}
            }
            if list != ListKind::Number {
                numbers[indent as usize] = 0;
            }
            line_start = false;
        }
        if i == chars.len() {
            break;
        }
        if let Some(s) = t.spans.iter().find(|s| s.start == i) {
            let seg: String = chars[s.start..s.end].iter().collect();
            let f = &s.format;
            let mut w = seg.clone();
            if f.strike == Some(true) {
                w = format!("~~{w}~~");
            }
            if f.italic == Some(true) {
                w = format!("*{w}*");
            }
            if f.bold == Some(true) {
                w = format!("**{w}**");
            }
            if let Some(l) = &f.link {
                w = format!("[{w}]({l})");
            }
            out.push_str(&w);
            i = s.end;
            continue;
        }
        let c = chars[i];
        out.push(c);
        if c == '\n' {
            para_i += 1;
            line_start = true;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> SpanFormat {
        SpanFormat { bold: Some(true), ..Default::default() }
    }

    #[test]
    fn format_a_word_then_type_around_it() {
        let mut t = TextData { text: "Hello big world".into(), ..Default::default() };
        apply_format(&mut t, 6, 9, &bold());
        assert_eq!(t.spans, vec![TextSpan { start: 6, end: 9, format: bold() }]);
        edit_text(&mut t, "Oh, Hello big world");
        assert_eq!((t.spans[0].start, t.spans[0].end), (10, 13), "shifted by the insertion before it");
        edit_text(&mut t, "Oh, Hello bigger world");
        assert_eq!((t.spans[0].start, t.spans[0].end), (10, 16), "typing at the end of a bold word continues bold");
        edit_text(&mut t, "Oh, Hello world");
        assert!(t.spans.is_empty(), "deleting the word removes its span");
    }

    #[test]
    fn overlapping_formats_merge_and_unset() {
        let mut t = TextData { text: "abcdefgh".into(), ..Default::default() };
        apply_format(&mut t, 0, 6, &bold());
        apply_format(&mut t, 3, 8, &SpanFormat { italic: Some(true), ..Default::default() });
        assert_eq!(t.spans.len(), 3);
        assert_eq!(common_format(&t, 3, 6).bold, Some(true));
        assert_eq!(common_format(&t, 3, 6).italic, Some(true));
        assert_eq!(common_format(&t, 0, 8).bold, None, "mixed selection");
        apply_format(&mut t, 0, 8, &SpanFormat { bold: Some(false), italic: Some(false), ..Default::default() });
        // bold=false is still stored (explicitly not bold), but uniformly.
        assert_eq!(t.spans.len(), 1);
    }

    #[test]
    fn unicode_ranges_are_characters() {
        let mut t = TextData { text: "héllo wörld 🎉".into(), ..Default::default() };
        apply_format(&mut t, 6, 11, &bold());
        let s = &t.spans[0];
        let b = &t.text[byte_at(&t.text, s.start)..byte_at(&t.text, s.end)];
        assert_eq!(b, "wörld");
    }

    #[test]
    fn new_paragraphs_inherit_list_and_indent() {
        let mut t = TextData { text: "one\ntwo".into(), ..Default::default() };
        format_paragraphs(&mut t, 1, 1, Some(ListKind::Bullet), None, 1, None);
        assert_eq!(para(&t, 1), (ListKind::Bullet, 1, Align::Left));
        edit_text(&mut t, "one\ntwo\nthree");
        assert_eq!(para(&t, 2), (ListKind::Bullet, 1, Align::Left), "Return in a list continues the list");
        edit_text(&mut t, "one\nthree");
        assert_eq!(t.text.split('\n').count(), 2);
        assert!(t.paras.len() <= 2);
    }

    #[test]
    fn markdown_round_trip() {
        let md = "Why **zebras** have *stripes*\n\n- flies get [confused](https://example.com)\n  - really\n1. first\n2. second";
        let t = from_markdown(md);
        assert!(t.text.starts_with("Why zebras have stripes\nflies get confused"), "{:?}", t.text);
        assert_eq!(t.spans.iter().filter(|s| s.format.bold == Some(true)).count(), 1);
        assert!(t.spans.iter().any(|s| s.format.link.as_deref() == Some("https://example.com")));
        assert_eq!(para(&t, 1).0, ListKind::Bullet);
        assert_eq!(para(&t, 2), (ListKind::Bullet, 1, Align::Left));
        assert_eq!(para(&t, 3).0, ListKind::Number);
        let back = to_markdown(&t);
        assert!(back.contains("**zebras**") && back.contains("[confused](https://example.com)") && back.contains("  - really") && back.contains("2. second"), "{back}");
        let again = from_markdown(&back);
        assert_eq!(again.text, t.text);
        assert_eq!(again.spans, t.spans);
    }
}
