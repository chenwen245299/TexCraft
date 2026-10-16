//! The AI Assistant's providers: their configuration (kept in `ai.json` next to the app's
//! settings, readable only by the user since it holds API keys), and streaming chat requests.
//!
//! Two wire formats cover the providers people use: Anthropic's Messages API, and the
//! OpenAI-compatible chat completions API (OpenAI, DeepSeek, Google Gemini's compatibility
//! endpoint, OpenRouter, Ollama, LM Studio, …). Requests go through the system `curl`, like
//! update downloads: the API key and URL reach curl on its standard input (`--config -`), never
//! on the command line where other processes could read them. Answers stream back as
//! server-sent events, parsed here into [`Event`]s on a background thread ([`Job`]).

pub mod inline;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The wire format a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Kind {
    /// Anthropic's Messages API (`/v1/messages`).
    Anthropic,
    /// OpenAI-compatible chat completions (`/chat/completions`).
    #[default]
    OpenAi,
}

impl Kind {
    /// Shown in Settings.
    pub fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic Messages API",
            Self::OpenAi => "OpenAI-compatible",
        }
    }
}

/// One configured provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Provider {
    /// Stable id (`anthropic`, `openai`, `custom-2`, …).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Wire format.
    pub kind: Kind,
    /// API base URL: `https://api.anthropic.com` (Anthropic) or the URL that ends before
    /// `/chat/completions` (`https://api.openai.com/v1`).
    pub base_url: String,
    /// An API key is stored for it in the [`KeyStore`] (the key itself never lives here, so
    /// it is never written to `ai.json`, shown or reported).
    #[serde(skip)]
    pub has_key: bool,
    /// Model ids offered in the Assistant.
    pub models: Vec<String>,
    /// Shown in the Assistant's model menu.
    pub enabled: bool,
}

impl Default for Provider {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: Kind::OpenAi,
            base_url: String::new(),
            has_key: false,
            models: Vec::new(),
            enabled: true,
        }
    }
}

/// A provider template for the "+" menu in Settings → AI.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    /// Id prefix and display name.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Wire format.
    pub kind: Kind,
    /// Base URL.
    pub base_url: &'static str,
    /// Models offered at first (more can be fetched or typed).
    pub models: &'static [&'static str],
    /// Whether it needs an API key.
    pub needs_key: bool,
}

/// Provider templates.
pub const PRESETS: &[Preset] = &[
    // NUS School of Computing's LLM service (first, as in Argus). OpenAI-compatible; its model
    // names are aliases the operators may repoint, so nothing is inferred from them.
    Preset {
        id: "soclaas",
        name: "SoCLaaS (NUS)",
        kind: Kind::OpenAi,
        base_url: SOCLAAS_BASE_URL,
        models: &[],
        needs_key: true,
    },
    Preset {
        id: "anthropic",
        name: "Anthropic",
        kind: Kind::Anthropic,
        base_url: "https://api.anthropic.com",
        models: &["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"],
        needs_key: true,
    },
    Preset {
        id: "openai",
        name: "OpenAI",
        kind: Kind::OpenAi,
        base_url: "https://api.openai.com/v1",
        models: &[],
        needs_key: true,
    },
    Preset {
        id: "gemini",
        name: "Google Gemini",
        kind: Kind::OpenAi,
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        models: &[],
        needs_key: true,
    },
    Preset {
        id: "openrouter",
        name: "OpenRouter",
        kind: Kind::OpenAi,
        base_url: "https://openrouter.ai/api/v1",
        models: &[],
        needs_key: true,
    },
    Preset {
        id: "deepseek",
        name: "DeepSeek",
        kind: Kind::OpenAi,
        base_url: "https://api.deepseek.com/v1",
        models: &["deepseek-chat", "deepseek-reasoner"],
        needs_key: true,
    },
    // The Chinese providers below are the ones Argus offers, at the same addresses (their
    // OpenAI-compatible endpoints); their model lists change often, so Fetch Models fills them.
    Preset {
        id: "kimi",
        name: "Kimi (Moonshot)",
        kind: Kind::OpenAi,
        base_url: "https://api.moonshot.cn/v1",
        models: &[],
        needs_key: true,
    },
    Preset {
        id: "qwen",
        name: "Qwen Token Plan (千问)",
        kind: Kind::OpenAi,
        base_url: "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
        models: &[],
        needs_key: true,
    },
    Preset {
        id: "mimo",
        name: "Xiaomi MiMo",
        kind: Kind::OpenAi,
        base_url: "https://api.xiaomimimo.com/v1",
        models: &[],
        needs_key: true,
    },
    Preset {
        id: "zhipu",
        name: "Zhipu GLM (智谱)",
        kind: Kind::OpenAi,
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        models: &[],
        needs_key: true,
    },
    Preset {
        id: "minimax",
        name: "MiniMax",
        kind: Kind::OpenAi,
        base_url: "https://api.minimax.cn/v1",
        models: &[],
        needs_key: true,
    },
    Preset {
        id: "stepfun",
        name: "StepFun (阶跃星辰)",
        kind: Kind::OpenAi,
        base_url: "https://api.stepfun.com/v1",
        models: &[],
        needs_key: true,
    },
    Preset {
        id: "moleapi",
        name: "MoleAPI",
        kind: Kind::OpenAi,
        base_url: "https://api.moleapi.com/v1",
        models: &[],
        needs_key: true,
    },
    Preset {
        id: "ollama",
        name: "Ollama (local)",
        kind: Kind::OpenAi,
        base_url: "http://localhost:11434/v1",
        models: &[],
        needs_key: false,
    },
    Preset {
        id: "custom",
        name: "Custom (OpenAI-compatible)",
        kind: Kind::OpenAi,
        base_url: "",
        models: &[],
        needs_key: true,
    },
];

/// SoCLaaS's public gateway (https://dochub.comp.nus.edu.sg/cf/guides/soclaas/api).
pub const SOCLAAS_BASE_URL: &str = "https://soclaas-api.comp.nus.edu.sg/v1";

/// NUS SoCLaaS, by preset, address or name (as Argus recognises it, "SoCLass" spelling too).
/// Its model names are private aliases: no vendor or logo is inferred from them.
pub fn is_soclaas(provider: &Provider) -> bool {
    let name = provider.name.to_ascii_lowercase();
    provider.id.starts_with("soclaas")
        || provider.base_url.to_ascii_lowercase().contains("soclaas")
        || name.contains("soclaas")
        || name.contains("soclass")
}

/// The Assistant's settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Providers, in Settings order.
    pub providers: Vec<Provider>,
    /// The chosen provider id.
    pub provider: String,
    /// The chosen model.
    pub model: String,
    /// Send the open document with each question.
    pub include_document: bool,
    /// Inline suggestions in the editor.
    pub inline: inline::InlineSettings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            provider: String::new(),
            model: String::new(),
            include_document: true,
            inline: inline::InlineSettings::default(),
        }
    }
}

impl Config {
    /// Reads `path`; a missing or unreadable file gives the defaults.
    pub fn load(path: &Path) -> Self {
        let mut c: Self = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        c.normalize();
        c
    }

    /// Writes `path`, readable by the owner only (it holds API keys).
    pub fn save(&self, path: &Path) -> Result<(), AiError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| err(format!("cannot save: {e}")))?;
        }
        let text =
            serde_json::to_string_pretty(self).map_err(|e| err(format!("cannot save: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        write_private(&tmp, text.as_bytes())?;
        std::fs::rename(&tmp, path).map_err(|e| err(format!("cannot save: {e}")))
    }

    /// Adds a provider from `preset` with a unique id; returns its index.
    pub fn add(&mut self, preset: &Preset) -> usize {
        let mut id = preset.id.to_string();
        let mut n = 2;
        while self.providers.iter().any(|p| p.id == id) {
            id = format!("{}-{n}", preset.id);
            n += 1;
        }
        self.providers.push(Provider {
            id,
            name: preset.name.to_string(),
            kind: preset.kind,
            base_url: preset.base_url.to_string(),
            has_key: false,
            models: preset.models.iter().map(|m| (*m).to_string()).collect(),
            enabled: true,
        });
        self.normalize();
        self.providers.len() - 1
    }

    /// The chosen provider and model, falling back to the first usable ones.
    pub fn selection(&self) -> Option<(&Provider, &str)> {
        let usable = |p: &&Provider| p.enabled && !p.models.is_empty();
        let p = self
            .providers
            .iter()
            .filter(usable)
            .find(|p| p.id == self.provider)
            .or_else(|| self.providers.iter().find(usable))?;
        let m = p
            .models
            .iter()
            .find(|m| **m == self.model)
            .or_else(|| p.models.first())?;
        Some((p, m.as_str()))
    }

    /// The provider and model for inline suggestions: the ones chosen for them, else the
    /// Assistant's.
    pub fn inline_selection(&self) -> Option<(&Provider, &str)> {
        let chosen = self
            .providers
            .iter()
            .filter(|p| p.enabled)
            .find(|p| p.id == self.inline.provider)
            .and_then(|p| {
                p.models
                    .iter()
                    .find(|m| **m == self.inline.model)
                    .map(|m| (p, m.as_str()))
            });
        chosen.or_else(|| self.selection())
    }

    /// Keeps the choice valid after edits.
    pub fn normalize(&mut self) {
        for p in &mut self.providers {
            p.models.retain(|m| !m.trim().is_empty());
            p.models.dedup();
        }
        if let Some((p, m)) = self.selection().map(|(p, m)| (p.id.clone(), m.to_string())) {
            self.provider = p;
            self.model = m;
        }
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), AiError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| err(format!("cannot save: {e}")))?;
    f.write_all(bytes)
        .map_err(|e| err(format!("cannot save: {e}")))
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), AiError> {
    // On Windows the file inherits the user profile folder's permissions (owner only).
    std::fs::write(path, bytes).map_err(|e| err(format!("cannot save: {e}")))
}

/// Where API keys are kept: encrypted with AES-256-GCM in `api_keys.json` under a random
/// 256-bit master key in `.keymaster`, both readable by the owner only, in the app's settings
/// folder (the scheme Argus uses). Each key is bound to its provider id (associated data) and
/// sealed with a fresh random nonce. Keys are decrypted only when a request is sent; nothing else
/// in TexCraft reads them, and `ai.json` never holds them.
#[derive(Clone)]
pub struct KeyStore {
    dir: PathBuf,
}

impl std::fmt::Debug for KeyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyStore").field("dir", &self.dir).finish()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| s.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()))
        .collect()
}

impl KeyStore {
    /// The store in `dir` (created on the first save).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn map_path(&self) -> PathBuf {
        self.dir.join("api_keys.json")
    }

    fn master(&self, create: bool) -> Result<[u8; 32], AiError> {
        let path = self.dir.join(".keymaster");
        if let Ok(text) = std::fs::read_to_string(&path) {
            return unhex(&text)
                .and_then(|b| <[u8; 32]>::try_from(b).ok())
                .ok_or_else(|| err("the key store's master key is damaged"));
        }
        if !create {
            return Err(err("no key store yet"));
        }
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|e| err(format!("no random numbers: {e}")))?;
        std::fs::create_dir_all(&self.dir).map_err(|e| err(format!("cannot save: {e}")))?;
        write_private(&path, hex(&key).as_bytes())?;
        Ok(key)
    }

    fn read_map(&self) -> std::collections::BTreeMap<String, String> {
        std::fs::read_to_string(self.map_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn write_map(&self, map: &std::collections::BTreeMap<String, String>) -> Result<(), AiError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| err(format!("cannot save: {e}")))?;
        let text = serde_json::to_string_pretty(map).map_err(|e| err(e.to_string()))?;
        let tmp = self.dir.join("api_keys.json.tmp");
        write_private(&tmp, text.as_bytes())?;
        std::fs::rename(&tmp, self.map_path()).map_err(|e| err(format!("cannot save: {e}")))
    }

    /// Stores `key` for provider `id` (replacing any earlier one).
    pub fn save(&self, id: &str, key: &str) -> Result<(), AiError> {
        use aes_gcm::aead::{Aead, KeyInit, Payload};
        let master = self.master(true)?;
        let cipher = aes_gcm::Aes256Gcm::new_from_slice(&master)
            .map_err(|_| err("cannot set up encryption"))?;
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|e| err(format!("no random numbers: {e}")))?;
        let sealed = cipher
            .encrypt(
                &aes_gcm::Nonce::from(nonce),
                Payload {
                    msg: key.trim().as_bytes(),
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| err("encryption failed"))?;
        let mut map = self.read_map();
        map.insert(id.to_string(), format!("{}:{}", hex(&nonce), hex(&sealed)));
        self.write_map(&map)
    }

    /// The key of provider `id`, decrypted (only for sending a request).
    pub fn get(&self, id: &str) -> Option<String> {
        use aes_gcm::aead::{Aead, KeyInit, Payload};
        let map = self.read_map();
        let (n, c) = map.get(id)?.split_once(':')?;
        let nonce = <[u8; 12]>::try_from(unhex(n)?).ok()?;
        let sealed = unhex(c)?;
        let master = self.master(false).ok()?;
        let cipher = aes_gcm::Aes256Gcm::new_from_slice(&master).ok()?;
        let plain = cipher
            .decrypt(
                &aes_gcm::Nonce::from(nonce),
                Payload {
                    msg: &sealed,
                    aad: id.as_bytes(),
                },
            )
            .ok()?;
        String::from_utf8(plain).ok().filter(|k| !k.is_empty())
    }

    /// Whether a key is stored for `id` (without decrypting it).
    pub fn has(&self, id: &str) -> bool {
        self.read_map().contains_key(id)
    }

    /// Forgets the key of `id`.
    pub fn delete(&self, id: &str) -> Result<(), AiError> {
        let mut map = self.read_map();
        if map.remove(id).is_some() {
            self.write_map(&map)?;
        }
        Ok(())
    }

    /// Sets each provider's `has_key`.
    pub fn mark(&self, config: &mut Config) {
        let map = self.read_map();
        for p in &mut config.providers {
            p.has_key = map.contains_key(&p.id);
        }
    }
}

/// Whether `provider` runs on this computer (no key needed).
pub fn is_local(provider: &Provider) -> bool {
    let u = provider.base_url.to_ascii_lowercase();
    u.contains("://localhost") || u.contains("://127.0.0.1") || u.contains("://[::1]")
}

/// An error shown in the Assistant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiError(pub String);

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err(msg: impl Into<String>) -> AiError {
    AiError(msg.into())
}

/// Who wrote a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// The person.
    User,
    /// The model.
    Assistant,
}

/// One turn of the conversation as sent to a provider.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    /// Who.
    pub role: Role,
    /// The text.
    pub text: String,
    /// For assistant turns of the Anthropic API: the content blocks as received (thinking
    /// blocks included), sent back unchanged.
    pub blocks: Vec<Value>,
}

/// A request: the provider, model, instructions and conversation.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// Where to send it.
    pub provider: Provider,
    /// Model id.
    pub model: String,
    /// System prompt.
    pub system: String,
    /// The provider's API key, read from the [`KeyStore`] just for this request (empty for
    /// local servers).
    pub api_key: String,
    /// The conversation, ending with the user's question.
    pub turns: Vec<Turn>,
    /// The longest answer, in tokens (`None`: as long as the model allows, for chat).
    pub max_tokens: Option<u32>,
}

/// Models that accept Anthropic's server-side refusal fallback (`fallbacks: "default"`).
fn supports_fallback(model: &str) -> bool {
    [
        "claude-opus-5-5",
        "claude-opus-5",
        "claude-fable-5-1",
        "claude-fable-5",
        "claude-sonnet-5-5",
    ]
    .contains(&model)
}

/// Anthropic content blocks of an assistant turn, ready to send back: after a mid-output
/// fallback, thinking and tool blocks before the last `fallback` block are left out (the API's
/// rule for echoing fallback turns).
fn echo_blocks(blocks: &[Value]) -> Vec<Value> {
    let boundary = blocks
        .iter()
        .rposition(|b| b.get("type").and_then(Value::as_str) == Some("fallback"));
    blocks
        .iter()
        .enumerate()
        .filter(|(i, b)| {
            let ty = b.get("type").and_then(Value::as_str).unwrap_or("");
            let before = boundary.is_some_and(|k| *i < k);
            !(before && matches!(ty, "thinking" | "redacted_thinking" | "tool_use"))
        })
        .map(|(_, b)| b.clone())
        .collect()
}

/// The endpoint, headers and JSON body of a streaming request.
pub fn build(req: &ChatRequest) -> (String, Vec<String>, Value) {
    let base = req.provider.base_url.trim().trim_end_matches('/');
    match req.provider.kind {
        Kind::Anthropic => {
            let messages: Vec<Value> = req
                .turns
                .iter()
                .map(|t| match t.role {
                    Role::User => json!({ "role": "user", "content": t.text }),
                    Role::Assistant if !t.blocks.is_empty() => {
                        json!({ "role": "assistant", "content": echo_blocks(&t.blocks) })
                    }
                    Role::Assistant => json!({ "role": "assistant", "content": t.text }),
                })
                .collect();
            let mut body = json!({
                "model": req.model,
                "max_tokens": req.max_tokens.unwrap_or(64000),
                "stream": true,
                "system": req.system,
                "messages": messages,
            });
            let mut headers = vec![
                format!("x-api-key: {}", req.api_key.trim()),
                "anthropic-version: 2023-06-01".to_string(),
                "content-type: application/json".to_string(),
            ];
            // A declined request is retried on Anthropic's recommended fallback model instead
            // of coming back as a refusal (Claude API only; proxies may not know the field).
            if supports_fallback(&req.model) && base == "https://api.anthropic.com" {
                headers.push("anthropic-beta: server-side-fallback-2026-07-01".to_string());
                if let Some(o) = body.as_object_mut() {
                    o.insert("fallbacks".into(), json!("default"));
                }
            }
            (format!("{base}/v1/messages"), headers, body)
        }
        Kind::OpenAi => {
            let mut messages = vec![json!({ "role": "system", "content": req.system })];
            messages.extend(req.turns.iter().map(|t| {
                json!({
                    "role": if t.role == Role::User { "user" } else { "assistant" },
                    "content": t.text,
                })
            }));
            let mut headers = vec!["content-type: application/json".to_string()];
            let key = req.api_key.trim();
            if !key.is_empty() {
                headers.push(format!("authorization: Bearer {key}"));
            }
            let mut body = json!({ "model": req.model, "stream": true, "messages": messages });
            if let Some(n) = req.max_tokens {
                // OpenAI's own API takes the newer name (its reasoning models refuse the old).
                let field = if base.contains("api.openai.com") {
                    "max_completion_tokens"
                } else {
                    "max_tokens"
                };
                body[field] = json!(n);
            }
            if is_minimax(&req.provider) {
                // Without it MiniMax puts its chain of thought in the answer, in <think> tags.
                body["reasoning_split"] = json!(true);
            }
            (format!("{base}/chat/completions"), headers, body)
        }
    }
}

/// MiniMax (its own servers): it reasons in the answer unless asked to split the reasoning out.
fn is_minimax(provider: &Provider) -> bool {
    provider.id.starts_with("minimax") || provider.base_url.to_ascii_lowercase().contains("minimax")
}

/// What a running request reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// More answer text.
    Text(String),
    /// The model is thinking (no text yet).
    Thinking,
    /// A declined request was handed to another model (`from` → `to`).
    Fallback(String),
    /// Finished: the full Anthropic content blocks (empty for other providers).
    Done {
        /// Content blocks to send back with the next request.
        blocks: Vec<Value>,
        /// The model that answered.
        model: Option<String>,
    },
    /// The model declined; any partial answer must be discarded.
    Refused(Option<String>),
    /// The request failed.
    Failed(String),
}

/// Turns server-sent events into [`Event`]s. Feed it one line at a time.
#[derive(Debug, Default)]
pub struct SseParser {
    kind: Kind,
    blocks: Vec<Value>,
    model: Option<String>,
    stop_reason: Option<String>,
    refusal_category: Option<String>,
    /// Lines that were not events (an error body).
    other: String,
    done: bool,
}

impl SseParser {
    /// A parser for `kind`.
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }

    /// Handles one line of the stream.
    pub fn line(&mut self, line: &str) -> Vec<Event> {
        let line = line.trim_end_matches(['\r', '\n']);
        let Some(data) = line.strip_prefix("data:") else {
            if !line.starts_with("event:") && !line.starts_with(':') && !line.trim().is_empty() {
                self.other.push_str(line);
                self.other.push('\n');
            }
            return Vec::new();
        };
        let data = data.trim();
        if data == "[DONE]" {
            self.done = true;
            return Vec::new();
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        match self.kind {
            Kind::Anthropic => self.anthropic(&v),
            Kind::OpenAi => self.openai(&v),
        }
    }

    fn anthropic(&mut self, v: &Value) -> Vec<Event> {
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            "message_start" => {
                self.model = v
                    .pointer("/message/model")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                Vec::new()
            }
            "content_block_start" => {
                let block = v.get("content_block").cloned().unwrap_or(Value::Null);
                let bty = block.get("type").and_then(Value::as_str).unwrap_or("");
                let mut out = Vec::new();
                match bty {
                    "thinking" | "redacted_thinking" => out.push(Event::Thinking),
                    "fallback" => {
                        let from = block
                            .pointer("/from/model")
                            .and_then(Value::as_str)
                            .unwrap_or("the model");
                        let to = block
                            .pointer("/to/model")
                            .and_then(Value::as_str)
                            .unwrap_or("another model");
                        out.push(Event::Fallback(format!("{from} → {to}")));
                    }
                    "text" => {
                        if let Some(t) = block.get("text").and_then(Value::as_str)
                            && !t.is_empty()
                        {
                            out.push(Event::Text(t.to_string()));
                        }
                    }
                    _ => {}
                }
                self.blocks.push(block);
                out
            }
            "content_block_delta" => {
                let Some(block) = self.blocks.last_mut() else {
                    return Vec::new();
                };
                let delta = v.get("delta").cloned().unwrap_or(Value::Null);
                let append = |block: &mut Value, field: &str, s: &str| {
                    let cur = block
                        .get(field)
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if let Some(o) = block.as_object_mut() {
                        o.insert(field.into(), Value::String(cur + s));
                    }
                };
                match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text_delta" => {
                        let t = delta.get("text").and_then(Value::as_str).unwrap_or("");
                        append(block, "text", t);
                        vec![Event::Text(t.to_string())]
                    }
                    "thinking_delta" => {
                        let t = delta.get("thinking").and_then(Value::as_str).unwrap_or("");
                        append(block, "thinking", t);
                        Vec::new()
                    }
                    "signature_delta" => {
                        if let (Some(sig), Some(o)) = (
                            delta.get("signature").and_then(Value::as_str),
                            block.as_object_mut(),
                        ) {
                            o.insert("signature".into(), Value::String(sig.into()));
                        }
                        Vec::new()
                    }
                    _ => Vec::new(),
                }
            }
            "message_delta" => {
                if let Some(r) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(r.to_string());
                }
                if let Some(c) = v
                    .pointer("/delta/stop_details/category")
                    .and_then(Value::as_str)
                {
                    self.refusal_category = Some(c.to_string());
                }
                Vec::new()
            }
            "message_stop" => {
                self.done = true;
                Vec::new()
            }
            "error" => {
                let msg = v
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("the provider reported an error");
                vec![Event::Failed(msg.to_string())]
            }
            _ => Vec::new(),
        }
    }

    fn openai(&mut self, v: &Value) -> Vec<Event> {
        if let Some(e) = v.get("error") {
            let msg = e
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| e.to_string(), str::to_string);
            return vec![Event::Failed(msg)];
        }
        if self.model.is_none() {
            self.model = v.get("model").and_then(Value::as_str).map(str::to_string);
        }
        let mut out = Vec::new();
        if let Some(choice) = v.pointer("/choices/0") {
            if let Some(t) = choice.pointer("/delta/content").and_then(Value::as_str)
                && !t.is_empty()
            {
                out.push(Event::Text(t.to_string()));
            } else if reasoning_delta(choice) {
                out.push(Event::Thinking);
            }
            if let Some(r) = choice.get("finish_reason").and_then(Value::as_str) {
                self.stop_reason = Some(r.to_string());
            }
        }
        out
    }

    /// The final event once the stream ended (curl exited with `status_ok`).
    pub fn finish(&mut self, status_ok: bool) -> Event {
        if self.stop_reason.as_deref() == Some("refusal") {
            return Event::Refused(self.refusal_category.clone());
        }
        if !status_ok || (!self.done && self.blocks.is_empty() && self.stop_reason.is_none()) {
            return Event::Failed(error_message(&self.other));
        }
        Event::Done {
            blocks: std::mem::take(&mut self.blocks),
            model: self.model.clone(),
        }
    }
}

/// Whether an OpenAI-style stream chunk carries reasoning rather than answer text:
/// `reasoning_content` (DeepSeek, MiMo, GLM, Kimi, Qwen), `reasoning` (OpenRouter, Ollama,
/// StepFun), `thinking`, or MiniMax's `reasoning_details[].text`.
fn reasoning_delta(choice: &Value) -> bool {
    let text = |p: &str| {
        choice
            .pointer(p)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
    };
    text("/delta/reasoning_content")
        || text("/delta/reasoning")
        || text("/delta/thinking")
        || choice
            .pointer("/delta/reasoning_details")
            .and_then(Value::as_array)
            .is_some_and(|d| {
                d.iter().any(|x| {
                    x.get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty())
                })
            })
}

/// A readable message from an error body (JSON `error.message`, else the text).
pub fn error_message(body: &str) -> String {
    let body = body.trim();
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        let m = v
            .pointer("/error/message")
            .or_else(|| v.get("message"))
            .or_else(|| v.pointer("/error"))
            .and_then(Value::as_str);
        if let Some(m) = m {
            return m.to_string();
        }
    }
    if body.is_empty() {
        "The provider did not answer. Check the base URL, API key and network.".into()
    } else {
        body.chars().take(400).collect()
    }
}

/// Quotes a value for a curl config file.
fn curl_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Starts curl for `url` with `headers` (and the JSON `body` in a private temp file), the
/// secrets passed on standard input. Returns the child and the temp file to delete.
fn spawn_curl(
    url: &str,
    headers: &[String],
    body: Option<&Value>,
    stream: bool,
) -> Result<(Child, Option<PathBuf>), AiError> {
    let mut config = format!("url = {}\n", curl_quote(url));
    for h in headers {
        config.push_str(&format!("header = {}\n", curl_quote(h)));
    }
    let body_file = match body {
        Some(b) => {
            let path = std::env::temp_dir().join(format!(
                "texcraft-ai-{}-{}.json",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos())
            ));
            write_private(&path, b.to_string().as_bytes())?;
            config.push_str(&format!(
                "data-binary = {}\n",
                curl_quote(&format!("@{}", path.display()))
            ));
            Some(path)
        }
        None => None,
    };
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "--fail-with-body",
        "--config",
        "-",
        "--max-time",
        "600",
    ]);
    if stream {
        cmd.arg("-N");
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| {
        if let Some(p) = &body_file {
            let _ = std::fs::remove_file(p);
        }
        err(format!("cannot run curl: {e}"))
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(config.as_bytes());
    }
    Ok((child, body_file))
}

/// A running chat request.
pub struct Job {
    rx: Receiver<Event>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Job {
    /// The next event, if any.
    pub fn try_recv(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }

    /// Stops the request.
    pub fn cancel(&self) {
        if let Ok(mut c) = self.child.lock()
            && let Some(child) = c.as_mut()
        {
            let _ = child.kill();
        }
    }
}

/// Sends `req` on a background thread; events arrive through the [`Job`] (`waker` is called
/// after each).
pub fn start(req: &ChatRequest, waker: Arc<dyn Fn() + Send + Sync>) -> Job {
    let (tx, rx) = channel();
    let slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let job = Job {
        rx,
        child: slot.clone(),
    };
    let (url, headers, body) = build(req);
    let kind = req.provider.kind;
    let send = move |tx: &Sender<Event>, e: Event| {
        let _ = tx.send(e);
        waker();
    };
    let spawned = std::thread::Builder::new()
        .name("texcraft-ai".into())
        .spawn(move || {
            let (mut child, body_file) = match spawn_curl(&url, &headers, Some(&body), true) {
                Ok(c) => c,
                Err(e) => {
                    send(&tx, Event::Failed(e.0));
                    return;
                }
            };
            let stdout = child.stdout.take();
            let mut stderr = child.stderr.take();
            if let Ok(mut c) = slot.lock() {
                *c = Some(child);
            }
            let mut parser = SseParser::new(kind);
            if let Some(out) = stdout {
                for line in BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    for e in parser.line(&line) {
                        send(&tx, e);
                    }
                }
            }
            let mut errs = String::new();
            if let Some(e) = stderr.as_mut() {
                let _ = e.read_to_string(&mut errs);
            }
            let status = slot
                .lock()
                .ok()
                .and_then(|mut c| c.take())
                .and_then(|mut c| c.wait().ok());
            if let Some(p) = body_file {
                let _ = std::fs::remove_file(p);
            }
            let ok = status.is_some_and(|s| s.success());
            let mut last = parser.finish(ok);
            if let Event::Failed(m) = &mut last
                && parser.other.trim().is_empty()
                && !errs.trim().is_empty()
            {
                *m = errs.trim().to_string();
            }
            send(&tx, last);
        });
    if let Err(e) = spawned {
        let (tx2, rx2) = channel();
        let _ = tx2.send(Event::Failed(format!("cannot start the request: {e}")));
        return Job {
            rx: rx2,
            child: Arc::new(Mutex::new(None)),
        };
    }
    job
}

/// Lists the provider's models (`GET /v1/models` or `/models`). Blocking: call it off the UI
/// thread.
pub fn fetch_models(provider: &Provider, api_key: &str) -> Result<Vec<String>, AiError> {
    let base = provider.base_url.trim().trim_end_matches('/');
    let (url, headers) = match provider.kind {
        Kind::Anthropic => (
            format!("{base}/v1/models?limit=100"),
            vec![
                format!("x-api-key: {}", api_key.trim()),
                "anthropic-version: 2023-06-01".to_string(),
            ],
        ),
        Kind::OpenAi => {
            let mut h = Vec::new();
            if !api_key.trim().is_empty() {
                h.push(format!("authorization: Bearer {}", api_key.trim()));
            }
            (format!("{base}/models"), h)
        }
    };
    let (mut child, _) = spawn_curl(&url, &headers, None, false)?;
    let mut out = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    let mut errs = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut errs);
    }
    let ok = child.wait().is_ok_and(|s| s.success());
    if !ok {
        let msg = error_message(&out);
        return Err(err(if out.trim().is_empty() && !errs.trim().is_empty() {
            errs.trim().to_string()
        } else {
            msg
        }));
    }
    parse_models(&out)
}

/// Model ids from a `/models` answer (`{"data": [{"id": …}]}`, also Ollama's `{"models": …}`).
pub fn parse_models(body: &str) -> Result<Vec<String>, AiError> {
    let v: Value =
        serde_json::from_str(body).map_err(|_| err("the provider sent an unexpected answer"))?;
    let list = v
        .get("data")
        .or_else(|| v.get("models"))
        .and_then(Value::as_array)
        .ok_or_else(|| err("the provider sent no model list"))?;
    let mut ids: Vec<String> = list
        .iter()
        .filter_map(|m| {
            m.get("id")
                .or_else(|| m.get("name"))
                .and_then(Value::as_str)
                .map(|s| s.trim_start_matches("models/").to_string())
        })
        .collect();
    ids.dedup();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(kind: Kind, base: &str) -> Provider {
        Provider {
            id: "p".into(),
            name: "P".into(),
            kind,
            base_url: base.into(),
            has_key: true,
            models: vec!["m".into()],
            enabled: true,
        }
    }

    #[test]
    fn builds_anthropic_requests_with_fallbacks_and_echoed_blocks() {
        let thinking = json!({"type": "thinking", "thinking": "", "signature": "sig"});
        let req = ChatRequest {
            provider: provider(Kind::Anthropic, "https://api.anthropic.com/"),
            model: "claude-opus-5-5".into(),
            system: "You help with LaTeX.".into(),
            api_key: "sk-test".into(),
            turns: vec![
                Turn {
                    role: Role::User,
                    text: "Hi".into(),
                    blocks: vec![],
                },
                Turn {
                    role: Role::Assistant,
                    text: "Hello".into(),
                    blocks: vec![thinking.clone(), json!({"type": "text", "text": "Hello"})],
                },
                Turn {
                    role: Role::User,
                    text: "Table?".into(),
                    blocks: vec![],
                },
            ],
            max_tokens: None,
        };
        let (url, headers, body) = build(&req);
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert!(headers.contains(&"x-api-key: sk-test".to_string()));
        assert!(headers.contains(&"anthropic-beta: server-side-fallback-2026-07-01".to_string()));
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["stream"], true);
        assert_eq!(body["system"], "You help with LaTeX.");
        assert_eq!(body["messages"][1]["content"][0], thinking);
        // No fallbacks for models without them, or through other hosts.
        let mut r2 = req.clone();
        r2.model = "claude-haiku-5-5".into();
        assert!(build(&r2).2.get("fallbacks").is_none());
        let mut r3 = req;
        r3.provider.base_url = "https://proxy.example/".into();
        assert!(build(&r3).2.get("fallbacks").is_none());
    }

    #[test]
    fn echo_drops_thinking_before_a_fallback_boundary() {
        let blocks = vec![
            json!({"type": "thinking", "thinking": "a"}),
            json!({"type": "text", "text": "partial"}),
            json!({"type": "fallback", "from": {"model": "x"}, "to": {"model": "y"}}),
            json!({"type": "thinking", "thinking": "b"}),
            json!({"type": "text", "text": "rest"}),
        ];
        let out = echo_blocks(&blocks);
        let types: Vec<&str> = out.iter().filter_map(|b| b["type"].as_str()).collect();
        assert_eq!(types, ["text", "fallback", "thinking", "text"]);
    }

    #[test]
    fn builds_openai_requests() {
        let req = ChatRequest {
            provider: provider(Kind::OpenAi, "http://localhost:11434/v1"),
            model: "llama".into(),
            system: "S".into(),
            api_key: "sk-test".into(),
            turns: vec![Turn {
                role: Role::User,
                text: "Q".into(),
                blocks: vec![],
            }],
            max_tokens: None,
        };
        let (url, headers, body) = build(&req);
        assert_eq!(url, "http://localhost:11434/v1/chat/completions");
        assert!(headers.contains(&"authorization: Bearer sk-test".to_string()));
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "Q");
    }

    #[test]
    fn presets_cover_argus_providers_at_their_addresses() {
        let url = |id: &str| PRESETS.iter().find(|p| p.id == id).map(|p| p.base_url);
        for (id, base) in [
            ("soclaas", "https://soclaas-api.comp.nus.edu.sg/v1"),
            ("openrouter", "https://openrouter.ai/api/v1"),
            ("deepseek", "https://api.deepseek.com/v1"),
            ("kimi", "https://api.moonshot.cn/v1"),
            (
                "qwen",
                "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
            ),
            ("mimo", "https://api.xiaomimimo.com/v1"),
            ("zhipu", "https://open.bigmodel.cn/api/paas/v4"),
            ("minimax", "https://api.minimax.cn/v1"),
            ("stepfun", "https://api.stepfun.com/v1"),
            ("moleapi", "https://api.moleapi.com/v1"),
        ] {
            assert_eq!(url(id), Some(base), "{id}");
        }
        // Ids are unique (they key the API keys).
        let mut ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len());
    }

    #[test]
    fn soclaas_is_recognised_by_preset_address_or_name() {
        let p = |id: &str, name: &str, url: &str| Provider {
            id: id.into(),
            name: name.into(),
            base_url: url.into(),
            ..Provider::default()
        };
        assert!(is_soclaas(&p("soclaas", "SoCLaaS (NUS)", SOCLAAS_BASE_URL)));
        assert!(is_soclaas(&p("custom", "Research", SOCLAAS_BASE_URL)));
        assert!(is_soclaas(&p(
            "custom",
            "My SoCLass",
            "https://relay.example/v1"
        )));
        assert!(!is_soclaas(&p(
            "openai",
            "OpenAI",
            "https://api.openai.com/v1"
        )));
    }

    #[test]
    fn minimax_is_asked_to_keep_its_reasoning_out_of_the_answer() {
        let mut req = ChatRequest {
            provider: provider(Kind::OpenAi, "https://api.minimax.cn/v1"),
            model: "m".into(),
            system: "S".into(),
            api_key: "k".into(),
            turns: vec![],
            max_tokens: None,
        };
        assert_eq!(build(&req).2["reasoning_split"], true);
        req.provider = provider(Kind::OpenAi, "https://api.deepseek.com/v1");
        assert!(build(&req).2.get("reasoning_split").is_none());
    }

    #[test]
    fn reasoning_fields_of_every_provider_are_not_answer_text() {
        for chunk in [
            r#"{"choices":[{"delta":{"reasoning_content":"r"}}]}"#,
            r#"{"choices":[{"delta":{"reasoning":"r"}}]}"#,
            r#"{"choices":[{"delta":{"thinking":"r"}}]}"#,
            r#"{"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.text","text":"r"}]}}]}"#,
        ] {
            let mut p = SseParser::new(Kind::OpenAi);
            assert_eq!(
                p.line(&format!("data: {chunk}")),
                vec![Event::Thinking],
                "{chunk}"
            );
        }
    }

    #[test]
    fn parses_anthropic_streams() {
        let mut p = SseParser::new(Kind::Anthropic);
        let mut text = String::new();
        let mut thinking = false;
        for line in [
            "event: message_start",
            r#"data: {"type":"message_start","message":{"model":"claude-opus-5-5"}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Use "}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"\\begin{table}"}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            r#"data: {"type":"message_stop"}"#,
        ] {
            for e in p.line(line) {
                match e {
                    Event::Text(t) => text.push_str(&t),
                    Event::Thinking => thinking = true,
                    _ => {}
                }
            }
        }
        assert_eq!(text, "Use \\begin{table}");
        assert!(thinking);
        match p.finish(true) {
            Event::Done { blocks, model } => {
                assert_eq!(model.as_deref(), Some("claude-opus-5-5"));
                assert_eq!(blocks[0]["signature"], "abc");
                assert_eq!(blocks[1]["text"], "Use \\begin{table}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn refusals_and_errors() {
        let mut p = SseParser::new(Kind::Anthropic);
        p.line(r#"data: {"type":"message_delta","delta":{"stop_reason":"refusal","stop_details":{"type":"refusal","category":"cyber"}}}"#);
        assert_eq!(p.finish(true), Event::Refused(Some("cyber".into())));
        // An HTTP error body (curl --fail-with-body prints it, exits 22).
        let mut p = SseParser::new(Kind::Anthropic);
        p.line(r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#);
        assert_eq!(p.finish(false), Event::Failed("invalid x-api-key".into()));
        let mut p = SseParser::new(Kind::OpenAi);
        assert_eq!(
            p.line(r#"data: {"choices":[{"delta":{"content":"Hi"}}],"model":"m"}"#),
            vec![Event::Text("Hi".into())]
        );
        p.line("data: [DONE]");
        assert!(matches!(p.finish(true), Event::Done { .. }));
    }

    #[test]
    fn config_round_trips_privately_and_keeps_a_valid_choice() {
        let dir = std::env::temp_dir().join(format!("texcraft-ai-cfg-{}", std::process::id()));
        let path = dir.join("ai.json");
        let mut c = Config::load(&path);
        assert!(c.include_document && c.providers.is_empty() && c.selection().is_none());
        let anthropic = PRESETS.iter().find(|p| p.id == "anthropic").unwrap();
        c.add(anthropic);
        assert_eq!(
            c.selection().map(|(p, m)| (p.id.as_str(), m)),
            Some(("anthropic", "claude-opus-5-5"))
        );
        let j = c.add(anthropic);
        assert_eq!(c.providers[j].id, "anthropic-2");
        c.model = "nonexistent".into();
        c.normalize();
        assert_eq!(c.model, "claude-opus-5-5");
        c.save(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(Config::load(&path), c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_are_stored_encrypted_privately_and_never_in_ai_json() {
        let dir = std::env::temp_dir().join(format!("texcraft-ai-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = KeyStore::new(&dir);
        assert_eq!(store.get("anthropic"), None);
        assert!(!store.has("anthropic"));
        store.save("anthropic", " sk-ant-secret-123 ").unwrap();
        store.save("openai", "sk-openai-456").unwrap();
        assert_eq!(store.get("anthropic").as_deref(), Some("sk-ant-secret-123"));
        assert_eq!(store.get("openai").as_deref(), Some("sk-openai-456"));
        // Neither file contains a key in clear; both are owner-only.
        let map = std::fs::read_to_string(dir.join("api_keys.json")).unwrap();
        // (Whole keys: a short fragment such as "456" turns up in random hex now and then.)
        assert!(
            !map.contains("sk-ant-secret-123")
                && !map.contains("sk-openai-456")
                && !map.contains("sk-"),
            "{map}"
        );
        #[cfg(unix)]
        for f in ["api_keys.json", ".keymaster"] {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(f)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{f}");
        }
        // A sealed key moved to another provider id does not open (bound to its id).
        let mut m: std::collections::BTreeMap<String, String> = serde_json::from_str(&map).unwrap();
        let sealed = m.get("anthropic").unwrap().clone();
        m.insert("openai".into(), sealed);
        std::fs::write(
            dir.join("api_keys.json"),
            serde_json::to_string(&m).unwrap(),
        )
        .unwrap();
        assert_eq!(store.get("openai"), None);
        // ai.json never holds keys; has_key comes from the store.
        let mut c = Config::default();
        c.add(PRESETS.iter().find(|p| p.id == "anthropic").unwrap());
        store.mark(&mut c);
        assert!(c.providers[0].has_key);
        c.save(&dir.join("ai.json")).unwrap();
        let json = std::fs::read_to_string(dir.join("ai.json")).unwrap();
        assert!(!json.contains("sk-") && !json.contains("has_key"), "{json}");
        store.delete("anthropic").unwrap();
        assert!(!store.has("anthropic"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_lists_and_error_bodies() {
        assert_eq!(
            parse_models(r#"{"data":[{"id":"a"},{"id":"b"}]}"#).unwrap(),
            vec!["a", "b"]
        );
        assert_eq!(
            parse_models(r#"{"models":[{"name":"models/gemini-x"}]}"#).unwrap(),
            vec!["gemini-x"]
        );
        assert!(parse_models("nope").is_err());
        assert_eq!(
            error_message(r#"{"error":{"message":"bad key"}}"#),
            "bad key"
        );
        assert_eq!(curl_quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    /// The whole path through curl, against a local server that streams a canned answer.
    #[test]
    fn streams_through_curl_from_a_local_server() {
        use std::net::TcpListener;
        let Ok(listener) = TcpListener::bind("127.0.0.1:0") else {
            return;
        };
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 65536];
            let n = s.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"\\\\section{Hi}\"}}]}\n\ndata: [DONE]\n\n";
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            req
        });
        let req = ChatRequest {
            provider: provider(Kind::OpenAi, &format!("http://127.0.0.1:{port}/v1")),
            model: "m".into(),
            system: "S".into(),
            api_key: "sk-test".into(),
            turns: vec![Turn {
                role: Role::User,
                text: "Q".into(),
                blocks: vec![],
            }],
            max_tokens: None,
        };
        let job = start(&req, Arc::new(|| {}));
        let mut text = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            match job.try_recv() {
                Some(Event::Text(t)) => text.push_str(&t),
                Some(Event::Done { .. }) => break,
                Some(other) => panic!("{other:?}"),
                None if std::time::Instant::now() > deadline => panic!("timed out"),
                None => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        assert_eq!(text, "\\section{Hi}");
        let seen = server.join().unwrap();
        assert!(seen.starts_with("POST /v1/chat/completions"), "{seen}");
        assert!(seen.contains("Bearer sk-test"), "{seen}");
    }
}
