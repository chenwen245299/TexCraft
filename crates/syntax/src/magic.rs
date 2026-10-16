//! `% !TEX ...` magic comments (TeXShop/Texifier convention).

/// Magic comments found at the top of a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MagicComments {
    /// `% !TEX program = xelatex` (also `TS-program`), lowercased.
    pub program: Option<String>,
    /// `% !TEX root = ../main.tex`
    pub root: Option<String>,
    /// `% !TEX spellcheck = en_GB`
    pub spellcheck: Option<String>,
    /// `% !BIB program = biber` / `% !TEX bibprogram = biber`, lowercased.
    pub bib_program: Option<String>,
    /// `% !TEX encoding = UTF-8`
    pub encoding: Option<String>,
}

/// How many lines from the top are scanned, like other editors do.
pub const MAGIC_SCAN_LINES: usize = 30;

/// Reads the magic comments in the first [`MAGIC_SCAN_LINES`] lines of `text`.
pub fn magic_comments(text: &str) -> MagicComments {
    let mut m = MagicComments::default();
    for line in text.lines().take(MAGIC_SCAN_LINES) {
        let l = line.trim_start();
        let Some(body) = l.strip_prefix('%') else {
            continue;
        };
        let body = body.trim_start_matches('%').trim_start();
        let (is_bib, rest) = if let Some(r) = strip_prefix_ci(body, "!TEX") {
            (false, r)
        } else if let Some(r) = strip_prefix_ci(body, "!BIB") {
            (true, r)
        } else {
            continue;
        };
        let Some((key, value)) = rest.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        match (is_bib, key.as_str()) {
            (true, "program" | "ts-program") | (false, "bibprogram" | "bib program") => {
                m.bib_program = Some(value.to_ascii_lowercase());
            }
            (false, "program" | "ts-program") => m.program = Some(value.to_ascii_lowercase()),
            (false, "root") => m.root = Some(value),
            (false, "spellcheck") => m.spellcheck = Some(value),
            (false, "encoding") => m.encoding = Some(value),
            _ => {}
        }
    }
    m
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| s.get(prefix.len()..))
        .flatten()
}

/// Whether `text` looks like a main (root) document: it has `\documentclass` outside a comment.
pub fn is_root_document(text: &str) -> bool {
    text.lines().any(|l| {
        let code = l.split('%').next().unwrap_or("");
        code.contains("\\documentclass")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_magic_comments() {
        let m = magic_comments(
            "% !TEX program = XeLaTeX\n%!TEX root = ../main.tex\n% !BIB program = biber\n% !TEX spellcheck = en_GB\n\\documentclass{article}",
        );
        assert_eq!(m.program.as_deref(), Some("xelatex"));
        assert_eq!(m.root.as_deref(), Some("../main.tex"));
        assert_eq!(m.bib_program.as_deref(), Some("biber"));
        assert_eq!(m.spellcheck.as_deref(), Some("en_GB"));
    }

    #[test]
    fn ts_program_and_non_magic() {
        let m = magic_comments("% !TEX TS-program = lualatex\n% just a comment = no\n");
        assert_eq!(m.program.as_deref(), Some("lualatex"));
        assert_eq!(m.root, None);
    }

    #[test]
    fn root_detection() {
        assert!(is_root_document("\\documentclass{article}"));
        assert!(!is_root_document(
            "% \\documentclass{article}\n\\section{x}"
        ));
    }
}
