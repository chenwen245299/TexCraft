//! Minimal BibTeX reader: entry keys with author/title/year for citation completion.

/// One bibliography entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BibEntry {
    /// Citation key.
    pub key: String,
    /// Entry type, lowercased (`article`, `book`, ...).
    pub entry_type: String,
    /// `author` field, braces removed.
    pub author: String,
    /// `title` field, braces removed.
    pub title: String,
    /// `year` (or the year part of `date`).
    pub year: String,
    /// 1-based line of the `@type{` in the file.
    pub line: u32,
}

/// Parses the entries of a `.bib` file. Never fails; malformed entries are skipped.
pub fn bib_entries(text: &str) -> Vec<BibEntry> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut line_at = 0usize;
    while let Some(at_rel) = bytes
        .get(i..)
        .and_then(|r| r.iter().position(|&c| c == b'@'))
    {
        let at = i + at_rel;
        line = line.saturating_add(
            u32::try_from(
                bytes
                    .get(line_at..at)
                    .map_or(0, |r| r.iter().filter(|&&c| c == b'\n').count()),
            )
            .unwrap_or(0),
        );
        line_at = at;
        i = at + 1;
        let type_len = bytes.get(i..).map_or(0, |r| {
            r.iter().take_while(|c| c.is_ascii_alphabetic()).count()
        });
        let entry_type = text.get(i..i + type_len).unwrap_or("").to_ascii_lowercase();
        i += type_len;
        while matches!(bytes.get(i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            i += 1;
        }
        let open = match bytes.get(i) {
            Some(b'{') => b'}',
            Some(b'(') => b')',
            _ => continue,
        };
        if matches!(entry_type.as_str(), "comment" | "preamble" | "string") || entry_type.is_empty()
        {
            continue;
        }
        let body_start = i + 1;
        // Find the end of the entry by brace depth.
        let mut depth = 1i32;
        let mut j = body_start;
        while let Some(&c) = bytes.get(j) {
            if c == b'{' || (open == b')' && c == b'(') {
                depth += 1;
            } else if c == b'}' || c == open {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            j += 1;
        }
        let body = text.get(body_start..j.min(text.len())).unwrap_or("");
        i = j.saturating_add(1);
        let Some((key, fields)) = body.split_once(',') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || key.contains(char::is_whitespace) {
            continue;
        }
        let mut entry = BibEntry {
            key: key.to_string(),
            entry_type,
            line,
            ..BibEntry::default()
        };
        for (name, value) in fields_of(fields) {
            match name.as_str() {
                "author" => entry.author = value,
                "title" => entry.title = value,
                "year" => entry.year = value,
                "date" if entry.year.is_empty() => entry.year = value.chars().take(4).collect(),
                _ => {}
            }
        }
        out.push(entry);
    }
    out
}

fn fields_of(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let Some(eq_rel) = bytes
            .get(i..)
            .and_then(|r| r.iter().position(|&c| c == b'='))
        else {
            break;
        };
        let name = body
            .get(i..i + eq_rel)
            .unwrap_or("")
            .trim()
            .trim_start_matches(',')
            .trim()
            .to_ascii_lowercase();
        i += eq_rel + 1;
        while matches!(bytes.get(i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            i += 1;
        }
        let value;
        match bytes.get(i) {
            Some(b'{') => {
                let mut depth = 0i32;
                let start = i + 1;
                while let Some(&c) = bytes.get(i) {
                    if c == b'{' {
                        depth += 1;
                    } else if c == b'}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    i += 1;
                }
                value = body.get(start..i.min(body.len())).unwrap_or("").to_string();
                i += 1;
            }
            Some(b'"') => {
                let start = i + 1;
                i += 1;
                while let Some(&c) = bytes.get(i) {
                    if c == b'"' {
                        break;
                    }
                    i += 1;
                }
                value = body.get(start..i.min(body.len())).unwrap_or("").to_string();
                i += 1;
            }
            _ => {
                let start = i;
                while bytes.get(i).is_some_and(|&c| c != b',') {
                    i += 1;
                }
                value = body.get(start..i).unwrap_or("").trim().to_string();
            }
        }
        let cleaned: String = value
            .chars()
            .filter(|c| *c != '{' && *c != '}')
            .map(|c| if c.is_whitespace() { ' ' } else { c })
            .collect();
        out.push((
            name,
            cleaned.split_whitespace().collect::<Vec<_>>().join(" "),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_entries() {
        let bib = "@comment{ignore me}\n@article{knuth84,\n  author = {Donald E. Knuth},\n  title = {Literate {P}rogramming},\n  year = 1984,\n}\n\n@book{lamport94, title=\"LaTeX: A Document Preparation System\", author={Leslie Lamport}, date={1994-06-01}}\n@misc{broken";
        let e = bib_entries(bib);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].key, "knuth84");
        assert_eq!(e[0].title, "Literate Programming");
        assert_eq!(e[0].year, "1984");
        assert_eq!(e[0].line, 2);
        assert_eq!(e[1].key, "lamport94");
        assert_eq!(e[1].year, "1994");
        assert_eq!(e[1].line, 8);
    }

    #[test]
    fn garbage_never_panics() {
        for t in [
            "@",
            "@{",
            "@article{",
            "@article{k,",
            "@article{k, a=",
            "@article{k, a={",
            "@article{k, a=\"",
        ] {
            let _ = bib_entries(t);
        }
    }
}
