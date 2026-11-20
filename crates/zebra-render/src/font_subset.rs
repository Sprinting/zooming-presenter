//! Fonts cut down to the glyphs a page uses, for exports that carry fonts
//! (the HTML player). Shipping a whole system font is both huge (Apple
//! Color Emoji is about 180 MB) and not allowed; shipping the glyphs a
//! document shows is what PDF and print embedding do too.
//!
//! The cut keeps glyph ids, `cmap` and the shaping tables, so the page
//! lays text out exactly as the app does; only the drawings of glyphs the
//! document doesn't use are left out. Two kinds of drawings are cut:
//! colour bitmaps (`sbix`), TrueType outlines (`glyf`/`loca`, with the
//! layers of COLR v0 colour glyphs, and a variable font's `gvar` with
//! them: the cut keeps every weight) and CFF outlines (charstrings; shared
//! subroutines stay). Other drawings (`CFF2`, `CBDT` bitmaps, COLR v1
//! outlines) come back whole, in a one-face file.
use std::collections::BTreeSet;

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

struct Table<'a> {
    tag: [u8; 4],
    data: &'a [u8],
}

/// The tables of face `index` (of a collection, or the only face).
fn tables(file: &[u8], index: u32) -> Option<(u32, Vec<Table<'_>>)> {
    let face = if file.get(0..4)? == b"ttcf" {
        let n = u32_at(file, 8)?;
        if index >= n {
            return None;
        }
        u32_at(file, 12 + 4 * index as usize)? as usize
    } else {
        0
    };
    let version = u32_at(file, face)?;
    let n = u16_at(file, face + 4)? as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let r = face + 12 + 16 * i;
        let tag: [u8; 4] = file.get(r..r + 4)?.try_into().ok()?;
        let off = u32_at(file, r + 8)? as usize;
        let len = u32_at(file, r + 12)? as usize;
        out.push(Table { tag, data: file.get(off..off.checked_add(len)?)? });
    }
    Some((version, out))
}

fn checksum(d: &[u8]) -> u32 {
    d.chunks(4).fold(0u32, |s, c| {
        let mut b = [0u8; 4];
        b[..c.len()].copy_from_slice(c);
        s.wrapping_add(u32::from_be_bytes(b))
    })
}

/// One face of `file`, with glyph drawings other than `keep` left out.
/// None when the file can't be read (then ship it as it is, or not at all).
pub fn subset(file: &[u8], index: u32, keep: &BTreeSet<u16>) -> Option<Vec<u8>> {
    let (version, tables) = tables(file, index)?;
    let find = |t: &[u8; 4]| tables.iter().find(|x| &x.tag == t).map(|x| x.data);
    let num_glyphs = u16_at(find(b"maxp")?, 4)? as usize;
    let long_loca = find(b"head").and_then(|h| u16_at(h, 50)).map(|f| f == 1);
    // Colour glyphs drawn from layers (COLR v0) need their layer glyphs;
    // COLR v1 paint graphs aren't followed, so their outlines stay whole.
    let mut keep = keep.clone();
    let mut cut_outlines = true;
    if let Some(colr) = find(b"COLR") {
        match colr_layers(colr, &keep) {
            Some(layers) => keep.extend(layers),
            None => cut_outlines = false,
        }
    }
    let keep = &keep;
    let mut glyf_loca: Option<(Vec<u8>, Vec<u8>, BTreeSet<u16>)> = None;
    let mut cff: Option<Vec<u8>> = None;
    if cut_outlines {
        if let (Some(glyf), Some(loca), Some(long)) = (find(b"glyf"), find(b"loca"), long_loca) {
            glyf_loca = cut_glyf(glyf, loca, long, num_glyphs, keep);
        }
        if let Some(c) = find(b"CFF ") {
            cff = cut_cff(c, num_glyphs, keep);
        }
    }
    // A variable font: its outline variations go with the outlines.
    let gvar = match (&glyf_loca, find(b"gvar")) {
        (Some((_, _, want)), Some(g)) => cut_gvar(g, want),
        _ => None,
    };
    let mut out_tables: Vec<([u8; 4], Vec<u8>)> = vec![];
    for t in &tables {
        let data = match &t.tag {
            b"sbix" => cut_sbix(t.data, num_glyphs, keep)?,
            b"glyf" if glyf_loca.is_some() => glyf_loca.as_ref()?.0.clone(),
            b"loca" if glyf_loca.is_some() => glyf_loca.as_ref()?.1.clone(),
            b"gvar" if gvar.is_some() => gvar.clone()?,
            b"CFF " if cff.is_some() => cff.clone()?,
            // DSIG would no longer match; drop it.
            b"DSIG" => continue,
            _ => t.data.to_vec(),
        };
        out_tables.push((t.tag, data));
    }
    out_tables.sort_by_key(|(t, _)| *t);
    // Write an sfnt: header, records, then each table padded to 4 bytes.
    let n = out_tables.len();
    let mut es = 0u16;
    while (1usize << (es + 1)) <= n {
        es += 1;
    }
    let sr = (1u16 << es) * 16;
    let mut out = vec![];
    out.extend(version.to_be_bytes());
    out.extend((n as u16).to_be_bytes());
    out.extend(sr.to_be_bytes());
    out.extend(es.to_be_bytes());
    out.extend((n as u16 * 16 - sr).to_be_bytes());
    let mut off = 12 + 16 * n;
    let mut body = vec![];
    for (tag, data) in &out_tables {
        out.extend(tag);
        out.extend(checksum(data).to_be_bytes());
        out.extend((off as u32).to_be_bytes());
        out.extend((data.len() as u32).to_be_bytes());
        body.extend(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        off = 12 + 16 * n + body.len();
    }
    out.extend(body);
    Some(out)
}

/// `sbix` keeping every strike but only the bitmaps of `keep` (and the
/// glyphs their `dupe` records point at).
fn cut_sbix(sbix: &[u8], num_glyphs: usize, keep: &BTreeSet<u16>) -> Option<Vec<u8>> {
    let strikes = u32_at(sbix, 4)? as usize;
    let mut out = vec![];
    out.extend(sbix.get(0..8)?);
    let header = 8 + 4 * strikes;
    out.resize(header, 0);
    for s in 0..strikes {
        let so = u32_at(sbix, 8 + 4 * s)? as usize;
        let data_of = |g: usize| -> Option<&[u8]> {
            let a = u32_at(sbix, so + 4 + 4 * g)? as usize;
            let b = u32_at(sbix, so + 4 + 4 * (g + 1))? as usize;
            sbix.get(so + a..so + b.max(a))
        };
        let mut want: BTreeSet<u16> = keep.iter().copied().filter(|g| (*g as usize) < num_glyphs).collect();
        for g in keep {
            if let Some(d) = data_of(*g as usize) {
                if d.get(4..8) == Some(b"dupe") {
                    if let Some(t) = u16_at(d, 8) {
                        want.insert(t);
                    }
                }
            }
        }
        let start = out.len();
        out[8 + 4 * s..12 + 4 * s].copy_from_slice(&(start as u32).to_be_bytes());
        out.extend(sbix.get(so..so + 4)?); // ppem, ppi
        let offsets_at = out.len();
        out.resize(offsets_at + 4 * (num_glyphs + 1), 0);
        let mut pos = 4 + 4 * (num_glyphs + 1);
        for g in 0..num_glyphs {
            out[offsets_at + 4 * g..offsets_at + 4 * g + 4].copy_from_slice(&(pos as u32).to_be_bytes());
            if want.contains(&(g as u16)) {
                if let Some(d) = data_of(g) {
                    out.extend(d);
                    pos += d.len();
                }
            }
        }
        out[offsets_at + 4 * num_glyphs..offsets_at + 4 * num_glyphs + 4].copy_from_slice(&(pos as u32).to_be_bytes());
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    Some(out)
}

/// `glyf` and `loca` keeping only the outlines of `keep` and the parts of
/// composite glyphs they use; every other glyph becomes empty.
fn cut_glyf(glyf: &[u8], loca: &[u8], long: bool, num_glyphs: usize, keep: &BTreeSet<u16>) -> Option<(Vec<u8>, Vec<u8>, BTreeSet<u16>)> {
    let at = |g: usize| -> Option<usize> {
        if long {
            u32_at(loca, 4 * g).map(|v| v as usize)
        } else {
            u16_at(loca, 2 * g).map(|v| v as usize * 2)
        }
    };
    let data = |g: usize| -> Option<&[u8]> {
        let (a, b) = (at(g)?, at(g + 1)?);
        glyf.get(a..b.max(a))
    };
    // Composite glyphs pull in their components.
    let mut want: BTreeSet<u16> = BTreeSet::new();
    let mut todo: Vec<u16> = keep.iter().copied().chain([0]).collect();
    while let Some(g) = todo.pop() {
        if (g as usize) >= num_glyphs || !want.insert(g) {
            continue;
        }
        let Some(d) = data(g as usize) else { continue };
        if d.len() >= 10 && (u16_at(d, 0)? as i16) < 0 {
            let mut o = 10;
            loop {
                let flags = u16_at(d, o)?;
                todo.push(u16_at(d, o + 2)?);
                o += 4 + if flags & 1 != 0 { 4 } else { 2 };
                o += if flags & 8 != 0 { 2 } else if flags & 0x40 != 0 { 4 } else if flags & 0x80 != 0 { 8 } else { 0 };
                if flags & 0x20 == 0 {
                    break;
                }
            }
        }
    }
    let mut new_glyf = vec![];
    let mut new_loca = vec![];
    let put = |v: usize, loca: &mut Vec<u8>| {
        if long {
            loca.extend((v as u32).to_be_bytes());
        } else {
            loca.extend(((v / 2) as u16).to_be_bytes());
        }
    };
    for g in 0..num_glyphs {
        put(new_glyf.len(), &mut new_loca);
        if want.contains(&(g as u16)) {
            new_glyf.extend(data(g)?);
            while new_glyf.len() % 4 != 0 {
                new_glyf.push(0);
            }
        }
    }
    put(new_glyf.len(), &mut new_loca);
    Some((new_glyf, new_loca, want))
}

/// `gvar` (a variable TrueType font's outline variations) keeping only
/// the variations of `want`, the glyphs left in `glyf`. The header, the
/// shared tuples and the offset format stay as they were.
fn cut_gvar(gvar: &[u8], want: &BTreeSet<u16>) -> Option<Vec<u8>> {
    let shared_at = u32_at(gvar, 8)? as usize;
    let count = u16_at(gvar, 12)? as usize;
    let long = u16_at(gvar, 14)? & 1 != 0;
    let data_at = u32_at(gvar, 16)? as usize;
    let offsets_at = 20;
    let offsets_len = (count + 1) * if long { 4 } else { 2 };
    // Shared tuples after the variation data would move; leave such a
    // file whole.
    if data_at < offsets_at + offsets_len || shared_at > data_at {
        return None;
    }
    let off = |g: usize| -> Option<usize> {
        if long { u32_at(gvar, offsets_at + 4 * g).map(|v| v as usize) } else { u16_at(gvar, offsets_at + 2 * g).map(|v| v as usize * 2) }
    };
    let mut out = gvar.get(..data_at)?.to_vec();
    let mut body: Vec<u8> = vec![];
    let put = |g: usize, v: usize, out: &mut Vec<u8>| {
        if long {
            out[offsets_at + 4 * g..offsets_at + 4 * g + 4].copy_from_slice(&(v as u32).to_be_bytes());
        } else {
            out[offsets_at + 2 * g..offsets_at + 2 * g + 2].copy_from_slice(&((v / 2) as u16).to_be_bytes());
        }
    };
    for g in 0..count {
        put(g, body.len(), &mut out);
        if want.contains(&(g as u16)) {
            let (a, b) = (off(g)?, off(g + 1)?);
            body.extend(gvar.get(data_at + a..data_at + b.max(a))?);
            if !long && body.len() % 2 != 0 {
                body.push(0);
            }
        }
    }
    put(count, body.len(), &mut out);
    out.extend(body);
    Some(out)
}

/// COLR v0: the layer glyphs of `keep`'s colour glyphs. None for other
/// versions (their outlines are then kept whole).
fn colr_layers(colr: &[u8], keep: &BTreeSet<u16>) -> Option<Vec<u16>> {
    if u16_at(colr, 0)? != 0 {
        return None;
    }
    let n = u16_at(colr, 2)? as usize;
    let bases = u32_at(colr, 4)? as usize;
    let layers = u32_at(colr, 8)? as usize;
    let mut out = vec![];
    for i in 0..n {
        let r = bases + 6 * i;
        if keep.contains(&u16_at(colr, r)?) {
            let (first, count) = (u16_at(colr, r + 2)? as usize, u16_at(colr, r + 4)? as usize);
            for l in first..first + count {
                out.push(u16_at(colr, layers + 4 * l)?);
            }
        }
    }
    Some(out)
}

/// A CFF INDEX at `off`: its items' byte ranges and where it ends.
fn cff_index(d: &[u8], off: usize) -> Option<(Vec<std::ops::Range<usize>>, usize)> {
    let count = u16_at(d, off)? as usize;
    if count == 0 {
        return Some((vec![], off + 2));
    }
    let os = *d.get(off + 2)? as usize;
    if !(1..=4).contains(&os) {
        return None;
    }
    let read = |i: usize| -> Option<usize> {
        let p = off + 3 + i * os;
        let b = d.get(p..p + os)?;
        Some(b.iter().fold(0usize, |v, x| (v << 8) | *x as usize))
    };
    let base = off + 3 + (count + 1) * os - 1;
    let mut items = Vec::with_capacity(count);
    for i in 0..count {
        let (a, b) = (read(i)?, read(i + 1)?);
        if b < a || base + b > d.len() {
            return None;
        }
        items.push(base + a..base + b);
    }
    Some((items, base + read(count)?))
}

fn write_index(items: &[&[u8]]) -> Vec<u8> {
    let mut out = (items.len() as u16).to_be_bytes().to_vec();
    if items.is_empty() {
        return out;
    }
    let total: usize = items.iter().map(|i| i.len()).sum::<usize>() + 1;
    let os = if total < 1 << 8 { 1 } else if total < 1 << 16 { 2 } else if total < 1 << 24 { 3 } else { 4 };
    out.push(os as u8);
    let mut at = 1usize;
    let put = |v: usize, out: &mut Vec<u8>| out.extend(&(v as u32).to_be_bytes()[4 - os..]);
    put(at, &mut out);
    for i in items {
        at += i.len();
        put(at, &mut out);
    }
    for i in items {
        out.extend(*i);
    }
    out
}

#[derive(Clone)]
enum Operand {
    Int(i32),
    /// A real number, as its encoded bytes (after the 30 prefix).
    Real(Vec<u8>),
}

type Dict = Vec<(u16, Vec<Operand>)>;

fn parse_dict(d: &[u8]) -> Option<Dict> {
    let mut out = vec![];
    let mut ops = vec![];
    let mut i = 0;
    while i < d.len() {
        let b = d[i];
        match b {
            0..=21 => {
                let op = if b == 12 {
                    i += 1;
                    1200 + *d.get(i)? as u16
                } else {
                    b as u16
                };
                out.push((op, std::mem::take(&mut ops)));
                i += 1;
            }
            28 => {
                ops.push(Operand::Int(i16::from_be_bytes([*d.get(i + 1)?, *d.get(i + 2)?]) as i32));
                i += 3;
            }
            29 => {
                ops.push(Operand::Int(i32::from_be_bytes(d.get(i + 1..i + 5)?.try_into().ok()?)));
                i += 5;
            }
            30 => {
                let start = i + 1;
                i += 1;
                loop {
                    let n = *d.get(i)?;
                    i += 1;
                    if n & 0x0f == 0x0f || n >> 4 == 0x0f {
                        break;
                    }
                }
                ops.push(Operand::Real(d[start..i].to_vec()));
            }
            32..=246 => {
                ops.push(Operand::Int(b as i32 - 139));
                i += 1;
            }
            247..=250 => {
                ops.push(Operand::Int((b as i32 - 247) * 256 + *d.get(i + 1)? as i32 + 108));
                i += 2;
            }
            251..=254 => {
                ops.push(Operand::Int(-(b as i32 - 251) * 256 - *d.get(i + 1)? as i32 - 108));
                i += 2;
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Every integer as 5 bytes, so a dict's size doesn't depend on the
/// offsets in it.
fn write_dict(dict: &Dict) -> Vec<u8> {
    let mut out = vec![];
    for (op, operands) in dict {
        for o in operands {
            match o {
                Operand::Int(v) => {
                    out.push(29);
                    out.extend(v.to_be_bytes());
                }
                Operand::Real(r) => {
                    out.push(30);
                    out.extend(r);
                }
            }
        }
        if *op >= 1200 {
            out.extend([12, (*op - 1200) as u8]);
        } else {
            out.push(*op as u8);
        }
    }
    out
}

fn int_of(dict: &Dict, op: u16, i: usize) -> Option<usize> {
    let (_, o) = dict.iter().find(|(k, _)| *k == op)?;
    match o.get(i)? {
        Operand::Int(v) if *v >= 0 => Some(*v as usize),
        _ => None,
    }
}

fn set_ints(dict: &mut Dict, op: u16, vals: &[usize]) {
    let v: Vec<Operand> = vals.iter().map(|x| Operand::Int(*x as i32)).collect();
    match dict.iter_mut().find(|(k, _)| *k == op) {
        Some(e) => e.1 = v,
        None => dict.push((op, v)),
    }
}

/// A Private DICT (at `off`, `size` bytes) and its local subroutines,
/// rewritten to sit together: (dict bytes, subrs bytes).
fn private_part(cff: &[u8], off: usize, size: usize) -> Option<(Dict, Vec<u8>)> {
    let dict = parse_dict(cff.get(off..off + size)?)?;
    let subrs = match int_of(&dict, 19, 0) {
        Some(rel) => {
            let (_, end) = cff_index(cff, off + rel)?;
            cff.get(off + rel..end)?.to_vec()
        }
        None => vec![],
    };
    Some((dict, subrs))
}

fn place_private(out: &mut Vec<u8>, mut dict: Dict, subrs: &[u8]) -> (usize, usize) {
    let at = out.len();
    if !subrs.is_empty() {
        // Subrs offset is relative to the dict; with 5-byte ints the size is
        // known before the offset is.
        set_ints(&mut dict, 19, &[0]);
        let size = write_dict(&dict).len();
        set_ints(&mut dict, 19, &[size]);
    }
    let bytes = write_dict(&dict);
    let size = bytes.len();
    out.extend(bytes);
    out.extend(subrs);
    (size, at)
}

/// Length of a charset at `off` for `n` glyphs.
fn charset_len(d: &[u8], off: usize, n: usize) -> Option<usize> {
    let format = *d.get(off)?;
    let mut covered = 1usize;
    let mut p = off + 1;
    match format {
        0 => Some(1 + 2 * n.saturating_sub(1)),
        1 | 2 => {
            while covered < n {
                let left = if format == 1 { *d.get(p + 2)? as usize } else { u16_at(d, p + 2)? as usize };
                covered += left + 1;
                p += if format == 1 { 3 } else { 4 };
            }
            Some(p - off)
        }
        _ => None,
    }
}

/// Length of an FDSelect at `off` for `n` glyphs.
fn fdselect_len(d: &[u8], off: usize, n: usize) -> Option<usize> {
    match *d.get(off)? {
        0 => Some(1 + n),
        3 => Some(1 + 2 + 3 * u16_at(d, off + 1)? as usize + 2),
        _ => None,
    }
}

/// `CFF ` with the charstrings of glyphs outside `keep` replaced by an
/// empty glyph. The rest is rebuilt around it with fresh offsets.
fn cut_cff(cff: &[u8], num_glyphs: usize, keep: &BTreeSet<u16>) -> Option<Vec<u8>> {
    let hdr = *cff.get(2)? as usize;
    let (names, names_end) = cff_index(cff, hdr)?;
    if names.len() != 1 {
        return None;
    }
    let (tops, tops_end) = cff_index(cff, names_end)?;
    let mut top = parse_dict(cff.get(tops.first()?.clone())?)?;
    let (_, strings_end) = cff_index(cff, tops_end)?;
    let (_, gsubrs_end) = cff_index(cff, strings_end)?;
    // A custom encoding (rare in OpenType) isn't moved: leave the font whole.
    if int_of(&top, 16, 0).is_some_and(|e| e > 1) {
        return None;
    }
    let cs_off = int_of(&top, 17, 0)?;
    let (charstrings, _) = cff_index(cff, cs_off)?;
    if charstrings.len() != num_glyphs {
        return None;
    }
    let charset = match int_of(&top, 15, 0) {
        Some(off) if off > 2 => Some(cff.get(off..off + charset_len(cff, off, num_glyphs)?)?),
        _ => None,
    };
    let cid = top.iter().any(|(k, _)| *k == 1230);
    let private = match int_of(&top, 18, 1) {
        Some(off) if !cid => Some(private_part(cff, off, int_of(&top, 18, 0)?)?),
        _ => None,
    };
    let (fdselect, fds) = if cid {
        let fs = int_of(&top, 1237, 0)?;
        let fdselect = cff.get(fs..fs + fdselect_len(cff, fs, num_glyphs)?)?;
        let (fd_items, _) = cff_index(cff, int_of(&top, 1236, 0)?)?;
        let mut fds = vec![];
        for r in fd_items {
            let fd = parse_dict(cff.get(r)?)?;
            let p = match (int_of(&fd, 18, 0), int_of(&fd, 18, 1)) {
                (Some(size), Some(off)) => Some(private_part(cff, off, size)?),
                _ => None,
            };
            fds.push((fd, p));
        }
        (Some(fdselect), fds)
    } else {
        (None, vec![])
    };
    // New charstrings: unused glyphs are a bare `endchar`.
    const EMPTY: &[u8] = &[14];
    let glyphs: Vec<&[u8]> =
        (0..num_glyphs).map(|g| if g == 0 || keep.contains(&(g as u16)) { &cff[charstrings[g].clone()] } else { EMPTY }).collect();
    let cs_index = write_index(&glyphs);

    // Layout: header, names, top dict, strings, global subrs (copied), then
    // charset, charstrings, FDSelect, private dicts, FDArray.
    for op in [15u16, 17, 1236, 1237] {
        if top.iter().any(|(k, _)| *k == op) {
            set_ints(&mut top, op, &[0]);
        }
    }
    if top.iter().any(|(k, _)| *k == 18) {
        set_ints(&mut top, 18, &[0, 0]);
    }
    let top_len = write_dict(&top).len();
    let pre_len = names_end + write_index(&[&vec![0u8; top_len]]).len() + (gsubrs_end - tops_end);
    let mut tail = vec![];
    let at = |tail: &Vec<u8>| pre_len + tail.len();
    if let Some(c) = charset {
        set_ints(&mut top, 15, &[at(&tail)]);
        tail.extend(c);
    }
    set_ints(&mut top, 17, &[at(&tail)]);
    tail.extend(&cs_index);
    if let Some(fs) = fdselect {
        set_ints(&mut top, 1237, &[at(&tail)]);
        tail.extend(fs);
    }
    if let Some((dict, subrs)) = private {
        let mut buf = vec![];
        let start = at(&tail);
        let (size, _) = place_private(&mut buf, dict, &subrs);
        set_ints(&mut top, 18, &[size, start]);
        tail.extend(buf);
    }
    if cid {
        let mut fd_dicts = vec![];
        for (mut fd, p) in fds {
            if let Some((dict, subrs)) = p {
                let mut buf = vec![];
                let start = at(&tail);
                let (size, _) = place_private(&mut buf, dict, &subrs);
                set_ints(&mut fd, 18, &[size, start]);
                tail.extend(buf);
            }
            fd_dicts.push(write_dict(&fd));
        }
        set_ints(&mut top, 1236, &[at(&tail)]);
        let refs: Vec<&[u8]> = fd_dicts.iter().map(|v| v.as_slice()).collect();
        tail.extend(write_index(&refs));
    }
    let top_bytes = write_dict(&top);
    debug_assert_eq!(top_bytes.len(), top_len);
    let mut out = cff.get(0..names_end)?.to_vec();
    out.extend(write_index(&[&top_bytes]));
    out.extend(cff.get(tops_end..gsubrs_end)?);
    if out.len() != pre_len {
        return None;
    }
    out.extend(tail);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_font_cut_keeps_used_glyphs_and_ids() {
        let file = crate::text::bundled_font_bytes_for_tests();
        let keep: BTreeSet<u16> = [36u16, 37, 38].into_iter().collect();
        let cut = subset(file, 0, &keep).expect("cut");
        assert!(cut.len() < file.len(), "{} >= {}", cut.len(), file.len());
        // Same glyph count and the kept glyphs still have outlines.
        use skrifa::raw::TableProvider;
        use skrifa::MetadataProvider;
        let a = skrifa::FontRef::from_index(file, 0).unwrap();
        let b = skrifa::FontRef::from_index(&cut, 0).unwrap();
        assert_eq!(a.maxp().unwrap().num_glyphs(), b.maxp().unwrap().num_glyphs());
        assert_eq!(a.charmap().map('A'), b.charmap().map('A'));
        let og = b.outline_glyphs();
        for g in &keep {
            assert!(og.get(skrifa::GlyphId::new(*g as u32)).is_some(), "glyph {g} lost");
        }
    }

    /// Inter is a variable font: its outline variations (`gvar`, most of
    /// the file) are cut with the outlines, and a kept glyph still draws
    /// the same at any weight.
    #[test]
    fn variable_font_cut_keeps_every_weight_of_the_used_glyphs() {
        use skrifa::instance::{LocationRef, Size};
        use skrifa::outline::{DrawSettings, OutlinePen};
        use skrifa::MetadataProvider;
        #[derive(Default)]
        struct P(String);
        impl OutlinePen for P {
            fn move_to(&mut self, x: f32, y: f32) { self.0 += &format!("M{x},{y}"); }
            fn line_to(&mut self, x: f32, y: f32) { self.0 += &format!("L{x},{y}"); }
            fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) { self.0 += &format!("Q{a},{b},{x},{y}"); }
            fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) { self.0 += &format!("C{a},{b},{c},{d},{x},{y}"); }
            fn close(&mut self) { self.0 += "Z"; }
        }
        let file = crate::text::bundled_font_bytes_for_tests();
        let a = skrifa::FontRef::from_index(file, 0).unwrap();
        let keep: BTreeSet<u16> = "Zebra".chars().filter_map(|c| a.charmap().map(c)).map(|g| g.to_u32() as u16).collect();
        let cut = subset(file, 0, &keep).expect("cut");
        assert!(cut.len() < file.len() / 2, "{} of {}", cut.len(), file.len());
        let b = skrifa::FontRef::from_index(&cut, 0).unwrap();
        for w in [100.0, 400.0, 650.0, 900.0] {
            let la = a.axes().location([("wght", w)]);
            let lb = b.axes().location([("wght", w)]);
            for g in &keep {
                let draw = |f: &skrifa::FontRef, loc: LocationRef| {
                    let mut p = P::default();
                    f.outline_glyphs().get(skrifa::GlyphId::new(*g as u32)).unwrap().draw(DrawSettings::unhinted(Size::new(100.0), loc), &mut p).unwrap();
                    p.0
                };
                assert_eq!(draw(&a, (&la).into()), draw(&b, (&lb).into()), "glyph {g} at wght {w}");
            }
        }
    }
}
