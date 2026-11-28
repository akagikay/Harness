//! Deterministic, dependency-free skill router.
//!
//! Given a user message plus the skills the current workspace exposes, the router
//! returns a ranked list of skill matches so a matched skill's instructions can be
//! injected as a labelled `[SYSTEM COMMAND DIRECTIVE /<skill>]` block without the
//! user having to type `/skill-name` by hand.
//!
//! Design goals:
//! - **Explainable.** Every match reports the terms that produced its score, and
//!   the score is a plain weighted sum (see [`SCORE_DENOMINATOR`], the weight
//!   constants below). No hidden state, no model calls, no randomness.
//! - **Safe.** A skill is only ever injected when its confidence clears
//!   [`DEFAULT_MIN_CONFIDENCE`]; the leader must also beat the runner-up by
//!   [`DEFAULT_MIN_MARGIN`] before it is treated as a single, dominant intent.
//!   Multi-intent messages get at most [`DEFAULT_MAX_SKILLS`] skills, and an
//!   explicit `/slash` command always wins (the caller never routes when a slash
//!   command is present).
//! - **Cheap.** Word stemming + set membership only, so routing costs nothing at
//!   runtime and is fully testable.

use crate::config::SkillDefinition;

/// Minimum confidence (0.0..=1.0) for a skill to be injected at all.
pub const DEFAULT_MIN_CONFIDENCE: f32 = 0.35;
/// Maximum number of skills injected for one message (multi-intent cap).
pub const DEFAULT_MAX_SKILLS: usize = 2;
/// Confidence gap the leader must hold over the next-best match before it is
/// treated as one dominant intent instead of an ambiguous multi-intent message.
pub const DEFAULT_MIN_MARGIN: f32 = 0.05;
/// Hard safety cap: never inject more than this many skills, whatever the config
/// says, so a misconfigured value cannot flood the context.
pub const MAX_INJECTABLE_SKILLS: usize = 4;

/// Confidence saturation constant: `confidence = raw / (raw + SCORE_DENOMINATOR)`.
/// Chosen so a single trigger hit (1.2) lands at ~0.32 (below the default gate)
/// while two trigger hits (~0.48) or one hit plus a name match (~0.46) clear it:
/// routing needs more than one signal.
const SCORE_DENOMINATOR: f32 = 2.6;

/// Weight of one matched trigger keyword.
const TRIGGER_HIT_WEIGHT: f32 = 1.2;
/// Trigger matches above this count add nothing (one strong skill rarely needs more).
const MAX_TRIGGER_HITS: usize = 3;
/// Weight of matching the whole skill name (every name part is present).
const NAME_HIT_WEIGHT: f32 = 1.0;
/// Weight of matching one `-`/`_` separated part of the skill name.
const NAME_PART_WEIGHT: f32 = 0.5;
/// Weight of one low-signal hint (description/body keyword). Hints alone can never
/// reach the default confidence gate: [`MAX_HINT_HITS`] of them score 1.05 (~0.29).
const HINT_HIT_WEIGHT: f32 = 0.35;
const MAX_HINT_HITS: usize = 3;
/// Weight of an `@tool` mention the skill actually references. A hint only: it can
/// lift a skill over the gate, but a missing mention never blocks a match.
const TOOL_HINT_WEIGHT: f32 = 0.5;
const MAX_TOOL_HINTS: usize = 2;
/// Keywords shorter than this are ignored entirely.
const MIN_KEYWORD_LEN: usize = 3;
/// A keyword this long also matches by prefix (`research` ~ `researcher`).
/// A keyword shorter than this must match a stem exactly; prefix matching only
/// kicks in at 5+ characters (see [`keyword_matches`]).
const PREFIX_MATCH_MIN_LEN: usize = 5;

/// Low-signal words that carry no routing information on their own.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "any", "can", "her", "was", "one",
    "our", "out", "get", "has", "him", "his", "how", "its", "new", "now", "old", "see", "two",
    "who", "did", "she", "too", "use", "that", "this", "with", "have", "from", "they", "know",
    "want", "been", "good", "much", "some", "time", "very", "when", "just", "like", "long",
    "make", "many", "more", "only", "over", "such", "take", "than", "them", "well", "what",
    "will", "your", "about", "after", "again", "being", "below", "between", "both", "could",
    "does", "done", "down", "each", "else", "even", "ever", "every", "here", "into", "let",
    "lets", "made", "most", "must", "need", "next", "once", "other", "please", "really",
    "same", "should", "since", "still", "then", "there", "these", "those", "under", "until",
    "upon", "also", "away", "back", "because", "before", "best", "better", "cant", "doing",
    "don", "few", "give", "going", "gone", "got", "great", "had", "hard", "help", "hi", "hey",
    "hello", "huh", "isnt", "keep", "kind", "last", "later", "least", "left", "less", "lot",
    "maybe", "mean", "might", "mine", "near", "never", "off", "ok", "okay", "own", "per",
    "put", "rather", "right", "said", "say", "says", "seem", "seen", "set", "shall", "sure",
    "thanks", "thank", "thx", "yes", "yep", "no", "nope", "yo", "sup", "lol", "th", "us", "we",
    "why", "yet", "thing", "things", "stuff", "way", "ways", "able", "ask", "asks", "try",
    "tries", "tried", "using", "used", "uses", "via", "etc", "im", "ive", "id", "youre",
    "youll", "youve", "ur", "pls", "would", "may", "dont", "wont", "arent", "wasnt", "got",
];

/// Routing knobs, derived from `HarnessConfig`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouterSettings {
    /// Minimum confidence before a skill may be injected.
    pub min_confidence: f32,
    /// Maximum skills injected for one message (capped by [`MAX_INJECTABLE_SKILLS`]).
    pub max_skills: usize,
    /// Confidence gap that marks the leader as a single, dominant intent.
    pub min_margin: f32,
}

impl Default for RouterSettings {
    fn default() -> Self {
        Self {
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            max_skills: DEFAULT_MAX_SKILLS,
            min_margin: DEFAULT_MIN_MARGIN,
        }
    }
}

impl RouterSettings {
    /// Clamp untrusted config values into a sane, finite range.
    pub fn clamped(&self) -> Self {
        let min_confidence = if self.min_confidence.is_finite() {
            self.min_confidence.clamp(0.0, 1.0)
        } else {
            DEFAULT_MIN_CONFIDENCE
        };
        let min_margin = if self.min_margin.is_finite() {
            self.min_margin.clamp(0.0, 1.0)
        } else {
            DEFAULT_MIN_MARGIN
        };
        let max_skills = self.max_skills.min(MAX_INJECTABLE_SKILLS);
        Self { min_confidence, max_skills, min_margin }
    }
}

/// A skill in the shape the router scores it: high-signal `triggers` plus
/// low-signal `hints`.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutableSkill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub path: String,
    /// High-signal keywords (skill name parts plus a curated list).
    pub triggers: Vec<String>,
    /// Low-signal keywords (description/body words) that only add small weight.
    pub hints: Vec<String>,
}

impl RoutableSkill {
    /// Build a routable view of a shipped built-in skill.
    pub fn from_builtin(skill: &crate::builtin_skills::BuiltinSkill) -> Self {
        let mut triggers = name_parts(&skill.name);
        for t in skill.triggers {
            triggers.push((*t).to_string());
        }
        let mut hints = keywords(&skill.description, 8);
        hints.extend(frequent_keywords(&skill.content, 6));
        Self {
            name: skill.name.to_string(),
            description: skill.description.to_string(),
            content: skill.content.to_string(),
            path: format!("builtin://{}", skill.name),
            triggers: expand_keywords(triggers),
            hints: dedupe(hints),
        }
    }

    /// Build a routable view of a discovered workspace skill. Workspace skills
    /// carry no explicit trigger list, so their name parts and description/body
    /// keywords act as signals.
    pub fn from_definition(def: &SkillDefinition) -> Self {
        Self {
            name: def.name.clone(),
            description: def.description.clone(),
            content: def.content.clone(),
            path: def.path.clone(),
            triggers: expand_keywords(name_parts(&def.name)),
            hints: dedupe(
                keywords(&def.description, 8)
                    .into_iter()
                    .chain(frequent_keywords(&def.content, 6))
                    .collect(),
            ),
        }
    }

    /// Build a routable view of a discovered definition, merging in the matching
    /// built-in's curated `triggers` when one shares its name. Discovered
    /// built-ins carry no trigger list of their own, so this keeps routed
    /// built-in skills as precise as skills routed straight from
    /// [`BUILTIN_SKILLS`](crate::builtin_skills::BUILTIN_SKILLS).
    pub fn from_definition_merged(
        def: &SkillDefinition,
        builtins: &[crate::builtin_skills::BuiltinSkill],
    ) -> Self {
        let mut merged = Self::from_definition(def);
        if let Some(builtin) = builtins.iter().find(|b| b.name == def.name) {
            let mut triggers = name_parts(&def.name);
            triggers.extend(builtin.triggers.iter().map(|s| s.to_string()));
            merged.triggers = expand_keywords(triggers);
        }
        merged
    }
}

/// One routed skill plus the evidence that produced the decision.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillMatch {
    pub name: String,
    pub path: String,
    pub description: String,
    pub content: String,
    /// Saturated 0.0..=1.0 confidence: `raw / (raw + SCORE_DENOMINATOR)`.
    pub confidence: f32,
    /// The raw weighted sum behind `confidence`.
    pub score: f32,
    /// Trigger/name keywords that matched the message.
    pub matched_terms: Vec<String>,
    /// `@tool` mentions the skill references.
    pub tool_hints: Vec<String>,
}

/// Case-insensitive, stem-friendly view of a user message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessageFeatures {
    /// Stems of the meaningful words in the message.
    pub stems: Vec<String>,
    /// `@tool` mentions (without the `@`), e.g. `bash`, `browser_ui`.
    pub tool_mentions: Vec<String>,
}

/// Tokenize a message into stems plus `@tool` mentions.
pub fn analyze_message(message: &str) -> MessageFeatures {
    const TRIM: &[char] = &[
        '.', ',', ';', ':', '!', '?', '(', ')', '[', ']', '{', '}', '"', '\'', '`', '*', '<',
        '>', '=', '|', '\\', '/', '~', '#', '$', '%', '^', '&', '+',
    ];
    let mut stems: Vec<String> = Vec::new();
    let mut tool_mentions: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for raw in message.split_whitespace() {
        let cleaned = raw.trim_matches(|c| TRIM.contains(&c));
        if cleaned.is_empty() {
            continue;
        }
        if let Some(rest) = cleaned.strip_prefix('@') {
            let name = rest.trim_matches(|c| TRIM.contains(&c));
            // An @mention is a routing hint, never a requirement.
            if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && !name.is_empty()
            {
                push_unique(&mut tool_mentions, &mut seen, &name.to_ascii_lowercase());
            }
            continue;
        }
        // Keep the whole token (`read_file`) and its parts (`read`, `file`) so a
        // trigger can match either form.
        push_token(cleaned, &mut stems, &mut seen);
        for part in cleaned.split(|c| c == '_' || c == '-') {
            if part.len() >= MIN_KEYWORD_LEN {
                push_token(part, &mut stems, &mut seen);
            }
        }
    }

    MessageFeatures { stems, tool_mentions }
}

fn push_token(
    raw: &str,
    stems: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
) {
    let lower = raw.to_ascii_lowercase();
    let stemmed = stem(&lower);
    if stemmed.len() < MIN_KEYWORD_LEN || STOPWORDS.contains(&stemmed.as_str()) {
        return;
    }
    push_unique(stems, seen, &stemmed);
}

fn push_unique(
    out: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
    value: &str,
) {
    if seen.insert(value.to_string()) {
        out.push(value.to_string());
    }
}

/// Very small suffix stemmer: `researching` -> `research`, `tests` -> `test`.
/// It is intentionally lossy and symmetric, which is all the router needs.
/// Compound words containing `-` or `_` are left whole: `read_file` must stay a
/// tool name, not become `read_file`.
pub fn stem(word: &str) -> String {
    let w = word.to_ascii_lowercase();
    if w.contains('-') || w.contains('_') {
        return w;
    }
    let strip = |suffix: &str| w.trim_end_matches(suffix).to_string();
    if w.len() > 4 && w.ends_with("ies") {
        return format!("{}y", &w[..w.len() - 3]);
    }
    if w.len() > 5 && w.ends_with("ing") {
        return strip("ing");
    }
    if w.len() > 4 && w.ends_with("ed") {
        return strip("ed");
    }
    if w.len() > 4 && w.ends_with("es") {
        return strip("es");
    }
    if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") {
        return strip("s");
    }
    w
}

/// The full skill name plus each `-`/`_` separated part, longest first.
pub fn name_parts(name: &str) -> Vec<String> {
    let mut parts = vec![name.to_string()];
    for part in name.split(|c| c == '-' || c == '_') {
        if part.len() >= MIN_KEYWORD_LEN {
            parts.push(part.to_string());
        }
    }
    dedupe(parts)
}

/// First N meaningful words of a text, in order (for descriptions).
fn keywords(text: &str, limit: usize) -> Vec<String> {
    let features = analyze_message(text);
    features
        .stems
        .into_iter()
        .filter(|s| s.len() > 3)
        .take(limit)
        .collect()
}

/// The N most frequent meaningful words of a body, so workspace skills route on
/// their subject matter without an explicit trigger list.
fn frequent_keywords(text: &str, limit: usize) -> Vec<String> {
    let features = analyze_message(text);
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for s in features.stems {
        if s.len() <= 3 {
            continue;
        }
        if !counts.contains_key(&s) {
            order.push(s.clone());
        }
        *counts.entry(s).or_insert(0) += 1;
    }
    order.sort_by(|a, b| counts[b].cmp(&counts[a]).then_with(|| a.cmp(b)));
    order.into_iter().take(limit).collect()
}

fn dedupe(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

/// True when a message stem matches a keyword, exactly or by prefix.
///
/// Prefix matching is one-directional and single-word only: `refactor` matches
/// `refactoring`, but the phrase `refactor plan` never matches just `refactor`,
/// and `skill-authoring` never matches just `skill` (that is a name *part* hit).
///
/// Prefixes are only trusted at 5+ characters. A 4-letter abbreviation such as
/// `diff` (a `reviewer` trigger) is a *word*, not a stem family: letting it
/// prefix-match made `diff` fire on "different", which routed a plain
/// game-building task to the reviewer skill on a 0.37-confidence ghost match.
/// Short keywords must therefore match exactly.
fn keyword_matches(keyword: &str, stems: &[String]) -> bool {
    let key = stem(keyword);
    if key.len() < MIN_KEYWORD_LEN || key.contains(char::is_whitespace) {
        return false;
    }
    stems
        .iter()
        .any(|s| *s == key || (key.len() >= PREFIX_MATCH_MIN_LEN && s.starts_with(&key)))
}

/// Expand keywords into individually matchable terms: the whole entry plus each
/// whitespace-separated word, so a phrase trigger like `multi-agent review` also
/// matches a message that only says `review`.
fn expand_keywords<I: IntoIterator<Item = String>>(items: I) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        let lower = item.to_ascii_lowercase();
        out.push(lower.clone());
        for word in lower.split_whitespace() {
            if word.len() >= MIN_KEYWORD_LEN {
                out.push(word.to_string());
            }
        }
    }
    dedupe(out)
}

/// Score every skill against a message and return the ones worth injecting,
/// ranked best-first.
pub fn route_skills(
    message: &str,
    skills: &[RoutableSkill],
    settings: RouterSettings,
) -> Vec<SkillMatch> {
    let settings = settings.clamped();
    if settings.max_skills == 0 {
        return Vec::new();
    }
    let features = analyze_message(message);
    if features.stems.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<SkillMatch> = skills
        .iter()
        .filter_map(|skill| score_skill(&features, skill))
        .collect();
    // Rank by confidence, then by name so equal scores are stable.
    scored.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });

    let Some(leader) = scored.first() else {
        return Vec::new();
    };
    // Absolute gate: a low-signal message never injects anything.
    if leader.confidence < settings.min_confidence {
        return Vec::new();
    }
    // Relative gate: only a clear leader is a single intent. A near-tie means the
    // message carries several intents, so the top candidates are injected together.
    let dominant = scored
        .get(1)
        .map(|runner_up| leader.confidence - runner_up.confidence >= settings.min_margin)
        .unwrap_or(true);
    if dominant {
        return vec![leader.clone()];
    }

    scored
        .into_iter()
        .filter(|m| m.confidence >= settings.min_confidence)
        .take(settings.max_skills)
        .collect()
}

/// Weighted scoring of one skill; `None` when nothing matched.
fn score_skill(features: &MessageFeatures, skill: &RoutableSkill) -> Option<SkillMatch> {
    let mut matched: Vec<String> = Vec::new();
    // Count each distinct keyword once: a keyword repeated in the trigger list
    // (the name and an explicit trigger often coincide) must not score twice.
    let mut seen_triggers: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut trigger_hits = 0usize;
    for keyword in &skill.triggers {
        let stemmed = stem(keyword);
        if STOPWORDS.contains(&stemmed.as_str()) || !seen_triggers.insert(stemmed.clone()) {
            continue;
        }
        if keyword_matches(keyword, &features.stems) {
            trigger_hits += 1;
            matched.push(keyword.clone());
        }
    }

    // Name matching: the full name (every part present) is worth more than a lone
    // part, so `skill` alone does not fully fire `skill-authoring`.
    let parts: Vec<&str> = skill.name.split(|c| c == '-' || c == '_').collect();
    let whole_name = keyword_matches(&skill.name, &features.stems)
        || (parts.len() > 1
            && parts
                .iter()
                .all(|p| keyword_matches(p, &features.stems)));
    let part_hit = parts
        .iter()
        .any(|p| keyword_matches(p, &features.stems));
    if whole_name || part_hit {
        matched.push(skill.name.clone());
    }

    let mut seen_hints: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut hint_hits = 0usize;
    for keyword in &skill.hints {
        let stemmed = stem(keyword);
        if STOPWORDS.contains(&stemmed.as_str()) || !seen_hints.insert(stemmed) {
            continue;
        }
        if keyword_matches(keyword, &features.stems) {
            hint_hits += 1;
        }
    }

    let mut tool_hints: Vec<String> = Vec::new();
    let haystack = skill.content.to_ascii_lowercase();
    for mention in &features.tool_mentions {
        let name = mention.to_ascii_lowercase();
        let referenced = name.len() >= MIN_KEYWORD_LEN
            && (haystack.contains(&name) || skill.triggers.iter().any(|t| *t == name));
        if referenced {
            tool_hints.push(mention.clone());
        }
    }

    let name_score = if whole_name {
        NAME_HIT_WEIGHT
    } else if part_hit {
        NAME_PART_WEIGHT
    } else {
        0.0
    };
    let score = TRIGGER_HIT_WEIGHT * trigger_hits.min(MAX_TRIGGER_HITS) as f32
        + name_score
        + HINT_HIT_WEIGHT * hint_hits.min(MAX_HINT_HITS) as f32
        + TOOL_HINT_WEIGHT * tool_hints.len().min(MAX_TOOL_HINTS) as f32;
    if score <= 0.0 || matched.is_empty() {
        return None;
    }

    Some(SkillMatch {
        name: skill.name.clone(),
        path: skill.path.clone(),
        description: skill.description.clone(),
        content: skill.content.clone(),
        confidence: score / (score + SCORE_DENOMINATOR),
        score,
        matched_terms: dedupe(matched),
        tool_hints,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic skill with no hints, so a test controls every signal.
    fn skill(name: &str, triggers: &[&str]) -> RoutableSkill {
        let mut all = name_parts(name);
        all.extend(triggers.iter().map(|s| s.to_string()));
        RoutableSkill {
            name: name.to_string(),
            description: format!("A {name} skill for testing."),
            content: "Body text with tools like bash and read_file mentioned.".to_string(),
            path: format!("builtin://{name}"),
            triggers: dedupe(all),
            hints: Vec::new(),
        }
    }

    fn research_and_code() -> Vec<RoutableSkill> {
        vec![
            skill(
                "web-research",
                &["research", "cite", "source", "web", "search"],
            ),
            skill(
                "implement-code",
                &["implement", "refactor", "fix", "bug", "build", "test"],
            ),
        ]
    }

    #[test]
    fn clear_match_routes_to_the_expected_skill() {
        let matches = route_skills(
            "Research the latest Rust async patterns and cite your sources",
            &research_and_code(),
            RouterSettings::default(),
        );
        assert_eq!(matches.len(), 1, "a clear message must inject exactly one skill");
        assert_eq!(matches[0].name, "web-research");
        assert!(matches[0].confidence >= DEFAULT_MIN_CONFIDENCE);
        assert!(matches[0].matched_terms.iter().any(|t| t == "research"));
        assert!(matches[0].matched_terms.iter().any(|t| t == "cite"));
    }

    #[test]
    fn low_signal_messages_route_to_nothing() {
        let skills = research_and_code();
        for msg in ["ok", "thanks", "hi", "yes", "please", "", "lol", "hey there", "sounds good"] {
            assert!(
                route_skills(msg, &skills, RouterSettings::default()).is_empty(),
                "'{msg}' must not route"
            );
        }
    }

    #[test]
    fn tool_mention_is_a_hint_not_a_requirement() {
        let skills = vec![
            skill(
                "implement-code",
                &["implement", "refactor", "fix", "bug", "build", "test"],
            ),
            skill("browser-ui-audit", &["browser", "browsing", "screenshot", "page"]),
        ];
        // Without any @mention the code skill still routes on its own keywords.
        let no_hint = route_skills(
            "Fix the failing build and refactor the parser",
            &skills,
            RouterSettings::default(),
        );
        assert_eq!(no_hint.len(), 1);
        assert_eq!(no_hint[0].name, "implement-code");
        assert!(no_hint[0].tool_hints.is_empty());

        // An @bash mention still routes, and the hint is recorded.
        let with_hint = route_skills(
            "Fix the failing build and refactor the parser, run @bash to check",
            &skills,
            RouterSettings::default(),
        );
        assert_eq!(with_hint.len(), 1);
        assert_eq!(with_hint[0].name, "implement-code");
        assert_eq!(with_hint[0].tool_hints, vec!["bash".to_string()]);

        // But a bare mention with no other signal must not force a match.
        assert!(route_skills("ping @bash", &skills, RouterSettings::default()).is_empty());
    }

    #[test]
    fn multi_intent_picks_top_two_up_to_the_cap() {
        // Two skills whose names never appear in the message, with disjoint and
        // equally strong triggers: a genuine multi-intent message.
        let skills = vec![
            skill("one", &["alpha", "beta"]),
            skill("two", &["gamma", "delta"]),
            skill("three", &["zebra"]),
        ];
        let matches = route_skills(
            "alpha beta gamma delta",
            &skills,
            RouterSettings::default(),
        );
        let names: Vec<&str> = matches.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names.len(), 2, "near-tie must be treated as multi-intent");
        assert!(names.contains(&"one"));
        assert!(names.contains(&"two"));

        // With the cap lowered to 1 only the leader survives.
        let capped = route_skills(
            "alpha beta gamma delta",
            &skills,
            RouterSettings { max_skills: 1, ..RouterSettings::default() },
        );
        assert_eq!(capped.len(), 1);
    }

    #[test]
    fn dominant_intent_injects_only_the_leader() {
        let skills = vec![
            skill(
                "web-research",
                &["research", "cite", "source", "web", "search", "paper"],
            ),
            skill("implement-code", &["implement", "refactor", "fix", "bug", "build", "test"]),
        ];
        // Research keywords dominate; the incidental word overlap is not a second intent.
        let matches = route_skills(
            "Research, cite and summarise the latest papers on async runtimes",
            &skills,
            RouterSettings::default(),
        );
        assert_eq!(matches.len(), 1, "incidental overlap must not add a skill");
        assert_eq!(matches[0].name, "web-research");
    }

    #[test]
    fn threshold_blocks_weak_matches() {
        let skills = vec![skill("web-research", &["research", "cite", "source"])];
        let msg = "cite the source of that research paper";
        assert_eq!(route_skills(msg, &skills, RouterSettings::default()).len(), 1);
        let strict = route_skills(
            msg,
            &skills,
            RouterSettings { min_confidence: 0.95, ..RouterSettings::default() },
        );
        assert!(strict.is_empty(), "a high threshold must block injection");
        // A zero cap (router disabled) also injects nothing.
        assert!(route_skills(
            msg,
            &skills,
            RouterSettings { max_skills: 0, ..RouterSettings::default() }
        )
        .is_empty());
    }

    #[test]
    fn margin_separates_single_intent_from_multi_intent() {
        // Skill names are chosen so no name-part bonus applies.
        let skills = vec![
            skill("one", &["alpha", "beta", "delta"]),
            skill("two", &["beta", "gamma", "delta"]),
        ];
        // Clear leader: three strong hits vs two, so the margin rule keeps it single.
        let single = route_skills("alpha beta delta", &skills, RouterSettings::default());
        assert_eq!(single.len(), 1, "a clear leader must not pull in the runner-up");
        assert_eq!(single[0].name, "one");
        // Exact tie on two strong hits each: multi intent.
        let tie = route_skills("beta delta", &skills, RouterSettings::default());
        assert_eq!(tie.len(), 2);
    }

    #[test]
    fn settings_are_clamped_and_finite() {
        let bad = RouterSettings {
            min_confidence: f32::NAN,
            max_skills: 99,
            min_margin: -1.0,
        };
        let fixed = bad.clamped();
        assert_eq!(fixed.min_confidence, DEFAULT_MIN_CONFIDENCE);
        assert_eq!(fixed.max_skills, MAX_INJECTABLE_SKILLS);
        assert_eq!(fixed.min_margin, 0.0);
    }

    #[test]
    fn stemming_matches_word_variants() {
        let features = analyze_message("researching refactoring tests browsers");
        assert!(features.stems.contains(&"research".to_string()));
        assert!(features.stems.contains(&"refactor".to_string()));
        assert!(keyword_matches("research", &features.stems));
        assert!(keyword_matches("refactor", &features.stems));
        assert!(keyword_matches("browser", &features.stems));
        assert!(!keyword_matches("zebra", &features.stems));
        assert!(analyze_message("hi ok").stems.is_empty());
    }

    #[test]
    fn real_builtin_skills_route_end_to_end() {
        let skills: Vec<RoutableSkill> = crate::builtin_skills::BUILTIN_SKILLS
            .iter()
            .map(RoutableSkill::from_builtin)
            .collect();
        let hit = route_skills(
            "Refactor the router and run the tests to verify it",
            &skills,
            RouterSettings::default(),
        );
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].name, "code");
        assert!(hit[0].confidence >= DEFAULT_MIN_CONFIDENCE);

        let ui = route_skills(
            "Audit the browser page for console errors and fix the layout bug",
            &skills,
            RouterSettings::default(),
        );
        assert_eq!(ui.len(), 1, "a dominant UI-intent message must stay single");
        assert_eq!(ui[0].name, "browser-ui-audit");

        // A genuine two-intent message routes to both, in rank order.
        let both = route_skills(
            "Research and cite the latest sources, then implement and test the fix",
            &skills,
            RouterSettings::default(),
        );
        let names: Vec<&str> = both.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["web-research", "code"]);

        assert!(route_skills("ok", &skills, RouterSettings::default()).is_empty());
    }

    #[test]
    fn definitions_and_builtins_both_route() {
        let builtin = RoutableSkill::from_builtin(&crate::builtin_skills::BUILTIN_SKILLS[0]);
        assert!(!builtin.triggers.is_empty());
        assert!(!builtin.hints.is_empty());
        assert!(builtin.path.starts_with("builtin://"));

        let def = SkillDefinition {
            name: "release-notes".to_string(),
            description: "Draft release notes from the changelog and tags".to_string(),
            content: "release notes changelog tags version".to_string(),
            path: ".harness/skills/release-notes.md".to_string(),
            enabled: true,
        };
        let workspace = RoutableSkill::from_definition(&def);
        assert!(workspace.triggers.iter().any(|t| t == "release"));
        let matches = route_skills(
            "Draft the release notes for the new version",
            std::slice::from_ref(&workspace),
            RouterSettings::default(),
        );
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "release-notes");
    }
}