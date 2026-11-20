//! What the table styles look like in a theme, and the contrast rule.
//!
//! A style names theme slots (or mixes of a slot with the background),
//! never fixed colours, so a theme change recolours the table. TS10 Dark is
//! the one exception: it keeps its own dark fill whatever the theme.
//!
//! Contrast (WCAG ratios): text needs 4.5:1 against the fill it sits on
//! in every style, 7:1 in the default style (TS4). When a style's text
//! colour falls short in a theme, the first of the theme's title colour,
//! background colour, near-black and near-white that passes is used (the
//! best of them when none does). Colours a person sets on cells are kept
//! as they are. The renderer and the test that checks every style in
//! every built-in theme both go through here.
use crate::table::{Role, TableData, TableStyle};
use crate::theme::Theme;

pub type Rgba = [u8; 4];

/// The contrast every style's text meets.
pub const MIN_TEXT: f64 = 4.5;
/// The contrast the default style's text meets.
pub const DEFAULT_TEXT: f64 = 7.0;
/// Marks drawn on a cell (the error triangle, tick boxes).
pub const MIN_MARK: f64 = 3.0;

pub const NEAR_BLACK: Rgba = [0x14, 0x14, 0x14, 0xff];
pub const NEAR_WHITE: Rgba = [0xfa, 0xfa, 0xfa, 0xff];

/// Relative luminance (WCAG 2).
pub fn luminance(c: Rgba) -> f64 {
    let ch = |v: u8| {
        let s = v as f64 / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
}

/// Contrast ratio of two opaque colours, 1 to 21.
pub fn contrast(a: Rgba, b: Rgba) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `a` over `b` by `a`'s alpha.
pub fn over(a: Rgba, b: Rgba) -> Rgba {
    let k = a[3] as f64 / 255.0;
    let m = |x: u8, y: u8| (x as f64 * k + y as f64 * (1.0 - k)).round() as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2]), 255]
}

/// `t` of `a` with the rest `b`.
pub fn mix(a: Rgba, b: Rgba, t: f64) -> Rgba {
    let m = |x: u8, y: u8| (x as f64 * t + y as f64 * (1.0 - t)).round().clamp(0.0, 255.0) as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2]), 255]
}

/// A colour in a style: a theme slot, or a slot mixed into the background.
#[derive(Debug, Clone, Copy)]
enum Paint {
    Slot(&'static str),
    /// This much of the slot over the background.
    Tint(&'static str, f64),
}

impl Paint {
    fn rgba(self, th: &Theme) -> Rgba {
        let slot = |s: &str| th.color(&format!("theme:{s}"));
        match self {
            Paint::Slot(s) => slot(s),
            Paint::Tint(s, t) => mix(slot(s), slot("background"), t),
        }
    }
}

/// How the footer is set off from the body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FootRule {
    None,
    /// A line on top (colour, width in body-text ems).
    Line(Rgba, f64),
    /// Two thin lines (an accountant's total).
    Double(Rgba),
}

/// A style's colours in a theme. Widths are in ems of the table's text.
#[derive(Debug, Clone)]
pub struct Look {
    /// What's behind a cell without a fill: the theme background.
    pub canvas: Rgba,
    pub header_fill: Option<Rgba>,
    pub header_text: Rgba,
    pub header_column_fill: Option<Rgba>,
    pub body_fill: Option<Rgba>,
    pub band_fill: Option<Rgba>,
    pub footer_fill: Option<Rgba>,
    pub body_text: Rgba,
    /// Lines between body rows.
    pub row_rule: Option<Rgba>,
    /// Lines between columns.
    pub column_rule: Option<Rgba>,
    /// The line under the header rows.
    pub header_rule: Option<(Rgba, f64)>,
    pub footer_rule: FootRule,
    pub outline: Option<(Rgba, f64)>,
    /// The contrast text must reach.
    pub min_text: f64,
}

/// Thin and thick lines, in ems.
pub const THIN: f64 = 0.045;
pub const THICK: f64 = 0.1;

const DARK_HEAD: Rgba = [0x2b, 0x2b, 0x2b, 0xff];
const DARK_BODY: Rgba = [0x3a, 0x3a, 0x3a, 0xff];
const DARK_TEXT: Rgba = [0xee, 0xee, 0xee, 0xff];
const DARK_LINE: Rgba = [0x55, 0x55, 0x55, 0xff];

/// The style's colours in `th`, before the table's own settings
/// (banding, gridlines) and before contrast.
fn style_look(s: TableStyle, th: &Theme) -> Look {
    use Paint::*;
    let c = |p: Paint| p.rgba(th);
    let bg = c(Slot("background"));
    let mut l = Look {
        canvas: bg,
        header_fill: None,
        header_text: c(Slot("title")),
        header_column_fill: None,
        body_fill: None,
        band_fill: None,
        footer_fill: None,
        body_text: c(Slot("body")),
        row_rule: None,
        column_rule: None,
        header_rule: None,
        footer_rule: FootRule::Line(c(Slot("frame")), THICK),
        outline: None,
        min_text: MIN_TEXT,
    };
    match s {
        TableStyle::Banner => {
            l.header_fill = Some(c(Slot("frame")));
            l.header_text = bg;
            l.row_rule = Some(c(Tint("frame", 0.2)));
        }
        TableStyle::Banded => {
            l.header_fill = Some(c(Slot("frame")));
            l.header_text = bg;
            l.band_fill = Some(c(Tint("frame", 0.12)));
        }
        TableStyle::Grid => {
            l.header_fill = Some(c(Tint("frame", 0.12)));
            l.row_rule = Some(c(Tint("frame", 0.5)));
            l.column_rule = Some(c(Tint("frame", 0.5)));
            l.outline = Some((c(Tint("frame", 0.5)), THIN));
            l.header_rule = Some((c(Tint("frame", 0.5)), THIN));
        }
        TableStyle::Ruled => {
            // The default: header in the title colour (the spec's decision),
            // a strong rule under it, faint rules between rows.
            l.header_rule = Some((c(Slot("frame")), THICK));
            l.row_rule = Some(c(Tint("title", 0.12)));
            l.min_text = DEFAULT_TEXT;
        }
        TableStyle::Accent => {
            l.header_fill = Some(c(Slot("accent")));
            l.header_text = bg;
            l.row_rule = Some(c(Tint("accent", 0.25)));
            l.footer_rule = FootRule::Line(c(Slot("accent")), THICK);
        }
        TableStyle::SideHeader => {
            l.header_fill = Some(c(Slot("frame")));
            l.header_text = bg;
            l.header_column_fill = Some(c(Tint("frame", 0.12)));
            l.row_rule = Some(c(Tint("frame", 0.2)));
        }
        TableStyle::Boxed => {
            l.header_fill = Some(c(Slot("frame")));
            l.header_text = bg;
            l.row_rule = Some(c(Tint("frame", 0.2)));
            l.outline = Some((c(Slot("frame")), THICK));
        }
        TableStyle::Soft => {
            l.header_fill = Some(c(Tint("frame", 0.25)));
            l.body_fill = Some(c(Tint("frame", 0.07)));
            l.footer_fill = l.header_fill;
            l.row_rule = Some(bg);
            l.footer_rule = FootRule::None;
        }
        TableStyle::Dark => {
            l.header_fill = Some(DARK_HEAD);
            l.header_text = DARK_TEXT;
            l.body_fill = Some(DARK_BODY);
            l.body_text = DARK_TEXT;
            l.row_rule = Some(DARK_LINE);
            l.footer_rule = FootRule::Line(DARK_HEAD, THICK);
            l.outline = Some((DARK_HEAD, THIN));
            l.canvas = DARK_BODY;
        }
        TableStyle::Ledger => {
            l.header_rule = Some((c(Slot("title")), THIN));
            l.band_fill = Some(c(Tint("title", 0.06)));
            l.footer_rule = FootRule::Double(c(Slot("title")));
        }
    }
    l
}

/// The look of `t` (its style, banding and gridline settings) in `th`.
pub fn look(t: &TableData, th: &Theme) -> Look {
    let mut l = style_look(t.style, th);
    let soft_line = || mix(th.color(&"theme:frame".to_string()), l.canvas, 0.35);
    match t.banded {
        Some(false) => l.band_fill = None,
        Some(true) if l.band_fill.is_none() => l.band_fill = Some(mix(th.color(&"theme:frame".to_string()), l.body_fill.unwrap_or(l.canvas), 0.1)),
        _ => {}
    }
    if let Some(g) = t.gridlines {
        let line = l.row_rule.or(l.column_rule).unwrap_or_else(soft_line);
        l.row_rule = g.horizontal.then_some(l.row_rule.unwrap_or(line));
        l.column_rule = g.vertical.then_some(l.column_rule.unwrap_or(line));
        l.outline = if g.outline { Some(l.outline.unwrap_or((line, THIN))) } else { None };
    }
    l
}

/// The fill a cell sits on in the style (before any fill set on it), if any.
pub fn role_fill(l: &Look, role: Role) -> Option<Rgba> {
    match role {
        Role::Header => l.header_fill,
        Role::Footer => l.footer_fill.or(l.body_fill),
        Role::HeaderColumn => l.header_column_fill.or(l.body_fill),
        Role::Body { band: true } => l.band_fill.or(l.body_fill),
        Role::Body { band: false } => l.body_fill,
    }
}

/// The text colour a style wants for a role.
pub fn role_text(l: &Look, role: Role) -> Rgba {
    match role {
        Role::Header => l.header_text,
        _ => l.body_text,
    }
}

/// `want` if it reads on `under` at `ratio`; else the first of the theme's
/// title colour, background, near-black and near-white that does; else the
/// one that reads best.
pub fn readable(want: Rgba, under: Rgba, ratio: f64, th: &Theme) -> Rgba {
    if contrast(want, under) >= ratio {
        return want;
    }
    let cands = [th.color(&"theme:title".to_string()), th.color(&"theme:background".to_string()), NEAR_BLACK, NEAR_WHITE];
    cands
        .iter()
        .copied()
        .find(|c| contrast(*c, under) >= ratio)
        .unwrap_or_else(|| cands.into_iter().max_by(|a, b| contrast(*a, under).total_cmp(&contrast(*b, under))).unwrap_or(NEAR_BLACK))
}

/// The same hue made darker or lighter until it reads on `under` at
/// `ratio` (red for negatives, the error triangle).
pub fn adjust_hue(c: Rgba, under: Rgba, ratio: f64) -> Rgba {
    if contrast(c, under) >= ratio {
        return c;
    }
    // Darker or lighter, whichever gets there sooner.
    for i in 1..=20 {
        for toward in [[0, 0, 0, 255], [255, 255, 255, 255]] {
            let m = mix(toward, c, i as f64 / 20.0);
            if contrast(m, under) >= ratio {
                return m;
            }
        }
    }
    if contrast([0, 0, 0, 255], under) > contrast([255, 255, 255, 255], under) { [0, 0, 0, 255] } else { [255, 255, 255, 255] }
}

/// Red for negative numbers and errors.
pub const RED: Rgba = [0xd0, 0x31, 0x2d, 0xff];

/// The colours of one cell as drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellPaint {
    /// The cell's fill, if it has one (style or its own).
    pub fill: Option<Rgba>,
    /// What the text sits on (the fill, else the canvas).
    pub under: Rgba,
    pub text: Rgba,
    /// Negative numbers in red, and errors.
    pub red: Rgba,
    /// The error triangle.
    pub mark: Rgba,
}

/// Colours of cell (r, c) of `t` in `th` with look `l`.
pub fn cell_paint(t: &TableData, th: &Theme, l: &Look, r: usize, c: usize) -> CellPaint {
    let role = t.role(r, c);
    let f = t.format_at(r, c);
    let own_fill = f.fill.as_ref().filter(|s| !s.is_empty()).map(|s| th.color(s));
    let mut fill = own_fill.or(role_fill(l, role));
    let mut under = fill.map(|x| over(x, l.canvas)).unwrap_or(l.canvas);
    let text = match f.color.as_ref().filter(|s| !s.is_empty()) {
        // A colour a person chose is theirs.
        Some(s) => th.color(s),
        None => {
            let text = readable(role_text(l, role), under, l.min_text, th);
            // No text colour reads on the style's own fill (white on
            // Classic's blue is 4.49:1): the fill's hue goes a little
            // darker or lighter until it does, as marks do.
            if contrast(text, under) < l.min_text && own_fill.is_none() && fill.is_some() {
                let toward = if luminance(text) > luminance(under) { [0, 0, 0, 255] } else { [255, 255, 255, 255] };
                for i in 1..=20 {
                    let m = mix(toward, under, i as f64 / 40.0);
                    if contrast(text, m) >= l.min_text {
                        (fill, under) = (Some(m), m);
                        break;
                    }
                }
            }
            text
        }
    };
    CellPaint { fill, under, text, red: adjust_hue(RED, under, l.min_text), mark: adjust_hue(RED, under, MIN_MARK) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::Cell;

    /// The spec's table: header row, four body rows, a total row.
    fn sample() -> TableData {
        let mut t = TableData::new(6, 3, 24.0);
        t.footer_rows = 1;
        for (r, row) in [["Region", "Q2", "Q3"], ["North", "1,240", "1,410"], ["South", "980", "1,050"], ["East", "760", "702"], ["West", "1,105", "1,230"], ["Total", "=SUM(B2:B5)", "=SUM(C2:C5)"]].iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                t.cells[r][c] = Cell::new(*v);
            }
        }
        t
    }

    /// Every style in every built-in theme: each cell's text, red and
    /// error mark against what it sits on, with banding and a header
    /// column on so every role is drawn.
    #[test]
    fn every_style_in_every_theme_meets_its_contrast() {
        let mut failures = vec![];
        for th in Theme::builtins() {
            for s in TableStyle::ALL {
                for (banded, header_columns) in [(None, 0), (Some(true), 1)] {
                    let mut t = sample();
                    t.style = s;
                    t.banded = banded;
                    t.header_columns = header_columns;
                    let l = look(&t, &th);
                    let need = if s == TableStyle::Ruled { DEFAULT_TEXT } else { MIN_TEXT };
                    assert_eq!(l.min_text, need);
                    for r in 0..t.n_rows() {
                        for c in 0..t.n_cols() {
                            let p = cell_paint(&t, &th, &l, r, c);
                            for (what, col, ratio) in [("text", p.text, need), ("negative", p.red, need), ("error mark", p.mark, MIN_MARK)] {
                                let got = contrast(col, p.under);
                                if got < ratio {
                                    failures.push(format!("{} {} {what} at {}: {got:.2} < {ratio}", th.id, s.id(), crate::table::cell_name(r, c)));
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn the_spec_fallbacks() {
        let by = |id: &str| Theme::builtins().into_iter().find(|t| t.id == id).unwrap();
        let mut t = sample();
        // Classic TS6: dark header text on the orange.
        t.style = TableStyle::Accent;
        let classic = by("classic");
        let p = cell_paint(&t, &classic, &look(&t, &classic), 0, 0);
        assert!(luminance(p.text) < 0.1, "{:?}", p.text);
        // Chalkboard TS1: header text takes the background colour.
        t.style = TableStyle::Banner;
        let chalk = by("chalkboard");
        let p = cell_paint(&t, &chalk, &look(&t, &chalk), 0, 0);
        assert_eq!(p.text, chalk.color(&"theme:background".to_string()));
        // Blueprint TS4 (7:1): body text takes the title colour.
        t.style = TableStyle::Ruled;
        let bp = by("blueprint");
        let p = cell_paint(&t, &bp, &look(&t, &bp), 1, 0);
        assert_eq!(p.text, bp.color(&"theme:title".to_string()));
        // Classic TS4: header in the title colour.
        let p = cell_paint(&t, &classic, &look(&t, &classic), 0, 0);
        assert_eq!(p.text, classic.color(&"theme:title".to_string()));
    }

    #[test]
    fn a_chosen_text_colour_is_kept() {
        let mut t = sample();
        t.add_format(crate::table::CellRange::cell(1, 0), crate::table::CellFormat { color: Some("#ffffff".into()), ..Default::default() });
        let th = Theme::resolve("classic");
        assert_eq!(cell_paint(&t, &th, &look(&t, &th), 1, 0).text, [255, 255, 255, 255]);
    }
}
