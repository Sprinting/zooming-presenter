//! PowerPoint tables and charts come in as Zebra tables and charts
//! (plan-21nov/28): fixtures from tests/fixtures/gen_pptx.py.
use zebra_core::chart::{ChartData, ChartType};
use zebra_core::import::{ImportNote, ImportedDeck};
use zebra_core::table::TableData;
use zebra_core::{Editor, ElementKind};
use zebra_import::{Control, Options};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn editor() -> Editor {
    let mut ed = Editor::default();
    ed.measure = zebra_render::Renderer::new().measure();
    ed
}

fn read(name: &str, ed: &Editor) -> ImportedDeck {
    let r = zebra_import::open_bytes(fixture(name), name.trim_end_matches(".pptx").into()).unwrap();
    r.read(&Options::all(ed.theme()), &Control::default()).unwrap()
}

fn tables(deck: &ImportedDeck, page: usize) -> Vec<&TableData> {
    deck.pages[page].items.iter().filter_map(|i| if let ElementKind::Table(t) = &i.element.kind { Some(t) } else { None }).collect()
}

fn charts(deck: &ImportedDeck, page: usize) -> Vec<&ChartData> {
    deck.pages[page].items.iter().filter_map(|i| if let ElementKind::Chart(c) = &i.element.kind { Some(c) } else { None }).collect()
}

fn inputs(t: &TableData) -> Vec<Vec<String>> {
    (0..t.n_rows()).map(|r| (0..t.n_cols()).map(|c| t.cells[r][c].input.clone()).collect()).collect()
}

#[test]
fn a_table_comes_in_editable_and_looking_as_it_did() {
    let ed = editor();
    let deck = read("tables_charts.pptx", &ed);
    let want = std::collections::BTreeMap::from([(ImportNote::ChartNearest, 1), (ImportNote::TableText, 1), (ImportNote::TableStyle, 1)]);
    assert_eq!(deck.notes, want);
    let t = tables(&deck, 0)[0];
    assert_eq!((t.n_rows(), t.n_cols()), (5, 3));
    // The header merged across two columns; the total row and header row.
    assert_eq!(inputs(t)[0], ["Region and Q2", "", "Q3"]);
    assert_eq!(t.merges.len(), 1);
    assert_eq!((t.merges[0].r0, t.merges[0].c0, t.merges[0].r1, t.merges[0].c1), (0, 0, 0, 1));
    assert_eq!((t.header_rows, t.footer_rows, t.header_columns), (1, 1, 0));
    // Text that looks like a formula stays text.
    assert_eq!(t.cells[2][2].input, "'=SUM(B2:B3)");
    // Medium Style 2 - Accent 1: a filled header in white bold, banded
    // rows, the cell filled by hand, the red bold run.
    let f = |r, c| t.format_at(r, c);
    assert_eq!((f(0, 0).fill.as_deref(), f(0, 0).color.as_deref(), f(0, 0).bold), (Some("#4f81bd"), Some("#ffffff"), Some(true)));
    assert_eq!(f(1, 0).fill.as_deref(), Some("#d0d8e8"));
    assert_eq!(f(2, 1).fill.as_deref(), Some("#e9edf4"));
    assert_eq!(f(2, 0).fill.as_deref(), Some("#ffee00"));
    assert_eq!(f(4, 2).fill.as_deref(), Some("#4f81bd"));
    assert_eq!(f(1, 1).align, Some(zebra_core::Align::Right));
    assert_eq!(t.cells[3][0].input, "East");
    assert!(t.cells[3][0].spans.iter().any(|s| s.format.bold.is_some() || s.format.color.is_some()), "{:?}", t.cells[3][0].spans);
    // White lines between the cells, as the style draws them.
    assert!(t.cell_formats.iter().any(|rf| rf.format.border.as_ref().is_some_and(|b| b.color.as_deref() == Some("#ffffff"))));
    // A plain Zebra style under it, with nothing of its own showing.
    assert_eq!(t.gridlines.map(|g| (g.horizontal, g.vertical, g.outline)), Some((false, false, false)));
    assert_eq!(t.banded, Some(false));
    // As wide as PowerPoint drew it: 6 inches of a 13.333-inch slide.
    let w: f64 = t.columns.iter().map(|c| c.width).sum();
    assert!((w - 1000.0 * 6.0 / 13.333).abs() < 1.0, "{w}");

    // Table Grid: black lines on every edge, no fills.
    let g = tables(&deck, 1)[0];
    assert_eq!(g.header_rows, 0);
    assert_eq!(g.format_at(0, 0).fill.as_deref(), Some("#ffffff00"));
    let lines = g.cell_formats.iter().filter(|rf| rf.format.border.as_ref().is_some_and(|b| b.color.as_deref() == Some("#000000"))).count();
    assert!(lines >= 4, "{:?}", g.cell_formats);
    // 200 rows, every one kept.
    let big = tables(&deck, 1)[1];
    assert_eq!((big.n_rows(), big.n_cols()), (200, 8));
    assert_eq!(big.cells[199][0].input, "Row 200");
    // An unknown style: the plain look, still its text.
    assert_eq!(tables(&deck, 1)[2].cells[0][0].input, "Unknown");
}

#[test]
fn charts_come_in_editable_with_their_values_and_colours() {
    let ed = editor();
    let deck = read("tables_charts.pptx", &ed);
    let cs = charts(&deck, 0 + 2);
    let types: Vec<ChartType> = cs.iter().map(|c| c.chart_type).collect();
    use ChartType::*;
    // The 3D column comes in flat; 100% stacked bar as stacked bar (noted).
    assert_eq!(types, [StackedBar, Line, StackedArea, Radar, StackedBar, Column, PercentColumn, Pie, Donut, Scatter, Bubble]);
    let c = cs[0];
    assert_eq!(inputs(&c.data), [["", "Q2", "Q3"], ["North", "1240", "1410"], ["South", "980", "1050"], ["East", "760", "702"]]);
    assert_eq!((c.title.as_str(), c.legend, c.axis_min, c.axis_max), ("Stacked", true, Some(0.0), Some(3000.0)));
    // The deck's series colours.
    assert_eq!(c.series.iter().map(|s| s.color.as_deref()).collect::<Vec<_>>(), [Some("#4f81bd"), Some("#c0504d")]);
    // What it draws is what PowerPoint saved.
    let p = c.plot(&zebra_core::Document::default());
    assert_eq!(p.categories, ["North", "South", "East"]);
    assert_eq!(p.series[1].values, [1410.0, 1050.0, 702.0]);
    // Scatter: X and Y pairs; bubble: X, Y and size.
    let sc = cs[9].plot(&zebra_core::Document::default());
    assert_eq!(sc.series[0].values, [2.0, 4.0, 5.0]);
    assert_eq!(inputs(&cs[10].data)[1], ["1", "1", "2", "3"]);
}

#[test]
fn imported_tables_and_charts_draw_save_and_open_again() {
    let r = zebra_render::Renderer::new();
    let mut ed = Editor::default();
    ed.measure = r.measure();
    let deck = read("tables_charts.pptx", &ed);
    ed.import_deck(&deck, "Import".into(), zebra_core::Origin::Human).unwrap();
    let n = |ed: &Editor, k: &str| ed.doc.elements.values().filter(|e| e.kind.type_name() == k).count();
    assert_eq!((n(&ed, "table"), n(&ed, "chart")), (4, 11));
    // Rows grew to fit their text; nothing was dropped by the editor's
    // checks.
    let shots = std::env::var("ZEBRA_SHOTS").ok().map(std::path::PathBuf::from);
    let frames: Vec<String> = ed.doc.elements.values().filter(|e| matches!(&e.kind, ElementKind::Frame(f) if f.slide)).map(|e| e.id.clone()).collect();
    assert_eq!(frames.len(), 3);
    for (i, id) in frames.iter().enumerate() {
        let png = r.capture(&ed, &zebra_render::CaptureTarget::Element { id: id.clone() }, 1333, 750).unwrap();
        assert!(png.len() > 2000);
        if let Some(d) = &shots {
            let _ = std::fs::create_dir_all(d);
            let _ = std::fs::write(d.join(format!("pptx-tables-charts-{i}.png")), &png);
        }
    }
    let back = zebra_core::io::load_bytes(&zebra_core::io::save_bytes(&ed.doc).unwrap()).unwrap();
    assert_eq!(back.elements.len(), ed.doc.elements.len());
    for (id, e) in &ed.doc.elements {
        assert_eq!(back.elements[id].kind, e.kind, "{id}");
    }
}
