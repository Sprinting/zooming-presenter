//! Exports carry fonts cut down to the glyphs used: one emoji must not ship
//! the whole system emoji font (about 180 MB, and not ours to give away).
use std::collections::BTreeSet;

use skrifa::MetadataProvider;
use zebra_core::*;
use zebra_render::Renderer;

mod common;
use common::*;

#[test]
fn bundled_text_ships_small_fonts() {
    let r = Renderer::new();
    let mut ed = sample(&r, "classic");
    add(&mut ed, "e", 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, text("Zebra party", TextStyle::Title));
    let web = r.web_text(&ed);
    let total: usize = web.fonts.iter().map(|f| f.len()).sum();
    eprintln!("{} font files, {} bytes", web.fonts.len(), total);
    assert!(web.rasterise.is_empty(), "bundled fonts may ship: {:?}", web.rasterise);
    // Cut Inter faces: a few hundred KB, not the whole family.
    assert!(total < 2 << 20, "fonts for a few lines of text: {total} bytes");
}

/// Apple's fonts never go into a page, not even cut down: text set in
/// them (here the emoji) becomes a picture drawn on this Mac.
#[cfg(target_os = "macos")]
#[test]
fn apple_font_text_becomes_a_picture() {
    let r = Renderer::new();
    let mut ed = sample(&r, "classic");
    add(&mut ed, "e", 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, text("Zebra 🦓 party 🎉", TextStyle::Title));
    let web = r.web_text(&ed);
    assert!(web.rasterise.contains(&"e".to_string()), "emoji text drawn as a picture: {:?}", web.rasterise);
    for f in &web.fonts {
        assert_eq!(zebra_render::font_rights::web_use(f, 0), zebra_render::font_rights::WebUse::Ship, "a shipped font is Apple's");
        assert!(f.len() < 2 << 20);
    }
    let pic = r.rasterise_text(&ed, "e", 1024).expect("picture");
    assert_eq!(pic.width, 1024);
    let img = image::load_from_memory(&pic.png).unwrap().to_rgba8();
    let inked = img.pixels().filter(|p| p[3] > 0).count();
    let clear = img.pixels().filter(|p| p[3] == 0).count();
    assert!(inked > 1000 && clear > inked, "text drawn on a transparent picture ({inked} inked, {clear} clear)");
    // Colour from the emoji bitmap made it in.
    assert!(img.pixels().any(|p| p[3] > 200 && (p[0] as i32 - p[2] as i32).abs() > 80), "emoji colours drawn");
}

#[cfg(target_os = "macos")]
#[test]
fn apple_emoji_cut_keeps_its_bitmaps() {
    let Ok(file) = std::fs::read("/System/Library/Fonts/Apple Color Emoji.ttc") else { return };
    let font = skrifa::FontRef::from_index(&file, 0).unwrap();
    let zebra = font.charmap().map('🦓').expect("zebra emoji");
    let keep: BTreeSet<u16> = [zebra.to_u32() as u16].into_iter().collect();
    let cut = zebra_render::font_subset::subset(&file, 0, &keep).expect("cut");
    eprintln!("{} -> {} bytes", file.len(), cut.len());
    assert!(cut.len() < 4 << 20);
    let small = skrifa::FontRef::from_index(&cut, 0).unwrap();
    assert_eq!(small.charmap().map('🦓'), Some(zebra));
    let strikes = skrifa::bitmap::BitmapStrikes::new(&small);
    assert!(!strikes.is_empty());
    assert!(strikes.glyph_for_size(skrifa::instance::Size::new(160.0), zebra).is_some(), "zebra bitmap kept");
    let other = small.charmap().map('🎉').unwrap();
    assert!(strikes.glyph_for_size(skrifa::instance::Size::new(160.0), other).is_none_or(|g| matches!(g.data, skrifa::bitmap::BitmapData::Png(d) if d.is_empty())), "unused bitmap dropped");
}

/// Outline of glyph `g` as a list of pen commands (to compare fonts).
fn outline(font: &skrifa::FontRef, g: skrifa::GlyphId) -> Option<String> {
    use skrifa::outline::{DrawSettings, OutlinePen};
    struct P(String);
    impl OutlinePen for P {
        fn move_to(&mut self, x: f32, y: f32) { self.0 += &format!("M{x},{y}"); }
        fn line_to(&mut self, x: f32, y: f32) { self.0 += &format!("L{x},{y}"); }
        fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) { self.0 += &format!("Q{a},{b},{x},{y}"); }
        fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) { self.0 += &format!("C{a},{b},{c},{d},{x},{y}"); }
        fn close(&mut self) { self.0 += "Z"; }
    }
    let og = font.outline_glyphs();
    let mut p = P(String::new());
    og.get(g)?.draw(DrawSettings::unhinted(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default()), &mut p).ok()?;
    Some(p.0)
}

/// CFF fonts (CJK and Indic system fonts on the Mac): kept glyphs draw
/// exactly as before, the others are empty, and the file is much smaller.
#[cfg(target_os = "macos")]
#[test]
fn cff_fonts_are_cut_too() {
    for (path, text) in [
        ("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", "こんにちは"),
        ("/System/Library/Fonts/Kohinoor.ttc", "नमस्ते"),
        ("/System/Library/Fonts/AppleSDGothicNeo.ttc", "안녕"),
    ] {
        let Ok(file) = std::fs::read(path) else { continue };
        let font = skrifa::FontRef::from_index(&file, 0).unwrap();
        let used: Vec<skrifa::GlyphId> = text.chars().filter_map(|c| font.charmap().map(c)).collect();
        let keep: BTreeSet<u16> = used.iter().map(|g| g.to_u32() as u16).collect();
        let cut = zebra_render::font_subset::subset(&file, 0, &keep).expect("cut");
        eprintln!("{path}: {} -> {} bytes", file.len(), cut.len());
        assert!(cut.len() * 3 < file.len(), "{path} barely shrank");
        let small = skrifa::FontRef::from_index(&cut, 0).unwrap();
        for g in &used {
            assert_eq!(small.charmap().map(text.chars().next().unwrap()).is_some(), true);
            assert_eq!(outline(&small, *g), outline(&font, *g), "{path}: glyph {g:?} changed");
            assert!(outline(&small, *g).is_some_and(|o| !o.is_empty()));
        }
        // A glyph the text doesn't use is now empty.
        let other = font.charmap().map('X').or_else(|| font.charmap().map('一')).unwrap();
        if !keep.contains(&(other.to_u32() as u16)) {
            assert!(outline(&small, other).is_none_or(|o| o.is_empty()), "{path}: unused glyph kept");
        }
    }
}
