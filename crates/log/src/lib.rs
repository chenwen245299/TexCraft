//! Parser for TeX engine logs (pdfTeX, XeTeX, LuaTeX, also when run by latexmk) and
//! BibTeX/biber `.blg` files.
//!
//! Turns a log into a list of [`Issue`]s (errors, warnings, bad boxes) with the source file and
//! line they belong to, so the UI can list them and jump to the source. Never panics on any
//! input.
//!
//! The parser understands the `-file-line-error` and classic (`! message` + `l.<n>`) error
//! styles, follows TeX's parenthesised file stack (including names wrapped at the 79/80
//! column `max_print_line` limit and names with spaces), and recognises LaTeX, package, class
//! and font warnings, bad boxes, engine warnings (`pdfTeX warning`, `Missing character`), Lua
//! errors, rerun requests and the `Output written on` summary.
#![deny(clippy::indexing_slicing)]
#![cfg_attr(not(test), deny(clippy::string_slice))]

mod blg;
mod paths;
mod tex;
mod text;

use std::path::{Path, PathBuf};

/// How serious an [`Issue`] is. Ordered from most to least severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    /// A TeX error (`! ...` or `file:line: ...` with `-file-line-error`).
    Error,
    /// A LaTeX, package or class warning, or an undefined reference/citation.
    Warning,
    /// An overfull or underfull `\hbox` / `\vbox`.
    BadBox,
    /// Informational (e.g. a package info message worth showing).
    Info,
}

/// One problem found in a log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// How serious it is.
    pub severity: Severity,
    /// The source file it belongs to. Relative paths in the log are resolved against the
    /// `base_dir` passed to the parser (the directory the engine ran in); `./` segments removed.
    pub file: Option<PathBuf>,
    /// 1-based line in `file`, when the log says.
    pub line: Option<u32>,
    /// One-line message, e.g. `Undefined control sequence.` or
    /// `Reference 'sec:intro' on page 1 undefined`. The `LaTeX Warning:` / `LaTeX Error:`
    /// prefixes and a trailing `on input line N.` are dropped, `Package x Warning: m` becomes
    /// `Package x: m` (likewise for classes and errors), wrapped and continuation lines are
    /// joined and TeX's `` `quotes' `` become `'quotes'`.
    pub message: String,
    /// Extra context from the log (the `l.20 \foo` excerpt, help lines); may be empty.
    pub detail: String,
    /// 1-based line in the log where the issue starts (for "show in log").
    pub log_line: u32,
}

/// Everything learned from one log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogReport {
    /// Issues in log order.
    pub issues: Vec<Issue>,
    /// The log asks for another run (labels changed, rerun to get cross-references right, ...).
    pub rerun_suggested: bool,
    /// Pages written, from `Output written on x.pdf (N pages, ...)`.
    pub output_pages: Option<u32>,
    /// The output file named on that line.
    pub output_file: Option<PathBuf>,
    /// The run stopped without finishing (`Emergency stop`, `Fatal error occurred`,
    /// `==> Fatal error occurred, no output PDF file produced!`).
    pub fatal: bool,
}

impl LogReport {
    /// Number of [`Severity::Error`] issues.
    pub fn errors(&self) -> usize {
        self.count(Severity::Error)
    }

    /// Number of [`Severity::Warning`] issues.
    pub fn warnings(&self) -> usize {
        self.count(Severity::Warning)
    }

    /// Number of [`Severity::BadBox`] issues.
    pub fn bad_boxes(&self) -> usize {
        self.count(Severity::BadBox)
    }

    fn count(&self, severity: Severity) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == severity)
            .count()
    }
}

/// Parses a TeX engine `.log`. `base_dir` is the directory the engine ran in.
///
/// Callers holding raw bytes should pass `String::from_utf8_lossy(bytes)`.
pub fn parse_log(log: &str, base_dir: &Path) -> LogReport {
    tex::parse(log, base_dir)
}

/// Parses a BibTeX or biber `.blg` file. `base_dir` resolves relative `.bib` paths.
pub fn parse_blg(blg: &str, base_dir: &Path) -> Vec<Issue> {
    blg::parse(blg, base_dir)
}
