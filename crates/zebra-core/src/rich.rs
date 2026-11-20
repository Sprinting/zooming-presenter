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

/// Most characters one paste or insert puts into a text box. More is
/// refused whole (a note says it is too long for one box), not cut: a
/// megabyte of text in one box laid out on every frame froze the window.
pub const MAX_INSERT_CHARS: usize = 100_000;

/// Whether `text` is more than one paste may put into a box.
pub fn too_long_to_insert(text: &str) -> bool {
    // Counting stops past the limit: a 1 GB paste isn't walked twice.
    text.chars().nth(MAX_INSERT_CHARS).is_some()
}

/// The error for an agent's command that would put more than
/// [`MAX_INSERT_CHARS`] into one text box at once: `add_text`, an `update`
/// whose text grows by more, or `add_element` with such a text. None for
/// everything else. `doc` is where an update's box is now.
pub fn insert_error(cmd: &crate::command::Command, doc: &crate::model::Document) -> Option<crate::error::ZError> {
    use crate::command::Command;
    use crate::model::ElementKind;
    let (text, before) = match cmd {
        Command::AddText { text, .. } => (text.as_str(), 0),
        Command::AddElement { element } => match &element.kind {
            ElementKind::Text(t) => (t.text.as_str(), 0),
            _ => return None,
        },
        Command::Update { id, patch } => {
            let text = patch.text.as_deref()?;
            let before = match doc.get(id).map(|e| &e.kind) {
                Some(ElementKind::Text(t)) => char_len(&t.text),
                _ => 0,
            };
            (text, before)
        }
        _ => return None,
    };
    // Counted only as far as needed.
    let n = text.chars().take(before + MAX_INSERT_CHARS + 1).count();
    (n > before + MAX_INSERT_CHARS).then(|| {
        crate::error::ZError::invalid(format!("that adds more than {MAX_INSERT_CHARS} characters to one text box at once"))
            .hint("split the text into several boxes, or add it in parts")
    })
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
    // One sorted sweep over the span edges. Overlaying the covering spans in
    // order is, field by field, "the last span that sets it wins", so each
    // field keeps the indices of the covering spans that set it and reads
    // the largest. O(spans log spans); a nested loop was O(spans²), which a
    // pptx with 100k runs turned into minutes.
    let live: Vec<(usize, usize, usize)> =
        t.spans.iter().enumerate().map(|(i, s)| (s.start.min(n), s.end.min(n), i)).filter(|(a, b, _)| a < b).collect();
    let mut cuts: Vec<usize> = vec![0, n];
    for &(a, b, _) in &live {
        cuts.push(a);
        cuts.push(b);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut starts: Vec<(usize, usize)> = live.iter().map(|&(a, _, i)| (a, i)).collect();
    let mut ends: Vec<(usize, usize)> = live.iter().map(|&(_, b, i)| (b, i)).collect();
    starts.sort_unstable();
    ends.sort_unstable();
    let sets = |f: &SpanFormat| -> [bool; 7] {
        [
            f.bold.is_some(),
            f.italic.is_some(),
            f.underline.is_some(),
            f.strike.is_some(),
            f.color.is_some(),
            f.link.is_some(),
            f.size.is_some(),
        ]
    };
    let mut setters: [std::collections::BTreeSet<usize>; 7] = Default::default();
    let (mut si, mut ei) = (0, 0);
    let mut out: Vec<TextSpan> = vec![];
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        while ei < ends.len() && ends[ei].0 <= a {
            let i = ends[ei].1;
            for (k, on) in sets(&t.spans[i].format).into_iter().enumerate() {
                if on {
                    setters[k].remove(&i);
                }
            }
            ei += 1;
        }
        while si < starts.len() && starts[si].0 <= a {
            let i = starts[si].1;
            for (k, on) in sets(&t.spans[i].format).into_iter().enumerate() {
                if on {
                    setters[k].insert(i);
                }
            }
            si += 1;
        }
        let top = |k: usize| setters[k].last().map(|&i| &t.spans[i].format);
        // The same clean-up `overlay` does: empty strings clear, size 1 is
        // plain.
        let f = overlay(&SpanFormat::default(), &SpanFormat {
            bold: top(0).and_then(|f| f.bold),
            italic: top(1).and_then(|f| f.italic),
            underline: top(2).and_then(|f| f.underline),
            strike: top(3).and_then(|f| f.strike),
            color: top(4).and_then(|f| f.color.clone()),
            link: top(5).and_then(|f| f.link.clone()),
            size: top(6).and_then(|f| f.size),
        });
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

/// The bold setting of character `i`: its span's, else the box's (None
/// when neither sets it, so the style's weight applies).
pub fn bold_at(t: &TextData, i: usize) -> Option<bool> {
    t.spans.iter().find(|s| s.start <= i && i < s.end).and_then(|s| s.format.bold).or(t.bold.then_some(true))
}

/// The weight character `i` is drawn at in a style of weight `style`.
pub fn weight_at(t: &TextData, style: u16, i: usize) -> u16 {
    crate::theme::bold_weight(style, bold_at(t, i))
}

/// Whether every character in `start..end` is Bold or heavier (the B
/// button's state). An empty range asks about the whole text.
pub fn all_bold(t: &TextData, style: u16, start: usize, end: usize) -> bool {
    let n = char_len(&t.text);
    let (a, b) = if start < end { (start.min(n), end.min(n)) } else { (0, n) };
    if a >= b {
        return crate::theme::bold_weight(style, t.bold.then_some(true)) >= crate::theme::BOLD_FROM;
    }
    (a..b).all(|i| weight_at(t, style, i) >= crate::theme::BOLD_FROM)
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
    (list, p.indent.min(8), p.align.unwrap_or(t.align))
}

// ----------------------------------------------------------------------
// Moving words: drag text out of a box, or to another place in it.

/// `start..end` (characters) widened to whole graphemes, so "e" plus a
/// combining accent, a flag or a ZWJ family emoji is never cut in two.
/// A backwards range comes back as it was.
pub fn whole_graphemes(text: &str, start: usize, end: usize) -> (usize, usize) {
    use unicode_segmentation::UnicodeSegmentation;
    if start >= end {
        return (start, end);
    }
    let (mut a, mut b) = (start, end);
    let mut at = 0;
    for g in text.graphemes(true) {
        let next = at + g.chars().count();
        if at < start && start < next {
            a = at;
        }
        if at < end && end < next {
            b = next;
            break;
        }
        if next >= end {
            break;
        }
        at = next;
    }
    (a, b)
}

/// The characters `start..end` of `t` as a text of their own, the way a
/// dragged selection becomes a new box: the box's own settings (style,
/// colour, alignment, bold, italic, bullets, width mode), the characters'
/// formatting, and the paragraph formatting (list, indent, alignment) of
/// each paragraph wholly inside the range. A part of a paragraph comes out
/// as a plain paragraph. Spaces and line breaks at either end are dropped.
pub fn slice(t: &TextData, start: usize, end: usize) -> TextData {
    let (start, end) = whole_graphemes(&t.text, start, end);
    let chars: Vec<char> = t.text.chars().collect();
    let a = start.min(chars.len());
    // A backwards range is empty, not a panic.
    let (mut a, mut b) = (a, end.min(chars.len()).max(a));
    while a < b && chars[a].is_whitespace() {
        a += 1;
    }
    while b > a && chars[b - 1].is_whitespace() {
        b -= 1;
    }
    let mut out = TextData { text: chars[a..b].iter().collect(), spans: vec![], paras: vec![], ..t.clone() };
    out.spans = t
        .spans
        .iter()
        .filter(|s| s.start < b && s.end > a)
        .map(|s| TextSpan { start: s.start.max(a) - a, end: s.end.min(b) - a, format: s.format.clone() })
        .collect();
    // Paragraph by paragraph: whole ones keep their format.
    let first = chars[..a].iter().filter(|c| **c == '\n').count();
    let mut ps = chars[..a].iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
    for k in 0..=out.text.matches('\n').count() {
        let pe = chars[ps..].iter().position(|c| *c == '\n').map_or(chars.len(), |i| ps + i);
        let (mut cs, mut ce) = (ps, pe);
        while cs < ce && chars[cs].is_whitespace() {
            cs += 1;
        }
        while ce > cs && chars[ce - 1].is_whitespace() {
            ce -= 1;
        }
        let whole = if cs < ce { a <= cs && ce <= b } else { a <= ps && pe <= b };
        let p = if whole {
            t.paras.get(first + k).cloned().unwrap_or_default()
        } else {
            // Plain: no marker even in a box whose every paragraph has one.
            ParaFormat { list: t.bullets.then_some(ListKind::None), ..Default::default() }
        };
        out.paras.push(p);
        ps = pe + 1;
    }
    normalize(&mut out);
    out
}

/// Take characters `a..b` out of `t`, formatting moving with what's left.
/// A paragraph cut from its start keeps the format of the one it joins.
fn remove(t: &mut TextData, a: usize, b: usize) {
    let chars: Vec<char> = t.text.chars().collect();
    let (a, b) = (a.min(chars.len()), b.min(chars.len()));
    if a >= b {
        return;
    }
    let gone = b - a;
    for s in &mut t.spans {
        let map = |i: usize| if i <= a { i } else if i >= b { i - gone } else { a };
        s.start = map(s.start);
        s.end = map(s.end);
    }
    let para_at = chars[..a].iter().filter(|c| **c == '\n').count();
    let nl = chars[a..b].iter().filter(|c| **c == '\n').count();
    if nl > 0 && !t.paras.is_empty() {
        let at_start = a == 0 || chars[a - 1] == '\n';
        let from = if at_start { para_at } else { para_at + 1 };
        let (from, to) = (from.min(t.paras.len()), (from + nl).min(t.paras.len()));
        t.paras.drain(from..to);
    }
    t.text = chars[..a].iter().chain(&chars[b..]).collect();
    normalize(t);
}

/// Put `piece` (its characters, their formatting and its paragraphs) into
/// `t` at `at`. Inserted words never take on the formatting around them.
fn insert(t: &mut TextData, at: usize, piece: &TextData) {
    let chars: Vec<char> = t.text.chars().collect();
    let at = at.min(chars.len());
    let n = char_len(&piece.text);
    if n == 0 {
        return;
    }
    let mut spans = vec![];
    for s in t.spans.drain(..) {
        if s.end <= at {
            spans.push(s);
        } else if s.start >= at {
            spans.push(TextSpan { start: s.start + n, end: s.end + n, format: s.format });
        } else {
            spans.push(TextSpan { start: s.start, end: at, format: s.format.clone() });
            spans.push(TextSpan { start: at + n, end: s.end + n, format: s.format });
        }
    }
    spans.extend(piece.spans.iter().map(|s| TextSpan { start: s.start + at, end: s.end + at, format: s.format.clone() }));
    t.spans = spans;
    let nl = piece.text.matches('\n').count();
    if nl > 0 {
        let para_at = chars[..at].iter().filter(|c| **c == '\n').count();
        let count = t.text.split('\n').count();
        if t.paras.len() < count {
            t.paras.resize(count, ParaFormat::default());
        }
        // The paragraph split by the insert: its first part keeps its
        // format unless the piece starts it, its last part likewise unless
        // the piece ends it.
        let here = t.paras[para_at].clone();
        let at_start = at == 0 || chars[at - 1] == '\n';
        let at_end = at == chars.len() || chars[at] == '\n';
        let of_piece = |k: usize| piece.paras.get(k).cloned().unwrap_or_default();
        let mut new = vec![if at_start { of_piece(0) } else { here.clone() }];
        new.extend((1..nl).map(of_piece));
        new.push(if at_end { of_piece(nl) } else { here });
        t.paras.splice(para_at..=para_at, new);
    }
    t.text = chars[..at].iter().copied().chain(piece.text.chars()).chain(chars[at..].iter().copied()).collect();
    normalize(t);
}

/// Where position `p` ends up after the removals `cuts` (each `a..b`, in
/// the text as it was when it was made, applied in order).
fn after_cuts(mut p: usize, cuts: &[(usize, usize)]) -> usize {
    for &(a, b) in cuts {
        if p >= b {
            p -= b - a;
        } else if p > a {
            p = a;
        }
    }
    p
}

/// Cut characters `start..end` out of `t` and tidy the gap as the Mac's
/// smart cut does: no double space left, no space at the start or end of a
/// paragraph or before punctuation, and no empty paragraph where a whole
/// one was. Returns each removal made, in order (for [`after_cuts`]).
pub fn cut(t: &mut TextData, start: usize, end: usize) -> Vec<(usize, usize)> {
    let n = char_len(&t.text);
    let (a, b) = (start.min(n), end.min(n));
    if a >= b {
        return vec![];
    }
    let had_words = t.text.chars().skip(a).take(b - a).any(|c| !c.is_whitespace());
    let mut cuts = vec![(a, b)];
    remove(t, a, b);
    let mut take = |t: &mut TextData, x: usize, y: usize| {
        remove(t, x, y);
        cuts.push((x, y));
    };
    let chars = |t: &TextData| t.text.chars().collect::<Vec<char>>();
    let mut i = a;
    // A whole paragraph went: its line goes too.
    let c = chars(t);
    let para_start = i == 0 || c[i - 1] == '\n';
    let para_end = i == c.len() || c[i] == '\n';
    if had_words && para_start && para_end && !c.is_empty() {
        if i < c.len() {
            take(t, i, i + 1);
        } else {
            take(t, i - 1, i);
            i -= 1;
        }
    }
    let c = chars(t);
    let space = |k: usize| c.get(k) == Some(&' ');
    let mut s = i;
    while s > 0 && space(s - 1) {
        s -= 1;
    }
    let mut e = i;
    while space(e) {
        e += 1;
    }
    let s_start = s == 0 || c[s - 1] == '\n';
    let e_end = e == c.len() || c[e] == '\n';
    let punct = c.get(e).is_some_and(|ch| matches!(ch, '.' | ',' | ';' | ':' | '!' | '?' | ')' | '’' | '”'));
    let (x, y) = if s_start || e_end || punct {
        // Nothing before or after the gap but its spaces: they all go.
        (s, e)
    } else if e - s > 1 {
        // Words on both sides: one space between them.
        (s + 1, e)
    } else {
        (i, i)
    };
    if x < y {
        take(t, x, y);
    }
    cuts
}

/// Move (or with `copy`, copy) characters `start..end` of `t` to `to`, a
/// position in the text as it is now, as dragging a selection inside a
/// text box does. The words keep their formatting; spaces are tidied where
/// they left and added where they land when a word would otherwise run
/// into its neighbour. Returns the words' new range, or `None` when `to`
/// is inside the range itself (nothing to do) or nothing but spaces is
/// selected.
pub fn move_within(t: &mut TextData, start: usize, end: usize, to: usize, copy: bool) -> Option<(usize, usize)> {
    let n = char_len(&t.text);
    let (start, end, to) = (start.min(n), end.min(n), to.min(n));
    // Whole graphemes, as `slice` takes them; a drop point inside one goes
    // to its start.
    let (start, end) = whole_graphemes(&t.text, start, end);
    let to = if to < n { whole_graphemes(&t.text, to, to + 1).0 } else { to };
    if start >= end || (start..=end).contains(&to) {
        return None;
    }
    let piece = slice(t, start, end);
    if piece.text.is_empty() {
        return None;
    }
    // Whole paragraphs dropped at a paragraph's start or end move as
    // paragraphs; anything else lands among the words there.
    let c: Vec<char> = t.text.chars().collect();
    let (mut a, mut b) = (start, end);
    while a < b && c[a].is_whitespace() {
        a += 1;
    }
    while b > a && c[b - 1].is_whitespace() {
        b -= 1;
    }
    let whole = (a == 0 || c[a - 1] == '\n') && (b == c.len() || c[b] == '\n');
    let to = if copy { to } else { after_cuts(to, &cut(t, start, end)) };
    let c: Vec<char> = t.text.chars().collect();
    let to = to.min(c.len());
    let at_start = to == 0 || c[to - 1] == '\n';
    let at_end = to == c.len() || c[to] == '\n';
    if whole && (at_start || at_end) && !c.is_empty() {
        let mut p = piece.clone();
        let blank = TextData { text: "\n".into(), ..Default::default() };
        let at = if at_start {
            p.text.push('\n');
            p.paras.resize(p.text.split('\n').count(), ParaFormat::default());
            to
        } else {
            insert(t, to, &blank);
            to + 1
        };
        if at_start {
            insert(t, at, &p);
        } else {
            // The new empty paragraph after `to` takes the piece's format.
            let at_para = t.text.chars().take(at).filter(|c| *c == '\n').count();
            if t.paras.len() <= at_para {
                t.paras.resize(at_para + 1, ParaFormat::default());
            }
            t.paras[at_para] = piece.paras.first().cloned().unwrap_or_default();
            insert(t, at, &p);
        }
        return Some((at, at + char_len(&piece.text)));
    }
    let word = |ch: Option<&char>| ch.is_some_and(|ch| !ch.is_whitespace());
    let pad_before = to > 0 && word(c.get(to - 1)) && piece.text.chars().next().is_some_and(|ch| ch.is_alphanumeric());
    let pad_after = word(c.get(to)) && c.get(to).is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '(' | '“' | '‘'));
    let mut at = to;
    let space = TextData { text: " ".into(), ..Default::default() };
    if pad_after {
        insert(t, at, &space);
    }
    if pad_before {
        insert(t, at, &space);
        at += 1;
    }
    insert(t, at, &piece);
    Some((at, at + char_len(&piece.text)))
}

/// Character offset → byte offset in `s`.
pub fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map(|(b, _)| b).unwrap_or(s.len())
}

/// The text as Markdown: bold, italic, strike, links, bullets, numbers
/// and indents.
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
            let indent = indent.min(8);
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
        // A span that is empty or runs past the text (a file from elsewhere)
        // is skipped, not followed.
        if let Some(s) = t.spans.iter().find(|s| s.start == i && s.end > i && s.end <= chars.len()) {
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
    fn to_markdown_writes_spans_and_lists() {
        let mut t = TextData { text: "Why zebras have stripes\nflies get confused\nreally maybe\nfirst\nsecond".into(), ..Default::default() };
        apply_format(&mut t, 4, 10, &SpanFormat { bold: Some(true), ..Default::default() });
        apply_format(&mut t, 16, 23, &SpanFormat { italic: Some(true), ..Default::default() });
        apply_format(&mut t, 34, 42, &SpanFormat { link: Some("https://example.com".into()), ..Default::default() });
        apply_format(&mut t, 50, 55, &SpanFormat { strike: Some(true), ..Default::default() });
        format_paragraphs(&mut t, 1, 2, Some(ListKind::Bullet), None, 0, None);
        format_paragraphs(&mut t, 2, 2, None, Some(1), 0, None);
        format_paragraphs(&mut t, 3, 4, Some(ListKind::Number), None, 0, None);
        let md = to_markdown(&t);
        for part in ["Why **zebras** have *stripes*", "- flies get [confused](https://example.com)", "  - really ~~maybe~~", "1. first", "2. second"] {
            assert!(md.contains(part), "{part} missing from {md}");
        }
    }

    /// W8 of the 20 Nov audit: `slice` is public, so a backwards range is
    /// guarded here and not only by its callers.
    #[test]
    fn a_backwards_slice_is_empty() {
        let t = TextData { text: "one\ntwo".into(), ..Default::default() };
        assert_eq!(slice(&t, 5, 2).text, "");
        assert_eq!(slice(&t, 99, 0).text, "");
        assert_eq!(slice(&t, 0, 3).text, "one");
    }
}

#[cfg(test)]
mod normalize_tests {
    use super::*;

    /// The old nested loop, kept as the reference.
    fn slow(t: &TextData) -> Vec<TextSpan> {
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
        out
    }

    #[test]
    fn the_sweep_matches_the_nested_loop() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % m
        };
        for _ in 0..2000 {
            let mut t = TextData { text: "x".repeat(rnd(30) as usize), ..Default::default() };
            for _ in 0..rnd(12) {
                let pick = |r: u64| match r {
                    0 => None,
                    1 => Some(false),
                    _ => Some(true),
                };
                let format = SpanFormat {
                    bold: pick(rnd(3)),
                    italic: pick(rnd(3)),
                    underline: pick(rnd(4)),
                    strike: pick(rnd(5)),
                    color: [None, Some(""), Some("#ff0000"), Some("accent")][rnd(4) as usize].map(String::from),
                    link: [None, Some(""), Some("https://a.example")][rnd(3) as usize].map(String::from),
                    size: [None, Some(1.0), Some(2.0), Some(f64::NAN), Some(-1.0)][rnd(5) as usize],
                };
                let (a, b) = (rnd(40) as usize, rnd(40) as usize);
                t.spans.push(TextSpan { start: a, end: b, format });
            }
            let want = slow(&t);
            normalize(&mut t);
            assert_eq!(t.spans, want);
        }
    }

    /// H1 of the 20 Nov audit: 100k nested runs took minutes.
    #[test]
    fn a_hundred_thousand_runs_normalize_quickly() {
        let n = 100_000;
        let mut t = TextData { text: "x".repeat(2 * n), ..Default::default() };
        for i in 0..n {
            let format = SpanFormat { bold: Some(i % 2 == 0), size: Some(1.0 + (i % 3) as f64), ..Default::default() };
            t.spans.push(TextSpan { start: i, end: 2 * n - i, format });
        }
        let at = std::time::Instant::now();
        normalize(&mut t);
        assert!(at.elapsed().as_secs_f32() < 5.0, "took {:?}", at.elapsed());
        assert_eq!(t.spans.first().map(|s| s.start), Some(0));
    }
}

#[cfg(test)]
mod bold_tests {
    use super::*;

    #[test]
    fn b_button_state_follows_the_style_weight() {
        let mut t = TextData { text: "Big title".into(), ..Default::default() };
        assert!(!all_bold(&t, 400, 0, 3));
        assert!(all_bold(&t, 600, 0, 3), "a Semibold style is already bold");
        apply_format(&mut t, 0, 3, &SpanFormat { bold: Some(false), ..Default::default() });
        assert!(!all_bold(&t, 800, 0, 3), "bold: false in a heavy style is Regular");
        assert_eq!(weight_at(&t, 800, 0), 400);
        assert_eq!(weight_at(&t, 800, 5), 800);
        assert!(!all_bold(&t, 800, 0, 9), "mixed");
        assert_eq!(t.spans[0].format.bold, Some(false), "kept, not normalised away");
    }
}
