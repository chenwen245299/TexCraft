//! Compares forward and inverse search with the output of the real `synctex` CLI, recorded by
//! `tests/data/generate.py` (see `tests/data/README.md`).

// Test code may unwrap and panic (AGENTS.md: tests are exempt from the never-panic rule).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use texcraft_synctex::{PdfRect, SyncTex};

/// (fixture directory, SyncTeX file name)
const FIXTURES: &[(&str, &str)] = &[
    ("single", "single.synctex.gz"),
    ("multi", "main.synctex.gz"),
    ("twocol", "twocol.synctex.gz"),
    ("xetex", "xedoc.synctex.gz"),
    ("lualatex", "luadoc.synctex.gz"),
    ("mag", "mag.synctex.gz"),
    ("xform", "xform.synctex.gz"),
];

/// Coordinates are compared with this tolerance, in PDF points.
const TOLERANCE: f32 = 0.5;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
}

fn load(fixture: &str, file: &str) -> SyncTex {
    let path = data_dir().join(fixture).join(file);
    SyncTex::load(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

struct ViewCase {
    input: String,
    line: u32,
    hint: u32,
    expected: Vec<PdfRect>,
}

fn read_forward(fixture: &str) -> Vec<ViewCase> {
    let text = std::fs::read_to_string(data_dir().join(fixture).join("forward.txt")).unwrap();
    let mut cases: Vec<ViewCase> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields[0] == "view" {
            cases.push(ViewCase {
                input: fields[1].to_string(),
                line: fields[2].parse().unwrap(),
                hint: fields[3].parse().unwrap(),
                expected: Vec::new(),
            });
        } else {
            let n: Vec<f32> = fields.iter().map(|f| f.parse().unwrap()).collect();
            // page x y h v W H, where v is the bottom edge and H counts upwards.
            let (h, v, w, hh) = (n[3], n[4], n[5], n[6]);
            let (left, right, top, bottom) = (h, h + w, v - hh, v);
            cases.last_mut().unwrap().expected.push(PdfRect {
                page: n[0] as u32,
                x: left.min(right),
                y: top.min(bottom),
                width: (right - left).abs(),
                height: (bottom - top).abs(),
            });
        }
    }
    cases
}

fn rect_close(a: &PdfRect, b: &PdfRect) -> bool {
    a.page == b.page
        && (a.x - b.x).abs() <= TOLERANCE
        && (a.y - b.y).abs() <= TOLERANCE
        && (a.width - b.width).abs() <= TOLERANCE
        && (a.height - b.height).abs() <= TOLERANCE
}

struct EditCase {
    page: u32,
    x: f32,
    y: f32,
    /// (input, line, column) of the first result, `None` when `synctex edit` found nothing.
    expected: Option<(String, u32, i32)>,
    /// Line of the second result, if any.
    second: Option<u32>,
}

fn read_inverse(fixture: &str) -> Vec<EditCase> {
    let text = std::fs::read_to_string(data_dir().join(fixture).join("inverse.txt")).unwrap();
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let expected = (f[4] != "none").then(|| {
                (
                    f[4].to_string(),
                    f[5].parse().unwrap(),
                    f[6].parse().unwrap(),
                )
            });
            EditCase {
                page: f[0].parse().unwrap(),
                x: f[1].parse().unwrap(),
                y: f[2].parse().unwrap(),
                expected,
                second: f.get(7).map(|s| s.parse().unwrap()),
            }
        })
        .collect()
}

/// Does the recorded `file` correspond to the oracle's relative name (or `@basename`)?
fn same_input(file: &Path, oracle: &str) -> bool {
    match oracle.strip_prefix('@') {
        Some(name) => file.file_name().is_some_and(|n| n == name),
        None => file.ends_with(oracle),
    }
}

#[test]
fn forward_search_matches_synctex_view() {
    let mut total = 0;
    let mut agree = 0;
    let mut report = String::new();
    for (fixture, file) in FIXTURES {
        let synctex = load(fixture, file);
        let cases = read_forward(fixture);
        assert!(!cases.is_empty(), "{fixture}: no forward cases");
        let mut fixture_agree = 0;
        for case in &cases {
            let got = synctex.forward_with_hint(Path::new(&case.input), case.line, case.hint);
            let ok = got.len() == case.expected.len()
                && got
                    .iter()
                    .zip(&case.expected)
                    .all(|(a, b)| rect_close(a, b));
            if ok {
                fixture_agree += 1;
            } else if report.lines().count() < 40 {
                report.push_str(&format!(
                    "{fixture} {}:{} hint {}:\n  expected {:?}\n  got      {:?}\n",
                    case.input, case.line, case.hint, case.expected, got
                ));
            }
        }
        eprintln!(
            "forward {fixture}: {fixture_agree}/{} queries agree",
            cases.len()
        );
        total += cases.len();
        agree += fixture_agree;
    }
    eprintln!("forward total: {agree}/{total}");
    assert_eq!(
        agree, total,
        "forward search disagrees with synctex view:\n{report}"
    );
}

#[test]
fn forward_dedups_and_matches_by_file_name() {
    let synctex = load("multi", "main.synctex.gz");
    // `forward` reports each rectangle once.
    for line in 1..=12 {
        let all = synctex.forward_with_hint(Path::new("sections/intro.tex"), line, 0);
        let dedup = synctex.forward(Path::new("sections/intro.tex"), line);
        assert!(dedup.len() <= all.len());
        for (i, a) in dedup.iter().enumerate() {
            assert!(!dedup[i + 1..].contains(a));
            assert!(all.contains(a));
        }
        for a in &all {
            assert!(dedup.contains(a));
        }
    }
    // File-name and relative-path matching find the same input.
    let by_name = synctex.forward(Path::new("detail.tex"), 2);
    let by_rel = synctex.forward(Path::new("sections/detail.tex"), 2);
    let by_abs = synctex.forward(&data_dir().join("multi/sections/detail.tex"), 2);
    assert!(!by_name.is_empty());
    assert_eq!(by_name, by_rel);
    assert_eq!(by_name, by_abs);
    assert!(synctex.forward(Path::new("missing.tex"), 2).is_empty());
    // Every rectangle is on a real page and well formed.
    for line in 1..=25 {
        for r in synctex.forward(Path::new("main.tex"), line) {
            assert!(r.page >= 1 && r.page <= synctex.pages());
            assert!(r.width >= 0.0 && r.height >= 0.0);
        }
    }
}

#[test]
fn inverse_search_matches_synctex_edit() {
    let mut total = 0;
    let mut agree_line = 0;
    let mut report = String::new();
    for (fixture, file) in FIXTURES {
        let synctex = load(fixture, file);
        let cases = read_inverse(fixture);
        assert!(!cases.is_empty(), "{fixture}: no inverse cases");
        let mut fixture_line = 0;
        let mut fixture_second = 0;
        let mut with_second = 0;
        for case in &cases {
            let got = synctex.inverse_candidates(case.page, case.x, case.y);
            let ok = match (&case.expected, got.first()) {
                (None, None) => true,
                (Some((input, line, column)), Some(loc)) => {
                    same_input(&loc.file, input)
                        && loc.line == *line
                        && loc.column.map_or(-1, |c| c as i32 - 1) == *column
                }
                _ => false,
            };
            if ok {
                fixture_line += 1;
            } else if report.lines().count() < 40 {
                report.push_str(&format!(
                    "{fixture} page {} ({}, {}): expected {:?}, got {:?}\n",
                    case.page, case.x, case.y, case.expected, got
                ));
            }
            if let Some(second) = case.second {
                with_second += 1;
                if got.get(1).is_some_and(|l| l.line == second) {
                    fixture_second += 1;
                }
            }
            // We never invent a second candidate. (We may omit one: `synctex edit` also
            // reports nodes whose tag names no input, e.g. LuaTeX's font kerns, printing
            // the first result's file name next to them; we drop those.)
            if let Some(loc) = got.get(1) {
                assert_eq!(
                    case.second,
                    Some(loc.line),
                    "{fixture} page {} ({}, {}): unexpected second candidate {got:?}",
                    case.page,
                    case.x,
                    case.y
                );
            }
        }
        eprintln!(
            "inverse {fixture}: {fixture_line}/{} points agree (second candidate {fixture_second}/{with_second})",
            cases.len()
        );
        total += cases.len();
        agree_line += fixture_line;
    }
    eprintln!("inverse total: {agree_line}/{total}");
    assert_eq!(
        agree_line, total,
        "inverse search disagrees with synctex edit:\n{report}"
    );
}

#[test]
fn inverse_reports_relative_inputs_of_multi_file_projects() {
    let synctex = load("multi", "main.synctex.gz");
    let mut seen = std::collections::BTreeSet::new();
    for case in read_inverse("multi") {
        if let Some(loc) = synctex.inverse(case.page, case.x, case.y) {
            let name = loc.file.file_name().unwrap().to_string_lossy().into_owned();
            seen.insert(name);
            assert!(
                !loc.file.to_string_lossy().contains("/./"),
                "{:?}",
                loc.file
            );
        }
    }
    for name in ["main.tex", "intro.tex", "results.tex", "detail.tex"] {
        assert!(seen.contains(name), "no inverse hit in {name}: {seen:?}");
    }
}

#[test]
fn inputs_and_pages() {
    let synctex = load("multi", "main.synctex.gz");
    assert_eq!(synctex.pages(), 1);
    let inputs = synctex.inputs();
    for rel in [
        "main.tex",
        "sections/intro.tex",
        "sections/results.tex",
        "sections/detail.tex",
    ] {
        assert!(
            inputs.iter().any(|p| p.ends_with(rel)),
            "{rel} missing from {inputs:?}"
        );
    }
    assert!(inputs.iter().all(|p| !p.to_string_lossy().contains("/./")));
    assert_eq!(load("single", "single.synctex.gz").pages(), 3);
    assert_eq!(load("twocol", "twocol.synctex.gz").pages(), 3);
    assert_eq!(load("xetex", "xedoc.synctex.gz").pages(), 2);
}

#[test]
fn path_for_pdf_finds_the_synctex_file() {
    let dir = data_dir().join("single");
    assert_eq!(
        SyncTex::path_for_pdf(&dir.join("single.pdf")),
        Some(dir.join("single.synctex.gz"))
    );
    assert_eq!(SyncTex::path_for_pdf(&dir.join("nothing.pdf")), None);
}
