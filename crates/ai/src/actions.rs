//! The editor's right-click AI actions (Polish, Grammar Check, …): an instruction each, sent to
//! the Assistant with the selected passage. They live in `ai.json` with the providers and are
//! edited in Settings → AI → Actions. A built-in action keeps an empty name or prompt while the
//! user has not changed it, so it shows in the interface language and picks up better default
//! prompts in later versions.

use serde::{Deserialize, Serialize};
use texcraft_i18n::tr;

/// What becomes of an action's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Output {
    /// A rewrite of the passage, which can replace it in the document.
    #[default]
    Rewrite,
    /// An answer to read in the Assistant.
    Answer,
}

/// An action shipped with TexCraft.
#[derive(Debug, Clone, Copy)]
pub struct Builtin {
    /// Stable id.
    pub id: &'static str,
    /// Menu name in English (the catalogue key).
    pub name: &'static str,
    /// The instruction (sent to the model as is, in English).
    pub prompt: &'static str,
    /// What its answer is for.
    pub output: Output,
}

/// The built-in actions, in menu order.
pub const BUILTINS: &[Builtin] = &[
    Builtin {
        id: "polish",
        name: "Polish",
        prompt: "Polish the wording of this passage: make it clearer, more fluent and more \
                 natural in an academic register, without changing its meaning or adding claims. \
                 Keep its language, and keep every LaTeX command, environment, math expression, \
                 citation, reference and label exactly as it is.",
        output: Output::Rewrite,
    },
    Builtin {
        id: "grammar",
        name: "Grammar Check",
        prompt: "Correct the grammar, spelling and punctuation of this passage. Change as little \
                 as possible: keep the wording, the meaning, the language and every LaTeX \
                 command, math expression, citation, reference and label as they are.",
        output: Output::Rewrite,
    },
    Builtin {
        id: "shorten",
        name: "Shorten",
        prompt: "Make this passage more concise: remove redundancy and wordiness while keeping \
                 its meaning, its key details and its language. Keep every LaTeX command, math \
                 expression, citation, reference and label.",
        output: Output::Rewrite,
    },
    Builtin {
        id: "translate",
        name: "Translate to English",
        prompt: "Translate this passage into fluent academic English. Translate only the prose: \
                 keep every LaTeX command, environment, math expression, citation, reference and \
                 label exactly as it is.",
        output: Output::Rewrite,
    },
    Builtin {
        id: "explain",
        name: "Explain",
        prompt: "Explain this passage of my LaTeX document: what it says or does, and how it \
                 will look when typeset. Briefly point out any errors or likely problems.",
        output: Output::Answer,
    },
];

/// One action of the right-click menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Action {
    /// Stable id (a built-in's, or `custom`, `custom-2`, …).
    pub id: String,
    /// Its menu name; empty: the built-in's, in the interface language.
    pub name: String,
    /// The instruction; empty: the built-in's.
    pub prompt: String,
    /// What its answer is for.
    pub output: Output,
}

impl Default for Action {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            prompt: String::new(),
            output: Output::Rewrite,
        }
    }
}

impl Action {
    /// The built-in action `b`, unchanged.
    pub fn from_builtin(b: &Builtin) -> Self {
        Self {
            id: b.id.to_string(),
            name: String::new(),
            prompt: String::new(),
            output: b.output,
        }
    }

    /// The built-in it is (by id).
    pub fn builtin(&self) -> Option<&'static Builtin> {
        BUILTINS.iter().find(|b| b.id == self.id)
    }

    /// Its menu name in the interface language.
    pub fn label(&self) -> String {
        let name = self.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        self.builtin()
            .map_or_else(|| tr("Untitled Action"), |b| tr(b.name))
            .to_string()
    }

    /// The instruction sent to the model (the built-in's while it is empty).
    pub fn prompt(&self) -> &str {
        let own = self.prompt.trim();
        match self.builtin() {
            Some(b) if own.is_empty() => b.prompt,
            _ => own,
        }
    }

    /// A built-in whose prompt the user changed (Restore Default Prompt applies).
    pub fn prompt_changed(&self) -> bool {
        self.builtin().is_some() && !self.prompt.trim().is_empty()
    }

    /// Back to the built-in's prompt.
    pub fn restore_prompt(&mut self) {
        self.prompt.clear();
    }

    /// A name or prompt the same as the built-in's is stored empty, so it follows the interface
    /// language and later versions.
    pub fn normalize(&mut self) {
        if let Some(b) = self.builtin() {
            let name = self.name.trim();
            if name == b.name || name == tr(b.name) {
                self.name.clear();
            }
            if self.prompt.trim() == b.prompt {
                self.prompt.clear();
            }
        }
    }
}

/// The actions of a new configuration: every built-in.
pub fn defaults() -> Vec<Action> {
    BUILTINS.iter().map(Action::from_builtin).collect()
}

/// The built-ins missing from `actions` (removed by the user), for adding back.
pub fn missing(actions: &[Action]) -> Vec<&'static Builtin> {
    BUILTINS
        .iter()
        .filter(|b| !actions.iter().any(|a| a.id == b.id))
        .collect()
}

/// An id for a new action that none of `actions` has.
pub fn new_id(actions: &[Action]) -> String {
    let mut id = "custom".to_string();
    let mut n = 2;
    while actions.iter().any(|a| a.id == id) {
        id = format!("custom-{n}");
        n += 1;
    }
    id
}

/// Drops actions without an id and repeated ids, and normalises the rest.
pub fn normalize(actions: &mut Vec<Action>) {
    let mut seen = std::collections::HashSet::new();
    actions.retain(|a| !a.id.trim().is_empty() && seen.insert(a.id.clone()));
    for a in actions.iter_mut() {
        a.normalize();
    }
}

/// The question an action asks: its `prompt`, then the `passage` in a `latex` code block whose
/// fence is longer than any run of backticks in it.
pub fn message(prompt: &str, passage: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in passage.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(longest.max(2) + 1);
    format!(
        "{}\n\n{fence}latex\n{}\n{fence}",
        prompt.trim(),
        passage.trim_end_matches('\n')
    )
}

/// What the system prompt adds for an action: a rewrite comes back as the whole passage in one
/// code block (which replaces the passage); explanations are written in `explain_in` (the
/// interface language, `None`: the model's choice).
pub fn system(output: Output, explain_in: Option<&str>) -> String {
    let mut s = String::new();
    if output == Output::Rewrite {
        s.push_str(
            "\n\nThe user's message is an instruction about a passage of the open document, \
             given after it in a code block. Reply with the whole rewritten passage in a single \
             fenced code block marked `latex`: it will replace the passage in the document, so \
             include all of it, keep what the instruction does not ask you to change, keep its \
             line breaks where you can, and add nothing that is not part of the passage. Write \
             nothing before the block; after it, at most a sentence or two on what you changed.",
        );
    }
    if let Some(lang) = explain_in {
        s.push_str(&format!(
            "\n\nWrite explanations in {lang}; the document's own text keeps its language."
        ));
    }
    s
}

/// A rewrite action's answer: the rewritten passage and what the model said besides it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reply {
    /// The passage: the first `latex` / `tex` code block (else the first code block, else the
    /// whole answer). While the answer streams, what of the block has arrived.
    pub text: String,
    /// The rest of the answer (a sentence on what changed).
    pub note: String,
}

/// Splits an answer into the rewritten passage and the note around it ([`Reply`]).
pub fn split_reply(answer: &str) -> Reply {
    // (language, body, first line, line after the closing fence or None while open)
    let mut blocks: Vec<(String, String, usize, Option<usize>)> = Vec::new();
    let mut open: Option<(usize, String, String, usize)> = None;
    let lines: Vec<&str> = answer.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let ticks = trimmed.chars().take_while(|c| *c == '`').count();
        match open.as_mut() {
            Some((fence, _, body, _)) => {
                if ticks >= *fence && trimmed.len() == ticks {
                    if let Some((_, lang, body, first)) = open.take() {
                        blocks.push((lang, body, first, Some(i + 1)));
                    }
                } else {
                    body.push_str(line);
                    body.push('\n');
                }
            }
            None if ticks >= 3 => {
                let lang = trimmed.get(ticks..).unwrap_or("").trim().to_lowercase();
                open = Some((ticks, lang, String::new(), i));
            }
            None => {}
        }
    }
    if let Some((_, lang, body, first)) = open {
        blocks.push((lang, body, first, None));
    }
    let pick = blocks
        .iter()
        .find(|(lang, ..)| lang == "latex" || lang == "tex")
        .or_else(|| blocks.first());
    let Some((_, body, first, end)) = pick else {
        return Reply {
            text: answer.trim().to_string(),
            note: String::new(),
        };
    };
    let join = |range: &[&str]| range.join("\n").trim().to_string();
    let before = join(lines.get(..*first).unwrap_or(&[]));
    let after = end.map_or_else(String::new, |e| join(lines.get(e..).unwrap_or(&[])));
    let note = match (before.is_empty(), after.is_empty()) {
        (true, _) => after,
        (false, true) => before,
        (false, false) => format!("{before}\n\n{after}"),
    };
    Reply {
        text: body.trim_end_matches('\n').to_string(),
        note,
    }
}

/// What became of a piece of text in a [`word_diff`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// In both.
    Same,
    /// Only in the new text.
    Added,
    /// Only in the old text.
    Removed,
}

/// Words and the spaces between them (a run of white space is one piece).
fn words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut space: Option<bool> = None;
    for (i, c) in text.char_indices() {
        let s = c.is_whitespace();
        if space.is_some_and(|prev| prev != s) {
            out.push(text.get(start..i).unwrap_or(""));
            start = i;
        }
        space = Some(s);
    }
    if start < text.len() {
        out.push(text.get(start..).unwrap_or(""));
    }
    out
}

/// The most word pairs [`word_diff`] compares (a longer passage is shown without marks).
const DIFF_CELLS: usize = 1_000_000;

/// `new` as pieces marked against `old`, word by word (the longest common subsequence), for
/// showing a rewrite's changes as proofreading does. White space counts as the same wherever
/// both have some. Passages too long to compare come back as one unmarked piece.
pub fn word_diff(old: &str, new: &str) -> Vec<(Change, String)> {
    let (a, b) = (words(old), words(new));
    let (n, m) = (a.len(), b.len());
    let cells = n.saturating_add(1).saturating_mul(m.saturating_add(1));
    if cells > DIFF_CELLS {
        return vec![(Change::Same, new.to_string())];
    }
    let same = |x: &str, y: &str| x == y || (x.trim().is_empty() && y.trim().is_empty());
    // lcs[i * (m + 1) + j]: the common subsequence of a[i..] and b[j..].
    let w = m + 1;
    let mut lcs = vec![0u32; cells];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let v = match (a.get(i), b.get(j)) {
                (Some(x), Some(y)) if same(x, y) => {
                    lcs.get((i + 1) * w + j + 1).copied().unwrap_or(0) + 1
                }
                _ => {
                    let down = lcs.get((i + 1) * w + j).copied().unwrap_or(0);
                    let right = lcs.get(i * w + j + 1).copied().unwrap_or(0);
                    down.max(right)
                }
            };
            if let Some(c) = lcs.get_mut(i * w + j) {
                *c = v;
            }
        }
    }
    let mut out: Vec<(Change, String)> = Vec::new();
    let mut push = |kind: Change, piece: &str| match out.last_mut() {
        Some((k, text)) if *k == kind => text.push_str(piece),
        _ => out.push((kind, piece.to_string())),
    };
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) if same(x, y) => {
                push(Change::Same, y);
                i += 1;
                j += 1;
            }
            (Some(x), Some(y)) => {
                let down = lcs.get((i + 1) * w + j).copied().unwrap_or(0);
                let right = lcs.get(i * w + j + 1).copied().unwrap_or(0);
                if down >= right {
                    push(Change::Removed, x);
                    i += 1;
                } else {
                    push(Change::Added, y);
                    j += 1;
                }
            }
            (Some(x), None) => {
                push(Change::Removed, x);
                i += 1;
            }
            (None, Some(y)) => {
                push(Change::Added, y);
                j += 1;
            }
            (None, None) => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use texcraft_i18n::{Lang, has, scoped};

    #[test]
    fn defaults_are_the_builtins_and_show_translated() {
        let d = defaults();
        assert_eq!(d.len(), BUILTINS.len());
        assert!(d.iter().any(|a| a.id == "polish"));
        assert!(d.iter().any(|a| a.id == "grammar"));
        assert_eq!(
            d.iter().find(|a| a.id == "explain").map(|a| a.output),
            Some(Output::Answer)
        );
        for b in BUILTINS {
            assert!(has(Lang::ZhHans, b.name), "no translation for {:?}", b.name);
            assert!(!b.prompt.contains("  "), "a double space in {:?}", b.id);
        }
        let polish = Action::from_builtin(&BUILTINS[0]);
        {
            let _zh = scoped(Lang::ZhHans);
            assert_eq!(polish.label(), "润色");
        }
        {
            let _en = scoped(Lang::En);
            assert_eq!(polish.label(), "Polish");
            assert_eq!(Action::default().label(), "Untitled Action");
        }
        assert_eq!(polish.prompt(), BUILTINS[0].prompt);
        assert!(!polish.prompt_changed());
    }

    #[test]
    fn own_names_and_prompts_win_and_the_default_comes_back() {
        let mut a = Action::from_builtin(&BUILTINS[1]);
        a.name = "Proofread".into();
        a.prompt = "  Fix the typos only.  ".into();
        a.normalize();
        assert_eq!(a.label(), "Proofread");
        assert_eq!(a.prompt(), "Fix the typos only.");
        assert!(a.prompt_changed());
        a.restore_prompt();
        assert_eq!(a.prompt(), BUILTINS[1].prompt);
        // The default text typed back, or the default name, is stored empty again.
        a.prompt = BUILTINS[1].prompt.into();
        a.name = "Grammar Check".into();
        a.normalize();
        assert!(a.prompt.is_empty() && a.name.is_empty());
        {
            let _zh = scoped(Lang::ZhHans);
            a.name = "语法检查".into();
            a.normalize();
            assert!(a.name.is_empty());
        }
        // A custom action has no default to fall back on.
        let custom = Action {
            id: "custom".into(),
            ..Default::default()
        };
        assert_eq!(custom.prompt(), "");
        assert!(!custom.prompt_changed());
    }

    #[test]
    fn ids_missing_builtins_and_normalising_the_list() {
        let mut list = defaults();
        assert!(missing(&list).is_empty());
        list.retain(|a| a.id != "shorten");
        assert_eq!(
            missing(&list).iter().map(|b| b.id).collect::<Vec<_>>(),
            ["shorten"]
        );
        assert_eq!(new_id(&list), "custom");
        list.push(Action {
            id: "custom".into(),
            name: "Mine".into(),
            ..Default::default()
        });
        assert_eq!(new_id(&list), "custom-2");
        list.push(Action::default());
        list.push(Action {
            id: "polish".into(),
            name: "Twice".into(),
            ..Default::default()
        });
        normalize(&mut list);
        assert_eq!(list.len(), BUILTINS.len());
        assert_eq!(list.iter().filter(|a| a.id == "polish").count(), 1);
        assert!(
            list.iter()
                .find(|a| a.id == "polish")
                .unwrap()
                .name
                .is_empty()
        );
    }

    #[test]
    fn the_message_fences_the_passage() {
        let m = message("  Polish it. ", "Hello \\emph{world}.\n");
        assert_eq!(m, "Polish it.\n\n```latex\nHello \\emph{world}.\n```");
        // A passage with a code fence of its own gets a longer one.
        let m = message("Explain.", "\\begin{verbatim}\n````\n\\end{verbatim}");
        assert!(m.contains("\n`````latex\n"), "{m}");
        assert!(m.ends_with("\n`````"), "{m}");
    }

    #[test]
    fn the_system_addition_asks_for_one_block_and_a_language() {
        let s = system(Output::Rewrite, None);
        assert!(s.contains("single fenced code block marked `latex`"));
        assert!(!s.contains("Write explanations"));
        let s = system(Output::Answer, Some("Simplified Chinese"));
        assert!(!s.contains("code block"));
        assert!(s.contains("Write explanations in Simplified Chinese"));
    }

    #[test]
    fn a_reply_splits_into_the_passage_and_the_note() {
        let r = split_reply("```latex\nNew text.\nSecond line.\n```\nFixed two typos.");
        assert_eq!(r.text, "New text.\nSecond line.");
        assert_eq!(r.note, "Fixed two typos.");
        // A latex block wins over another one; words before it join the note.
        let r = split_reply("Here:\n```\nplain\n```\n```tex\nTeX\n```\nDone.");
        assert_eq!(r.text, "TeX");
        assert_eq!(r.note, "Here:\n```\nplain\n```\n\nDone.");
        // While it streams: what of the block has arrived.
        let r = split_reply("```latex\nNew te");
        assert_eq!((r.text.as_str(), r.note.as_str()), ("New te", ""));
        // A longer fence closes only with as many backticks.
        let r = split_reply("````latex\na ``` b\n```\nc\n````\nok");
        assert_eq!(r.text, "a ``` b\n```\nc");
        // No block: the whole answer is the passage.
        let r = split_reply("  Just the text.\n");
        assert_eq!((r.text.as_str(), r.note.as_str()), ("Just the text.", ""));
        assert_eq!(split_reply(""), Reply::default());
    }

    #[test]
    fn the_word_diff_marks_what_changed() {
        let d = word_diff(
            "It refers to a model's ability.",
            "It is a model's ability.",
        );
        assert_eq!(
            d,
            vec![
                (Change::Same, "It ".to_string()),
                (Change::Removed, "refers to".to_string()),
                (Change::Added, "is".to_string()),
                (Change::Same, " a model's ability.".to_string()),
            ]
        );
        // Line breaks and spaces count as the same; all of the new text is there, in order.
        let d = word_diff("one two\nthree", "one  two three four");
        let new: String = d
            .iter()
            .filter(|(k, _)| *k != Change::Removed)
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(new, "one  two three four");
        assert_eq!(d.last(), Some(&(Change::Added, " four".to_string())));
        assert_eq!(
            word_diff("", "new"),
            vec![(Change::Added, "new".to_string())]
        );
        assert_eq!(
            word_diff("old", ""),
            vec![(Change::Removed, "old".to_string())]
        );
        assert!(word_diff("", "").is_empty());
        // Unicode words, no panics.
        let d = word_diff("中文 段落", "中文 句子");
        assert!(d.contains(&(Change::Added, "句子".to_string())));
        // Too long to compare: one unmarked piece.
        let (long, other) = ("w ".repeat(1200), "x ".repeat(600));
        assert_eq!(
            word_diff(&long, &other),
            vec![(Change::Same, other.clone())]
        );
    }

    #[test]
    fn actions_read_from_json_with_defaults_for_what_is_missing() {
        let a: Action = serde_json::from_str(r#"{"id": "custom", "name": "Mine"}"#).unwrap();
        assert_eq!(a.output, Output::Rewrite);
        assert_eq!(a.prompt, "");
        let a: Action = serde_json::from_str(r#"{"id": "x", "output": "Answer"}"#).unwrap();
        assert_eq!(a.output, Output::Answer);
    }
}
