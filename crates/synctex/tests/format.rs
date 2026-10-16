//! Format features the TeX Live fixtures do not exercise, checked on hand-written files.

// Test code may unwrap and panic (AGENTS.md: tests are exempt from the never-panic rule).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use texcraft_synctex::SyncTex;

/// 1 bp in scaled points.
const BP: f32 = 65781.76;

fn header(extra_inputs: &str) -> String {
    format!(
        "SyncTeX Version:1\nInput:1:/p/./main.tex\n{extra_inputs}Output:pdf\nMagnification:1000\n\
         Unit:1\nX Offset:0\nY Offset:0\nContent:\n"
    )
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn page_hint_orders_pages() {
    let mut text = header("");
    for page in 1..=2 {
        text.push_str(&format!(
            "{{{page}\n[1,1:0,0:30000000,40000000,0\n(1,3:4736286,10000000:5000000,655360,0\n\
             g1,3:5000000,10000000\n)\n]\n}}{page}\n"
        ));
    }
    text.push_str("Postamble:\nCount:4\nPost scriptum:\n");
    let s = SyncTex::from_bytes(text.as_bytes()).unwrap();
    let pages = |hint| -> Vec<u32> {
        s.forward_with_hint(Path::new("main.tex"), 3, hint)
            .iter()
            .map(|r| r.page)
            .collect()
    };
    assert_eq!(pages(0), vec![1, 2]);
    assert_eq!(pages(1), vec![1, 2]);
    assert_eq!(pages(2), vec![2, 1]);
    assert_eq!(s.forward(Path::new("/p/main.tex"), 3).len(), 2);
    let r = s.forward(Path::new("/p/main.tex"), 3)[0];
    assert!(
        close(r.x, 4736286.0 / BP) && close(r.width, 5000000.0 / BP),
        "{r:?}"
    );
    assert!(close(r.y, (10000000.0 - 655360.0) / BP) && close(r.height, 655360.0 / BP));
}

#[test]
fn forms_are_expanded_at_their_references() {
    let mut text = header("");
    // A form whose content is an hbox from line 5, used once on page 1.
    text.push_str("<1\n(1,5:0,0:6578176,655360,0\ng1,5:1000000,0\nx1,5:3000000,0\n)\n>\n");
    text.push_str(
        "{1\n[1,1:4736286,46220574:30000000,40000000,0\n(1,2:4736286,10000000:30000000,655360,0\n\
         x1,2:5000000,10000000\nf1:10000000,20000000\nx1,2:9000000,10000000\n)\n]\n}1\n",
    );
    text.push_str("Postamble:\nCount:9\n");
    let s = SyncTex::from_bytes(text.as_bytes()).unwrap();
    let rects = s.forward(Path::new("main.tex"), 5);
    assert_eq!(rects.len(), 1, "{rects:?}");
    let r = rects[0];
    assert_eq!(r.page, 1);
    assert!(close(r.x, 10000000.0 / BP), "{r:?}");
    assert!(close(r.width, 100.0), "{r:?}");
    assert!(close(r.y, (20000000.0 - 655360.0) / BP), "{r:?}");
    let loc = s.inverse(1, r.x + 30.0, r.y + r.height / 2.0).unwrap();
    assert_eq!(loc.line, 5);
    assert_eq!(loc.file, Path::new("/p/main.tex"));
    // A reference to an undefined form is dropped.
    let text = text.replace("f1:", "f7:");
    let s = SyncTex::from_bytes(text.as_bytes()).unwrap();
    assert!(
        s.forward(Path::new("main.tex"), 5)
            .iter()
            .all(|r| !close(r.x, 10000000.0 / BP))
    );
}

#[test]
fn post_scriptum_overrides_magnification_and_offsets() {
    let body = "{1\n[1,1:0,0:30000000,40000000,0\n(1,3:6578176,13156352:6578176,657818,0\n\
                g1,3:6578176,13156352\n)\n]\n}1\nPostamble:\nCount:4\n";
    let plain = format!("{}{body}Post scriptum:\n", header(""));
    let scaled = format!(
        "{}{body}Post scriptum:\nMagnification:2\nX Offset:1in\nY Offset:-36bp\n",
        header("")
    );
    let a = SyncTex::from_bytes(plain.as_bytes())
        .unwrap()
        .forward(Path::new("main.tex"), 3);
    let b = SyncTex::from_bytes(scaled.as_bytes())
        .unwrap()
        .forward(Path::new("main.tex"), 3);
    let (a, b) = (a[0], b[0]);
    assert!(close(a.x, 100.0) && close(a.width, 100.0), "{a:?}");
    assert!(close(b.x, 200.0 + 72.0) && close(b.width, 200.0), "{b:?}");
    assert!(close(b.y + b.height, 400.0 - 36.0), "{b:?}");
}

#[test]
fn magnification_record_scales_coordinates() {
    let text = header("").replace("Magnification:1000", "Magnification:2000")
        + "{1\n[1,1:0,0:30000000,40000000,0\n(1,3:6578176,13156352:6578176,657818,0\n\
           g1,3:6578176,13156352\n)\n]\n}1\n";
    let s = SyncTex::from_bytes(text.as_bytes()).unwrap();
    let r = s.forward(Path::new("main.tex"), 3)[0];
    assert!(close(r.x, 200.0) && close(r.width, 200.0), "{r:?}");
    let loc = s.inverse(1, 300.0, 395.0).unwrap();
    assert_eq!(loc.line, 3);
}

#[test]
fn columns_compressed_v_and_late_inputs() {
    let mut text = header("");
    text.push_str("{1\n[1,1:0,0:30000000,40000000,0\n(2,4,6:6578176,13156352:6578176,657818,0\n");
    text.push_str("$2,4,6:6578176,=\n$2,4,6:9867264,=\n)\n]\n}1\n");
    // The input of tag 2 is recorded after the page that uses it (still resolved).
    text.push_str("Input:2:/p/./sub/late.tex\nPostamble:\nCount:4\n");
    let s = SyncTex::from_bytes(text.as_bytes()).unwrap();
    assert_eq!(
        s.inputs(),
        vec![Path::new("/p/main.tex"), Path::new("/p/sub/late.tex")]
    );
    let r = s.forward(Path::new("sub/late.tex"), 4);
    assert_eq!(r.len(), 1);
    assert!(close(r[0].y + r[0].height, 200.0), "{r:?}");
    let loc = s.inverse(1, 120.0, 195.0).unwrap();
    assert_eq!(loc.file, Path::new("/p/sub/late.tex"));
    assert_eq!((loc.line, loc.column), (4, Some(7)));
}

#[test]
fn nearest_line_is_used_when_a_line_has_no_records() {
    let mut text = header("");
    text.push_str("{1\n[1,1:0,0:30000000,40000000,0\n");
    for (line, v) in [(2, 10000000), (9, 20000000)] {
        text.push_str(&format!(
            "(1,{line}:0,{v}:6578176,657818,0\ng1,{line}:100,{v}\n)\n"
        ));
    }
    text.push_str("]\n}1\n");
    let s = SyncTex::from_bytes(text.as_bytes()).unwrap();
    let v_of = |line| s.forward(Path::new("main.tex"), line)[0].y;
    let at2 = v_of(2);
    let at9 = v_of(9);
    // 3 -> tries 3, 4, 2: line 2 wins; 7 -> 7, 8, 6, 9: line 9 wins; past the end -> 9.
    assert_eq!(v_of(3), at2);
    assert_eq!(v_of(7), at9);
    assert_eq!(v_of(100), at9);
    assert_eq!(v_of(1), at2);
}

#[test]
fn missing_preamble_fields_and_crlf_are_tolerated() {
    let text = "junk line\r\nSyncTeX Version:1\r\nInput:1:a.tex\r\n{1\r\n[1,1:0,0:10,10,0\r\n\
                (1,2:0,5:100000,10,0\r\ng1,2:50,5\r\n)\r\n]\r\n}1\r\n";
    let s = SyncTex::from_bytes(text.as_bytes()).unwrap();
    assert_eq!(s.pages(), 1);
    assert_eq!(s.inputs(), vec![Path::new("a.tex")]);
    assert!(!s.forward(Path::new("a.tex"), 2).is_empty());
}
