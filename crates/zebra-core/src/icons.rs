//! The built-in icon library: Lucide line icons (ISC licence, see
//! `assets/icons/LICENSE-lucide.txt`). Each icon is SVG path data on a 24x24
//! grid, meant to be stroked with round caps and joins.
use std::sync::OnceLock;

static DATA: &str = include_str!("../../../assets/icons/lucide.tsv");
/// Licence text that must ship with the app.
pub static LICENSE: &str = include_str!("../../../assets/icons/LICENSE-lucide.txt");

#[derive(Debug, Clone, Copy)]
pub struct Icon {
    pub name: &'static str,
    /// Comma-separated search keywords.
    pub tags: &'static str,
    /// SVG path data, one subpath list per `|`-separated piece.
    pub paths: &'static str,
}

impl Icon {
    pub fn path_data(&self) -> impl Iterator<Item = &'static str> {
        self.paths.split('|')
    }
}

/// Every icon, sorted by name.
pub fn all() -> &'static [Icon] {
    static ALL: OnceLock<Vec<Icon>> = OnceLock::new();
    ALL.get_or_init(|| {
        DATA.lines()
            .filter_map(|l| {
                let mut it = l.splitn(3, '\t');
                Some(Icon { name: it.next()?, tags: it.next()?, paths: it.next()? })
            })
            .collect()
    })
}

pub fn get(name: &str) -> Option<Icon> {
    let all = all();
    all.binary_search_by(|i| i.name.cmp(name)).ok().map(|i| all[i])
}

/// Icons matching every word of `query` in their name or keywords, best
/// matches first. An empty query returns everything.
pub fn search(query: &str, limit: usize) -> Vec<Icon> {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    let mut hits: Vec<(u8, Icon)> = all()
        .iter()
        .filter_map(|i| {
            let mut rank = 0u8;
            for w in &words {
                let r = if i.name == w {
                    0
                } else if i.name.starts_with(w.as_str()) {
                    1
                } else if i.name.split('-').any(|p| p.starts_with(w.as_str())) {
                    2
                } else if i.tags.split(',').any(|t| t.starts_with(w.as_str())) {
                    3
                } else if i.name.contains(w.as_str()) || i.tags.contains(w.as_str()) {
                    4
                } else {
                    return None;
                };
                rank = rank.max(r);
            }
            Some((rank, *i))
        })
        .collect();
    hits.sort_by_key(|(r, i)| (*r, i.name.len(), i.name));
    hits.into_iter().take(limit).map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_loads_and_searches() {
        assert!(all().len() > 1500);
        assert!(all().windows(2).all(|w| w[0].name < w[1].name), "sorted for binary search");
        assert!(get("lightbulb").is_some());
        assert!(get("no-such-icon").is_none());
        let r = search("light bulb", 5);
        assert!(r.iter().any(|i| i.name.contains("lightbulb")), "{:?}", r.iter().map(|i| i.name).collect::<Vec<_>>());
        assert_eq!(search("arrow", 3)[0].name.split('-').next(), Some("arrow"));
        assert!(search("", usize::MAX).len() == all().len());
    }
}
