//! Commit messages written by the model (the Source Control tab's sparkle button): the request
//! for a short message from the changes a commit would record, and the clean-up of the answer.

use crate::{ChatRequest, Provider, Role, Turn};

/// The longest message kept (in characters); a longer answer is cut at a word.
pub const MAX_CHARS: usize = 200;

/// The request for a commit message: `changes` is what the commit would record (a file
/// summary, new files and the diff, as `texcraft-git` reads them).
pub fn request(provider: &Provider, model: &str, api_key: String, changes: &str) -> ChatRequest {
    let system = "You write git commit messages for the projects of a LaTeX editor. You get the \
        changes a commit will record and reply with only the commit message: one or two short \
        sentences, at most 25 words in all, in the imperative mood (\"Add\", \"Fix\", \
        \"Rewrite\"), in English. Say what changed in the document (sections, wording, figures, \
        tables, references, packages), not how the diff looks. Plain text only: no quotes, no \
        Markdown, no prefix such as \"feat:\", no body or list."
        .to_string();
    let user =
        format!("<changes>\n{changes}\n</changes>\n\nThe commit message (one or two sentences):");
    ChatRequest {
        provider: provider.clone(),
        model: model.to_string(),
        system,
        api_key,
        turns: vec![Turn {
            role: Role::User,
            text: user,
            blocks: Vec::new(),
            images: Vec::new(),
            ..Default::default()
        }],
        max_tokens: Some(300),
        no_reasoning: true,
        tools: Vec::new(),
    }
}

/// The message in a model's `answer`: its first paragraph without code fences, quotes or a
/// "Commit message:" label, on one line, at most two sentences and [`MAX_CHARS`] characters.
pub fn clean(answer: &str) -> String {
    let mut lines = Vec::new();
    for line in answer.lines() {
        let line = line.trim();
        if line.starts_with("```") {
            continue;
        }
        if line.is_empty() {
            if lines.is_empty() {
                continue;
            }
            break;
        }
        lines.push(line);
    }
    let mut text = lines.join(" ");
    for label in ["commit message:", "message:", "commit:"] {
        if text
            .get(..label.len())
            .is_some_and(|h| h.eq_ignore_ascii_case(label))
        {
            text = text.get(label.len()..).unwrap_or("").trim().to_string();
        }
    }
    // A quoted message followed by remarks: the quote is the message.
    let quoted = [('"', '"'), ('“', '”'), ('「', '」'), ('`', '`')]
        .into_iter()
        .find_map(|(open, close)| {
            let rest = text.strip_prefix(open)?;
            rest.find(close).and_then(|i| rest.get(..i))
        })
        .filter(|q| !q.trim().is_empty())
        .map(str::to_string);
    let text = quoted
        .unwrap_or(text)
        .trim_matches(is_quote)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let text = two_sentences(&text);
    if text.chars().count() <= MAX_CHARS {
        return text.to_string();
    }
    let cut = text
        .char_indices()
        .nth(MAX_CHARS)
        .map_or(text.len(), |(i, _)| i);
    let head = text.get(..cut).unwrap_or(text);
    head.rfind(' ')
        .and_then(|i| head.get(..i))
        .unwrap_or(head)
        .trim_end_matches([',', ';', ':'])
        .to_string()
}

fn is_quote(c: char) -> bool {
    matches!(c, '"' | '\'' | '`' | '“' | '”' | '「' | '」')
}

/// `text` up to the end of its second sentence (a full stop, or `.`, `!`, `?` before a space
/// or the end, closing quotes and brackets allowed in between).
fn two_sentences(text: &str) -> &str {
    let mut ends = 0;
    for (i, c) in text.char_indices() {
        let end = match c {
            '。' | '！' | '？' => true,
            '.' | '!' | '?' => text
                .get(i + c.len_utf8()..)
                .unwrap_or("")
                .trim_start_matches(|n| is_quote(n) || n == ')')
                .chars()
                .next()
                .is_none_or(char::is_whitespace),
            _ => false,
        };
        if end {
            ends += 1;
            if ends == 2 {
                return text.get(..i + c.len_utf8()).unwrap_or(text);
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_keeps_one_or_two_sentences_on_one_line() {
        assert_eq!(
            clean("Add the related-work section and cite the survey."),
            "Add the related-work section and cite the survey."
        );
        // Fences, a label, quotes and a body go; lines join.
        assert_eq!(
            clean(
                "```\nCommit message: \"Rewrite the abstract\nfor clarity.\"\n\nLonger body.\n```"
            ),
            "Rewrite the abstract for clarity."
        );
        // At most two sentences (a version number is not a sentence end).
        assert_eq!(
            clean("Fix typos in v2.1 of the intro. Update the figure. Bump the date."),
            "Fix typos in v2.1 of the intro. Update the figure."
        );
        assert_eq!(
            clean("更新摘要。补充参考文献。修正图表。"),
            "更新摘要。补充参考文献。"
        );
        // A quoted message with remarks after it: the quote.
        assert_eq!(
            clean("Commit message: \"Rewrite the introduction.\" This also adds a paragraph."),
            "Rewrite the introduction."
        );
        // A sentence end before a closing parenthesis counts.
        assert_eq!(
            clean("Fix the table (rows 2 and 3.) Cite Knuth. Update the date."),
            "Fix the table (rows 2 and 3.) Cite Knuth."
        );
        assert_eq!(clean("  \n\n"), "");
        // Too long: cut at a word.
        let long = clean(&"word ".repeat(80));
        assert!(long.chars().count() <= MAX_CHARS, "{long}");
        assert!(long.ends_with("word"), "{long}");
    }

    #[test]
    fn request_carries_the_changes_and_a_small_budget() {
        let p = Provider::default();
        let r = request(&p, "m", String::new(), "main.tex | 2 +-");
        assert!(
            r.turns
                .first()
                .is_some_and(|t| t.text.contains("main.tex | 2 +-"))
        );
        assert!(r.system.contains("one or two short sentences"));
        assert_eq!(r.max_tokens, Some(300));
    }
}
