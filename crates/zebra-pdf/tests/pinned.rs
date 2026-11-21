//! The PDFium library Zebra ships and the binding it is used through must
//! be the same build (docs/PDFIUM.md).

#[test]
fn library_and_binding_name_the_same_pdfium_build() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let script = std::fs::read_to_string(format!("{root}/scripts/fetch-pdfium.sh")).unwrap();
    let version = script.lines().find_map(|l| l.strip_prefix("VERSION=")).expect("VERSION= in fetch-pdfium.sh").trim();
    let cargo = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    assert!(cargo.contains(&format!("\"pdfium_{version}\"")), "fetch-pdfium.sh fetches build {version}, but zebra-pdf's pdfium-render feature is another build");
    let doc = std::fs::read_to_string(format!("{root}/docs/PDFIUM.md")).unwrap();
    assert!(doc.contains(&format!("build **{version}**")), "docs/PDFIUM.md names another build");
}
