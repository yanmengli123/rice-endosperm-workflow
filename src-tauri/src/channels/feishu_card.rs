//! Pure CardKit builders and progress projection for Feishu streamed replies.
//!
//! We intentionally expose tool names and coarse status only. Raw reasoning,
//! command output, and tool results can contain secrets and never belong in an
//! external progress card.

use serde_json::json;
use std::collections::VecDeque;

pub const PROGRESS_ELEMENT_ID: &str = "md";
const MAX_ASSISTANT_CHARS: usize = 5_500;
const MAX_TOOL_ROWS: usize = 4;

pub fn build_streaming_card(initial_markdown: &str) -> String {
    json!({
        "schema": "2.0",
        "config": { "streaming_mode": true },
        "body": {
            "elements": [{
                "tag": "markdown",
                "element_id": PROGRESS_ELEMENT_ID,
                "content": initial_markdown,
            }]
        }
    })
    .to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ToolRow {
    name: String,
    state: ToolState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ToolState {
    Running,
    Done { ok: bool, duration_ms: u64 },
}

#[derive(Default)]
pub struct ProgressState {
    assistant: String,
    tools: VecDeque<ToolRow>,
    activity_seen: bool,
}

impl ProgressState {
    pub fn assistant_delta(&mut self, delta: &str) {
        self.activity_seen = true;
        self.assistant.push_str(delta);
        self.assistant = tail_chars(&self.assistant, MAX_ASSISTANT_CHARS);
    }

    pub fn reasoning_activity(&mut self) {
        // Never forward model reasoning. This flag only changes the generic
        // activity label from "preparing" to "working".
        self.activity_seen = true;
    }

    pub fn tool_started(&mut self, name: &str) {
        self.activity_seen = true;
        self.tools.push_back(ToolRow {
            name: safe_tool_name(name),
            state: ToolState::Running,
        });
        while self.tools.len() > MAX_TOOL_ROWS {
            self.tools.pop_front();
        }
    }

    pub fn tool_finished(&mut self, name: &str, ok: bool, duration_ms: u64) {
        self.activity_seen = true;
        if let Some(row) = self
            .tools
            .iter_mut()
            .rev()
            .find(|row| row.name == safe_tool_name(name) && row.state == ToolState::Running)
        {
            row.state = ToolState::Done { ok, duration_ms };
            return;
        }
        self.tools.push_back(ToolRow {
            name: safe_tool_name(name),
            state: ToolState::Done { ok, duration_ms },
        });
        while self.tools.len() > MAX_TOOL_ROWS {
            self.tools.pop_front();
        }
    }

    pub fn render(&self) -> String {
        let mut sections = vec![if self.activity_seen {
            "**Wisp Science 正在处理**".to_string()
        } else {
            "**已收到消息，正在准备…**".to_string()
        }];
        if !self.assistant.trim().is_empty() {
            sections.push(self.assistant.trim().to_string());
        }
        if !self.tools.is_empty() {
            let rows = self
                .tools
                .iter()
                .map(|row| match row.state {
                    ToolState::Running => format!("⏳ `{}` 正在执行", row.name),
                    ToolState::Done { ok, duration_ms } => format!(
                        "{} `{}` 已{} · {}",
                        if ok { "✅" } else { "⚠️" },
                        row.name,
                        if ok { "完成" } else { "失败" },
                        format_duration(duration_ms)
                    ),
                })
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(rows);
        }
        sections.push("_完成后，最终答案会固定在这张卡片中。_".to_string());
        sections.join("\n\n")
    }
}

fn safe_tool_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|ch| !ch.is_control() && *ch != '`')
        .take(80)
        .collect();
    if cleaned.trim().is_empty() {
        "tool".into()
    } else {
        cleaned
    }
}

fn tail_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - max_chars).collect();
    format!("…{tail}")
}

fn format_duration(duration_ms: u64) -> String {
    if duration_ms < 1_000 {
        format!("{duration_ms} ms")
    } else {
        format!("{:.1} s", duration_ms as f64 / 1_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_card_is_valid_and_addresses_markdown_element() {
        let card = build_streaming_card("处理中…");
        let value: serde_json::Value = serde_json::from_str(&card).unwrap();
        assert_eq!(value["schema"], "2.0");
        assert_eq!(value["config"]["streaming_mode"], true);
        assert_eq!(value["body"]["elements"][0]["element_id"], "md");
        assert_eq!(value["body"]["elements"][0]["content"], "处理中…");
    }

    #[test]
    fn progress_reports_tools_but_not_reasoning_or_results() {
        let mut state = ProgressState::default();
        state.reasoning_activity();
        state.tool_started("shell");
        state.tool_finished("shell", true, 1_250);
        state.assistant_delta("partial answer");
        let rendered = state.render();
        assert!(rendered.contains("partial answer"));
        assert!(rendered.contains("`shell` 已完成 · 1.2 s"));
        assert!(!rendered.contains("reasoning"));
    }

    #[test]
    fn progress_keeps_a_bounded_recent_tool_window() {
        let mut state = ProgressState::default();
        for index in 0..7 {
            state.tool_started(&format!("tool-{index}"));
        }
        let rendered = state.render();
        assert!(!rendered.contains("tool-0"));
        assert!(rendered.contains("tool-6"));
    }
}
