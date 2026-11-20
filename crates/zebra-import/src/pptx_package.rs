//! A PowerPoint file read a few slides at a time.
//!
//! BetterOffice's parser reads a whole package or nothing: one missing
//! slide, layout or theme, one relationship pointing outside the file, or
//! a deck longer than its 5000-part zip limit made the whole file
//! "damaged", and its budgets (XML events, shapes) were the whole deck's.
//! So the parser is never given the file. It is given small zips made
//! here, each holding a batch of slides and only the parts they reach:
//!
//! - every XML part is unpacked once first (within a size cap, its real
//!   size counted, so a zip directory can't lie) and checked to be well
//!   formed; compressed parts are then copied as they are;
//! - every relationships part is written again, keeping links to parts in
//!   the batch and outside links, and dropping links to parts that are
//!   missing, broken, outside the package or not in this batch;
//! - `presentation.xml` lists only the batch's slides;
//! - media goes in as empty entries ([`crate::pptx_media`] streams it).
//!
//! A slide that is missing, broken or too big is left out and named; a
//! broken layout, master or theme is left out (its slides come in without
//! that design) and noted. A batch the parser still refuses is halved
//! until the slide it can't read is found. One batch is in memory at a
//! time.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Cursor, Write};
use std::sync::Mutex;

use pptx_parse::PptxPackage;
use quick_xml::events::Event;

use crate::pptx_media::{is_media, MediaSource};
use crate::ImportError;

/// Slides read together (fewer when they are big).
pub const BATCH_SLIDES: usize = 100;
/// XML of a batch's own parts (not counting the shared layouts, masters
/// and themes), unpacked. A slide bigger than this is read on its own.
const BATCH_XML: u64 = 4 << 20;
/// Parts in one batch's zip: BetterOffice refuses more than 5000.
const BATCH_PARTS: usize = 4000;
/// What parsing takes per byte of XML: BetterOffice builds a whole tree,
/// about 55 times the XML (measured: +452 MB for an 8 MB slide).
const PARSE_COST: u64 = 55;

/// The most XML one part may have: what its parse would take must fit
/// in [`zebra_core::io::memory_budget`] (half the Mac's memory). There is
/// no size limit beyond that, as Keynote has none; a part past it is left
/// out and the import says so.
pub fn max_xml() -> u64 {
    zebra_core::io::memory_budget() / PARSE_COST
}

/// Deepest XML nesting BetterOffice reads.
const MAX_DEPTH: usize = 256;

const OFFICE_DOCUMENT: &str = "/officeDocument";
const CONTENT_TYPES: &str = "[Content_Types].xml";
const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// One relationship.
#[derive(Debug, Clone)]
struct Rel {
    id: String,
    kind: String,
    target: String,
    external: bool,
    /// The part it points at, when it is inside the package.
    to: Option<String>,
}

impl Rel {
    fn is(&self, suffix: &str) -> bool {
        self.kind.ends_with(suffix)
    }
}

/// What a check of a part found.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Checked {
    /// Well formed, this many bytes unpacked.
    Ok(u64),
    /// Missing or broken.
    Bad,
    /// Over its size cap.
    TooBig,
}

/// What a batch made of its slides, in slide order.
pub enum Read {
    /// The parsed slides: indices (0-based, into the deck's slide list) of
    /// `pkg.slides`, one for one.
    Parsed(Box<PptxPackage>, Vec<usize>),
    /// A slide that can't be read, and why.
    Skipped(usize, Skip),
}

/// Why a slide was left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// Missing or broken.
    Broken,
    /// Over a limit: more XML than [`max_xml`], or more shapes or
    /// XML than the parser reads.
    TooBig,
}

pub struct Package {
    src: MediaSource,
    /// `ppt/presentation.xml` (or wherever the package says it is).
    presentation: String,
    presentation_xml: Vec<u8>,
    content_types: Vec<u8>,
    /// Each slide the presentation lists, in order: its part, when its
    /// relationship and part exist.
    slides: Vec<Option<String>>,
    /// Parts checked so far.
    checked: Mutex<HashMap<String, Checked>>,
    /// Layouts, masters and themes that couldn't be read.
    broken_design: Mutex<BTreeSet<String>>,
}

impl Package {
    /// Read the file's directory and its slide list (no slide is read).
    pub fn open(src: MediaSource) -> Result<Package, ImportError> {
        let damaged = ImportError::Damaged("PowerPoint");
        let root = read_rels(&src, "");
        let presentation = root.iter().find(|r| r.is(OFFICE_DOCUMENT)).and_then(|r| r.to.clone()).filter(|p| src.has(p)).unwrap_or_else(|| "ppt/presentation.xml".into());
        let presentation_xml = src.read_part(&presentation, max_xml()).filter(|b| check(b)).ok_or(damaged.clone())?;
        let rels = read_rels(&src, &presentation);
        let list = id_list(&presentation_xml, "sldIdLst", "sldId").ok_or(damaged.clone())?;
        let slides: Vec<Option<String>> = list
            .iter()
            .map(|rid| rid.as_deref().and_then(|id| rels.iter().find(|r| r.id == id && r.is("/slide"))).and_then(|r| r.to.clone()).filter(|p| src.has(p)))
            .collect();
        // No slide at all, or none whose part is there.
        if slides.iter().all(Option::is_none) {
            return Err(damaged);
        }
        let content_types = src.read_part(CONTENT_TYPES, max_xml()).filter(|b| check(b)).unwrap_or_else(default_content_types);
        Ok(Package { src, presentation, presentation_xml, content_types, slides, checked: Mutex::new(HashMap::new()), broken_design: Mutex::new(BTreeSet::new()) })
    }

    /// The file's media.
    pub fn media(&self) -> &MediaSource {
        &self.src
    }

    /// Slides the presentation lists (missing ones included, so numbers
    /// match PowerPoint's).
    pub fn slide_count(&self) -> usize {
        self.slides.len()
    }

    /// Layouts, masters and themes found broken so far.
    pub fn broken_designs(&self) -> usize {
        self.broken_design.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Read slides `range` (0-based), a batch at a time: `each` gets every
    /// parsed batch or skipped slide, in slide order. Stops when `each`
    /// returns false.
    pub fn read(&self, range: std::ops::Range<usize>, mut each: impl FnMut(Read) -> bool) {
        let mut i = range.start;
        while i < range.end {
            let end = (i + BATCH_SLIDES).min(range.end);
            let wanted: Vec<usize> = (i..end).collect();
            if !self.read_batch(&wanted, &mut each) {
                return;
            }
            i = end;
        }
    }

    /// Parse `slides`; halve them when they are too big together or the
    /// parser refuses them. False: stop.
    fn read_batch(&self, slides: &[usize], each: &mut impl FnMut(Read) -> bool) -> bool {
        let mut ok = vec![];
        for &i in slides {
            let checked = self.slides.get(i).cloned().flatten().map_or(Checked::Bad, |p| self.check(&p, max_xml()));
            let why = if checked == Checked::TooBig { Skip::TooBig } else { Skip::Broken };
            if matches!(checked, Checked::Ok(_)) {
                ok.push(i);
            } else if !ok.is_empty() {
                // Keep slide order: what came before goes first.
                if !self.parse_or_halve(&std::mem::take(&mut ok), each) {
                    return false;
                }
                if !each(Read::Skipped(i, why)) {
                    return false;
                }
            } else if !each(Read::Skipped(i, why)) {
                return false;
            }
        }
        ok.is_empty() || self.parse_or_halve(&ok, each)
    }

    fn parse_or_halve(&self, slides: &[usize], each: &mut impl FnMut(Read) -> bool) -> bool {
        let built = self.build(slides);
        // Why the parser refused it (a slide on its own over its limits
        // is too big, not broken).
        let mut why = Skip::Broken;
        let parsed = match &built {
            Some(b) if slides.len() == 1 || (b.xml <= BATCH_XML && b.parts <= BATCH_PARTS) => match pptx_parse::parse_pptx(&b.zip) {
                Ok(p) => Some(p).filter(|p| p.slides.len() == slides.len()),
                Err(pptx_parse::PptxError::ResourceLimit { .. }) => {
                    why = Skip::TooBig;
                    None
                }
                Err(_) => None,
            },
            _ => None,
        };
        drop(built);
        match parsed {
            Some(pkg) => each(Read::Parsed(Box::new(pkg), slides.to_vec())),
            None if slides.len() > 1 => {
                let (a, b) = slides.split_at(slides.len() / 2);
                self.parse_or_halve(a, each) && self.parse_or_halve(b, each)
            }
            None => each(Read::Skipped(slides[0], why)),
        }
    }

    /// Check part `path` (once): well formed, at most `cap` bytes.
    fn check(&self, path: &str, cap: u64) -> Checked {
        if let Some(c) = self.checked.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
            return *c;
        }
        let c = match self.src.read_part(path, cap) {
            Some(b) if check(&b) => Checked::Ok(b.len() as u64),
            // Says it is bigger than its cap (or turned out so).
            None if self.src.size(path).is_some_and(|n| n > cap) => Checked::TooBig,
            _ => Checked::Bad,
        };
        self.checked.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_string(), c);
        c
    }

    /// The zip for `slides` (all of them readable on their own).
    fn build(&self, slides: &[usize]) -> Option<Batch> {
        let wanted: BTreeSet<&str> = slides.iter().filter_map(|&i| self.slides.get(i)?.as_deref()).collect();
        // The parts the batch reaches, from the presentation.
        let mut parts: BTreeMap<String, Kind> = BTreeMap::new();
        let mut rels: BTreeMap<String, Vec<Rel>> = BTreeMap::new();
        let mut queue = vec![self.presentation.clone()];
        let mut xml = 0u64;
        parts.insert(self.presentation.clone(), Kind::Xml);
        while let Some(part) = queue.pop() {
            let from_slide = wanted.contains(part.as_str());
            let own = read_rels(&self.src, &part);
            for r in &own {
                let Some(to) = &r.to else { continue };
                if parts.contains_key(to) {
                    continue;
                }
                // A slide comes in only from the presentation, and only the
                // batch's; a layout only from a slide (a master lists them
                // all).
                if r.is("/slide") && !(part == self.presentation && wanted.contains(to.as_str())) {
                    continue;
                }
                if r.is("/slideLayout") && !from_slide {
                    continue;
                }
                let design = r.is("/slideLayout") || r.is("/slideMaster") || r.is("/theme");
                if !self.src.has(to) {
                    if design {
                        self.broken_design.lock().unwrap_or_else(|e| e.into_inner()).insert(to.clone());
                    }
                    continue;
                }
                if is_media(to) || !is_xml(to) {
                    parts.insert(to.clone(), Kind::Empty);
                    continue;
                }
                let cap = max_xml();
                match self.check(to, cap) {
                    Checked::Ok(n) => {
                        if !design {
                            xml += n;
                        }
                        parts.insert(to.clone(), Kind::Xml);
                        queue.push(to.clone());
                    }
                    Checked::Bad | Checked::TooBig => {
                        if design {
                            self.broken_design.lock().unwrap_or_else(|e| e.into_inner()).insert(to.clone());
                        }
                    }
                }
            }
            rels.insert(part, own);
        }
        let n = parts.len() + rels.len() + 2;
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file(CONTENT_TYPES, deflated).ok()?;
        z.write_all(&self.content_types).ok()?;
        let root = Rel { id: "rId1".into(), kind: "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument".into(), target: format!("/{}", self.presentation), external: false, to: None };
        z.start_file("_rels/.rels", deflated).ok()?;
        z.write_all(&rels_xml(std::slice::from_ref(&root))).ok()?;
        for (part, kind) in &parts {
            if part == &self.presentation {
                let keep_rels = &rels[part];
                let ids = |kind: &str| keep_rels.iter().filter(|r| r.is(kind) && r.to.as_ref().is_some_and(|t| parts.contains_key(t))).map(|r| r.id.clone()).collect::<BTreeSet<String>>();
                // Slides by place in the list (two may share a part), and
                // masters by relationship.
                let keep: BTreeSet<usize> = slides.iter().copied().collect();
                let xml = filter_presentation(&self.presentation_xml, &keep, &ids("/slideMaster"))?;
                z.start_file(part.as_str(), deflated).ok()?;
                z.write_all(&xml).ok()?;
                continue;
            }
            match kind {
                Kind::Xml => {
                    if !self.src.copy_raw(part, &mut z) {
                        return None;
                    }
                }
                Kind::Empty => z.start_file(part.as_str(), stored).ok()?,
            }
        }
        for (part, own) in &rels {
            let kept: Vec<Rel> = own.iter().filter(|r| r.external || r.to.as_ref().is_some_and(|t| parts.contains_key(t))).cloned().collect();
            z.start_file(rels_path(part).as_str(), deflated).ok()?;
            z.write_all(&rels_xml(&kept)).ok()?;
        }
        let zip = z.finish().ok()?.into_inner();
        Some(Batch { zip, xml, parts: n })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// Checked XML, copied as it is.
    Xml,
    /// Media and other binary parts: an empty entry (the parser only needs
    /// it to exist).
    Empty,
}

struct Batch {
    zip: Vec<u8>,
    /// The batch's own XML (shared design parts left out), unpacked.
    xml: u64,
    parts: usize,
}

fn is_xml(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".xml") || lower.ends_with(".rels") || lower.ends_with(".vml")
}

/// `part`'s relationships part: `ppt/slides/_rels/slide1.xml.rels`.
fn rels_path(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None if part.is_empty() => "_rels/.rels".into(),
        None => format!("_rels/{part}.rels"),
    }
}

/// `part`'s relationships, read from the file; none when they are missing
/// or broken.
fn read_rels(src: &MediaSource, part: &str) -> Vec<Rel> {
    let Some(bytes) = src.read_part(&rels_path(part), max_xml()) else { return vec![] };
    parse_rels(&bytes, part).unwrap_or_default()
}

/// The relationships in a `.rels` part whose source is `part`; None when
/// it isn't well formed.
fn parse_rels(bytes: &[u8], part: &str) -> Option<Vec<Rel>> {
    if !check(bytes) {
        return None;
    }
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let decoder = reader.decoder();
    let mut out = vec![];
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"Relationship" => {
                let get = |name: &[u8]| {
                    e.attributes().filter_map(Result::ok).find(|a| a.key.as_ref() == name).and_then(|a| a.decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder).ok()).map(|v| v.into_owned())
                };
                let id = get(b"Id").unwrap_or_default();
                let kind = get(b"Type").unwrap_or_default();
                let target = get(b"Target").unwrap_or_default();
                let external = get(b"TargetMode").as_deref() == Some("External");
                let to = if external { None } else { resolve(part, &target) };
                // An inside link the parser would refuse (outside the
                // package, a web address) is left out.
                if !external && to.is_none() {
                    continue;
                }
                out.push(Rel { id, kind, target, external, to });
            }
            Event::Eof => return Some(out),
            _ => {}
        }
    }
}

/// An inside relationship's target, resolved against its source part as
/// BetterOffice resolves it; None where BetterOffice would refuse the
/// whole package (a target outside it, or a web or data address).
fn resolve(source: &str, target: &str) -> Option<String> {
    if target.contains("://") || target.starts_with("mailto:") || target.starts_with("data:") {
        return None;
    }
    let target = target.split('#').next().unwrap_or_default().replace('\\', "/");
    let mut segments: Vec<&str> = vec![];
    if let (false, Some((dir, _))) = (target.starts_with('/'), source.rsplit_once('/')) {
        segments.extend(dir.split('/').filter(|s| !s.is_empty()));
    }
    for s in target.trim_start_matches('/').split('/') {
        match s {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            s => segments.push(s),
        }
    }
    (!segments.is_empty()).then(|| segments.join("/"))
}

/// A `.rels` part holding `rels`.
fn rels_xml(rels: &[Rel]) -> Vec<u8> {
    use quick_xml::escape::escape;
    let mut s = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"{RELS_NS}\">");
    for r in rels {
        s += &format!("<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"", escape(&r.id), escape(&r.kind), escape(&r.target));
        if r.external {
            s += " TargetMode=\"External\"";
        }
        s += "/>";
    }
    s += "</Relationships>";
    s.into_bytes()
}

/// Content types for a file whose own are missing or broken.
fn default_content_types() -> Vec<u8> {
    br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Default Extension="jpg" ContentType="image/jpeg"/><Default Extension="gif" ContentType="image/gif"/><Default Extension="svg" ContentType="image/svg+xml"/><Default Extension="emf" ContentType="image/x-emf"/><Default Extension="wmf" ContentType="image/x-wmf"/><Default Extension="mp4" ContentType="video/mp4"/><Default Extension="m4a" ContentType="audio/mp4"/><Default Extension="mp3" ContentType="audio/mpeg"/><Default Extension="wav" ContentType="audio/wav"/></Types>"#
        .to_vec()
}

/// Is this a well-formed XML document BetterOffice can read (one root,
/// every element closed, at most [`MAX_DEPTH`] deep)?
pub(crate) fn check(bytes: &[u8]) -> bool {
    let mut reader = quick_xml::Reader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    let (mut depth, mut roots) = (0usize, 0usize);
    loop {
        match reader.read_event() {
            Err(_) => return false,
            Ok(Event::Start(_)) => {
                if depth == 0 {
                    roots += 1;
                }
                depth += 1;
                if depth > MAX_DEPTH {
                    return false;
                }
            }
            Ok(Event::Empty(_)) => {
                if depth == 0 {
                    roots += 1;
                }
            }
            Ok(Event::End(_)) => depth = depth.saturating_sub(1),
            Ok(Event::DocType(_)) => return false,
            Ok(Event::Eof) => return depth == 0 && roots == 1,
            Ok(_) => {}
        }
    }
}

/// The relationship id of each `item` in the presentation's `list`
/// (`sldIdLst`/`sldId`), in order; None when the XML is broken.
fn id_list(xml: &[u8], list: &str, item: &str) -> Option<Vec<Option<String>>> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let (mut inside, mut depth, mut out) = (false, 0usize, vec![]);
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => {
                depth += 1;
                if e.local_name().as_ref() == list.as_bytes() && depth == 2 {
                    inside = true;
                } else if inside && depth == 3 && e.local_name().as_ref() == item.as_bytes() {
                    out.push(rel_id(&e, reader.decoder()));
                }
            }
            Event::Empty(e) => {
                if inside && depth == 2 && e.local_name().as_ref() == item.as_bytes() {
                    out.push(rel_id(&e, reader.decoder()));
                }
            }
            Event::End(e) => {
                if inside && depth == 2 && e.local_name().as_ref() == list.as_bytes() {
                    inside = false;
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => return Some(out),
            _ => {}
        }
    }
}

/// An `sldId`'s or `sldMasterId`'s relationship id (`r:id`, whatever the
/// prefix).
fn rel_id(e: &quick_xml::events::BytesStart, decoder: quick_xml::encoding::Decoder) -> Option<String> {
    let attrs: Vec<_> = e.attributes().filter_map(Result::ok).collect();
    let id = attrs.iter().find(|a| a.key.as_ref() == b"r:id").or_else(|| attrs.iter().find(|a| a.key.prefix().is_some() && a.key.local_name().as_ref() == b"id"))?;
    id.decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder).ok().map(|v| v.into_owned())
}

/// `presentation.xml` listing only the slides at places `keep` (0-based,
/// in `sldIdLst`) and the masters whose relationships are in `masters`.
fn filter_presentation(xml: &[u8], keep: &BTreeSet<usize>, masters: &BTreeSet<String>) -> Option<Vec<u8>> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut w = quick_xml::Writer::new(Vec::with_capacity(xml.len()));
    let (mut depth, mut slide, mut skip_to) = (0usize, 0usize, None::<usize>);
    loop {
        let ev = reader.read_event().ok()?;
        if let Event::Eof = ev {
            return Some(w.into_inner());
        }
        // Inside an element being left out: drop everything to its end.
        if let Some(d) = skip_to {
            match &ev {
                Event::Start(_) => depth += 1,
                Event::End(_) => {
                    depth -= 1;
                    if depth == d {
                        skip_to = None;
                    }
                }
                _ => {}
            }
            continue;
        }
        let drop_it = match &ev {
            Event::Start(e) | Event::Empty(e) if depth == 2 => match e.local_name().as_ref() {
                b"sldId" => {
                    slide += 1;
                    !keep.contains(&(slide - 1))
                }
                b"sldMasterId" => !rel_id(e, reader.decoder()).is_some_and(|id| masters.contains(&id)),
                _ => false,
            },
            _ => false,
        };
        match &ev {
            Event::Start(_) => {
                if drop_it {
                    skip_to = Some(depth);
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        if !drop_it {
            w.write_event(ev).ok()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_resolve_as_the_parser_resolves_them() {
        assert_eq!(resolve("ppt/slides/slide1.xml", "../slideLayouts/slideLayout1.xml").as_deref(), Some("ppt/slideLayouts/slideLayout1.xml"));
        assert_eq!(resolve("ppt/presentation.xml", "slides/slide1.xml").as_deref(), Some("ppt/slides/slide1.xml"));
        assert_eq!(resolve("ppt/presentation.xml", "/ppt/slides/slide1.xml#x").as_deref(), Some("ppt/slides/slide1.xml"));
        assert_eq!(resolve("", "ppt/presentation.xml").as_deref(), Some("ppt/presentation.xml"));
        for bad in ["../../../../../../etc/passwd", "https://x.example/a.png", "mailto:a@b", "data:image/png;base64,AA", "../../.."] {
            assert_eq!(resolve("ppt/slides/slide1.xml", bad), None, "{bad}");
        }
        assert_eq!(rels_path("ppt/slides/slide1.xml"), "ppt/slides/_rels/slide1.xml.rels");
        assert_eq!(rels_path(""), "_rels/.rels");
    }

    #[test]
    fn well_formed_means_one_closed_root() {
        assert!(check(b"<?xml version=\"1.0\"?><a><b/><c>t</c></a>"));
        assert!(!check(b""));
        assert!(!check(b"<a>"));
        assert!(!check(b"<a></b>"));
        assert!(!check(b"<a/><b/>"));
        assert!(!check(b"<!DOCTYPE a [<!ENTITY x \"y\">]><a/>"));
        assert!(!check(&[b"<a>".repeat(300), b"</a>".repeat(300)].concat()));
    }

    #[test]
    fn rels_round_trip_and_bad_targets_are_dropped() {
        let xml = br#"<Relationships xmlns="x"><Relationship Id="rId1" Type="t/slideLayout" Target="../slideLayouts/a&amp;b.xml"/><Relationship Id="rId2" Type="t/image" Target="../../../../etc/passwd"/><Relationship Id="rId3" Type="t/hyperlink" Target="https://e.x/?a=1&amp;b=2" TargetMode="External"/></Relationships>"#;
        let rels = parse_rels(xml, "ppt/slides/slide1.xml").unwrap();
        assert_eq!(rels.len(), 2);
        assert_eq!(rels[0].to.as_deref(), Some("ppt/slideLayouts/a&b.xml"));
        assert!(rels[1].external && rels[1].target == "https://e.x/?a=1&b=2");
        let again = parse_rels(&rels_xml(&rels), "ppt/slides/slide1.xml").unwrap();
        assert_eq!(again.iter().map(|r| (&r.id, &r.target)).collect::<Vec<_>>(), rels.iter().map(|r| (&r.id, &r.target)).collect::<Vec<_>>());
        assert!(parse_rels(b"<Relationships>", "x").is_none());
    }

    #[test]
    fn the_presentation_lists_only_the_batch() {
        let xml = br#"<?xml version="1.0"?><p:presentation xmlns:p="p" xmlns:r="r"><p:sldMasterIdLst><p:sldMasterId id="1" r:id="rId1"/><p:sldMasterId id="2" r:id="rId9"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"><p:extLst/></p:sldId><p:sldId id="258" r:id="rId4"/></p:sldIdLst><p:sldSz cx="1" cy="1"/></p:presentation>"#;
        assert_eq!(id_list(xml, "sldIdLst", "sldId").unwrap(), [Some("rId2".to_string()), Some("rId3".into()), Some("rId4".into())]);
        let out = filter_presentation(xml, &BTreeSet::from([1]), &BTreeSet::from(["rId1".to_string()])).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(check(out.as_bytes()), "{out}");
        assert!(out.contains("rId3") && out.contains("rId1") && out.contains("sldSz"), "{out}");
        assert!(!out.contains("rId2") && !out.contains("rId4") && !out.contains("rId9"), "{out}");
        assert_eq!(id_list(out.as_bytes(), "sldIdLst", "sldId").unwrap(), [Some("rId3".to_string())]);
    }
}
