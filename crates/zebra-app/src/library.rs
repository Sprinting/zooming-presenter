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
    if s.is_empty() { "Untitled".into() } else { s }
}

/// Save a theme (with its pictures and fonts) as a theme file.
pub fn save_theme(from: &Document, theme: &Theme, dir: &Path) -> ZResult<PathBuf> {
    let mut doc = Document { title: theme.name.clone(), theme: theme.id.clone(), themes: vec![theme.clone()], ..Default::default() };
    for k in theme.asset_keys() {
        if let Some(a) = from.assets.get(k) {
            doc.assets.insert(k.to_string(), a.clone());
        }
    }
    let path = dir.join(format!("{}.zebra", file_stem(&theme.name)));
    io::write_file(&doc, &path)?;
    Ok(path)
}

/// Themes saved in the library: (file, document holding it, theme).
pub fn list_themes() -> Vec<(PathBuf, Document, Theme)> {
    let Some(d) = themes_dir() else { return vec![] };
    let mut v: Vec<_> = zebra_files(&d)
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
pub fn save_template(doc: &Document, name: &str, dir: &Path) -> ZResult<PathBuf> {
    let path = dir.join(format!("{}.zebra", file_stem(name)));
    let mut d = doc.clone();
    d.title = name.to_string();
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
