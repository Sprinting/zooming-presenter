//! B14: a text's formatted spans were counted in a u16, so formatting past
//! the 65,534th span was dropped. Now every span keeps its formatting (a
//! paragraph still holds what the layout engine takes: 60,000 style
//! changes).
use zebra_core::{SpanFormat, TextData, TextSpan, Theme};
use zebra_render::text::Fonts;

/// `paras` paragraphs of `per` one-letter spans, alternately red and blue.
fn text(paras: usize, per: usize) -> TextData {
    let para = "a".repeat(per);
    let text = vec![para.as_str(); paras].join("\n");
    let mut spans = Vec::with_capacity(per * paras);
    for p in 0..paras {
        let base = p * (per + 1);
        for i in 0..per {
            spans.push(TextSpan { start: base + i, end: base + i + 1, format: SpanFormat { color: Some(if i % 2 == 0 { "#ff0000" } else { "#0000ff" }.into()), ..Default::default() } });
        }
    }
    TextData { text, spans, ..Default::default() }
}

/// Slow in a debug build (about 15 minutes): `cargo test --release -p
/// zebra-render --test many_spans -- --ignored`.
#[test]
#[ignore]
fn spans_past_65534_keep_their_formatting() {
    let t = text(3, 23_000);
    let l = Fonts::new().layout(&t, &Theme::by_id("classic").unwrap(), None);
    let top = l.runs.iter().filter_map(|r| r.format).max().unwrap();
    assert_eq!(top, 68_999, "the last span keeps its formatting");
}

/// A colour-only span costs one style change: a paragraph of 30,000 of
/// them keeps them all (spans were counted as 4 changes each, so 15,000).
#[test]
fn a_paragraph_keeps_more_spans_than_before() {
    let t = text(1, 16_000);
    let l = Fonts::new().layout(&t, &Theme::by_id("classic").unwrap(), None);
    let top = l.runs.iter().filter_map(|r| r.format).max().unwrap();
    assert_eq!(top, 15_999);
}
