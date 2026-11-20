//! Small PDFs made by hand for tests, as the robustness audit's mkpdf.py
//! makes them: many pages, the page PDFium spends 12 GB on, odd page
//! boxes. (Not used by the app.)

/// A PDF of these objects (object `n` is `objs[n - 1]`; 1 is the catalog).
pub fn pdf(objs: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = vec![];
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

pub fn stream(data: &[u8], extra: &str) -> Vec<u8> {
    let mut s = format!("<< /Length {} {extra} >>\nstream\n", data.len()).into_bytes();
    s.extend_from_slice(data);
    s.extend_from_slice(b"\nendstream");
    s
}

/// `n` pages, 200 x 200 points, each a blue square on white (one
/// content stream shared by all).
pub fn many_pages(n: usize) -> Vec<u8> {
    let mut objs = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), vec![], stream(b"0 0 1 rg 10 10 100 100 re f", "")];
    let mut kids = String::new();
    for _ in 0..n {
        objs.push(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 3 0 R >>".to_vec());
        kids.push_str(&format!("{} 0 R ", objs.len()));
    }
    objs[1] = format!("<< /Type /Pages /Count {n} /Kids [{kids}] >>").into_bytes();
    pdf(&objs)
}

/// One page drawing a form that draws the one below it 10 times, 8
/// levels deep: 10^7 rectangles. The audit's slow-page.pdf: PDFium takes
/// 12 GB on it, then aborts.
pub fn slow_page() -> Vec<u8> {
    let mut objs = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(), vec![]];
    objs.push(stream(b"0 0 1 rg 0 0 1 1 re f", "/Type /XObject /Subtype /Form /BBox [0 0 1000 1000]"));
    for _ in 1..8 {
        let body: Vec<u8> = (0..10).flat_map(|j| format!("q 1 0 0 1 {j} {j} cm /X Do Q\n").into_bytes()).collect();
        let below = objs.len();
        objs.push(stream(&body, &format!("/Type /XObject /Subtype /Form /BBox [0 0 1000 1000] /Resources << /XObject << /X {below} 0 R >> >>")));
    }
    let top = objs.len();
    objs.push(stream(b"/X Do", ""));
    objs[2] = format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 600] /Contents {} 0 R /Resources << /XObject << /X {top} 0 R >> >> >>", top + 1).into_bytes();
    pdf(&objs)
}

/// One page with this MediaBox.
pub fn boxed(media_box: &str) -> Vec<u8> {
    pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox {media_box} /Contents 4 0 R >>").into_bytes(),
        stream(b"0 0 1 rg 10 10 100 100 re f", ""),
    ])
}


/// `n` pages of 600 x 600 points: those in `slow` (0-based) draw the
/// 10^7 rectangles of [`slow_page`], the others a blue square.
pub fn some_slow_pages(n: usize, slow: &[usize]) -> Vec<u8> {
    // 1 catalog, 2 pages, 3 plain content, 4.. the forms, then the slow
    // content, then the pages.
    let mut objs = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), vec![], stream(b"0 0 1 rg 10 10 100 100 re f", "")];
    objs.push(stream(b"0 0 1 rg 0 0 1 1 re f", "/Type /XObject /Subtype /Form /BBox [0 0 1000 1000]"));
    for _ in 1..8 {
        let body: Vec<u8> = (0..10).flat_map(|j| format!("q 1 0 0 1 {j} {j} cm /X Do Q\n").into_bytes()).collect();
        let below = objs.len();
        objs.push(stream(&body, &format!("/Type /XObject /Subtype /Form /BBox [0 0 1000 1000] /Resources << /XObject << /X {below} 0 R >> >>")));
    }
    let top = objs.len();
    objs.push(stream(b"/X Do", ""));
    let slow_content = objs.len();
    let mut kids = String::new();
    for i in 0..n {
        let page = if slow.contains(&i) {
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 600] /Contents {slow_content} 0 R /Resources << /XObject << /X {top} 0 R >> >> >>")
        } else {
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 600] /Contents 3 0 R >>".to_string()
        };
        objs.push(page.into_bytes());
        kids.push_str(&format!("{} 0 R ", objs.len()));
    }
    objs[1] = format!("<< /Type /Pages /Count {n} /Kids [{kids}] >>").into_bytes();
    pdf(&objs)
}
