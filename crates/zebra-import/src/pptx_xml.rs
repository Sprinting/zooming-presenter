//! What BetterOffice's model of a slide part leaves out, read from the
//! part's own XML: runs' strike-through and the text styles it inherits
//! from.
//!
//! Shapes are found by their `cNvPr/@id` (unique within a part), walking
//! the shape tree the way the parser does: of an `mc:AlternateContent`,
//! only the branch the parser reads counts.
use std::collections::BTreeMap;

use pptx_parse::PptxPackage;

const DRAWINGML_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const PRESENTATIONML_NS: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";

/// What the model leaves out of one part.
#[derive(Debug, Default)]
pub(crate) struct PartInfo {
    pub shapes: BTreeMap<u32, ShapeInfo>,
    /// A master's title, body and other text styles' strike-through, by
    /// level; the presentation part's default text style is in `other`
    /// (with its `defPPr` in `default`).
    pub styles: [[Option<bool>; 9]; 3],
    pub default: Option<bool>,
}

#[derive(Debug, Default)]
pub(crate) struct ShapeInfo {
    pub text: Option<TextStrike>,
}

/// The strike-through a text body says, at each level of its cascade
/// (`None`: not said there).
#[derive(Debug, Default)]
pub(crate) struct TextStrike {
    /// `a:lstStyle/a:defPPr/a:defRPr`.
    pub default: Option<bool>,
    /// `a:lstStyle/a:lvlNpPr/a:defRPr`.
    pub levels: [Option<bool>; 9],
    /// Each paragraph's `a:pPr/a:defRPr`, and its runs' (`a:r`, `a:fld`,
    /// `a:br`, in order) `a:rPr`.
    pub paras: Vec<(Option<bool>, Vec<Option<bool>>)>,
}

/// Which text style of a master a text takes.
#[derive(Debug, Clone, Copy)]
pub(crate) enum StyleKind {
    Title = 0,
    Body = 1,
    Other = 2,
}

/// The part's info, read only when the part says something the model
/// doesn't keep (most parts don't, and cost nothing).
pub(crate) fn part_info(pkg: &PptxPackage, part: &str) -> PartInfo {
    let mut info = PartInfo::default();
    let Some(bytes) = pkg.part_bytes(part) else { return info };
    if !contains(bytes, b"strike=") {
        return info;
    }
    let Some(root) = parse(bytes) else { return info };
    if let Some(tree) = root.child("cSld").and_then(|c| c.child("spTree")) {
        walk(tree, &mut info);
    }
    if let Some(styles) = root.child("txStyles") {
        for (i, name) in ["titleStyle", "bodyStyle", "otherStyle"].into_iter().enumerate() {
            if let Some(s) = styles.child(name) {
                info.styles[i] = levels(s);
            }
        }
    }
    if let Some(s) = root.child("defaultTextStyle") {
        info.styles[StyleKind::Other as usize] = levels(s);
        info.default = s.child("defPPr").and_then(def_strike);
    }
    info
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn walk(tree: &Elem, info: &mut PartInfo) {
    for child in &tree.children {
        match child.local() {
            "AlternateContent" => {
                if let Some(branch) = branch(child) {
                    walk(branch, info);
                }
            }
            "sp" | "cxnSp" | "pic" | "graphicFrame" | "grpSp" => {
                let id = child
                    .children
                    .iter()
                    .find(|c| c.local().starts_with("nv"))
                    .and_then(|nv| nv.child("cNvPr"))
                    .and_then(|c| c.attr("id"))
                    .and_then(|id| id.parse::<u32>().ok());
                if let Some(id) = id {
                    let shape = ShapeInfo { text: child.child("txBody").map(text_strike) };
                    info.shapes.entry(id).or_insert(shape);
                }
                if child.local() == "grpSp" {
                    walk(child, info);
                }
            }
            _ => {}
        }
    }
}

/// The branch of an `mc:AlternateContent` the parser reads: the first
/// `mc:Choice` that requires only DrawingML or PresentationML, else the
/// `mc:Fallback`.
fn branch(alt: &Elem) -> Option<&Elem> {
    let supported = |choice: &Elem| {
        let Some(req) = choice.attr("Requires") else { return false };
        let mut prefixes = req.split_whitespace().peekable();
        prefixes.peek().is_some()
            && prefixes.all(|p| {
                let decl = format!("xmlns:{p}");
                choice.attr(&decl).or_else(|| alt.attr(&decl)).is_some_and(|ns| ns == DRAWINGML_NS || ns == PRESENTATIONML_NS)
            })
    };
    alt.children.iter().find(|c| c.local() == "Choice" && supported(c)).or_else(|| alt.children.iter().find(|c| c.local() == "Fallback"))
}

/// `strike` of an `a:rPr`/`a:defRPr`: sngStrike and dblStrike strike,
/// noStrike doesn't.
fn strike(rpr: &Elem) -> Option<bool> {
    rpr.attr("strike").map(|v| v != "noStrike")
}

/// `a:defRPr/@strike` of a paragraph properties element.
fn def_strike(ppr: &Elem) -> Option<bool> {
    ppr.child("defRPr").and_then(strike)
}

fn levels(style: &Elem) -> [Option<bool>; 9] {
    std::array::from_fn(|i| style.child(&format!("lvl{}pPr", i + 1)).and_then(def_strike))
}

fn text_strike(body: &Elem) -> TextStrike {
    let lst = body.child("lstStyle");
    TextStrike {
        default: lst.and_then(|l| l.child("defPPr")).and_then(def_strike),
        levels: lst.map(levels).unwrap_or_default(),
        paras: body
            .children
            .iter()
            .filter(|p| p.local() == "p")
            .map(|p| {
                let runs = p.children.iter().filter(|r| matches!(r.local(), "r" | "fld" | "br")).map(|r| r.child("rPr").and_then(strike)).collect();
                (p.child("pPr").and_then(def_strike), runs)
            })
            .collect(),
    }
}

/// An XML element: its qualified name, attributes and child elements
/// (text is left out).
#[derive(Debug, Default)]
pub(crate) struct Elem {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Elem>,
}

impl Elem {
    pub fn local(&self) -> &str {
        self.name.rsplit_once(':').map_or(self.name.as_str(), |(_, l)| l)
    }
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    /// The first child element with this local name.
    pub fn child(&self, local: &str) -> Option<&Elem> {
        self.children.iter().find(|c| c.local() == local)
    }
}

/// Parsing stops (and gives up) past this many elements or this depth: the
/// parser has read the part within its own limits already, so only a part
/// far outside them gets here.
const MAX_ELEMENTS: usize = 4_000_000;
const MAX_DEPTH: usize = 256;

/// The root element of an XML document; None when it isn't well formed.
pub(crate) fn parse(bytes: &[u8]) -> Option<Elem> {
    use quick_xml::events::{BytesStart, Event};
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let decoder = reader.decoder();
    let elem = |e: &BytesStart| Elem {
        name: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
        attrs: e
            .attributes()
            .filter_map(Result::ok)
            .map(|a| (String::from_utf8_lossy(a.key.as_ref()).into_owned(), a.decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder).map(|v| v.into_owned()).unwrap_or_default()))
            .collect(),
        children: vec![],
    };
    let mut stack: Vec<Elem> = vec![];
    let mut n = 0usize;
    loop {
        let done = match reader.read_event().ok()? {
            Event::Start(e) => {
                stack.push(elem(&e));
                if stack.len() > MAX_DEPTH {
                    return None;
                }
                None
            }
            Event::Empty(e) => Some(elem(&e)),
            Event::End(_) => Some(stack.pop()?),
            Event::Eof => return None,
            _ => None,
        };
        if let Some(e) = done {
            n += 1;
            if n > MAX_ELEMENTS {
                return None;
            }
            match stack.last_mut() {
                Some(parent) => parent.children.push(e),
                None => return Some(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_elements_and_attributes() {
        let root = parse(br#"<?xml version="1.0"?><p:sld xmlns:p="x"><a:r><a:rPr strike="sngStrike" lang="en"/><a:t>x &amp; y</a:t></a:r></p:sld>"#).unwrap();
        assert_eq!(root.local(), "sld");
        let rpr = root.child("r").and_then(|r| r.child("rPr")).unwrap();
        assert_eq!(strike(rpr), Some(true));
        assert_eq!(rpr.attr("lang"), Some("en"));
        assert!(parse(b"<a><b></a>").is_none());
        assert!(parse(b"<a>").is_none());
        assert!(parse(b"").is_none());
    }

    #[test]
    fn reads_the_branch_the_parser_reads() {
        let alt = parse(
            br#"<mc:AlternateContent xmlns:mc="m"><mc:Choice xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main" Requires="a14"><p:sp id="1"/></mc:Choice><mc:Fallback><p:sp id="2"/></mc:Fallback></mc:AlternateContent>"#,
        )
        .unwrap();
        assert_eq!(branch(&alt).map(Elem::local), Some("Fallback"));
        let alt = parse(
            br#"<mc:AlternateContent xmlns:mc="m" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><mc:Choice Requires="p"><p:sp/></mc:Choice><mc:Fallback/></mc:AlternateContent>"#,
        )
        .unwrap();
        assert_eq!(branch(&alt).map(Elem::local), Some("Choice"));
    }
}
