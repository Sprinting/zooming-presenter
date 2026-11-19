//! Font fallback per cluster: a character the run's font (Inter) lacks must
//! come from a font that has it, whatever its script. Spec 02: "Hello।"
//! drew U+0964 as tofu because the danda is script-neutral (Common), so it
//! took the Latin run's script and only Latin fallback fonts were tried.

use zebra_core::{TextData, Theme};
use zebra_render::text::Fonts;

const INTER: &[u8] = include_bytes!("../../../assets/fonts/inter/Inter-Regular.ttf");

/// Characters in `s` that some installed or bundled font can show.
fn shown(fonts: &Fonts, s: &str) -> bool {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .all(|c| fonts.has_glyph_somewhere(c))
}

/// The source text of every .notdef glyph (id 0) in the layout of `s` set in Inter.
fn tofu(fonts: &Fonts, s: &str) -> Vec<String> {
    tofu_in(
        fonts,
        &TextData {
            text: s.into(),
            ..Default::default()
        },
    )
}

fn tofu_in(fonts: &Fonts, t: &TextData) -> Vec<String> {
    let s = t.text.as_str();
    let theme = Theme::by_id("classic").unwrap();
    let l = fonts.layout(t, &theme, None);
    // The Latin letters stay in the theme font: only the missing characters fall back.
    if let Some(i) = s.find('H') {
        let run = l
            .runs
            .iter()
            .find(|r| r.glyphs.iter().any(|g| g.start as usize == i))
            .expect("run for H");
        assert!(
            t.bold || run.font.data.as_ref() == INTER,
            "{s:?}: the run with H is not Inter"
        );
    }
    let mut v = vec![];
    for r in &l.runs {
        for g in &r.glyphs {
            if g.id == 0 {
                v.push(l.text[g.start as usize..g.end as usize].to_string());
            }
        }
    }
    v
}

const CASES: &[&str] = &[
    "Hello\u{0964}", // Devanagari danda after Latin
    "Hello\u{0965}", // double danda
    "Hello \u{0964} world",
    "\u{0964}Hello",                                            // leading
    "\u{0928}\u{092E}\u{0938}\u{094D}\u{0924}\u{0947}\u{0964}", // नमस्ते। (was fine)
    "Hello\u{3002}",                                            // CJK full stop
    "Hello\u{300C}quote\u{300D}",                               // CJK corner brackets
    "Hello\u{060C} world",                                      // Arabic comma
    "Hello \u{1F600}",                                          // emoji
    "Hello \u{2764}\u{FE0F}",                                   // emoji presentation sequence
    "e\u{0301}a\u{0951}", // combining acute (Inter has it) and Devanagari stress sign on Latin
    "Hello\u{2E3A}",      // two-em dash (Common, no script extensions)
];

#[test]
fn characters_inter_lacks_fall_back_whatever_their_script() {
    let fonts = Fonts::new();
    let mut failures = vec![];
    let mut skipped = vec![];
    for s in CASES {
        if !shown(&fonts, s) {
            skipped.push(*s);
            continue;
        }
        let bad = tofu(&fonts, s);
        if !bad.is_empty() {
            failures.push(format!("{s:?}: tofu for {bad:?}"));
        }
    }
    if !skipped.is_empty() {
        eprintln!("skipped (no font on this machine has every character): {skipped:?}");
    }
    assert!(failures.is_empty(), "{failures:#?}");
    // The case the spec is about must actually have been checked.
    assert!(
        !skipped.contains(&CASES[0]),
        "no Devanagari font on this machine; install one (e.g. FreeSans, Noto Sans Devanagari) to run this test"
    );
}

/// Bold, wrapped, a list paragraph: the fallback keeps the text's other styling.
#[test]
fn fallback_works_in_bold_wrapped_and_list_text() {
    let fonts = Fonts::new();
    if !fonts.has_glyph_somewhere('\u{0964}') {
        eprintln!("skipped: no Devanagari font on this machine");
        return;
    }
    let theme = Theme::by_id("classic").unwrap();
    let bold = TextData {
        text: "Hello\u{0964}".into(),
        bold: true,
        ..Default::default()
    };
    assert_eq!(tofu_in(&fonts, &bold), Vec::<String>::new());
    let long = TextData {
        text: "Hello\u{0964} ".repeat(20),
        bullets: true,
        fixed_width: true,
        ..Default::default()
    };
    let l = fonts.layout(&long, &theme, Some(200.0));
    assert!(
        l.runs.iter().all(|r| r.glyphs.iter().all(|g| g.id != 0)),
        "tofu in wrapped list text"
    );
    assert!(l.height > 24.0 * 2.0, "it wrapped");
}
