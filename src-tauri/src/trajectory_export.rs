//! Export a session trajectory as a self-contained HTML document.
//!
//! The HTML is generated from a freshly folded snapshot (persisted messages
//! + UI events), not from the frontend's filtered inspector view. The Gantt
//! timeline is the reason this is HTML rather than Markdown.

use crate::trajectory::{
    fold_trajectory, TrajectoryCell, TrajectorySnapshot, TrajectoryStats, TrajectoryUsage,
};
use crate::AppState;
use std::fmt::Write as _;
use tauri::{AppHandle, State};

const EXPORT_CSS: &str = r#"
:root {
  --bg: #f6f4ef; --card: #fff; --text: #2b2a27; --muted: #6d6a63;
  --line: #e4e0d6; --input: #c5c1b8; --model: #4d84c4; --tools: #c45a3a;
  --user: #2f6f9f; --assistant: #6b4ea3; --error: #b42318;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #1c1b18; --card: #26241f; --text: #ece8df; --muted: #a8a39a;
    --line: #3a372f; --input: #8c877c; --model: #7aa7d9; --tools: #d57a5e;
    --user: #7eb3d6; --assistant: #b39adf; --error: #f2b8b5;
  }
}
* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body {
  margin: 0; background: var(--bg); color: var(--text);
  font: 14px/1.5 ui-sans-serif, system-ui, -apple-system, sans-serif;
}
main { max-width: 980px; margin: 0 auto; padding: 28px 20px 64px; }
header.meta { margin-bottom: 22px; }
header.meta h1 { margin: 0 0 8px; font-size: 22px; letter-spacing: -0.02em; }
.meta-line { display: flex; flex-wrap: wrap; gap: 8px 18px; color: var(--muted); font-size: 13px; }
.meta-line strong { color: var(--text); font-weight: 600; }
.stats {
  margin: 12px 0 0; padding: 10px 12px; background: var(--card);
  border: 1px solid var(--line); border-radius: 8px; color: var(--muted); font-size: 13px;
}
.timeline { margin: 22px 0 28px; }
.timeline h2, .turn h2 { margin: 0 0 10px; font-size: 15px; }
.gantt { display: flex; flex-direction: column; gap: 8px; }
.lane { display: flex; align-items: center; gap: 10px; }
.lane-label { width: 56px; flex: 0 0 auto; color: var(--muted); font-size: 12px; }
.track {
  position: relative; flex: 1 1 auto; height: 14px;
  background: color-mix(in srgb, var(--line) 70%, transparent);
  border-radius: 4px;
}
.seg {
  position: absolute; top: 0; height: 100%; border-radius: 3px; min-width: 3px;
}
.seg.input { background: var(--input); }
.seg.model { background: var(--model); }
.seg.tools { background: var(--tools); }
.seg.error { outline: 1px solid var(--error); }
.turn {
  margin: 0 0 22px; padding: 14px 16px 8px; background: var(--card);
  border: 1px solid var(--line); border-radius: 10px;
}
.event { margin: 0 0 14px; padding-bottom: 12px; border-bottom: 1px solid var(--line); }
.event:last-child { border-bottom: 0; margin-bottom: 0; }
.event-head { display: flex; flex-wrap: wrap; align-items: baseline; gap: 8px 12px; }
.badge {
  display: inline-block; padding: 1px 7px; border-radius: 999px;
  font-size: 11px; font-weight: 700; letter-spacing: 0.04em;
}
.badge.user { color: var(--user); background: color-mix(in srgb, var(--user) 14%, transparent); }
.badge.assistant { color: var(--assistant); background: color-mix(in srgb, var(--assistant) 14%, transparent); }
.badge.tool { color: var(--tools); background: color-mix(in srgb, var(--tools) 14%, transparent); }
.badge.usage { color: var(--model); background: color-mix(in srgb, var(--model) 14%, transparent); }
.badge.error { color: var(--error); background: color-mix(in srgb, var(--error) 16%, transparent); }
.event-head .summary { color: var(--muted); font-size: 13px; }
.kv { display: flex; flex-wrap: wrap; gap: 6px 16px; margin: 6px 0 8px; color: var(--muted); font-size: 12px; }
pre {
  margin: 6px 0 0; padding: 10px 12px; overflow: auto;
  background: color-mix(in srgb, var(--bg) 70%, var(--card));
  border: 1px solid var(--line); border-radius: 6px;
  font: 12.5px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  white-space: pre-wrap; overflow-wrap: anywhere;
}
.block-label { margin: 10px 0 0; color: var(--muted); font-size: 12px; font-weight: 600; }
.empty { color: var(--muted); padding: 24px 0; }
.raw { margin-top: 28px; }
.raw summary { cursor: pointer; color: var(--muted); }
"#;

/// Safe default file name for the native save dialog.
pub(crate) fn trajectory_file_name(frame_id: &str) -> String {
    let safe: String = frame_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    let safe = safe.trim_matches('-');
    if safe.is_empty() {
        "wisp-trajectory.html".into()
    } else {
        format!("wisp-trajectory-{safe}.html")
    }
}

#[derive(Clone, Copy)]
struct Labels {
    lang: &'static str,
    title: &'static str,
    session: &'static str,
    model: &'static str,
    exported: &'static str,
    timeline: &'static str,
    input: &'static str,
    model_lane: &'static str,
    tools: &'static str,
    turn: &'static str,
    user: &'static str,
    assistant: &'static str,
    tool: &'static str,
    usage: &'static str,
    arguments: &'static str,
    result: &'static str,
    status: &'static str,
    duration: &'static str,
    timestamp: &'static str,
    completed: &'static str,
    error: &'static str,
    pending: &'static str,
    empty: &'static str,
    raw: &'static str,
    unknown_model: &'static str,
}

fn labels(locale: &str) -> Labels {
    if locale.eq_ignore_ascii_case("zh") || locale.starts_with("zh-") || locale.starts_with("zh_") {
        Labels {
            lang: "zh",
            title: "轨迹",
            session: "会话",
            model: "模型",
            exported: "导出时间",
            timeline: "时间线",
            input: "输入",
            model_lane: "模型",
            tools: "工具",
            turn: "第 {n} 轮",
            user: "用户",
            assistant: "助手",
            tool: "工具",
            usage: "用量",
            arguments: "参数",
            result: "结果",
            status: "状态",
            duration: "耗时",
            timestamp: "时间",
            completed: "已完成",
            error: "错误",
            pending: "等待中",
            empty: "暂无轨迹事件。",
            raw: "原始快照（JSON）",
            unknown_model: "未知",
        }
    } else {
        Labels {
            lang: "en",
            title: "Trajectory",
            session: "Session",
            model: "Model",
            exported: "Exported",
            timeline: "Timeline",
            input: "Input",
            model_lane: "Model",
            tools: "Tools",
            turn: "Turn {n}",
            user: "User",
            assistant: "Assistant",
            tool: "Tool",
            usage: "Usage",
            arguments: "Arguments",
            result: "Result",
            status: "Status",
            duration: "Duration",
            timestamp: "Time",
            completed: "Completed",
            error: "Error",
            pending: "Pending",
            empty: "No trajectory events.",
            raw: "Raw snapshot (JSON)",
            unknown_model: "unknown",
        }
    }
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn fmt_tokens(n: i64) -> String {
    if n.abs() < 1000 {
        n.to_string()
    } else {
        format!("{:.1}k", n as f64 / 1000.0)
    }
}

fn format_duration_ms(ms: i64) -> String {
    let ms = ms.max(0) as u64;
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{}s", ms / 1000)
    } else {
        let mins = ms / 60_000;
        let secs = (ms % 60_000) / 1000;
        if secs == 0 {
            format!("{mins}m")
        } else {
            format!("{mins}m {secs}s")
        }
    }
}

fn format_ts(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| ms.to_string())
}

fn cell_lane(kind: &str) -> &'static str {
    match kind {
        "user" => "input",
        "tool" => "tools",
        _ => "model",
    }
}

fn cell_status<'a>(cell: &'a TrajectoryCell, l: &Labels) -> &'a str {
    if cell.is_error || cell.ok == Some(false) {
        l.error
    } else if cell.kind == "tool" && cell.ok.is_none() {
        l.pending
    } else {
        l.completed
    }
}

fn kind_label<'a>(kind: &'a str, l: &'a Labels) -> &'a str {
    match kind {
        "user" => l.user,
        "assistant" => l.assistant,
        "tool" => l.tool,
        "usage" => l.usage,
        _ => kind,
    }
}

fn stats_line(stats: &TrajectoryStats, l: &Labels) -> String {
    let cache = stats
        .cache_hit_pct
        .map(|v| format!("{v:.0}%"))
        .unwrap_or_else(|| "–".into());
    let tok_s = stats
        .tokens_per_sec
        .map(|v| format!("{v:.1}"))
        .unwrap_or_else(|| "–".into());
    format!(
        "{} · {} | LLM {} · {} {} | {} tok/s | cache {cache} | in {} · out {}",
        l.turn.replace("{n}", &stats.turns.to_string()),
        stats.steps,
        format_duration_ms(stats.llm_ms),
        l.tools,
        format_duration_ms(stats.tool_ms),
        tok_s,
        fmt_tokens(stats.input_tokens),
        fmt_tokens(stats.output_tokens),
    )
}

fn usage_line(usage: &TrajectoryUsage) -> String {
    let mut line = format!(
        "round {} · in {} · out {}",
        usage.round,
        fmt_tokens(usage.input_tokens),
        fmt_tokens(usage.output_tokens)
    );
    if usage.input_tokens > 0 && usage.cached_input_tokens > 0 {
        let pct = (usage.cached_input_tokens as f64 * 100.0 / usage.input_tokens as f64).round();
        let _ = write!(line, " · cached {pct:.0}%");
    }
    if usage.reasoning_tokens > 0 {
        let _ = write!(line, " · reasoning {}", fmt_tokens(usage.reasoning_tokens));
    }
    line
}

struct GanttSeg {
    id: String,
    lane: &'static str,
    left_pct: f64,
    width_pct: f64,
    error: bool,
}

fn gantt_segments(snapshot: &TrajectorySnapshot) -> Vec<GanttSeg> {
    let mut events: Vec<(String, &TrajectoryCell, f64)> = Vec::new();
    for turn in &snapshot.turns {
        for (ci, cell) in turn.cells.iter().enumerate() {
            if cell.kind == "usage" {
                continue;
            }
            events.push((
                format!("t{}-c{ci}", turn.index),
                cell,
                cell.duration_ms.unwrap_or(0).max(0) as f64,
            ));
        }
    }
    if events.is_empty() {
        return Vec::new();
    }
    let positive: Vec<f64> = events
        .iter()
        .map(|(_, _, d)| *d)
        .filter(|d| *d > 0.0)
        .collect();
    let weights: Vec<f64> = if positive.is_empty() {
        events.iter().map(|_| 1.0).collect()
    } else {
        let avg = positive.iter().sum::<f64>() / positive.len() as f64;
        let floor = (avg * 0.5).max(1.0);
        events
            .iter()
            .map(|(_, _, d)| if *d > 0.0 { *d } else { floor })
            .collect()
    };
    let total = weights.iter().copied().sum::<f64>().max(1.0);
    let mut acc = 0.0;
    events
        .into_iter()
        .enumerate()
        .map(|(i, (id, cell, _))| {
            let w = weights[i];
            let left = acc / total * 100.0;
            acc += w;
            GanttSeg {
                id,
                lane: cell_lane(&cell.kind),
                left_pct: left,
                width_pct: w / total * 100.0,
                error: cell.is_error || cell.ok == Some(false),
            }
        })
        .collect()
}

fn write_gantt(out: &mut String, snapshot: &TrajectorySnapshot, l: &Labels) {
    let segs = gantt_segments(snapshot);
    if segs.is_empty() {
        return;
    }
    let _ = write!(
        out,
        "<section class=\"timeline\">\n<h2>{}</h2>\n<div class=\"gantt\">\n",
        escape_html(l.timeline)
    );
    for (lane, label) in [
        ("input", l.input),
        ("model", l.model_lane),
        ("tools", l.tools),
    ] {
        let _ = write!(
            out,
            "<div class=\"lane\"><span class=\"lane-label\">{}</span><div class=\"track\">",
            escape_html(label)
        );
        for seg in segs.iter().filter(|seg| seg.lane == lane) {
            let class = if seg.error {
                format!("seg {} error", seg.lane)
            } else {
                format!("seg {}", seg.lane)
            };
            let _ = write!(
                out,
                "<a class=\"{class}\" href=\"#{}\" style=\"left:{:.2}%;width:{:.2}%\"></a>",
                escape_html(&seg.id),
                seg.left_pct,
                seg.width_pct
            );
        }
        out.push_str("</div></div>\n");
    }
    out.push_str("</div>\n</section>\n");
}

fn write_pre(out: &mut String, label: &str, text: &str) {
    let _ = write!(
        out,
        "<div class=\"block-label\">{}</div>\n<pre>{}</pre>\n",
        escape_html(label),
        escape_html(text)
    );
}

fn write_cell(out: &mut String, turn: i64, index: usize, cell: &TrajectoryCell, l: &Labels) {
    let id = format!("t{turn}-c{index}");
    let kind = cell.kind.as_str();
    let badge_class = if cell.is_error || cell.ok == Some(false) {
        format!("badge {kind} error")
    } else {
        format!("badge {kind}")
    };
    let _ = write!(
        out,
        "<article class=\"event\" id=\"{}\">\n<div class=\"event-head\">\
         <span class=\"{badge_class}\">{}</span>\
         <span class=\"summary\">{}</span>\n</div>\n<div class=\"kv\">",
        escape_html(&id),
        escape_html(kind_label(kind, l)),
        escape_html(&cell.summary)
    );
    let _ = write!(
        out,
        "<span>{} {}</span>",
        escape_html(l.status),
        escape_html(cell_status(cell, l))
    );
    if let Some(ts) = cell.ts {
        let _ = write!(
            out,
            "<span>{} {}</span>",
            escape_html(l.timestamp),
            escape_html(&format_ts(ts))
        );
    }
    if let Some(ms) = cell.duration_ms {
        let _ = write!(
            out,
            "<span>{} {}</span>",
            escape_html(l.duration),
            escape_html(&format_duration_ms(ms))
        );
    }
    out.push_str("</div>\n");
    match kind {
        "tool" => {
            if let Some(input) = cell.detail_input.as_deref() {
                write_pre(out, l.arguments, input);
            }
            if let Some(output) = cell.detail_output.as_deref() {
                write_pre(out, l.result, output);
            }
        }
        "usage" => {
            if let Some(usage) = &cell.usage {
                let mut body = usage_line(usage);
                if let Some(model) = &usage.model {
                    let _ = write!(body, "\nmodel {model}");
                }
                write_pre(out, l.usage, &body);
            }
        }
        _ => {
            if let Some(text) = cell
                .detail_output
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                write_pre(out, kind_label(kind, l), text);
            }
        }
    }
    out.push_str("</article>\n");
}

/// Build a self-contained HTML document for the folded trajectory.
pub(crate) fn render_trajectory_html(
    snapshot: &TrajectorySnapshot,
    locale: &str,
    exported_at: &str,
) -> String {
    let l = labels(locale);
    let model = snapshot
        .model
        .as_deref()
        .filter(|model| !model.is_empty())
        .unwrap_or(l.unknown_model);
    let mut out = String::with_capacity(8192);
    let _ = write!(
        out,
        "<!doctype html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{} · {}</title>\n<style>\n{EXPORT_CSS}\n</style>\n</head>\n<body>\n<main>\n\
         <header class=\"meta\">\n<h1>{}</h1>\n<div class=\"meta-line\">\
         <span>{} <strong>{}</strong></span>\
         <span>{} <strong>{}</strong></span>\
         <span>{} <strong>{}</strong></span></div>\n<p class=\"stats\">{}</p>\n</header>\n",
        escape_html(l.lang),
        escape_html(l.title),
        escape_html(&snapshot.frame_id),
        escape_html(l.title),
        escape_html(l.session),
        escape_html(&snapshot.frame_id),
        escape_html(l.model),
        escape_html(model),
        escape_html(l.exported),
        escape_html(exported_at),
        escape_html(&stats_line(&snapshot.stats, &l)),
    );
    write_gantt(&mut out, snapshot, &l);
    if snapshot.turns.is_empty() {
        let _ = write!(out, "<p class=\"empty\">{}</p>\n", escape_html(l.empty));
    } else {
        for turn in &snapshot.turns {
            let heading = l.turn.replace("{n}", &turn.index.to_string());
            let _ = write!(
                out,
                "<section class=\"turn\" id=\"turn-{}\">\n<h2>{}</h2>\n",
                turn.index,
                escape_html(&heading)
            );
            for (ci, cell) in turn.cells.iter().enumerate() {
                write_cell(&mut out, turn.index, ci, cell, &l);
            }
            out.push_str("</section>\n");
        }
    }
    let raw = serde_json::to_string_pretty(snapshot).unwrap_or_else(|_| "{}".into());
    let _ = write!(
        out,
        "<details class=\"raw\"><summary>{}</summary>\n<pre>{}</pre>\n</details>\n\
         </main>\n</body>\n</html>\n",
        escape_html(l.raw),
        escape_html(&raw)
    );
    out
}

/// Reload the persisted trajectory and save it as HTML via the native dialog.
/// Returns the saved path, or `None` when the user cancels.
#[tauri::command]
pub(super) async fn export_session_trajectory(
    app: AppHandle,
    state: State<'_, AppState>,
    frame_id: String,
    locale: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    if frame_id.trim().is_empty() {
        return Err("No session to export.".into());
    }
    let messages = state
        .store
        .load_messages_with_seq(&frame_id)
        .await
        .map_err(|error| error.to_string())?;
    let events = state
        .store
        .load_session_ui_events_timed(&frame_id)
        .await
        .map_err(|error| error.to_string())?;
    let model = state
        .store
        .frame_model(&frame_id)
        .await
        .map_err(|error| error.to_string())?;
    let snapshot = fold_trajectory(&frame_id, model, &messages, &events);
    let locale = locale.unwrap_or_else(|| "en".into());
    let exported_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let html = render_trajectory_html(&snapshot, &locale, &exported_at);
    let default_name = trajectory_file_name(&frame_id);
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&default_name)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(dest) = rx.await.map_err(|e| format!("{e}"))? else {
        return Ok(None);
    };
    let dest_path = std::path::PathBuf::from(dest.to_string());
    tokio::fs::write(&dest_path, html)
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    Ok(Some(dest_path.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trajectory::{TrajectoryTurn, TrajectoryUsage};

    fn cell(kind: &str, summary: &str) -> TrajectoryCell {
        TrajectoryCell {
            kind: kind.into(),
            summary: summary.into(),
            ..Default::default()
        }
    }

    fn sample_snapshot() -> TrajectorySnapshot {
        TrajectorySnapshot {
            frame_id: "sess-42".into(),
            model: Some("deepseek-v4-pro".into()),
            turns: vec![TrajectoryTurn {
                index: 1,
                started_at: Some(1_755_000_000_000),
                cells: vec![
                    TrajectoryCell {
                        kind: "user".into(),
                        summary: "Analyze ESR1".into(),
                        detail_output: Some(
                            "Analyze the ESR1 dataset\nwith fences ```md```".into(),
                        ),
                        ts: Some(1_755_000_000_000),
                        ..Default::default()
                    },
                    TrajectoryCell {
                        kind: "tool".into(),
                        summary: "python · describe".into(),
                        detail_input: Some(r#"{"code":"df.describe()"}"#.into()),
                        detail_output: Some(
                            "count  612.0\n```html\n</pre><script>alert(1)</script>\n```".into(),
                        ),
                        ok: Some(true),
                        ts: Some(1_755_000_002_000),
                        duration_ms: Some(3400),
                        ..Default::default()
                    },
                    TrajectoryCell {
                        kind: "tool".into(),
                        summary: "python · boom".into(),
                        detail_input: Some(r#"{"code":"1/0"}"#.into()),
                        detail_output: Some("ZeroDivisionError".into()),
                        ok: Some(false),
                        is_error: true,
                        ts: Some(1_755_000_006_000),
                        duration_ms: Some(80),
                        ..Default::default()
                    },
                    TrajectoryCell {
                        kind: "assistant".into(),
                        summary: "Here is the answer".into(),
                        detail_output: Some(
                            "Here is the full answer that is longer than the preview.".into(),
                        ),
                        ts: Some(1_755_000_007_000),
                        duration_ms: Some(1200),
                        ..Default::default()
                    },
                    TrajectoryCell {
                        kind: "usage".into(),
                        summary: "round 1".into(),
                        ts: Some(1_755_000_008_000),
                        usage: Some(TrajectoryUsage {
                            round: 1,
                            model: Some("deepseek-v4-pro".into()),
                            input_tokens: 12300,
                            output_tokens: 1400,
                            reasoning_tokens: 300,
                            cached_input_tokens: 9225,
                        }),
                        ..Default::default()
                    },
                ],
            }],
            stats: TrajectoryStats {
                turns: 1,
                steps: 1,
                llm_ms: 3300,
                tool_ms: 3480,
                input_tokens: 12300,
                output_tokens: 1400,
                cached_input_tokens: 9225,
                cache_hit_pct: Some(75.0),
                tokens_per_sec: Some(12.5),
            },
        }
    }

    #[test]
    fn file_name_keeps_only_safe_components() {
        assert_eq!(
            trajectory_file_name("abc-123"),
            "wisp-trajectory-abc-123.html"
        );
        assert_eq!(
            trajectory_file_name("../../etc/passwd"),
            "wisp-trajectory-etc-passwd.html"
        );
        assert_eq!(trajectory_file_name(""), "wisp-trajectory.html");
        assert_eq!(trajectory_file_name(".."), "wisp-trajectory.html");
    }

    #[test]
    fn html_includes_session_model_stats_and_timeline() {
        let html = render_trajectory_html(&sample_snapshot(), "en", "2026-08-24T00:00:00Z");
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("lang=\"en\""));
        assert!(html.contains("sess-42"));
        assert!(html.contains("deepseek-v4-pro"));
        assert!(html.contains("2026-08-24T00:00:00Z"));
        assert!(html.contains("Turn 1"));
        assert!(html.contains("12.5 tok/s"));
        assert!(html.contains("cache 75%"));
        assert!(html.contains("class=\"gantt\""));
        assert!(html.contains("href=\"#t1-c1\""));
        assert!(html.contains("id=\"t1-c1\""));
        assert!(html.ends_with("</html>\n"));
    }

    #[test]
    fn html_keeps_full_payloads_and_escapes_hostile_tool_output() {
        let html = render_trajectory_html(&sample_snapshot(), "en", "t");
        assert!(html.contains("Analyze the ESR1 dataset"));
        assert!(html.contains("Here is the full answer that is longer than the preview."));
        assert!(html.contains(&escape_html(r#"{"code":"df.describe()"}"#)));
        assert!(html.contains("count  612.0"));
        assert!(html.contains("```html"));
        assert!(!html.contains("</pre><script>alert(1)</script>"));
        assert!(html.contains("&lt;/pre&gt;&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("ZeroDivisionError"));
        assert!(html.contains("badge tool error"));
        let ts = chrono::DateTime::from_timestamp_millis(1_755_000_000_000)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        assert!(html.contains(&ts));
        assert!(html.contains("round 1 · in 12.3k · out 1.4k · cached 75%"));
        assert!(html.contains("reasoning 300"));
        assert!(html.contains("Raw snapshot (JSON)"));
        assert!(html.contains("&quot;frame_id&quot;"));
    }

    #[test]
    fn zh_locale_uses_chinese_labels() {
        let html = render_trajectory_html(&sample_snapshot(), "zh", "t");
        assert!(html.contains("lang=\"zh\""));
        assert!(html.contains("轨迹"));
        assert!(html.contains("第 1 轮"));
        assert!(html.contains("参数"));
        assert!(html.contains("结果"));
        assert!(html.contains("时间线"));
        assert!(html.contains("原始快照（JSON）"));
    }

    #[test]
    fn empty_snapshot_still_produces_a_document() {
        let html = render_trajectory_html(
            &TrajectorySnapshot {
                frame_id: "empty".into(),
                ..Default::default()
            },
            "en",
            "t",
        );
        assert!(html.contains("empty"));
        assert!(html.contains("No trajectory events."));
        assert!(!html.contains("class=\"gantt\""));
    }

    #[test]
    fn unused_preview_is_not_what_gets_exported() {
        let mut snap = sample_snapshot();
        snap.turns[0].cells[0].summary = "truncated…".into();
        let html = render_trajectory_html(&snap, "en", "t");
        assert!(html.contains("Analyze the ESR1 dataset"));
        assert!(html.contains("truncated…"));
    }

    #[test]
    fn gantt_skips_usage_and_marks_errors() {
        let segs = gantt_segments(&sample_snapshot());
        assert_eq!(segs.len(), 4);
        assert!(segs.iter().all(|seg| !seg.id.contains("c4")));
        assert!(segs.iter().any(|seg| seg.error && seg.lane == "tools"));
        let span: f64 = segs.iter().map(|s| s.width_pct).sum();
        assert!((span - 100.0).abs() < 0.01);
    }

    #[test]
    fn cell_helpers_cover_pending_tools() {
        let l = labels("en");
        let pending = cell("tool", "wait");
        assert_eq!(cell_status(&pending, &l), "Pending");
        assert_eq!(kind_label("unknown", &l), "unknown");
    }
}
