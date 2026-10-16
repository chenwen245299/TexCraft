//! Parser for BibTeX and biber `.blg` files.

use std::path::{Path, PathBuf};

use crate::paths::resolve;
use crate::text::{leading_number, lines, tidy};
use crate::{Issue, Severity};

/// Most context lines kept for one BibTeX error.
const MAX_DETAIL: usize = 12;

pub(crate) fn parse(blg: &str, base_dir: &Path) -> Vec<Issue> {
    let lines = lines(blg);
    let is_biber = lines
        .iter()
        .find(|l| !l.trim().is_empty())
        .is_some_and(|l| biber_line(l).is_some());
    if is_biber {
        parse_biber(&lines, base_dir)
    } else {
        parse_bibtex(&lines, base_dir)
    }
}

fn log_line(i: usize) -> u32 {
    u32::try_from(i.saturating_add(1)).unwrap_or(u32::MAX)
}

// --------------------------------------------------------------------------------------
// biber: `[123] Utils.pm:410> WARN - message`
// --------------------------------------------------------------------------------------

/// Splits a biber log line into its level and message.
fn biber_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix('[')?;
    let close = rest.find("] ")?;
    if close == 0 || !rest.get(..close)?.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let rest = rest.get(close + 2..)?;
    let rest = rest.get(rest.find("> ")? + 2..)?;
    let dash = rest.find(" - ")?;
    Some((rest.get(..dash)?, rest.get(dash + 3..)?))
}

fn parse_biber(lines: &[&str], base_dir: &Path) -> Vec<Issue> {
    let mut issues = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some((level, message)) = biber_line(line) else {
            continue;
        };
        let severity = match level {
            "WARN" => Severity::Warning,
            "ERROR" => Severity::Error,
            _ => continue,
        };
        let (file, line_no, message) = biber_location(message, base_dir);
        issues.push(Issue {
            severity,
            file,
            line: line_no,
            message: tidy(message),
            detail: String::new(),
            log_line: log_line(i),
        });
    }
    issues
}

/// Pulls the `.bib` file and line out of a biber message. Biber reports parse problems
/// against a temporary copy (`/tmp/.../refs.bib_1234.utf8`); that is mapped back to
/// `refs.bib` in `base_dir`.
fn biber_location<'m>(
    message: &'m str,
    base_dir: &Path,
) -> (Option<PathBuf>, Option<u32>, &'m str) {
    const SUBSYSTEM: &str = "BibTeX subsystem: ";
    if let Some(rest) = message.strip_prefix(SUBSYSTEM)
        && let Some(comma) = rest.find(", ")
        && let (Some(path), Some(after)) = (rest.get(..comma), rest.get(comma + 2..))
    {
        let file = Some(resolve(&clean_biber_path(path), base_dir));
        if let Some(after_line) = after.strip_prefix("line ") {
            let number = leading_number(after_line);
            let text = after_line
                .find(", ")
                .and_then(|p| after_line.get(p + 2..))
                .unwrap_or(after);
            return (file, number, text);
        }
        return (file, None, after);
    }
    // `Datamodel: Entry 'x' (refs.bib): Missing mandatory field 'author'`
    if let Some(end) = message.find(".bib)")
        && let Some(open) = message.get(..end).and_then(|h| h.rfind('('))
        && let Some(name) = message.get(open + 1..end + 4)
    {
        return (Some(resolve(name, base_dir)), None, message);
    }
    (None, None, message)
}

/// `/var/folders/x/T/biber_tmp_1/refs.bib_1234.utf8` -> `refs.bib`.
fn clean_biber_path(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let base = base.strip_suffix(".utf8").unwrap_or(base);
    match base.rfind('_') {
        Some(us)
            if base
                .get(us + 1..)
                .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())) =>
        {
            base.get(..us).unwrap_or(base).to_string()
        }
        _ => base.to_string(),
    }
}

// --------------------------------------------------------------------------------------
// BibTeX
// --------------------------------------------------------------------------------------

/// Lines BibTeX prints that are never problems.
fn is_chatter(line: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "This is BibTeX",
        "Capacity: ",
        "The top-level auxiliary file:",
        "A level-",
        "The style file:",
        "Database file #",
        "You've used",
        "and the built_in function-call counts",
        "(There ",
        "Reallocated",
        "I'm skipping whatever remains",
        "(Error may have been on previous line)",
    ];
    line.trim().is_empty()
        || line.starts_with(char::is_whitespace)
        || PREFIXES.iter().any(|p| line.starts_with(p))
        || is_function_count(line)
}

/// `add.period$ -- 3`: the built-in function call counts at the end.
fn is_function_count(line: &str) -> bool {
    line.rsplit_once(" -- ").is_some_and(|(name, count)| {
        !name.contains(' ') && count.bytes().all(|b| b.is_ascii_digit())
    })
}

/// `line 10 of file refs.bib` / `while reading file main.aux`.
fn bibtex_location(text: &str, base_dir: &Path) -> (Option<PathBuf>, Option<u32>) {
    let line = text
        .find("line ")
        .and_then(|p| leading_number(text.get(p + 5..)?));
    let file = text
        .find("file ")
        .and_then(|p| text.get(p + 5..))
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(|f| resolve(f, base_dir));
    (file, line)
}

/// BibTeX's error-context lines (` : ...`, `I'm skipping ...`), collected into the detail.
fn bibtex_detail(lines: &[&str], mut k: usize) -> (Vec<String>, usize) {
    let mut detail = Vec::new();
    while let Some(line) = lines.get(k) {
        let is_context = line.starts_with(" : ")
            || line.trim_end() == " :"
            || line.starts_with("(Error may have been on previous line)")
            || line.starts_with("I'm skipping whatever remains");
        if !is_context {
            break;
        }
        if detail.len() < MAX_DETAIL {
            detail.push(line.trim_end().to_string());
        }
        k += 1;
    }
    (detail, k)
}

fn parse_bibtex(lines: &[&str], base_dir: &Path) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut i = 0;
    while let Some(line) = lines.get(i).copied() {
        if let Some(rest) = line.strip_prefix("Warning--") {
            let mut issue = Issue {
                severity: Severity::Warning,
                file: None,
                line: None,
                message: tidy(rest),
                detail: String::new(),
                log_line: log_line(i),
            };
            i += 1;
            // `--line 5 of file refs.bib` on the next line.
            if let Some(location) = lines.get(i).and_then(|l| l.strip_prefix("--")) {
                (issue.file, issue.line) = bibtex_location(location, base_dir);
                i += 1;
            }
            issues.push(issue);
            continue;
        }
        if is_chatter(line) {
            i += 1;
            continue;
        }
        // Errors carry their location after `---`, on the same line or the next one.
        let (message, location, next) = if let Some((message, location)) = line.split_once("---") {
            (message, location, i + 1)
        } else if let Some(location) = lines.get(i + 1).and_then(|l| {
            l.strip_prefix("---")
                .or_else(|| l.strip_prefix("while executing---"))
        }) {
            (line, location, i + 2)
        } else {
            i += 1;
            continue;
        };
        let (file, line_no) = bibtex_location(location, base_dir);
        let (detail, next) = bibtex_detail(lines, next);
        issues.push(Issue {
            severity: Severity::Error,
            file,
            line: line_no,
            message: tidy(message),
            detail: detail.join("\n"),
            log_line: log_line(i),
        });
        i = next;
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn biber_lines() {
        assert_eq!(
            biber_line(
                "[50] Biber.pm:4419> WARN - I didn't find a database entry for 'x' (section 0)"
            ),
            Some(("WARN", "I didn't find a database entry for 'x' (section 0)"))
        );
        assert_eq!(biber_line("[x] a> WARN - b"), None);
        assert_eq!(biber_line("Warning--x"), None);
        assert_eq!(
            clean_biber_path("/var/folders/T/biber_tmp_1/refs.bib_1234.utf8"),
            "refs.bib"
        );
        assert_eq!(clean_biber_path("my_refs.bib"), "my_refs.bib");
    }

    #[test]
    fn biber_locations() {
        let base = Path::new("/p");
        let (file, line, msg) = biber_location(
            "BibTeX subsystem: /tmp/b/refs.bib_99.utf8, line 10, syntax error: found \"title\"",
            base,
        );
        assert_eq!(file, Some(PathBuf::from("/p/refs.bib")));
        assert_eq!(line, Some(10));
        assert_eq!(msg, "syntax error: found \"title\"");
        let (file, line, _) = biber_location(
            "Datamodel: Entry 'k' (refs.bib): Missing mandatory field 'author'",
            base,
        );
        assert_eq!((file, line), (Some(PathBuf::from("/p/refs.bib")), None));
    }

    #[test]
    fn function_counts_are_chatter() {
        assert!(is_chatter("add.period$ -- 3"));
        assert!(is_chatter("= -- 19"));
        assert!(is_chatter(
            "            2118 wiz_defined-function locations,"
        ));
        assert!(!is_chatter("I couldn't open database file x.bib"));
    }
}
