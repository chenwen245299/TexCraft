//! Garbage, truncated, mutated and huge inputs never panic; parsing is fast.

// Test code may unwrap and panic (AGENTS.md: tests are exempt from the never-panic rule).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use texcraft_synctex::SyncTex;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
}

fn gz_bytes(rel: &str) -> Vec<u8> {
    std::fs::read(data_dir().join(rel)).unwrap()
}

fn plain_text(rel: &str) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(&gz_bytes(rel)[..])
        .read_to_end(&mut out)
        .unwrap();
    out
}

/// Deterministic xorshift generator (no external crates).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    fn int(&mut self) -> i64 {
        match self.below(6) {
            0 => 0,
            1 => -(self.below(1000) as i64),
            2 => i64::from(i32::MAX) + self.below(3) as i64 - 1,
            3 => i64::from(i32::MIN) + self.below(3) as i64,
            _ => self.below(50_000_000) as i64,
        }
    }
}

/// Runs every public query on `s`; they must not panic.
fn exercise(s: &SyncTex) {
    let inputs: Vec<PathBuf> = s.inputs().iter().map(|p| p.to_path_buf()).collect();
    let mut files = inputs.clone();
    files.push(PathBuf::from("main.tex"));
    files.push(PathBuf::from(""));
    for file in files.iter().take(6) {
        for line in [0, 1, 2, 5, 17, u32::MAX] {
            for r in s.forward(file, line) {
                assert!(r.width >= 0.0 && r.height >= 0.0, "{r:?}");
            }
            let _ = s.forward_with_hint(file, line, 2);
        }
    }
    for page in [0, 1, 2, s.pages(), u32::MAX] {
        for (x, y) in [
            (0.0, 0.0),
            (100.0, 200.0),
            (300.0, 700.0),
            (-5.0, 1e9),
            (f32::NAN, 1.0),
        ] {
            let _ = s.inverse(page, x, y);
            let _ = s.inverse_candidates(page, x, y);
        }
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    for i in 0..2000 {
        let len = rng.below(if i % 10 == 0 { 20_000 } else { 400 });
        let mut bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        if i % 3 == 0 {
            // Give the parser a real header so it reaches the content code.
            let mut v = b"SyncTeX Version:1\nInput:1:a.tex\nContent:\n{1\n".to_vec();
            v.append(&mut bytes);
            bytes = v;
        }
        if i % 7 == 0 {
            bytes.splice(0..0, [0x1f, 0x8b]);
        }
        if let Ok(s) = SyncTex::from_bytes(&bytes) {
            exercise(&s);
        }
    }
}

#[test]
fn truncated_files_never_panic() {
    for rel in [
        "single/single.synctex.gz",
        "twocol/twocol.synctex.gz",
        "xetex/xedoc.synctex.gz",
    ] {
        let text = plain_text(rel);
        let step = (text.len() / 150).max(1);
        let mut parsed = 0;
        for cut in (0..text.len()).step_by(step) {
            if let Ok(s) = SyncTex::from_bytes(&text[..cut]) {
                parsed += 1;
                exercise(&s);
            }
        }
        assert!(
            parsed > 100,
            "{rel}: truncated prefixes should mostly parse ({parsed})"
        );
        // Truncated gzip streams keep what could be inflated.
        let gz = gz_bytes(rel);
        for cut in (0..gz.len()).step_by((gz.len() / 60).max(1)) {
            if let Ok(s) = SyncTex::from_bytes(&gz[..cut]) {
                exercise(&s);
            }
        }
    }
    // A file cut in the middle of page 2 still answers queries about page 1.
    let text = plain_text("single/single.synctex.gz");
    let full = SyncTex::from_bytes(&text).unwrap();
    let page2 = text.windows(3).position(|w| w == b"\n{2").unwrap();
    let cut = SyncTex::from_bytes(&text[..page2 + 500]).unwrap();
    assert_eq!(cut.pages(), 2);
    let file = Path::new("single.tex");
    assert_eq!(cut.forward(file, 15), full.forward(file, 15));
    assert_eq!(cut.inverse(1, 200.0, 300.0), full.inverse(1, 200.0, 300.0));
}

#[test]
fn mutated_files_never_panic() {
    let text = plain_text("multi/main.synctex.gz");
    let lines: Vec<&[u8]> = text.split(|&b| b == b'\n').collect();
    let mut rng = Rng(42);
    for _ in 0..300 {
        let mut out: Vec<Vec<u8>> = lines.iter().map(|l| l.to_vec()).collect();
        for _ in 0..1 + rng.below(20) {
            let i = rng.below(out.len());
            match rng.below(7) {
                0 => {
                    out.remove(i);
                }
                1 => {
                    let l = out[i].clone();
                    out.insert(i, l);
                }
                2 => {
                    let j = rng.below(out.len());
                    out.swap(i, j);
                }
                3 => {
                    // Corrupt a byte.
                    if !out[i].is_empty() {
                        let k = rng.below(out[i].len());
                        out[i][k] = b"0123456789-,:=([{)]}xkghv$rf<>!"[rng.below(31)];
                    }
                }
                4 => {
                    // Replace a number by an extreme one.
                    let n = rng.int().to_string();
                    out[i] = format!("x1,{n}:{n},{n}").into_bytes();
                }
                5 => {
                    let keep = rng.below(out[i].len() + 1);
                    out[i].truncate(keep);
                }
                _ => {
                    let rec = b"([{)]}<>"[rng.below(8)];
                    out.insert(
                        i,
                        vec![rec, b'1', b',', b'3', b':', b'5', b',', b'5', b':', b'9'],
                    );
                }
            }
        }
        let bytes = out.join(&b'\n');
        if let Ok(s) = SyncTex::from_bytes(&bytes) {
            exercise(&s);
        }
    }
}

/// Grammar-based fuzzing: random but well-formed-looking records.
#[test]
fn random_records_never_panic() {
    let mut rng = Rng(7);
    for _ in 0..300 {
        let mut text = String::from("SyncTeX Version:1\nInput:1:/p/./a.tex\nInput:2:b.tex\n");
        text.push_str("Output:pdf\nMagnification:1000\nUnit:1\nX Offset:0\nY Offset:0\nContent:\n");
        let pages = 1 + rng.below(3);
        for page in 1..=pages {
            if rng.below(4) == 0 {
                text.push_str(&format!(
                    "<{}\n(1,1:0,0:100,100,0\nx1,2:5,5\n)\n>\n",
                    rng.below(3)
                ));
            }
            text.push_str(&format!("{{{page}\n"));
            let mut depth = 0;
            for _ in 0..rng.below(200) {
                let tag = 1 + rng.below(3);
                let line = rng.below(30);
                let (h, v, w, ht, d) = (rng.int(), rng.int(), rng.int(), rng.int(), rng.int());
                let rec = match rng.below(14) {
                    0 | 1 => {
                        depth += 1;
                        format!("({tag},{line}:{h},{v}:{w},{ht},{d}")
                    }
                    2 => {
                        depth += 1;
                        format!("[{tag},{line}:{h},{v}:{w},{ht},{d}")
                    }
                    3 if depth > 0 => {
                        depth -= 1;
                        ")".into()
                    }
                    4 if depth > 0 => {
                        depth -= 1;
                        "]".into()
                    }
                    5 => format!("k{tag},{line}:{h},{v}:{w}"),
                    6 => format!("g{tag},{line}:{h},="),
                    7 => format!("${tag},{line},{}:{h},{v}", rng.below(80)),
                    8 => format!("h{tag},{line}:{h},{v}:{w},{ht},{d}"),
                    9 => format!("v{tag},{line}:{h},{v}:{w},{ht},{d}"),
                    10 => format!("r{tag},{line}:{h},{v}:{w},{ht},{d}"),
                    11 => format!("f{}:{h},{v}", rng.below(3)),
                    _ => format!("x{tag},{line}:{h},{v}"),
                };
                text.push_str(&rec);
                text.push('\n');
            }
            for _ in 0..depth {
                text.push_str(if rng.below(2) == 0 { ")\n" } else { "]\n" });
            }
            text.push_str(&format!("}}{page}\n"));
        }
        if rng.below(2) == 0 {
            text.push_str("Postamble:\nCount:1\nPost scriptum:\nMagnification:2\nX Offset:1in\nY Offset:-2cm\n");
        }
        let s = SyncTex::from_bytes(text.as_bytes()).unwrap();
        exercise(&s);
    }
}

#[test]
fn deep_nesting_and_huge_numbers_are_handled() {
    let mut text = String::from("SyncTeX Version:1\nInput:1:a.tex\nContent:\n{1\n");
    for i in 0..100_000 {
        text.push_str(&format!("(1,{}:{i},{i}:100,10,10\n", i % 50));
    }
    text.push_str("x1,7:5,5\nk1,7:99999999999999999999,-99999999999999999999:-2147483648\n");
    for _ in 0..100_000 {
        text.push_str(")\n");
    }
    text.push_str("}1\n");
    let s = SyncTex::from_bytes(text.as_bytes()).unwrap();
    assert_eq!(s.pages(), 1);
    exercise(&s);
    // Lots of records on one line of a single vbox (quadratic traps).
    let mut text =
        String::from("SyncTeX Version:1\nInput:1:a.tex\nContent:\n{1\n[1,1:0,0:100,100,0\n");
    for i in 0..50_000 {
        text.push_str(&format!("g1,3:{i},{i}\n"));
    }
    text.push_str("]\n}1\n");
    let s = SyncTex::from_bytes(text.as_bytes()).unwrap();
    let start = Instant::now();
    assert!(!s.forward(Path::new("a.tex"), 3).is_empty());
    assert!(start.elapsed() < Duration::from_secs(5));
}

/// Builds an uncompressed SyncTeX file of at least `size` bytes by repeating the pages of a
/// real one.
fn big_synctex(size: usize) -> Vec<u8> {
    let text = String::from_utf8(plain_text("twocol/twocol.synctex.gz")).unwrap();
    let start = text.find("\n{1\n").unwrap() + 1;
    let end = text.find("Postamble:").unwrap();
    let (head, body, tail) = (&text[..start], &text[start..end], &text[end..]);
    let mut out = head.to_string();
    let mut page = 0;
    while out.len() < size {
        for line in body.lines() {
            if let Some(n) = line.strip_prefix('{')
                && n.parse::<u32>().is_ok()
            {
                page += 1;
                out.push_str(&format!("{{{page}\n"));
                continue;
            }
            if let Some(n) = line.strip_prefix('}')
                && n.parse::<u32>().is_ok()
            {
                out.push_str(&format!("}}{page}\n"));
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(tail);
    out.into_bytes()
}

#[test]
fn parses_two_megabytes_quickly() {
    let bytes = big_synctex(2 * 1024 * 1024);
    assert!(bytes.len() >= 2 * 1024 * 1024);
    let mut best = Duration::MAX;
    let mut parsed = None;
    for _ in 0..3 {
        let start = Instant::now();
        let s = SyncTex::from_bytes(&bytes).unwrap();
        best = best.min(start.elapsed());
        parsed = Some(s);
    }
    let s = parsed.unwrap();
    assert!(s.pages() >= 30, "{} pages", s.pages());
    // Queries stay cheap too.
    let start = Instant::now();
    for line in 1..=160 {
        let _ = s.forward(Path::new("twocol.tex"), line);
    }
    for page in 1..=50 {
        let _ = s.inverse(page, 200.0, 400.0);
    }
    let queries = start.elapsed();
    eprintln!(
        "parsed {} bytes ({} pages) in {best:?}; 160 forward + 50 inverse queries in {queries:?}",
        bytes.len(),
        s.pages()
    );
    // The 100 ms budget applies to optimized builds; debug builds get more slack.
    let budget = if cfg!(debug_assertions) { 2000 } else { 100 };
    assert!(
        best < Duration::from_millis(budget),
        "parsing took {best:?}"
    );
}
