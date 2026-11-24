//! The AI Assistant (the sidebar's sparkles tab): a conversation with a model from the
//! providers configured in Settings → AI, optionally about the open document.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use texcraft_ai::{
    ChatRequest, Config, Event, Image, Job, KeyStore, Model, Role, Tool, ToolResult, Turn, Usage,
};
use texcraft_i18n::{tr, trf};

use crate::ai_tools::{self, ToolUse};
use crate::ai_usage::{ChatUsage, ContextUsage, PromptSample, RunUsage};
use crate::chats::{self, ChatStore, ChatSummary};
use crate::{CommandError, Effect, Session, edit};

/// When `path` last changed.
fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// One message of the conversation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MessageSource {
    pub project: Option<PathBuf>,
    pub file: Option<PathBuf>,
    #[serde(default)]
    pub root: Option<PathBuf>,
}

/// One message of the conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    /// The question's original project and file; retained with saved conversations.
    pub source: Option<MessageSource>,
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
    fn user(text: String, images: Vec<Image>, source: MessageSource) -> Self {
        Self {
            source: Some(source),
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
    /// The configuration as this window last read or wrote it: what it changed since then is
    /// its own ([`crate::prefs::merge3`]); the rest follows the file (other windows).
    pub config_base: Option<serde_json::Value>,
    /// When the file was last read or written here.
    pub config_seen: Option<std::time::SystemTime>,
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
    /// Opt-in local/iCloud storage and its background copy.
    pub storage: Option<crate::chat_storage::ChatStorage>,
    /// The conversation shown (`None`: new, not saved yet).
    pub chat_id: Option<String>,
    /// The current project's saved conversations, the latest first.
    pub history: Vec<ChatSummary>,
    /// The right-click AI action running or showing its result (the editor's result card).
    pub action: Option<crate::ai_actions::ActionRun>,
    /// The project folder the conversation belongs to (canonical; `None`: none).
    chat_project: Option<PathBuf>,
    /// The folder last seen (`None`: not looked yet), to notice another project opening.
    chat_seen: Option<(Option<PathBuf>, Option<PathBuf>)>,
    /// The next change of project is the same work under a new name (a document saved into a
    /// folder, a project that gained its root): the conversation goes along.
    carry: Option<Carry>,
    /// What the conversation used (its answers' requests), saved with it.
    pub usage: ChatUsage,
    /// The last context measured, and what it depended on (see `Session::ai_context`).
    pub(crate) context_cache: Option<(u64, Option<ContextUsage>)>,
}

/// What a carried conversation's sources pointed at before the change (see `AiState::carry`).
struct Carry {
    project: Option<PathBuf>,
    root: Option<PathBuf>,
    /// The saved document's old file (an untitled one's scratch file) and its new one.
    file: Option<(PathBuf, PathBuf)>,
}

/// An answer in rounds: the model calls tools, gets their answers and goes on.
struct Round {
    /// The directory this question was sent from; tool rounds never consult another project.
    project: Option<PathBuf>,
    req: ChatRequest,
    /// Rounds of tool calls so far.
    n: u32,
    /// The text of this round (it goes back with the round's calls).
    text: String,
    /// The system prompt without the tools (for asking again without them).
    plain_system: String,
    /// The answer's entry in `usage.runs`.
    run: usize,
    /// When the request in flight was sent.
    started: Instant,
    /// Its estimated size (to calibrate the context meter with what the provider counts).
    estimate: u64,
}

impl Round {
    fn new(req: ChatRequest, plain_system: String, run: usize, project: Option<PathBuf>) -> Self {
        let estimate = texcraft_ai::usage::estimate(&req).total();
        Self {
            project,
            req,
            n: 0,
            text: String::new(),
            plain_system,
            run,
            started: Instant::now(),
            estimate,
        }
    }

    /// The request was sent again (grown, or changed): its time and size start afresh.
    fn restart(&mut self) {
        self.started = Instant::now();
        self.estimate = texcraft_ai::usage::estimate(&self.req).total();
    }
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

    /// The request being answered.
    pub(crate) fn round_request(&self) -> Option<&ChatRequest> {
        self.job.as_ref().and(self.round.as_ref()).map(|r| &r.req)
    }

    /// The request in flight ended (`tool_calls`: the tools it called that will run): one more
    /// step, and its time with the model when it ran to the end (`timed`).
    fn end_request(&mut self, tool_calls: usize, timed: bool) {
        let Some(round) = self.round.as_ref() else {
            return;
        };
        let ms = u64::try_from(round.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        if let Some(run) = self.usage.runs.get_mut(round.run) {
            run.calls = run.calls.saturating_add(1);
            run.tool_calls = run
                .tool_calls
                .saturating_add(u32::try_from(tool_calls).unwrap_or(u32::MAX));
            if timed {
                run.model_ms = run.model_ms.saturating_add(ms);
            }
        }
    }

    /// The provider reported what the request in flight used.
    fn record_usage(&mut self, u: &Usage) {
        let Some(round) = self.round.as_ref() else {
            return;
        };
        if let Some(run) = self.usage.runs.get_mut(round.run) {
            run.add(u);
        }
        if u.input > 0 {
            self.usage.sample = Some(PromptSample {
                provider: round.req.provider.id.clone(),
                model: round.req.model.clone(),
                estimated: round.estimate,
                input: u.input,
            });
        }
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
    pub fn ai_source(&self) -> MessageSource {
        MessageSource {
            project: self.ai_project_root(),
            root: self.project_root.clone(),
            file: self
                .doc()
                .and_then(|doc| doc.typeset_path())
                .map(std::path::Path::to_path_buf),
        }
    }

    /// Reject inconsistent/legacy mixed sessions before any document text can reach a model.
    pub(crate) fn ai_validate_context(&self) -> Result<(), CommandError> {
        let Some(path) = self.doc().and_then(|doc| doc.path.as_ref()) else {
            return Ok(());
        };
        let Some(directory) = self.ai_project_root() else {
            return Ok(());
        };
        let entry = texcraft_project::project_entry(path, &|path| self.text_of(path));
        let included = self.project_root.as_ref().is_some_and(|root| {
            let index = crate::index::build(root, &|path| self.text_of(path));
            index.files.iter().any(|file| file.path == *path)
                || index.assets.iter().any(|file| file.path == *path)
        });
        let different_root = self
            .project_root
            .as_ref()
            .is_some_and(|root| *root != entry)
            && self
                .text_of(&entry)
                .is_some_and(|text| texcraft_syntax::is_root_document(&text));
        let same_root = self.project_root.as_ref() == Some(&entry);
        if !included && !same_root && (!path.starts_with(directory) || different_root) {
            return Err(CommandError::Failed(tr("The active file does not belong to this project. Open it in its own project window before asking AI.").into()));
        }
        Ok(())
    }

    fn ai_context_path(&self, path: &std::path::Path) -> String {
        let path = texcraft_project::normalize(path);
        self.ai_project_root()
            .and_then(|root| {
                path.strip_prefix(root)
                    .ok()
                    .map(std::path::Path::to_path_buf)
            })
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }

    /// The current tab's project-relative path, or its untitled name. Never a scratch path.
    pub fn ai_active_file(&self) -> Option<String> {
        self.doc().map(|doc| {
            doc.path
                .as_ref()
                .map_or_else(|| doc.display_name(), |path| self.ai_context_path(path))
        })
    }

    /// Editor identity captured afresh for each question, independent of content inclusion.
    pub fn ai_editor_context(&self) -> serde_json::Value {
        let doc = self.doc();
        serde_json::json!({
            "project": self.ai_project_root().and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned())),
            "file": self.ai_active_file(),
            "rootFile": doc.filter(|doc| doc.path.is_some()).and_then(|_| self.project_root.clone().or_else(|| self.root_path())).map(|path| self.ai_context_path(&path)),
            "modified": doc.is_some_and(|doc| doc.is_dirty()),
            "cursor": doc.map(|doc| serde_json::json!({
                "line": doc.caret_line(),
                "column": edit::column_of_char(&doc.text, doc.selection.cursor),
            })),
            "selection": doc.filter(|doc| !doc.selection.is_empty()).map(|doc| serde_json::json!({
                "startLine": edit::line_of_char(&doc.text, doc.selection.start()),
                "endLine": edit::line_of_char(&doc.text, doc.selection.end()),
            })),
            "contentsIncluded": doc.is_some() && self.ai.config.include_document,
        })
    }

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
        self.ai.config_base = serde_json::to_value(&self.ai.config).ok();
        self.ai.config_seen = modified(&path);
        self.ai.config_path = Some(path);
    }

    /// The AI configuration as the file holds it now, merged with this window's own changes
    /// (`None`: the file cannot be read; this window's then stands).
    fn ai_config_merged(&self) -> Option<(Config, serde_json::Value)> {
        let path = self.ai.config_path.as_ref()?;
        let disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        let ours = serde_json::to_value(&self.ai.config).ok()?;
        let base = self.ai.config_base.as_ref().unwrap_or(&ours);
        let merged = crate::prefs::merge3(base, &ours, &disk);
        let mut config: Config = serde_json::from_value(merged).ok()?;
        config.normalize();
        if let Some(keys) = &self.ai.keys {
            keys.mark(&mut config);
        }
        Some((config, disk))
    }

    /// Another window changed the AI configuration: take its changes over (this window's own
    /// unsaved changes stay). Cheap when nothing changed (one file date).
    pub fn ai_follow_config_file(&mut self) {
        let seen = self.ai.config_path.as_deref().and_then(modified);
        if seen.is_some()
            && seen != self.ai.config_seen
            && let Some((config, disk)) = self.ai_config_merged()
        {
            self.ai.config = config;
            self.ai.config_base = Some(disk);
            self.ai.config_seen = seen;
        }
        self.ai_follow_chat_storage();
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
        // Other windows' changes since this one last read the file stay.
        if let Some((config, _)) = self.ai_config_merged() {
            self.ai.config = config;
        }
        if let Some(p) = self.ai.config_path.clone() {
            self.ai
                .config
                .save(&p)
                .map_err(|e| CommandError::Failed(e.0))?;
            self.ai.config_base = serde_json::to_value(&self.ai.config).ok();
            self.ai.config_seen = modified(&p);
        }
        self.ai_follow_chat_storage();
        Ok(())
    }

    /// The system prompt with the open document (and the selection) when that is enabled.
    fn ai_system(&self) -> String {
        let mut s = SYSTEM.to_string();
        s.push_str(&format!(
            "\n\nCurrent editor context for this question (authoritative; earlier turns may refer \
             to other tabs). Use this file when the user says 'this file' or 'the current file':\n\
             <editor_context>\n{}\n</editor_context>",
            self.ai_editor_context()
        ));
        if !self.ai.config.include_document {
            return s;
        }
        let Some(doc) = self.doc() else { return s };
        s.push_str(&format!(
            "\n\nThe open document is “{}” (each line prefixed with its number):\n<document>\n",
            self.ai_active_file().unwrap_or_else(|| doc.display_name())
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
        self.ai_follow_project();
        self.ai_validate_context()?;
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
        let source = self.ai_source();
        self.ai
            .messages
            .push(ChatMessage::user(text.to_string(), images, source.clone()));
        let turns = self.ai_turns(&model);
        let provider_id = provider.id.clone();
        let (system, tools, plain_system) = self.ai_prompt();
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
            source: Some(source),
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
        self.ai.usage.runs.push(RunUsage {
            message: self.ai.messages.len().saturating_sub(1),
            ..RunUsage::default()
        });
        let run = self.ai.usage.runs.len().saturating_sub(1);
        self.ai.job = Some(texcraft_ai::start(&req, Arc::clone(&self.waker)));
        self.ai.round = Some(Round::new(req, plain_system, run, self.ai_project_root()));
        self.ai_save_chat();
        Ok(())
    }

    /// The system prompt, the tools and the system prompt without them: with a project folder
    /// the model may read its files (never change them).
    pub(crate) fn ai_prompt(&self) -> (String, Vec<Tool>, String) {
        let plain_system = self.ai_system();
        let tools = self
            .ai_project_root()
            .map_or_else(Vec::new, |_| ai_tools::specs());
        let system = if tools.is_empty() {
            plain_system.clone()
        } else {
            format!("{plain_system}{}", ai_tools::PROMPT)
        };
        (system, tools, plain_system)
    }

    /// The conversation to send to `model`: answered questions only (failed or declined ones are
    /// left out), and the last question when it is not answered yet.
    pub(crate) fn ai_turns(&self, model: &str) -> Vec<Turn> {
        let question = |message: &ChatMessage| match message
            .source
            .as_ref()
            .and_then(|source| source.file.as_ref().map(|file| (source, file)))
        {
            Some((source, file)) => {
                let file = if let Some(root) = &source.project {
                    file.strip_prefix(root).unwrap_or(file)
                } else {
                    file.file_name().map(std::path::Path::new).unwrap_or(file)
                };
                format!(
                    "Editor file for this question: {}\n\n{}",
                    file.display(),
                    message.text
                )
            }
            None => message.text.clone(),
        };
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
                            text: question(m),
                            blocks: Vec::new(),
                            images: m.images.clone(),
                            ..Default::default()
                        });
                        turns.push(Turn {
                            role: Role::Assistant,
                            text: a.text.clone(),
                            // Blocks only go back to the model that wrote them.
                            blocks: if a.model.as_deref() == Some(model) {
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
                        text: question(m),
                        blocks: Vec::new(),
                        images: m.images.clone(),
                        ..Default::default()
                    }),
                    _ => {}
                }
            }
            i += 1;
        }
        turns
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
        let runs_tools = self
            .ai
            .round
            .as_ref()
            .is_some_and(|r| r.n < MAX_TOOL_ROUNDS);
        self.ai
            .end_request(if runs_tools { tool_calls.len() } else { 0 }, true);
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
        let root = round.project.clone();
        if root != self.ai_project_root() {
            self.ai_stop();
            return;
        }
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
        round.restart();
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

    /// Keeps conversations in `root` (never the project).
    pub fn set_chat_store(&mut self, root: PathBuf) {
        self.set_chats(ChatStore::new(root));
    }

    /// Keeps conversations in `store` (the app's: [`ChatStore::locate`]). Those of projects
    /// missing here stay: the project may be on another Mac or a drive not connected now.
    pub fn set_chats(&mut self, store: ChatStore) {
        self.ai.chats = Some(store);
        self.ai.chat_seen = None;
        self.ai_follow_project();
    }

    /// Conversations belong to a project: when another one opens, the conversation is saved
    /// and the Assistant starts afresh with that project's history. An old answer is stopped
    /// before it can run tools against the new directory.
    pub(crate) fn ai_follow_project(&mut self) {
        let carry = self.ai.carry.take();
        let seen = (self.git_dir(), self.project_root.clone());
        if self.ai.chat_seen.as_ref() == Some(&seen) {
            return;
        }
        let first = self.ai.chat_seen.is_none();
        let project = seen
            .1
            .as_ref()
            .or(seen.0.as_ref())
            .and_then(|path| path.canonicalize().ok());
        self.ai.chat_seen = Some(seen);
        if !first && project == self.ai.chat_project {
            return;
        }
        if !first {
            match carry {
                Some(carry) => self.ai_carry_conversation(&carry, project.as_deref()),
                None => self.ai_start_afresh(),
            }
        }
        self.ai.chat_project = project;
        self.ai_refresh_history();
    }

    /// The project's identity is about to change, but the work is the same: an untitled
    /// document saved into a folder (`file`: its scratch file and new path), or a project that
    /// just gained its root. The conversation shown, its draft and its history entry go along,
    /// as an editor keeps a document's chat when the document is saved.
    pub(crate) fn ai_carry_next(&mut self, file: Option<(PathBuf, PathBuf)>) {
        let source = self.ai_source();
        self.ai.carry = Some(Carry {
            project: source.project,
            root: source.root,
            file,
        });
    }

    /// Moves the conversation shown to project `to`: its answers' sources follow the change,
    /// and its saved file moves to that project's history.
    fn ai_carry_conversation(&mut self, carry: &Carry, to: Option<&Path>) {
        let now = self.ai_source();
        for m in &mut self.ai.messages {
            let Some(source) = &mut m.source else {
                continue;
            };
            if source.project == carry.project {
                source.project.clone_from(&now.project);
            }
            if source.root == carry.root {
                source.root.clone_from(&now.root);
            }
            if let Some((from, into)) = &carry.file
                && source.file.as_deref() == Some(from.as_path())
            {
                source.file = Some(into.clone());
            }
        }
        let from = self.ai.chat_project.clone();
        if let (Some(store), Some(id)) = (&self.ai.chats, &self.ai.chat_id)
            && from.as_deref() != to
            && store
                .save(to, id, &self.ai.messages, &self.ai.usage)
                .is_ok()
        {
            let _ = store.delete(from.as_deref(), id);
        }
    }

    /// Empties the Assistant: no conversation, draft or attachments (the conversation shown is
    /// already saved).
    pub(crate) fn ai_start_afresh(&mut self) {
        self.ai_stop();
        self.ai_action_dismiss();
        self.inline_after_caret_move();
        self.ai.messages.clear();
        self.ai.usage = ChatUsage::default();
        self.ai.chat_id = None;
        self.ai.draft.clear();
        self.ai.draft_images.clear();
        self.ai.notice = None;
        self.ai.context_cache = None;
    }

    /// `from` was renamed to `to`. The answers shown follow their files, and when the
    /// conversation's project is what moved, it carries on (with its draft) under the new name
    /// instead of starting afresh: it is the same project.
    pub(crate) fn ai_follow_rename(&mut self, from: &Path, to: &Path) {
        for m in &mut self.ai.messages {
            if let Some(source) = &mut m.source {
                for path in [&mut source.project, &mut source.file, &mut source.root]
                    .into_iter()
                    .flatten()
                {
                    if let Some(moved) = crate::files::rebase(path, from, to) {
                        *path = moved;
                    }
                }
            }
        }
        if let Some(moved) = self
            .ai
            .chat_project
            .as_deref()
            .and_then(|project| crate::files::rebase(project, from, to))
        {
            self.ai.chat_project = Some(moved);
        }
    }

    pub(crate) fn ai_refresh_history(&mut self) {
        self.ai.history = self
            .ai
            .chats
            .as_ref()
            .map(|s| s.list(self.ai.chat_project.as_deref()))
            .unwrap_or_default();
    }

    /// Saves the conversation shown (once it has a question).
    pub(crate) fn ai_save_chat(&mut self) {
        let Some(store) = &self.ai.chats else {
            return;
        };
        if !self.ai.messages.iter().any(|m| m.role == Role::User) {
            return;
        }
        let id = self.ai.chat_id.get_or_insert_with(chats::new_id).clone();
        let saved = store.save(
            self.ai.chat_project.as_deref(),
            &id,
            &self.ai.messages,
            &self.ai.usage,
        );
        if let Err(e) = saved {
            self.set_status(e);
        }
        self.ai_refresh_history();
    }

    /// Starts a new conversation (the one shown stays in the history).
    pub fn ai_new_chat(&mut self) {
        self.ai_follow_project();
        self.ai_stop();
        self.ai.messages.clear();
        self.ai.usage = ChatUsage::default();
        self.ai.chat_id = None;
    }

    /// Shows saved conversation `id` of the current project.
    pub fn ai_open_chat(&mut self, id: &str) -> Result<(), CommandError> {
        self.ai_follow_project();
        self.ai_stop();
        let (messages, usage) = self
            .ai
            .chats
            .as_ref()
            .and_then(|s| s.load(self.ai.chat_project.as_deref(), id))
            .ok_or_else(|| CommandError::Failed(tr("No such conversation.").into()))?;
        self.ai.messages = messages;
        self.ai.usage = usage;
        self.ai.chat_id = Some(id.to_string());
        Ok(())
    }

    /// Deletes saved conversation `id` (shown, the Assistant starts afresh).
    pub fn ai_delete_chat(&mut self, id: &str) -> Result<(), CommandError> {
        self.ai_follow_project();
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
            self.ai.usage = ChatUsage::default();
            self.ai.chat_id = None;
        }
        self.ai_refresh_history();
        Ok(())
    }

    /// Stops the answer that is arriving (what arrived so far stays).
    pub fn ai_stop(&mut self) {
        // A step without a measured end: no time (its output is not known).
        if self.ai.job.is_some() {
            self.ai.end_request(0, false);
        }
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
            if let Event::Usage(u) = &e {
                self.ai.record_usage(u);
                continue;
            }
            if matches!(&e, Event::Done { tool_calls, .. } if !tool_calls.is_empty()) {
                self.ai_tool_round(e);
                continue;
            }
            if let Event::Failed(msg) = &e
                && self.ai_retry_without_tools(msg)
            {
                continue;
            }
            if matches!(e, Event::Done { .. } | Event::Refused(_) | Event::Failed(_)) {
                self.ai.end_request(0, true);
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
                Event::Usage(_) => {}
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
        self.ai.end_request(0, true);
        let Some(round) = self.ai.round.as_mut() else {
            return false;
        };
        round.req.tools.clear();
        round.req.system = round.plain_system.clone();
        round.restart();
        self.ai.job = Some(texcraft_ai::start(&round.req, Arc::clone(&self.waker)));
        true
    }

    /// Inserts answer `index`'s LaTeX at the caret (replacing the selection).
    pub fn ai_insert(&mut self, index: usize) -> Result<(), CommandError> {
        self.ai_follow_project();
        let text = self
            .ai_snippet(index)
            .ok_or_else(|| CommandError::Failed(tr("nothing to insert").into()))?;
        let source = self.ai.messages.get(index).and_then(|message| message.source.as_ref())
            .ok_or_else(|| CommandError::Failed(tr("No file context was saved for that answer. Copy its text to insert it manually.").into()))?;
        if *source != self.ai_source() {
            return Err(CommandError::Failed(
                tr("That answer belongs to another file. Open its file before inserting it.")
                    .into(),
            ));
        }
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

#[cfg(test)]
mod editor_context_tests {
    use super::*;
    use serde_json::{Value, json};
    use std::time::Duration;

    #[test]
    fn successive_questions_follow_the_active_tab_even_when_file_names_match() {
        let base = std::env::temp_dir().join(format!("texcraft-ai-editor-{}", std::process::id()));
        for folder in ["sections", "appendix"] {
            std::fs::create_dir_all(base.join(folder)).unwrap();
            std::fs::write(
                base.join(folder).join("notes.tex"),
                "% !TEX root = ../main.tex\nSaved text.\n",
            )
            .unwrap();
        }
        std::fs::write(base.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/notes}\n\\input{appendix/notes}\n\\end{document}\n").unwrap();
        let mut session = Session::empty(Arc::new(|| {}));
        session.settings.live_typeset = false;
        session.update.enabled = false;
        let answers = (0..3)
            .map(|_| crate::ai_tools::tests::sse(json!({"content": "Verified."}), Some("stop")))
            .collect();
        let (port, server) = crate::ai_tools::tests::fake_model(answers);
        let preset = texcraft_ai::PRESETS
            .iter()
            .find(|preset| preset.id == "ollama")
            .unwrap();
        let provider = session.ai.config.add(preset);
        session.ai.config.providers[provider].base_url = format!("http://127.0.0.1:{port}/v1");
        session.ai.config.providers[provider].models = vec!["local-model".into()];
        session.ai.config.inline.enabled = false;
        session.ai.config.normalize();

        for (folder, marker) in [
            ("sections", "UNSAVED_ACTIVE_A"),
            ("appendix", "UNSAVED_ACTIVE_B"),
        ] {
            session
                .open_path(&base.join(folder).join("notes.tex"))
                .unwrap();
            let text = format!("% !TEX root = ../main.tex\n中文 {marker}\n");
            session.run("edit.setText", &json!({"text": text})).unwrap();
            let start = "% !TEX root = ../main.tex\n".chars().count();
            session
                .run(
                    "edit.select",
                    &json!({"anchor": start, "cursor": start + 2}),
                )
                .unwrap();
            let context = session.ai_editor_context();
            assert_eq!(context["file"], format!("{folder}/notes.tex"));
            assert_eq!(context["rootFile"], "main.tex");
            assert_eq!(context["cursor"], json!({"line": 2, "column": 3}));
            assert_eq!(context["selection"], json!({"startLine": 2, "endLine": 2}));
            assert_eq!(context["modified"], true);
            session
                .run("ai.send", &json!({"text": "Which file am I editing?"}))
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while session.ai.is_busy() && Instant::now() < deadline {
                session.tick(Instant::now());
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(!session.ai.is_busy());
        }
        // Excluding contents still identifies the active file, without sending its text.
        session.active = 0;
        session.run("ai.toggleDocument", &Value::Null).unwrap();
        session
            .run("ai.send", &json!({"text": "Which file now?"}))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while session.ai.is_busy() && Instant::now() < deadline {
            session.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(10));
        }
        let requests: Vec<Value> = server
            .join()
            .unwrap()
            .iter()
            .map(|request| serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap())
            .collect();
        let system = |index: usize| requests[index]["messages"][0]["content"].as_str().unwrap();
        assert!(system(0).contains("sections/notes.tex") && system(0).contains("UNSAVED_ACTIVE_A"));
        assert!(system(1).contains("appendix/notes.tex") && system(1).contains("UNSAVED_ACTIVE_B"));
        assert!(!system(1).contains("UNSAVED_ACTIVE_A"));
        assert!(system(1).contains("<selection>\n中文\n</selection>"));
        assert!(system(2).contains("sections/notes.tex"));
        assert!(!system(2).contains("<document>") && !system(2).contains("<selection>"));
        assert!(!system(2).contains("UNSAVED_ACTIVE_A") && !system(2).contains("UNSAVED_ACTIVE_B"));
        assert_eq!(session.ai_editor_context()["contentsIncluded"], false);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn an_empty_editor_or_untitled_document_never_exposes_a_scratch_file_as_the_active_file() {
        let _language = texcraft_i18n::scoped(texcraft_i18n::Lang::En);
        let mut session = Session::empty(Arc::new(|| {}));
        session.settings.live_typeset = false;
        assert_eq!(session.ai_editor_context()["file"], Value::Null);
        assert_eq!(session.ai_editor_context()["contentsIncluded"], false);
        session.new_untitled("Untitled text".into());
        assert_eq!(session.ai_active_file().as_deref(), Some("Untitled"));
        assert_eq!(session.ai_editor_context()["rootFile"], Value::Null);
        assert!(!session.ai_system().contains("texcraft-"));
    }
}
