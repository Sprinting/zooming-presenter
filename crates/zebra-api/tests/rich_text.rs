//! Formatted text without Markdown input: what a Markdown box held (a bold
//! run, italic, strike, a link, nested bullets, a numbered list) is built
//! by an agent with `add_text`, `format_text` and `format_paragraphs`.
use std::path::PathBuf;
use std::sync::Once;

use serde_json::json;
use zebra_api::client::{Client, ClientError};
use zebra_api::{ApiOptions, HeadlessHandle, Permission};
use zebra_core::model::{ElementKind, ListKind, ParaFormat, SpanFormat, TextSpan};

fn start() -> HeadlessHandle {
    static INIT: Once = Once::new();
    let dir: PathBuf = std::env::temp_dir().join(format!("zebra-api-rich-{}", std::process::id()));
    INIT.call_once(|| {
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once before any server starts in this test binary.
        unsafe { std::env::set_var("ZEBRA_DATA_DIR", dir.join("data")) };
    });
    HeadlessHandle::spawn(ApiOptions { socket: None, permission: Permission::Edit }, None).expect("instance starts")
}

#[test]
fn format_commands_reproduce_a_markdown_box() {
    // The box `add_text {"markdown": true}` made from:
    //   Why **zebras** have *stripes*
    //
    //   - flies get [confused](https://example.com)
    //     - really ~~maybe~~
    //   1. first
    //   2. second
    let h = start();
    let mut c = Client::connect_hello(&h.socket, "rich", &h.token).unwrap();
    c.call(
        "batch",
        json!({"label": "Formatted box", "commands": [
            {"op":"add_text","id":"t","text":"Why zebras have stripes\nflies get confused\nreally maybe\nfirst\nsecond","x":0,"y":0},
            {"op":"format_text","id":"t","find":"zebras","format":{"bold":true}},
            {"op":"format_text","id":"t","find":"stripes","format":{"italic":true}},
            {"op":"format_text","id":"t","find":"confused","format":{"link":"https://example.com"}},
            {"op":"format_text","id":"t","find":"maybe","format":{"strike":true}},
            {"op":"format_paragraphs","id":"t","first":1,"last":2,"list":"bullet"},
            {"op":"format_paragraphs","id":"t","first":2,"last":2,"indent":1},
            {"op":"format_paragraphs","id":"t","first":3,"last":4,"list":"number"}
        ]}),
    )
    .unwrap();
    let doc: zebra_core::Document = serde_json::from_value(c.call("query", json!({"query":{"q":"document"}})).unwrap()).unwrap();
    let ElementKind::Text(t) = &doc.get("t").unwrap().kind else { panic!("not text") };

    assert_eq!(t.text, "Why zebras have stripes\nflies get confused\nreally maybe\nfirst\nsecond");
    let span = |start, end, format| TextSpan { start, end, format };
    assert_eq!(
        t.spans,
        vec![
            span(4, 10, SpanFormat { bold: Some(true), ..Default::default() }),
            span(16, 23, SpanFormat { italic: Some(true), ..Default::default() }),
            span(34, 42, SpanFormat { link: Some("https://example.com".into()), ..Default::default() }),
            span(50, 55, SpanFormat { strike: Some(true), ..Default::default() }),
        ]
    );
    let para = |i: usize| t.paras.get(i).cloned().unwrap_or_default();
    let list = |p: ParaFormat| (p.list.unwrap_or(ListKind::None), p.indent);
    assert_eq!(
        (0..5).map(|i| list(para(i))).collect::<Vec<_>>(),
        [(ListKind::None, 0), (ListKind::Bullet, 0), (ListKind::Bullet, 1), (ListKind::Number, 0), (ListKind::Number, 0)]
    );
    // And `describe` reads it back as the same Markdown.
    let md = zebra_core::rich::to_markdown(t);
    for part in ["**zebras**", "*stripes*", "[confused](https://example.com)", "  - really ~~maybe~~", "1. first", "2. second"] {
        assert!(md.contains(part), "{part} missing from {md}");
    }
    h.stop().unwrap();
}

#[test]
fn markdown_input_is_refused_with_a_hint() {
    let h = start();
    let mut c = Client::connect_hello(&h.socket, "rich", &h.token).unwrap();
    let Err(ClientError::Rpc(e)) = c.call("apply", json!({"command":{"op":"add_text","text":"**Hi**","markdown":true}})) else {
        panic!("`markdown` must be refused")
    };
    assert!(e.message.contains("unknown field `markdown`"), "{e}");
    let hint = e.hint_text().unwrap_or("");
    assert!(hint.contains("format_text") && hint.contains("format_paragraphs"), "{e}");
    h.stop().unwrap();
}
