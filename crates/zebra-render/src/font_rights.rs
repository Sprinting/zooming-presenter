//! Which fonts a web page may carry. A page plays on any device, so it may
//! only carry fonts licensed for that:
//!
//! * Never Apple's. The fonts that come with macOS may be used "while
//!   running the Apple Software", and SF Pro, SF Compact, SF Mono and New
//!   York only for interface mock-ups; neither covers a web player, not even
//!   a subset. Text set in them is drawn into pictures on the person's Mac
//!   instead (see `Renderer::rasterise_text`).
//! * Never a font whose embedding permissions (OS/2 `fsType`) are
//!   "restricted", "preview & print" (enough for a PDF, not the web) or
//!   "bitmap only".
//!
//! * Only fonts that say they are open. A font states its licence in its
//!   name table (licence description, id 13, and licence URL, id 14): the
//!   SIL Open Font License, Apache, MIT, the Ubuntu Font Licence, CC0 or
//!   public domain qualify. Anything else (a bought font, one that says
//!   nothing) is treated as not open: its text becomes pictures too. The
//!   embedding flags alone don't say a font is open (many commercial fonts
//!   are marked "installable").
//!
//! Fonts that qualify (Zebra's own, open fonts the person added) ship cut
//! to the glyphs used (see [`crate::font_subset`]).
use std::sync::OnceLock;

/// What a web page may do with a font.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebUse {
    /// Carry it, cut to the glyphs used.
    Ship,
    /// Don't carry it: draw its text as pictures. The reason, for notes.
    Rasterise(Reason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// Installed with macOS, or made by Apple.
    Apple,
    /// SF Pro, SF Compact, SF Mono, New York: licensed for UI mock-ups only.
    /// Holds the family name.
    AppleUiFont(String),
    /// Its embedding permissions don't allow a web page.
    NoWebEmbedding,
    /// It doesn't say it is under an open licence.
    NotOpen,
}

/// Licence wording (lower case) that marks a font as open.
const OPEN_LICENCES: &[&str] = &[
    "open font license",
    "scripts.sil.org/ofl",
    "openfontlicense.org",
    "apache license",
    "apache.org/licenses",
    "mit license",
    "opensource.org/licenses/mit",
    "ubuntu font licen",
    "creativecommons.org/publicdomain/zero",
    "cc0",
    "public domain",
];

/// True when the font's own licence fields name an open licence.
pub fn says_open(file: &[u8], index: u32) -> bool {
    names(file, index, &[13, 14]).iter().any(|s| {
        let s = s.to_lowercase();
        OPEN_LICENCES.iter().any(|l| s.contains(l))
    })
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// A table of face `index`.
fn table<'a>(file: &'a [u8], index: u32, tag: &[u8; 4]) -> Option<&'a [u8]> {
    let face = if file.get(0..4)? == b"ttcf" { u32_at(file, 12 + 4 * index as usize)? as usize } else { 0 };
    let n = u16_at(file, face + 4)? as usize;
    (0..n).find_map(|i| {
        let r = face + 12 + 16 * i;
        (file.get(r..r + 4)? == tag).then_some(())?;
        let (off, len) = (u32_at(file, r + 8)? as usize, u32_at(file, r + 12)? as usize);
        file.get(off..off.checked_add(len)?)
    })
}

/// Strings of the name table with these ids (UTF-16 or Mac Roman).
fn names(file: &[u8], index: u32, ids: &[u16]) -> Vec<String> {
    let Some(t) = table(file, index, b"name") else { return vec![] };
    let (Some(count), Some(store)) = (u16_at(t, 2), u16_at(t, 4)) else { return vec![] };
    let mut out = vec![];
    for i in 0..count as usize {
        let r = 6 + 12 * i;
        let (Some(platform), Some(id), Some(len), Some(off)) = (u16_at(t, r), u16_at(t, r + 6), u16_at(t, r + 8), u16_at(t, r + 10)) else { break };
        if !ids.contains(&id) {
            continue;
        }
        let Some(b) = t.get(store as usize + off as usize..store as usize + off as usize + len as usize) else { continue };
        let s = if platform == 0 || platform == 3 {
            String::from_utf16_lossy(&b.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>())
        } else {
            b.iter().map(|c| *c as char).collect()
        };
        out.push(s);
    }
    out
}

/// Families licensed by Apple for interface mock-ups only.
fn apple_ui_family(family: &str) -> bool {
    let f = family.trim_start_matches('.');
    f.starts_with("SF ") || f.starts_with("SF-") || f == "SF" || f.starts_with("New York")
}

/// Where macOS keeps its own fonts (and the ones it downloads on demand).
const APPLE_DIRS: &[&str] = &[
    "/System/Library/Fonts",
    "/System/Library/Fonts/Supplemental",
    "/Library/Apple/System/Library/Fonts",
    "/System/Library/AssetsV2/com_apple_MobileAsset_Font7",
    "/System/Library/AssetsV2/com_apple_MobileAsset_Font8",
];

/// Sizes and paths of the font files macOS installed (read once).
fn apple_files() -> &'static Vec<(u64, std::path::PathBuf)> {
    static FILES: OnceLock<Vec<(u64, std::path::PathBuf)>> = OnceLock::new();
    FILES.get_or_init(|| {
        let mut out = vec![];
        let mut todo: Vec<(std::path::PathBuf, u8)> = APPLE_DIRS.iter().map(|d| (d.into(), 0)).collect();
        while let Some((dir, depth)) = todo.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let Ok(m) = e.metadata() else { continue };
                if m.is_dir() && depth < 4 {
                    todo.push((e.path(), depth + 1));
                } else if m.is_file() {
                    let ext = e.path().extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                    if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc" | "dfont") {
                        out.push((m.len(), e.path()));
                    }
                }
            }
        }
        out
    })
}

/// True when `file` is byte for byte one of macOS's own font files (also
/// catches a copy of one embedded in a document).
fn is_apple_file(file: &[u8]) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let len = file.len() as u64;
    let probe = 1 << 16;
    apple_files().iter().filter(|(l, _)| *l == len).any(|(_, p)| {
        let Ok(mut f) = std::fs::File::open(p) else { return false };
        let mut head = vec![0; probe.min(file.len())];
        if f.read_exact(&mut head).is_err() || head != file[..head.len()] {
            return false;
        }
        let start = file.len().saturating_sub(probe);
        let mut tail = vec![0; file.len() - start];
        f.seek(SeekFrom::Start(start as u64)).is_ok() && f.read_exact(&mut tail).is_ok() && tail == file[start..]
    })
}

/// What a web page may do with face `index` of `file`.
pub fn web_use(file: &[u8], index: u32) -> WebUse {
    rights(file, index, false)
}

/// What an exported PDF may do with face `index` of `file`: the same as a
/// web page, except that a font whose embedding permissions are "preview
/// & print" may go in (that is what they are for). Apple's fonts and
/// fonts that don't say they are open never do: their text is drawn as
/// pictures.
pub fn pdf_use(file: &[u8], index: u32) -> WebUse {
    rights(file, index, true)
}

fn rights(file: &[u8], index: u32, print: bool) -> WebUse {
    // Family names (typographic and legacy).
    if let Some(f) = names(file, index, &[16, 1]).into_iter().find(|f| apple_ui_family(f)) {
        return WebUse::Rasterise(Reason::AppleUiFont(f.trim_start_matches('.').to_string()));
    }
    // Copyright, trademark, manufacturer.
    if names(file, index, &[0, 7, 8]).iter().any(|s| s.contains("Apple Inc") || s.contains("Apple Computer")) || is_apple_file(file) {
        return WebUse::Rasterise(Reason::Apple);
    }
    // OS/2 fsType: 2 restricted, 4 preview & print, 0x200 bitmap only.
    if let Some(fs) = table(file, index, b"OS/2").and_then(|t| u16_at(t, 8)) {
        let kind = fs & 0x000f;
        let preview_only = kind & 0x0004 != 0 && kind & 0x0008 == 0;
        if kind & 0x0002 != 0 || (preview_only && !print) || fs & 0x0200 != 0 {
            return WebUse::Rasterise(Reason::NoWebEmbedding);
        }
    }
    if !says_open(file, index) {
        return WebUse::Rasterise(Reason::NotOpen);
    }
    WebUse::Ship
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_inter_ships() {
        // Inter states the SIL Open Font License in its name table.
        assert_eq!(web_use(crate::text::bundled_font_bytes_for_tests(), 0), WebUse::Ship);
    }

    #[test]
    fn a_font_that_names_no_open_licence_is_not_shipped() {
        // The same font with its licence fields blanked out.
        let mut f = crate::text::bundled_font_bytes_for_tests().to_vec();
        let t = table(&f, 0, b"name").map(|t| t.as_ptr() as usize - f.as_ptr() as usize).unwrap();
        let (count, store) = (u16_at(&f, t + 2).unwrap() as usize, u16_at(&f, t + 4).unwrap() as usize);
        for i in 0..count {
            let r = t + 6 + 12 * i;
            if matches!(u16_at(&f, r + 6), Some(13 | 14)) {
                let (len, off) = (u16_at(&f, r + 8).unwrap() as usize, u16_at(&f, r + 10).unwrap() as usize);
                for b in &mut f[t + store + off..t + store + off + len] {
                    *b = 0;
                }
            }
        }
        assert_eq!(web_use(&f, 0), WebUse::Rasterise(Reason::NotOpen));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_fonts_never_ship() {
        for p in ["/System/Library/Fonts/Apple Color Emoji.ttc", "/System/Library/Fonts/Helvetica.ttc", "/System/Library/Fonts/Supplemental/Futura.ttc", "/System/Library/Fonts/SFNS.ttf"] {
            let Ok(f) = std::fs::read(p) else { continue };
            assert!(matches!(web_use(&f, 0), WebUse::Rasterise(_)), "{p} would ship");
        }
    }
}
