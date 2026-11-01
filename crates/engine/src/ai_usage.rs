//! The Assistant's running statistics and context meter, counted as Argus counts them for the
//! row under its question box: turns and steps, the latest answer's speed, the tokens the
//! conversation used and the share the provider's cache served, what it cost when the provider
//! billed it, and how full the model's context window is.
//!
//! Each answer is a run of requests (one, or one per round of tool calls). A run adds up what
//! the provider reported for its requests; a figure no provider reported stays unknown, and the
//! panel leaves out what it cannot show truthfully (no cache rate unless every measured request
//! reported its cache hits, no cost unless every measured answer was billed). The context is
//! estimated from the request's text (Argus's estimate) and calibrated with the prompt size the
//! provider last counted for the same model.

use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use texcraft_ai::usage::{self, Estimate};
use texcraft_ai::{ChatRequest, Role, Turn, Usage};

use crate::Session;

/// One answer's requests (Argus's "run").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunUsage {
    /// The answer (its index in the conversation).
    pub message: usize,
    /// Requests sent to the model (steps).
    pub calls: u32,
    /// Tools the model called.
    pub tool_calls: u32,
    /// Requests whose usage the provider reported.
    pub measured: u32,
    /// Prompt tokens (cache hits included).
    pub input: u64,
    /// Answer tokens.
    pub output: u64,
    /// Prompt tokens the provider's cache served.
    pub cache_hit: u64,
    /// Every measured request reported its cache hits (a miss included).
    pub cache_reported: bool,
    /// What the provider billed, in US dollars (`None`: no request was billed).
    pub cost_usd: Option<f64>,
    /// Time spent on the model's requests, tools excluded (ms).
    pub model_ms: u64,
}

impl RunUsage {
    /// Folds in one request's usage.
    pub fn add(&mut self, u: &Usage) {
        // An unknown cache figure cannot pass for a measured miss.
        self.cache_reported = (self.measured == 0 || self.cache_reported) && u.cache_hit.is_some();
        self.measured = self.measured.saturating_add(1);
        self.input = self.input.saturating_add(u.input);
        self.output = self.output.saturating_add(u.output);
        self.cache_hit = self.cache_hit.saturating_add(u.cache_hit.unwrap_or(0));
        if let Some(c) = u.cost_usd {
            *self.cost_usd.get_or_insert(0.0) += c;
        }
    }
}

/// A prompt the provider counted: `input` tokens for a request estimated at `estimated`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PromptSample {
    /// The provider's id.
    pub provider: String,
    /// The model.
    pub model: String,
    /// The estimate of the request.
    pub estimated: u64,
    /// What the provider counted.
    pub input: u64,
}

/// What a conversation used: its runs and the last measured prompt (saved with it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatUsage {
    /// One per answer, in order.
    pub runs: Vec<RunUsage>,
    /// The last prompt the provider counted.
    pub sample: Option<PromptSample>,
}

/// The row under the question box.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStats {
    /// Questions asked.
    pub turns: usize,
    /// Requests sent to the model, tool rounds included.
    pub steps: u32,
    /// Tools the model called.
    pub tool_calls: u32,
    /// The latest answer's output speed (tokens a second, model time only).
    pub speed: Option<u32>,
    /// Prompt tokens of all measured requests.
    pub input: u64,
    /// Answer tokens of all measured requests.
    pub output: u64,
    /// Both (`None`: no request reported usage).
    pub tokens: Option<u64>,
    /// Prompt tokens the cache served, in percent (`None`: not every request reported it).
    pub cache_percent: Option<u32>,
    /// What the conversation cost, in US dollars (`None`: not every measured answer was billed).
    pub cost_usd: Option<f64>,
}

/// The statistics of a conversation of `turns` questions and `runs`.
pub fn summarize(turns: usize, runs: &[RunUsage]) -> ChatStats {
    let mut s = ChatStats {
        turns,
        ..ChatStats::default()
    };
    let (mut measured, mut cache_input, mut cache_hits, mut cache_complete) =
        (0u32, 0u64, 0u64, true);
    let (mut cost, mut billed, mut unbilled) = (0.0f64, 0u32, 0u32);
    for r in runs {
        s.steps = s.steps.saturating_add(r.calls);
        s.tool_calls = s.tool_calls.saturating_add(r.tool_calls);
        if r.measured > 0 {
            measured = measured.saturating_add(1);
            s.input = s.input.saturating_add(r.input);
            s.output = s.output.saturating_add(r.output);
        }
        if r.measured > 0 && r.input > 0 {
            if r.cache_reported {
                cache_input = cache_input.saturating_add(r.input);
                cache_hits = cache_hits.saturating_add(r.cache_hit.min(r.input));
            } else {
                cache_complete = false;
            }
        }
        match r.cost_usd {
            Some(c) if c.is_finite() && c >= 0.0 => {
                cost += c;
                billed = billed.saturating_add(1);
            }
            _ if r.measured > 0 => unbilled = unbilled.saturating_add(1),
            _ => {}
        }
    }
    if measured > 0 {
        s.tokens = Some(s.input.saturating_add(s.output));
    }
    if cache_complete && cache_input > 0 {
        s.cache_percent = Some(percent(cache_hits, cache_input));
    }
    if billed > 0 && unbilled == 0 {
        s.cost_usd = Some(cost);
    }
    // The latest answer that reported its output and took model time.
    s.speed = runs
        .iter()
        .rev()
        .find(|r| r.measured > 0 && r.model_ms > 0)
        .map(|r| r.output.saturating_mul(1000).saturating_add(r.model_ms / 2) / r.model_ms)
        .filter(|v| *v > 0)
        .map(|v| u32::try_from(v).unwrap_or(u32::MAX));
    s
}

/// `part` of `whole` in whole percent, rounded, at most 100.
fn percent(part: u64, whole: u64) -> u32 {
    if whole == 0 {
        return 0;
    }
    let p = part.saturating_mul(100).saturating_add(whole / 2) / whole;
    u32::try_from(p.min(100)).unwrap_or(100)
}

/// The window assumed when the model's is not known (Argus's figure).
pub const ASSUMED_WINDOW: u64 = 256_000;

/// How full the model's context window is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    /// Tokens in use.
    pub used: u64,
    /// The model's window.
    pub window: u64,
    /// Tokens left.
    pub remaining: u64,
    /// The system prompt with the open document.
    pub system: u64,
    /// The tools' definitions.
    pub tools: u64,
    /// The conversation and the tools' results.
    pub messages: u64,
    /// `used` of `window`, in percent.
    pub percent: u32,
    /// `used` is an estimate (no measured prompt, or the conversation changed since).
    pub estimated: bool,
    /// The model's window is not known ([`ASSUMED_WINDOW`] is assumed).
    pub window_assumed: bool,
    /// Images are part of it (their share is a guess).
    pub images: bool,
}

/// The context of a request estimated at `est`, for a model whose window is `window`, calibrated
/// with the prompt the provider last counted (`sample`, for the same model).
pub fn measure(est: &Estimate, window: Option<u64>, sample: Option<&PromptSample>) -> ContextUsage {
    let total = est.total();
    let used = sample.map_or(total, |s| {
        s.input.saturating_add(total).saturating_sub(s.estimated)
    });
    let (window, window_assumed) = match window.filter(|n| *n > 0) {
        Some(n) => (n, false),
        None => (ASSUMED_WINDOW, true),
    };
    ContextUsage {
        used,
        window,
        remaining: window.saturating_sub(used),
        system: est.system,
        tools: est.tools,
        messages: est.messages,
        percent: percent(used, window),
        estimated: sample.is_none_or(|s| s.estimated != total),
        window_assumed,
        images: est.images,
    }
}

impl Session {
    /// The conversation's running statistics.
    pub fn ai_stats(&self) -> ChatStats {
        let turns = self
            .ai
            .messages
            .iter()
            .filter(|m| m.role == Role::User)
            .count();
        summarize(turns, &self.ai.usage.runs)
    }

    /// How full the model's context is: the request being answered, else the next question's
    /// (with the draft). `None` without a model.
    pub fn ai_context_now(&self) -> Option<ContextUsage> {
        let (est, provider, model) = match self.ai.round_request() {
            Some(req) => (
                usage::estimate(req),
                req.provider.clone(),
                req.model.clone(),
            ),
            None => {
                let (p, m) = self.ai.config.selection()?;
                let (provider, model) = (p.clone(), m.to_string());
                (self.ai_next_estimate(&provider, &model), provider, model)
            }
        };
        let window = provider
            .models
            .iter()
            .find(|m| m.id == model)
            .and_then(|m| m.context_length);
        let sample = self
            .ai
            .usage
            .sample
            .as_ref()
            .filter(|s| s.provider == provider.id && s.model == model);
        Some(measure(&est, window, sample))
    }

    /// [`Self::ai_context_now`], measured again only when what it depends on changed (the panel
    /// asks every frame; the document can be long).
    pub fn ai_context(&mut self) -> Option<ContextUsage> {
        let key = self.ai_context_key();
        if let Some((k, v)) = &self.ai.context_cache
            && *k == key
        {
            return v.clone();
        }
        let v = self.ai_context_now();
        self.ai.context_cache = Some((key, v.clone()));
        v
    }

    /// The estimate of the next question's request: the conversation, then the draft.
    fn ai_next_estimate(&self, provider: &texcraft_ai::Provider, model: &str) -> Estimate {
        let (system, tools, _) = self.ai_prompt();
        let mut turns = self.ai_turns(model);
        let draft = self.ai.draft.trim();
        if !draft.is_empty() || !self.ai.draft_images.is_empty() {
            turns.push(Turn {
                role: Role::User,
                text: draft.to_string(),
                images: self.ai.draft_images.clone(),
                ..Default::default()
            });
        }
        usage::estimate(&ChatRequest {
            provider: provider.clone(),
            model: model.to_string(),
            system,
            api_key: String::new(),
            turns,
            max_tokens: None,
            no_reasoning: false,
            tools,
        })
    }

    /// What the context depends on, hashed.
    fn ai_context_key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let sel = self.ai.config.selection();
        sel.map(|(p, m)| (&p.id, m)).hash(&mut h);
        sel.and_then(|(p, m)| p.models.iter().find(|x| x.id == m))
            .and_then(|m| m.context_length)
            .hash(&mut h);
        self.ai.usage.sample.hash(&mut h);
        if let Some(req) = self.ai.round_request() {
            (1u8, req.turns.len(), req.tools.len(), req.system.len()).hash(&mut h);
            return h.finish();
        }
        let include = self.ai.config.include_document;
        include.hash(&mut h);
        if include && let Some(d) = self.doc() {
            (
                self.active,
                d.revision,
                d.selection.start(),
                d.selection.end(),
            )
                .hash(&mut h);
        }
        self.git_dir().hash(&mut h);
        self.ai.messages.len().hash(&mut h);
        if let Some(m) = self.ai.messages.last() {
            (m.text.len(), m.pending, m.blocks.len(), m.error.is_some()).hash(&mut h);
        }
        (self.ai.draft.len(), self.ai.draft_images.len()).hash(&mut h);
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, output: u64, cache_hit: Option<u64>, cost: Option<f64>) -> Usage {
        Usage {
            input,
            output,
            cache_hit,
            cost_usd: cost,
        }
    }

    #[test]
    fn runs_add_up_their_requests() {
        let mut r = RunUsage::default();
        r.add(&usage(1000, 10, Some(800), None));
        r.add(&usage(1200, 40, Some(1000), Some(0.5)));
        assert_eq!(
            (r.measured, r.input, r.output, r.cache_hit),
            (2, 2200, 50, 1800)
        );
        assert!(r.cache_reported);
        assert_eq!(r.cost_usd, Some(0.5));
        // One request without a cache figure: the run's cache is unknown from then on.
        r.add(&usage(10, 1, None, None));
        assert!(!r.cache_reported);
        r.add(&usage(10, 1, Some(0), None));
        assert!(!r.cache_reported);
    }

    #[test]
    fn statistics_count_turns_steps_tokens_cache_cost_and_speed() {
        let a = RunUsage {
            message: 1,
            calls: 3,
            tool_calls: 4,
            measured: 3,
            input: 9000,
            output: 300,
            cache_hit: 6000,
            cache_reported: true,
            cost_usd: Some(0.012),
            model_ms: 6000,
        };
        let b = RunUsage {
            message: 3,
            calls: 1,
            measured: 1,
            input: 3000,
            output: 120,
            cache_hit: 3000,
            cache_reported: true,
            cost_usd: Some(0.003),
            model_ms: 1000,
            ..RunUsage::default()
        };
        let s = summarize(2, &[a.clone(), b.clone()]);
        assert_eq!((s.turns, s.steps, s.tool_calls), (2, 4, 4));
        assert_eq!((s.input, s.output, s.tokens), (12000, 420, Some(12420)));
        assert_eq!(s.cache_percent, Some(75), "weighted by the prompt");
        assert!((s.cost_usd.unwrap() - 0.015).abs() < 1e-12);
        assert_eq!(s.speed, Some(120), "the latest answer's");
        // An answer the provider measured but did not bill: no total; nor a cache rate when it
        // did not report its cache.
        let c = RunUsage {
            message: 5,
            calls: 1,
            measured: 1,
            input: 100,
            output: 0,
            ..RunUsage::default()
        };
        let s = summarize(3, &[a.clone(), b.clone(), c]);
        assert_eq!((s.cost_usd, s.cache_percent), (None, None));
        assert_eq!(
            s.speed,
            Some(120),
            "an answer without model time is skipped"
        );
        // A stopped answer (a step, nothing measured) changes neither.
        let stopped = RunUsage {
            message: 5,
            calls: 1,
            ..RunUsage::default()
        };
        let s = summarize(3, &[a, b, stopped]);
        assert_eq!(s.steps, 5);
        assert_eq!(s.cache_percent, Some(75));
        assert!(s.cost_usd.is_some());
        // Nothing measured: no tokens, rate, cost or speed.
        let s = summarize(1, &[RunUsage::default()]);
        assert_eq!(
            (s.tokens, s.cache_percent, s.cost_usd, s.speed),
            (None, None, None, None)
        );
    }

    #[test]
    fn context_is_calibrated_by_the_measured_prompt() {
        let est = Estimate {
            system: 9000,
            tools: 1000,
            messages: 20000,
            images: false,
        };
        let c = measure(&est, Some(100_000), None);
        assert_eq!((c.used, c.remaining, c.percent), (30000, 70000, 30));
        assert!(c.estimated && !c.window_assumed);
        // The provider counted 33,000 for a request estimated at 30,000.
        let sample = PromptSample {
            provider: "p".into(),
            model: "m".into(),
            estimated: 30000,
            input: 33000,
        };
        let c = measure(&est, Some(100_000), Some(&sample));
        assert_eq!((c.used, c.percent), (33000, 33));
        assert!(!c.estimated, "exactly what was measured");
        // 500 more estimated tokens (a draft) go on top, and make it an estimate again.
        let more = Estimate {
            messages: 20500,
            ..est
        };
        let c = measure(&more, Some(100_000), Some(&sample));
        assert_eq!(c.used, 33500);
        assert!(c.estimated);
        // No window known: Argus's assumption; never more than 100 %.
        let c = measure(&est, None, None);
        assert_eq!((c.window, c.window_assumed), (ASSUMED_WINDOW, true));
        assert_eq!(measure(&est, Some(1000), None).percent, 100);
        assert_eq!(measure(&est, Some(1000), None).remaining, 0);
    }

    /// An OpenAI-style stream: one chunk, and the usage the provider sends last.
    fn answer(delta: serde_json::Value, finish: &str, usage: serde_json::Value) -> String {
        let chunk = serde_json::json!({"choices": [{"delta": delta, "finish_reason": finish}]});
        let last = serde_json::json!({"choices": [], "usage": usage});
        format!("data: {chunk}\n\ndata: {last}\n\ndata: [DONE]\n\n")
    }

    fn settle(s: &mut Session) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while s.ai.is_busy() && std::time::Instant::now() < deadline {
            s.tick(std::time::Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// A question answered after reading a file (two requests), then another: the steps, tools,
    /// tokens and cache hits add up, the context is calibrated by what the provider counted, and
    /// a new conversation starts from nothing.
    #[test]
    fn a_conversation_counts_its_steps_tokens_and_context() {
        use serde_json::json;
        let (mut s, root) = crate::ai_tools::tests::project("usage");
        let read = answer(
            json!({"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
                "function": {"name": "read_file", "arguments": "{\"path\": \"main.tex\"}"}}]}),
            "tool_calls",
            json!({"prompt_tokens": 5000, "completion_tokens": 20, "prompt_cache_hit_tokens": 4096}),
        );
        let first = answer(
            json!({"content": "It inputs the intro."}),
            "stop",
            json!({"prompt_tokens": 5600, "completion_tokens": 80, "prompt_cache_hit_tokens": 4992}),
        );
        let second = answer(
            json!({"content": "Yes."}),
            "stop",
            json!({"prompt_tokens": 1400, "completion_tokens": 10, "prompt_cache_hit_tokens": 1024}),
        );
        let (port, server) = crate::ai_tools::tests::fake_model(vec![read, first, second]);
        let ollama = texcraft_ai::PRESETS
            .iter()
            .find(|p| p.id == "ollama")
            .unwrap();
        let i = s.ai.config.add(ollama);
        s.ai.config.providers[i].base_url = format!("http://127.0.0.1:{port}/v1");
        s.ai.config.providers[i].models = vec![texcraft_ai::Model {
            id: "m".into(),
            context_length: Some(128_000),
            ..Default::default()
        }];
        s.ai.config.normalize();
        // Before anything is sent: the system prompt and the tools, estimated.
        let empty = s.ai_context().unwrap();
        assert!(empty.system > 0 && empty.tools > 0 && empty.messages == 0);
        assert!(empty.estimated && !empty.window_assumed && empty.window == 128_000);
        assert_eq!(s.ai_stats().tokens, None);

        s.run("ai.send", &json!({"text": "What is in main.tex?"}))
            .unwrap();
        settle(&mut s);
        assert_eq!(s.ai.messages.last().unwrap().error, None);
        let st = s.ai_stats();
        assert_eq!((st.turns, st.steps, st.tool_calls), (1, 2, 1));
        assert_eq!((st.input, st.output, st.tokens), (10600, 100, Some(10700)));
        assert_eq!(st.cache_percent, Some(86)); // 9,088 of 10,600
        assert!(st.speed.is_some_and(|v| v > 0));
        assert_eq!(st.cost_usd, None, "nothing billed");
        // The provider counted 5,600 for the last request; the next one leaves the tool round
        // out and adds the answer.
        let sample = s.ai.usage.sample.clone().unwrap();
        assert_eq!((sample.input, sample.model.as_str()), (5600, "m"));
        let c = s.ai_context().unwrap();
        let total = c.system + c.tools + c.messages;
        assert_eq!(c.used, 5600 + total - sample.estimated);
        assert_eq!(c.percent, percent(c.used, 128_000));
        // The draft counts too (measured again once it changes).
        s.ai.draft = "x".repeat(400);
        assert_eq!(s.ai_context().unwrap().used, c.used + 4 + 100);
        s.ai.draft.clear();
        assert_eq!(s.ai_context().unwrap(), c);

        s.run("ai.send", &json!({"text": "Is that all?"})).unwrap();
        settle(&mut s);
        let st = s.ai_stats();
        assert_eq!((st.turns, st.steps, st.tool_calls), (2, 3, 1));
        assert_eq!(st.tokens, Some(10700 + 1410));
        let ai = &s.inspect()["ai"];
        assert_eq!(ai["stats"]["steps"], 3);
        assert_eq!(ai["stats"]["cachePercent"], 84); // 10,112 of 12,000
        assert_eq!(ai["context"]["window"], 128_000);
        assert_eq!(ai["runs"][1]["message"], 3);
        let seen = server.join().unwrap();
        assert!(seen[0].contains("\"include_usage\":true"), "{}", seen[0]);

        s.run("ai.clear", &json!(null)).unwrap();
        let st = s.ai_stats();
        assert_eq!((st.turns, st.steps, st.tokens), (0, 0, None));
        assert!(
            s.ai_context().unwrap().estimated,
            "no measured prompt any more"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
