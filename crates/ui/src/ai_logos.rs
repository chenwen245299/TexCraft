//! Logos of AI providers and model families, shown in the Assistant's model menu, its answers
//! and Settings → AI. The SVGs come from LobeHub's icon set (Apache-2.0, see ATTRIBUTION.md)
//! by way of Argus; they identify the services and imply no endorsement. Monochrome logos are
//! drawn in the text colour (dark mode included); the others keep their colours. SoCLaaS shows
//! the NUS crest (a PNG, also by way of Argus).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use egui::{Color32, Rect, Ui};
use texcraft_ai::Provider;

/// (key, SVG, monochrome)
const LOGOS: &[(&str, &[u8], bool)] = &[
    // Not a provider: the AI Suggestions button's copilot (drawn for TexCraft, see
    // ATTRIBUTION.md), loaded and tinted like the monochrome logos.
    (
        "icons/copilot",
        include_bytes!("../../../assets/icons/copilot.svg"),
        true,
    ),
    (
        "providers/MoleAPI",
        include_bytes!("../../../assets/ai-logos/providers/MoleAPI.svg"),
        false,
    ),
    (
        "providers/alibaba",
        include_bytes!("../../../assets/ai-logos/providers/alibaba.svg"),
        false,
    ),
    (
        "providers/baidu",
        include_bytes!("../../../assets/ai-logos/providers/baidu.svg"),
        false,
    ),
    (
        "providers/bytedance",
        include_bytes!("../../../assets/ai-logos/providers/bytedance.svg"),
        false,
    ),
    (
        "providers/claude",
        include_bytes!("../../../assets/ai-logos/providers/claude.svg"),
        false,
    ),
    (
        "providers/deepseek",
        include_bytes!("../../../assets/ai-logos/providers/deepseek.svg"),
        false,
    ),
    (
        "providers/dots",
        include_bytes!("../../../assets/ai-logos/providers/dots.svg"),
        false,
    ),
    (
        "providers/gemini",
        include_bytes!("../../../assets/ai-logos/providers/gemini.svg"),
        false,
    ),
    (
        "providers/gemma",
        include_bytes!("../../../assets/ai-logos/providers/gemma.svg"),
        false,
    ),
    (
        "providers/grok",
        include_bytes!("../../../assets/ai-logos/providers/grok.svg"),
        true,
    ),
    (
        "providers/huggingface",
        include_bytes!("../../../assets/ai-logos/providers/huggingface.svg"),
        false,
    ),
    (
        "providers/kimi",
        include_bytes!("../../../assets/ai-logos/providers/kimi.svg"),
        false,
    ),
    (
        "providers/kling",
        include_bytes!("../../../assets/ai-logos/providers/kling.svg"),
        false,
    ),
    (
        "providers/lmstudio",
        include_bytes!("../../../assets/ai-logos/providers/lmstudio.svg"),
        true,
    ),
    (
        "providers/meituan",
        include_bytes!("../../../assets/ai-logos/providers/meituan.svg"),
        false,
    ),
    (
        "providers/microsoft",
        include_bytes!("../../../assets/ai-logos/providers/microsoft.svg"),
        false,
    ),
    (
        "providers/minimax",
        include_bytes!("../../../assets/ai-logos/providers/minimax.svg"),
        false,
    ),
    (
        "providers/nvidia",
        include_bytes!("../../../assets/ai-logos/providers/nvidia.svg"),
        false,
    ),
    (
        "providers/ollama-color",
        include_bytes!("../../../assets/ai-logos/providers/ollama-color.svg"),
        true,
    ),
    (
        "providers/ollama",
        include_bytes!("../../../assets/ai-logos/providers/ollama.svg"),
        true,
    ),
    (
        "providers/openai",
        include_bytes!("../../../assets/ai-logos/providers/openai.svg"),
        true,
    ),
    (
        "providers/openrouter",
        include_bytes!("../../../assets/ai-logos/providers/openrouter.svg"),
        false,
    ),
    (
        "providers/qwenai",
        include_bytes!("../../../assets/ai-logos/providers/qwenai.svg"),
        false,
    ),
    (
        "providers/siliconflow",
        include_bytes!("../../../assets/ai-logos/providers/siliconflow.svg"),
        false,
    ),
    (
        "providers/stepfun",
        include_bytes!("../../../assets/ai-logos/providers/stepfun.svg"),
        false,
    ),
    (
        "providers/tencent",
        include_bytes!("../../../assets/ai-logos/providers/tencent.svg"),
        false,
    ),
    (
        "providers/xai",
        include_bytes!("../../../assets/ai-logos/providers/xai.svg"),
        true,
    ),
    (
        "providers/xiaomimimo",
        include_bytes!("../../../assets/ai-logos/providers/xiaomimimo.svg"),
        true,
    ),
    (
        "providers/zhipu",
        include_bytes!("../../../assets/ai-logos/providers/zhipu.svg"),
        false,
    ),
    (
        "models/alibaba",
        include_bytes!("../../../assets/ai-logos/models/alibaba.svg"),
        false,
    ),
    (
        "models/baidu",
        include_bytes!("../../../assets/ai-logos/models/baidu.svg"),
        false,
    ),
    (
        "models/bytedance",
        include_bytes!("../../../assets/ai-logos/models/bytedance.svg"),
        false,
    ),
    (
        "models/claude",
        include_bytes!("../../../assets/ai-logos/models/claude.svg"),
        false,
    ),
    (
        "models/deepseek",
        include_bytes!("../../../assets/ai-logos/models/deepseek.svg"),
        false,
    ),
    (
        "models/dots",
        include_bytes!("../../../assets/ai-logos/models/dots.svg"),
        false,
    ),
    (
        "models/gemini",
        include_bytes!("../../../assets/ai-logos/models/gemini.svg"),
        false,
    ),
    (
        "models/gemma",
        include_bytes!("../../../assets/ai-logos/models/gemma.svg"),
        false,
    ),
    (
        "models/grok",
        include_bytes!("../../../assets/ai-logos/models/grok.svg"),
        true,
    ),
    (
        "models/huggingface",
        include_bytes!("../../../assets/ai-logos/models/huggingface.svg"),
        false,
    ),
    (
        "models/inclusionai",
        include_bytes!("../../../assets/ai-logos/models/inclusionai.svg"),
        false,
    ),
    (
        "models/kimi",
        include_bytes!("../../../assets/ai-logos/models/kimi.svg"),
        false,
    ),
    (
        "models/kling",
        include_bytes!("../../../assets/ai-logos/models/kling.svg"),
        false,
    ),
    (
        "models/meituan",
        include_bytes!("../../../assets/ai-logos/models/meituan.svg"),
        false,
    ),
    (
        "models/microsoft",
        include_bytes!("../../../assets/ai-logos/models/microsoft.svg"),
        false,
    ),
    (
        "models/minimax",
        include_bytes!("../../../assets/ai-logos/models/minimax.svg"),
        false,
    ),
    (
        "models/nvidia",
        include_bytes!("../../../assets/ai-logos/models/nvidia.svg"),
        false,
    ),
    (
        "models/ollama-color",
        include_bytes!("../../../assets/ai-logos/models/ollama-color.svg"),
        true,
    ),
    (
        "models/openai",
        include_bytes!("../../../assets/ai-logos/models/openai.svg"),
        true,
    ),
    (
        "models/qwen",
        include_bytes!("../../../assets/ai-logos/models/qwen.svg"),
        false,
    ),
    (
        "models/stepfun",
        include_bytes!("../../../assets/ai-logos/models/stepfun.svg"),
        false,
    ),
    (
        "models/tencent",
        include_bytes!("../../../assets/ai-logos/models/tencent.svg"),
        false,
    ),
    (
        "models/xai",
        include_bytes!("../../../assets/ai-logos/models/xai.svg"),
        true,
    ),
    (
        "models/xiaomimimo",
        include_bytes!("../../../assets/ai-logos/models/xiaomimimo.svg"),
        true,
    ),
    (
        "models/z-ai",
        include_bytes!("../../../assets/ai-logos/models/z-ai.svg"),
        false,
    ),
    (
        "models/zhipu",
        include_bytes!("../../../assets/ai-logos/models/zhipu.svg"),
        false,
    ),
];

/// (key, PNG): logos that only exist as pictures, drawn in their own colours.
const PNG_LOGOS: &[(&str, &[u8])] = &[(
    "providers/nus",
    include_bytes!("../../../assets/nus/nus.png"),
)];

/// A logo ready to load.
struct Source {
    data: Arc<[u8]>,
    /// Drawn in the ink colour.
    mono: bool,
    /// `svg` or `png`: picks egui's loader.
    ext: &'static str,
}

/// Logo key → its bytes.
type Sources = HashMap<&'static str, Source>;

/// The bytes to load: monochrome SVGs are recoloured white so a tint gives any colour.
fn sources() -> &'static Sources {
    static SOURCES: OnceLock<Sources> = OnceLock::new();
    SOURCES.get_or_init(|| {
        let svgs = LOGOS.iter().map(|(key, bytes, mono)| {
            let data: Arc<[u8]> = if *mono {
                Arc::from(recolour(&String::from_utf8_lossy(bytes)).into_bytes())
            } else {
                Arc::from(*bytes)
            };
            let source = Source {
                data,
                mono: *mono,
                ext: "svg",
            };
            (*key, source)
        });
        let pngs = PNG_LOGOS.iter().map(|(key, bytes)| {
            let source = Source {
                data: Arc::from(*bytes),
                mono: false,
                ext: "png",
            };
            (*key, source)
        });
        svgs.chain(pngs).collect()
    })
}

/// A monochrome logo in white, for `paint` to tint: `currentColor` becomes white, and paths
/// without a fill of their own get white from the root, unless the root sets a fill already (a
/// second `fill` attribute makes the file unreadable: the logo would show as egui's red
/// "failed to load" triangle).
fn recolour(svg: &str) -> String {
    let text = svg.replace("currentColor", "#FFFFFF");
    let root_has_fill = text
        .find("<svg")
        .and_then(|start| {
            let tag = text.get(start..)?;
            tag.get(..tag.find('>')?)
        })
        .is_some_and(|tag| tag.contains(" fill="));
    if root_has_fill {
        text
    } else {
        text.replacen("<svg ", "<svg fill=\"#FFFFFF\" ", 1)
    }
}

/// The logo for model `m` of `provider`: its maker's when the model's id or display name says
/// who that is, else the provider's. SoCLaaS ids are aliases its operators can repoint, so only
/// its display names (the operators' own words, "Testing GLM 5.3 Flash") are read.
pub fn for_entry(provider: &Provider, m: &texcraft_ai::Model) -> Option<&'static str> {
    let by_name = Some(m.name.trim())
        .filter(|n| !n.is_empty())
        .and_then(self::model);
    if texcraft_ai::is_soclaas(provider) {
        return by_name.or_else(|| self::provider(provider));
    }
    self::model(&m.id)
        .or(by_name)
        .or_else(|| self::provider(provider))
}

/// The logo for model `id` of `provider` ([`for_entry`] with the model's display name when the
/// provider lists it).
pub fn for_model(provider: &Provider, id: &str) -> Option<&'static str> {
    match provider.models.iter().find(|m| m.id == id) {
        Some(m) => for_entry(provider, m),
        None if texcraft_ai::is_soclaas(provider) => self::provider(provider),
        None => self::model(id).or_else(|| self::provider(provider)),
    }
}

/// Draws logo `key` (`"providers/claude"`, `"models/openai"`, …) into `rect`; monochrome logos
/// take `ink`.
pub fn paint(ui: &Ui, key: &str, rect: Rect, ink: Color32) {
    let Some(source) = sources().get(key) else {
        return;
    };
    let image = egui::Image::new(egui::ImageSource::Bytes {
        uri: format!("bytes://texcraft/ai-logos/{key}.{}", source.ext).into(),
        bytes: egui::load::Bytes::Shared(source.data.clone()),
    })
    .fit_to_exact_size(rect.size());
    let image = if source.mono { image.tint(ink) } else { image };
    image.paint_at(ui, rect);
}

fn has(key: &str) -> bool {
    sources().contains_key(key)
}

/// The logo of a provider, from its preset id or its server.
pub fn provider(p: &Provider) -> Option<&'static str> {
    let id = p.id.to_ascii_lowercase();
    let url = p.base_url.to_ascii_lowercase();
    let pick = |needles: &[&str]| needles.iter().any(|n| id.starts_with(n) || url.contains(n));
    let key = if texcraft_ai::is_soclaas(p) {
        "providers/nus"
    } else if pick(&["anthropic"]) {
        "providers/claude"
    } else if pick(&["openrouter"]) {
        "providers/openrouter"
    } else if pick(&["gemini", "generativelanguage.googleapis"]) {
        // Before OpenAI: Gemini's OpenAI-compatible address ends in /openai.
        "providers/gemini"
    } else if pick(&["openai"]) {
        "providers/openai"
    } else if pick(&["deepseek"]) {
        "providers/deepseek"
    } else if pick(&["ollama", ":11434"]) {
        "providers/ollama"
    } else if pick(&["lmstudio", ":1234"]) {
        "providers/lmstudio"
    } else if pick(&["x.ai", "xai", "grok"]) {
        "providers/xai"
    } else if pick(&["siliconflow"]) {
        "providers/siliconflow"
    } else if pick(&["moonshot", "kimi"]) {
        "providers/kimi"
    } else if pick(&["bigmodel", "zhipu"]) {
        "providers/zhipu"
    } else if pick(&["minimax"]) {
        "providers/minimax"
    } else if pick(&["dashscope", "aliyun", "qwen"]) {
        "providers/qwenai"
    } else if pick(&["volces", "volcengine", "bytedance"]) {
        "providers/bytedance"
    } else if pick(&["huggingface"]) {
        "providers/huggingface"
    } else if pick(&["nvidia"]) {
        "providers/nvidia"
    } else if pick(&["stepfun"]) {
        "providers/stepfun"
    } else if pick(&["mimo", "xiaomi"]) {
        "providers/xiaomimimo"
    } else if pick(&["moleapi"]) {
        "providers/MoleAPI"
    } else {
        return None;
    };
    has(key).then_some(key)
}

/// The logo of a model family, from the model id.
pub fn model(id: &str) -> Option<&'static str> {
    let m = id.to_ascii_lowercase();
    let m = m.rsplit('/').next().unwrap_or(&m).to_string();
    let starts = |p: &[&str]| p.iter().any(|x| m.starts_with(x));
    let has_any = |p: &[&str]| p.iter().any(|x| m.contains(x));
    let key = if has_any(&["claude"]) {
        "models/claude"
    } else if starts(&["gpt", "o1", "o3", "o4", "chatgpt", "davinci"]) || has_any(&["openai"]) {
        "models/openai"
    } else if has_any(&["gemma"]) {
        "models/gemma"
    } else if has_any(&["gemini"]) {
        "models/gemini"
    } else if has_any(&["deepseek"]) {
        "models/deepseek"
    } else if has_any(&["qwen", "qwq"]) {
        "models/qwen"
    } else if has_any(&["grok"]) {
        "models/grok"
    } else if has_any(&["kimi", "moonshot"]) {
        "models/kimi"
    } else if has_any(&["glm", "zhipu", "chatglm"]) {
        "models/zhipu"
    } else if has_any(&["minimax", "abab"]) {
        "models/minimax"
    } else if has_any(&["doubao", "seed-"]) {
        "models/bytedance"
    } else if has_any(&["hunyuan"]) {
        "models/tencent"
    } else if has_any(&["ernie"]) {
        "models/baidu"
    } else if has_any(&["step-"]) {
        "models/stepfun"
    } else if has_any(&["mimo"]) {
        "models/xiaomimimo"
    } else if has_any(&["phi-", "phi3", "phi4"]) {
        "models/microsoft"
    } else if has_any(&["nemotron"]) {
        "models/nvidia"
    } else if has_any(&["llama", "mistral", "ollama"]) {
        "models/ollama-color"
    } else {
        return None;
    };
    has(key).then_some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logos_match_providers_and_models() {
        let p = |id: &str, url: &str| Provider {
            id: id.into(),
            base_url: url.into(),
            ..Provider::default()
        };
        assert_eq!(
            provider(&p("anthropic", "https://api.anthropic.com")),
            Some("providers/claude")
        );
        assert_eq!(
            provider(&p("custom", "http://localhost:11434/v1")),
            Some("providers/ollama")
        );
        assert_eq!(
            provider(&p("openrouter", "https://openrouter.ai/api/v1")),
            Some("providers/openrouter")
        );
        assert_eq!(provider(&p("custom-2", "https://example.com/v1")), None);
        // Every preset but Custom shows its own logo.
        for preset in texcraft_ai::PRESETS.iter().filter(|x| x.id != "custom") {
            let key = provider(&p(preset.id, preset.base_url));
            assert!(key.is_some_and(has), "{}", preset.id);
        }
        assert_eq!(
            provider(&p("mimo", "https://api.xiaomimimo.com/v1")),
            Some("providers/xiaomimimo")
        );
        assert_eq!(
            provider(&p(
                "gemini",
                "https://generativelanguage.googleapis.com/v1beta/openai"
            )),
            Some("providers/gemini")
        );
        assert_eq!(
            provider(&p("moleapi", "https://api.moleapi.com/v1")),
            Some("providers/MoleAPI")
        );
        assert_eq!(model("MiniMax-M2.7"), Some("models/minimax"));
        assert_eq!(model("glm-4.6"), Some("models/zhipu"));
        assert_eq!(model("mimo-v2-flash"), Some("models/xiaomimimo"));
        assert_eq!(model("step-3.5-flash"), Some("models/stepfun"));
        assert_eq!(model("kimi-k2-turbo-preview"), Some("models/kimi"));
        // SoCLaaS shows the NUS crest, also for its models: their aliases say nothing about
        // the model behind them.
        let nus = p("soclaas", texcraft_ai::SOCLAAS_BASE_URL);
        assert_eq!(provider(&nus), Some("providers/nus"));
        assert_eq!(for_model(&nus, "gpt-private-alias"), Some("providers/nus"));
        // …but its display names are the operators' word: GLM's and DeepSeek's logos.
        let glm = texcraft_ai::Model {
            id: "x-test-1".into(),
            name: "Testing GLM 5.3 Flash".into(),
            ..Default::default()
        };
        assert_eq!(for_entry(&nus, &glm), Some("models/zhipu"));
        let ds = texcraft_ai::Model {
            id: "x-test-2".into(),
            name: "Testing DeepSeek V4.1 Flash".into(),
            ..Default::default()
        };
        assert_eq!(for_entry(&nus, &ds), Some("models/deepseek"));
        let alias = texcraft_ai::Model {
            id: "deepseek-alias".into(),
            ..Default::default()
        };
        assert_eq!(for_entry(&nus, &alias), Some("providers/nus"));
        let mut listed = nus.clone();
        listed.models = vec![glm];
        assert_eq!(for_model(&listed, "x-test-1"), Some("models/zhipu"));
        let named = Provider {
            name: "My SoCLaaS".into(),
            ..p("custom-3", "https://example.com/v1")
        };
        assert_eq!(provider(&named), Some("providers/nus"));
        let openai = p("openai", "https://api.openai.com/v1");
        assert_eq!(for_model(&openai, "gpt-4.1"), Some("models/openai"));
        assert_eq!(model("claude-opus-5-5"), Some("models/claude"));
        assert_eq!(model("gpt-4.1-mini"), Some("models/openai"));
        assert_eq!(model("deepseek-chat"), Some("models/deepseek"));
        assert_eq!(model("google/gemini-2.5-pro"), Some("models/gemini"));
        assert_eq!(model("qwen3-coder"), Some("models/qwen"));
        assert_eq!(model("something-else"), None);
        // Every SVG logo is SVG and mono ones were recoloured.
        for (key, source) in sources().iter().filter(|(_, s)| s.ext == "svg") {
            let text = String::from_utf8_lossy(&source.data);
            assert!(text.contains("<svg"), "{key}");
            if source.mono {
                assert!(!text.contains("currentColor"), "{key}");
            }
        }
    }

    /// Every logo renders with the SVG renderer egui uses, to visible pixels: a logo it cannot
    /// read shows as egui's red "failed to load" triangle in the menus.
    #[test]
    fn every_logo_renders() {
        use resvg::{tiny_skia, usvg};
        let mut broken = Vec::new();
        for (key, source) in sources().iter().filter(|(_, s)| s.ext == "svg") {
            let tree = match usvg::Tree::from_data(&source.data, &usvg::Options::default()) {
                Ok(t) => t,
                Err(e) => {
                    broken.push(format!("{key}: {e}"));
                    continue;
                }
            };
            let Some(mut pixmap) = tiny_skia::Pixmap::new(32, 32) else {
                panic!("pixmap");
            };
            let size = tree.size();
            let scale = 32.0 / size.width().max(size.height());
            resvg::render(
                &tree,
                tiny_skia::Transform::from_scale(scale, scale),
                &mut pixmap.as_mut(),
            );
            if pixmap.pixels().iter().all(|p| p.alpha() == 0) {
                broken.push(format!("{key}: renders nothing"));
            }
        }
        assert!(broken.is_empty(), "{broken:#?}");
    }

    /// Every PNG logo loads through egui's image loaders, as `paint` draws it.
    #[test]
    fn every_png_logo_loads() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        for (key, source) in sources().iter().filter(|(_, s)| s.ext == "png") {
            let uri = format!("bytes://texcraft/ai-logos/{key}.png");
            ctx.include_bytes(uri.clone(), egui::load::Bytes::Shared(source.data.clone()));
            let mut size = None;
            for _ in 0..500 {
                match ctx.try_load_image(&uri, egui::SizeHint::default()) {
                    Ok(egui::load::ImagePoll::Ready { image }) => {
                        size = Some(image.size);
                        break;
                    }
                    Ok(egui::load::ImagePoll::Pending { .. }) => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(e) => panic!("{key}: {e}"),
                }
            }
            assert!(
                size.is_some_and(|[w, h]| w >= 64 && h >= 64),
                "{key}: {size:?}"
            );
        }
    }
}
