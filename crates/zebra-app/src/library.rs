//! The person's own themes and templates, kept as ordinary `.zebra` files
//! in `~/Library/Application Support/Zebra/` (Themes/ and Templates/), so
//! they can be shared by copying a file.
use std::path::{Path, PathBuf};

use zebra_core::{io, Document, Theme, ZError, ZResult};

fn dir(name: &str) -> Option<PathBuf> {
    let d = dirs::data_dir()?.join("Zebra").join(name);
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

pub fn themes_dir() -> Option<PathBuf> {
    dir("Themes")
}
pub fn templates_dir() -> Option<PathBuf> {
    dir("Templates")
}

/// A file name from a human name: letters, digits, spaces and dashes.
pub fn file_stem(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let s = s.trim().to_string();
    if s.is_empty() { crate::t!("library-file-untitled") } else { s }
}

/// Save a theme (with its pictures and fonts) as a theme file.
pub fn save_theme(from: &Document, theme: &Theme, dir: &Path) -> ZResult<PathBuf> {
    let mut doc = Document { title: theme.name.clone(), theme: theme.id.clone(), themes: vec![theme.clone()], ..Default::default() };
    for k in theme.asset_keys() {
        if let Some(a) = from.assets.get(k) {
            doc.assets.insert(k.to_string(), a.clone());
        }
    }
    // Saving the same theme again updates its file. A different theme whose
    // name gives the same file name gets a file of its own: it used to
    // replace the other theme without a word.
    let path = free_path(dir, &file_stem(&theme.name), |p| io::load(p).is_ok_and(|d| d.theme == theme.id));
    io::write_file(&doc, &path)?;
    Ok(path)
}

/// `stem.zebra` in `dir`, or `stem 2.zebra`, `stem 3.zebra`… : the first
/// that does not exist yet, or that `mine` says may be replaced.
fn free_path(dir: &Path, stem: &str, mine: impl Fn(&Path) -> bool) -> PathBuf {
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{stem}.zebra") } else { format!("{stem} {n}.zebra") };
        let p = dir.join(name);
        if !p.exists() || mine(&p) {
            return p;
        }
        n += 1;
    }
}

/// Themes saved in the library: (file, document holding it, theme).
///
/// The theme menu asks on every frame it is open, so the files are read
/// again only when the folder's listing (names, sizes, dates) changed.
pub fn list_themes() -> Vec<(PathBuf, Document, Theme)> {
    type Stamp = Vec<(PathBuf, u64, Option<std::time::SystemTime>)>;
    static CACHE: std::sync::Mutex<Option<(Stamp, Vec<(PathBuf, Document, Theme)>)>> = std::sync::Mutex::new(None);
    let Some(d) = themes_dir() else { return vec![] };
    let files = zebra_files(&d);
    let stamp: Stamp = files
        .iter()
        .map(|p| {
            let m = std::fs::metadata(p).ok();
            (p.clone(), m.as_ref().map_or(0, |m| m.len()), m.and_then(|m| m.modified().ok()))
        })
        .collect();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((s, v)) = &*cache {
        if *s == stamp {
            return v.clone();
        }
    }
    let v = read_themes(files);
    *cache = Some((stamp, v.clone()));
    v
}

fn read_themes(files: Vec<PathBuf>) -> Vec<(PathBuf, Document, Theme)> {
    let mut v: Vec<_> = files
        .into_iter()
        .filter_map(|p| {
            let doc = io::load(&p).ok()?;
            let t = doc.themes.first()?.clone();
            Some((p, doc, t))
        })
        .collect();
    v.sort_by_key(|(_, _, t)| t.name.to_lowercase());
    v
}

/// Templates the person saved: (file, title).
pub fn list_templates() -> Vec<(PathBuf, String)> {
    let Some(d) = templates_dir() else { return vec![] };
    let mut v: Vec<_> = zebra_files(&d)
        .into_iter()
        .map(|p| {
            let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            (p, name)
        })
        .collect();
    v.sort_by_key(|(_, n)| n.to_lowercase());
    v
}

/// Save the current presentation as a template.
///
/// A template of that name stays as it is; this one is saved as "name 2".
pub fn save_template(doc: &Document, name: &str, dir: &Path) -> ZResult<PathBuf> {
    let path = free_path(dir, &file_stem(name), |_| false);
    let mut d = doc.clone();
    d.title = if path.file_stem().is_some_and(|s| s.to_string_lossy() == file_stem(name)) {
        name.to_string()
    } else {
        path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string())
    };
    io::write_file(&d, &path).map_err(|e| ZError::new("io", e.message))?;
    Ok(path)
}

fn zebra_files(d: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(d)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e == "zebra")).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_never_replaces_another_theme_or_template() {
        let tmp = std::env::temp_dir().join(format!("zebra-lib-clash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let mut a = Theme::resolve("night");
        a.id = "mine-a".into();
        a.name = "Mine".into();
        let mut b = a.clone();
        b.id = "mine-b".into();
        let pa = save_theme(&Document::default(), &a, &tmp).unwrap();
        let pb = save_theme(&Document::default(), &b, &tmp).unwrap();
        assert_ne!(pa, pb, "a second theme called Mine gets its own file");
        assert_eq!(io::load(&pa).unwrap().theme, "mine-a");
        assert_eq!(save_theme(&Document::default(), &a, &tmp).unwrap(), pa, "saving the same theme again updates it");
        assert_eq!(save_theme(&Document::default(), &b, &tmp).unwrap(), pb);

        let mut one = Document::default();
        one.title = "Pitch".into();
        let t1 = save_template(&one, "Pitch", &tmp).unwrap();
        let t2 = save_template(&Document::default(), "Pitch", &tmp).unwrap();
        assert_ne!(t1, t2);
        assert_eq!(io::load(&t1).unwrap().title, "Pitch");
        assert_eq!(io::load(&t2).unwrap().title, "Pitch 2");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn theme_file_round_trip() {
        let tmp = std::env::temp_dir().join(format!("zebra-lib-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut t = Theme::resolve("night");
        t.id = "mine".into();
        t.name = "Mine / v2".into();
        let p = save_theme(&Document::default(), &t, &tmp).unwrap();
        assert!(p.file_name().unwrap().to_string_lossy().starts_with("Mine _ v2"));
        let back = io::load(&p).unwrap();
        assert_eq!(back.themes[0], t);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
