//! Oracle tests: real logs produced by TeX Live 2022 (pdfTeX, XeTeX, LuaHBTeX via latexmk),
//! BibTeX and Tectonic 0.16 from the deliberately broken documents in `tests/data/<case>/`
//! (regenerate with `tests/data/generate.sh`). Every expectation below was checked by reading
//! the corresponding log.
//!
//! Project files appear in the logs as relative paths, so the logs are parsed against a fake
//! base directory and files are compared relative to it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Component, Path};

use texcraft_log::Severity::{self, BadBox, Error, Warning};
use texcraft_log::{Issue, LogReport, parse_blg, parse_log};

const BASE: &str = "/project";

/// (severity, log line, file relative to BASE, line, message)
type Row<'a> = (Severity, u32, Option<&'a str>, Option<u32>, &'a str);
/// [`Row`] with owned strings, for comparing.
type OwnedRow = (Severity, u32, Option<String>, Option<u32>, String);

fn read(case: &str, file: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(case)
        .join(file);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    String::from_utf8_lossy(&bytes).into_owned()
}

fn log(case: &str) -> LogReport {
    parse_log(&read(case, "main.log"), Path::new(BASE))
}

fn blg(case: &str) -> Vec<Issue> {
    parse_blg(&read(case, "main.blg"), Path::new(BASE))
}

/// The issue's file relative to BASE, `/`-separated (absolute when outside BASE).
fn relative(issue: &Issue) -> Option<String> {
    let file = issue.file.as_ref()?;
    Some(match file.strip_prefix(BASE) {
        Ok(rel) => rel
            .components()
            .map(|c: Component<'_>| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => file.to_string_lossy().into_owned(),
    })
}

fn assert_issues(issues: &[Issue], expected: &[Row]) {
    let actual: Vec<OwnedRow> = issues
        .iter()
        .map(|i| {
            (
                i.severity,
                i.log_line,
                relative(i),
                i.line,
                i.message.clone(),
            )
        })
        .collect();
    let expected: Vec<OwnedRow> = expected
        .iter()
        .map(|&(s, l, f, n, m)| (s, l, f.map(str::to_string), n, m.to_string()))
        .collect();
    assert_eq!(
        actual.len(),
        expected.len(),
        "issue count\nactual: {actual:#?}"
    );
    for (a, e) in actual.iter().zip(&expected) {
        assert_eq!(a, e);
    }
}

fn find(issues: &[Issue], log_line: u32) -> &Issue {
    issues
        .iter()
        .find(|i| i.log_line == log_line)
        .expect("issue at log line")
}

#[test]
fn latexmk_pdflatex_multi_file_project() {
    let report = log("latexmk-pdf");
    assert_issues(
        &report.issues,
        &[
            (
                Warning,
                214,
                Some("main.tex"),
                Some(6),
                "Reference 'sec:missing' on page 1 undefined",
            ),
            (
                Warning,
                217,
                Some("main.tex"),
                Some(6),
                "Citation 'nokey' on page 1 undefined",
            ),
            (
                Error,
                220,
                Some("chapters/intro.tex"),
                Some(2),
                "Undefined control sequence.",
            ),
            (
                BadBox,
                230,
                Some("chapters/intro.tex"),
                Some(4),
                "Overfull \\hbox (206.08583pt too wide) detected at line 4",
            ),
            // The paragraph began in chapters/intro.tex but ended (at \include's \clearpage)
            // after that file was closed: TeX reports it in main.tex.
            (
                BadBox,
                235,
                Some("main.tex"),
                Some(6),
                "Overfull \\hbox (86.7038pt too wide) in paragraph at lines 6--8",
            ),
            (
                Warning,
                248,
                Some("chapter2.tex"),
                Some(2),
                "Package mypkg: A package warning spanning two lines",
            ),
            (
                Error,
                251,
                Some("chapter2.tex"),
                Some(3),
                "Missing $ inserted.",
            ),
            (
                Error,
                263,
                Some("chapter2.tex"),
                Some(4),
                "Missing $ inserted.",
            ),
            // The file name is wrapped across log lines 274-275.
            (
                Warning,
                277,
                Some(
                    "a-rather-long-file-name-used-to-check-that-wrapped-file-names-in-the-log-are-joined.tex",
                ),
                Some(1),
                "Reference 'sec:alsomissing' on page 3 undefined",
            ),
            (
                Error,
                281,
                Some("sub dir/with space.tex"),
                Some(2),
                "Undefined control sequence.",
            ),
            (
                Warning,
                295,
                Some("main.tex"),
                None,
                "There were undefined references.",
            ),
        ],
    );
    assert!(!report.fatal);
    assert!(!report.rerun_suggested);
    assert_eq!(report.output_pages, Some(3));
    assert_eq!(
        report.output_file.as_deref(),
        Some(Path::new("/project/main.pdf"))
    );
    assert_eq!(
        (report.errors(), report.warnings(), report.bad_boxes()),
        (4, 5, 2)
    );

    let undefined = find(&report.issues, 220);
    assert!(
        undefined
            .detail
            .starts_with("l.2 Some text \\undefinedmacro\n")
    );
    assert!(
        undefined
            .detail
            .contains("{} here (with an unbalanced paren.")
    );
    assert!(undefined.detail.contains("was never \\def'ed"));
    let wide = find(&report.issues, 235);
    assert_eq!(
        wide.detail,
        "[]\\T1/cmr/m/n/10 Supercalifragilisticexpialidociousandsomemorewordswithoutbreaksthatcannotbehyphenatedatall"
    );
    let math = find(&report.issues, 251);
    assert!(math.detail.contains("<inserted text>"));
    assert!(math.detail.contains("l.3 Text with x^"));
}

#[test]
fn latexmk_bibtex_blg() {
    let issues = blg("latexmk-pdf");
    assert_issues(
        &issues,
        &[
            (
                Error,
                7,
                Some("refs.bib"),
                Some(10),
                "I was expecting a ',' or a '}'",
            ),
            (
                Warning,
                12,
                None,
                None,
                "I didn't find a database entry for \"nokey\"",
            ),
        ],
    );
    let detail = &issues[0].detail;
    assert!(detail.contains("title  = {Missing comma above}"));
    assert!(detail.contains("(Error may have been on previous line)"));
    assert!(detail.ends_with("I'm skipping whatever remains of this entry"));
}

#[test]
fn pdflatex_without_file_line_error() {
    let report = log("pdflatex-plain");
    assert_issues(
        &report.issues,
        &[
            (
                Warning,
                165,
                Some("main.tex"),
                Some(4),
                "Class article: A class warning",
            ),
            (
                Warning,
                168,
                Some("main.tex"),
                Some(7),
                "A LaTeX warning continued on a second line",
            ),
            (
                Warning,
                222,
                Some("main.tex"),
                Some(10),
                "Package hyperref: Token not allowed in a PDF string (Unicode): removing 'math shift'",
            ),
            (
                Warning,
                226,
                Some("main.tex"),
                Some(10),
                "Package hyperref: Token not allowed in a PDF string (Unicode): removing 'math shift'",
            ),
            (
                Warning,
                230,
                Some("main.tex"),
                Some(11),
                "Reference 'sec:one' on page 1 undefined",
            ),
            (
                Warning,
                233,
                Some("main.tex"),
                Some(11),
                "Reference 'sec:two' on page 1 undefined",
            ),
            (
                Warning,
                236,
                Some("main.tex"),
                Some(11),
                "Citation 'nobody' on page 1 undefined",
            ),
            // Message wrapped at column 79 (log lines 239-240).
            (
                Warning,
                239,
                Some("main.tex"),
                Some(12),
                "Reference 'sec:a-very-long-label-name-that-makes-the-warning-wrap-around' on page 1 undefined",
            ),
            (
                Error,
                242,
                Some("main.tex"),
                Some(13),
                "Undefined control sequence.",
            ),
            (
                Error,
                251,
                Some("main.tex"),
                Some(14),
                "Missing $ inserted.",
            ),
            (
                Error,
                262,
                Some("parts/part-one.tex"),
                Some(1),
                "Missing $ inserted.",
            ),
            (
                Warning,
                271,
                Some("parts/part-one.tex"),
                None,
                "Package fakepkg: Something odd happened here.",
            ),
            (
                Error,
                277,
                Some("parts/part-one.tex"),
                Some(5),
                "Undefined control sequence.",
            ),
            (
                Error,
                289,
                Some("main.tex"),
                Some(17),
                "Paragraph ended before \\shortarg was complete.",
            ),
            (
                Warning,
                299,
                Some("main.tex"),
                Some(18),
                "Font shape 'T1/cmr/m/scit' undefined using 'T1/cmr/m/n' instead",
            ),
            (
                BadBox,
                303,
                Some("main.tex"),
                Some(19),
                "Overfull \\hbox (126.87044pt too wide) detected at line 19",
            ),
            (
                BadBox,
                308,
                Some("main.tex"),
                Some(20),
                "Overfull \\vbox (1422.63779pt too high) detected at line 20",
            ),
            (
                BadBox,
                312,
                Some("main.tex"),
                Some(18),
                "Underfull \\hbox (badness 10000) in paragraph at lines 18--22",
            ),
            (
                BadBox,
                319,
                Some("main.tex"),
                None,
                "Underfull \\vbox (badness 10000) has occurred while \\output is active",
            ),
            (
                Warning,
                322,
                Some("main.tex"),
                Some(28),
                "pdfTeX warning (ext4): destination with the same identifier (name{page.1}) has been already used, duplicate ignored",
            ),
            (
                BadBox,
                330,
                Some("main.tex"),
                None,
                "Overfull \\vbox (303.58267pt too high) has occurred while \\output is active",
            ),
            (
                Warning,
                335,
                Some("main.tex"),
                None,
                "Some font shapes were not available, defaults substituted.",
            ),
            (
                Warning,
                338,
                Some("main.tex"),
                None,
                "There were undefined references.",
            ),
            (
                Warning,
                341,
                Some("main.tex"),
                None,
                "Label(s) may have changed. Rerun to get cross-references right.",
            ),
            (
                Warning,
                344,
                Some("main.tex"),
                None,
                "Package rerunfilecheck: File 'main.out' has changed. Rerun to get outlines right or use package 'bookmark'.",
            ),
        ],
    );
    assert!(report.rerun_suggested);
    assert!(!report.fatal);
    assert_eq!(report.output_pages, Some(3));

    let runaway = find(&report.issues, 289);
    assert!(
        runaway
            .detail
            .starts_with("Runaway argument?\n{This is never closed (really.")
    );
    assert!(runaway.detail.contains("l.17"));
    let duplicate = find(&report.issues, 322);
    assert!(duplicate.detail.contains("l.28 \\clearpage"));
    let shown = find(&report.issues, 303);
    assert_eq!(
        shown.detail,
        "\\T1/cmr/m/n/10 Wide box contents here (unbalanced"
    );
}

#[test]
fn missing_package_is_fatal() {
    for (case, first, stop) in [
        ("missing-package", 86, 92),
        ("missing-package-plain", 85, 91),
    ] {
        let report = log(case);
        assert_issues(
            &report.issues,
            &[
                (
                    Error,
                    first,
                    Some("main.tex"),
                    Some(4),
                    "File 'doesnotexist.sty' not found.",
                ),
                (Error, stop, Some("main.tex"), Some(4), "Emergency stop."),
            ],
        );
        assert!(report.fatal, "{case}");
        assert_eq!(report.output_pages, None);
        assert!(
            find(&report.issues, stop)
                .detail
                .contains("*** (cannot \\read from terminal")
        );
    }
}

#[test]
fn missing_begin_document() {
    let report = log("no-document");
    assert_issues(
        &report.issues,
        &[
            (
                Error,
                85,
                Some("main.tex"),
                Some(3),
                "Missing \\begin{document}.",
            ),
            (
                Error,
                97,
                Some("main.tex"),
                Some(4),
                "Missing \\begin{document}.",
            ),
            (
                Error,
                115,
                Some("main.tex"),
                Some(5),
                "I can't find file 'main.aux'.",
            ),
            (Error, 122, Some("main.tex"), Some(5), "Emergency stop."),
        ],
    );
    assert!(report.fatal);
    // LaTeX's boilerplate lines are dropped from the detail; the context is kept.
    let first = &report.issues[0].detail;
    assert!(first.starts_with("l.3 S\n     ome text before the document."));
    assert!(!first.contains("See the LaTeX manual"));
}

#[test]
fn missing_end_document() {
    let report = log("no-end");
    assert_issues(
        &report.issues,
        &[(Error, 50, None, None, "Emergency stop.")],
    );
    assert!(report.fatal);
    assert!(
        report.issues[0]
            .detail
            .contains("*** (job aborted, no legal \\end found)")
    );
}

#[test]
fn xelatex_missing_character_and_xdv_output() {
    let report = log("xelatex");
    assert_issues(
        &report.issues,
        &[
            // Wrapped at 79 characters (81 bytes) on log lines 153-154.
            (
                Warning,
                153,
                Some("main.tex"),
                None,
                "Missing character: There is no 中 (U+4E2D) in font [lmroman10-regular]:mapping=tex-text;!",
            ),
            (
                Error,
                155,
                Some("main.tex"),
                Some(5),
                "Undefined control sequence.",
            ),
            (
                BadBox,
                165,
                Some("main.tex"),
                Some(6),
                "Overfull \\hbox (80.47617pt too wide) detected at line 6",
            ),
        ],
    );
    assert_eq!(report.output_pages, Some(1));
    assert_eq!(
        report.output_file.as_deref(),
        Some(Path::new("/project/main.xdv"))
    );
}

#[test]
fn xelatex_fontspec_error() {
    let report = log("xelatex-font");
    let font = "Package fontspec: The font \"NoSuchFontAnywhere\" cannot be found.";
    assert_issues(
        &report.issues,
        &[
            (Error, 72, Some("main.tex"), Some(4), font),
            (Error, 100, Some("main.tex"), Some(4), font),
            (Error, 116, Some("main.tex"), Some(4), font),
            (
                Error,
                143,
                Some("main.tex"),
                Some(4),
                "Font TU/NoSuchFontAnywhere(0)/m/n/10=NoSuchFontAnywhere at 10.0pt not loadable: Metric (TFM) file or installed font not found.",
            ),
            (
                Warning,
                244,
                Some("main.tex"),
                None,
                "Missing character: There is no T (\"54) in font nullfont!",
            ),
            (
                Warning,
                245,
                Some("main.tex"),
                None,
                "Missing character: There is no e (\"65) in font nullfont!",
            ),
            (
                Warning,
                246,
                Some("main.tex"),
                None,
                "Missing character: There is no x (\"78) in font nullfont!",
            ),
            (
                Warning,
                247,
                Some("main.tex"),
                None,
                "Missing character: There is no t (\"74) in font nullfont!",
            ),
            (
                Warning,
                248,
                Some("main.tex"),
                None,
                "Missing character: There is no . (\"2E) in font nullfont!",
            ),
            (
                Warning,
                249,
                Some("main.tex"),
                None,
                "Missing character: There is no 1 (\"31) in font nullfont!",
            ),
        ],
    );
    assert!(!report.fatal);
}

#[test]
fn lualatex_lua_error_and_80_column_wrapping() {
    let report = log("lualatex");
    assert_issues(
        &report.issues,
        &[
            (
                Error,
                187,
                Some("main.tex"),
                Some(6),
                "Undefined control sequence.",
            ),
            (
                Error,
                196,
                Some("main.tex"),
                Some(7),
                "[\\directlua]:1: deliberate lua error",
            ),
            (
                Warning,
                208,
                Some("main.tex"),
                None,
                "Missing character: There is no 中 (U+4E2D) in font [lmroman10-regular]:+tlig;!",
            ),
        ],
    );
    let lua = find(&report.issues, 196);
    assert!(lua.detail.starts_with("stack traceback:"));
    assert!(
        lua.detail
            .contains("l.7 \\directlua{error(\"deliberate lua error\")}")
    );
    assert_eq!(report.output_pages, Some(1));
}

#[test]
fn biblatex_asks_for_biber() {
    let report = log("biblatex");
    assert_issues(
        &report.issues,
        &[
            (
                Warning,
                342,
                Some("main.tex"),
                Some(5),
                "Citation 'knuth84' on page 1 undefined",
            ),
            (
                Warning,
                345,
                Some("main.tex"),
                Some(5),
                "Citation 'nokey' on page 1 undefined",
            ),
            (
                Warning,
                348,
                Some("main.tex"),
                Some(6),
                "Empty bibliography",
            ),
            (
                Warning,
                355,
                Some("main.tex"),
                None,
                "There were undefined references.",
            ),
            (
                Warning,
                358,
                Some("main.tex"),
                None,
                "Package biblatex: Please (re)run Biber on the file: main and rerun LaTeX afterwards.",
            ),
        ],
    );
    assert!(report.rerun_suggested);
}

#[test]
fn bibtex_missing_database() {
    let report = log("bibtex-missing-db");
    assert_issues(
        &report.issues,
        &[
            (
                Warning,
                50,
                Some("main.tex"),
                Some(3),
                "Citation 'something' on page 1 undefined",
            ),
            (
                Warning,
                54,
                Some("main.bbl"),
                Some(3),
                "Empty 'thebibliography' environment",
            ),
            (
                Warning,
                61,
                Some("main.tex"),
                None,
                "There were undefined references.",
            ),
        ],
    );
    let issues = blg("bibtex-missing-db");
    assert_issues(
        &issues,
        &[
            (
                Error,
                5,
                Some("main.aux"),
                Some(4),
                "I couldn't open database file doesnotexist.bib",
            ),
            (
                Error,
                10,
                Some("main.aux"),
                None,
                "I found no database files",
            ),
            (
                Warning,
                11,
                None,
                None,
                "I didn't find a database entry for \"something\"",
            ),
        ],
    );
    assert!(issues[0].detail.contains(": \\bibdata{doesnotexist"));
}

#[test]
fn tectonic_logs() {
    let report = log("tectonic");
    assert_issues(
        &report.issues,
        &[
            (
                Warning,
                56,
                Some("main.tex"),
                Some(4),
                "Reference 'sec:nothere' on page 1 undefined",
            ),
            (
                Error,
                58,
                Some("main.tex"),
                Some(5),
                "Undefined control sequence.",
            ),
            (
                BadBox,
                68,
                Some("main.tex"),
                Some(6),
                "Overfull \\hbox (87.14726pt too wide) detected at line 6",
            ),
            // Tectonic prints `\input` names without the extension: `(parts/extra`.
            (
                Error,
                73,
                Some("parts/extra"),
                Some(2),
                "Undefined control sequence.",
            ),
            (
                Warning,
                86,
                Some("main.tex"),
                None,
                "There were undefined references.",
            ),
        ],
    );
    assert_eq!(report.output_pages, Some(1));

    let halted = log("tectonic-halt");
    assert_issues(
        &halted.issues,
        &[
            (
                Warning,
                57,
                Some("main.tex"),
                Some(4),
                "Reference 'sec:nothere' on page 1 undefined",
            ),
            (
                Error,
                59,
                Some("main.tex"),
                Some(5),
                "Undefined control sequence.",
            ),
        ],
    );
    assert_eq!(halted.output_pages, Some(0));
    assert_eq!(halted.output_file, None);
}

#[test]
fn crlf_logs_parse_the_same() {
    let text = read("latexmk-pdf", "main.log");
    let crlf = text.replace('\n', "\r\n");
    assert_eq!(parse_log(&crlf, Path::new(BASE)), log("latexmk-pdf"));
}

#[test]
fn base_dir_is_used_for_relative_files_only() {
    let report = parse_log(&read("latexmk-pdf", "main.log"), Path::new("/other/dir"));
    assert_eq!(
        report.issues[2].file.as_deref(),
        Some(Path::new("/other/dir/chapters/intro.tex"))
    );
}
