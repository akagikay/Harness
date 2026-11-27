use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningSupport {
    /// Model exposes user-selectable reasoning/thinking effort levels.
    Configurable,
    /// Model always reasons internally and does not expose selectable effort levels.
    Fixed,
    /// Standard model that does not expose reasoning effort controls.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelReasoningSpec {
    pub support: ReasoningSupport,
    /// Exact ordered levels supported by the model (e.g. ["low", "medium", "high"]).
    /// Empty when `support` is `Fixed` or `None`.
    pub levels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_level: Option<String>,
    /// Where capability detection came from: "api_metadata", "model_family", or "default".
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveReasoningAction {
    /// Model does not support configurable reasoning parameters; omit parameter completely.
    Omit,
    /// Model supports disabling reasoning explicitly (requested "none").
    Disable,
    /// Send the selected effort level (guaranteed to belong to the model's exposed levels).
    Effort(String),
}

fn normalize_level(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" | "disabled" | "false" => "none".to_string(),
        "med" => "medium".to_string(),
        "min" => "minimal".to_string(),
        other => other.to_string(),
    }
}

fn push_unique_levels(raw_levels: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    for item in raw_levels {
        if let Some(s) = item.as_str() {
            let norm = normalize_level(s);
            if !norm.is_empty() && !out.iter().any(|existing: &String| existing == &norm) {
                out.push(norm);
            }
        }
    }
    out
}

fn extract_explicit_api_levels(item: &Value) -> Option<Vec<String>> {
    let candidate_arrays = [
        item.get("reasoning_effort_levels"),
        item.get("supported_reasoning_efforts"),
        item.get("thinking_levels"),
        item.get("reasoning").and_then(|r| r.get("levels")),
        item.get("reasoning").and_then(|r| r.get("efforts")),
        item.get("thinking").and_then(|t| t.get("levels")),
    ];

    for arr_opt in candidate_arrays {
        if let Some(arr) = arr_opt.and_then(|v| v.as_array()) {
            let levels = push_unique_levels(arr);
            if !levels.is_empty() {
                return Some(levels);
            }
        }
    }
    None
}

fn infer_family_spec(model_id: &str) -> ModelReasoningSpec {
    let lower = model_id.trim().to_ascii_lowercase();
    let short = lower.rsplit('/').next().unwrap_or(&lower);

    if short == "google-search-model" {
        return ModelReasoningSpec {
            support: ReasoningSupport::None,
            levels: Vec::new(),
            default_level: None,
            source: "experimental_builtin".to_string(),
        };
    }

    // 1. Fixed always-on reasoning models (never expose selectable effort levels)
    let is_fixed_r1 = short.contains("deepseek-r1")
        || short.ends_with("-r1")
        || short == "r1"
        || short.starts_with("qwq")
        || short.contains("marco-o1")
        || short.contains("skywork-o1");
    if is_fixed_r1 {
        return ModelReasoningSpec {
            support: ReasoningSupport::Fixed,
            levels: Vec::new(),
            default_level: None,
            source: "model_family".to_string(),
        };
    }

    // 2. Grok 3 Mini exposes strictly ["low", "high"]
    if short.contains("grok-3-mini") {
        return ModelReasoningSpec {
            support: ReasoningSupport::Configurable,
            levels: vec!["low".into(), "high".into()],
            default_level: Some("high".into()),
            source: "model_family".to_string(),
        };
    }

    // 3. GPT-5.1 / GPT-5.2 series expose none..high
    if short.starts_with("gpt-5.1") || short.starts_with("gpt-5.2") || short.contains("gpt-5.1") || short.contains("gpt-5.2") {
        return ModelReasoningSpec {
            support: ReasoningSupport::Configurable,
            levels: vec!["none".into(), "minimal".into(), "low".into(), "medium".into(), "high".into()],
            default_level: Some("medium".into()),
            source: "model_family".to_string(),
        };
    }

    // 4. GPT-5 series expose ["minimal", "low", "medium", "high"] (no "none")
    if short.starts_with("gpt-5") || short.contains("gpt-5-") {
        return ModelReasoningSpec {
            support: ReasoningSupport::Configurable,
            levels: vec!["minimal".into(), "low".into(), "medium".into(), "high".into()],
            default_level: Some("medium".into()),
            source: "model_family".to_string(),
        };
    }

    // 5. OpenAI o-series & gpt-oss expose strictly ["low", "medium", "high"] (no "none")
    let is_openai_low_med_high = short == "o1"
        || short.starts_with("o1-")
        || short == "o3"
        || short.starts_with("o3-")
        || short == "o4"
        || short.starts_with("o4-")
        || short.contains("codex-mini")
        || short.contains("gpt-oss");
    if is_openai_low_med_high {
        return ModelReasoningSpec {
            support: ReasoningSupport::Configurable,
            levels: vec!["low".into(), "medium".into(), "high".into()],
            default_level: Some("medium".into()),
            source: "model_family".to_string(),
        };
    }

    // 6. Gemini 2.5 Pro & Gemini 3 Pro expose strictly ["low", "medium", "high"]
    if short.contains("gemini-2.5-pro") || short.contains("gemini-3-pro") || short.contains("gemini-3.") {
        return ModelReasoningSpec {
            support: ReasoningSupport::Configurable,
            levels: vec!["low".into(), "medium".into(), "high".into()],
            default_level: Some("medium".into()),
            source: "model_family".to_string(),
        };
    }

    // 7. Toggleable + budgeted reasoning models:
    //    Gemini 2.5 Flash / 2.0 Thinking, Claude 3.7 / 4+, Qwen3 (unless explicit -instruct non-thinking),
    //    DeepSeek V3.1/V3.2/Reasoner, Nemotron reasoning/nano models, Kimi thinking, Magistral, Step-5
    let is_non_thinking_qwen = short.contains("qwen3") && short.contains("-instruct");
    let is_toggleable_reasoning = (!is_non_thinking_qwen && short.contains("qwen3"))
        || short.contains("gemini-2.5-flash")
        || short.contains("gemini-2.0-flash-thinking")
        || short.contains("claude-3-7")
        || short.contains("claude-3.7")
        || short.contains("claude-sonnet-4")
        || short.contains("claude-opus-4")
        || short.contains("claude-haiku-4")
        || short.contains("claude-4")
        || short.contains("deepseek-reasoner")
        || short.contains("deepseek-v3.1")
        || short.contains("deepseek-v3.2")
        || short.contains("deepseek-v4")
        || short.contains("magistral")
        || short.contains("kimi-k2-thinking")
        || short.contains("glm-4.5")
        || short.contains("glm-z1")
        || (short.contains("nemotron") && (short.contains("reasoning") || short.contains("nano") || short.contains("super") || short.contains("ultra")))
        || short.contains("thinking")
        || short.ends_with("-reasoning")
        || short.contains("step-5");

    if is_toggleable_reasoning {
        return ModelReasoningSpec {
            support: ReasoningSupport::Configurable,
            levels: vec!["none".into(), "low".into(), "medium".into(), "high".into()],
            default_level: Some("high".into()),
            source: "model_family".to_string(),
        };
    }

    // 8. Standard non-reasoning models (Gemma, Llama, Mistral Large, Mixtral, GPT-4o, DeepSeek Chat/V3 base, MiniMax, etc.)
    ModelReasoningSpec {
        support: ReasoningSupport::None,
        levels: Vec::new(),
        default_level: None,
        source: "model_family".to_string(),
    }
}

/// Modality & tool capabilities detected from provider metadata and model family.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelModalityCaps {
    pub text: bool,
    pub vision: bool,
    pub video: bool,
    pub audio: bool,
    pub files: bool,
    pub pdf: bool,
    pub tools: bool,
    pub source: String,
}

/// Combined model capabilities (reasoning controls, context window, and modalities).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub reasoning: ModelReasoningSpec,
    pub max_context_tokens: usize,
    pub modalities: ModelModalityCaps,
}

const DEFAULT_CONTEXT_WINDOW: usize = 128_000;
const MIN_PLAUSIBLE_CONTEXT_WINDOW: usize = 1024;

fn parse_plausible_context_len(val: &Value) -> Option<usize> {
    let raw = if let Some(n) = val.as_u64() {
        usize::try_from(n).ok()
    } else if let Some(n) = val.as_i64() {
        usize::try_from(n).ok()
    } else if let Some(s) = val.as_str() {
        s.trim().parse::<usize>().ok()
    } else {
        None
    }?;
    (raw >= MIN_PLAUSIBLE_CONTEXT_WINDOW).then_some(raw)
}

fn has_delimited_token(s: &str, token: &str) -> bool {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| part == token)
}

/// Infer a model's context window (in tokens) from live API metadata first, falling back to model family.
pub fn infer_model_context_window(model_id: &str, api_item: Option<&Value>) -> usize {
    if let Some(item) = api_item {
        let candidates = [
            item.get("context_length"),
            item.get("max_context_length"),
            item.get("context_window"),
            item.get("max_model_len"),
            item.get("input_token_limit"),
            item.get("max_input_tokens"),
            item.get("top_provider").and_then(|tp| tp.get("context_length")),
        ];
        for cand in candidates.into_iter().flatten() {
            if let Some(len) = parse_plausible_context_len(cand) {
                return len;
            }
        }
    }

    let lower = model_id.trim().to_ascii_lowercase();
    let short = lower.rsplit('/').next().unwrap_or(&lower);

    if short == "google-search-model" {
        return 32_768;
    }

    // Gemma / Gemini
    if short.contains("gemma-3") {
        return 128_000;
    }
    if short.contains("gemini-1.5-pro") {
        return 2_097_152;
    }
    if short.contains("gemini-2.5")
        || short.contains("gemini-2.0")
        || short.contains("gemini-1.5")
        || short.contains("gemini-3")
    {
        return 1_048_576;
    }

    // Anthropic Claude
    if short.contains("claude-3-5")
        || short.contains("claude-3.5")
        || short.contains("claude-3-7")
        || short.contains("claude-3.7")
        || short.contains("claude-sonnet-4")
        || short.contains("claude-opus-4")
        || short.contains("claude-haiku-4")
        || short.contains("claude-4")
        || short.contains("claude-5")
        || short.contains("claude-sonnet-5")
        || short.contains("claude-opus-5")
        || short.contains("claude-haiku-5")
    {
        return if has_delimited_token(short, "1m") {
            1_000_000
        } else {
            200_000
        };
    }

    // OpenAI
    if short.starts_with("gpt-5") || short.contains("gpt-5") {
        return 400_000;
    }
    if short.contains("gpt-4.1") {
        return 1_047_576;
    }
    if short == "o1"
        || short.starts_with("o1-")
        || short == "o3"
        || short.starts_with("o3-")
        || short == "o4"
        || short.starts_with("o4-")
    {
        return 200_000;
    }
    if short.contains("gpt-4o") || short.contains("gpt-oss") || short.contains("gpt-4-turbo") {
        return 128_000;
    }

    // DeepSeek
    if short.contains("deepseek-r1")
        || short.contains("deepseek-v3")
        || short.contains("deepseek-v4")
        || short.contains("deepseek-chat")
        || short.contains("deepseek-reasoner")
    {
        return 128_000;
    }

    // NVIDIA Nemotron (check before Llama because some Nemotron variants have llama-3.x in their ID)
    if short.contains("nemotron") {
        return if short.contains("super") {
            262_144
        } else {
            128_000
        };
    }

    // Meta Llama
    if short.contains("llama-4-scout") || short.contains("llama4-scout") {
        return 10_000_000;
    }
    if short.contains("llama-4-maverick") || short.contains("llama4-maverick") {
        return 1_000_000;
    }
    if short.contains("llama-3.1")
        || short.contains("llama-3.2")
        || short.contains("llama-3.3")
        || short.contains("llama3.1")
        || short.contains("llama3.2")
        || short.contains("llama3.3")
    {
        return 128_000;
    }

    // Qwen
    if short.contains("qwen") || short.starts_with("qwq") {
        if has_delimited_token(short, "1m") {
            return 1_000_000;
        }
        if short.contains("qwen3-coder") {
            return 262_144;
        }
        if short.contains("qwen3")
            || short.starts_with("qwq")
            || short.contains("qwen2.5")
            || short.contains("qwen-2.5")
        {
            return 128_000;
        }
    }

    // Mistral
    if short.contains("devstral-2") {
        return 256_000;
    }
    if short.contains("mistral-large-4") {
        return 262_144;
    }
    if short.contains("mistral-large")
        || short.contains("codestral")
        || short.contains("devstral")
    {
        return 128_000;
    }

    // xAI Grok
    if short.contains("grok-4") {
        return 256_000;
    }
    if short.contains("grok-3") {
        return 131_072;
    }

    DEFAULT_CONTEXT_WINDOW
}

/// Resolve the active context window from auto-detection or explicit user configuration.
pub fn effective_max_context_tokens(
    model_id: &str,
    configured_max: usize,
    auto_detect: bool,
    api_item: Option<&Value>,
) -> usize {
    if auto_detect || configured_max == 0 {
        infer_model_context_window(model_id, api_item)
    } else {
        configured_max
    }
}

/// Infer multimodal capabilities (`vision`, `video`, `audio`, `pdf`, `files`, `tools`)
/// from live API metadata first, combining with model-family knowledge when metadata omits fields.
pub fn infer_model_modalities(model_id: &str, api_item: Option<&Value>) -> ModelModalityCaps {
    let lower = model_id.trim().to_ascii_lowercase();
    let short = lower.rsplit('/').next().unwrap_or(&lower);

    if short == "google-search-model" {
        return ModelModalityCaps {
            text: true,
            vision: false,
            video: false,
            audio: false,
            files: true,
            pdf: false,
            tools: true,
            source: "experimental_builtin".to_string(),
        };
    }

    let mut vision = false;
    let mut video = false;
    let mut audio = false;
    let mut pdf = false;
    let mut tools = true;

    let is_gemini_mm = short.contains("gemini-1.5")
        || short.contains("gemini-2.0")
        || short.contains("gemini-2.5")
        || short.contains("gemini-3")
        || short.contains("gemini-4");
    let is_omni = short.contains("omni") || short.contains("qwen2.5-omni") || short.contains("qwen3-omni");
    let is_qwen_vl = short.contains("qwen2-vl")
        || short.contains("qwen2.5-vl")
        || short.contains("qwen3-vl")
        || short.contains("qvq");
    let is_claude_mm = short.contains("claude-3")
        || short.contains("claude-sonnet-4")
        || short.contains("claude-opus-4")
        || short.contains("claude-haiku-4")
        || short.contains("claude-4")
        || short.contains("claude-5");
    let is_openai_mm = short.contains("gpt-4o")
        || short.contains("gpt-4.1")
        || short.contains("gpt-4.5")
        || short.contains("gpt-4-turbo")
        || short.contains("gpt-5")
        || has_delimited_token(short, "o1")
        || has_delimited_token(short, "o3")
        || has_delimited_token(short, "o4");
    let is_gemma_mm = short.contains("gemma-3") && !short.contains("-1b");
    let is_llama_mm = short.contains("llama-4")
        || (short.contains("llama-3.2") && (short.contains("11b") || short.contains("90b") || short.contains("vision")));
    let is_explicit_vl = has_delimited_token(short, "vl")
        || short.contains("vision")
        || short.contains("multimodal")
        || short.contains("pixtral")
        || short.contains("llava")
        || short.contains("internvl")
        || short.contains("minicpm-v")
        || short.contains("molmo")
        || short.contains("kimi-vl")
        || short.contains("kimi-k2.5")
        || short.contains("grok-2-vision")
        || short.contains("grok-3")
        || short.contains("grok-4");

    if is_gemini_mm {
        vision = true;
        video = true;
        audio = true;
        pdf = true;
    } else if is_omni {
        vision = true;
        video = true;
        audio = true;
    } else if is_qwen_vl {
        vision = true;
        video = true;
    } else if is_claude_mm || is_openai_mm {
        vision = true;
        pdf = true;
        if short.contains("audio") || short.contains("realtime") {
            audio = true;
        }
    } else if is_gemma_mm || is_llama_mm || is_explicit_vl {
        vision = true;
        if short.contains("video") {
            video = true;
        }
    }

    let mut from_api = false;
    if let Some(item) = api_item {
        let mut saw_mod_list = false;
        let mod_arrays = [
            item.pointer("/architecture/input_modalities"),
            item.get("input_modalities"),
            item.pointer("/capabilities/input_modalities"),
        ];
        for arr in mod_arrays.into_iter().flatten().filter_map(|v| v.as_array()) {
            saw_mod_list = true;
            from_api = true;
            for entry in arr.iter().filter_map(|v| v.as_str()) {
                match entry.trim().to_ascii_lowercase().as_str() {
                    "image" | "images" | "vision" => vision = true,
                    "video" | "videos" => video = true,
                    "audio" => audio = true,
                    "file" | "files" | "document" | "pdf" => pdf = true,
                    _ => {}
                }
            }
        }

        if !saw_mod_list {
            if let Some(mod_str) = item
                .pointer("/architecture/modality")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("modality").and_then(|v| v.as_str()))
            {
                let input_side = mod_str
                    .split("->")
                    .next()
                    .unwrap_or(mod_str)
                    .to_ascii_lowercase();
                if !input_side.trim().is_empty() {
                    from_api = true;
                    if input_side.contains("image") || input_side.contains("vision") || input_side.contains("multimodal") {
                        vision = true;
                    }
                    if input_side.contains("video") {
                        video = true;
                    }
                    if input_side.contains("audio") {
                        audio = true;
                    }
                    if input_side.contains("file") || input_side.contains("pdf") {
                        pdf = true;
                    }
                }
            }
        }

        if let Some(v) = item.get("vision").and_then(|v| v.as_bool()) {
            from_api = true;
            vision = v;
        }

        if let Some(caps) = item.get("capabilities").and_then(|v| v.as_object()) {
            if let Some(v) = caps.get("vision").and_then(|v| v.as_bool()) {
                from_api = true;
                vision = v;
            }
            if let Some(v) = caps.get("video").and_then(|v| v.as_bool()) {
                from_api = true;
                video = v;
            }
            if let Some(v) = caps.get("audio").and_then(|v| v.as_bool()) {
                from_api = true;
                audio = v;
            }
            if let Some(v) = caps.get("pdf").and_then(|v| v.as_bool()) {
                from_api = true;
                pdf = v;
            }
            if let Some(v) = caps.get("tools").and_then(|v| v.as_bool())
                .or_else(|| caps.get("function_calling").and_then(|v| v.as_bool()))
            {
                from_api = true;
                tools = v;
            }
        }

        if let Some(params) = item.get("supported_parameters").and_then(|v| v.as_array()) {
            let param_strs: Vec<String> = params
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_ascii_lowercase())
                .collect();
            if !param_strs.is_empty() {
                from_api = true;
                tools = param_strs.iter().any(|p| p == "tools" || p == "tool_choice" || p == "functions");
            }
        }
    }

    ModelModalityCaps {
        text: true,
        vision,
        video,
        audio,
        files: true,
        pdf,
        tools,
        source: if from_api { "api_metadata" } else { "model_family" }.to_string(),
    }
}

/// Resolve reasoning capabilities, context window, and modalities for a model in one call.
pub fn resolve_model_capabilities(model_id: &str, api_item: Option<&Value>) -> ModelCapabilities {
    ModelCapabilities {
        reasoning: infer_model_reasoning_spec(model_id, api_item),
        max_context_tokens: infer_model_context_window(model_id, api_item),
        modalities: infer_model_modalities(model_id, api_item),
    }
}

/// Inspect live API model metadata first, falling back cleanly to well-known model family specs.
pub fn infer_model_reasoning_spec(model_id: &str, api_item: Option<&Value>) -> ModelReasoningSpec {
    let family = infer_family_spec(model_id);

    if let Some(item) = api_item {
        if let Some(explicit_levels) = extract_explicit_api_levels(item) {
            let def = explicit_levels
                .iter()
                .find(|l| l.as_str() == "medium" || l.as_str() == "high")
                .cloned()
                .or_else(|| explicit_levels.first().cloned());
            return ModelReasoningSpec {
                support: ReasoningSupport::Configurable,
                levels: explicit_levels,
                default_level: def,
                source: "api_metadata".to_string(),
            };
        }

        if let Some(params) = item.get("supported_parameters").and_then(|v| v.as_array()) {
            let has_reasoning_param = params.iter().filter_map(|v| v.as_str()).any(|p| {
                matches!(
                    p.to_ascii_lowercase().as_str(),
                    "reasoning" | "reasoning_effort" | "include_reasoning" | "thinking"
                )
            });

            if !has_reasoning_param && family.support != ReasoningSupport::Fixed {
                return ModelReasoningSpec {
                    support: ReasoningSupport::None,
                    levels: Vec::new(),
                    default_level: None,
                    source: "api_metadata".to_string(),
                };
            }

            if has_reasoning_param {
                return match family.support {
                    ReasoningSupport::Configurable => ModelReasoningSpec {
                        source: "api_metadata".to_string(),
                        ..family
                    },
                    ReasoningSupport::Fixed => ModelReasoningSpec {
                        support: ReasoningSupport::Configurable,
                        levels: vec!["low".into(), "medium".into(), "high".into()],
                        default_level: Some("medium".into()),
                        source: "api_metadata".to_string(),
                    },
                    ReasoningSupport::None => ModelReasoningSpec {
                        support: ReasoningSupport::Configurable,
                        levels: vec!["low".into(), "medium".into(), "high".into()],
                        default_level: Some("medium".into()),
                        source: "api_metadata".to_string(),
                    },
                };
            }
        }
    }

    family
}

/// Resolve the outbound API parameter behavior for the active model and requested effort level.
pub fn resolve_effective_reasoning(
    model_id: &str,
    requested_effort: &str,
    api_item: Option<&Value>,
) -> EffectiveReasoningAction {
    let spec = infer_model_reasoning_spec(model_id, api_item);
    if spec.support != ReasoningSupport::Configurable || spec.levels.is_empty() {
        return EffectiveReasoningAction::Omit;
    }

    let req = normalize_level(requested_effort);
    let chosen = if spec.levels.iter().any(|l| l == &req) {
        req
    } else {
        spec.default_level
            .unwrap_or_else(|| spec.levels.last().cloned().unwrap_or_default())
    };

    if chosen == "none" {
        EffectiveReasoningAction::Disable
    } else if chosen.is_empty() {
        EffectiveReasoningAction::Omit
    } else {
        EffectiveReasoningAction::Effort(chosen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_family_reasoning_levels() {
        let oss = infer_model_reasoning_spec("openai/gpt-oss-20b", None);
        assert_eq!(oss.support, ReasoningSupport::Configurable);
        assert_eq!(oss.levels, vec!["low", "medium", "high"]);

        let gemma = infer_model_reasoning_spec("google/gemma-3-12b-it", None);
        assert_eq!(gemma.support, ReasoningSupport::None);
        assert!(gemma.levels.is_empty());

        let r1 = infer_model_reasoning_spec("deepseek-ai/deepseek-r1", None);
        assert_eq!(r1.support, ReasoningSupport::Fixed);
        assert!(r1.levels.is_empty());

        let flash = infer_model_reasoning_spec("gemini-2.5-flash", None);
        assert_eq!(flash.support, ReasoningSupport::Configurable);
        assert_eq!(flash.levels, vec!["none", "low", "medium", "high"]);

        let mistral = infer_model_reasoning_spec("mistralai/mistral-large-4-1t-a52b", None);
        assert_eq!(mistral.support, ReasoningSupport::None);
    }

    #[test]
    fn test_api_metadata_overrides() {
        let meta = json!({
            "id": "custom/org-model",
            "reasoning_effort_levels": ["low", "high"]
        });
        let spec = infer_model_reasoning_spec("custom/org-model", Some(&meta));
        assert_eq!(spec.support, ReasoningSupport::Configurable);
        assert_eq!(spec.levels, vec!["low", "high"]);
        assert_eq!(spec.source, "api_metadata");

        let no_param_meta = json!({
            "id": "custom/chat-only",
            "supported_parameters": ["temperature", "tools"]
        });
        let none_spec = infer_model_reasoning_spec("custom/chat-only", Some(&no_param_meta));
        assert_eq!(none_spec.support, ReasoningSupport::None);
        assert_eq!(none_spec.source, "api_metadata");
    }

    #[test]
    fn test_resolve_effective_reasoning_clamps_invalid_none() {
        // gpt-oss does not expose "none"; falls back to its default ("medium")
        let act = resolve_effective_reasoning("openai/gpt-oss-20b", "none", None);
        assert_eq!(act, EffectiveReasoningAction::Effort("medium".into()));

        // gemma-3 omits parameter completely
        let none_act = resolve_effective_reasoning("google/gemma-3-12b-it", "high", None);
        assert_eq!(none_act, EffectiveReasoningAction::Omit);
    }

    #[test]
    fn test_infer_model_context_window_from_api_metadata() {
        let cases = [
            json!({ "context_length": 163_840 }),
            json!({ "max_context_length": 65_536 }),
            json!({ "context_window": 32_768 }),
            json!({ "max_model_len": 131_072 }),
            json!({ "input_token_limit": 1_048_576 }),
            json!({ "max_input_tokens": 200_000 }),
            json!({ "top_provider": { "context_length": 500_000 } }),
        ];
        let expected = [163_840, 65_536, 32_768, 131_072, 1_048_576, 200_000, 500_000];
        for (meta, exp) in cases.iter().zip(expected.iter()) {
            assert_eq!(infer_model_context_window("custom/model", Some(meta)), *exp);
        }

        // Implausible small integers (< 1024) are ignored and fall back to model family / default
        let tiny = json!({ "context_length": 512 });
        assert_eq!(infer_model_context_window("openai/gpt-5", Some(&tiny)), 400_000);
        assert_eq!(infer_model_context_window("unknown/model", Some(&tiny)), 128_000);
    }

    #[test]
    fn test_infer_model_context_window_by_family() {
        // Gemini & Gemma
        assert_eq!(infer_model_context_window("google/gemini-2.5-pro", None), 1_048_576);
        assert_eq!(infer_model_context_window("gemini-2.0-flash", None), 1_048_576);
        assert_eq!(infer_model_context_window("gemini-3-pro", None), 1_048_576);
        assert_eq!(infer_model_context_window("gemini-1.5-flash", None), 1_048_576);
        assert_eq!(infer_model_context_window("gemini-1.5-pro", None), 2_097_152);
        assert_eq!(infer_model_context_window("google/gemma-3-27b-it", None), 128_000);

        // Claude
        assert_eq!(infer_model_context_window("anthropic/claude-3-5-sonnet", None), 200_000);
        assert_eq!(infer_model_context_window("claude-3-7-sonnet", None), 200_000);
        assert_eq!(infer_model_context_window("claude-sonnet-4", None), 200_000);
        assert_eq!(infer_model_context_window("claude-opus-4-1m", None), 1_000_000);
        assert_eq!(infer_model_context_window("claude-5-sonnet:1m", None), 1_000_000);

        // OpenAI
        assert_eq!(infer_model_context_window("openai/gpt-5", None), 400_000);
        assert_eq!(infer_model_context_window("openai/gpt-4.1", None), 1_047_576);
        assert_eq!(infer_model_context_window("openai/o3-mini", None), 200_000);
        assert_eq!(infer_model_context_window("o4-mini", None), 200_000);
        assert_eq!(infer_model_context_window("o1", None), 200_000);
        assert_eq!(infer_model_context_window("gpt-4o", None), 128_000);
        assert_eq!(infer_model_context_window("openai/gpt-oss-120b", None), 128_000);
        assert_eq!(infer_model_context_window("gpt-4-turbo", None), 128_000);

        // DeepSeek
        assert_eq!(infer_model_context_window("deepseek-ai/deepseek-r1", None), 128_000);
        assert_eq!(infer_model_context_window("deepseek-v3", None), 128_000);
        assert_eq!(infer_model_context_window("deepseek-v4", None), 128_000);
        assert_eq!(infer_model_context_window("deepseek-chat", None), 128_000);
        assert_eq!(infer_model_context_window("deepseek-reasoner", None), 128_000);

        // Llama
        assert_eq!(infer_model_context_window("meta-llama/llama-4-scout-17b-16e", None), 10_000_000);
        assert_eq!(infer_model_context_window("meta-llama/llama-4-maverick-17b-128e", None), 1_000_000);
        assert_eq!(infer_model_context_window("meta-llama/llama-3.3-70b-instruct", None), 128_000);

        // Qwen
        assert_eq!(infer_model_context_window("qwen/qwen3-235b-a22b", None), 128_000);
        assert_eq!(infer_model_context_window("qwen/qwen3-coder-480b-a35b", None), 262_144);
        assert_eq!(infer_model_context_window("qwen/qwen2.5-72b-instruct-1m", None), 1_000_000);
        assert_eq!(infer_model_context_window("qwen/qwq-32b", None), 128_000);

        // Mistral
        assert_eq!(infer_model_context_window("mistralai/mistral-large-2411", None), 128_000);
        assert_eq!(infer_model_context_window("mistralai/codestral-2501", None), 128_000);
        assert_eq!(infer_model_context_window("mistralai/devstral-small", None), 128_000);
        assert_eq!(infer_model_context_window("mistralai/devstral-2", None), 256_000);
        assert_eq!(infer_model_context_window("mistralai/mistral-large-4-1t-a52b", None), 262_144);

        // NVIDIA & Grok & fallback
        assert_eq!(infer_model_context_window("nvidia/nemotron-3-nano", None), 128_000);
        assert_eq!(infer_model_context_window("nvidia/nemotron-3-super", None), 262_144);
        assert_eq!(infer_model_context_window("x-ai/grok-3", None), 131_072);
        assert_eq!(infer_model_context_window("x-ai/grok-4", None), 256_000);
        assert_eq!(infer_model_context_window("some-vendor/custom-model", None), 128_000);
    }

    #[test]
    fn test_effective_max_context_tokens_and_capabilities() {
        assert_eq!(
            effective_max_context_tokens("openai/gpt-5", 128_000, true, None),
            400_000
        );
        assert_eq!(
            effective_max_context_tokens("openai/gpt-5", 64_000, false, None),
            64_000
        );
        assert_eq!(
            effective_max_context_tokens("openai/gpt-5", 0, false, None),
            400_000
        );

        let caps = resolve_model_capabilities("google/gemini-2.5-flash", None);
        assert_eq!(caps.max_context_tokens, 1_048_576);
        assert_eq!(caps.reasoning.support, ReasoningSupport::Configurable);

        let gs_caps = resolve_model_capabilities("Google-Search-Model", None);
        assert_eq!(gs_caps.max_context_tokens, 32_768);
        assert_eq!(gs_caps.reasoning.support, ReasoningSupport::None);
        assert!(gs_caps.reasoning.levels.is_empty());
        assert_eq!(gs_caps.reasoning.default_level, None);
        assert_eq!(gs_caps.reasoning.source, "experimental_builtin");

        let gs_exp_caps = resolve_model_capabilities("experimental/google-search-model", None);
        assert_eq!(gs_exp_caps.max_context_tokens, 32_768);
        assert_eq!(gs_exp_caps.reasoning.support, ReasoningSupport::None);
        assert_eq!(gs_exp_caps.reasoning.source, "experimental_builtin");
    }
}
