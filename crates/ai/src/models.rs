//! A provider's models: their ids, display names, context lengths and capabilities (vision,
//! reasoning, tool calling, embeddings, audio, image generation), as Argus keeps them. The
//! capabilities are read once, when a list is fetched, and are the user's from then on.
//!
//! The provider's own word comes first: SoCLaaS and Ollama list `capabilities`, OpenRouter
//! `architecture` modalities and `supported_parameters`, OpenAI-style servers `modalities`,
//! LiteLLM `model_info`. What a list leaves unsaid comes from the families names show.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::{AiError, err};

/// Something a model can do. Stored by Argus's names (`"vision"`, `"tool_calling"`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Reads images: the Assistant can attach them.
    Vision,
    /// Reasons before answering.
    Reasoning,
    /// Calls tools (functions).
    ToolCalling,
    /// Embeddings (no chat).
    Embedding,
    /// Audio in or out: transcription, speech.
    Audio,
    /// Makes images.
    ImageGen,
}

impl Capability {
    /// Every capability, in the order Settings shows them.
    pub const ALL: [Capability; 6] = [
        Capability::Vision,
        Capability::Reasoning,
        Capability::ToolCalling,
        Capability::Embedding,
        Capability::Audio,
        Capability::ImageGen,
    ];

    /// Its name in Settings and menus.
    pub fn label(self) -> &'static str {
        match self {
            Capability::Vision => "Vision",
            Capability::Reasoning => "Reasoning",
            Capability::ToolCalling => "Tools",
            Capability::Embedding => "Embedding",
            Capability::Audio => "Audio",
            Capability::ImageGen => "Image Generation",
        }
    }

    /// It is no chat capability: a model with only these (and none of vision, reasoning,
    /// tools) is left out of the chat menus.
    pub fn non_chat(self) -> bool {
        matches!(
            self,
            Capability::Embedding | Capability::Audio | Capability::ImageGen
        )
    }
}

/// Whether a model with `caps` chats: no capabilities at all (plain text) or at least one chat
/// capability (Argus's rule).
pub fn chats(caps: &[Capability]) -> bool {
    caps.is_empty() || caps.iter().any(|c| !c.non_chat())
}

/// One model of a provider.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Model {
    /// The id requests use.
    pub id: String,
    /// The name shown (empty: the id).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// What it can do, sorted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<Capability>,
    /// Its context window in tokens, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u64>,
}

impl Model {
    /// A model known by its id only: capabilities from the family the id shows.
    pub fn from_id(id: &str) -> Self {
        Self {
            id: id.trim().to_string(),
            capabilities: capabilities_from_name(id),
            ..Self::default()
        }
    }

    /// The name to show: the display name, else the id.
    pub fn label(&self) -> &str {
        let name = self.name.trim();
        if name.is_empty() { &self.id } else { name }
    }

    /// Whether it can do `c`.
    pub fn has(&self, c: Capability) -> bool {
        self.capabilities.contains(&c)
    }

    /// Whether the Assistant can chat with it.
    pub fn chats(&self) -> bool {
        chats(&self.capabilities)
    }

    /// Sorted capabilities without duplicates; a name equal to the id is dropped.
    pub fn normalize(&mut self) {
        self.id = self.id.trim().to_string();
        self.name = self.name.trim().to_string();
        if self.name == self.id {
            self.name.clear();
        }
        self.capabilities.sort();
        self.capabilities.dedup();
        if self.context_length == Some(0) {
            self.context_length = None;
        }
    }

    /// A short line about it: capabilities and context ("Vision · Reasoning · 128K").
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = self
            .capabilities
            .iter()
            .map(|c| c.label().to_string())
            .collect();
        if let Some(n) = self.context_length {
            parts.push(format_context(n));
        }
        parts.join(" · ")
    }
}

impl From<&str> for Model {
    fn from(id: &str) -> Self {
        Model::from_id(id)
    }
}

/// `131072` → "128K", `1047576` → "1M", `200000` → "200K", `80000` → "80K".
pub fn format_context(n: u64) -> String {
    let trim = |v: f64| {
        let t = format!("{v:.1}");
        t.strip_suffix(".0").map_or(t.clone(), str::to_string)
    };
    if n >= 1_000_000 {
        format!("{}M", trim(n as f64 / 1_000_000.0))
    } else if n >= 1024 && n.is_multiple_of(1024) {
        format!("{}K", n / 1024)
    } else if n >= 1000 {
        format!("{}K", trim(n as f64 / 1000.0))
    } else {
        n.to_string()
    }
}

/// "128K", "1M", "200000", "128 k" → tokens; `None` for anything else (or empty).
pub fn parse_context(text: &str) -> Option<u64> {
    let t = text.trim().replace([' ', ','], "").to_lowercase();
    if t.is_empty() {
        return None;
    }
    let (num, mul) = if let Some(n) = t.strip_suffix('k') {
        (n, 1024.0)
    } else if let Some(n) = t.strip_suffix('m') {
        (n, 1_048_576.0)
    } else {
        (t.as_str(), 1.0)
    };
    let v: f64 = num.parse().ok()?;
    (v.is_finite() && v > 0.0 && v * mul < 1e12).then(|| (v * mul).round() as u64)
}

/// Settings written before models had names were plain ids: both shapes read.
impl<'de> Deserialize<'de> for Model {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Shape {
            Id(String),
            Full {
                id: String,
                #[serde(default)]
                name: String,
                #[serde(default)]
                capabilities: Vec<Value>,
                #[serde(default)]
                context_length: Option<u64>,
            },
        }
        Ok(match Shape::deserialize(d)? {
            Shape::Id(id) => Model::from_id(&id),
            Shape::Full {
                id,
                name,
                capabilities,
                context_length,
            } => {
                // Tags this build does not know are dropped rather than failing the file.
                let capabilities = capabilities
                    .into_iter()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect();
                let mut m = Model {
                    id,
                    name,
                    capabilities,
                    context_length,
                };
                m.normalize();
                m
            }
        })
    }
}

/// The models in a `/models` answer (`{"data": [{"id": …}]}`, also Ollama's and Gemini's
/// `{"models": …}`), with display names, context lengths and capabilities. `trust_ids`: read
/// capabilities from the id when the item says nothing (off for SoCLaaS, whose ids are aliases
/// its operators can repoint; its display names and descriptions still count).
pub fn parse_models(body: &str, trust_ids: bool) -> Result<Vec<Model>, AiError> {
    let v: Value =
        serde_json::from_str(body).map_err(|_| err("the provider sent an unexpected answer"))?;
    let list = v
        .get("data")
        .or_else(|| v.get("models"))
        .and_then(Value::as_array)
        .ok_or_else(|| err("the provider sent no model list"))?;
    let mut out: Vec<Model> = Vec::new();
    for item in list {
        let Some(id) = item
            .get("id")
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)
            .map(|s| s.trim_start_matches("models/").trim())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if out.iter().any(|m| m.id == id) {
            continue;
        }
        out.push(model_from_item(id, item, trust_ids));
    }
    Ok(out)
}

/// One `/models` item as a [`Model`].
pub fn model_from_item(id: &str, item: &Value, trust_ids: bool) -> Model {
    let text = |k: &str| {
        item.pointer(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    // The catalogue's name: SoCLaaS's display name, Anthropic's display_name, OpenRouter's name
    // (Gemini's `name` is its id, "models/…", and is skipped).
    let name = text("/soclaas/display_name")
        .or_else(|| text("/display_name"))
        .or_else(|| text("/displayName"))
        .or_else(|| text("/name").filter(|n| !n.starts_with("models/") && *n != id))
        .unwrap_or("")
        .to_string();
    let context_length = [
        "/context_length",
        "/context_window",
        "/max_context_length",
        "/max_model_len",
        "/max_input_tokens",
        "/inputTokenLimit",
        "/top_provider/context_length",
    ]
    .iter()
    .find_map(|k| item.pointer(k).and_then(Value::as_u64).filter(|n| *n > 0));
    let stated = stated_capabilities(item);
    let mut names: Vec<Capability> = Vec::new();
    if trust_ids {
        names.extend(capabilities_from_name(id));
    }
    if !name.is_empty() {
        names.extend(capabilities_from_name(&name));
    }
    let mut caps: Vec<Capability> = match stated.modal {
        // The list says what goes in and out: names add only reasoning.
        Some(modal) => modal
            .into_iter()
            .chain(names.into_iter().filter(|c| *c == Capability::Reasoning))
            .collect(),
        None => names,
    };
    caps.extend(stated.extra);
    if chats(&caps) && !caps.contains(&Capability::Vision) && described_as_vision(item) {
        caps.push(Capability::Vision);
    }
    let mut m = Model {
        id: id.to_string(),
        name,
        capabilities: caps,
        context_length,
    };
    m.normalize();
    m
}

/// What an item states: what goes in and out (`modal`, `None` when it says nothing about that;
/// empty: text only) and the reasoning and tool flags (`extra`).
#[derive(Default)]
struct Stated {
    modal: Option<Vec<Capability>>,
    extra: Vec<Capability>,
}

fn words(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_lowercase)
            .collect(),
        Value::String(s) => vec![s.to_lowercase()],
        _ => Vec::new(),
    }
}

fn has(list: &[String], needles: &[&str]) -> bool {
    list.iter().any(|w| needles.iter().any(|n| w.contains(n)))
}

fn stated_capabilities(item: &Value) -> Stated {
    use Capability as C;
    let mut s = Stated::default();
    let mut extra = |c: C| s.extra.push(c);
    // Reasoning and tools: OpenRouter's supported_parameters.
    let params = item
        .get("supported_parameters")
        .map(words)
        .unwrap_or_default();
    if has(&params, &["tools", "tool_choice", "functions"]) {
        extra(C::ToolCalling);
    }
    if has(
        &params,
        &["reasoning", "include_reasoning", "reasoning_effort"],
    ) {
        extra(C::Reasoning);
    }
    // OpenRouter: architecture.{input,output}_modalities, or "text+image->text".
    let arch = item.get("architecture");
    let get = |k: &str| arch.and_then(|a| a.get(k)).map(words).unwrap_or_default();
    let (mut input, mut output) = (get("input_modalities"), get("output_modalities"));
    if let Some(m) = arch
        .and_then(|a| a.get("modality"))
        .and_then(Value::as_str)
        .map(str::to_lowercase)
    {
        let (i, o) = m.split_once("->").unwrap_or((m.as_str(), ""));
        input.push(i.to_string());
        output.push(o.to_string());
    }
    if !input.is_empty() || !output.is_empty() {
        let mut modal = Vec::new();
        if has(&output, &["embed", "vector"]) {
            modal.push(C::Embedding);
        }
        if has(&output, &["image"]) {
            modal.push(C::ImageGen);
        }
        if has(&input, &["image", "vision", "video"]) {
            modal.push(C::Vision);
        }
        if has(&input, &["audio"]) || has(&output, &["audio"]) {
            modal.push(C::Audio);
        }
        s.modal = Some(modal);
        return s;
    }
    // Ollama's /api/show and others: capabilities ["vision", "tools", "thinking", …]; SoCLaaS:
    // soclaas.capabilities ["chat"], ["embeddings"], ["audio-transcription"]; OpenAI-style:
    // modalities ["text", "image"].
    let caps: Vec<String> = [
        "/capabilities",
        "/modalities",
        "/input_modalities",
        "/soclaas/capabilities",
    ]
    .iter()
    .flat_map(|k| item.pointer(k).map(words).unwrap_or_default())
    .collect();
    if !caps.is_empty() {
        let mut modal = Vec::new();
        if has(&caps, &["embed", "rerank"]) {
            modal.push(C::Embedding);
        }
        if has(
            &caps,
            &[
                "image-gen",
                "image_gen",
                "imagegen",
                "text-to-image",
                "image generation",
            ],
        ) {
            modal.push(C::ImageGen);
        } else if has(&caps, &["vision", "image"]) {
            modal.push(C::Vision);
        }
        if has(&caps, &["audio", "speech", "transcri"]) {
            modal.push(C::Audio);
        }
        if has(&caps, &["tool", "function"]) {
            s.extra.push(C::ToolCalling);
        }
        if has(&caps, &["thinking", "reason"]) {
            s.extra.push(C::Reasoning);
        }
        // "chat" or "completion" alone says nothing about images: names decide then.
        s.modal = (!modal.is_empty()).then_some(modal);
        return s;
    }
    // LiteLLM-style model info: mode "embedding" / "audio_transcription", supports_vision.
    let mode = item
        .get("mode")
        .or_else(|| item.pointer("/model_info/mode"))
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    let flag = |k: &str| {
        item.get(k)
            .or_else(|| item.pointer(&format!("/model_info/{k}")))
            .and_then(Value::as_bool)
    };
    let vision = flag("supports_vision");
    if mode.is_some() || vision.is_some() {
        let mut modal = Vec::new();
        match mode.as_deref() {
            Some(m) if m.contains("embed") || m.contains("rerank") => modal.push(C::Embedding),
            Some(m) if m.contains("audio") || m.contains("speech") => modal.push(C::Audio),
            Some(m) if m.contains("image_generation") => modal.push(C::ImageGen),
            _ => {}
        }
        if vision == Some(true) {
            modal.push(C::Vision);
        }
        if flag("supports_function_calling") == Some(true) {
            s.extra.push(C::ToolCalling);
        }
        if flag("supports_reasoning") == Some(true) {
            s.extra.push(C::Reasoning);
        }
        s.modal = Some(modal);
    }
    s
}

/// A chat model whose catalogue entry (its operator's description, not the id) says it reads
/// images: SoCLaaS lists `qwen3-vl:32b` as "chat", described "Vision-capable model".
fn described_as_vision(item: &Value) -> bool {
    let text = |k: &str| item.pointer(k).and_then(Value::as_str).unwrap_or_default();
    let description = format!("{} {}", text("/description"), text("/soclaas/description"));
    let description = description.to_lowercase();
    [
        "vision",
        "image input",
        "image understanding",
        "multimodal",
        "multi-modal",
    ]
    .iter()
    .any(|w| description.contains(w))
}

/// What Ollama's `/api/show` says a model can do (vision, tools, thinking, embedding).
pub fn capabilities_from_show(show: &Value) -> Option<Vec<Capability>> {
    let caps = show.get("capabilities").map(words)?;
    let mut out = Vec::new();
    for c in &caps {
        match c.as_str() {
            "vision" => out.push(Capability::Vision),
            "tools" => out.push(Capability::ToolCalling),
            "thinking" => out.push(Capability::Reasoning),
            "embedding" => out.push(Capability::Embedding),
            _ => {}
        }
    }
    out.sort();
    out.dedup();
    Some(out)
}

/// The capabilities a model's name shows (`text-embedding-3-small`, `qwen3-vl:32b`,
/// `whisper-1`, `deepseek-reasoner`, "Qwen 3 VL 32B", …); none when it shows nothing.
pub fn capabilities_from_name(name: &str) -> Vec<Capability> {
    use Capability as C;
    let full = name.to_lowercase();
    // The model's own name, after any vendor prefix.
    let m = full.rsplit('/').next().unwrap_or(&full);
    let any = |needles: &[&str]| needles.iter().any(|n| m.contains(n));
    let starts = |needles: &[&str]| needles.iter().any(|n| m.starts_with(n));
    let mut out = Vec::new();
    if any(&[
        "embed",
        "bge-",
        "bge:",
        "gte-",
        "e5-",
        "voyage-",
        "rerank",
        "jina-clip",
        "minilm",
    ]) || m == "bge"
    {
        return vec![C::Embedding];
    }
    if any(&[
        "whisper",
        "tts",
        "transcribe",
        "speech",
        "sensevoice",
        "paraformer",
        "-asr",
        "asr-",
        "voice",
        "audio",
    ]) {
        out.push(C::Audio);
    }
    if any(&[
        "dall-e",
        "gpt-image",
        "imagen",
        "flux",
        "stable-diffusion",
        "sdxl",
        "sd3",
        "seedream",
        "cogview",
        "wanx",
        "kolors",
        "midjourney",
        "image-gen",
    ]) {
        return vec![C::ImageGen];
    }
    // Vision: names that say so ("qwen3-vl", "Qwen 3 VL 32B"), then families that all read
    // images.
    let says_vision = m
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| matches!(w, "vl" | "vision" | "omni" | "multimodal"))
        || any(&[
            "-vl",
            "vl-",
            "vl:",
            "llava",
            "pixtral",
            "qvq",
            "minicpm-v",
            "internvl",
            "moondream",
            "-4v",
            "4.1v",
            "4.5v",
            "5v",
        ])
        || m.ends_with("vl");
    let family = any(&[
        "gemini",
        "gpt-4o",
        "gpt-4.1",
        "gpt-5",
        "chatgpt-4o",
        "llama4",
        "llama-4",
    ]) || (starts(&["o1", "o3"]) && !m.contains("mini"))
        || starts(&["o4"])
        || (starts(&["claude"]) && !starts(&["claude-2", "claude-instant"]))
        || starts(&[
            "gemma3",
            "gemma-3",
            "gemma4",
            "gemma-4",
            "grok-4",
            "mistral-medium-3",
        ])
        || any(&[
            "mistral-small-3.1",
            "mistral-small-3.2",
            "kimi-k2.5",
            "kimi-latest",
        ]);
    if (says_vision || family) && !out.contains(&C::Audio) {
        out.push(C::Vision);
    }
    // Reasoning: Argus's markers, on the full name (vendor prefixes included).
    let reasons = [
        "reasoning",
        "reasoner",
        "thinking",
        "/r1",
        "-r1",
        "/o1",
        "-o1",
        "/o3",
        "-o3",
        "/o4",
        "-o4",
        "qwq",
    ]
    .iter()
    .any(|n| full.contains(n))
        || starts(&["o1", "o3", "o4", "r1"]);
    if reasons {
        out.push(C::Reasoning);
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn caps(list: &[Capability]) -> Vec<Capability> {
        let mut v = list.to_vec();
        v.sort();
        v
    }

    #[test]
    fn names_show_their_capabilities() {
        use Capability as C;
        for (id, want) in [
            ("bge-m3", vec![C::Embedding]),
            ("text-embedding-3-small", vec![C::Embedding]),
            ("nomic-embed-text:latest", vec![C::Embedding]),
            ("whisper-large-v3", vec![C::Audio]),
            ("gpt-4o-mini-tts", vec![C::Audio]),
            ("dall-e-3", vec![C::ImageGen]),
            ("black-forest-labs/flux-1.1-pro", vec![C::ImageGen]),
            ("qwen3-vl:32b", vec![C::Vision]),
            ("Qwen/Qwen2.5-VL-72B-Instruct", vec![C::Vision]),
            ("Qwen 3 VL 32B", vec![C::Vision]),
            ("gemma4:26b", vec![C::Vision]),
            ("gpt-4.1-mini", vec![C::Vision]),
            ("claude-opus-5-5", vec![C::Vision]),
            ("google/gemini-2.5-pro", vec![C::Vision]),
            ("glm-4.5v", vec![C::Vision]),
            ("o4-mini", vec![C::Vision, C::Reasoning]),
            ("o3-mini", vec![C::Reasoning]),
            ("deepseek-reasoner", vec![C::Reasoning]),
            ("deepseek/deepseek-r1", vec![C::Reasoning]),
            ("qwq-32b", vec![C::Reasoning]),
            ("llama3.1:8b", vec![]),
            ("deepseek-chat", vec![]),
            ("qwen3-coder-next", vec![]),
            ("gpt-oss-120b", vec![]),
            ("gpt-3.5-turbo", vec![]),
        ] {
            assert_eq!(capabilities_from_name(id), caps(&want), "{id}");
        }
    }

    #[test]
    fn items_state_their_capabilities() {
        use Capability as C;
        let model = |item: Value| model_from_item("m", &item, true);
        // Ollama's /api/show and others.
        assert_eq!(
            model(json!({"capabilities": ["vision", "tool_calling"]})).capabilities,
            caps(&[C::Vision, C::ToolCalling])
        );
        assert_eq!(
            model(json!({"capabilities": ["completion", "tools", "thinking"]})).capabilities,
            caps(&[C::ToolCalling, C::Reasoning])
        );
        assert_eq!(
            model(json!({"capabilities": ["embedding"]})).capabilities,
            vec![C::Embedding]
        );
        // OpenRouter: modalities, parameters, name, context.
        let or = model(
            json!({"id": "x/y", "name": "X: Y", "context_length": 131072,
            "architecture": {"modality": "text+image->text",
                "input_modalities": ["text", "image"], "output_modalities": ["text"]},
            "supported_parameters": ["tools", "reasoning", "max_tokens"]}),
        );
        assert_eq!(
            or.capabilities,
            caps(&[C::Vision, C::Reasoning, C::ToolCalling])
        );
        assert_eq!(
            (or.name.as_str(), or.context_length),
            ("X: Y", Some(131072))
        );
        assert_eq!(
            model(json!({"architecture": {"modality": "text->text"}})).capabilities,
            vec![]
        );
        assert_eq!(
            model(json!({"architecture": {"input_modalities": ["text"],
                "output_modalities": ["image"]}}))
            .capabilities,
            vec![C::ImageGen]
        );
        // OpenAI-style modalities, LiteLLM model info.
        assert_eq!(
            model(json!({"modalities": ["text", "image"]})).capabilities,
            vec![C::Vision]
        );
        assert_eq!(
            model(json!({"model_info": {"mode": "embedding"}})).capabilities,
            vec![C::Embedding]
        );
        assert_eq!(
            model(json!({"model_info": {"mode": "chat", "supports_vision": true}})).capabilities,
            vec![C::Vision]
        );
        // Anthropic's catalogue names.
        let claude = model_from_item(
            "claude-opus-5-5",
            &json!({"id": "claude-opus-5-5", "display_name": "Claude Opus 5.5"}),
            true,
        );
        assert_eq!(claude.label(), "Claude Opus 5.5");
        assert_eq!(claude.capabilities, vec![C::Vision]);
        // Gemini's name is its id.
        let gemini = model_from_item("gemini-x", &json!({"name": "models/gemini-x"}), true);
        assert_eq!(gemini.label(), "gemini-x");
    }

    /// The shape of SoCLaaS's catalogue (2026-10-17): the kind sits in `soclaas.capabilities`,
    /// a vision model is "chat" with an operator description saying so; ids are aliases.
    #[test]
    fn soclaas_catalogue_entries() {
        use Capability as C;
        let entry = |id: &str, caps: &[&str], description: &str, name: &str| {
            json!({"id": id, "object": "model", "owned_by": "soclaas", "context_length": 131072,
                "soclaas": {"capabilities": caps, "description": description, "display_name": name}})
        };
        let body = json!({"object": "list", "data": [
            entry("bge-m3", &["embeddings"], "", "bge-m3"),
            entry("qwen3.6:35b", &["chat"], "Efficient agentic workflows and advanced coding",
                "Qwen 3.6 35B A3B"),
            entry("qwen3-vl:32b", &["chat"], "Vision-capable model", "Qwen 3 VL 32B"),
            entry("whisper-large-v3", &["audio-transcription"], "For audio transcription only",
                "Whisper Large v3"),
            entry("coder", &["chat"], "High-quality general reasoning and multimodal tasks",
                "Qwen 3.8 27B"),
            entry("alias-vl", &["chat"], "", "Something else"),
            entry("painter", &["image-generation"], "Generates images", "Painter"),
        ]})
        .to_string();
        let models = parse_models(&body, false).unwrap();
        let got: Vec<(&str, &str, Vec<Capability>)> = models
            .iter()
            .map(|m| (m.id.as_str(), m.label(), m.capabilities.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("bge-m3", "bge-m3", vec![C::Embedding]),
                ("qwen3.6:35b", "Qwen 3.6 35B A3B", vec![]),
                ("qwen3-vl:32b", "Qwen 3 VL 32B", vec![C::Vision]),
                ("whisper-large-v3", "Whisper Large v3", vec![C::Audio]),
                ("coder", "Qwen 3.8 27B", vec![C::Vision]),
                // The alias says vision, but aliases are not read.
                ("alias-vl", "Something else", vec![]),
                ("painter", "Painter", vec![C::ImageGen]),
            ]
        );
        assert_eq!(models[0].context_length, Some(131072));
        assert!(!models[0].chats() && models[1].chats() && !models[3].chats());
    }

    #[test]
    fn lists_read_and_old_settings_migrate() {
        let body =
            r#"{"data":[{"id":"qwen3-vl:32b"},{"id":"bge-m3"},{"id":"bge-m3"},{"id":"plain"}]}"#;
        let ids: Vec<String> = parse_models(body, true)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec!["qwen3-vl:32b", "bge-m3", "plain"]);
        assert!(parse_models("nope", true).is_err());
        assert!(parse_models(r#"{"object":"list"}"#, true).is_err());
        // Plain ids (settings from before) and full entries both read; unknown tags are dropped.
        let models: Vec<Model> = serde_json::from_str(
            r#"["qwen3-vl:32b", {"id": "m", "name": " My Model ", "capabilities":
                ["reasoning", "vision", "teleport", "vision"], "context_length": 0}]"#,
        )
        .unwrap();
        assert_eq!(models[0].capabilities, vec![Capability::Vision]);
        assert_eq!(models[1].label(), "My Model");
        assert_eq!(
            models[1].capabilities,
            vec![Capability::Vision, Capability::Reasoning]
        );
        assert_eq!(models[1].context_length, None);
        // Written back as objects, by Argus's tag names.
        let json = serde_json::to_value(&models[1]).unwrap();
        assert_eq!(json["capabilities"], json!(["vision", "reasoning"]));
        assert!(json.get("context_length").is_none());
        let plain = serde_json::to_value(Model::from_id("plain")).unwrap();
        assert_eq!(plain, json!({"id": "plain"}));
    }

    #[test]
    fn context_lengths_read_and_print() {
        assert_eq!(parse_context("128K"), Some(131072));
        assert_eq!(parse_context("1m"), Some(1_048_576));
        assert_eq!(parse_context(" 200,000 "), Some(200_000));
        assert_eq!(parse_context(""), None);
        assert_eq!(parse_context("lots"), None);
        assert_eq!(parse_context("-5"), None);
        assert_eq!(format_context(131072), "128K");
        assert_eq!(format_context(1_047_576), "1M");
        assert_eq!(format_context(2_000_000), "2M");
        assert_eq!(format_context(80_000), "80K");
        assert_eq!(format_context(200_000), "200K");
        assert_eq!(format_context(512), "512");
    }

    #[test]
    fn chat_menus_leave_out_non_chat_models() {
        use Capability as C;
        assert!(chats(&[]));
        assert!(chats(&[C::Vision]));
        assert!(chats(&[C::Audio, C::Reasoning]));
        assert!(!chats(&[C::Embedding]));
        assert!(!chats(&[C::Audio]));
        assert!(!chats(&[C::ImageGen]));
        let m = Model {
            id: "m".into(),
            capabilities: vec![C::Vision, C::Reasoning],
            context_length: Some(131072),
            ..Model::default()
        };
        assert_eq!(m.summary(), "Vision · Reasoning · 128K");
    }
}
