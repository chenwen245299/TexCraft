//! Inline suggestions in the editor (as an IDE's AI autocomplete): the request for the text
//! around the caret and the clean-up of the answer into the text to insert.

use serde::{Deserialize, Serialize};

use crate::{ChatRequest, Provider, Role, Turn};

/// How inline suggestions behave (Settings: the editor header's suggestion button).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InlineSettings {
    /// Suggest while typing (otherwise only on request, ⌥\).
    pub enabled: bool,
    /// Pause after the last keystroke before asking, in milliseconds.
    pub delay_ms: u32,
    /// The longest suggestion, in lines.
    pub max_lines: u32,
    /// Characters of the document before the caret sent as context.
    pub context_before: u32,
    /// Characters after the caret sent as context.
    pub context_after: u32,
    /// Provider id for suggestions; empty: the Assistant's.
    pub provider: String,
    /// Model for suggestions; empty: the Assistant's.
    pub model: String,
}

impl Default for InlineSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            delay_ms: 500,
            max_lines: 3,
            context_before: 4000,
            context_after: 1000,
            provider: String::new(),
            model: String::new(),
        }
    }
}

impl InlineSettings {
    /// Keeps the numbers in their ranges (a hand-edited `ai.json` included).
    pub fn clamp(&mut self) {
        self.delay_ms = self.delay_ms.clamp(100, 5000);
        self.max_lines = self.max_lines.clamp(1, 30);
        self.context_before = self.context_before.clamp(200, 50_000);
        self.context_after = self.context_after.min(20_000);
    }

    /// The token budget for an answer of `max_lines` lines.
    pub fn max_tokens(&self) -> u32 {
        (self.max_lines.saturating_mul(48)).clamp(48, 1200)
    }
}

/// Marks the caret in the prompt.
pub const CURSOR: &str = "<cursor/>";

/// The request for a suggestion at the caret: `before` and `after` are the document's text
/// around it (already cut to the context sizes), `file` its name.
pub fn request(
    provider: &Provider,
    model: &str,
    api_key: String,
    file: &str,
    before: &str,
    after: &str,
    settings: &InlineSettings,
) -> ChatRequest {
    let lines = settings.max_lines.max(1);
    let system = format!(
        "You are the inline autocomplete of a LaTeX editor, like an IDE's code completion. You \
         get the document around the cursor, marked {CURSOR}, and reply with only the text to \
         insert there: no explanation, no Markdown or code fences, nothing repeated from before \
         or after the cursor. Continue naturally in the document's language and style, with \
         valid LaTeX (close what you open unless the text after the cursor already does). At \
         most {lines} line{}; stop at a natural end (a word, sentence, line or environment). \
         When nothing useful fits, reply with nothing.",
        if lines == 1 { "" } else { "s" }
    );
    let user = format!(
        "<document name=\"{file}\">\n{before}{CURSOR}{after}\n</document>\n\nThe text to insert at {CURSOR}:"
    );
    ChatRequest {
        provider: provider.clone(),
        model: model.to_string(),
        system,
        api_key,
        turns: vec![Turn {
            role: Role::User,
            text: user,
            blocks: Vec::new(),
        }],
        max_tokens: Some(settings.max_tokens()),
    }
}

/// The last `n` characters of `text` (cut at a character boundary).
pub fn tail(text: &str, n: usize) -> &str {
    let skip = text.chars().count().saturating_sub(n);
    text.char_indices()
        .nth(skip)
        .map_or("", |(i, _)| text.get(i..).unwrap_or(""))
}

/// The first `n` characters of `text`.
pub fn head(text: &str, n: usize) -> &str {
    text.char_indices()
        .nth(n)
        .map_or(text, |(i, _)| text.get(..i).unwrap_or(text))
}

/// The text to insert from a model's `answer`: code fences and a repeated start of the current
/// line removed, the part the text after the caret already has dropped, at most `max_lines`
/// lines, trailing blanks trimmed. Empty when nothing is left.
pub fn clean(answer: &str, before: &str, after: &str, max_lines: u32) -> String {
    let mut text = answer.replace("\r\n", "\n");
    // A fenced block: keep its content.
    if let Some(start) = text.find("```") {
        let rest = text.get(start + 3..).unwrap_or("");
        let body_start = rest.find('\n').map_or(rest.len(), |i| i + 1);
        let body = rest.get(body_start..).unwrap_or("");
        let body = body.split("```").next().unwrap_or(body);
        text = body.to_string();
    }
    text = text.replace(CURSOR, "");
    // The model repeated the start of the current line.
    let line_start = before.rsplit('\n').next().unwrap_or(before);
    if line_start.trim().chars().count() >= 3 && text.starts_with(line_start) {
        text = text.get(line_start.len()..).unwrap_or("").to_string();
    }
    // At most `max_lines` lines.
    let keep = usize::try_from(max_lines.max(1)).unwrap_or(1);
    let mut lines: Vec<&str> = text.split('\n').collect();
    lines.truncate(keep);
    let mut text = lines.join("\n").trim_end().to_string();
    // What the text after the caret already has (on its first line): a closing brace, say.
    let next = after.split('\n').next().unwrap_or("");
    let overlap = (1..=next.len().min(text.len()))
        .rev()
        .find(|&k| next.is_char_boundary(k) && text.ends_with(next.get(..k).unwrap_or("\u{0}")));
    if let Some(k) = overlap {
        text.truncate(text.len() - k);
        text = text.trim_end().to_string();
    }
    if text.trim().is_empty() {
        String::new()
    } else {
        text
    }
}

/// The first word of a suggestion, with the spaces before it (⌘→ accepts it): up to and
/// including the next space or line break after the first non-blank character.
pub fn first_word(suggestion: &str) -> &str {
    let mut seen = false;
    for (i, c) in suggestion.char_indices() {
        if c.is_whitespace() {
            if seen {
                return suggestion
                    .get(..i + if c == '\n' { 0 } else { c.len_utf8() })
                    .unwrap_or(suggestion);
            }
            if c == '\n' {
                return suggestion.get(..i + 1).unwrap_or(suggestion);
            }
        } else {
            seen = true;
        }
    }
    suggestion
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Kind;

    #[test]
    fn requests_carry_the_context_and_a_small_budget() {
        let p = Provider {
            id: "deepseek".into(),
            kind: Kind::OpenAi,
            base_url: "https://api.deepseek.com/v1".into(),
            ..Provider::default()
        };
        let s = InlineSettings {
            max_lines: 2,
            ..InlineSettings::default()
        };
        let r = request(
            &p,
            "deepseek-chat",
            "k".into(),
            "main.tex",
            "A \\textbf{",
            "} B",
            &s,
        );
        assert!(r.system.contains("At most 2 lines"));
        assert!(r.turns[0].text.contains("A \\textbf{<cursor/>} B"));
        assert_eq!(r.max_tokens, Some(96));
        let (_, _, body) = crate::build(&r);
        assert_eq!(body["max_tokens"], 96);
        let openai = Provider {
            base_url: "https://api.openai.com/v1".into(),
            ..p
        };
        let (_, _, body) = crate::build(&request(&openai, "m", "k".into(), "a", "", "", &s));
        assert_eq!(body["max_completion_tokens"], 96);
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn answers_are_cleaned_into_the_text_to_insert() {
        // Fences and the cursor mark go; the repeated line start goes.
        assert_eq!(clean("```latex\nworld}\n```", "Hello ", "", 3), "world}");
        assert_eq!(
            clean("\\section{Intro} text", "\\section{Intro", "", 3),
            "} text"
        );
        // The closing brace after the caret is not doubled.
        assert_eq!(clean("bold}", "\\textbf{", "}", 3), "bold");
        assert_eq!(clean("}", "\\textbf{x", "}", 3), "");
        // Line limit, trailing blanks, empty answers.
        assert_eq!(clean("a\nb\nc\nd", "", "", 2), "a\nb");
        assert_eq!(clean("  \n ", "", "", 3), "");
        assert_eq!(clean("x<cursor/>", "", "", 3), "x");
    }

    #[test]
    fn context_cuts_and_first_words() {
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("ab", 5), "ab");
        assert_eq!(tail("你好世界", 2), "世界");
        assert_eq!(head("你好世界", 1), "你");
        assert_eq!(head("ab", 5), "ab");
        assert_eq!(first_word("hello world"), "hello ");
        assert_eq!(first_word("  hello world"), "  hello ");
        assert_eq!(first_word("hi\nthere"), "hi");
        assert_eq!(first_word("\nthere"), "\n");
        assert_eq!(first_word("end"), "end");
        let mut s = InlineSettings {
            delay_ms: 1,
            max_lines: 99,
            ..InlineSettings::default()
        };
        s.clamp();
        assert_eq!((s.delay_ms, s.max_lines), (100, 30));
    }
}
