use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use wisp_core::{Agent, Output, OutputFuture};
use wisp_llm::{Message, ToolCall};
use wisp_tools::{Approval, ConfirmDecision};

pub const RPC_SCHEMA: &str = "wisp.agent-rpc.v1";

const SAFE_READ_ONLY_TOOLS: &[&str] = &[
    "read",
    "search",
    "grep",
    "view_image",
    "update_plan",
    "attempt_completion",
    "list_skill_catalog",
    "search_skills",
    "search_models",
    "use_skill",
    "search_memory",
    "search_mcp_tools",
];

fn configured_approval(tool: &str) -> Approval {
    match std::env::var("WISP_APPROVAL_MODE")
        .unwrap_or_else(|_| "safe".into())
        .to_ascii_lowercase()
        .as_str()
    {
        "allow" => Approval::Allow,
        "deny" => Approval::Deny,
        "ask" => Approval::Ask,
        _ if SAFE_READ_ONLY_TOOLS.contains(&tool) => Approval::Allow,
        _ => Approval::Ask,
    }
}

#[derive(Debug, Deserialize)]
struct CommandEnvelope {
    schema: String,
    id: String,
    #[serde(flatten)]
    command: RpcCommand,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RpcCommand {
    Prompt {
        prompt: String,
    },
    Cancel,
    ApprovalResponse {
        approval_id: String,
        approved: bool,
        #[serde(default)]
        feedback: Option<String>,
    },
    Ping,
    Shutdown,
}

struct RpcOutput<W> {
    writer: Mutex<W>,
    sequence: AtomicU64,
    session_id: String,
    command_id: Mutex<Option<String>>,
    pending_calls: Mutex<VecDeque<ToolCall>>,
    active_call_ids: Mutex<VecDeque<String>>,
    approvals: Mutex<HashMap<String, tokio::sync::oneshot::Sender<ConfirmDecision>>>,
}

impl<W: Write + Send> RpcOutput<W> {
    fn new(writer: W) -> Self {
        Self {
            writer: Mutex::new(writer),
            sequence: AtomicU64::new(0),
            session_id: uuid::Uuid::new_v4().to_string(),
            command_id: Mutex::new(None),
            pending_calls: Mutex::new(VecDeque::new()),
            active_call_ids: Mutex::new(VecDeque::new()),
            approvals: Mutex::new(HashMap::new()),
        }
    }

    fn set_command(&self, command_id: Option<String>) {
        if let Ok(mut current) = self.command_id.lock() {
            *current = command_id;
        }
    }

    fn emit(&self, mut event: Value) {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        if let Some(object) = event.as_object_mut() {
            object.insert("schema".into(), RPC_SCHEMA.into());
            object.insert("sequence".into(), sequence.into());
            object.insert("session_id".into(), self.session_id.clone().into());
            if let Some(id) = self.command_id.lock().ok().and_then(|id| id.clone()) {
                object.insert("command_id".into(), id.into());
            }
        }
        let Ok(mut writer) = self.writer.lock() else {
            return;
        };
        if serde_json::to_writer(&mut *writer, &event).is_ok() {
            let _ = writer.write_all(b"\n");
            let _ = writer.flush();
        }
    }

    fn emit_for(&self, command_id: &str, event: Value) {
        let previous = self.command_id.lock().ok().and_then(|id| id.clone());
        self.set_command(Some(command_id.to_string()));
        self.emit(event);
        self.set_command(previous);
    }

    fn resolve_approval(
        &self,
        approval_id: &str,
        approved: bool,
        feedback: Option<String>,
    ) -> bool {
        let sender = self
            .approvals
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(approval_id));
        let decision = if approved {
            ConfirmDecision::Approved
        } else {
            ConfirmDecision::Denied { feedback }
        };
        sender.is_some_and(|sender| sender.send(decision).is_ok())
    }

    fn reject_pending_approvals(&self) {
        let pending = self
            .approvals
            .lock()
            .map(|mut approvals| {
                approvals
                    .drain()
                    .map(|(_, sender)| sender)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for sender in pending {
            let _ = sender.send(ConfirmDecision::Denied {
                feedback: Some("turn cancelled".into()),
            });
        }
    }
}

impl<W: Write + Send> Output for RpcOutput<W> {
    fn assistant_text(&self, delta: &str) {
        self.emit(json!({"type": "text", "delta": delta}));
    }

    fn reasoning(&self, delta: &str) {
        self.emit(json!({"type": "reasoning", "delta": delta}));
    }

    fn tool_call(&self, name: &str, preview: &str) {
        let call = self.pending_calls.lock().ok().and_then(|mut calls| {
            calls
                .iter()
                .position(|call| call.function.name == name)
                .and_then(|index| calls.remove(index))
        });
        let (call_id, arguments) = call
            .map(|call| {
                let arguments = call.args_value();
                (Some(call.id), Some(arguments))
            })
            .unwrap_or_default();
        if let Some(call_id) = &call_id {
            if let Ok(mut active) = self.active_call_ids.lock() {
                active.push_back(call_id.clone());
            }
        }
        self.emit(json!({
            "type": "tool_call",
            "call_id": call_id,
            "name": name,
            "arguments": arguments,
            "preview": preview,
        }));
    }

    fn tool_result(&self, name: &str, ok: bool, content: &str, duration_ms: u64) {
        let call_id = self
            .active_call_ids
            .lock()
            .ok()
            .and_then(|mut ids| ids.pop_front());
        self.emit(json!({
            "type": "tool_result",
            "call_id": call_id,
            "name": name,
            "ok": ok,
            "content": content,
            "duration_ms": duration_ms,
        }));
    }

    fn usage(
        &self,
        round: usize,
        input: u64,
        output: u64,
        reasoning: u64,
        cached: u64,
        ctx_tokens: usize,
        max_context: usize,
        context_usage: wisp_core::ContextUsage,
    ) {
        self.emit(json!({
            "type": "usage",
            "round": round,
            "input_tokens": input,
            "output_tokens": output,
            "reasoning_tokens": reasoning,
            "cached_tokens": cached,
            "context_tokens": ctx_tokens,
            "max_context_tokens": max_context,
            "context_usage": context_usage,
        }));
    }

    fn compaction_started(&self, strategy: &str) {
        self.emit(json!({"type": "compaction_started", "strategy": strategy}));
    }

    fn compaction(&self, before: usize, after: usize, strategy: &str) {
        self.emit(json!({
            "type": "compaction",
            "before_tokens": before,
            "after_tokens": after,
            "strategy": strategy,
        }));
    }

    fn context_warning(&self, ctx_tokens: usize, max_context: usize) {
        self.emit(json!({
            "type": "context_warning",
            "context_tokens": ctx_tokens,
            "max_context_tokens": max_context,
        }));
    }

    fn diff(&self, path: &str, old: &str, new: &str) {
        self.emit(json!({"type": "diff", "path": path, "old": old, "new": new}));
    }

    fn file_changed(&self, path: &str) {
        self.emit(json!({"type": "file_changed", "path": path}));
    }

    fn stdout_chunk(&self, chunk: &str) {
        self.emit(json!({"type": "stdout", "chunk": chunk}));
    }

    fn tool_presentation(
        &self,
        kind: &str,
        payload: &Value,
        _server: Option<std::sync::Arc<dyn wisp_tools::McpAppServer>>,
    ) {
        self.emit(json!({"type": "tool_presentation", "kind": kind, "payload": payload}));
    }

    fn approval_mode(&self, tool: &str) -> Approval {
        configured_approval(tool)
    }

    fn force_ask_mutations(&self) -> bool {
        !matches!(
            std::env::var("WISP_APPROVAL_MODE")
                .unwrap_or_else(|_| "safe".into())
                .to_ascii_lowercase()
                .as_str(),
            "allow"
        )
    }

    fn restrict_read_paths_to_project(&self) -> bool {
        std::env::var("WISP_RESTRICT_READS")
            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true)
    }

    fn provenance(&self, record: &wisp_core::provenance::ProvenanceRecord) {
        let file_changes = record
            .file_changes
            .iter()
            .map(|change| {
                json!({
                    "path": change.path,
                    "before_exists": change.before_exists,
                    "before_checksum": change.before_checksum,
                    "after_checksum": change.after_checksum,
                    "reversible": change.reversible,
                    "reason": change.reason,
                })
            })
            .collect::<Vec<_>>();
        self.emit(json!({
            "type": "provenance",
            "tool": record.tool,
            "language": record.language,
            "source": record.source,
            "success": record.success,
            "files_written": record.files_written,
            "files_read": record.files_read,
            "file_changes": file_changes,
        }));
    }

    fn confirm_decision_async<'a>(&'a self, message: &'a str) -> OutputFuture<'a, ConfirmDecision> {
        Box::pin(async move {
            let approval_id = uuid::Uuid::new_v4().to_string();
            let (sender, receiver) = tokio::sync::oneshot::channel();
            if let Ok(mut approvals) = self.approvals.lock() {
                approvals.insert(approval_id.clone(), sender);
            } else {
                return ConfirmDecision::Denied {
                    feedback: Some("approval state unavailable".into()),
                };
            }
            self.emit(json!({
                "type": "approval_required",
                "approval_id": approval_id,
                "message": message,
            }));
            receiver.await.unwrap_or(ConfirmDecision::Denied {
                feedback: Some("approval channel closed".into()),
            })
        })
    }

    fn confirm_async<'a>(&'a self, message: &'a str) -> OutputFuture<'a, bool> {
        Box::pin(async move {
            matches!(
                self.confirm_decision_async(message).await,
                ConfirmDecision::Approved
            )
        })
    }

    fn on_message(&self, message: &Message) {
        if !message.tool_calls.is_empty() {
            if let Ok(mut calls) = self.pending_calls.lock() {
                calls.extend(message.tool_calls.iter().cloned());
            }
        }
        self.emit(json!({
            "type": "message",
            "role": message.role,
            "content": message.content,
            "reasoning": message.reasoning,
            "tool_call_id": message.tool_call_id,
            "tool_name": message.tool_name,
            "tool_calls": message.tool_calls,
        }));
    }
}

pub async fn serve(mut agent: Agent) -> Result<()> {
    let output = Arc::new(RpcOutput::new(std::io::stdout()));
    output.emit(json!({
        "type": "ready",
        "protocol": RPC_SCHEMA,
        "model": agent.provider.model(),
        "root": agent.root,
        "capabilities": ["prompt", "cancel", "approval_response", "ping", "shutdown"],
    }));

    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => {
                    if input_tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut command_ids = HashSet::new();
    while let Some(line) = input_rx.recv().await {
        let command = match parse_command(&line) {
            Ok(command) => command,
            Err(error) => {
                output.emit(json!({"type": "protocol_error", "message": error.to_string()}));
                continue;
            }
        };
        if !command_ids.insert(command.id.clone()) {
            output.emit_for(
                &command.id,
                json!({"type": "command_error", "message": "duplicate command id"}),
            );
            continue;
        }
        match command.command {
            RpcCommand::Ping => output.emit_for(&command.id, json!({"type": "pong"})),
            RpcCommand::Shutdown => {
                output.emit_for(&command.id, json!({"type": "shutdown_complete"}));
                break;
            }
            RpcCommand::Cancel | RpcCommand::ApprovalResponse { .. } => output.emit_for(
                &command.id,
                json!({"type": "command_error", "message": "no turn is active"}),
            ),
            RpcCommand::Prompt { prompt } => {
                if prompt.trim().is_empty() {
                    output.emit_for(
                        &command.id,
                        json!({"type": "command_error", "message": "prompt cannot be empty"}),
                    );
                    continue;
                }
                let turn_id = command.id;
                let cancel = Arc::new(AtomicBool::new(false));
                output.set_command(Some(turn_id.clone()));
                output.emit(json!({"type": "turn_started"}));
                let stamped = format!(
                    "{}, Current date: {}",
                    prompt,
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
                );
                let mut run = Box::pin(agent.run(&stamped, output.as_ref(), Some(&cancel)));
                let mut shutdown = false;
                let mut shutdown_command_id = None;
                let mut input_open = true;
                let result = loop {
                    tokio::select! {
                        result = &mut run => break result,
                        next = input_rx.recv(), if input_open => {
                            let Some(line) = next else {
                                cancel.store(true, Ordering::Relaxed);
                                output.reject_pending_approvals();
                                shutdown = true;
                                input_open = false;
                                continue;
                            };
                            let incoming = match parse_command(&line) {
                                Ok(command) => command,
                                Err(error) => {
                                    output.emit(json!({"type": "protocol_error", "message": error.to_string()}));
                                    continue;
                                }
                            };
                            if !command_ids.insert(incoming.id.clone()) {
                                output.emit_for(
                                    &incoming.id,
                                    json!({"type": "command_error", "message": "duplicate command id"}),
                                );
                                continue;
                            }
                            match incoming.command {
                                RpcCommand::Cancel => {
                                    cancel.store(true, Ordering::Relaxed);
                                    output.reject_pending_approvals();
                                    output.emit_for(&incoming.id, json!({"type": "cancel_accepted", "turn_id": turn_id}));
                                }
                                RpcCommand::ApprovalResponse { approval_id, approved, feedback } => {
                                    let accepted = output.resolve_approval(&approval_id, approved, feedback);
                                    output.emit_for(&incoming.id, json!({
                                        "type": "approval_response_accepted",
                                        "approval_id": approval_id,
                                        "accepted": accepted,
                                    }));
                                }
                                RpcCommand::Ping => output.emit_for(&incoming.id, json!({"type": "pong"})),
                                RpcCommand::Shutdown => {
                                    cancel.store(true, Ordering::Relaxed);
                                    output.reject_pending_approvals();
                                    output.emit_for(&incoming.id, json!({"type": "shutdown_accepted"}));
                                    shutdown = true;
                                    shutdown_command_id = Some(incoming.id);
                                }
                                RpcCommand::Prompt { .. } => output.emit_for(
                                    &incoming.id,
                                    json!({"type": "command_error", "message": "a turn is already active"}),
                                ),
                            }
                        }
                    }
                };
                drop(run);
                agent.ctx.clear_runtime_injections();
                agent.save();
                match result {
                    Ok(outcome) => output.emit(json!({
                        "type": "turn_completed",
                        "ok": true,
                        "stop_reason": outcome.stop_reason(),
                    })),
                    Err(error) => output.emit(json!({
                        "type": "turn_completed",
                        "ok": false,
                        "error": error.to_string(),
                    })),
                }
                output.set_command(None);
                if shutdown {
                    if let Some(command_id) = shutdown_command_id {
                        output.emit_for(&command_id, json!({"type": "shutdown_complete"}));
                    } else {
                        output.emit(json!({
                            "type": "shutdown_complete",
                            "reason": "stdin_closed",
                        }));
                    }
                    break;
                }
            }
        }
    }
    Ok(())
}

pub fn startup_error(error: &anyhow::Error) {
    RpcOutput::new(std::io::stdout()).emit(json!({
        "type": "startup_error",
        "message": error.to_string(),
    }));
}

fn parse_command(line: &str) -> Result<CommandEnvelope> {
    let command: CommandEnvelope =
        serde_json::from_str(line).context("invalid RPC command JSON")?;
    if command.schema != RPC_SCHEMA {
        anyhow::bail!(
            "unsupported RPC schema '{}'; expected {RPC_SCHEMA}",
            command.schema
        );
    }
    if command.id.trim().is_empty() {
        anyhow::bail!("RPC command id cannot be empty");
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_mode_allows_reads_but_requires_approval_for_mutations() {
        // Pure default behavior: tests do not mutate the process environment.
        assert!(SAFE_READ_ONLY_TOOLS.contains(&"read"));
        assert!(!SAFE_READ_ONLY_TOOLS.contains(&"write"));
        assert!(!SAFE_READ_ONLY_TOOLS.contains(&"shell"));
    }

    #[test]
    fn parses_versioned_commands_and_rejects_unknown_schemas() {
        let command = parse_command(
            r#"{"schema":"wisp.agent-rpc.v1","id":"turn-1","type":"prompt","prompt":"inspect"}"#,
        )
        .unwrap();
        assert_eq!(command.id, "turn-1");
        assert!(matches!(command.command, RpcCommand::Prompt { .. }));

        assert!(
            parse_command(r#"{"schema":"wisp.agent-rpc.v0","id":"turn-1","type":"cancel"}"#)
                .unwrap_err()
                .to_string()
                .contains("unsupported RPC schema")
        );
    }

    #[tokio::test]
    async fn approval_responses_are_correlated() {
        let output = Arc::new(RpcOutput::new(Vec::new()));
        let waiting = {
            let output = output.clone();
            tokio::spawn(async move { output.confirm_decision_async("write file?").await })
        };
        tokio::task::yield_now().await;
        let approval_id = output
            .approvals
            .lock()
            .unwrap()
            .keys()
            .next()
            .cloned()
            .unwrap();
        assert!(output.resolve_approval(&approval_id, false, Some("not now".into())));
        assert_eq!(
            waiting.await.unwrap(),
            ConfirmDecision::Denied {
                feedback: Some("not now".into())
            }
        );
        let bytes = output.writer.lock().unwrap().clone();
        let event: Value = serde_json::from_slice(
            bytes
                .split(|byte| *byte == b'\n')
                .find(|line| !line.is_empty())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(event["schema"], RPC_SCHEMA);
        assert_eq!(event["sequence"], 0);
        assert_eq!(event["type"], "approval_required");
        assert_eq!(event["approval_id"], approval_id);
    }
}
