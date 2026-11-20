//! PDFs made by hand for the tests (as the robustness audit's mkpdf.py
//! makes them), and finding PDFium and the helper.
#![allow(dead_code)]
use std::path::PathBuf;

/// PDFium for the helper: `ZEBRA_PDFIUM`, else target/pdfium/<platform>
/// (scripts/fetch-pdfium.sh). None: the tests say so and pass.
pub fn engine() -> bool {
    if std::env::var_os("ZEBRA_PDFIUM").is_none() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/pdfium");
        let lib = if cfg!(target_os = "macos") { "libpdfium.dylib" } else { "libpdfium.so" };
        let found = ["mac-arm64", "mac-univ", "mac-x64", "linux-x64"].iter().map(|p| root.join(p).join("lib").join(lib)).find(|p| p.is_file());
        if let Some(p) = found {
            // SAFETY: set before this test binary starts any helper.
            unsafe { std::env::set_var("ZEBRA_PDFIUM", p) };
        }
    }
    let ok = std::env::var_os("ZEBRA_PDFIUM").is_some();
    if !ok {
        assert!(std::env::var_os("ZEBRA_SKIP_PDF").is_some(), "PDFium not found: run scripts/fetch-pdfium.sh for this platform or set ZEBRA_PDFIUM (ZEBRA_SKIP_PDF=1 skips the PDF tests)");
        eprintln!("SKIPPED: PDFium not found (run scripts/fetch-pdfium.sh linux-x64 or set ZEBRA_PDFIUM)");
    }
    ok
}

pub fn helper() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_zebra-pdf-helper"))
}

pub use zebra_pdf::samples::*;

/// Anonymous memory of this process (bytes).
pub fn heap() -> u64 {
    zebra_pdf::sys::footprint(std::process::id()).unwrap_or(0)
}
