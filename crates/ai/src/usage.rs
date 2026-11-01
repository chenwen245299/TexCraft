//! What a request used, as the provider reports it at the end of a stream, and Argus's estimate
//! of how many tokens a request takes before it is sent (for the context meter).
//!
//! Usage comes from Anthropic's `message_start` / `message_delta` and from the `usage` object an
//! OpenAI-compatible stream sends last (asked for with `stream_options.include_usage`). Cached
//! prompt tokens and a billed cost are kept only when the provider names them: a figure it left
//! out is unknown, never zero.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{ChatRequest, Role, Turn};

/// What one request used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// Prompt tokens, cache reads and writes included.
    pub input: u64,
    /// Answer tokens (reasoning included).
    pub output: u64,
    /// Prompt tokens read from the provider's cache (`None`: the provider did not say).
    pub cache_hit: Option<u64>,
    /// What the provider billed, in US dollars (OpenRouter's `usage.cost`).
    pub cost_usd: Option<f64>,
}

impl Usage {
    /// An OpenAI-compatible stream's `usage` object: `prompt_tokens`, `completion_tokens`, cache
    /// hits where DeepSeek (`prompt_cache_hit_tokens`), OpenAI, Kimi and OpenRouter
    /// (`prompt_tokens_details.cached_tokens`) or StepFun (`cached_tokens`) put them.
    pub(crate) fn from_openai(u: &Value) -> Option<Self> {
        if !u.is_object() {
            return None;
        }
        let input = u.get("prompt_tokens").and_then(Value::as_u64);
        let output = u.get("completion_tokens").and_then(Value::as_u64);
        if input.is_none() && output.is_none() {
            return None;
        }
        let cache_hit = u
            .get("prompt_cache_hit_tokens")
            .and_then(Value::as_u64)
            .or_else(|| u.pointer("/prompt_tokens_details/cached_tokens")?.as_u64())
            .or_else(|| u.get("cached_tokens").and_then(Value::as_u64));
        Some(Self {
            input: input.unwrap_or(0),
            output: output.unwrap_or(0),
            cache_hit,
            cost_usd: cost(u),
        })
    }

    /// Folds in an Anthropic `usage` object (`message_start`'s, then `message_delta`'s running
    /// totals): the prompt is `input_tokens` plus the cache's reads and writes.
    pub(crate) fn merge_anthropic(&mut self, u: &Value) {
        let get = |k: &str| u.get(k).and_then(Value::as_u64);
        let read = get("cache_read_input_tokens");
        if let Some(base) = get("input_tokens") {
            let write = get("cache_creation_input_tokens").unwrap_or(0);
            self.input = base.saturating_add(read.unwrap_or(0)).saturating_add(write);
        }
        if read.is_some() {
            self.cache_hit = read;
        }
        if let Some(out) = get("output_tokens") {
            self.output = out;
        }
    }
}

/// A billed cost (a number, or a number in a string), when it is a real amount.
fn cost(u: &Value) -> Option<f64> {
    let v = u.get("cost")?;
    let n = v
        .as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))?;
    (n.is_finite() && n >= 0.0).then_some(n)
}

/// Argus's token estimate: a wide (non-ASCII) character is a token, so are four ASCII ones.
pub fn estimate_text(text: &str) -> u64 {
    let (mut ascii, mut wide) = (0u64, 0u64);
    for c in text.chars() {
        if c.is_ascii() {
            ascii = ascii.saturating_add(1);
        } else {
            wide = wide.saturating_add(1);
        }
    }
    wide.saturating_add(ascii.div_ceil(4))
}

/// What an attached image is taken to cost (Argus's figure for a full-detail image).
pub const IMAGE_TOKENS: u64 = 2048;
/// The overhead of one message.
const MESSAGE_TOKENS: u64 = 4;

/// How many tokens a request's parts take, estimated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Estimate {
    /// The system prompt (with the document when it goes along).
    pub system: u64,
    /// The tools' definitions.
    pub tools: u64,
    /// The conversation: questions, answers, tool calls and their results.
    pub messages: u64,
    /// Images are part of it (their share is a guess).
    pub images: bool,
}

impl Estimate {
    /// All of it.
    pub fn total(&self) -> u64 {
        self.system
            .saturating_add(self.tools)
            .saturating_add(self.messages)
    }
}

/// The estimate of one turn of the conversation.
pub fn estimate_turn(t: &Turn) -> u64 {
    let mut n = MESSAGE_TOKENS;
    if t.role == Role::User && !t.tool_results.is_empty() {
        // One message per result.
        for r in &t.tool_results {
            n = n
                .saturating_add(MESSAGE_TOKENS)
                .saturating_add(estimate_text(&r.content));
        }
        return n;
    }
    if t.blocks.is_empty() {
        n = n.saturating_add(estimate_text(&t.text));
    } else {
        for b in &t.blocks {
            let field = |k: &str| b.get(k).and_then(Value::as_str).map_or(0, estimate_text);
            let input = b.get("input").map_or(0, |i| estimate_text(&i.to_string()));
            n = n
                .saturating_add(MESSAGE_TOKENS)
                .saturating_add(field("text"))
                .saturating_add(field("thinking"))
                .saturating_add(input);
        }
    }
    if !t.images.is_empty() {
        let images = u64::try_from(t.images.len()).unwrap_or(u64::MAX);
        n = n
            .saturating_add(MESSAGE_TOKENS)
            .saturating_add(images.saturating_mul(IMAGE_TOKENS));
    }
    if !t.tool_calls.is_empty() {
        let calls: Vec<Value> = t
            .tool_calls
            .iter()
            .map(|c| {
                json!({"id": c.id, "type": "function",
                    "function": {"name": c.name, "arguments": c.arguments}})
            })
            .collect();
        n = n.saturating_add(estimate_text(&Value::Array(calls).to_string()));
    }
    n.saturating_add(estimate_text(&t.reasoning))
}

/// The estimate of `req` as it would be sent.
pub fn estimate(req: &ChatRequest) -> Estimate {
    let tools = if req.tools.is_empty() {
        0
    } else {
        let specs: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({"type": "function", "function": {"name": t.name,
                    "description": t.description, "parameters": t.parameters}})
            })
            .collect();
        estimate_text(&Value::Array(specs).to_string()).saturating_add(MESSAGE_TOKENS)
    };
    Estimate {
        system: MESSAGE_TOKENS.saturating_add(estimate_text(&req.system)),
        tools,
        messages: req
            .turns
            .iter()
            .map(estimate_turn)
            .fold(0, u64::saturating_add),
        images: req.turns.iter().any(|t| !t.images.is_empty()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_usage_finds_cache_hits_where_each_provider_puts_them() {
        let deepseek = json!({"prompt_tokens": 1200, "completion_tokens": 80,
            "prompt_cache_hit_tokens": 1024, "prompt_cache_miss_tokens": 176});
        let u = Usage::from_openai(&deepseek).unwrap();
        assert_eq!((u.input, u.output, u.cache_hit), (1200, 80, Some(1024)));
        let openai = json!({"prompt_tokens": 50, "completion_tokens": 5,
            "prompt_tokens_details": {"cached_tokens": 0}, "cost": "0.0012"});
        let u = Usage::from_openai(&openai).unwrap();
        assert_eq!(u.cache_hit, Some(0), "a reported miss is a figure");
        assert_eq!(u.cost_usd, Some(0.0012));
        let stepfun = json!({"prompt_tokens": 9, "completion_tokens": 1, "cached_tokens": 4});
        assert_eq!(Usage::from_openai(&stepfun).unwrap().cache_hit, Some(4));
        let plain = json!({"prompt_tokens": 9, "completion_tokens": 1, "cost": -1});
        let u = Usage::from_openai(&plain).unwrap();
        assert_eq!((u.cache_hit, u.cost_usd), (None, None), "unknown, not zero");
        assert_eq!(Usage::from_openai(&Value::Null), None);
        assert_eq!(Usage::from_openai(&json!({"total_tokens": 3})), None);
    }

    #[test]
    fn anthropic_usage_counts_cache_reads_and_writes_in_the_prompt() {
        let mut u = Usage::default();
        u.merge_anthropic(&json!({"input_tokens": 10, "cache_read_input_tokens": 900,
            "cache_creation_input_tokens": 90, "output_tokens": 1}));
        assert_eq!((u.input, u.output, u.cache_hit), (1000, 1, Some(900)));
        // message_delta: the running output total.
        u.merge_anthropic(&json!({"output_tokens": 321}));
        assert_eq!((u.input, u.output, u.cache_hit), (1000, 321, Some(900)));
    }

    #[test]
    fn estimates_like_argus() {
        assert_eq!(estimate_text(""), 0);
        assert_eq!(estimate_text("abcd"), 1);
        assert_eq!(estimate_text("abcde"), 2);
        assert_eq!(estimate_text("上下文"), 3);
        assert_eq!(estimate_text("ab上下"), 3);
        let turn = |role, text: &str| Turn {
            role,
            text: text.into(),
            ..Default::default()
        };
        assert_eq!(estimate_turn(&turn(Role::User, "abcdefgh")), 4 + 2);
        let mut with_image = turn(Role::User, "abcd");
        with_image.images.push(crate::Image {
            media_type: "image/png".into(),
            data: std::sync::Arc::from(vec![0u8; 4]),
            name: "a.png".into(),
        });
        assert_eq!(estimate_turn(&with_image), 4 + 1 + 4 + IMAGE_TOKENS);
        let results = Turn {
            role: Role::User,
            tool_results: vec![crate::ToolResult {
                id: "1".into(),
                name: "read_file".into(),
                content: "abcdefgh".into(),
            }],
            ..Default::default()
        };
        assert_eq!(estimate_turn(&results), 4 + 4 + 2);
        // Anthropic blocks replace the text.
        let mut blocks = turn(Role::Assistant, "ignored text");
        blocks.blocks = vec![
            json!({"type": "thinking", "thinking": "abcd", "signature": "long signature"}),
            json!({"type": "text", "text": "abcdefgh"}),
        ];
        assert_eq!(estimate_turn(&blocks), 4 + (4 + 1) + (4 + 2));
    }
}
