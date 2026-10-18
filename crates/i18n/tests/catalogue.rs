//! Checks the catalogue against the code: every string passed to `tr("…")`, `tr_in(…, "…")` or
//! `trf!("…", …)` anywhere in the workspace has a Simplified Chinese entry, entries keep their
//! key's `{placeholders}`, `trf!` gets exactly the placeholders as arguments, a key never has
//! two different translations, and no entry is left over from a string the code no longer has.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use texcraft_i18n::{Lang, catalogue, has, placeholders};

/// A translatable string found in the code.
#[derive(Debug)]
struct Use {
    file: PathBuf,
    line: usize,
    text: String,
    /// `trf!` argument names, `None` for `tr`.
    args: Option<Vec<String>>,
}

fn crate_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf())
}

fn workspace() -> PathBuf {
    let dir = crate_dir();
    dir.parent()
        .and_then(Path::parent)
        .map_or(dir.clone(), Path::to_path_buf)
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Source files: `crates/*/src`, `crates/*/tests`, `apps/*/src`, except this crate's own (the
/// catalogue, and tests that use strings it deliberately lacks).
fn sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for group in ["crates", "apps"] {
        let Ok(entries) = std::fs::read_dir(workspace().join(group)) else {
            continue;
        };
        for e in entries.flatten() {
            for sub in ["src", "tests"] {
                rust_files(&e.path().join(sub), &mut out);
            }
        }
    }
    let own = crate_dir();
    out.retain(|p| !p.starts_with(&own));
    out.sort();
    out
}

/// A tiny Rust lexer: enough to find string literals outside comments and the identifiers and
/// punctuation around them.
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Punct(char),
}

fn unescape(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('u') => {
                let mut hex = String::new();
                if chars.next() == Some('{') {
                    for h in chars.by_ref() {
                        if h == '}' {
                            break;
                        }
                        hex.push(h);
                    }
                }
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            // Line continuation: the newline and the next line's leading whitespace go.
            Some('\n') => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn lex(src: &str) -> Vec<(Tok, usize)> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let at = |i: usize| chars.get(i).copied();
    while let Some(c) = at(i) {
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c == '/' && at(i + 1) == Some('/') {
            while at(i).is_some_and(|c| c != '\n') {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == Some('*') {
            let mut depth = 0;
            while let Some(c) = at(i) {
                if c == '/' && at(i + 1) == Some('*') {
                    depth += 1;
                    i += 2;
                } else if c == '*' && at(i + 1) == Some('/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if c == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if c == 'r' && (at(i + 1) == Some('"') || at(i + 1) == Some('#')) && {
            let mut j = i + 1;
            while at(j) == Some('#') {
                j += 1;
            }
            at(j) == Some('"')
        } {
            // Raw string r#"…"#.
            let start_line = line;
            let mut j = i + 1;
            let mut hashes = 0;
            while at(j) == Some('#') {
                hashes += 1;
                j += 1;
            }
            j += 1;
            let mut text = String::new();
            while let Some(c) = at(j) {
                if c == '"' && (1..=hashes).all(|k| at(j + k) == Some('#')) {
                    j += 1 + hashes;
                    break;
                }
                if c == '\n' {
                    line += 1;
                }
                text.push(c);
                j += 1;
            }
            toks.push((Tok::Str(text), start_line));
            i = j;
        } else if c == '"' || (c == 'b' && at(i + 1) == Some('"')) {
            let start_line = line;
            let mut j = if c == 'b' { i + 2 } else { i + 1 };
            let mut raw = String::new();
            while let Some(c) = at(j) {
                if c == '\\' {
                    raw.push(c);
                    if let Some(n) = at(j + 1) {
                        if n == '\n' {
                            line += 1;
                        }
                        raw.push(n);
                    }
                    j += 2;
                    continue;
                }
                if c == '"' {
                    j += 1;
                    break;
                }
                if c == '\n' {
                    line += 1;
                }
                raw.push(c);
                j += 1;
            }
            toks.push((Tok::Str(unescape(&raw)), start_line));
            i = j;
        } else if c == '\'' {
            // A char literal ('x', '\n', '"') or a lifetime ('a).
            if at(i + 1) == Some('\\') {
                let mut j = i + 2;
                while at(j).is_some_and(|c| c != '\'') {
                    j += 1;
                }
                i = j + 1;
            } else if at(i + 2) == Some('\'') {
                i += 3;
            } else {
                i += 1;
            }
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i;
            let mut ident = String::new();
            while let Some(c) = at(j).filter(|c| c.is_alphanumeric() || *c == '_') {
                ident.push(c);
                j += 1;
            }
            toks.push((Tok::Ident(ident), line));
            i = j;
        } else if c.is_whitespace() || c.is_ascii_digit() {
            i += 1;
        } else {
            toks.push((Tok::Punct(c), line));
            i += 1;
        }
    }
    toks
}

/// The `name = …` arguments after a `trf!` literal, up to the macro's closing parenthesis.
fn trf_args(toks: &[(Tok, usize)]) -> Vec<String> {
    let mut names = Vec::new();
    let mut depth = 0i32;
    let mut start_of_arg = false;
    let mut k = 0;
    while let Some((t, _)) = toks.get(k) {
        match t {
            Tok::Punct('(' | '[' | '{') => depth += 1,
            Tok::Punct(')' | ']' | '}') => {
                depth -= 1;
                if depth < 0 {
                    break;
                }
            }
            Tok::Punct(',') if depth == 0 => start_of_arg = true,
            Tok::Ident(name) if start_of_arg && depth == 0 => {
                // `name = value` (not `name == value`).
                if toks.get(k + 1).map(|t| &t.0) == Some(&Tok::Punct('='))
                    && toks.get(k + 2).map(|t| &t.0) != Some(&Tok::Punct('='))
                {
                    names.push(name.clone());
                }
                start_of_arg = false;
            }
            _ => start_of_arg = false,
        }
        k += 1;
    }
    names
}

fn uses_in(file: &Path, src: &str) -> Vec<Use> {
    let toks = lex(src);
    let mut out = Vec::new();
    for (k, (t, line)) in toks.iter().enumerate() {
        let Tok::Ident(name) = t else { continue };
        let next = |n: usize| toks.get(k + n).map(|t| &t.0);
        // Skip method calls and definitions (`.tr(`, `fn tr(`).
        let prev = k.checked_sub(1).and_then(|p| toks.get(p)).map(|t| &t.0);
        if prev == Some(&Tok::Punct('.')) || prev == Some(&Tok::Ident("fn".into())) {
            continue;
        }
        match name.as_str() {
            "tr" => {
                if let (Some(Tok::Punct('(')), Some(Tok::Str(s))) = (next(1), next(2)) {
                    out.push(Use {
                        file: file.to_path_buf(),
                        line: *line,
                        text: s.clone(),
                        args: None,
                    });
                }
            }
            // tr_in(lang, "…"): the first string literal before the closing parenthesis.
            "tr_in" if next(1) == Some(&Tok::Punct('(')) => {
                let mut n = 2;
                while let Some(t) = next(n) {
                    match t {
                        Tok::Str(s) => {
                            out.push(Use {
                                file: file.to_path_buf(),
                                line: *line,
                                text: s.clone(),
                                args: None,
                            });
                            break;
                        }
                        Tok::Punct(')') => break,
                        _ => n += 1,
                    }
                }
            }
            "trf" => {
                if let (Some(Tok::Punct('!')), Some(Tok::Punct('(')), Some(Tok::Str(s))) =
                    (next(1), next(2), next(3))
                {
                    out.push(Use {
                        file: file.to_path_buf(),
                        line: *line,
                        text: s.clone(),
                        args: Some(trf_args(toks.get(k + 4..).unwrap_or(&[]))),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

fn all_uses() -> Vec<Use> {
    let mut out = Vec::new();
    for f in sources() {
        if let Ok(src) = std::fs::read_to_string(&f) {
            out.extend(uses_in(&f, &src));
        }
    }
    out
}

fn sorted(v: Vec<&str>) -> Vec<&str> {
    let mut v = v;
    v.sort_unstable();
    v
}

fn location(u: &Use) -> String {
    let rel = u.file.strip_prefix(workspace()).unwrap_or(&u.file);
    format!("{}:{}", rel.display(), u.line)
}

#[test]
fn the_scanner_finds_what_it_should() {
    let src = r##"
        // tr("in a comment")
        /* tr("in /* a nested */ block") */
        let a = tr("plain");
        let b = texcraft_i18n::tr(
            "on the next line \"quoted\" \\end{…}",
        );
        let c = trf!("{n} files in {dir}", n = count(x, y), dir = d.display());
        let d = trf!("{n} item", n = if a == b { 1 } else { 2 },);
        let e = tr_in(Lang::ZhHans, "explicit");
        let f = x.tr("a method");
        let g = '"'; let h = tr("after a quote char");
        let i = tr(r#"raw "string""#);
        let j = tr("continued \
                    line");
        fn tr(x: &str) {}
        let k = tr(name);
    "##;
    let uses = uses_in(Path::new("x.rs"), src);
    let texts: Vec<&str> = uses.iter().map(|u| u.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "plain",
            "on the next line \"quoted\" \\end{…}",
            "{n} files in {dir}",
            "{n} item",
            "explicit",
            "after a quote char",
            "raw \"string\"",
            "continued line",
        ]
    );
    assert_eq!(uses[0].line, 4);
    assert_eq!(uses[1].line, 5);
    assert_eq!(
        uses[2].args.as_deref(),
        Some(&["n".to_string(), "dir".to_string()][..])
    );
    assert_eq!(uses[3].args.as_deref(), Some(&["n".to_string()][..]));
    assert!(uses[0].args.is_none());
}

#[test]
fn every_string_in_the_code_is_translated() {
    let uses = all_uses();
    assert!(
        uses.len() > 50,
        "the scan found only {} strings",
        uses.len()
    );
    let missing: BTreeMap<String, String> = uses
        .iter()
        .filter(|u| !has(Lang::ZhHans, &u.text))
        .map(|u| (u.text.clone(), location(u)))
        .collect();
    assert!(
        missing.is_empty(),
        "{} strings have no zh-Hans entry:\n{}",
        missing.len(),
        missing
            .iter()
            .map(|(t, at)| format!("    ({t:?}, \"\"), // {at}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn trf_arguments_match_the_placeholders() {
    let mut bad = Vec::new();
    for u in all_uses() {
        let expected = sorted(placeholders(&u.text));
        let expected: BTreeSet<&str> = expected.into_iter().collect();
        match &u.args {
            Some(args) => {
                let given: BTreeSet<&str> = args.iter().map(String::as_str).collect();
                if given != expected || given.len() != args.len() {
                    bad.push(format!(
                        "{}: {:?} has {:?}, given {:?}",
                        location(&u),
                        u.text,
                        expected,
                        args
                    ));
                }
            }
            // `tr` with a placeholder would show it unfilled: that needs `trf!`.
            None if !expected.is_empty() => bad.push(format!(
                "{}: {:?} has placeholders {:?}: use trf!",
                location(&u),
                u.text,
                expected
            )),
            None => {}
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn translations_keep_their_placeholders() {
    let mut bad = Vec::new();
    for (en, zh) in catalogue(Lang::ZhHans) {
        if sorted(placeholders(en)) != sorted(placeholders(zh)) {
            bad.push(format!("{en:?} → {zh:?}"));
        }
        if zh.trim().is_empty() || (en.trim() != en) != (zh.trim() != zh) {
            bad.push(format!("{en:?} → {zh:?}: empty or different outer spaces"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_key_has_one_translation() {
    let mut seen: HashMap<&str, &str> = HashMap::new();
    let mut bad = Vec::new();
    for (en, zh) in catalogue(Lang::ZhHans) {
        if let Some(prev) = seen.insert(en, zh)
            && prev != zh
        {
            bad.push(format!("{en:?}: {prev:?} and {zh:?}"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// An entry whose English is not a string literal anywhere in the code is left over (keys
/// used through a variable, such as command labels and template names, are literals where
/// they are defined).
#[test]
fn no_stale_entries() {
    let mut literals = BTreeSet::new();
    for f in sources() {
        if let Ok(src) = std::fs::read_to_string(&f) {
            for (t, _) in lex(&src) {
                if let Tok::Str(s) = t {
                    literals.insert(s);
                }
            }
        }
    }
    let stale: BTreeSet<&str> = catalogue(Lang::ZhHans)
        .into_iter()
        .map(|(en, _)| en)
        .filter(|en| !literals.contains(*en))
        .collect();
    assert!(stale.is_empty(), "entries no code uses: {stale:#?}");
}
