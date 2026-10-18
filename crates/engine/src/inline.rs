//! Inline AI suggestions in the editor (like an IDE's AI autocomplete): after a pause in typing
//! the text around the caret goes to the chosen model, and its continuation shows as grey text
//! at the caret; Tab inserts it, ⌘→ its next word, Esc dismisses it, typing what it says keeps
//! the rest. Settings: `texcraft_ai::inline::InlineSettings` (in the AI configuration).

use std::sync::Arc;
use std::time::{Duration, Instant};

use texcraft_ai::Event;
use texcraft_ai::inline as ai_inline;
use texcraft_i18n::{tr, trf};

use crate::{CommandError, Session, edit};

/// A suggestion ready to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The document it belongs to (index) and its revision.
    pub doc: usize,
    /// The document's revision when it was made.
    pub revision: u64,
    /// The caret (character index) it continues from.
    pub at: usize,
    /// The text to insert.
    pub text: String,
}

/// A request on its way.
struct Pending {
    doc: usize,
    revision: u64,
    at: usize,
    before: String,
    after: String,
    answer: String,
    /// The model reasoned (sent thinking) on the way.
    thought: bool,
    started: Instant,
    job: texcraft_ai::Job,
}

/// How much of a suggestion to accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accept {
    /// All of it (Tab).
    All,
    /// Its next sentence (⌘→).
    Sentence,
    /// Its next word.
    Word,
}

/// Inline suggestion state of a session.
#[derive(Default)]
pub struct InlineState {
    suggestion: Option<Suggestion>,
    pending: Option<Pending>,
    /// When to ask (a pause in typing ends then).
    pub(crate) due: Option<Instant>,
    /// What went wrong with the last request (shown in the settings popover).
    pub last_error: Option<String>,
    /// How long the last answer took.
    pub last_latency: Option<Duration>,
}

impl InlineState {
    /// A request is running.
    pub fn is_busy(&self) -> bool {
        self.pending.is_some()
    }
}

/// Files suggestions are offered in (LaTeX and its companions).
fn suggestible(path: Option<&std::path::Path>) -> bool {
    path.and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .is_none_or(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "tex" | "ltx" | "sty" | "cls" | "bib" | "dtx" | "md" | "txt"
            )
        })
}

impl Session {
    /// The suggestion to show now: for the active document, at its caret, with nothing
    /// selected and the text unchanged since.
    pub fn inline_suggestion(&self) -> Option<&Suggestion> {
        let s = self.inline.suggestion.as_ref()?;
        let d = self.docs.get(s.doc)?;
        (s.doc == self.active
            && s.revision == d.revision
            && d.selection.is_empty()
            && d.selection.cursor == s.at)
            .then_some(s)
    }

    /// Whether suggestions can be asked for at all (a model is set up).
    pub fn inline_available(&self) -> bool {
        self.ai.config.inline_selection().is_some_and(|(p, _)| {
            texcraft_ai::is_local(p) || self.ai.keys.as_ref().is_some_and(|k| k.has(&p.id))
        })
    }

    /// After an edit: keeps the rest of a suggestion the user is typing out, otherwise drops
    /// it, stops a running request and waits for the next pause.
    pub(crate) fn inline_after_edit(&mut self) {
        let typed_through = self.inline.suggestion.as_ref().and_then(|s| {
            let d = self.docs.get(s.doc)?;
            let caret = d.selection.cursor;
            let n = caret.checked_sub(s.at).filter(|n| *n > 0)?;
            if s.doc != self.active || !d.selection.is_empty() {
                return None;
            }
            let typed = d
                .text
                .get(edit::byte_of(&d.text, s.at)..edit::byte_of(&d.text, caret))?;
            let rest = s.text.strip_prefix(typed)?;
            (!rest.is_empty() && n == typed.chars().count()).then(|| Suggestion {
                doc: s.doc,
                revision: d.revision,
                at: caret,
                text: rest.to_string(),
            })
        });
        self.cancel_inline_request();
        self.inline.suggestion = typed_through;
        self.inline.due = None;
        if self.inline.suggestion.is_none()
            && self.ai.config.inline.enabled
            && self.inline_available()
        {
            let delay = u64::from(self.ai.config.inline.delay_ms.clamp(100, 5000));
            self.inline.due = Some(Instant::now() + Duration::from_millis(delay));
        }
    }

    /// Asks for a suggestion now (⌥\), even with suggestions while typing off.
    pub fn inline_trigger(&mut self) -> Result<(), CommandError> {
        self.cancel_inline_request();
        self.inline.suggestion = None;
        self.inline.due = None;
        self.start_inline_request()
    }

    /// Drops the suggestion (Esc) until the next pause in typing.
    pub fn inline_dismiss(&mut self) {
        self.cancel_inline_request();
        self.inline.suggestion = None;
        self.inline.due = None;
    }

    /// Inserts the suggestion (Tab), its next sentence (⌘→) or its next word.
    pub fn inline_accept(&mut self, part: Accept) -> Result<(), CommandError> {
        let Some(s) = self.inline_suggestion().cloned() else {
            return Err(CommandError::Failed(tr("no suggestion to accept").into()));
        };
        let take = match part {
            Accept::All => s.text.clone(),
            Accept::Sentence => ai_inline::first_sentence(&s.text).to_string(),
            Accept::Word => ai_inline::first_word(&s.text).to_string(),
        };
        let rest = s.text.get(take.len()..).unwrap_or("").to_string();
        self.run("edit.insertText", &serde_json::json!({ "text": take }))?;
        let (revision, at) = self
            .docs
            .get(s.doc)
            .map_or((0, 0), |d| (d.revision, d.selection.cursor));
        self.cancel_inline_request();
        self.inline.due = None;
        let done = rest.is_empty();
        self.inline.suggestion = (!done).then_some(Suggestion {
            doc: s.doc,
            revision,
            at,
            text: rest,
        });
        // All of it taken: ask for what comes next straight away, so Tab, Tab, … keeps writing.
        if done && self.ai.config.inline.enabled && self.inline_available() {
            self.inline.due = Some(Instant::now());
        }
        Ok(())
    }

    /// The caret moved without an edit (a click, the arrow keys): suggestions follow typing only,
    /// so the suggestion goes and nothing is asked for until the next pause in typing.
    pub fn inline_after_caret_move(&mut self) {
        self.cancel_inline_request();
        self.inline.suggestion = None;
        self.inline.due = None;
    }

    fn cancel_inline_request(&mut self) {
        if let Some(p) = self.inline.pending.take() {
            p.job.cancel();
        }
    }

    fn start_inline_request(&mut self) -> Result<(), CommandError> {
        let Some((provider, model)) = self
            .ai
            .config
            .inline_selection()
            .map(|(p, m)| (p.clone(), m.to_string()))
        else {
            return Err(CommandError::Failed(
                tr("Set up an AI provider in Settings → AI first.").into(),
            ));
        };
        let api_key = self.ai_key_for(&provider);
        if api_key.trim().is_empty() && !texcraft_ai::is_local(&provider) {
            return Err(CommandError::Failed(trf!(
                "Add the API key for {provider} in Settings → AI.",
                provider = provider.name
            )));
        }
        let Some(d) = self.docs.get(self.active) else {
            return Ok(());
        };
        if !d.selection.is_empty() || !suggestible(d.path.as_deref()) {
            return Ok(());
        }
        let mut settings = self.ai.config.inline.clone();
        settings.clamp();
        let at = d.selection.cursor;
        let caret = edit::byte_of(&d.text, at);
        let before = ai_inline::tail(
            d.text.get(..caret).unwrap_or(""),
            settings.context_before as usize,
        )
        .to_string();
        let after = ai_inline::head(
            d.text.get(caret..).unwrap_or(""),
            settings.context_after as usize,
        )
        .to_string();
        if before.trim().is_empty() && after.trim().is_empty() {
            return Ok(());
        }
        let file = d.path.as_ref().and_then(|p| p.file_name()).map_or_else(
            || "untitled.tex".into(),
            |n| n.to_string_lossy().into_owned(),
        );
        let req = ai_inline::request(
            &provider, &model, api_key, &file, &before, &after, &settings,
        );
        self.inline.pending = Some(Pending {
            doc: self.active,
            revision: d.revision,
            at,
            before,
            after,
            answer: String::new(),
            thought: false,
            started: Instant::now(),
            job: texcraft_ai::start(&req, Arc::clone(&self.waker)),
        });
        Ok(())
    }

    /// Starts a request when a pause in typing ends and collects answers. Returns when to look
    /// again.
    pub(crate) fn tick_inline(&mut self, now: Instant) -> Option<Duration> {
        if let Some(due) = self.inline.due {
            if now >= due {
                self.inline.due = None;
                if let Err(e) = self.start_inline_request() {
                    self.inline.last_error = Some(e.to_string());
                }
            } else {
                return Some(due - now);
            }
        }
        let events: Vec<Event> = self
            .inline
            .pending
            .as_ref()
            .map(|p| std::iter::from_fn(|| p.job.try_recv()).collect())
            .unwrap_or_default();
        for e in events {
            let Some(p) = self.inline.pending.as_mut() else {
                break;
            };
            match e {
                Event::Text(t) => p.answer.push_str(&t),
                Event::Thinking => p.thought = true,
                Event::Fallback(_) => {}
                Event::Done { .. } => {
                    if let Some(p) = self.inline.pending.take() {
                        self.inline.last_latency = Some(p.started.elapsed());
                        // A reasoning model can spend the whole small budget thinking: say so
                        // rather than show nothing.
                        self.inline.last_error = (p.thought && p.answer.trim().is_empty())
                            .then(|| only_thought(&self.ai.config));
                        let text = ai_inline::clean(
                            &p.answer,
                            &p.before,
                            &p.after,
                            self.ai.config.inline.max_lines,
                        );
                        let current = self
                            .docs
                            .get(p.doc)
                            .is_some_and(|d| d.revision == p.revision);
                        self.inline.suggestion =
                            (current && !text.is_empty()).then_some(Suggestion {
                                doc: p.doc,
                                revision: p.revision,
                                at: p.at,
                                text,
                            });
                    }
                }
                Event::Refused(_) => {
                    self.inline.pending = None;
                    self.inline.last_error = Some(tr("The model declined to suggest here.").into());
                }
                Event::Failed(msg) => {
                    self.inline.pending = None;
                    self.inline.last_error = Some(msg);
                }
            }
        }
        None
    }

    /// The inline-suggestion part of `state.inspect`.
    pub(crate) fn inline_inspect(&self) -> serde_json::Value {
        let s = &self.ai.config.inline;
        let model = self
            .ai
            .config
            .inline_selection()
            .map(|(p, m)| format!("{} · {m}", p.name));
        serde_json::json!({
            "enabled": s.enabled,
            "available": self.inline_available(),
            "model": model,
            "delayMs": s.delay_ms,
            "maxLines": s.max_lines,
            "contextBefore": s.context_before,
            "contextAfter": s.context_after,
            "busy": self.inline.is_busy(),
            "waiting": self.inline.due.is_some(),
            "suggestion": self.inline_suggestion().map(|s| s.text.clone()),
            "lastError": self.inline.last_error,
            "lastLatencyMs": self.inline.last_latency.map(|d| d.as_millis() as u64),
        })
    }
}

/// Why a suggestion did not come: the model reasoned and wrote nothing.
fn only_thought(config: &texcraft_ai::Config) -> String {
    let model = config
        .inline_selection()
        .map_or_else(|| tr("The model").to_string(), |(_, m)| m.to_string());
    trf!(
        "{model} spent the suggestion on reasoning and wrote nothing. Pick a model that \
         answers without reasoning for suggestions.",
        model = model
    )
}
