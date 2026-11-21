//! PDF export carries only fonts it may: Zebra's own and other open
//! fonts, cut to the glyphs used. Text in Apple's fonts, or in a font that
//! doesn't say it is open, is drawn as a picture where it stands (the
//! "fonts in export" rule; the web export does the same).
use zebra_core::*;
use zebra_render::font_rights::{pdf_use, Reason, WebUse};
use zebra_render::Renderer;

mod common;
use common::*;

/// Replace every `from` (as ASCII and as UTF-16BE) with `to`, same length.
fn swap(f: &mut [u8], from: &str, to: &str) {
    assert_eq!(from.len(), to.len());
    let utf16 = |s: &str| s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect::<Vec<u8>>();
    for (a, b) in [(from.as_bytes().to_vec(), to.as_bytes().to_vec()), (utf16(from), utf16(to))] {
        let mut i = 0;
        while i + a.len() <= f.len() {
            if f[i..i + a.len()] == a[..] {
                f[i..i + a.len()].copy_from_slice(&b);
                i += a.len();
            } else {
                i += 1;
            }
        }
    }
}

/// Caveat (Zebra's own, open) renamed "Qzxyws" with its licence wording
/// taken out: a font that doesn't say it is open.
fn closed_font() -> Vec<u8> {
    let mut f = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/caveat/Caveat-Regular.ttf")).unwrap();
    swap(&mut f, "Caveat", "Qzxyws");
    for l in ["Open Font License", "OFL", "openfontlicense", "scripts.sil.org", "Licensed under", "SIL"] {
        swap(&mut f, l, &"x".repeat(l.len()));
    }
    f
}

fn deck(r: &Renderer, font: &str) -> Editor {
    let mut ed = sample(r, "classic");
    let mut th = Theme::resolve("classic");
    th.id = "f".into();
    th.title.font = font.into();
    ed.apply(Command::SaveTheme { theme: th, apply: true, to_library: false }, Origin::Human).unwrap();
    ed
}

#[test]
fn a_font_that_isnt_open_is_drawn_not_embedded() {
    let closed = closed_font();
    assert_eq!(pdf_use(&closed, 0), WebUse::Rasterise(Reason::NotOpen));
    let r = Renderer::new();
    r.add_font(closed);
    // The same deck in the open font: its title's font is in the file.
    let open = String::from_utf8_lossy(&r.export_pdf(&deck(&r, "Caveat")).unwrap()).into_owned();
    assert!(open.contains("Caveat"), "Caveat should be embedded");
    let open_images = open.matches("/Subtype/Image").count();
    let pdf = r.export_pdf(&deck(&r, "Qzxyws")).unwrap();
    std::fs::write(out_dir().join("closed-font.pdf"), &pdf).unwrap();
    let s = String::from_utf8_lossy(&pdf);
    assert!(!s.contains("Qzxyws"), "a font that isn't open went into the PDF");
    assert!(s.matches("/Subtype/Image").count() > open_images, "its text should be pictures");
}

#[test]
fn preview_and_print_fonts_may_go_in_a_pdf_not_a_web_page() {
    let mut f = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/caveat/Caveat-Regular.ttf")).unwrap();
    // OS/2 fsType = 4 (preview & print).
    let n = u16::from_be_bytes([f[4], f[5]]) as usize;
    let os2 = (0..n).map(|i| 12 + 16 * i).find(|&r| &f[r..r + 4] == b"OS/2").unwrap();
    let off = u32::from_be_bytes([f[os2 + 8], f[os2 + 9], f[os2 + 10], f[os2 + 11]]) as usize;
    f[off + 8..off + 10].copy_from_slice(&4u16.to_be_bytes());
    assert_eq!(pdf_use(&f, 0), WebUse::Ship);
    assert_eq!(zebra_render::font_rights::web_use(&f, 0), WebUse::Rasterise(Reason::NoWebEmbedding));
}
