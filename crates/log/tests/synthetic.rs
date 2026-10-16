//! Hand-built logs for the tricky cases (wrapped names, nesting, odd parentheses, other
//! engines' formats). Line wrapping is produced by [`tex_wrap`], which mimics TeX's
//! `max_print_line` behaviour, so the shapes match what the engines really write.

use std::path::{Path, PathBuf};

use texcraft_log::{Issue, LogReport, Severity, parse_blg, parse_log};

const PDFTEX: &str = "This is pdfTeX, Version 3.141592653-2.6-1.40.24 (TeX Live 2022)";

/// Hard-wraps each line after `width` characters like TeX: a line of exactly `width`
/// characters is followed by an empty line (the wrap's newline plus the line's own).
fn tex_wrap(text: &str, width: usize) -> String {
    let mut out = String::new();
    for line in text.split('\n') {
        let mut col = 0;
        for c in line.chars() {
            out.push(c);
            col += 1;
            if col == width {
                out.push('\n');
                col = 0;
            }
        }
        out.push('\n');
    }
    out
}

fn parse(text: &str) -> LogReport {
    parse_log(text, Path::new("/p"))
}

fn at(file: &str) -> Option<PathBuf> {
    Some(Path::new("/p").join(file))
}

/// (file, line, message) of each issue.
fn brief(report: &LogReport) -> Vec<(Option<PathBuf>, Option<u32>, String)> {
    report
        .issues
        .iter()
        .map(|i| (i.file.clone(), i.line, i.message.clone()))
        .collect()
}

#[test]
fn wrapped_name_in_nested_files() {
    let long = format!("chapters/{}.tex", "a-very-long-chapter-file-name".repeat(4));
    let log = tex_wrap(
        &format!(
            "{PDFTEX}\n(./main.tex (./{long}\n! Undefined control sequence.\nl.3 \\foo\n        \n\n)\n\
             ! Undefined control sequence.\nl.9 \\bar\n        \n\n)"
        ),
        79,
    );
    assert!(
        log.lines().any(|l| l.len() == 79),
        "the name must wrap:\n{log}"
    );
    assert_eq!(
        brief(&parse(&log)),
        vec![
            (at(&long), Some(3), "Undefined control sequence.".into()),
            (
                at("main.tex"),
                Some(9),
                "Undefined control sequence.".into()
            ),
        ]
    );
}

#[test]
fn name_ending_at_the_wrap_column() {
    // `(./x...x.tex` is exactly 79 characters, so whatever TeX prints next starts the
    // following line without a separator.
    let name = format!("./{}.tex", "x".repeat(79 - 7));
    let open = format!("({name}");
    assert_eq!(open.len(), 79);
    let warning = "LaTeX Warning: Reference `a' on page 1 undefined on input line 5.";
    let log =
        format!("{PDFTEX}\n{open}\n{warning}\n\n{open}\n[1] (./b.tex\n! Oops.\nl.2 x\n\n))\n");
    let report = parse(&log);
    let wide = at(&name[2..]);
    assert_eq!(
        brief(&report),
        vec![
            (
                wide.clone(),
                Some(5),
                "Reference 'a' on page 1 undefined".into()
            ),
            (at("b.tex"), Some(2), "Oops.".into()),
        ]
    );
}

#[test]
fn nested_files_and_parentheses_that_are_not_files() {
    let log = format!(
        "{PDFTEX}\n(./main.tex (./a.tex (./b.tex) (./c.tex\n\
         Package hyperref Info: Option `x' set (really) on input line 3.\n\
         ! Undefined control sequence.\nl.4 \\foo (unbalanced\n                    \n\n\
         ) (Font) [1] (./d.tex\n\
         LaTeX Warning: Reference `x' on page 1 undefined on input line 2.\n)\n\
         Overfull \\hbox (1.5pt too wide) in paragraph at lines 7--8\n[]\\OT1/cmr/m/n/10 text (with paren\n []\n\n\
         LaTeX Warning: Citation `y' on page 1 undefined on input line 9.\n)\n\
         LaTeX Warning: There were undefined references.\n)\n"
    );
    assert_eq!(
        brief(&parse(&log)),
        vec![
            (at("c.tex"), Some(4), "Undefined control sequence.".into()),
            (
                at("d.tex"),
                Some(2),
                "Reference 'x' on page 1 undefined".into()
            ),
            (
                at("a.tex"),
                Some(7),
                "Overfull \\hbox (1.5pt too wide) in paragraph at lines 7--8".into()
            ),
            (
                at("a.tex"),
                Some(9),
                "Citation 'y' on page 1 undefined".into()
            ),
            (
                at("main.tex"),
                None,
                "There were undefined references.".into()
            ),
        ]
    );
}

#[test]
fn luatex_wraps_at_80_columns() {
    let name = format!("./{}.tex", "lua-name-".repeat(10));
    let log = tex_wrap(
        &format!(
            "This is LuaHBTeX, Version 1.15.0 (TeX Live 2022)\n(./main.tex (/usr/share/texmf/{0}\n)\
             (./{1}\n./{1}:2: Undefined control sequence.\nl.2 \\x\n   \n\n))",
            "y".repeat(90),
            &name[2..]
        ),
        80,
    );
    assert!(log.lines().any(|l| l.chars().count() == 80));
    let report = parse(&log);
    assert_eq!(
        brief(&report),
        vec![(
            at(&name[2..]),
            Some(2),
            "Undefined control sequence.".into()
        )]
    );
}

#[test]
fn warning_text_ending_at_the_wrap_column() {
    let head = "Package foo Warning: ";
    let body = "z".repeat(79 - head.len());
    let log = tex_wrap(
        &format!(
            "{PDFTEX}\n(./main.tex\n{head}{body}\n(foo)                continued on input line 3.\n)"
        ),
        79,
    );
    assert!(log.contains(&format!("{body}\n\n(foo)")), "{log}");
    assert_eq!(
        brief(&parse(&log)),
        vec![(
            at("main.tex"),
            Some(3),
            format!("Package foo: {body} continued")
        )]
    );
}

#[test]
fn quoted_names_and_names_with_spaces() {
    let log = format!(
        "{PDFTEX}\n(./main.tex (\"./sub dir/a b.tex\"\n! Oops.\nl.1 x\n     \n\n) (./my file.tex\n! Again.\nl.2 y\n     \n\n))\n"
    );
    assert_eq!(
        brief(&parse(&log)),
        vec![
            (at("sub dir/a b.tex"), Some(1), "Oops.".into()),
            (at("my file.tex"), Some(2), "Again.".into()),
        ]
    );
}

#[test]
fn unwrapped_logs_are_not_joined() {
    // `max_print_line=10000`: long lines everywhere, and a 79-character line is just a line.
    let filler = format!("Package: filler {}\n", "f".repeat(150)).repeat(6);
    let first = format!("LaTeX Warning: {}", "w".repeat(79 - 15));
    assert_eq!(first.len(), 79);
    let long = format!("{}.tex", "n".repeat(120));
    let log = format!(
        "{PDFTEX}\n{filler}(./main.tex (./{long}\n{first}\nLaTeX Warning: second on input line 4.\n))\n"
    );
    assert_eq!(
        brief(&parse(&log)),
        vec![
            (at(&long), None, "w".repeat(64)),
            (at(&long), Some(4), "second".into()),
        ]
    );
}

#[test]
fn windows_paths() {
    let log = format!(
        "{PDFTEX}\n(C:/Users/me/proj/main.tex\nC:/Users/me/proj/main.tex:5: Undefined control sequence.\nl.5 \\x\n\n)\n"
    );
    let report = parse(&log);
    assert_eq!(report.issues.len(), 1);
    assert_eq!(
        report.issues[0].file.as_deref(),
        Some(Path::new("C:/Users/me/proj/main.tex"))
    );
    assert_eq!(report.issues[0].line, Some(5));
}

#[test]
fn alignment_boxes_and_rerun_requests() {
    let log = format!(
        "{PDFTEX}\n(./main.tex\nOverfull \\hbox (3.0pt too wide) in alignment at lines 10--14\n [] []\n\n\
         Package rerunfilecheck Info: File `main.out' has not changed.\n)\n"
    );
    let report = parse(&log);
    assert_eq!(
        brief(&report),
        vec![(
            at("main.tex"),
            Some(10),
            "Overfull \\hbox (3.0pt too wide) in alignment at lines 10--14".into()
        )]
    );
    assert!(
        !report.rerun_suggested,
        "an Info line is not a rerun request"
    );

    let natbib = format!(
        "{PDFTEX}\n(./main.tex\nPackage natbib Warning: Citation(s) may have changed.\n\
         (natbib)                Rerun to get citations correct.\n\n)\n"
    );
    let report = parse(&natbib);
    assert!(report.rerun_suggested);
    assert_eq!(
        report.issues[0].message,
        "Package natbib: Citation(s) may have changed. Rerun to get citations correct."
    );
}

#[test]
fn error_without_context_does_not_swallow_file_opens() {
    let log = format!(
        "{PDFTEX}\n(./main.tex\n! Something weird.\n(./x.tex\n! Undefined control sequence.\nl.1 \\x\n\n))\n"
    );
    assert_eq!(
        brief(&parse(&log)),
        vec![
            (at("main.tex"), None, "Something weird.".into()),
            (at("x.tex"), Some(1), "Undefined control sequence.".into()),
        ]
    );
}

#[test]
fn runaway_argument_at_end_of_file() {
    // From a real pdflatex log: `\textbf{` never closed.
    let log = format!(
        "{PDFTEX}\n(./main.tex\n))\nRunaway argument?\n{{This is never closed. \\par {{\\fontshape {{scit}}\\selectfont Small caps \\ETC.\n\
         ! File ended while scanning use of \\textbf .\n<inserted text> \n                \\par \n<*> main.tex\n            \n\
         I suspect you have forgotten a `}}', causing me\nto read past where you wanted me to stop.\n\n\
         ! Emergency stop.\n<*> main.tex\n            \n*** (job aborted, no legal \\end found)\n\n \n\
         !  ==> Fatal error occurred, no output PDF file produced!\n"
    );
    let report = parse(&log);
    let messages: Vec<&str> = report.issues.iter().map(|i| i.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "File ended while scanning use of \\textbf .",
            "Emergency stop."
        ]
    );
    assert!(
        report.issues[0]
            .detail
            .starts_with("Runaway argument?\n{This is never closed.")
    );
    assert!(report.fatal);
    assert_eq!(report.issues[0].log_line, 6);
}

#[test]
fn severities_and_counts() {
    let log = format!(
        "{PDFTEX}\n(./main.tex\n! A.\nl.1 x\n\n\nLaTeX Warning: B.\n\nUnderfull \\vbox (badness 10000) has occurred while \\output is active []\n\n)\n\
         Output written on out dir/main.pdf (12 pages, 345 bytes).\n"
    );
    let report = parse(&log);
    let severities: Vec<Severity> = report.issues.iter().map(|i| i.severity).collect();
    assert_eq!(
        severities,
        [Severity::Error, Severity::Warning, Severity::BadBox]
    );
    assert_eq!(
        (report.errors(), report.warnings(), report.bad_boxes()),
        (1, 1, 1)
    );
    assert_eq!(report.output_pages, Some(12));
    assert_eq!(report.output_file, at("out dir/main.pdf"));
}

#[test]
fn biber_blg() {
    // Biber's documented log format (the local biber binary could not run to produce one).
    let blg = "[0] Config.pm:307> INFO - This is Biber 2.17\n\
        [0] Config.pm:310> INFO - Logfile is 'main.blg'\n\
        [120] Utils.pm:410> WARN - BibTeX subsystem: /var/folders/xy/T/biber_tmp_abc/refs.bib_12345.utf8, line 10, syntax error: found \"title\", expected end of entry (\"}\" or \")\") (skipping to next \"@\")\n\
        [130] Biber.pm:131> WARN - I didn't find a database entry for 'nokey' (section 0)\n\
        [135] Biber.pm:131> WARN - Datamodel: Entry 'k' (refs.bib): Missing mandatory field 'author'\n\
        [140] Utils.pm:465> ERROR - BibTeX subsystem: /var/folders/xy/T/biber_tmp_abc/refs.bib_12345.utf8, line 20, syntax error: unexpected end of file\n\
        [150] Biber.pm:133> INFO - WARNINGS: 3\n";
    let issues = parse_blg(blg, Path::new("/p"));
    type BlgRow<'a> = (Severity, Option<PathBuf>, Option<u32>, &'a str, u32);
    let rows: Vec<BlgRow> = issues
        .iter()
        .map(|i: &Issue| {
            (
                i.severity,
                i.file.clone(),
                i.line,
                i.message.as_str(),
                i.log_line,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                Severity::Warning,
                at("refs.bib"),
                Some(10),
                "syntax error: found \"title\", expected end of entry (\"}\" or \")\") (skipping to next \"@\")",
                3
            ),
            (
                Severity::Warning,
                None,
                None,
                "I didn't find a database entry for 'nokey' (section 0)",
                4
            ),
            (
                Severity::Warning,
                at("refs.bib"),
                None,
                "Datamodel: Entry 'k' (refs.bib): Missing mandatory field 'author'",
                5
            ),
            (
                Severity::Error,
                at("refs.bib"),
                Some(20),
                "syntax error: unexpected end of file",
                6
            ),
        ]
    );
}

#[test]
fn bibtex_warning_locations_and_repeated_entries() {
    let blg = [
        "This is BibTeX, Version 0.99d (TeX Live 2022)",
        "Database file #1: refs.bib",
        "Warning--string name \"jan\" is undefined",
        "--line 5 of file refs.bib",
        "Repeated entry---line 20 of file refs.bib",
        " : @book{knuth84",
        " :              ,",
        "I'm skipping whatever remains of this entry",
        "Warning--empty journal in broken",
        "(There was 1 error message)",
    ]
    .join("\n");
    let issues = parse_blg(&blg, Path::new("/p"));
    let rows: Vec<(Severity, Option<PathBuf>, Option<u32>, &str)> = issues
        .iter()
        .map(|i| (i.severity, i.file.clone(), i.line, i.message.as_str()))
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                Severity::Warning,
                at("refs.bib"),
                Some(5),
                "string name \"jan\" is undefined"
            ),
            (Severity::Error, at("refs.bib"), Some(20), "Repeated entry"),
            (Severity::Warning, None, None, "empty journal in broken"),
        ]
    );
    assert_eq!(issues[1].detail.lines().count(), 3);
}
