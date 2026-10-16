//! Small string helpers shared by the log and `.blg` parsers.

/// Longest message we keep, in bytes (cut at a char boundary).
const MAX_MESSAGE: usize = 600;

/// Splits a log into lines, dropping the `\r` of CRLF line ends.
pub(crate) fn lines(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

/// Turns raw log text into a one-line message: trims, collapses whitespace runs into one
/// space, turns TeX's `quotes' into 'quotes' and caps the length.
pub(crate) fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(MAX_MESSAGE));
    let mut pending_space = false;
    for c in s.trim().chars() {
        if c.is_whitespace() {
            pending_space = true;
            continue;
        }
        if out.len() >= MAX_MESSAGE {
            out.push('…');
            break;
        }
        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;
        out.push(if c == '`' { '\'' } else { c });
    }
    out
}

/// Parses the ASCII digits at the start of `s` (after optional spaces).
pub(crate) fn leading_number(s: &str) -> Option<u32> {
    let s = s.trim_start();
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s.get(..end).filter(|d| !d.is_empty())?.parse().ok()
}

/// Splits `Package foo Error: body` / `Class foo Warning: body` into (`Package foo`, `body`).
pub(crate) fn split_origin<'s>(s: &'s str, word: &str) -> Option<(&'s str, &'s str)> {
    let rest = s
        .strip_prefix("Package ")
        .or_else(|| s.strip_prefix("Class "))?;
    let space = rest.find(' ')?;
    let after = rest
        .get(space + 1..)?
        .strip_prefix(word)?
        .strip_prefix(':')?;
    let origin_len = s.len().checked_sub(rest.len())?.checked_add(space)?;
    Some((s.get(..origin_len)?, after.trim_start()))
}

/// The message for a TeX error: `LaTeX Error: x` becomes `x`, `Package p Error: x` becomes
/// `Package p: x`; everything else is kept (tidied).
pub(crate) fn error_message(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix("LaTeX Error:") {
        return tidy(rest);
    }
    if let Some((origin, body)) = split_origin(raw, "Error") {
        return format!("{origin}: {}", tidy(body));
    }
    tidy(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_collapses_and_requotes() {
        assert_eq!(
            tidy("  Reference `a'  on\tpage 1 "),
            "Reference 'a' on page 1"
        );
        let long = "x".repeat(2000);
        assert!(tidy(&long).len() <= MAX_MESSAGE + '…'.len_utf8());
        assert_eq!(tidy(&"中".repeat(400)).chars().last(), Some('…'));
    }

    #[test]
    fn numbers_and_origins() {
        assert_eq!(leading_number("12--14"), Some(12));
        assert_eq!(leading_number(" 7 pages"), Some(7));
        assert_eq!(leading_number("x"), None);
        assert_eq!(leading_number("99999999999999"), None);
        assert_eq!(
            split_origin("Package fontspec Error: The font", "Error"),
            Some(("Package fontspec", "The font"))
        );
        assert_eq!(
            split_origin("Class foo Warning:x", "Warning"),
            Some(("Class foo", "x"))
        );
        assert_eq!(split_origin("LaTeX Error: x", "Error"), None);
        assert_eq!(
            error_message("LaTeX Error: File `a.sty' not found."),
            "File 'a.sty' not found."
        );
        assert_eq!(error_message("Package p Error: Bad."), "Package p: Bad.");
        assert_eq!(
            error_message("Undefined control sequence."),
            "Undefined control sequence."
        );
    }
}
