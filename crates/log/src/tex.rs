//! Parser for `.log` files written by pdfTeX, XeTeX, LuaTeX and Tectonic.
//!
//! The log is walked line by line. Lines that start a known message (errors, warnings, bad
//! boxes, ...) are handed to a block parser that consumes the message together with its
//! continuation and context lines. Every other line is scanned for parentheses, which is how
//! TeX reports the files it opens (`(./chapter.tex` ... `)`); that stack tells which file a
//! message belongs to. Message and context lines are never paren-scanned, because they quote
//! arbitrary source text.

use std::borrow::Cow;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::paths::{has_known_extension, is_file_like, path_like_start, resolve};
use crate::text::{error_message, leading_number, lines, tidy};
use crate::{Issue, LogReport, Severity};

/// Deepest file nesting we track; deeper (garbage) input only keeps a counter.
const MAX_STACK: usize = 4096;
/// Most wrapped continuation lines joined into one name or message.
const MAX_JOIN: usize = 8;
/// How far after an error message we look for its `l.<n>` context line.
const CONTEXT_SCAN: usize = 40;
/// Most help lines kept after an error's context.
const MAX_HELP: usize = 24;
/// Most detail lines kept per issue.
const MAX_DETAIL: usize = 24;
/// Most lines of box content skipped after a bad-box message.
const BOX_SCAN: usize = 200;
/// Most continuation lines joined into one warning.
const MAX_CONTINUATION: usize = 24;
/// Engine warnings, which may also start in the middle of a line.
const ENGINE_WARNINGS: [&str; 3] = ["pdfTeX warning", "LuaTeX warning", "XeTeX warning"];

pub(crate) fn parse(log: &str, base_dir: &Path) -> LogReport {
    let lines = lines(log);
    let wrap = wrap_width(&lines);
    // Tectonic logs have no engine banner and print `\input` names as given, often without
    // an extension (`(parts/extra`).
    let bare_names = !lines.first().is_some_and(|l| l.starts_with("This is "));
    let mut parser = Parser {
        lines,
        wrap,
        bare_names,
        base_dir,
        stack: Vec::new(),
        overflow: 0,
        files: Vec::new(),
        report: LogReport::default(),
        runaway: None,
    };
    parser.run();
    parser.report
}

/// The column at which the engine hard-wrapped the log (`max_print_line`): 79 for TeX, 80 for
/// LuaTeX. When the log has many longer lines it was not wrapped at all.
fn wrap_width(lines: &[&str]) -> usize {
    let width = if lines.first().is_some_and(|l| l.starts_with("This is Lua")) {
        80
    } else {
        79
    };
    let limit = width + 2;
    let long = lines
        .iter()
        .skip(1)
        .filter(|l| l.len() > limit && l.chars().count() > limit)
        .take(5)
        .count();
    if long >= 5 { usize::MAX } else { width }
}

/// What a log line starts.
enum Kind {
    /// Banner or command line: nothing to report, nothing to scan.
    Skip,
    /// `! message`.
    Bang,
    /// `file:line: message` (`-file-line-error`); `msg` is the message offset in the line
    /// joined with its wrapped continuations (long paths wrap too).
    FileLine { path: String, line: u32, msg: usize },
    /// A Lua error (`[\directlua]:1: ...` followed by a traceback).
    Lua,
    /// `Runaway argument?` and friends; the argument text follows, then the error.
    Runaway,
    /// `LaTeX Warning: ...`, `Package x Warning: ...`, ...
    Warning(Origin),
    /// `Missing character: There is no ...`.
    MissingChar,
    /// `Overfull \hbox ...` and friends.
    BadBox { vbox: bool },
    /// `Output written on x.pdf (N pages, M bytes).`
    Output,
    /// `No pages of output.`
    NoPages,
    /// `pdfTeX warning ...` at the start of the line.
    EngineWarning,
    /// A line that ends the run (`*** (job aborted ...)`, too many errors).
    Fatal,
    /// An orphaned `l.<n>` context line; skipped with its continuation.
    Context,
}

/// Who issued a warning, which decides its continuation-line prefix and message prefix.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OriginKind {
    Latex,
    Package,
    Class,
}

struct Origin {
    kind: OriginKind,
    /// Byte range of the name in the line (`hyperref` in `Package hyperref Warning:`; `Font`
    /// in `LaTeX Font Warning:`; empty for `LaTeX Warning:`).
    name: Range<usize>,
    /// Byte offset of the message body in the line.
    body: usize,
}

/// The context found after an error message.
struct Context {
    line: Option<u32>,
    detail: Vec<String>,
    next: usize,
    /// Location borrowed from a following `Emergency stop` (file from its `-file-line-error`
    /// prefix, line from its context).
    borrowed: Option<(Option<PathBuf>, Option<u32>)>,
}

struct Parser<'a> {
    lines: Vec<&'a str>,
    wrap: usize,
    /// Accept extension-less names after `(` (see [`bare_name_at`]).
    bare_names: bool,
    base_dir: &'a Path,
    /// Open parentheses: `Some(index into files)` for a file, `None` for a plain parenthesis.
    stack: Vec<Option<usize>>,
    /// Opens beyond [`MAX_STACK`] that were not recorded.
    overflow: usize,
    files: Vec<PathBuf>,
    report: LogReport,
    /// Text of a `Runaway argument?` block, for the error starting at the given line index.
    runaway: Option<(usize, String)>,
}

impl<'a> Parser<'a> {
    fn line(&self, i: usize) -> &'a str {
        self.lines.get(i).copied().unwrap_or("")
    }

    /// The line was cut by the engine at the wrap column and continues on the next line.
    fn is_wrapped(&self, i: usize) -> bool {
        let Some(line) = self.lines.get(i) else {
            return false;
        };
        let len = line.len();
        // A char is at most 4 bytes, so longer lines cannot have `wrap` chars; this also keeps
        // the char count cheap on huge lines.
        len >= self.wrap
            && len <= self.wrap.saturating_mul(4)
            && (len == self.wrap || line.chars().count() == self.wrap)
    }

    /// Line `i` joined with the lines it wraps onto. Returns the text and the last line used.
    fn joined(&self, i: usize, stop_at_context: bool) -> (Cow<'a, str>, usize) {
        let mut text = Cow::Borrowed(self.line(i));
        let mut last = i;
        for _ in 0..MAX_JOIN {
            if !self.is_wrapped(last) {
                break;
            }
            let Some(next) = self.lines.get(last + 1).copied() else {
                break;
            };
            if next.is_empty() || (stop_at_context && is_context_start(next)) {
                break;
            }
            text.to_mut().push_str(next);
            last += 1;
        }
        (text, last)
    }

    fn run(&mut self) {
        let n = self.lines.len();
        let (mut i, mut off) = (0, 0);
        while i < n {
            let (next_i, next_off) = if off == 0 {
                match self.message(i) {
                    Some(next) => (next, 0),
                    None => self.scan(i, 0),
                }
            } else {
                self.scan(i, off)
            };
            // Always make progress, whatever the block parsers returned.
            if (next_i, next_off) <= (i, off) {
                i += 1;
                off = 0;
            } else {
                (i, off) = (next_i, next_off);
            }
        }
    }

    // ----------------------------------------------------------------------------------
    // File tracking
    // ----------------------------------------------------------------------------------

    fn push(&mut self, file: Option<PathBuf>) {
        if self.stack.len() >= MAX_STACK {
            self.overflow = self.overflow.saturating_add(1);
            return;
        }
        let entry = file.map(|f| {
            self.files.push(f);
            self.files.len() - 1
        });
        self.stack.push(entry);
    }

    fn pop(&mut self) {
        if self.overflow > 0 {
            self.overflow -= 1;
        } else {
            self.stack.pop();
        }
    }

    /// The innermost open file.
    fn current_file(&self) -> Option<PathBuf> {
        let index = self.stack.iter().rev().find_map(|entry| *entry)?;
        self.files.get(index).cloned()
    }

    /// Scans line `i` from byte `off` for parentheses. Returns where to continue: normally the
    /// next line, or the middle of a later line when a wrapped file name ran onto it.
    fn scan(&mut self, i: usize, off: usize) -> (usize, usize) {
        let line = self.line(i);
        let Some(rest) = line.get(off..) else {
            return (i + 1, 0);
        };
        let warning_at = find_engine_warning(rest).map(|p| off + p);
        let end = warning_at.unwrap_or(line.len());
        let bytes = line.as_bytes();
        let mut p = off;
        while p < end {
            let Some(found) = bytes
                .get(p..end)
                .and_then(|s| s.iter().position(|&b| b == b'(' || b == b')'))
            else {
                break;
            };
            p += found;
            if bytes.get(p) == Some(&b')') {
                self.pop();
                p += 1;
                continue;
            }
            match self.open_paren(i, p + 1) {
                Some((next_i, next_off)) if next_i != i => return (next_i, next_off),
                Some((_, next_off)) => p = next_off.max(p + 1),
                None => p += 1,
            }
        }
        match warning_at {
            Some(col) => (self.engine_warning(i, col), 0),
            None => (i + 1, 0),
        }
    }

    /// Handles a `(` at byte `start - 1` of line `i`: pushes the file it opens (or a plain
    /// parenthesis). For a file, returns the position just after its name.
    fn open_paren(&mut self, i: usize, start: usize) -> Option<(usize, usize)> {
        let rest = self.line(i).get(start..).unwrap_or("");
        // Join the lines a long name may have been wrapped onto.
        let mut logical = Cow::Borrowed(rest);
        let mut bounds = Vec::new();
        let mut j = i;
        while bounds.len() < MAX_JOIN
            && self.is_wrapped(j)
            && j + 1 < self.lines.len()
            && !logical.contains(['(', ')'])
        {
            bounds.push(logical.len());
            j += 1;
            logical.to_mut().push_str(self.line(j));
        }
        let found = file_name_at(&logical, &bounds).or_else(|| {
            if self.bare_names {
                bare_name_at(&logical)
            } else {
                None
            }
        });
        let Some((name, end)) = found else {
            self.push(None);
            return None;
        };
        let path = resolve(logical.get(name).unwrap_or(""), self.base_dir);
        self.push(Some(path));
        let mut position = (i, start + end);
        for (k, &bound) in bounds.iter().enumerate() {
            if end >= bound {
                position = (i + k + 1, end - bound);
            }
        }
        Some(position)
    }

    // ----------------------------------------------------------------------------------
    // Messages
    // ----------------------------------------------------------------------------------

    fn classify(&self, i: usize) -> Option<Kind> {
        let line = self.line(i);
        let first = *line.as_bytes().first()?;
        match first {
            b'!' => return Some(Kind::Bang),
            b'T' if i == 0 && line.starts_with("This is ") => return Some(Kind::Skip),
            b'*' if i < 8 && line.starts_with("**") => return Some(Kind::Skip),
            b'*' if line.starts_with("*** (job aborted") => return Some(Kind::Fatal),
            b'(' if line.starts_with("(That makes 100 errors") => return Some(Kind::Fatal),
            b'R' if line.starts_with("Runaway ") && line.trim_end().ends_with('?') => {
                return Some(Kind::Runaway);
            }
            b'M' if line.starts_with("Missing character: ") => return Some(Kind::MissingChar),
            b'O' if line.starts_with("Output written on ") => return Some(Kind::Output),
            b'N' if line.starts_with("No pages of output.") => return Some(Kind::NoPages),
            b'l' if context_line(line).is_some() => return Some(Kind::Context),
            _ => {}
        }
        if let Some(origin) = warning_origin(line) {
            return Some(Kind::Warning(origin));
        }
        if let Some(vbox) = bad_box(line) {
            return Some(Kind::BadBox { vbox });
        }
        if ENGINE_WARNINGS.iter().any(|w| line.starts_with(w)) {
            return Some(Kind::EngineWarning);
        }
        if is_lua_error(line, self.lines.get(i + 1).copied()) {
            return Some(Kind::Lua);
        }
        self.file_line(i)
            .map(|(path, line, msg, _)| Kind::FileLine { path, line, msg })
    }

    /// The `-file-line-error` prefix of line `i`, which may itself be wrapped. Returns the
    /// path, the line number, the message offset and the joined text.
    fn file_line(&self, i: usize) -> Option<(String, u32, usize, Cow<'a, str>)> {
        let first = self.line(i);
        let text = match split_file_line(first) {
            Some(_) => Cow::Borrowed(first),
            None if self.is_wrapped(i) && !first.starts_with(char::is_whitespace) => {
                self.joined(i, true).0
            }
            None => return None,
        };
        let (path, line, msg) = split_file_line(&text)?;
        let path = text.get(path)?.to_string();
        Some((path, line, msg, text))
    }

    fn starts_message(&self, i: usize) -> bool {
        self.classify(i).is_some()
    }

    /// Parses the message starting at line `i`, if any. Returns the next line to look at.
    fn message(&mut self, i: usize) -> Option<usize> {
        let kind = self.classify(i)?;
        let line = self.line(i);
        Some(match kind {
            Kind::Skip => i + 1,
            Kind::Bang => {
                let msg = line.get(1..).unwrap_or("").trim_start();
                self.error(i, line.len() - msg.len(), None)
            }
            Kind::FileLine {
                path,
                line: number,
                msg,
            } => {
                let file = resolve(&path, self.base_dir);
                self.error(i, msg, Some((file, number)))
            }
            Kind::Lua => self.error(i, 0, None),
            Kind::Runaway => self.runaway(i),
            Kind::Warning(origin) => self.warning(i, &origin),
            Kind::MissingChar => {
                let (text, last) = self.joined(i, false);
                let message = tidy(&text);
                self.issue(Severity::Warning, None, message, Vec::new(), i);
                last + 1
            }
            Kind::BadBox { vbox } => self.bad_box(i, vbox),
            Kind::Output => self.output(i),
            Kind::NoPages => {
                self.report.output_pages = Some(0);
                i + 1
            }
            Kind::EngineWarning => self.engine_warning(i, 0),
            Kind::Fatal => {
                self.report.fatal = true;
                i + 1
            }
            Kind::Context if is_context_continuation(self.line(i + 1)) => i + 2,
            Kind::Context => i + 1,
        })
    }

    fn issue(
        &mut self,
        severity: Severity,
        line: Option<u32>,
        message: String,
        detail: Vec<String>,
        i: usize,
    ) {
        let file = self.current_file();
        self.issue_at(severity, file, line, message, detail, i);
    }

    fn issue_at(
        &mut self,
        severity: Severity,
        file: Option<PathBuf>,
        line: Option<u32>,
        message: String,
        detail: Vec<String>,
        i: usize,
    ) {
        let mut detail = detail;
        detail.truncate(MAX_DETAIL);
        self.report.issues.push(Issue {
            severity,
            file,
            line,
            message,
            detail: detail.join("\n"),
            log_line: u32::try_from(i.saturating_add(1)).unwrap_or(u32::MAX),
        });
    }

    /// An error starting at line `i`, its message at byte `msg` of that line. `location` is
    /// the `-file-line-error` prefix, when there is one.
    fn error(&mut self, i: usize, msg: usize, location: Option<(PathBuf, u32)>) -> usize {
        let (text, last) = self.joined(i, true);
        let raw = text.get(msg..).unwrap_or("").trim();
        // Tectonic doubles the prefix (`! ! LaTeX Error: ...`).
        let raw = raw.strip_prefix("! ").unwrap_or(raw);
        if raw.starts_with("==>") || raw.contains("Fatal error occurred") {
            self.report.fatal = true;
            return last + 1;
        }
        if raw.starts_with("Emergency stop") || raw.starts_with("TeX capacity exceeded") {
            self.report.fatal = true;
        }
        let message = error_message(raw);
        let context = self.context(last + 1, true, true);
        if context
            .detail
            .iter()
            .any(|d| d.starts_with("*** (job aborted"))
        {
            self.report.fatal = true;
        }
        let mut detail = Vec::new();
        if let Some((at, runaway)) = self.runaway.take()
            && at == i
        {
            detail.push(runaway);
        }
        detail.extend(context.detail);
        let (file, line) = match location {
            Some((file, line)) => (Some(file), Some(line)),
            None => {
                let (file, line) = context.borrowed.unwrap_or((None, None));
                (file.or_else(|| self.current_file()), context.line.or(line))
            }
        };
        self.issue_at(Severity::Error, file, line, message, detail, i);
        context.next
    }

    /// Collects the context printed after an error (or engine warning) message, starting at
    /// line `start`: everything up to the final `l.<n>` (or `<*>`) context pair, then, with
    /// `help`, the help text up to the next empty line.
    fn context(&self, start: usize, help: bool, borrow: bool) -> Context {
        let n = self.lines.len();
        let limit = start.saturating_add(CONTEXT_SCAN).min(n);
        let mut detail = Vec::new();
        let mut k = start;
        while k < limit {
            let line = self.line(k);
            if context_line(line).is_some() || line.starts_with("<*>") {
                let number = context_line(line).flatten();
                push_detail(&mut detail, line);
                k += 1;
                // The second context line (indented to the break point) is always printed.
                if let Some(rest) = self.lines.get(k).filter(|l| is_context_continuation(l)) {
                    push_detail(&mut detail, rest);
                    k += 1;
                }
                if help {
                    for _ in 0..MAX_HELP {
                        let Some(line) = self.lines.get(k) else { break };
                        if line.trim().is_empty() {
                            k += 1;
                            break;
                        }
                        // `*** (job aborted ...)` explains an emergency stop: keep it.
                        if (self.starts_message(k) && !line.starts_with("*** "))
                            || is_file_event(line)
                        {
                            break;
                        }
                        push_detail(&mut detail, line);
                        k += 1;
                    }
                }
                return Context {
                    line: number,
                    detail,
                    next: k,
                    borrowed: None,
                };
            }
            if self.starts_message(k) || is_file_event(line) {
                let borrowed = (borrow && self.is_emergency_stop(k)).then(|| self.stop_location(k));
                return self.fallback(start, k, borrowed);
            }
            push_detail(&mut detail, line);
            k += 1;
        }
        self.fallback(start, limit, None)
    }

    /// No final context line: consume up to the first empty line (or `stop`).
    fn fallback(
        &self,
        start: usize,
        stop: usize,
        borrowed: Option<(Option<PathBuf>, Option<u32>)>,
    ) -> Context {
        let mut detail = Vec::new();
        let mut k = start;
        while k < stop {
            let line = self.line(k);
            k += 1;
            if line.trim().is_empty() {
                break;
            }
            push_detail(&mut detail, line);
        }
        Context {
            line: None,
            detail,
            next: k,
            borrowed,
        }
    }

    fn is_emergency_stop(&self, i: usize) -> bool {
        let line = self.line(i);
        if let Some(rest) = line.strip_prefix('!') {
            return rest.trim_start().starts_with("Emergency stop");
        }
        self.file_line(i).is_some_and(|(_, _, msg, text)| {
            text.get(msg..)
                .is_some_and(|m| m.starts_with("Emergency stop"))
        })
    }

    /// File and line of the `Emergency stop` at line `i`.
    fn stop_location(&self, i: usize) -> (Option<PathBuf>, Option<u32>) {
        if let Some((path, number, _, _)) = self.file_line(i) {
            return (Some(resolve(&path, self.base_dir)), Some(number));
        }
        (None, self.context(i + 1, false, false).line)
    }

    fn runaway(&mut self, i: usize) -> usize {
        let mut text = self.line(i).trim_end().to_string();
        let next = i + 1;
        if next >= self.lines.len() || self.starts_message(next) {
            self.runaway = Some((next, text));
            return next;
        }
        let (argument, last) = self.joined(next, false);
        text.push('\n');
        text.push_str(argument.trim_end());
        self.runaway = Some((last + 1, text));
        last + 1
    }

    fn warning(&mut self, i: usize, origin: &Origin) -> usize {
        let first = self.line(i);
        let name = first.get(origin.name.clone()).unwrap_or("");
        let mut text = String::from(first);
        let mut k = i;
        for _ in 0..MAX_CONTINUATION {
            let Some(next) = self.lines.get(k + 1).copied() else {
                break;
            };
            if self.is_wrapped(k) {
                if !next.is_empty() {
                    text.push_str(next);
                    k += 1;
                    continue;
                }
                // The text ended exactly at the wrap column, so TeX printed an empty line
                // before the next `\MessageBreak` line.
                match self
                    .lines
                    .get(k + 2)
                    .and_then(|l| continuation(origin.kind, name, l))
                {
                    Some(rest) => {
                        text.push(' ');
                        text.push_str(rest);
                        k += 2;
                        continue;
                    }
                    None => break,
                }
            }
            match continuation(origin.kind, name, next) {
                Some(rest) => {
                    text.push(' ');
                    text.push_str(rest);
                    k += 1;
                }
                None => break,
            }
        }
        let (body, line) = split_input_line(text.get(origin.body..).unwrap_or(""));
        let lower = body.to_ascii_lowercase();
        if lower.contains("rerun") || lower.contains("re-run") {
            self.report.rerun_suggested = true;
        }
        let message = match origin.kind {
            OriginKind::Latex => tidy(body),
            OriginKind::Package => format!("Package {name}: {}", tidy(body)),
            OriginKind::Class => format!("Class {name}: {}", tidy(body)),
        };
        self.issue(Severity::Warning, line, message, Vec::new(), i);
        k + 1
    }

    fn bad_box(&mut self, i: usize, vbox: bool) -> usize {
        let header = self.line(i).trim_end();
        let header = header.strip_suffix("[]").unwrap_or(header);
        let line = box_line(header);
        let message = tidy(header);
        let n = self.lines.len();
        let mut detail = Vec::new();
        let mut k = i + 1;
        // An \hbox message is followed by the box text (possibly an empty line), then the box
        // display and an empty line; a \vbox message only by the display and an empty line.
        if !vbox && k < n && !self.starts_message(k) {
            let (shown, last) = self.joined(k, false);
            if !shown.trim().is_empty() {
                detail.push(shown.trim_end().to_string());
            }
            k = last + 1;
        }
        let limit = k.saturating_add(BOX_SCAN).min(n);
        while k < limit {
            if self.line(k).trim().is_empty() {
                k += 1;
                break;
            }
            if self.starts_message(k) {
                break;
            }
            k += 1;
        }
        self.issue(Severity::BadBox, line, message, detail, i);
        k
    }

    fn output(&mut self, i: usize) -> usize {
        let (text, last) = self.joined(i, false);
        let rest = text
            .strip_prefix("Output written on ")
            .unwrap_or("")
            .trim_end();
        let rest = rest.strip_suffix('.').unwrap_or(rest);
        if let Some(open) = rest.rfind(" (")
            && let (Some(name), Some(inner)) = (rest.get(..open), rest.get(open + 2..))
        {
            self.report.output_file = Some(resolve(name, self.base_dir));
            self.report.output_pages = leading_number(inner);
        }
        last + 1
    }

    /// `pdfTeX warning (...): ...` starting at byte `col` of line `i`, possibly followed by
    /// TeX context lines that give the source line.
    fn engine_warning(&mut self, i: usize, col: usize) -> usize {
        let (text, last) = self.joined(i, true);
        let message = tidy(text.get(col..).unwrap_or(""));
        let (line, detail, next) = if self
            .lines
            .get(last + 1)
            .is_some_and(|l| is_context_start(l))
        {
            let context = self.context(last + 1, false, false);
            (context.line, context.detail, context.next)
        } else {
            (None, Vec::new(), last + 1)
        };
        self.issue(Severity::Warning, line, message, detail, i);
        next
    }
}

// --------------------------------------------------------------------------------------
// Line recognisers
// --------------------------------------------------------------------------------------

/// Byte offset of the first engine warning (`pdfTeX warning`, ...) in `s`.
fn find_engine_warning(s: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(found) = s.get(from..)?.find("TeX warning") {
        let at = from + found;
        if let Some(start) = ENGINE_WARNINGS.iter().find_map(|w| {
            let prefix = w.len().checked_sub("TeX warning".len())?;
            let start = at.checked_sub(prefix)?;
            s.get(start..)?.starts_with(w).then_some(start)
        }) {
            return Some(start);
        }
        from = at + 1;
    }
    None
}

/// `l.<n> ...`: the source line TeX was reading. `Some(None)` when the number overflows.
fn context_line(line: &str) -> Option<Option<u32>> {
    let rest = line.strip_prefix("l.")?;
    let digits = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if digits == 0 {
        return None;
    }
    let after = rest.get(digits..)?;
    if !(after.is_empty() || after.starts_with(' ')) {
        return None;
    }
    Some(rest.get(..digits).and_then(|d| d.parse().ok()))
}

/// The second line of a TeX context pair: indentation, then the unread source text.
fn is_context_continuation(line: &str) -> bool {
    line.is_empty() || line.starts_with(' ')
}

/// A line that begins TeX's error context (`<argument> ...`, `l.12 ...`, `\macro #1->...`).
fn is_context_start(line: &str) -> bool {
    line.starts_with('<')
        || context_line(line).is_some()
        || (line.starts_with('\\') && line.contains("->"))
}

/// A line that opens or closes a file, or ships out a page: the end of any error context.
fn is_file_event(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with(')')
        || t.strip_prefix('[')
            .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
        || t.strip_prefix('(')
            .is_some_and(|r| file_name_at(r, &[]).is_some())
}

fn push_detail(detail: &mut Vec<String>, line: &str) {
    let t = line.trim_end();
    let trimmed = t.trim();
    if trimmed.is_empty()
        || trimmed == "..."
        || trimmed == "See the LaTeX manual or LaTeX Companion for explanation."
        || trimmed == "Type  H <return>  for immediate help."
        || trimmed == "For immediate help type H <return>."
    {
        return;
    }
    detail.push(t.to_string());
}

/// `LaTeX Warning: `, `LaTeX Font Warning: `, `Package foo Warning: `, `Class foo Warning: `.
fn warning_origin(line: &str) -> Option<Origin> {
    let (kind, rest) = if let Some(rest) = line.strip_prefix("LaTeX ") {
        (OriginKind::Latex, rest)
    } else if let Some(rest) = line.strip_prefix("Package ") {
        (OriginKind::Package, rest)
    } else if let Some(rest) = line.strip_prefix("Class ") {
        (OriginKind::Class, rest)
    } else {
        return None;
    };
    let name_start = line.len() - rest.len();
    let (name_len, after) = match rest.strip_prefix("Warning:") {
        Some(after) => (0, after),
        None => {
            let space = rest.find(' ')?;
            (space, rest.get(space + 1..)?.strip_prefix("Warning:")?)
        }
    };
    if kind != OriginKind::Latex && name_len == 0 {
        return None;
    }
    let body = line.len() - after.len();
    Some(Origin {
        kind,
        name: name_start..name_start + name_len,
        body,
    })
}

/// The rest of a warning continuation line: `(name)   text` for named origins, an indented
/// line for plain `LaTeX Warning:` messages.
fn continuation<'l>(kind: OriginKind, name: &str, line: &'l str) -> Option<&'l str> {
    if name.is_empty() {
        if kind == OriginKind::Latex && line.starts_with("    ") {
            let t = line.trim();
            return (!t.is_empty()).then_some(t);
        }
        return None;
    }
    let rest = line
        .strip_prefix('(')?
        .strip_prefix(name)?
        .strip_prefix(')')?;
    Some(rest.trim())
}

/// Splits a trailing ` on input line N.` off a warning body.
fn split_input_line(body: &str) -> (&str, Option<u32>) {
    const MARKER: &str = " on input line ";
    let t = body.trim();
    let without_dot = t.strip_suffix('.').unwrap_or(t);
    if let Some(pos) = without_dot.rfind(MARKER)
        && let (Some(head), Some(number)) = (
            without_dot.get(..pos),
            without_dot.get(pos + MARKER.len()..),
        )
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
    {
        return (head, number.parse().ok());
    }
    (t, None)
}

/// `Overfull \hbox (...`, `Underfull \vbox (...`, also `Tight`/`Loose`. Returns `vbox`.
fn bad_box(line: &str) -> Option<bool> {
    let rest = ["Overfull ", "Underfull ", "Tight ", "Loose "]
        .iter()
        .find_map(|w| line.strip_prefix(w))?;
    if rest.starts_with("\\hbox (") {
        Some(false)
    } else if rest.starts_with("\\vbox (") {
        Some(true)
    } else {
        None
    }
}

/// The source line of a bad box: the first of `in paragraph at lines a--b`, or `detected at
/// line n`. `None` for boxes built while `\output` is active.
fn box_line(header: &str) -> Option<u32> {
    [
        " in paragraph at lines ",
        " in alignment at lines ",
        " detected at line ",
    ]
    .iter()
    .find_map(|marker| header.find(marker).map(|p| p + marker.len()))
    .and_then(|p| leading_number(header.get(p..)?))
}

/// A Lua error: `[\directlua]:1: msg`, or any line followed by a Lua `stack traceback:`.
fn is_lua_error(line: &str, next: Option<&str>) -> bool {
    if next.is_some_and(|n| n.trim_end() == "stack traceback:") {
        return true;
    }
    let Some(rest) = line.strip_prefix("[\\") else {
        return false;
    };
    rest.find("]:")
        .and_then(|p| rest.get(p + 2..))
        .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
}

/// `path:line: message` as printed with `-file-line-error`. Returns the byte range of the
/// path, the line number and the byte offset of the message.
fn split_file_line(line: &str) -> Option<(Range<usize>, u32, usize)> {
    let bytes = line.as_bytes();
    let mut search = 0;
    // The path ends at the first `:<digits>:` followed by a space (or the end of the line).
    while let Some(found) = line.get(search..).and_then(|s| s.find(':')) {
        let colon = search + found;
        let digits = bytes
            .get(colon + 1..)?
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let close = colon + 1 + digits;
        if digits > 0
            && bytes.get(close) == Some(&b':')
            && matches!(bytes.get(close + 1), None | Some(b' '))
        {
            let path = line.get(..colon)?;
            if !plausible_path(path) {
                return None;
            }
            let number = line.get(colon + 1..close)?.parse().ok()?;
            let msg = line.get(close + 1..)?;
            let msg_start = line.len() - msg.trim_start().len();
            return Some((0..colon, number, msg_start));
        }
        search = colon + 1;
        if search > 1024 {
            return None;
        }
    }
    None
}

fn plausible_path(path: &str) -> bool {
    !path.starts_with(|c: char| c.is_whitespace() || matches!(c, '[' | '<' | '!'))
        && !path.contains(['(', ')'])
        && is_file_like(path)
}

// --------------------------------------------------------------------------------------
// File names after `(`
// --------------------------------------------------------------------------------------

fn is_name_stop(c: char) -> bool {
    c.is_whitespace() || matches!(c, '(' | ')' | '"')
}

/// Finds the file name at the start of `s` (the text after a `(`). `bounds` are the byte
/// offsets in `s` where wrapped continuation lines were appended. Returns the byte range of
/// the name and the offset just after it.
fn file_name_at(s: &str, bounds: &[usize]) -> Option<(Range<usize>, usize)> {
    if let Some(inner) = s.strip_prefix('"') {
        let close = inner.find('"')?;
        let name = inner.get(..close)?;
        return is_file_like(name).then_some((1..1 + close, close + 2));
    }
    let stop = s.find(is_name_stop).unwrap_or(s.len());
    if stop == 0 {
        return None;
    }
    // A name ending exactly at a wrap boundary may or may not continue on the next line;
    // prefer the longest cut that ends in a well-known extension.
    let cuts = bounds
        .iter()
        .copied()
        .filter(|&b| b > 0 && b < stop)
        .chain([stop]);
    let (mut known, mut plausible) = (None, None);
    for cut in cuts {
        let Some(candidate) = s.get(..cut) else {
            continue;
        };
        if has_known_extension(candidate) {
            known = Some(cut);
        }
        if is_file_like(candidate) {
            plausible = Some(cut);
        }
    }
    let cut = known
        .or_else(|| extend_over_spaces(s, stop))
        .or(plausible)?;
    Some((0..cut, cut))
}

/// An extension-less name (Tectonic): one word of path characters that is not immediately
/// closed again, so `(parts/extra` is a file but `(Font)` is not.
fn bare_name_at(s: &str) -> Option<(Range<usize>, usize)> {
    let stop = s.find(is_name_stop).unwrap_or(s.len());
    let token = s.get(..stop)?;
    let plausible = token.len() >= 2
        && token.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '.')
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '+' | '~'));
    (plausible && !s.get(stop..)?.starts_with(')')).then_some((0..stop, stop))
}

/// A name with spaces (`./sub dir/with space.tex`): extends the first word over up to four
/// more space-separated words until it ends in a well-known extension. Only for names that
/// look like paths, so that prose in parentheses is not taken for a file.
fn extend_over_spaces(s: &str, stop: usize) -> Option<usize> {
    let first = s.get(..stop)?;
    let bytes = s.as_bytes();
    let mut pos = stop;
    for _ in 0..4 {
        if bytes.get(pos) != Some(&b' ') {
            return None;
        }
        let word_start = pos + 1;
        let word_len = s
            .get(word_start..)?
            .find(is_name_stop)
            .unwrap_or(s.len() - word_start);
        if word_len == 0 {
            return None;
        }
        let end = word_start + word_len;
        let candidate = s.get(..end)?;
        if has_known_extension(candidate)
            && (path_like_start(first) || candidate.contains(['/', '\\']))
        {
            return Some(end);
        }
        pos = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &str, bounds: &[usize]) -> Option<String> {
        file_name_at(s, bounds).map(|(r, _)| s.get(r).unwrap().to_string())
    }

    #[test]
    fn file_names_after_parens() {
        assert_eq!(name("./main.tex", &[]).as_deref(), Some("./main.tex"));
        assert_eq!(
            name("./main.aux (./c.aux))", &[]).as_deref(),
            Some("./main.aux")
        );
        assert_eq!(name("ts1cmr.fd", &[]).as_deref(), Some("ts1cmr.fd"));
        assert_eq!(
            name("./sub dir/with space.tex", &[]).as_deref(),
            Some("./sub dir/with space.tex")
        );
        assert_eq!(
            name("\"./a b.tex\" more", &[]).as_deref(),
            Some("./a b.tex")
        );
        for prose in [
            "HO)",
            "Font)",
            "unbalanced",
            "see main.tex)",
            "U+4E2D) in",
            "e.g., `x')",
        ] {
            assert_eq!(name(prose, &[]), None, "{prose}");
        }
    }

    #[test]
    fn bare_names_for_tectonic() {
        assert_eq!(bare_name_at("parts/extra"), Some((0..11, 11)));
        assert_eq!(bare_name_at("extra (x"), Some((0..5, 5)));
        assert_eq!(bare_name_at("Font)  <7>"), None);
        assert_eq!(bare_name_at("x"), None);
        assert_eq!(bare_name_at("e.g., `\\hobx')"), None);
        assert_eq!(bare_name_at("a:b"), None);
    }

    #[test]
    fn wrapped_names_prefer_known_extensions() {
        // `./foo.t` + `ex` is one name; `./foo.tex` + `LaTeX` is a name and new text.
        assert_eq!(name("./foo.tex", &[7]).as_deref(), Some("./foo.tex"));
        assert_eq!(
            name("./foo.texLaTeX Warning: x", &[9]).as_deref(),
            Some("./foo.tex")
        );
        assert_eq!(name("./foo.tex[1]", &[9]).as_deref(), Some("./foo.tex"));
        let wrapped = "/usr/share/texmf/tex/latex/gettitlestring.sty";
        assert_eq!(name(wrapped, &[40]).as_deref(), Some(wrapped));
    }

    #[test]
    fn file_line_prefixes() {
        let l = "./chapters/intro.tex:2: Undefined control sequence.";
        let (path, line, msg) = split_file_line(l).unwrap();
        assert_eq!(&l[path], "./chapters/intro.tex");
        assert_eq!(line, 2);
        assert_eq!(&l[msg..], "Undefined control sequence.");
        let l = "./sub dir/with space.tex:12: Missing $ inserted.";
        assert_eq!(split_file_line(l).map(|(_, n, _)| n), Some(12));
        let l = "C:/Users/me/main.tex:3: Oops";
        assert_eq!(split_file_line(l).map(|(r, ..)| r), Some(0..20));
        assert!(split_file_line("[\\directlua]:1: lua error").is_none());
        assert!(split_file_line("Date: 2026/10/15 10:22:33 x").is_none());
        assert!(split_file_line("l.5 x:3: y").is_none());
        assert!(split_file_line("main.tex:99999999999: overflow").is_none());
        // A wrapped `) (./a.tex` joined with the error line after it is not a path.
        assert!(split_file_line(") (./a.tex./a.tex:3: Undefined").is_none());
    }

    #[test]
    fn engine_warnings_anywhere_in_a_line() {
        assert_eq!(find_engine_warning("[2pdfTeX warning (ext4): x"), Some(2));
        assert_eq!(find_engine_warning("LuaTeX warning: y"), Some(0));
        assert_eq!(find_engine_warning("TeX warning only"), None);
        assert_eq!(find_engine_warning("no warning"), None);
        assert_eq!(find_engine_warning("中TeX warning XeTeX warning"), Some(15));
    }

    #[test]
    fn small_recognisers() {
        assert_eq!(context_line("l.12 \\foo"), Some(Some(12)));
        assert_eq!(context_line("l.7"), Some(Some(7)));
        assert_eq!(context_line("l.x"), None);
        assert_eq!(context_line("l.12x"), None);
        assert_eq!(
            split_input_line("Ref `a' undefined on input line 6."),
            ("Ref `a' undefined", Some(6))
        );
        assert_eq!(split_input_line("Something odd."), ("Something odd.", None));
        assert_eq!(
            box_line("Overfull \\hbox (1pt too wide) in paragraph at lines 10--12"),
            Some(10)
        );
        assert_eq!(
            box_line("Overfull \\hbox (1pt too wide) detected at line 4"),
            Some(4)
        );
        assert_eq!(
            box_line("Underfull \\vbox (badness 10000) has occurred while \\output is active"),
            None
        );
        assert_eq!(
            bad_box("Loose \\hbox (badness 0) in paragraph at lines 1--1"),
            Some(false)
        );
        assert_eq!(
            bad_box("Overfull \\vbox (2pt too high) detected at line 3"),
            Some(true)
        );
        assert_eq!(bad_box("Overfull rule"), None);
        assert!(is_lua_error("[\\directlua]:1: boom", None));
        assert!(is_lua_error("x.lua:3: boom", Some("stack traceback:")));
        assert!(!is_lua_error("[\\directlua]: boom", None));
    }

    #[test]
    fn wrap_width_detection() {
        let tex = ["This is pdfTeX, Version 3", "x"];
        assert_eq!(wrap_width(&tex), 79);
        let lua = ["This is LuaHBTeX, Version 1.15.0", "x"];
        assert_eq!(wrap_width(&lua), 80);
        let long = "y".repeat(200);
        let unwrapped: Vec<&str> = std::iter::repeat_n(long.as_str(), 6).collect();
        assert_eq!(wrap_width(&unwrapped), usize::MAX);
    }
}
