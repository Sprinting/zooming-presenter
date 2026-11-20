//! A weight per theme text style (spec 18a): it reaches the layout as the
//! variable fonts' weight axis or the nearest static face, never a faked
//! bold, and the PDF and web exports carry the weights used.
use zebra_core::*;
use zebra_render::text::Fonts;
use zebra_render::Renderer;

mod common;
use common::*;

fn theme_with(title_font: &str, weight: u16) -> Theme {
    let mut t = Theme::resolve("classic");
    t.id = "w".into();
    t.title.font = title_font.into();
    t.title.weight = weight;
    t
}

fn wght(l: &zebra_render::text::TextLayout, run: usize) -> Option<f32> {
    l.runs[run].variations.iter().find(|(t, _)| t == b"wght").map(|(_, v)| *v)
}

#[test]
fn bundled_fonts_list_their_weights() {
    let f = Fonts::new();
    let doc = Document::default();
    let w = |fam: &str| f.faces(Some(&doc), fam).iter().map(|x| x.weight).collect::<Vec<_>>();
    assert_eq!(w("Sans"), vec![100, 200, 300, 400, 500, 600, 700, 800, 900], "Inter is variable");
    assert_eq!(w("Inter"), w("Sans"));
    assert_eq!(w("Serif"), vec![400, 500, 600, 700], "Lora");
    assert_eq!(w("Mono"), vec![100, 200, 300, 400, 500, 600, 700, 800], "JetBrains Mono");
    assert_eq!(w("Hand"), vec![400, 700], "Caveat stays Regular and Bold");
    assert!(f.faces(Some(&doc), "Sans").iter().all(|x| x.name.is_none()), "the app names bundled weights");
    assert!(f.faces(Some(&doc), "No Such Font 123").is_empty());
}

#[test]
fn style_weight_reaches_the_layout() {
    let f = Fonts::new();
    let t = TextData { text: "Zebra".into(), style: TextStyle::Title, ..Default::default() };
    let regular = f.layout(&t, &theme_with("Sans", 400), None);
    let semi = f.layout(&t, &theme_with("Sans", 600), None);
    let heavy = f.layout(&t, &theme_with("Sans", 800), None);
    assert_eq!(wght(&regular, 0), None, "Regular is the default instance");
    assert_eq!(wght(&semi, 0), Some(600.0));
    assert_eq!(wght(&heavy, 0), Some(800.0));
    assert!(!semi.runs[0].coords.is_empty(), "drawn at the weight axis location");
    assert!(regular.width < semi.width && semi.width < heavy.width, "{} {} {}", regular.width, semi.width, heavy.width);
    // Body text in the same theme stays Regular.
    let body = f.layout(&TextData { text: "Zebra".into(), ..Default::default() }, &theme_with("Sans", 800), None);
    assert_eq!(wght(&body, 0), None);
}

#[test]
fn a_missing_weight_uses_the_nearest_face_not_a_fake_bold() {
    let f = Fonts::new();
    let t = TextData { text: "Zebra".into(), style: TextStyle::Title, ..Default::default() };
    // Caveat has Regular and Bold only.
    let bold = f.layout(&t, &theme_with("Hand", 700), None);
    let semi = f.layout(&t, &theme_with("Hand", 600), None);
    let black = f.layout(&t, &theme_with("Hand", 900), None);
    let light = f.layout(&t, &theme_with("Hand", 300), None);
    let regular = f.layout(&t, &theme_with("Hand", 400), None);
    let id = |l: &zebra_render::text::TextLayout| l.runs[0].font.data.id();
    assert_eq!(id(&semi), id(&bold), "Semibold falls to Bold, the nearest");
    assert_eq!(id(&black), id(&bold));
    assert_eq!(id(&light), id(&regular));
    assert_ne!(id(&bold), id(&regular));
    assert_eq!(black.width, bold.width, "no faked extra weight");
    // Lora's axis stops at 400: Light is Regular.
    let lora = f.layout(&t, &theme_with("Serif", 300), None);
    assert_eq!(lora.width, f.layout(&t, &theme_with("Serif", 400), None).width);
}

#[test]
fn bold_off_in_a_heavy_style_is_regular() {
    let f = Fonts::new();
    let mut t = TextData { text: "Big title".into(), style: TextStyle::Title, ..Default::default() };
    rich::apply_format(&mut t, 0, 3, &SpanFormat { bold: Some(false), ..Default::default() });
    let l = f.layout(&t, &theme_with("Sans", 800), None);
    let run_of = |i: u32| l.runs.iter().position(|r| r.glyphs.iter().any(|g| g.start == i)).unwrap();
    assert_eq!(wght(&l, run_of(0)), None, "\"Big\" is Regular");
    assert_eq!(wght(&l, run_of(4)), Some(800.0), "\"title\" keeps the style's Heavy");
    // Bold on in a Semibold style keeps Semibold; in a Light one it is Bold.
    let mut b = TextData { text: "Big".into(), style: TextStyle::Title, bold: true, ..Default::default() };
    assert_eq!(wght(&f.layout(&b, &theme_with("Sans", 600), None), 0), Some(600.0));
    b.bold = false;
    rich::apply_format(&mut b, 0, 3, &SpanFormat { bold: Some(true), ..Default::default() });
    assert_eq!(wght(&f.layout(&b, &theme_with("Sans", 300), None), 0), Some(700.0));
}

fn weighted_deck(r: &Renderer) -> Editor {
    let mut ed = sample(r, "classic");
    let mut th = theme_with("Sans", 600);
    th.subtitle.weight = 800;
    ed.apply(Command::SaveTheme { theme: th, apply: true, to_library: false }, Origin::Human).unwrap();
    ed
}

#[test]
fn pdf_carries_each_weight_as_its_own_cut_font() {
    let r = Renderer::new();
    let ed = weighted_deck(&r);
    let pdf = r.export_pdf(&ed).expect("pdf");
    std::fs::write(out_dir().join("weights.pdf"), &pdf).unwrap();
    let s = String::from_utf8_lossy(&pdf);
    let fonts: std::collections::BTreeSet<&str> = s
        .match_indices("/Type/Font/Subtype/Type0/BaseFont/")
        .map(|(i, _)| s[i + 34..].split(|c: char| c == '/' || c.is_whitespace()).next().unwrap_or(""))
        .collect();
    eprintln!("{} bytes, fonts {fonts:?}", pdf.len());
    // Title Semibold, subtitle Heavy (and its italic), body Regular: one
    // plain font each, cut from the variable file.
    assert!(fonts.len() >= 3, "{fonts:?}");
    assert!(pdf.len() < 1 << 20, "cut fonts, not whole variable files: {} bytes", pdf.len());
}

#[test]
fn web_export_ships_the_variable_font_cut_to_the_glyphs_used() {
    let r = Renderer::new();
    let ed = weighted_deck(&r);
    let web = r.web_text(&ed);
    let total: usize = web.fonts.iter().map(|f| f.len()).sum();
    eprintln!("{} files, {total} bytes", web.fonts.len());
    assert!(web.rasterise.is_empty());
    // Each file keeps its shaping tables (Inter's kerning alone is 150 KB)
    // but only the outlines, and outline variations, of the glyphs used.
    for f in &web.fonts {
        assert!(f.len() < 400 << 10, "outline variations cut too: {} bytes", f.len());
    }
    // The page lays the deck out again from those files: same widths.
    // ("Inter Variable" in the file is Inter there too.)
    let page = Fonts::bare();
    for f in &web.fonts {
        page.add_font(f.clone());
    }
    let t = TextData { text: "Zebra: zoom into ideas".into(), style: TextStyle::Title, ..Default::default() };
    let th = ed.theme();
    let a = r.measure().measure(&t, &th, None);
    let b = page.layout(&t, &th, None);
    assert!((a.0 - b.width).abs() < 0.01, "{a:?} vs {}", b.width);
    assert_eq!(wght(&b, 0), Some(600.0));
}
