//! The editor's right-click AI actions (Settings → AI → Actions), as macOS Writing Tools or
//! Notion AI do them: the selection, or the paragraph at the caret when nothing is selected, goes
//! to the model with the action's instruction in a request of its own (the Assistant's
//! conversation is not involved), and the answer streams into a card over the editor
//! ([`ActionRun`]). A rewrite can then replace the text (⌘Z undoes it); an answer can be carried
//! into the Assistant to talk about it further.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use texcraft_ai::actions::{self, Output};
use texcraft_ai::{ChatRequest, Event, Job, Role, Turn};
use texcraft_i18n::{Lang, tr, trf};

use crate::ai::ChatMessage;
use crate::edit::{self, Selection};
use crate::{CommandError, Effect, Session};

/// A right-click action that ran: what it ran on and its answer (the editor's result card).
pub struct ActionRun {
    /// The action's id.
    pub action: String,
    /// Its name, in the interface language.
    pub label: String,
    /// What its answer is for.
    pub output: Output,
    /// The question sent (the instruction and the passage).
    pub question: String,
    /// The text's document (the file, or an untitled document's scratch file).
    pub doc: Option<PathBuf>,
    /// Where the text starts (characters), and the text.
    pub start: usize,
    pub original: String,
    /// The answer so far.
    pub answer: String,
    /// The answer is still arriving.
    pub pending: bool,
    /// The model is thinking (no text yet).
    pub thinking: bool,
    /// What went wrong.
    pub error: Option<String>,
    /// The model asked, and its provider's id (for the logo).
    pub model: String,
    pub provider: String,
    /// Counts the requests made for it (Try Again), so the card can start afresh.
    pub serial: u32,
    req: ChatRequest,
    job: Option<Job>,
}

impl ActionRun {
    /// A rewrite's passage and note, from the answer so far.
    pub fn reply(&self) -> actions::Reply {
        actions::split_reply(&self.answer)
    }

    /// Finished with a usable answer (a rewrite with text, or any answer).
    pub fn is_ready(&self) -> bool {
        !self.pending && self.error.is_none() && !self.reply().text.trim().is_empty()
    }
}

/// Lines that start or end a paragraph besides blank ones: headings and environments.
const BOUNDARIES: &[&str] = &[
    "\\begin",
    "\\end",
    "\\part",
    "\\chapter",
    "\\section",
    "\\subsection",
    "\\subsubsection",
    "\\paragraph",
    "\\subparagraph",
    "\\documentclass",
    "\\usepackage",
    "\\maketitle",
];

fn is_boundary(line: &str) -> bool {
    let l = line.trim_start();
    l.trim().is_empty() || BOUNDARIES.iter().any(|b| l.starts_with(b))
}

/// The paragraph around character `at` of `text`, in characters: the lines between blank lines,
/// without the heading or `\begin` / `\end` lines next to it. On such a line, that line alone;
/// on a blank line, nothing.
pub fn paragraph_at(text: &str, at: usize) -> Option<Range<usize>> {
    let caret = edit::byte_of(text, at);
    // (start, end) byte span of each line, without its line break.
    let mut lines = Vec::new();
    let mut start = 0usize;
    for line in text.split('\n') {
        let end = start.saturating_add(line.len());
        lines.push((start, end.saturating_sub(usize::from(line.ends_with('\r')))));
        start = end.saturating_add(1);
    }
    let line = |i: usize| {
        lines
            .get(i)
            .and_then(|&(a, b)| text.get(a..b))
            .unwrap_or("")
    };
    let i = lines.iter().position(|&(_, b)| caret <= b)?;
    let here = line(i);
    if here.trim().is_empty() {
        return None;
    }
    let (mut first, mut last) = (i, i);
    if !is_boundary(here) {
        while first > 0 && !is_boundary(line(first - 1)) {
            first -= 1;
        }
        while last + 1 < lines.len() && !is_boundary(line(last + 1)) {
            last += 1;
        }
    }
    let a = lines.get(first)?.0;
    let b = lines.get(last)?.1;
    Some(edit::char_of(text, a)..edit::char_of(text, b))
}

/// Where `needle` is in `text` (bytes): at byte `near` if it is there, else the occurrence
/// closest to it.
pub fn locate(text: &str, needle: &str, near: usize) -> Option<Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    let end = near.saturating_add(needle.len());
    if text.get(near..end) == Some(needle) {
        return Some(near..end);
    }
    text.match_indices(needle)
        .map(|(i, _)| i)
        .min_by_key(|i| i.abs_diff(near))
        .map(|i| i..i.saturating_add(needle.len()))
}

impl Session {
    /// Whether the actions can run: a document is open and a model with its key is set up.
    pub fn ai_actions_ready(&self) -> bool {
        self.doc().is_some()
            && self
                .ai
                .config
                .selection()
                .is_some_and(|(p, _)| p.has_key || texcraft_ai::is_local(p))
    }

    /// What an action works on in the active document: the selection, else the paragraph at the
    /// caret (characters, and the text).
    pub fn ai_action_target(&self) -> Option<(Range<usize>, String)> {
        let doc = self.doc()?;
        let sel = doc.selection;
        let range = if sel.is_empty() {
            paragraph_at(&doc.text, sel.cursor)?
        } else {
            sel.start()..sel.end()
        };
        let text = doc
            .text
            .get(edit::byte_of(&doc.text, range.start)..edit::byte_of(&doc.text, range.end))?;
        (!text.trim().is_empty()).then(|| (range, text.to_string()))
    }

    /// Runs action `id` on the selection, else on the paragraph at the caret (which it
    /// selects): a request of its own whose answer streams into [`ActionRun`]. A run already
    /// showing is dropped.
    pub fn ai_run_action(&mut self, id: &str) -> Result<(), CommandError> {
        let action = self
            .ai
            .config
            .actions
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| CommandError::Failed(trf!("No AI action “{id}”.", id = id)))?;
        let prompt = action.prompt().to_string();
        if prompt.is_empty() {
            return Err(CommandError::Failed(trf!(
                "“{name}” has no prompt: write one in Settings → AI → Actions.",
                name = action.label()
            )));
        }
        let (provider, model, api_key) = self.ai_target()?;
        let (range, text) = self.ai_action_target().ok_or_else(|| {
            CommandError::Failed(
                tr("Select some text first, or put the caret in a paragraph.").into(),
            )
        })?;
        let active = self.active;
        let doc = self
            .doc()
            .and_then(|d| d.typeset_path().map(Path::to_path_buf));
        // The paragraph it works on shows selected.
        if let Some(d) = self.doc_mut()
            && d.selection.is_empty()
        {
            d.selection = Selection {
                anchor: range.start,
                cursor: range.end,
            };
            self.effect(Effect::SelectionChanged { doc: active });
        }
        let explain_in = (self.lang() == Lang::ZhHans).then_some("Simplified Chinese");
        // The Assistant's instructions and the open document (no tools, no conversation).
        let (_, _, system) = self.ai_prompt();
        let question = actions::message(&prompt, &text);
        let req = ChatRequest {
            provider: provider.clone(),
            model: model.clone(),
            system: format!("{system}{}", actions::system(action.output, explain_in)),
            api_key,
            turns: vec![Turn {
                role: Role::User,
                text: question.clone(),
                ..Default::default()
            }],
            max_tokens: None,
            no_reasoning: false,
            tools: Vec::new(),
        };
        self.ai_action_dismiss();
        let job = texcraft_ai::start(&req, Arc::clone(&self.waker));
        self.ai.action = Some(ActionRun {
            action: action.id.clone(),
            label: action.label(),
            output: action.output,
            question,
            doc,
            start: range.start,
            original: text,
            answer: String::new(),
            pending: true,
            thinking: false,
            error: None,
            model,
            provider: provider.id,
            serial: 0,
            req,
            job: Some(job),
        });
        Ok(())
    }

    /// Polls the action's request.
    pub(crate) fn tick_ai_action(&mut self) -> Option<std::time::Duration> {
        let run = self.ai.action.as_mut()?;
        let job = run.job.as_ref()?;
        let events: Vec<Event> = std::iter::from_fn(|| job.try_recv()).collect();
        for e in events {
            match e {
                Event::Text(t) => {
                    run.thinking = false;
                    run.answer.push_str(&t);
                }
                Event::Thinking => run.thinking = run.answer.is_empty(),
                Event::Done { model, .. } => {
                    if let Some(m) = model {
                        run.model = m;
                    }
                    if run.answer.trim().is_empty() {
                        run.error = Some(tr("The model sent an empty answer.").into());
                    }
                    run.pending = false;
                }
                Event::Refused(category) => {
                    run.answer.clear();
                    run.error = Some(match category {
                        Some(c) => trf!("The model declined this request ({reason}).", reason = c),
                        None => tr("The model declined this request.").into(),
                    });
                    run.pending = false;
                }
                Event::Failed(msg) => {
                    run.error = Some(msg);
                    run.pending = false;
                }
                Event::Fallback(_) | Event::Usage(_) => {}
            }
        }
        if !run.pending {
            run.thinking = false;
            run.job = None;
            return None;
        }
        Some(std::time::Duration::from_millis(100))
    }

    /// The action's run, finished with a usable answer, or why there is none.
    fn ready_run(&self) -> Result<&ActionRun, CommandError> {
        let run = self
            .ai
            .action
            .as_ref()
            .ok_or_else(|| CommandError::Failed(tr("No AI action has run.").into()))?;
        if run.pending {
            return Err(CommandError::Failed(
                tr("Wait for the answer first.").into(),
            ));
        }
        if !run.is_ready() {
            return Err(CommandError::Failed(
                tr("The answer has no text to apply.").into(),
            ));
        }
        Ok(run)
    }

    /// Replaces the text a rewrite action ran on with the rewrite (selected after), and
    /// closes the result. Undo (⌘Z) brings the original back.
    pub fn ai_action_replace(&mut self) -> Result<(), CommandError> {
        let run = self.ready_run()?;
        if run.output != Output::Rewrite {
            return Err(CommandError::Failed(
                tr("That answer is not a rewrite of the document's text.").into(),
            ));
        }
        let (doc, start, original) = (run.doc.clone(), run.start, run.original.clone());
        let new = run.reply().text;
        self.ai_swap(doc.as_deref(), start, &original, &new)?;
        self.ai.action = None;
        Ok(())
    }

    /// Asks again (another answer to the same question).
    pub fn ai_action_retry(&mut self) -> Result<(), CommandError> {
        let waker = Arc::clone(&self.waker);
        let run = self
            .ai
            .action
            .as_mut()
            .ok_or_else(|| CommandError::Failed(tr("No AI action has run.").into()))?;
        if let Some(job) = run.job.take() {
            job.cancel();
        }
        run.answer.clear();
        run.error = None;
        run.pending = true;
        run.thinking = false;
        run.serial = run.serial.saturating_add(1);
        run.job = Some(texcraft_ai::start(&run.req, waker));
        Ok(())
    }

    /// Copies the result (a rewrite's passage, else the whole answer).
    pub fn ai_action_copy(&mut self) -> Result<(), CommandError> {
        let run = self.ready_run()?;
        let text = match run.output {
            Output::Rewrite => run.reply().text,
            Output::Answer => run.answer.trim().to_string(),
        };
        self.effect(Effect::CopyText(text));
        Ok(())
    }

    /// Closes the result (stopping its request).
    pub fn ai_action_dismiss(&mut self) {
        if let Some(job) = self.ai.action.take().and_then(|r| r.job) {
            job.cancel();
        }
    }

    /// Carries the question and its answer into the Assistant's conversation (to ask more
    /// about it there), and closes the result.
    pub fn ai_action_to_assistant(&mut self) -> Result<(), CommandError> {
        if self.ai.is_busy() {
            return Err(CommandError::Failed(
                tr("Wait for the answer, or stop it first.").into(),
            ));
        }
        let run = self.ready_run()?;
        let message =
            |role, text: String, model: Option<String>, provider: Option<String>| ChatMessage {
                role,
                text,
                blocks: Vec::new(),
                model,
                provider,
                pending: false,
                thinking: false,
                error: None,
                note: None,
                images: Vec::new(),
                tools: Vec::new(),
            };
        let question = message(Role::User, run.question.clone(), None, None);
        let answer = message(
            Role::Assistant,
            run.answer.trim().to_string(),
            Some(run.model.clone()),
            Some(run.provider.clone()),
        );
        self.ai.messages.push(question);
        self.ai.messages.push(answer);
        self.ai_action_dismiss();
        self.view.assistant = true;
        self.ai_save_chat();
        Ok(())
    }

    /// Replaces `from`, where it starts at character `start` (or its nearest copy), with `to`
    /// in document `doc`, which becomes the active one with `to` selected.
    fn ai_swap(
        &mut self,
        doc: Option<&Path>,
        start: usize,
        from: &str,
        to: &str,
    ) -> Result<(), CommandError> {
        let idx = self
            .docs
            .iter()
            .position(|d| d.typeset_path() == doc)
            .ok_or_else(|| {
                CommandError::Failed(tr("The document of that text is no longer open.").into())
            })?;
        let d = self
            .docs
            .get_mut(idx)
            .ok_or_else(|| CommandError::Failed(tr("no document").into()))?;
        let near = edit::byte_of(&d.text, start);
        let at = locate(&d.text, from, near).ok_or_else(|| {
            CommandError::Failed(
                tr("That text is no longer in the document: it changed after the action ran.")
                    .into(),
            )
        })?;
        let (a, b) = (
            edit::char_of(&d.text, at.start),
            edit::char_of(&d.text, at.end),
        );
        d.selection = Selection {
            anchor: a,
            cursor: b,
        };
        self.active = idx;
        self.edit_active(|t, sel| edit::replace_selection(t, sel, to))?;
        if let Some(d) = self.doc_mut() {
            d.selection = Selection {
                anchor: a,
                cursor: a.saturating_add(to.chars().count()),
            };
        }
        if let Some(line) = self.doc().map(|d| edit::line_of_char(&d.text, a)) {
            self.effect(Effect::RevealLine { doc: idx, line });
        }
        Ok(())
    }

    /// The actions and the result shown, for `state.inspect`.
    pub(crate) fn ai_actions_inspect(&self) -> serde_json::Value {
        let run = self.ai.action.as_ref().map(|r| {
            let reply = r.reply();
            serde_json::json!({
                "action": r.action, "label": r.label,
                "output": match r.output { Output::Rewrite => "rewrite", Output::Answer => "answer" },
                "start": r.start, "chars": r.original.chars().count(),
                "pending": r.pending, "error": r.error, "model": r.model,
                "answer": r.answer, "rewrite": reply.text, "note": reply.note,
            })
        });
        serde_json::json!({
            "ready": self.ai_actions_ready(),
            "list": self.ai.config.actions.iter().map(|a| serde_json::json!({
                "id": a.id,
                "label": a.label(),
                "output": match a.output { Output::Rewrite => "rewrite", Output::Answer => "answer" },
                "prompt": a.prompt(),
                "builtin": a.builtin().is_some(),
                "promptChanged": a.prompt_changed(),
            })).collect::<Vec<_>>(),
            "run": run,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::fake_ai_server;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const DOC: &str = "\\section{Intro}\nFirst line of text.\nSecond line.\n\n\\begin{itemize}\n\\item One\n\\end{itemize}\n";

    #[test]
    fn the_paragraph_at_the_caret() {
        let at = |s: &str| DOC.find(s).map(|b| edit::char_of(DOC, b)).unwrap();
        let text = |r: Range<usize>| {
            DOC.get(edit::byte_of(DOC, r.start)..edit::byte_of(DOC, r.end))
                .unwrap()
                .to_string()
        };
        // Inside the paragraph: its lines, without the heading over it.
        let r = paragraph_at(DOC, at("line of")).unwrap();
        assert_eq!(text(r), "First line of text.\nSecond line.");
        // At its very end too.
        let r = paragraph_at(DOC, at("\n\n")).unwrap();
        assert_eq!(text(r), "First line of text.\nSecond line.");
        // On a heading or \begin line: that line.
        assert_eq!(
            text(paragraph_at(DOC, at("Intro")).unwrap()),
            "\\section{Intro}"
        );
        assert_eq!(text(paragraph_at(DOC, at("One")).unwrap()), "\\item One");
        // On a blank line, or past the end: nothing.
        assert_eq!(paragraph_at(DOC, at("\n\n") + 1), None);
        assert_eq!(paragraph_at(DOC, DOC.chars().count()), None);
        // Characters, not bytes; Windows line ends left out.
        let t = "中文段落。\r\nsecond\r\n\r\nnext";
        assert_eq!(paragraph_at(t, 2), Some(0..13));
        assert_eq!(paragraph_at("", 0), None);
    }

    #[test]
    fn locating_text_that_moved() {
        let t = "aa XY bb XY cc";
        assert_eq!(locate(t, "XY", 3), Some(3..5));
        assert_eq!(locate(t, "XY", 10), Some(9..11));
        assert_eq!(locate(t, "XY", 1), Some(3..5));
        assert_eq!(locate(t, "XY", 100), Some(9..11));
        assert_eq!(locate(t, "QQ", 3), None);
        assert_eq!(locate(t, "", 3), None);
    }

    fn session_with(text: &str, port: u16) -> Session {
        let mut s = Session::new(Arc::new(|| {}));
        s.settings.live_typeset = false;
        s.view.language = crate::LanguageChoice::English;
        s.update.url = "/nonexistent/texcraft-latest.json".into();
        let ollama = texcraft_ai::PRESETS
            .iter()
            .find(|p| p.id == "ollama")
            .unwrap();
        let i = s.ai.config.add(ollama);
        s.ai.config.providers[i].base_url = format!("http://127.0.0.1:{port}/v1");
        s.ai.config.providers[i].models = vec!["local-model".into()];
        s.ai.config.normalize();
        s.run("edit.setText", &json!({ "text": text })).unwrap();
        s
    }

    fn wait(s: &mut Session) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while s.ai.action.as_ref().is_some_and(|r| r.pending) && Instant::now() < deadline {
            s.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn actions_need_a_model_and_some_text() {
        let mut s = Session::new(Arc::new(|| {}));
        s.settings.live_typeset = false;
        s.run("edit.setText", &json!({ "text": "Hello.\n\nWorld." }))
            .unwrap();
        assert!(!s.is_enabled("ai.runAction"), "no model yet");
        assert!(s.run("ai.runAction", &json!({ "id": "polish" })).is_err());
        let mut s = session_with("Hello.\n\nWorld.", 9);
        assert!(s.is_enabled("ai.runAction"));
        assert!(matches!(
            s.run("ai.runAction", &Value::Null),
            Err(CommandError::Params(_))
        ));
        assert!(s.run("ai.runAction", &json!({ "id": "nope" })).is_err());
        // On the blank line between the paragraphs there is nothing to work on.
        s.run("edit.select", &json!({ "anchor": 7 })).unwrap();
        let err = s
            .run("ai.runAction", &json!({ "id": "polish" }))
            .unwrap_err();
        assert!(err.to_string().contains("Select some text"), "{err}");
        assert!(s.ai.action.is_none());
        // A custom action without a prompt says so.
        s.ai.config.actions.push(texcraft_ai::actions::Action {
            id: "custom".into(),
            name: "Mine".into(),
            ..Default::default()
        });
        s.run("edit.select", &json!({ "anchor": 2 })).unwrap();
        let err = s
            .run("ai.runAction", &json!({ "id": "custom" }))
            .unwrap_err();
        assert!(err.to_string().contains("no prompt"), "{err}");
        // Nothing to replace yet.
        assert!(!s.is_enabled("ai.action.replace"));
        assert!(s.run("ai.action.replace", &Value::Null).is_err());
    }

    /// Polish on the paragraph at the caret: a request of its own (the Assistant's conversation
    /// stays as it is) with the action's prompt and the paragraph; Replace puts the rewrite in
    /// place of the paragraph even after edits above it.
    #[test]
    fn a_rewrite_runs_on_the_paragraph_and_replaces_it() {
        let (port, server) = fake_ai_server(
            "```latex\nThe first sentence, polished.\nThe second one.\n```\nTightened the wording.",
        );
        let doc = "\\section{A}\nThe first sentence.\nThe second.\n\nNext paragraph.\n";
        let mut s = session_with(doc, port);
        s.view.assistant = false;
        s.ai.messages.push(ChatMessage {
            role: Role::User,
            text: "An earlier question".into(),
            blocks: Vec::new(),
            model: None,
            provider: None,
            pending: false,
            thinking: false,
            error: None,
            note: None,
            images: Vec::new(),
            tools: Vec::new(),
        });
        s.run("edit.select", &json!({ "anchor": 15 })).unwrap(); // in "first"
        s.run("ai.runAction", &json!({ "id": "polish" })).unwrap();
        // The paragraph shows selected; the result shows in the editor, not the Assistant.
        let sel = s.doc().unwrap().selection;
        assert_eq!((sel.start(), sel.end()), (12, 43));
        assert!(!s.view.assistant);
        assert_eq!(s.ai.messages.len(), 1, "the conversation is left alone");
        let run = s.ai.action.as_ref().unwrap();
        assert!(run.pending);
        assert_eq!(run.label, "Polish");
        assert!(
            run.question
                .starts_with("Polish the wording of this passage"),
            "{}",
            run.question
        );
        assert!(
            run.question
                .ends_with("```latex\nThe first sentence.\nThe second.\n```")
        );
        assert!(!s.is_enabled("ai.action.replace"), "not while it streams");
        wait(&mut s);
        let seen = server.join().unwrap();
        assert!(seen.contains("single fenced code block"), "{seen}");
        assert!(
            !seen.contains("An earlier question"),
            "one question only: {seen}"
        );
        assert!(!seen.contains("\"tools\""), "no tools: {seen}");
        assert!(
            !seen.contains("Write explanations in"),
            "English interface: {seen}"
        );
        let run = s.ai.action.as_ref().unwrap();
        assert!(!run.pending && run.error.is_none());
        assert_eq!(
            run.reply().text,
            "The first sentence, polished.\nThe second one."
        );
        assert_eq!(run.reply().note, "Tightened the wording.");
        let inspect = s.inspect();
        assert_eq!(inspect["ai"]["actions"]["run"]["action"], "polish");
        assert_eq!(
            inspect["ai"]["actions"]["run"]["note"],
            "Tightened the wording."
        );
        assert_eq!(inspect["ai"]["actions"]["list"][0]["id"], "polish");

        s.run("ai.action.copy", &Value::Null).unwrap();
        assert!(s.effects.iter().any(
            |e| matches!(e, Effect::CopyText(t) if t == "The first sentence, polished.\nThe second one.")
        ));
        // Meanwhile the user typed above it: the text is found where it moved.
        s.run("edit.select", &json!({ "anchor": 0 })).unwrap();
        s.run("edit.insertText", &json!({ "text": "% note\n" }))
            .unwrap();
        assert!(s.is_enabled("ai.action.replace"));
        s.run("ai.action.replace", &Value::Null).unwrap();
        assert_eq!(
            s.doc().unwrap().text,
            "% note\n\\section{A}\nThe first sentence, polished.\nThe second one.\n\nNext paragraph.\n"
        );
        let sel = s.doc().unwrap().selection;
        assert_eq!(
            (sel.start(), sel.end()),
            (
                19,
                19 + "The first sentence, polished.\nThe second one.".len()
            ),
            "the new text shows selected"
        );
        assert!(
            s.effects
                .iter()
                .any(|e| matches!(e, Effect::RevealLine { line: 3, .. }))
        );
        assert!(s.ai.action.is_none(), "the result closes");
        assert!(s.run("ai.action.replace", &Value::Null).is_err());
    }

    /// Try Again asks the same question once more; Discard closes the result; a failed
    /// request shows its error and nothing to replace.
    #[test]
    fn try_again_discard_and_failures() {
        let mut s = session_with("Some text.\n", 9);
        s.run("ai.runAction", &json!({ "id": "grammar" })).unwrap();
        wait(&mut s);
        let run = s.ai.action.as_ref().unwrap();
        assert!(run.error.is_some(), "nothing listens on port 9");
        assert!(!run.is_ready());
        assert!(s.run("ai.action.replace", &Value::Null).is_err());
        // Pointed at a server that answers, Try Again asks the same question.
        let (port, server) = fake_ai_server("```latex\nSome better text.\n```");
        s.ai.action.as_mut().unwrap().req.provider.base_url = format!("http://127.0.0.1:{port}/v1");
        s.run("ai.action.retry", &Value::Null).unwrap();
        let run = s.ai.action.as_ref().unwrap();
        assert!(run.pending && run.error.is_none() && run.serial == 1);
        wait(&mut s);
        assert!(server.join().unwrap().contains("Correct the grammar"));
        assert_eq!(
            s.ai.action.as_ref().unwrap().reply().text,
            "Some better text."
        );
        s.run("ai.action.dismiss", &Value::Null).unwrap();
        assert!(s.ai.action.is_none());
        assert_eq!(s.doc().unwrap().text, "Some text.\n", "nothing replaced");
    }

    /// Explain answers in the card, in the interface language; Continue in Assistant carries
    /// the question and the answer into the conversation.
    #[test]
    fn an_answer_action_continues_in_the_assistant() {
        let (port, server) = fake_ai_server("It is a section heading.");
        let mut s = session_with("\\section{Results}\nText.\n", port);
        s.run("edit.select", &json!({ "anchor": 0, "cursor": 17 }))
            .unwrap();
        {
            let _zh = texcraft_i18n::scoped(Lang::ZhHans);
            s.view.language = crate::LanguageChoice::ChineseSimplified;
            s.run("ai.runAction", &json!({ "id": "explain" })).unwrap();
            assert_eq!(s.ai.action.as_ref().unwrap().label, "解释");
        }
        let sel = s.doc().unwrap().selection;
        assert_eq!((sel.start(), sel.end()), (0, 17), "the selection is kept");
        wait(&mut s);
        let seen = server.join().unwrap();
        assert!(
            seen.contains("Write explanations in Simplified Chinese"),
            "{seen}"
        );
        assert!(!seen.contains("single fenced code block"), "{seen}");
        assert!(
            s.run("ai.action.replace", &Value::Null).is_err(),
            "an answer is not a rewrite"
        );
        s.view.assistant = false;
        s.run("ai.action.toAssistant", &Value::Null).unwrap();
        assert!(s.ai.action.is_none());
        assert!(s.view.assistant);
        assert_eq!(s.ai.messages.len(), 2);
        assert!(
            s.ai.messages[0]
                .text
                .contains("```latex\n\\section{Results}\n```")
        );
        assert_eq!(s.ai.messages[1].text, "It is a section heading.");
        assert_eq!(s.ai.messages[1].model.as_deref(), Some("local-model"));
    }
}
