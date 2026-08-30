//! Trajectory (轨迹) folding: collapse a session's stored messages and
//! persisted UI events into turns of user/assistant/tool/usage cells with
//! per-cell timing plus session-level token/latency statistics.
//!
//! Pure functions only — the `load_session_trajectory` command fetches rows
//! and delegates here, so the folding rules are unit-tested without a
//! database.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use wisp_llm::{Message, Role};
use wisp_store::SessionUiEventRecord;

use crate::AgentEvent;

/// Preview strings are one line, clipped to this many characters.
const PREVIEW_MAX_CHARS: usize = 120;

#[derive(Serialize, Clone, Debug, Default)]
pub struct TrajectorySnapshot {
    pub frame_id: String,
    pub model: Option<String>,
    pub turns: Vec<TrajectoryTurn>,
    pub stats: TrajectoryStats,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct TrajectoryTurn {
    pub index: i64,
    /// Unix epoch milliseconds of the user message that opened the turn.
    pub started_at: Option<i64>,
    pub cells: Vec<TrajectoryCell>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct TrajectoryCell {
    /// `"user" | "assistant" | "tool" | "usage"`.
    pub kind: String,
    pub summary: String,
    /// Tool cells: full arguments JSON.
    pub detail_input: Option<String>,
    /// Tool cells: full result text; assistant cells: full text.
    pub detail_output: Option<String>,
    pub ok: Option<bool>,
    pub is_error: bool,
    /// Unix epoch milliseconds.
    pub ts: Option<i64>,
    /// Tool wall time in milliseconds.
    pub duration_ms: Option<i64>,
    pub usage: Option<TrajectoryUsage>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct TrajectoryUsage {
    pub round: i64,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cached_input_tokens: i64,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct TrajectoryStats {
    pub turns: i64,
    pub steps: i64,
    pub llm_ms: i64,
    pub tool_ms: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_hit_pct: Option<f64>,
    pub tokens_per_sec: Option<f64>,
}

/// First line of `text`, clipped to `PREVIEW_MAX_CHARS` characters.
fn preview(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or("").trim();
    let mut chars = first_line.chars();
    let clipped: String = chars.by_ref().take(PREVIEW_MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{clipped}…")
    } else {
        clipped
    }
}

fn tool_summary(name: &str, arguments: Option<&str>, result: Option<&str>) -> String {
    let mut summary = match arguments {
        Some(arguments) if !arguments.trim().is_empty() => {
            format!("{name} {}", preview(arguments))
        }
        _ => name.to_string(),
    };
    if let Some(result) = result {
        summary = format!("{summary} → {}", preview(result));
    }
    summary
}

struct TurnBuild {
    index: i64,
    started_at: Option<i64>,
    cells: Vec<TrajectoryCell>,
    /// Parallel to `cells`: tool name for `"tool"` cells, used to match
    /// `ToolResult` events (which carry a name but no call id).
    tool_names: Vec<Option<String>>,
    /// `ToolResult` events already consumed per cell index.
    matched: HashSet<usize>,
    /// tool_call_id → cell index awaiting its result message.
    pending: HashMap<String, usize>,
}

impl TurnBuild {
    fn new(index: i64, started_at: Option<i64>) -> Self {
        Self {
            index,
            started_at,
            cells: Vec::new(),
            tool_names: Vec::new(),
            matched: HashSet::new(),
            pending: HashMap::new(),
        }
    }

    fn push(&mut self, cell: TrajectoryCell, tool_name: Option<String>) -> usize {
        self.cells.push(cell);
        self.tool_names.push(tool_name);
        self.cells.len() - 1
    }

    fn match_tool_result(&mut self, name: &str, ok: bool, duration_ms: u64) {
        for index in 0..self.cells.len() {
            if self.matched.contains(&index) {
                continue;
            }
            if self.cells[index].kind != "tool" || self.tool_names[index].as_deref() != Some(name) {
                continue;
            }
            self.matched.insert(index);
            self.cells[index].ok = Some(ok);
            self.cells[index].is_error = !ok;
            if duration_ms > 0 {
                self.cells[index].duration_ms = Some(duration_ms as i64);
            }
            return;
        }
    }
}

/// Message timestamps are unix seconds; zero means unknown.
fn ms_from_secs(ts: i64) -> Option<i64> {
    if ts > 0 {
        Some(ts.saturating_mul(1000))
    } else {
        None
    }
}

/// Pick the turn an event belongs to: the latest turn that started at or
/// before the event timestamp (the "current" turn when the event occurred).
/// Without timing information the last open turn wins; an event predating
/// every turn lands in the first one.
fn turn_index_for(turns: &[TurnBuild], ts: Option<i64>) -> usize {
    match ts {
        Some(ts) => turns
            .iter()
            .enumerate()
            .rev()
            .find(|(_, turn)| turn.started_at.is_some_and(|started| started <= ts))
            .map(|(index, _)| index)
            .unwrap_or(0),
        None => turns.len() - 1,
    }
}

pub fn fold_trajectory(
    frame_id: &str,
    frame_model: Option<String>,
    messages: &[(i64, Message)],
    events: &[SessionUiEventRecord],
) -> TrajectorySnapshot {
    let mut turns: Vec<TurnBuild> = Vec::new();

    for (_seq, message) in messages {
        let ts = ms_from_secs(message.ts);
        match message.role {
            Role::System => continue,
            Role::User => {
                let text = message.content.as_text();
                let mut turn = TurnBuild::new(turns.len() as i64 + 1, ts);
                turn.push(
                    TrajectoryCell {
                        kind: "user".into(),
                        summary: preview(&text),
                        detail_output: (!text.trim().is_empty()).then_some(text),
                        ts,
                        ..Default::default()
                    },
                    None,
                );
                turns.push(turn);
            }
            Role::Assistant => {
                if turns.is_empty() {
                    turns.push(TurnBuild::new(1, ts));
                }
                let turn = turns.last_mut().expect("turn created above");
                let text = message.content.as_text();
                if !text.trim().is_empty() {
                    turn.push(
                        TrajectoryCell {
                            kind: "assistant".into(),
                            summary: preview(&text),
                            detail_output: Some(text),
                            ts,
                            ..Default::default()
                        },
                        None,
                    );
                }
                for call in &message.tool_calls {
                    let name = call.function.name.clone();
                    let arguments = call.function.arguments.clone();
                    let index = turn.push(
                        TrajectoryCell {
                            kind: "tool".into(),
                            summary: tool_summary(&name, Some(&arguments), None),
                            detail_input: Some(arguments),
                            ts,
                            ..Default::default()
                        },
                        Some(name),
                    );
                    if !call.id.is_empty() {
                        turn.pending.insert(call.id.clone(), index);
                    }
                }
            }
            Role::Tool => {
                if turns.is_empty() {
                    turns.push(TurnBuild::new(1, ts));
                }
                let turn = turns.last_mut().expect("turn created above");
                let result = message.content.as_text();
                let name = message.tool_name.clone().unwrap_or_default();
                match message
                    .tool_call_id
                    .as_deref()
                    .and_then(|id| turn.pending.remove(id))
                {
                    Some(index) => {
                        let cell = &mut turn.cells[index];
                        let arguments = cell.detail_input.clone();
                        cell.summary = tool_summary(&name, arguments.as_deref(), Some(&result));
                        cell.detail_output = Some(result);
                    }
                    None => {
                        // Result without a matching call (e.g. compacted or
                        // truncated history): keep it as a standalone cell.
                        turn.push(
                            TrajectoryCell {
                                kind: "tool".into(),
                                summary: tool_summary(&name, None, Some(&result)),
                                detail_output: Some(result),
                                ts,
                                ..Default::default()
                            },
                            Some(name),
                        );
                    }
                }
            }
        }
    }

    // Fold the persisted UI event stream in: usage accounting per model round
    // and per-call tool wall times/ok flags (messages store neither).
    for record in events {
        let event = match serde_json::from_str::<AgentEvent>(&record.event_json) {
            Ok(event) => event,
            Err(_) => continue,
        };
        match event {
            AgentEvent::Usage {
                round,
                model,
                created_at,
                input,
                output,
                reasoning,
                cached,
                ..
            } => {
                let ts = ms_from_secs(created_at).or(record.created_at);
                if turns.is_empty() {
                    turns.push(TurnBuild::new(1, ts));
                }
                let index = turn_index_for(&turns, ts);
                turns[index].push(
                    TrajectoryCell {
                        kind: "usage".into(),
                        summary: format!("round {round} · {} in / {} out", input, output),
                        ts,
                        usage: Some(TrajectoryUsage {
                            round: round as i64,
                            model: (!model.is_empty()).then_some(model),
                            input_tokens: input as i64,
                            output_tokens: output as i64,
                            reasoning_tokens: reasoning as i64,
                            cached_input_tokens: cached as i64,
                        }),
                        ..Default::default()
                    },
                    None,
                );
            }
            AgentEvent::ToolResult {
                name,
                ok,
                duration_ms,
                ..
            } => {
                if turns.is_empty() {
                    continue;
                }
                let index = turn_index_for(&turns, record.created_at);
                turns[index].match_tool_result(&name, ok, duration_ms);
            }
            _ => {}
        }
    }

    let mut latest_usage_model: Option<String> = None;
    let mut stats = TrajectoryStats::default();
    let mut out_turns = Vec::with_capacity(turns.len());
    for turn in turns {
        // Stable order by timestamp; cells without one keep their relative
        // order at the end.
        let mut order: Vec<usize> = (0..turn.cells.len()).collect();
        order.sort_by_key(|&index| turn.cells[index].ts.unwrap_or(i64::MAX));
        let ordered: Vec<TrajectoryCell> = order
            .into_iter()
            .map(|index| turn.cells[index].clone())
            .collect();
        let mut previous_ts = turn.started_at;
        for cell in &ordered {
            if cell.kind == "usage" {
                stats.steps += 1;
                if let (Some(ts), Some(previous)) = (cell.ts, previous_ts) {
                    stats.llm_ms += (ts - previous).max(0);
                }
                if let Some(usage) = &cell.usage {
                    stats.input_tokens += usage.input_tokens;
                    stats.output_tokens += usage.output_tokens;
                    stats.cached_input_tokens += usage.cached_input_tokens;
                    if let Some(model) = &usage.model {
                        latest_usage_model = Some(model.clone());
                    }
                }
            }
            if let Some(duration_ms) = cell.duration_ms {
                stats.tool_ms += duration_ms;
            }
            previous_ts = cell.ts.or(previous_ts);
        }
        out_turns.push(TrajectoryTurn {
            index: turn.index,
            started_at: turn.started_at,
            cells: ordered,
        });
    }
    stats.turns = out_turns.len() as i64;
    let billed = stats.input_tokens + stats.cached_input_tokens;
    if billed > 0 {
        stats.cache_hit_pct = Some(stats.cached_input_tokens as f64 / billed as f64 * 100.0);
    }
    if stats.llm_ms > 0 {
        stats.tokens_per_sec = Some(stats.output_tokens as f64 / (stats.llm_ms as f64 / 1000.0));
    }

    TrajectorySnapshot {
        frame_id: frame_id.to_string(),
        model: frame_model
            .filter(|model| !model.is_empty())
            .or(latest_usage_model),
        turns: out_turns,
        stats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_llm::{FunctionCall, ToolCall};

    fn tool_call(id: &str, name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }

    fn assistant_with_calls(text: &str, calls: Vec<ToolCall>, ts: i64) -> Message {
        let mut message = Message::assistant(text);
        message.tool_calls = calls;
        message.ts = ts;
        message
    }

    fn timed_message(mut message: Message, ts: i64) -> Message {
        message.ts = ts;
        message
    }

    fn usage_event(round: u64, model: &str, created_at: i64, input: u64, output: u64) -> String {
        serde_json::json!({
            "kind": "Usage",
            "frame_id": "f",
            "round": round,
            "model": model,
            "created_at": created_at,
            "input": input,
            "output": output,
            "reasoning": 0,
            "cached": 0,
            "ctx_tokens": 0,
            "max_context": 0,
        })
        .to_string()
    }

    fn tool_result_event(name: &str, ok: bool, duration_ms: u64) -> String {
        serde_json::json!({
            "kind": "ToolResult",
            "frame_id": "f",
            "name": name,
            "ok": ok,
            "content": "preview",
            "duration_ms": duration_ms,
        })
        .to_string()
    }

    fn record(seq: i64, created_at: Option<i64>, event_json: String) -> SessionUiEventRecord {
        SessionUiEventRecord {
            seq,
            created_at,
            event_json,
        }
    }

    #[test]
    fn empty_session_folds_to_no_turns() {
        let snapshot = fold_trajectory("f", Some("m".into()), &[], &[]);
        assert_eq!(snapshot.frame_id, "f");
        assert_eq!(snapshot.model.as_deref(), Some("m"));
        assert!(snapshot.turns.is_empty());
        assert_eq!(snapshot.stats.turns, 0);
        assert_eq!(snapshot.stats.steps, 0);
        assert_eq!(snapshot.stats.cache_hit_pct, None);
        assert_eq!(snapshot.stats.tokens_per_sec, None);
    }

    #[test]
    fn two_turns_pair_tools_and_accumulate_stats() {
        // Turn 1: user @1000s, assistant calls read @1001s, result @1002s.
        // Turn 2: user @2000s, assistant answer @2001s.
        let messages = vec![
            (1, timed_message(Message::user("first question"), 1000)),
            (
                2,
                assistant_with_calls(
                    "",
                    vec![tool_call("c1", "read_file", r#"{"path":"a.rs"}"#)],
                    1001,
                ),
            ),
            (
                3,
                timed_message(Message::tool("c1", "read_file", "file contents"), 1002),
            ),
            (4, timed_message(Message::assistant("first answer"), 1003)),
            (5, timed_message(Message::user("second question"), 2000)),
            (6, timed_message(Message::assistant("second answer"), 2001)),
        ];
        let events = vec![
            record(
                1,
                Some(1_004_000),
                tool_result_event("read_file", true, 250),
            ),
            record(2, Some(1_004_000), usage_event(1, "gpt-x", 1004, 100, 50)),
            record(3, Some(2_002_000), usage_event(2, "gpt-x", 2002, 200, 100)),
        ];
        let snapshot = fold_trajectory("f", None, &messages, &events);

        assert_eq!(snapshot.model.as_deref(), Some("gpt-x"));
        assert_eq!(snapshot.turns.len(), 2);
        assert_eq!(snapshot.turns[0].index, 1);
        assert_eq!(snapshot.turns[0].started_at, Some(1_000_000));
        assert_eq!(snapshot.turns[1].started_at, Some(2_000_000));

        let turn1 = &snapshot.turns[0];
        let kinds: Vec<&str> = turn1.cells.iter().map(|c| c.kind.as_str()).collect();
        assert_eq!(kinds, vec!["user", "tool", "assistant", "usage"]);
        let tool = &turn1.cells[1];
        assert_eq!(
            tool.summary,
            "read_file {\"path\":\"a.rs\"} → file contents"
        );
        assert_eq!(tool.detail_input.as_deref(), Some(r#"{"path":"a.rs"}"#));
        assert_eq!(tool.detail_output.as_deref(), Some("file contents"));
        assert_eq!(tool.ok, Some(true));
        assert!(!tool.is_error);
        assert_eq!(tool.duration_ms, Some(250));

        let usage1 = turn1
            .cells
            .iter()
            .find(|c| c.kind == "usage")
            .and_then(|c| c.usage.as_ref())
            .unwrap();
        assert_eq!(usage1.input_tokens, 100);
        let usage2 = snapshot.turns[1]
            .cells
            .iter()
            .find(|c| c.kind == "usage")
            .and_then(|c| c.usage.as_ref())
            .unwrap();
        assert_eq!(usage2.output_tokens, 100);

        // llm_ms: turn1 usage @1004s - previous cell (assistant @1003s) = 1000;
        // turn2 usage @2002s - assistant @2001s = 1000.
        assert_eq!(snapshot.stats.turns, 2);
        assert_eq!(snapshot.stats.steps, 2);
        assert_eq!(snapshot.stats.llm_ms, 2000);
        assert_eq!(snapshot.stats.tool_ms, 250);
        assert_eq!(snapshot.stats.input_tokens, 300);
        assert_eq!(snapshot.stats.output_tokens, 150);
        assert_eq!(snapshot.stats.tokens_per_sec, Some(75.0));
    }

    #[test]
    fn llm_ms_uses_turn_start_for_first_cell_and_clamps_negative() {
        // Usage is measured from the previous cell in the turn — here the
        // user message that opened it.
        let messages = vec![
            (1, timed_message(Message::user("q"), 1000)),
            (2, timed_message(Message::assistant("a"), 1005)),
        ];
        let events = vec![record(1, None, usage_event(1, "m", 1004, 10, 20))];
        let snapshot = fold_trajectory("f", None, &messages, &events);
        assert_eq!(snapshot.stats.llm_ms, 4000);
        assert_eq!(snapshot.stats.tokens_per_sec, Some(5.0));

        // A usage event stamped before the turn start clamps to zero.
        let events = vec![record(1, None, usage_event(1, "m", 500, 10, 20))];
        let snapshot = fold_trajectory("f", None, &messages, &events);
        assert_eq!(snapshot.stats.llm_ms, 0);
        assert_eq!(snapshot.stats.tokens_per_sec, None);
        // The early-stamped usage cell sorts before the user cell it precedes.
        assert_eq!(snapshot.turns[0].cells[0].kind, "usage");
        assert_eq!(snapshot.turns[0].cells[1].kind, "user");
    }

    #[test]
    fn unmatched_tool_result_becomes_standalone_cell() {
        let messages = vec![
            (1, timed_message(Message::user("q"), 1000)),
            (
                2,
                timed_message(Message::tool("c-gone", "shell", "orphan output"), 1001),
            ),
        ];
        let snapshot = fold_trajectory("f", None, &messages, &[]);
        let turn = &snapshot.turns[0];
        assert_eq!(turn.cells.len(), 2);
        let tool = &turn.cells[1];
        assert_eq!(tool.kind, "tool");
        assert_eq!(tool.summary, "shell → orphan output");
        assert_eq!(tool.detail_input, None);
        assert_eq!(tool.detail_output.as_deref(), Some("orphan output"));
        assert_eq!(tool.ok, None);
        assert!(!tool.is_error);
    }

    #[test]
    fn tool_result_events_match_per_name_in_order() {
        // Two shell calls in one turn; ToolResult events match FIFO per name,
        // so the failing result lands on the second call.
        let messages = vec![
            (1, timed_message(Message::user("q"), 1000)),
            (
                2,
                assistant_with_calls(
                    "",
                    vec![
                        tool_call("c1", "shell", r#"{"cmd":"ls"}"#),
                        tool_call("c2", "shell", r#"{"cmd":"false"}"#),
                    ],
                    1001,
                ),
            ),
            (3, timed_message(Message::tool("c1", "shell", "out1"), 1002)),
            (4, timed_message(Message::tool("c2", "shell", "err2"), 1003)),
        ];
        let events = vec![
            record(1, Some(1_002_000), tool_result_event("shell", true, 100)),
            record(2, Some(1_003_000), tool_result_event("shell", false, 200)),
        ];
        let snapshot = fold_trajectory("f", None, &messages, &events);
        let turn = &snapshot.turns[0];
        let tools: Vec<&TrajectoryCell> = turn.cells.iter().filter(|c| c.kind == "tool").collect();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].detail_output.as_deref(), Some("out1"));
        assert_eq!(tools[0].ok, Some(true));
        assert_eq!(tools[0].duration_ms, Some(100));
        assert_eq!(tools[1].detail_output.as_deref(), Some("err2"));
        assert_eq!(tools[1].ok, Some(false));
        assert!(tools[1].is_error);
        assert_eq!(tools[1].duration_ms, Some(200));
        assert_eq!(snapshot.stats.tool_ms, 300);
    }

    #[test]
    fn leading_assistant_messages_open_turn_one_lazily() {
        let messages = vec![
            (1, timed_message(Message::assistant("welcome"), 900)),
            (2, timed_message(Message::user("q"), 1000)),
            (3, timed_message(Message::assistant("a"), 1001)),
        ];
        let snapshot = fold_trajectory("f", None, &messages, &[]);
        assert_eq!(snapshot.turns.len(), 2);
        assert_eq!(snapshot.turns[0].index, 1);
        assert_eq!(snapshot.turns[0].cells[0].kind, "assistant");
        assert_eq!(snapshot.turns[1].cells[0].kind, "user");
    }

    #[test]
    fn cache_hit_percentage_computed_from_cached_tokens() {
        let mut event: serde_json::Value =
            serde_json::from_str(&usage_event(1, "m", 1001, 100, 50)).unwrap();
        event["cached"] = serde_json::json!(300);
        let messages = vec![
            (1, timed_message(Message::user("q"), 1000)),
            (2, timed_message(Message::assistant("a"), 1002)),
        ];
        let events = vec![record(1, Some(1_001_000), event.to_string())];
        let snapshot = fold_trajectory("f", None, &messages, &events);
        // 300 cached / (100 input + 300 cached) = 75%.
        assert_eq!(snapshot.stats.cached_input_tokens, 300);
        assert_eq!(snapshot.stats.cache_hit_pct, Some(75.0));
    }
}
