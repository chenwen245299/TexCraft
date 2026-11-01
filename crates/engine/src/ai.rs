//! The AI Assistant (the sidebar's sparkles tab): a conversation with a model from the
//! providers configured in Settings → AI, optionally about the open document.

use std::path::PathBuf;
use std::sync::Arc;

use texcraft_ai::{
    ChatRequest, Config, Event, Image, Job, KeyStore, Model, Role, ToolResult, Turn,
};
use texcraft_i18n::{tr, trf};

use crate::ai_tools::{self, ToolUse};
use crate::chats::{self, ChatStore, ChatSummary};
use crate::{CommandError, Effect, Session, edit};

/// One message of the conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    /// Who wrote it.
    pub role: Role,
    /// Its text (Markdown for answers).
    pub text: String,
    /// Anthropic content blocks of an answer, sent back with the next question.
    pub blocks: Vec<serde_json::Value>,
    /// The model that answered.
    pub model: Option<String>,
    /// The id of the provider that answered (for its logo).
    pub provider: Option<String>,
    /// The answer is still arriving.
    pub pending: bool,
    /// The model is thinking (no text yet).
    pub thinking: bool,
    /// What went wrong (the answer is not part of the conversation then).
    pub error: Option<String>,
    /// A note such as "declined, answered by …".
    pub note: Option<String>,
    /// Images attached to a question (sent again with every later question).
    pub images: Vec<Image>,
    /// The project files the model looked at while answering (read-only tools).
    pub tools: Vec<ToolUse>,
}

impl ChatMessage {
    fn user(text: String, images: Vec<Image>) -> Self {
        Self {
            images,
            role: Role::User,
            text,
            blocks: Vec::new(),
            model: None,
            provider: None,
            pending: false,
            thinking: false,
            error: None,
            note: None,
            tools: Vec::new(),
        }
    }
}

/// AI Assistant state.
#[derive(Default)]
pub struct AiState {
    /// Providers and the chosen model (Settings → AI).
    pub config: Config,
    /// Where the configuration is kept (`None`: not saved, as in tests).
    pub config_path: Option<PathBuf>,
    /// The encrypted API keys (next to the configuration).
    pub keys: Option<KeyStore>,
    /// The conversation.
    pub messages: Vec<ChatMessage>,
    /// The question being written.
    pub draft: String,
    /// Images attached to the question being written.
    pub draft_images: Vec<Image>,
    /// Why an image could not be attached (shown over the question box until the next try).
    pub notice: Option<String>,
    job: Option<Job>,
    /// The question being answered with tools: its request, grown by each round of calls.
    round: Option<Round>,
    /// Where conversations are saved (`None`: not saved, as in tests).
    pub chats: Option<ChatStore>,
    /// The conversation shown (`None`: new, not saved yet).
    pub chat_id: Option<String>,
    /// The current project's saved conversations, the latest first.
    pub history: Vec<ChatSummary>,
    /// The project folder the conversation belongs to (canonical; `None`: none).
    chat_project: Option<PathBuf>,
    /// The folder last seen (`None`: not looked yet), to notice another project opening.
    chat_seen: Option<Option<PathBuf>>,
}

/// An answer in rounds: the model calls tools, gets their answers and goes on.
struct Round {
    req: ChatRequest,
    /// Rounds of tool calls so far.
    n: u32,
    /// The text of this round (it goes back with the round's calls).
    text: String,
    /// The system prompt without the tools (for asking again without them).
    plain_system: String,
}

/// The most rounds of tool calls one answer may take.
pub const MAX_TOOL_ROUNDS: u32 = 12;

/// The most images one question takes, and the largest image (the chat APIs refuse more).
pub const MAX_IMAGES: usize = 10;
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

impl AiState {
    /// A request is running.
    pub fn is_busy(&self) -> bool {
        self.job.is_some()
    }
}

/// What the model is told about its role.
const SYSTEM: &str = "You are the assistant built into TexCraft, a LaTeX editor. Help the user \
write, fix and explain LaTeX documents. When you suggest LaTeX, put it in a fenced code block \
marked `latex` so it can be copied with one click; keep explanations short and to the point. \
When the user attaches images (figures, plots, photos of formulas or tables), read them and \
answer with LaTeX where it helps. If the open document is included below, refer to it by line numbers when \
helpful and keep its packages and conventions.";

impl Session {
    /// The model the Assistant talks to, with its name and capabilities.
    pub fn ai_model(&self) -> Option<&Model> {
        let (p, id) = self.ai.config.selection()?;
        p.models.iter().find(|m| m.id == id)
    }

    /// Whether the Assistant's model reads images (its Vision capability).
    pub fn ai_reads_images(&self) -> bool {
        self.ai_model()
            .is_some_and(|m| m.has(texcraft_ai::Capability::Vision))
    }

    /// Attaches an image (`bytes`, from a file named `name` or the clipboard) to the question
    /// being written. PNG, JPEG, GIF and WebP up to 20 MB, ten to a question, for models with
    /// Vision.
    pub fn ai_attach_image(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), CommandError> {
        if !self.ai_reads_images() {
            let label = self
                .ai_model()
                .map_or_else(|| tr("This model").to_string(), |m| m.label().to_string());
            return Err(CommandError::Failed(trf!(
                "{model} does not read images: pick a model with Vision (Settings → AI).",
                model = label
            )));
        }
        let Some(media_type) = texcraft_ai::image_media_type(&bytes) else {
            return Err(CommandError::Failed(trf!(
                "{name} is not a PNG, JPEG, GIF or WebP image.",
                name = name
            )));
        };
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(CommandError::Failed(trf!(
                "{name} is larger than {mb} MB.",
                name = name,
                mb = MAX_IMAGE_BYTES / (1024 * 1024)
            )));
        }
        if self.ai.draft_images.len() >= MAX_IMAGES {
            return Err(CommandError::Failed(trf!(
                "A question takes at most {n} images.",
                n = MAX_IMAGES
            )));
        }
        self.ai.draft_images.push(Image {
            media_type: media_type.to_string(),
            data: Arc::from(bytes),
            name: name.to_string(),
        });
        Ok(())
    }

    /// Attaches the image file at `path` (see [`Self::ai_attach_image`]).
    pub fn ai_attach_file(&mut self, path: &std::path::Path) -> Result<(), CommandError> {
        let name = path
            .file_name()
            .map_or_else(|| "image".to_string(), |n| n.to_string_lossy().into_owned());
        let meta =
            std::fs::metadata(path).map_err(|e| CommandError::Failed(format!("{name}: {e}")))?;
        if meta.len() > MAX_IMAGE_BYTES as u64 {
            return Err(CommandError::Failed(trf!(
                "{name} is larger than {mb} MB.",
                name = name,
                mb = MAX_IMAGE_BYTES / (1024 * 1024)
            )));
        }
        let bytes =
            std::fs::read(path).map_err(|e| CommandError::Failed(format!("{name}: {e}")))?;
        self.ai_attach_image(&name, bytes)
    }

    /// Reads the AI configuration from `path` (called once by the app).
    pub fn load_ai_config(&mut self, path: PathBuf) {
        // Request bodies a crash or a forced quit left in the temp folder go first.
        texcraft_ai::sweep_stale_request_files();
        self.ai.config = Config::load(&path);
        if let Some(dir) = path.parent() {
            let keys = KeyStore::new(dir);
            keys.mark(&mut self.ai.config);
            self.ai.keys = Some(keys);
        }
        self.ai.config_path = Some(path);
    }

    /// Stores the API key of provider `id` (encrypted; it cannot be read back from the app).
    pub fn ai_set_key(&mut self, id: &str, key: &str) -> Result<(), CommandError> {
        if key.trim().is_empty() {
            return Err(CommandError::Failed(tr("the key is empty").into()));
        }
        let keys = self
            .ai
            .keys
            .as_ref()
            .ok_or_else(|| CommandError::Failed(tr("keys cannot be saved here").into()))?;
        keys.save(id, key).map_err(|e| CommandError::Failed(e.0))?;
        keys.mark(&mut self.ai.config);
        Ok(())
    }

    /// Forgets the API key of provider `id`.
    pub fn ai_remove_key(&mut self, id: &str) -> Result<(), CommandError> {
        if let Some(keys) = &self.ai.keys {
            keys.delete(id).map_err(|e| CommandError::Failed(e.0))?;
            keys.mark(&mut self.ai.config);
        }
        Ok(())
    }

    /// Removes provider `index` and its key.
    pub fn ai_remove_provider(&mut self, index: usize) -> Result<(), CommandError> {
        if index >= self.ai.config.providers.len() {
            return Ok(());
        }
        let p = self.ai.config.providers.remove(index);
        self.ai_remove_key(&p.id)?;
        self.save_ai_config()
    }

    /// The decrypted key of `provider` (empty for local servers without one).
    pub fn ai_key_for(&self, provider: &texcraft_ai::Provider) -> String {
        self.ai
            .keys
            .as_ref()
            .and_then(|k| k.get(&provider.id))
            .unwrap_or_default()
    }

    /// Saves the AI configuration (after edits in Settings).
    pub fn save_ai_config(&mut self) -> Result<(), CommandError> {
        self.ai.config.normalize();
        match &self.ai.config_path {
            Some(p) => self
                .ai
                .config
                .save(p)
                .map_err(|e| CommandError::Failed(e.0)),
            None => Ok(()),
        }
    }

    /// The system prompt with the open document (and the selection) when that is enabled.
    fn ai_system(&self) -> String {
        let mut s = SYSTEM.to_string();
        if !self.ai.config.include_document {
            return s;
        }
        let Some(doc) = self.doc() else { return s };
        s.push_str(&format!(
            "\n\nThe open document is “{}” (each line prefixed with its number):\n<document>\n",
            doc.display_name()
        ));
        for (i, line) in doc.text.lines().enumerate() {
            s.push_str(&format!("{:>4}  {line}\n", i + 1));
        }
        s.push_str("</document>");
        let sel = doc.selection;
        if !sel.is_empty() {
            let (a, b) = (
                edit::byte_of(&doc.text, sel.start()),
                edit::byte_of(&doc.text, sel.end()),
            );
            if let Some(text) = doc.text.get(a..b) {
                s.push_str(&format!(
                    "\n\nThe user has selected lines {}–{}:\n<selection>\n{text}\n</selection>",
                    edit::line_of_char(&doc.text, sel.start()),
                    edit::line_of_char(&doc.text, sel.end())
                ));
            }
        }
        s
    }

    /// Asks the chosen model `text` (the conversation so far goes along).
    pub fn ai_send(&mut self, text: &str) -> Result<(), CommandError> {
        let text = text.trim();
        if text.is_empty() && self.ai.draft_images.is_empty() {
            return Err(CommandError::Failed(tr("Type a question first.").into()));
        }
        if self.ai.job.is_some() {
            return Err(CommandError::Failed(
                tr("Wait for the answer, or stop it first.").into(),
            ));
        }
        let (provider, model, api_key) = self.ai_target()?;
        let images = std::mem::take(&mut self.ai.draft_images);
        if !images.is_empty() && !self.ai_reads_images() {
            let label = self
                .ai_model()
                .map_or_else(|| model.clone(), |m| m.label().to_string());
            self.ai.draft_images = images;
            return Err(CommandError::Failed(trf!(
                "{model} does not read images: pick a model with Vision, or remove the images.",
                model = label
            )));
        }
        self.ai
            .messages
            .push(ChatMessage::user(text.to_string(), images));
        // The conversation: answered questions only (failed or declined ones are left out).
        let mut turns = Vec::new();
        let msgs = &self.ai.messages;
        let mut i = 0;
        while i < msgs.len() {
            let Some(m) = msgs.get(i) else { break };
            if m.role == Role::User {
                let answer = msgs.get(i + 1).filter(|a| a.role == Role::Assistant);
                let last = i + 1 == msgs.len();
                match answer {
                    Some(a) if a.error.is_none() && !a.pending => {
                        turns.push(Turn {
                            role: Role::User,
                            text: m.text.clone(),
                            blocks: Vec::new(),
                            images: m.images.clone(),
                            ..Default::default()
                        });
                        turns.push(Turn {
                            role: Role::Assistant,
                            text: a.text.clone(),
                            // Blocks only go back to the model that wrote them.
                            blocks: if a.model.as_deref() == Some(model.as_str()) {
                                a.blocks.clone()
                            } else {
                                Vec::new()
                            },
                            images: Vec::new(),
                            ..Default::default()
                        });
                        i += 2;
                        continue;
                    }
                    _ if last => turns.push(Turn {
                        role: Role::User,
                        text: m.text.clone(),
                        blocks: Vec::new(),
                        images: m.images.clone(),
                        ..Default::default()
                    }),
                    _ => {}
                }
            }
            i += 1;
        }
        let provider_id = provider.id.clone();
        // With a project folder the model may read its files (never change them).
        let plain_system = self.ai_system();
        let tools = self
            .ai_project_root()
            .map_or_else(Vec::new, |_| ai_tools::specs());
        let system = if tools.is_empty() {
            plain_system.clone()
        } else {
            format!("{plain_system}{}", ai_tools::PROMPT)
        };
        let req = ChatRequest {
            provider,
            model: model.clone(),
            system,
            api_key,
            turns,
            max_tokens: None,
            no_reasoning: false,
            tools,
        };
        self.ai.messages.push(ChatMessage {
            images: Vec::new(),
            role: Role::Assistant,
            text: String::new(),
            blocks: Vec::new(),
            model: Some(model),
            provider: Some(provider_id),
            pending: true,
            thinking: false,
            error: None,
            note: None,
            tools: Vec::new(),
        });
        self.ai.job = Some(texcraft_ai::start(&req, Arc::clone(&self.waker)));
        self.ai.round = Some(Round {
            req,
            n: 0,
            text: String::new(),
            plain_system,
        });
        self.ai_save_chat();
        Ok(())
    }

    /// The model called tools: run them (they only read), and ask again with their answers.
    fn ai_tool_round(&mut self, done: Event) {
        let Event::Done {
            blocks,
            model,
            tool_calls,
            reasoning,
        } = done
        else {
            return;
        };
        let Some(mut round) = self.ai.round.take() else {
            return;
        };
        if round.n >= MAX_TOOL_ROUNDS {
            self.ai.job = None;
            if let Some(m) = self.ai.messages.last_mut() {
                m.pending = false;
                m.thinking = false;
                m.note = Some(trf!(
                    "Stopped after {n} rounds of reading files",
                    n = round.n
                ));
                if m.text.trim().is_empty() {
                    m.error = Some(tr("The model kept reading files without answering.").into());
                }
            }
            return;
        }
        let root = self.ai_project_root();
        let mut results = Vec::new();
        let mut uses = Vec::new();
        for call in &tool_calls {
            let (content, used) = match &root {
                Some(r) => self.run_ai_tool(r, call),
                None => (
                    "Error: no project folder is open.".to_string(),
                    ToolUse {
                        kind: ai_tools::ToolKind::Read,
                        target: call.name.clone(),
                        lines: None,
                        failed: true,
                    },
                ),
            };
            results.push(ToolResult {
                id: call.id.clone(),
                name: call.name.clone(),
                content,
            });
            uses.push(used);
        }
        round.req.turns.push(Turn {
            role: Role::Assistant,
            text: std::mem::take(&mut round.text),
            blocks,
            tool_calls,
            reasoning,
            ..Default::default()
        });
        round.req.turns.push(Turn {
            role: Role::User,
            tool_results: results,
            ..Default::default()
        });
        round.n += 1;
        self.ai.job = Some(texcraft_ai::start(&round.req, Arc::clone(&self.waker)));
        self.ai.round = Some(round);
        if let Some(m) = self.ai.messages.last_mut() {
            m.tools.extend(uses);
            m.thinking = false;
            if let Some(model) = model {
                m.model = Some(model);
            }
            // The next round's text starts a paragraph of its own.
            if !m.text.trim().is_empty() && !m.text.ends_with("\n\n") {
                m.text.push_str("\n\n");
            }
        }
    }

    /// The Assistant's provider, model and API key, or why there are none.
    pub(crate) fn ai_target(
        &self,
    ) -> Result<(texcraft_ai::Provider, String, String), CommandError> {
        let (provider, model) = self
            .ai
            .config
            .selection()
            .map(|(p, m)| (p.clone(), m.to_string()))
            .ok_or_else(|| {
                CommandError::Failed(
                    tr("Add a provider and a model in Settings → AI first.").into(),
                )
            })?;
        let api_key = self.ai_key_for(&provider);
        if api_key.is_empty() && !texcraft_ai::is_local(&provider) {
            return Err(CommandError::Failed(trf!(
                "Enter the API key of {provider} in Settings → AI.",
                provider = provider.name
            )));
        }
        Ok((provider, model, api_key))
    }

    /// Keeps conversations in `root` (TexCraft's data folder's `chats`, never the project), and
    /// removes those of projects that no longer exist.
    pub fn set_chat_store(&mut self, root: PathBuf) {
        let store = ChatStore::new(root);
        store.sweep();
        self.ai.chats = Some(store);
        self.ai.chat_seen = None;
        self.ai_follow_project();
    }

    /// Conversations belong to a project: when another one opens, the conversation is saved
    /// and the Assistant starts afresh with that project's history (once an answer arriving
    /// has finished).
    pub(crate) fn ai_follow_project(&mut self) {
        if self.ai.chats.is_none() || self.ai.job.is_some() {
            return;
        }
        let seen = self.git_dir();
        if self.ai.chat_seen.as_ref() == Some(&seen) {
            return;
        }
        let first = self.ai.chat_seen.is_none();
        let project = seen.as_ref().and_then(|p| p.canonicalize().ok());
        self.ai.chat_seen = Some(seen);
        if !first && project == self.ai.chat_project {
            return;
        }
        if !first {
            self.ai_save_chat();
            self.ai.messages.clear();
            self.ai.chat_id = None;
        }
        self.ai.chat_project = project;
        self.ai_refresh_history();
    }

    fn ai_refresh_history(&mut self) {
        self.ai.history = self
            .ai
            .chats
            .as_ref()
            .map(|s| s.list(self.ai.chat_project.as_deref()))
            .unwrap_or_default();
    }

    /// Saves the conversation shown (once it has a question).
    fn ai_save_chat(&mut self) {
        let Some(store) = &self.ai.chats else {
            return;
        };
        if !self.ai.messages.iter().any(|m| m.role == Role::User) {
            return;
        }
        let id = self.ai.chat_id.get_or_insert_with(chats::new_id).clone();
        let saved = store.save(self.ai.chat_project.as_deref(), &id, &self.ai.messages);
        if let Err(e) = saved {
            self.set_status(e);
        }
        self.ai_refresh_history();
    }

    /// Starts a new conversation (the one shown stays in the history).
    pub fn ai_new_chat(&mut self) {
        self.ai_stop();
        self.ai.messages.clear();
        self.ai.chat_id = None;
    }

    /// Shows saved conversation `id` of the current project.
    pub fn ai_open_chat(&mut self, id: &str) -> Result<(), CommandError> {
        self.ai_stop();
        let messages = self
            .ai
            .chats
            .as_ref()
            .and_then(|s| s.load(self.ai.chat_project.as_deref(), id))
            .ok_or_else(|| CommandError::Failed(tr("No such conversation.").into()))?;
        self.ai.messages = messages;
        self.ai.chat_id = Some(id.to_string());
        Ok(())
    }

    /// Deletes saved conversation `id` (shown, the Assistant starts afresh).
    pub fn ai_delete_chat(&mut self, id: &str) -> Result<(), CommandError> {
        if let Some(store) = &self.ai.chats {
            store
                .delete(self.ai.chat_project.as_deref(), id)
                .map_err(CommandError::Failed)?;
        }
        if self.ai.chat_id.as_deref() == Some(id) {
            self.ai.round = None;
            if let Some(job) = self.ai.job.take() {
                job.cancel();
            }
            self.ai.messages.clear();
            self.ai.chat_id = None;
        }
        self.ai_refresh_history();
        Ok(())
    }

    /// Stops the answer that is arriving (what arrived so far stays).
    pub fn ai_stop(&mut self) {
        self.ai.round = None;
        if let Some(job) = self.ai.job.take() {
            job.cancel();
        }
        if let Some(m) = self.ai.messages.last_mut()
            && m.pending
        {
            m.pending = false;
            m.thinking = false;
            if m.text.is_empty() {
                m.error = Some(tr("Stopped.").into());
            } else {
                m.note = Some(tr("Stopped").into());
                m.blocks.clear();
            }
        }
        self.ai_save_chat();
    }

    /// The LaTeX of answer `index`: its first `latex`/`tex` code block, else its first code
    /// block, else the whole text.
    pub fn ai_snippet(&self, index: usize) -> Option<String> {
        let m = self.ai.messages.get(index)?;
        let mut blocks: Vec<(String, String)> = Vec::new();
        let mut current: Option<(String, String)> = None;
        for line in m.text.lines() {
            let trimmed = line.trim_start();
            if let Some(info) = trimmed.strip_prefix("```") {
                match current.take() {
                    Some(b) => blocks.push(b),
                    None => current = Some((info.trim().to_lowercase(), String::new())),
                }
            } else if let Some((_, body)) = current.as_mut() {
                body.push_str(line);
                body.push('\n');
            }
        }
        let pick = blocks
            .iter()
            .find(|(lang, _)| lang == "latex" || lang == "tex")
            .or_else(|| blocks.first())
            .map(|(_, body)| body.trim_end_matches('\n').to_string());
        pick.or_else(|| Some(m.text.trim().to_string()))
            .filter(|s| !s.is_empty())
    }

    /// Polls the running request.
    pub(crate) fn tick_ai(&mut self) -> Option<std::time::Duration> {
        self.ai_follow_project();
        let events: Vec<Event> = self
            .ai
            .job
            .as_ref()
            .map(|j| std::iter::from_fn(|| j.try_recv()).collect())
            .unwrap_or_default();
        let mut finished = false;
        for e in events {
            if matches!(&e, Event::Done { tool_calls, .. } if !tool_calls.is_empty()) {
                self.ai_tool_round(e);
                continue;
            }
            if let Event::Failed(msg) = &e
                && self.ai_retry_without_tools(msg)
            {
                continue;
            }
            let Some(m) = self.ai.messages.last_mut() else {
                break;
            };
            match e {
                Event::Text(t) => {
                    m.thinking = false;
                    m.text.push_str(&t);
                    if let Some(r) = self.ai.round.as_mut() {
                        r.text.push_str(&t);
                    }
                }
                Event::Thinking => m.thinking = m.text.is_empty(),
                Event::Fallback(what) => {
                    m.note = Some(trf!("Declined, answered by {model}", model = what));
                }
                Event::Done { blocks, model, .. } => {
                    m.pending = false;
                    m.thinking = false;
                    m.blocks = blocks;
                    if let Some(model) = model {
                        m.model = Some(model);
                    }
                    if m.text.trim().is_empty() {
                        m.error = Some(tr("The model sent an empty answer.").into());
                    }
                    self.ai.job = None;
                    self.ai.round = None;
                    finished = true;
                }
                Event::Refused(category) => {
                    // A declined answer is discarded, partial text included.
                    m.pending = false;
                    m.thinking = false;
                    m.text.clear();
                    m.blocks.clear();
                    m.error = Some(match category {
                        Some(c) => trf!("The model declined this request ({reason}).", reason = c),
                        None => tr("The model declined this request.").into(),
                    });
                    self.ai.job = None;
                    self.ai.round = None;
                    finished = true;
                }
                Event::Failed(msg) => {
                    m.pending = false;
                    m.thinking = false;
                    m.error = Some(msg);
                    self.ai.job = None;
                    self.ai.round = None;
                    finished = true;
                }
            }
        }
        if finished {
            self.ai_save_chat();
        }
        self.ai
            .job
            .is_some()
            .then_some(std::time::Duration::from_millis(100))
    }

    /// A model that takes no tools refuses the first request: ask again without them (and
    /// without telling it about them). Returns whether it did.
    fn ai_retry_without_tools(&mut self, msg: &str) -> bool {
        let Some(round) = self.ai.round.as_mut() else {
            return false;
        };
        let answered = self
            .ai
            .messages
            .last()
            .is_some_and(|m| !m.text.trim().is_empty());
        if round.n > 0
            || round.req.tools.is_empty()
            || answered
            || !msg.to_ascii_lowercase().contains("tool")
        {
            return false;
        }
        round.req.tools.clear();
        round.req.system = round.plain_system.clone();
        self.ai.job = Some(texcraft_ai::start(&round.req, Arc::clone(&self.waker)));
        true
    }

    /// Inserts answer `index`'s LaTeX at the caret (replacing the selection).
    pub fn ai_insert(&mut self, index: usize) -> Result<(), CommandError> {
        let text = self
            .ai_snippet(index)
            .ok_or_else(|| CommandError::Failed(tr("nothing to insert").into()))?;
        self.edit_active(|t, sel| edit::replace_selection(t, sel, &text))?;
        if let Some(line) = self.doc().map(|d| d.caret_line()) {
            self.effect(Effect::RevealLine {
                doc: self.active,
                line,
            });
        }
        Ok(())
    }
}
