//! What BetterOffice's model of a slide part leaves out, read from the
//! part's own XML: runs' strike-through and the text styles it inherits
//! from, what a shape read from an `mc:Fallback` stands in for, and the
//! video or sound a picture plays and how (`p:timing`).
//!
//! Shapes are found by their `cNvPr/@id` (unique within a part), walking
//! the shape tree the way the parser does: of an `mc:AlternateContent`,
//! only the branch the parser reads counts.
//!
//! Also a SmartArt graphic's data model: its items and their hierarchy,
//! which the model doesn't keep.
use std::collections::BTreeMap;

use pptx_parse::PptxPackage;
use zebra_core::diagrams::Item;

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
    /// How each video or sound plays, by its picture's id.
    pub timing: BTreeMap<u32, MediaTiming>,
}

/// The video or sound a picture plays.
#[derive(Debug, Default, Clone)]
pub(crate) struct MediaRef {
    /// The part's relationships to the media, best first: the embedded
    /// file (`p14:media`), then what `a:videoFile`/`a:audioFile` name.
    pub rels: Vec<String>,
    /// `p14:trim`: milliseconds cut from the start and from the end.
    pub trim: (f64, f64),
}

/// How a video or sound plays (`p:timing`).
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct MediaTiming {
    /// 0..1.
    pub volume: Option<f64>,
    pub mute: bool,
    /// Plays on across the slides after its own (`numSld`).
    pub across: bool,
    pub looping: bool,
    /// Starts by itself when its slide shows.
    pub auto: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ShapeInfo {
    pub text: Option<TextStrike>,
    /// What the shape stands in for, when it is an `mc:Fallback`'s.
    pub stands_in: Option<StandIn>,
    /// A picture's video or sound.
    pub media: Option<MediaRef>,
}

/// What an `mc:AlternateContent`'s choice the parser can't read holds,
/// when its fallback is read instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StandIn {
    /// A chart (PowerPoint 2016's chartex, or a classic one).
    Chart,
    /// Another graphic frame (an add-in's content, a new kind of object).
    Graphic,
    /// An equation (`a14:m`, Office Math); its fallback is a shape filled
    /// with a picture of it.
    Equation,
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
    const SAYS: [&[u8]; 7] = [b"strike=", b"AlternateContent", b"videoFile", b"audioFile", b"AudioFile", b"quickTimeFile", b"audioCd"];
    if !SAYS.iter().any(|n| contains(bytes, n)) {
        return info;
    }
    let Some(root) = parse(bytes) else { return info };
    if let Some(tree) = root.child("cSld").and_then(|c| c.child("spTree")) {
        walk(tree, &mut info, None);
    }
    if let Some(styles) = root.child("txStyles") {
        for (i, name) in ["titleStyle", "bodyStyle", "otherStyle"].into_iter().enumerate() {
            if let Some(s) = styles.child(name) {
                info.styles[i] = levels(s);
            }
        }
    }
    if let Some(t) = root.child("timing") {
        timing(t, &mut info.timing);
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

fn walk(tree: &Elem, info: &mut PartInfo, stands_in: Option<StandIn>) {
    for child in &tree.children {
        match child.local() {
            "AlternateContent" => {
                if let Some(branch) = branch(child) {
                    let choice = child.children.iter().find(|c| c.local() == "Choice");
                    let stands_in = if branch.local() == "Fallback" { choice.and_then(stand_in) } else { None };
                    walk(branch, info, stands_in);
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
                    let media = (child.local() == "pic").then(|| child.child("nvPicPr").and_then(|n| n.child("nvPr")).and_then(media_ref)).flatten();
                    let shape = ShapeInfo { text: child.child("txBody").map(text_strike), stands_in, media };
                    info.shapes.entry(id).or_insert(shape);
                }
                if child.local() == "grpSp" {
                    walk(child, info, None);
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

/// The media a picture's `p:nvPr` names, if it names any.
fn media_ref(nv: &Elem) -> Option<MediaRef> {
    let mut out = MediaRef::default();
    let mut found = false;
    let rel = |e: &Elem| e.attrs.iter().find(|(k, _)| k.ends_with(":embed") || k.ends_with(":link")).map(|(_, v)| v.clone()).filter(|v| !v.is_empty());
    for e in nv.descendants() {
        match e.local() {
            "media" => {
                found = true;
                // The embedded file first: it is what PowerPoint plays.
                if let Some(r) = rel(e) {
                    out.rels.insert(0, r);
                }
                if let Some(t) = e.child("trim") {
                    let ms = |k: &str| t.attr(k).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(0.0);
                    out.trim = (ms("st"), ms("end"));
                }
            }
            "videoFile" | "quickTimeFile" | "audioFile" | "wavAudioFile" | "audioCd" => {
                found = true;
                out.rels.extend(rel(e));
            }
            _ => {}
        }
    }
    found.then_some(out)
}

/// How the slide's videos and sounds play: `p:cMediaNode`s, and which
/// the main sequence starts by itself (a media "play" effect in its first
/// group that doesn't wait for a click).
fn timing(t: &Elem, out: &mut BTreeMap<u32, MediaTiming>) {
    let spid = |e: &Elem| e.descendants().into_iter().find(|d| d.local() == "spTgt").and_then(|d| d.attr("spid")).and_then(|v| v.parse::<u32>().ok());
    let all = t.descendants();
    for node in all.iter().filter(|e| e.local() == "cMediaNode") {
        let Some(id) = node.child("tgtEl").and_then(spid) else { continue };
        let m = out.entry(id).or_default();
        m.volume = node.attr("vol").and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite()).map(|v| (v / 100_000.0).clamp(0.0, 1.0));
        m.mute = matches!(node.attr("mute"), Some("1" | "true"));
        m.across = node.attr("numSld").and_then(|v| v.parse::<u32>().ok()).is_some_and(|n| n > 1);
        m.looping = node.child("cTn").and_then(|c| c.attr("repeatCount")) == Some("indefinite");
    }
    let main = all.iter().find(|e| e.local() == "cTn" && e.attr("nodeType") == Some("mainSeq"));
    let first = main.and_then(|m| m.child("childTnLst")).and_then(|l| l.child("par"));
    for effect in first.map(Elem::descendants).unwrap_or_default() {
        let play = effect.local() == "cTn" && effect.attr("presetClass") == Some("mediacall") && matches!(effect.attr("presetID"), None | Some("1"));
        if play && effect.attr("nodeType") != Some("clickEffect") {
            if let Some(id) = spid(effect) {
                out.entry(id).or_default().auto = true;
            }
        }
    }
}

/// What a choice the parser can't read holds, when it is something the
/// import notes.
fn stand_in(choice: &Elem) -> Option<StandIn> {
    let content = choice.children.first()?;
    if content.descendants().iter().any(|d| matches!(d.local(), "m" | "oMath" | "oMathPara")) {
        return Some(StandIn::Equation);
    }
    if content.local() != "graphicFrame" {
        return None;
    }
    let uri = content.child("graphic").and_then(|g| g.child("graphicData")).and_then(|d| d.attr("uri")).unwrap_or("");
    Some(if uri.contains("/chart") { StandIn::Chart } else { StandIn::Graphic })
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
    /// The text directly inside it (not its children's).
    pub text: String,
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
    /// Every element inside this one, in document order.
    pub fn descendants(&self) -> Vec<&Elem> {
        let mut out = vec![];
        let mut stack: Vec<&Elem> = self.children.iter().rev().collect();
        while let Some(e) = stack.pop() {
            out.push(e);
            stack.extend(e.children.iter().rev());
        }
        out
    }
}

/// Parsing stops (and gives up) past this many elements or this depth: the
/// parser has read the part within its own limits already, so only a part
/// far outside them gets here.
const MAX_ELEMENTS: usize = 4_000_000;
const MAX_DEPTH: usize = 256;
/// A SmartArt data or layout part is read only up to this many elements:
/// [`MAX_ITEMS`] items take far fewer, and 1.5 million empty points took a
/// gigabyte as a tree.
const MAX_SMARTART_ELEMENTS: usize = 200_000;

/// The root element of an XML document; None when it isn't well formed.
pub(crate) fn parse(bytes: &[u8]) -> Option<Elem> {
    parse_within(bytes, MAX_ELEMENTS)
}

/// [`parse`], giving up past `max` elements.
fn parse_within(bytes: &[u8], max: usize) -> Option<Elem> {
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
        text: String::new(),
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
            Event::Text(t) => {
                if let Some(e) = stack.last_mut() {
                    e.text.push_str(&t.xml_content(quick_xml::XmlVersion::Implicit1_0).ok()?);
                }
                None
            }
            Event::CData(t) => {
                if let Some(e) = stack.last_mut() {
                    e.text.push_str(&t.decode().ok()?);
                }
                None
            }
            Event::GeneralRef(r) => {
                if let Some(e) = stack.last_mut() {
                    match r.resolve_char_ref().ok()? {
                        Some(c) => e.text.push(c),
                        None => e.text.push_str(quick_xml::escape::resolve_predefined_entity(&r.decode().ok()?)?),
                    }
                }
                None
            }
            _ => None,
        };
        if let Some(e) = done {
            n += 1;
            if n > max {
                return None;
            }
            match stack.last_mut() {
                Some(parent) => parent.children.push(e),
                None => return Some(e),
            }
        }
    }
}

/// A SmartArt graphic's data model (`dgm:dataModel`): its items in the
/// hierarchy its `parOf` connections make, and its layout's name.
#[derive(Debug, Default)]
pub(crate) struct SmartArt {
    /// The last part of the layout's id (`process1` of
    /// `urn:microsoft.com/office/officeart/2005/8/layout/process1`).
    pub layout: Option<String>,
    pub items: Vec<Item>,
}

/// At most this many items are read from a data model.
const MAX_ITEMS: usize = 4_096;

/// Read a SmartArt data part, and its layout part when there is one. None
/// when the data part isn't a data model.
pub(crate) fn smart_art(data: &[u8], layout: Option<&[u8]>) -> Option<SmartArt> {
    let root = parse_within(data, MAX_SMARTART_ELEMENTS)?;
    if root.local() != "dataModel" {
        return None;
    }
    let pts: Vec<&Elem> = root.child("ptLst").map(|l| l.children.iter().filter(|c| c.local() == "pt").collect()).unwrap_or_default();
    let kind = |p: &Elem| p.attr("type").unwrap_or("node").to_string();
    let doc: Vec<&str> = pts.iter().filter(|p| kind(p) == "doc").filter_map(|p| p.attr("modelId")).collect();
    let name = |id: &str| id.rsplit('/').next().unwrap_or(id).to_string();
    let from_layout = layout.and_then(|l| parse_within(l, MAX_SMARTART_ELEMENTS)).and_then(|l| l.attr("uniqueId").map(name));
    let from_doc = || pts.iter().filter(|p| kind(p) == "doc").find_map(|p| p.child("prSet")?.attr("loTypeId").map(name));
    let layout = from_layout.or_else(from_doc).filter(|l| !l.is_empty());
    // Items (`node`, and an org chart's `asst`) by id, with their words.
    let mut words: BTreeMap<&str, String> = BTreeMap::new();
    for p in &pts {
        if matches!(kind(p).as_str(), "node" | "asst") {
            if let Some(id) = p.attr("modelId") {
                words.insert(id, point_text(p));
            }
        }
    }
    // parOf connections, each parent's children in order.
    let mut kids: BTreeMap<&str, Vec<(i64, usize, &str)>> = BTreeMap::new();
    let mut has_parent = std::collections::BTreeSet::new();
    let cxns = root.child("cxnLst").map(|l| l.children.iter().filter(|c| c.local() == "cxn").collect::<Vec<_>>()).unwrap_or_default();
    for (i, c) in cxns.iter().enumerate() {
        if c.attr("type").unwrap_or("parOf") != "parOf" {
            continue;
        }
        let (Some(src), Some(dest)) = (c.attr("srcId"), c.attr("destId")) else { continue };
        let ord = c.attr("srcOrd").and_then(|o| o.parse().ok()).unwrap_or(i64::MAX);
        kids.entry(src).or_default().push((ord, i, dest));
        has_parent.insert(dest);
    }
    for v in kids.values_mut() {
        v.sort();
    }
    let mut roots: Vec<&str> = doc.iter().flat_map(|d| kids.get(d).into_iter().flatten().map(|k| k.2)).filter(|id| words.contains_key(id)).collect();
    if roots.is_empty() {
        roots = pts.iter().filter_map(|p| p.attr("modelId")).filter(|id| words.contains_key(id) && !has_parent.contains(id)).collect();
    }
    let mut seen = std::collections::BTreeSet::new();
    fn build<'x>(id: &'x str, words: &BTreeMap<&'x str, String>, kids: &BTreeMap<&'x str, Vec<(i64, usize, &'x str)>>, seen: &mut std::collections::BTreeSet<&'x str>, depth: usize) -> Option<Item> {
        if depth > 32 || seen.len() >= MAX_ITEMS || !seen.insert(id) {
            return None;
        }
        let text = words.get(id)?.clone();
        let children = kids.get(id).into_iter().flatten().filter_map(|k| build(k.2, words, kids, seen, depth + 1)).collect();
        Some(Item { text, children })
    }
    let items = roots.into_iter().filter_map(|r| build(r, &words, &kids, &mut seen, 0)).collect();
    Some(SmartArt { layout, items })
}

/// A point's words (`dgm:t`), a line for each paragraph.
fn point_text(p: &Elem) -> String {
    let Some(t) = p.child("t") else { return String::new() };
    let lines: Vec<String> = t
        .descendants()
        .into_iter()
        .filter(|e| e.local() == "p")
        .map(|para| para.descendants().into_iter().filter(|e| e.local() == "t" || e.local() == "br").map(|e| if e.local() == "br" { " " } else { e.text.as_str() }).collect::<String>())
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect();
    lines.join("\n")
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
        assert_eq!(root.child("r").and_then(|r| r.child("t")).unwrap().text, "x & y");
        assert_eq!(parse(b"<t>a&#233;<![CDATA[<b>]]>&gt;</t>").unwrap().text, "a\u{e9}<b>>");
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
