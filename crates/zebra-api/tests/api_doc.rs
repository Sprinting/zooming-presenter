//! docs/API.md's command tables are generated from the catalogue.
//! ZEBRA_BLESS=1 rewrites them.
#[test]
fn api_doc_tables_match_the_catalogue() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/API.md");
    let doc = std::fs::read_to_string(&path).unwrap();
    let (b, e) = ("<!-- BEGIN GENERATED -->\n", "<!-- END GENERATED -->");
    let i = doc.find(b).unwrap() + b.len();
    let j = doc.find(e).unwrap();
    let want = format!("{}\n\n", zebra_api::catalogue::catalogue_markdown().trim_end());
    if std::env::var_os("ZEBRA_BLESS").is_some() {
        std::fs::write(&path, format!("{}{}{}", &doc[..i], want, &doc[j..])).unwrap();
        return;
    }
    assert_eq!(&doc[i..j], want, "docs/API.md is out of date: run with ZEBRA_BLESS=1");
}
