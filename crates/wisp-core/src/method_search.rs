//! Versioned contracts and deterministic validation for Workflow-native method search.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

pub const METHOD_SEARCH_SCHEMA_V1: &str = "wisp.method-search.v1";
pub const EVALUATOR_PROTOCOL_V1: &str = "wisp_evaluate_jsonl_v1";
pub const EVALUATOR_RESULT_PREFIX: &str = "wisp_evaluate: ";
pub const MAX_EVALUATOR_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_CANDIDATE_SOURCE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PythonSymbolKind {
    Function,
    AsyncFunction,
    Class,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonSymbolSpan {
    pub kind: PythonSymbolKind,
    pub name: String,
    pub start: usize,
    pub header_end: usize,
    pub end: usize,
    pub body_indent: String,
}

fn python_header(line: &str, symbol: &str) -> Option<PythonSymbolKind> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let line = line.trim_start();
    for (prefix, kind) in [
        ("async def ", PythonSymbolKind::AsyncFunction),
        ("def ", PythonSymbolKind::Function),
        ("class ", PythonSymbolKind::Class),
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let boundary = rest
                .find(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .unwrap_or(rest.len());
            return (rest[..boundary] == *symbol).then_some(kind);
        }
    }
    None
}

fn python_header_complete(header: &str) -> bool {
    let mut stack = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    for character in header.chars() {
        if comment {
            if character == '\n' {
                comment = false;
            }
            continue;
        }
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active {
                quote = None;
            }
            continue;
        }
        match character {
            '#' => comment = true,
            '\'' | '"' => quote = Some(character),
            '(' | '[' | '{' => stack.push(character),
            ')' | ']' | '}' => {
                stack.pop();
            }
            ':' if stack.is_empty() => return true,
            _ => {}
        }
    }
    false
}

pub fn locate_python_symbol(source: &str, symbol: &str) -> anyhow::Result<PythonSymbolSpan> {
    if source.len() > MAX_CANDIDATE_SOURCE_BYTES || !valid_text(symbol, 256) {
        anyhow::bail!("Python target source or symbol is outside v1 limits");
    }
    let lines = source.split_inclusive('\n').collect::<Vec<_>>();
    let mut offsets = Vec::with_capacity(lines.len());
    let mut offset = 0usize;
    for line in &lines {
        offsets.push(offset);
        offset += line.len();
    }
    let mut matches = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if let Some(kind) = python_header(line, symbol) {
            matches.push((index, kind));
        }
    }
    if matches.len() != 1 {
        anyhow::bail!("Python target symbol must have exactly one top-level definition");
    }
    let (start_line, kind) = matches[0];
    let mut header_end_line = None;
    let mut header = String::new();
    for (index, line) in lines.iter().enumerate().skip(start_line) {
        header.push_str(line);
        if python_header_complete(&header) {
            header_end_line = Some(index);
            break;
        }
    }
    let header_end_line = header_end_line
        .ok_or_else(|| anyhow::anyhow!("Python target definition has an incomplete header"))?;
    let header_end = offsets[header_end_line] + lines[header_end_line].len();
    let mut body_indent = None;
    let mut end = source.len();
    for (index, line) in lines.iter().enumerate().skip(header_end_line + 1) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indentation = line.len() - line.trim_start_matches([' ', '\t']).len();
        if indentation == 0 {
            end = offsets[index];
            break;
        }
        if body_indent.is_none() && !trimmed.starts_with('#') {
            body_indent = Some(line[..indentation].to_string());
        }
    }
    let body_indent = body_indent
        .ok_or_else(|| anyhow::anyhow!("Python target definition has no indented body"))?;
    Ok(PythonSymbolSpan {
        kind,
        name: symbol.into(),
        start: offsets[start_line],
        header_end,
        end,
        body_indent,
    })
}

pub fn inject_python_reachability_sentinel(source: &str, symbol: &str) -> anyhow::Result<String> {
    let span = locate_python_symbol(source, symbol)?;
    let mut output = String::with_capacity(source.len() + 96);
    output.push_str(&source[..span.header_end]);
    output.push_str(&span.body_indent);
    output.push_str("raise RuntimeError(\"wisp_method_search_reachability_sentinel\")\n");
    output.push_str(&source[span.header_end..]);
    Ok(output)
}

fn normalized_python_header(source: &str, span: &PythonSymbolSpan) -> String {
    source[span.start..span.header_end]
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

pub fn replace_python_symbol(
    baseline: &str,
    symbol: &str,
    replacement: &str,
) -> anyhow::Result<String> {
    if replacement.len() > MAX_CANDIDATE_SOURCE_BYTES {
        anyhow::bail!("candidate replacement exceeds the source-size limit");
    }
    let baseline_span = locate_python_symbol(baseline, symbol)?;
    let replacement_span = locate_python_symbol(replacement, symbol)?;
    if baseline_span.kind != replacement_span.kind
        || normalized_python_header(baseline, &baseline_span)
            != normalized_python_header(replacement, &replacement_span)
    {
        anyhow::bail!("candidate replacement changed the target kind or signature");
    }
    if !replacement[..replacement_span.start].trim().is_empty()
        || !replacement[replacement_span.end..].trim().is_empty()
    {
        anyhow::bail!("candidate response must contain only the declared Python symbol");
    }
    let mut output = String::with_capacity(baseline.len() + replacement.len());
    output.push_str(&baseline[..baseline_span.start]);
    output.push_str(replacement.trim_end());
    output.push('\n');
    output.push_str(&baseline[baseline_span.end..]);
    Ok(output)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreDirection {
    Maximize,
    Minimize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailOperator {
    Lte,
    Lt,
    Gte,
    Gt,
    Eq,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodSearchTarget {
    pub language: String,
    pub source_artifact_version_id: String,
    pub source_path: String,
    pub symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodSearchEvaluatorSpec {
    pub artifact_version_id: String,
    pub entry_path: String,
    pub repetitions: u32,
    pub timeout_seconds: u64,
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodSearchGuardrail {
    pub metric: String,
    pub op: GuardrailOperator,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodSearchMetrics {
    pub primary: String,
    pub direction: ScoreDirection,
    #[serde(default)]
    pub guardrails: Vec<MethodSearchGuardrail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodSearchInput {
    pub role: String,
    /// Portable project-relative reconstruction path. Frozen inputs must be
    /// restorable without consulting the mutable checkout.
    pub path: String,
    #[serde(default)]
    pub artifact_version_id: Option<String>,
    #[serde(default)]
    pub external_resource_id: Option<String>,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodSearchBudget {
    pub max_candidates: u32,
    pub max_wall_seconds: u64,
    pub max_evaluator_seconds: u64,
    pub max_cost_microunits: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalVerificationSpec {
    pub artifact_version_id: String,
    pub path: String,
    pub repetitions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodStrategySource {
    /// Stable paper/resource identity such as a DOI, PMID, URL, or existing
    /// Research Graph reference. It is evidence attribution, not executable
    /// authority.
    pub source_ref: String,
    pub title: String,
    pub summary: String,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodSearchSpec {
    pub schema: String,
    pub objective: String,
    pub target: MethodSearchTarget,
    pub evaluator: MethodSearchEvaluatorSpec,
    pub metrics: MethodSearchMetrics,
    pub inputs: Vec<MethodSearchInput>,
    #[serde(default)]
    pub protected_paths: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub strategy_sources: Vec<MethodStrategySource>,
    pub budget: MethodSearchBudget,
    #[serde(default)]
    pub final_verification: Option<FinalVerificationSpec>,
}

fn valid_text(value: &str, max_chars: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= max_chars && value == value.trim()
}

fn valid_checksum(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_portable_relative_path(value: &str) -> bool {
    valid_text(value, 1_024)
        && !value.starts_with('/')
        && !value.contains(['\\', '\0', ':'])
        && value
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

impl MethodSearchSpec {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.schema != METHOD_SEARCH_SCHEMA_V1 {
            anyhow::bail!("unsupported method-search schema: {}", self.schema);
        }
        if !valid_text(&self.objective, 4_000) {
            anyhow::bail!("method-search objective is missing or too large");
        }
        if self.target.language != "python"
            || !valid_text(&self.target.source_artifact_version_id, 256)
            || !valid_text(&self.target.source_path, 1_024)
            || !valid_text(&self.target.symbol, 256)
            || !valid_portable_relative_path(&self.target.source_path)
        {
            anyhow::bail!("method-search v1 requires one exact Python target");
        }
        if !valid_text(&self.evaluator.artifact_version_id, 256)
            || !valid_text(&self.evaluator.entry_path, 1_024)
            || !valid_portable_relative_path(&self.evaluator.entry_path)
            || !(3..=10).contains(&self.evaluator.repetitions)
            || self.evaluator.timeout_seconds == 0
            || self.evaluator.timeout_seconds > 300
            || self.evaluator.protocol != EVALUATOR_PROTOCOL_V1
        {
            anyhow::bail!("method-search evaluator contract is outside v1 limits");
        }
        if !valid_text(&self.metrics.primary, 128) {
            anyhow::bail!("method-search primary metric is required");
        }
        let mut metric_names = HashSet::new();
        for guardrail in &self.metrics.guardrails {
            if !valid_text(&guardrail.metric, 128)
                || guardrail.metric == self.metrics.primary
                || !guardrail.value.is_finite()
                || !metric_names.insert(guardrail.metric.as_str())
            {
                anyhow::bail!("method-search guardrails must be finite and uniquely named");
            }
        }
        if self.inputs.is_empty() || self.inputs.len() > 64 {
            anyhow::bail!("method-search requires between 1 and 64 exact inputs");
        }
        let mut roles = HashSet::new();
        for input in &self.inputs {
            let exactly_one_owner =
                input.artifact_version_id.is_some() ^ input.external_resource_id.is_some();
            if !valid_text(&input.role, 128)
                || !valid_portable_relative_path(&input.path)
                || !roles.insert(input.role.as_str())
                || !exactly_one_owner
                || input
                    .artifact_version_id
                    .as_deref()
                    .is_some_and(|value| !valid_text(value, 256))
                || input
                    .external_resource_id
                    .as_deref()
                    .is_some_and(|value| !valid_text(value, 256))
                || !valid_checksum(&input.checksum)
            {
                anyhow::bail!("method-search inputs must have unique roles and exact ownership");
            }
        }
        let mut protected = HashSet::new();
        if self.protected_paths.len() > 128
            || self
                .protected_paths
                .iter()
                .any(|path| !valid_text(path, 1_024) || !protected.insert(path.as_str()))
        {
            anyhow::bail!("method-search protected paths are invalid or duplicated");
        }
        if !self
            .protected_paths
            .iter()
            .any(|path| path == &self.evaluator.entry_path)
        {
            anyhow::bail!("the evaluator entry path must be protected");
        }
        if self.constraints.len() > 64
            || self
                .constraints
                .iter()
                .any(|constraint| !valid_text(constraint, 2_000))
        {
            anyhow::bail!("method-search constraints are invalid or too large");
        }
        let mut source_refs = HashSet::new();
        if self.strategy_sources.len() > 16
            || self.strategy_sources.iter().any(|source| {
                !valid_text(&source.source_ref, 1_024)
                    || !valid_text(&source.title, 500)
                    || !valid_text(&source.summary, 1_000)
                    || !matches!(
                        source.category.as_str(),
                        "literature_or_method"
                            | "diagnostic"
                            | "ablation_or_simplification"
                            | "alternative_family"
                    )
                    || !source_refs.insert(source.source_ref.as_str())
            })
        {
            anyhow::bail!("method-search strategy sources are invalid, duplicated, or too large");
        }
        if !(1..=50).contains(&self.budget.max_candidates)
            || self.budget.max_wall_seconds == 0
            || self.budget.max_wall_seconds > 7 * 24 * 60 * 60
            || self.budget.max_evaluator_seconds == 0
            || self.budget.max_evaluator_seconds > 300
            || self.budget.max_evaluator_seconds > self.evaluator.timeout_seconds
            || self.budget.max_cost_microunits == 0
        {
            anyhow::bail!("method-search budget is outside v1 limits");
        }
        if let Some(final_verification) = &self.final_verification {
            if !valid_text(&final_verification.artifact_version_id, 256)
                || !valid_portable_relative_path(&final_verification.path)
                || !(3..=10).contains(&final_verification.repetitions)
            {
                anyhow::bail!("final-verification contract is invalid");
            }
        }
        Ok(())
    }

    pub fn normalized_utility(&self, primary: f64) -> anyhow::Result<f64> {
        if !primary.is_finite() {
            anyhow::bail!("primary score must be finite");
        }
        Ok(match self.metrics.direction {
            ScoreDirection::Maximize => primary,
            ScoreDirection::Minimize => -primary,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorResult {
    pub primary: f64,
    pub metrics: BTreeMap<String, f64>,
}

impl EvaluatorResult {
    pub fn validate_for(&self, spec: &MethodSearchSpec) -> anyhow::Result<()> {
        if !self.primary.is_finite() || self.metrics.values().any(|value| !value.is_finite()) {
            anyhow::bail!("evaluator metrics must be finite JSON numbers");
        }
        let reported_primary = self
            .metrics
            .get(&spec.metrics.primary)
            .ok_or_else(|| anyhow::anyhow!("evaluator omitted the declared primary metric"))?;
        if reported_primary.to_bits() != self.primary.to_bits() {
            anyhow::bail!("evaluator primary value does not match its named metric");
        }
        for guardrail in &spec.metrics.guardrails {
            if !self.metrics.contains_key(&guardrail.metric) {
                anyhow::bail!("evaluator omitted guardrail metric {}", guardrail.metric);
            }
        }
        Ok(())
    }

    pub fn passes_guardrails(&self, spec: &MethodSearchSpec) -> bool {
        spec.metrics.guardrails.iter().all(|guardrail| {
            let Some(value) = self.metrics.get(&guardrail.metric) else {
                return false;
            };
            match guardrail.op {
                GuardrailOperator::Lte => *value <= guardrail.value,
                GuardrailOperator::Lt => *value < guardrail.value,
                GuardrailOperator::Gte => *value >= guardrail.value,
                GuardrailOperator::Gt => *value > guardrail.value,
                GuardrailOperator::Eq => value.to_bits() == guardrail.value.to_bits(),
            }
        })
    }
}

pub fn parse_evaluator_output(
    stdout: &str,
    spec: &MethodSearchSpec,
) -> anyhow::Result<EvaluatorResult> {
    if stdout.len() > MAX_EVALUATOR_OUTPUT_BYTES {
        anyhow::bail!("evaluator stdout exceeds the bounded protocol limit");
    }
    let lines = stdout
        .lines()
        .filter_map(|line| line.strip_prefix(EVALUATOR_RESULT_PREFIX))
        .collect::<Vec<_>>();
    if lines.len() != 1 {
        anyhow::bail!("evaluator must emit exactly one wisp_evaluate result line");
    }
    let result: EvaluatorResult = serde_json::from_str(lines[0])?;
    result.validate_for(spec)?;
    Ok(result)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineAuditSummary {
    pub repetitions: u32,
    pub successful_repetitions: u32,
    pub failure_rate: f64,
    pub median_primary: f64,
    pub spread: f64,
    pub median_absolute_deviation: f64,
    pub noise_floor: f64,
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

pub fn summarize_baseline(
    requested_repetitions: u32,
    results: &[Result<EvaluatorResult, String>],
) -> anyhow::Result<BaselineAuditSummary> {
    if requested_repetitions < 3 || results.len() != requested_repetitions as usize {
        anyhow::bail!("baseline audit result count does not match requested repetitions");
    }
    let mut scores = results
        .iter()
        .filter_map(|result| result.as_ref().ok().map(|result| result.primary))
        .collect::<Vec<_>>();
    if scores.len() < 3 || scores.iter().any(|score| !score.is_finite()) {
        anyhow::bail!("baseline audit requires at least three finite successful repetitions");
    }
    let min = scores.iter().copied().min_by(f64::total_cmp).unwrap();
    let max = scores.iter().copied().max_by(f64::total_cmp).unwrap();
    let median_primary = median(&mut scores);
    let mut deviations = scores
        .iter()
        .map(|score| (score - median_primary).abs())
        .collect::<Vec<_>>();
    let median_absolute_deviation = median(&mut deviations);
    let spread = max - min;
    let noise_floor = (median_absolute_deviation * 1.4826)
        .max(spread / 2.0)
        .max(f64::EPSILON);
    let successful_repetitions = scores.len() as u32;
    Ok(BaselineAuditSummary {
        repetitions: requested_repetitions,
        successful_repetitions,
        failure_rate: f64::from(requested_repetitions - successful_repetitions)
            / f64::from(requested_repetitions),
        median_primary,
        spread,
        median_absolute_deviation,
        noise_floor,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MethodStrategyCard {
    pub key: String,
    pub category: String,
    pub family: String,
    pub weight: f64,
    pub summary: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

pub fn default_strategy_cards() -> Vec<MethodStrategyCard> {
    vec![
        MethodStrategyCard {
            key: "literature_or_method:bounded_revision".into(),
            category: "literature_or_method".into(),
            family: "literature".into(),
            weight: 0.70,
            summary: "Apply one evidence-backed algorithmic or representation improvement.".into(),
            source_refs: vec![],
        },
        MethodStrategyCard {
            key: "diagnostic:failure_slice".into(),
            category: "diagnostic".into(),
            family: "diagnostic".into(),
            weight: 0.15,
            summary: "Address the most recent bounded evaluator diagnostic or error slice.".into(),
            source_refs: vec![],
        },
        MethodStrategyCard {
            key: "ablation_or_simplification:reduce_complexity".into(),
            category: "ablation_or_simplification".into(),
            family: "simplification".into(),
            weight: 0.10,
            summary: "Remove unnecessary computation or dependencies while preserving behavior."
                .into(),
            source_refs: vec![],
        },
        MethodStrategyCard {
            key: "alternative_family:independent_baseline".into(),
            category: "alternative_family".into(),
            family: "alternative".into(),
            weight: 0.05,
            summary: "Try a structurally distinct method family from the current champion.".into(),
            source_refs: vec![],
        },
    ]
}

/// Deterministic weighted selection. The seed is persisted Run identity and
/// the sequence is the candidate number, so crash/reopen selects the same card.
pub fn select_strategy_index(
    seed: &str,
    sequence: u32,
    cards: &[MethodStrategyCard],
) -> anyhow::Result<usize> {
    if cards.is_empty()
        || cards
            .iter()
            .any(|card| !card.weight.is_finite() || card.weight <= 0.0)
    {
        anyhow::bail!("strategy cards require positive finite weights");
    }
    let total = cards.iter().map(|card| card.weight).sum::<f64>();
    if !total.is_finite() || total <= 0.0 {
        anyhow::bail!("strategy weight total is invalid");
    }
    let digest = Sha256::digest(format!("{seed}\0{sequence}").as_bytes());
    let sample = u64::from_be_bytes(digest[..8].try_into().unwrap());
    let mut cursor = (sample as f64 / u64::MAX as f64) * total;
    for (index, card) in cards.iter().enumerate() {
        if cursor < card.weight || index + 1 == cards.len() {
            return Ok(index);
        }
        cursor -= card.weight;
    }
    unreachable!()
}

pub fn normalized_strategy_reward(
    candidate_utility: f64,
    parent_utility: f64,
    noise_floor: f64,
) -> anyhow::Result<f64> {
    if !candidate_utility.is_finite()
        || !parent_utility.is_finite()
        || !noise_floor.is_finite()
        || noise_floor < 0.0
    {
        anyhow::bail!("strategy reward inputs must be finite");
    }
    Ok(((candidate_utility - parent_utility) / noise_floor.max(f64::EPSILON)).clamp(-5.0, 5.0))
}

pub fn update_strategy_weight(weight: f64, reward: f64) -> anyhow::Result<f64> {
    if !weight.is_finite() || weight <= 0.0 || !reward.is_finite() {
        anyhow::bail!("strategy update inputs must be finite and positive");
    }
    Ok((weight * (0.12 * reward.clamp(-5.0, 5.0)).exp()).clamp(0.01, 100.0))
}

fn source_tokens(source: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in source.chars() {
        if character.is_alphanumeric() || character == '_' {
            current.push(character.to_ascii_lowercase());
        } else {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            if !character.is_whitespace() && character != '#' {
                tokens.push(character.to_string());
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

pub fn normalized_source_sha256(source: &str) -> String {
    let normalized = source_tokens(source).join("\u{1f}");
    Sha256::digest(normalized.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn source_jaccard_distance(left: &str, right: &str) -> f64 {
    fn shingles(source: &str) -> HashSet<String> {
        let tokens = source_tokens(source);
        if tokens.len() < 3 {
            return tokens.into_iter().collect();
        }
        tokens
            .windows(3)
            .map(|window| window.join("\u{1e}"))
            .collect()
    }
    let left = shingles(left);
    let right = shingles(right);
    if left.is_empty() && right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(&right).count() as f64;
    let union = left.union(&right).count() as f64;
    1.0 - intersection / union
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodCandidateRank {
    pub id: String,
    pub family: String,
    pub source: String,
    pub utility: f64,
    pub runtime_ms: i64,
    pub changed_lines: i64,
    pub dependency_count: i64,
}

fn rank_precedes(
    left: &MethodCandidateRank,
    right: &MethodCandidateRank,
    noise_floor: f64,
) -> bool {
    let delta = left.utility - right.utility;
    if delta.abs() > noise_floor {
        return delta > 0.0;
    }
    (
        left.changed_lines,
        left.runtime_ms,
        left.dependency_count,
        &left.id,
    ) < (
        right.changed_lines,
        right.runtime_ms,
        right.dependency_count,
        &right.id,
    )
}

/// Retain one champion per deterministic local diversity cluster, then fill
/// Top-K by robust score/simplicity order.
pub fn select_diverse_top_k(
    candidates: &[MethodCandidateRank],
    top_k: usize,
    noise_floor: f64,
    diversity_floor: f64,
) -> anyhow::Result<Vec<String>> {
    if top_k == 0
        || !noise_floor.is_finite()
        || noise_floor < 0.0
        || !diversity_floor.is_finite()
        || !(0.0..=1.0).contains(&diversity_floor)
        || candidates
            .iter()
            .any(|candidate| !candidate.utility.is_finite())
    {
        anyhow::bail!("Top-K selection parameters are invalid");
    }
    let mut ordered = candidates.to_vec();
    ordered.sort_by(|left, right| {
        if rank_precedes(left, right, noise_floor) {
            std::cmp::Ordering::Less
        } else if rank_precedes(right, left, noise_floor) {
            std::cmp::Ordering::Greater
        } else {
            left.id.cmp(&right.id)
        }
    });
    let mut selected = Vec::<MethodCandidateRank>::new();
    let mut deferred = Vec::new();
    for candidate in ordered {
        let distinct = selected.iter().all(|champion| {
            candidate.family != champion.family
                && source_jaccard_distance(&candidate.source, &champion.source) >= diversity_floor
        });
        if distinct && selected.len() < top_k {
            selected.push(candidate);
        } else {
            deferred.push(candidate);
        }
    }
    for candidate in deferred {
        if selected.len() == top_k {
            break;
        }
        selected.push(candidate);
    }
    Ok(selected.into_iter().map(|candidate| candidate.id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> MethodSearchSpec {
        MethodSearchSpec {
            schema: METHOD_SEARCH_SCHEMA_V1.into(),
            objective: "Improve validation accuracy".into(),
            target: MethodSearchTarget {
                language: "python".into(),
                source_artifact_version_id: "source-v1".into(),
                source_path: "analysis/model.py".into(),
                symbol: "fit_model".into(),
            },
            evaluator: MethodSearchEvaluatorSpec {
                artifact_version_id: "evaluator-v1".into(),
                entry_path: "analysis/evaluate.py".into(),
                repetitions: 3,
                timeout_seconds: 60,
                protocol: EVALUATOR_PROTOCOL_V1.into(),
            },
            metrics: MethodSearchMetrics {
                primary: "accuracy".into(),
                direction: ScoreDirection::Maximize,
                guardrails: vec![MethodSearchGuardrail {
                    metric: "runtime_seconds".into(),
                    op: GuardrailOperator::Lte,
                    value: 60.0,
                }],
            },
            inputs: vec![MethodSearchInput {
                role: "search_validation".into(),
                path: "data/validation.csv".into(),
                artifact_version_id: Some("data-v1".into()),
                external_resource_id: None,
                checksum: "a".repeat(64),
            }],
            protected_paths: vec!["analysis/evaluate.py".into()],
            constraints: vec!["Keep the public signature unchanged".into()],
            strategy_sources: vec![],
            budget: MethodSearchBudget {
                max_candidates: 20,
                max_wall_seconds: 3_600,
                max_evaluator_seconds: 60,
                max_cost_microunits: 1_000_000,
            },
            final_verification: None,
        }
    }

    #[test]
    fn evaluator_protocol_is_exact_and_finite() {
        let spec = spec();
        let output = "log\nwisp_evaluate: {\"primary\":0.8,\"metrics\":{\"accuracy\":0.8,\"runtime_seconds\":12.0}}";
        assert_eq!(parse_evaluator_output(output, &spec).unwrap().primary, 0.8);
        assert!(parse_evaluator_output(&format!("{output}\n{output}"), &spec).is_err());
        assert!(parse_evaluator_output(
            r#"wisp_evaluate: {"primary":0.8,"metrics":{"accuracy":0.7,"runtime_seconds":12.0}}"#,
            &spec,
        )
        .is_err());
        assert!(parse_evaluator_output(
            r#"wisp_evaluate: {"primary":0.8,"metrics":{"accuracy":0.8}}"#,
            &spec,
        )
        .is_err());
        assert!(parse_evaluator_output(
            r#"wisp_evaluate: {"primary":NaN,"metrics":{"accuracy":NaN,"runtime_seconds":12.0}}"#,
            &spec,
        )
        .is_err());
    }

    #[test]
    fn baseline_summary_uses_deterministic_robust_noise() {
        let result = |primary| {
            Ok(EvaluatorResult {
                primary,
                metrics: BTreeMap::from([
                    ("accuracy".into(), primary),
                    ("runtime_seconds".into(), 12.0),
                ]),
            })
        };
        let audit = summarize_baseline(3, &[result(0.5), result(0.6), result(0.7)]).unwrap();
        assert_eq!(audit.median_primary, 0.6);
        assert!((audit.spread - 0.2).abs() < 1e-12);
        assert!((audit.noise_floor - 0.14826).abs() < 1e-12);
    }

    #[test]
    fn python_symbol_sentinel_and_replacement_are_scoped() {
        let source = "import math\n\ndef fit_model(\n    rows,\n    seed=0,\n):\n    value = len(rows)\n    return value\n\ndef untouched():\n    return 1\n";
        let sentinel = inject_python_reachability_sentinel(source, "fit_model").unwrap();
        assert!(sentinel.contains("wisp_method_search_reachability_sentinel"));
        assert!(sentinel.contains("def untouched():"));

        let replacement = "def fit_model(\n    rows,\n    seed=0,\n):\n    return len(rows) + 1\n";
        let candidate = replace_python_symbol(source, "fit_model", replacement).unwrap();
        assert!(candidate.contains("return len(rows) + 1"));
        assert!(candidate.contains("def untouched():"));
        assert!(
            replace_python_symbol(source, "fit_model", "def fit_model(rows):\n    return 0\n")
                .is_err()
        );
        assert!(replace_python_symbol(
            source,
            "fit_model",
            "import os\ndef fit_model(\n    rows,\n    seed=0,\n):\n    return 0\n"
        )
        .is_err());
        let oversized = format!(
            "def fit_model(\n    rows,\n    seed=0,\n):\n    return '{} '\n",
            "x".repeat(MAX_CANDIDATE_SOURCE_BYTES)
        );
        assert!(replace_python_symbol(source, "fit_model", &oversized).is_err());
    }

    #[test]
    fn adaptive_strategy_and_diverse_top_k_are_deterministic() {
        let cards = default_strategy_cards();
        assert_eq!(
            select_strategy_index("run", 7, &cards).unwrap(),
            select_strategy_index("run", 7, &cards).unwrap()
        );
        assert!((normalized_strategy_reward(0.7, 0.5, 0.1).unwrap() - 2.0).abs() < 1e-12);
        assert!(update_strategy_weight(0.5, 2.0).unwrap() > 0.5);

        let candidate =
            |id: &str, family: &str, source: &str, utility, changed| MethodCandidateRank {
                id: id.into(),
                family: family.into(),
                source: source.into(),
                utility,
                runtime_ms: 10,
                changed_lines: changed,
                dependency_count: 0,
            };
        let selected = select_diverse_top_k(
            &[
                candidate("complex", "linear", "def f(x): return x + 1", 0.81, 8),
                candidate("simple", "linear", "def f(x): return x", 0.80, 1),
                candidate("tree", "tree", "def f(x): return tree(x)", 0.79, 5),
            ],
            2,
            0.02,
            0.25,
        )
        .unwrap();
        assert_eq!(selected, vec!["simple", "tree"]);
        assert_ne!(
            normalized_source_sha256("def f(x): return x"),
            normalized_source_sha256("def f(x): return x + 1")
        );
    }
}
