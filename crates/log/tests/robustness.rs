//! The parsers must never panic, whatever they are fed, and must stay fast.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::{Duration, Instant};

use texcraft_log::{parse_blg, parse_log};

fn fixtures() -> Vec<String> {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let mut out = Vec::new();
    let mut cases: Vec<_> = std::fs::read_dir(&data)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    for case in cases {
        for name in ["main.log", "main.blg"] {
            if let Ok(bytes) = std::fs::read(case.join(name)) {
                out.push(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
    }
    assert!(out.len() >= 14, "fixtures missing");
    out
}

/// xorshift64*: deterministic, dependency-free randomness.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn parse_both(text: &str) {
    let base = Path::new("/base");
    let report = parse_log(text, base);
    for issue in &report.issues {
        assert!(issue.log_line >= 1);
    }
    let _ = parse_blg(text, base);
}

#[test]
fn truncated_fixtures_never_panic() {
    for text in fixtures() {
        let bytes = text.as_bytes();
        let step = (bytes.len() / 150).max(1);
        for cut in (0..bytes.len()).step_by(step) {
            parse_both(&String::from_utf8_lossy(&bytes[..cut]));
            parse_both(&String::from_utf8_lossy(&bytes[cut..]));
        }
    }
}

#[test]
fn shuffled_fixture_lines_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let all = fixtures();
    let lines: Vec<&str> = all.iter().flat_map(|t| t.lines()).collect();
    for _ in 0..200 {
        let count = rng.below(400);
        let text: Vec<&str> = (0..count).map(|_| lines[rng.below(lines.len())]).collect();
        parse_both(&text.join("\n"));
    }
}

#[test]
fn random_garbage_never_panics() {
    const TOKENS: &[&str] = &[
        "(",
        ")",
        "((",
        "))",
        "\n",
        "\r\n",
        " ",
        "  ",
        "! ",
        "!",
        "l.",
        "l.12 ",
        "l.99999999999 ",
        ":",
        ":12: ",
        "./a.tex",
        "./a.tex:3: ",
        "(./sub dir/x.tex",
        "\"",
        "(\"./q z.tex\"",
        "LaTeX Warning: ",
        "Package p Warning: ",
        "(p)",
        "Class c Warning: ",
        "LaTeX Font Warning: ",
        "(Font)",
        " on input line ",
        "7.",
        "Overfull \\hbox (",
        "Underfull \\vbox (",
        "pt too wide) ",
        " in paragraph at lines ",
        "--",
        " detected at line ",
        "[]",
        "Runaway argument?",
        "Output written on ",
        " pages, ",
        "bytes).",
        "Missing character: There is no ",
        "pdfTeX warning (ext4): ",
        "<to be read again> ",
        "<*> ",
        "*** (job aborted",
        "Emergency stop.",
        "==> Fatal error occurred",
        "[\\directlua]:",
        "stack traceback:",
        "Warning--",
        "---line ",
        " of file ",
        "[12] Biber.pm:1> WARN - ",
        "ERROR - ",
        "中",
        "é",
        "\u{0}",
        "\t",
        "This is LuaHBTeX",
        "**",
        ".tex",
        ".sty",
        "/",
        "\\",
        "C:/x.tex:1: ",
    ];
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for round in 0..400 {
        let mut text = String::new();
        let len = rng.below(600);
        for _ in 0..len {
            if rng.below(5) == 0 {
                // A run of filler that may land exactly on the 79/80 wrap column.
                let n = [78, 79, 80, 81, rng.below(120)][rng.below(5)];
                text.extend(std::iter::repeat_n('x', n));
            } else {
                text.push_str(TOKENS[rng.below(TOKENS.len())]);
            }
        }
        if round % 2 == 0 {
            text.insert_str(0, "This is pdfTeX, Version 3\n");
        }
        parse_both(&text);
    }
    for _ in 0..100 {
        let bytes: Vec<u8> = (0..rng.below(4000)).map(|_| rng.next() as u8).collect();
        parse_both(&String::from_utf8_lossy(&bytes));
    }
}

#[test]
fn pathological_shapes_never_panic() {
    parse_both("");
    parse_both("\n\n\n");
    parse_both(&"(".repeat(200_000));
    parse_both(&format!(
        "{}{}",
        "(./a.tex ".repeat(20_000),
        ")".repeat(30_000)
    ));
    parse_both(&"x".repeat(1_000_000));
    parse_both(&format!("(./{}", "y".repeat(79 * 20)));
    let wrapped_lines = format!("({}\n", "z".repeat(78)).repeat(1000);
    parse_both(&wrapped_lines);
    parse_both(&"! Undefined control sequence.\n".repeat(10_000));
    parse_both(&"Runaway argument?\n".repeat(10_000));
    parse_both(&"Overfull \\hbox (1pt too wide) detected at line 1\n".repeat(10_000));
    parse_both(&"LaTeX Warning: x\n                  y\n".repeat(10_000));
    parse_both("! LaTeX Error: x\n\n\n\n");
    parse_both("./main.tex:4: Emergency stop.\n<*> main.tex\n");
}

#[test]
fn large_log_parses_quickly() {
    let base = include_str!("data/latexmk-pdf/main.log");
    let body: String = base.lines().skip(6).map(|l| format!("{l}\n")).collect();
    let mut log = String::from("This is pdfTeX, Version 3.141592653-2.6-1.40.24 (TeX Live 2022)\n");
    while log.lines().count() < 50_000 {
        log.push_str(&body);
    }
    let lines = log.lines().count();
    let mut best = Duration::MAX;
    let mut issues = 0;
    for _ in 0..3 {
        let start = Instant::now();
        let report = parse_log(&log, Path::new("/project"));
        best = best.min(start.elapsed());
        issues = report.issues.len();
    }
    assert!(issues > 1000, "{issues} issues");
    eprintln!("{lines} lines, {issues} issues: {best:?}");
    // 50 ms is the budget for an optimised build; unoptimised test builds get more slack.
    let budget = if cfg!(debug_assertions) {
        Duration::from_secs(2)
    } else {
        Duration::from_millis(50)
    };
    assert!(best < budget, "{lines} lines took {best:?}");
}
