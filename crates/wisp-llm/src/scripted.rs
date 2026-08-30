//! Deterministic provider for offline agent tests and host conformance suites.
//!
//! Unlike the small one-off fake providers used by unit tests, this provider
//! is public, serializable at its script boundary, records every request, and
//! drives the same streaming path as network providers. It deliberately lives
//! in `wisp-llm` so headless hosts can test the real agent loop without an API
//! key or network access.

use crate::{
    Completion, FunctionCall, LlmError, Message, Provider, Result, StreamSink, ToolCall,
    ToolSchema, Usage,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScriptedToolCall {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "empty_object")]
    pub arguments: serde_json::Value,
}

fn empty_object() -> serde_json::Value {
    serde_json::json!({})
}

/// A deterministic provider-side API failure. Suites use this to script error
/// paths such as context-overflow recovery without a real endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScriptedApiError {
    pub status: u16,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScriptedCompletion {
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ScriptedToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    /// Optional deterministic latency used by timeout and cancellation tests.
    #[serde(default)]
    pub delay_ms: u64,
    /// Split streamed text/reasoning into chunks no larger than this many
    /// Unicode scalar values. Zero emits one chunk.
    #[serde(default)]
    pub chunk_chars: usize,
    /// Fail this scripted step with an API error instead of a completion. The
    /// request is still recorded and the script advances past this step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_error: Option<ScriptedApiError>,
}

impl ScriptedCompletion {
    fn materialize(&self, sequence: usize) -> Completion {
        let calls = self
            .tool_calls
            .iter()
            .enumerate()
            .map(|(index, call)| ToolCall {
                id: if call.id.trim().is_empty() {
                    format!("script-{sequence}-{index}")
                } else {
                    call.id.clone()
                },
                kind: "function".into(),
                function: FunctionCall {
                    name: call.name.clone(),
                    arguments: serde_json::to_string(&call.arguments)
                        .unwrap_or_else(|_| "{}".into()),
                },
            })
            .collect::<Vec<_>>();
        Completion {
            content: self.content.clone(),
            reasoning: self.reasoning.clone(),
            finish_reason: self.finish_reason.clone().or_else(|| {
                Some(if calls.is_empty() {
                    "stop".into()
                } else {
                    "tool_calls".into()
                })
            }),
            tool_calls: calls,
            usage: Usage {
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
                reasoning_tokens: self.reasoning_tokens,
                cached_input_tokens: self.cached_input_tokens,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptedRequest {
    pub sequence: usize,
    pub messages: Vec<Message>,
    pub tool_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptedProviderSnapshot {
    pub model: String,
    pub requests: Vec<ScriptedRequest>,
    pub remaining_completions: usize,
}

#[derive(Debug)]
struct State {
    completions: VecDeque<ScriptedCompletion>,
    requests: Vec<ScriptedRequest>,
}

/// A cloneable provider whose clones consume one shared FIFO script.
#[derive(Clone, Debug)]
pub struct ScriptedProvider {
    model: String,
    state: Arc<Mutex<State>>,
}

impl ScriptedProvider {
    pub fn new(model: impl Into<String>, completions: Vec<ScriptedCompletion>) -> Self {
        Self {
            model: model.into(),
            state: Arc::new(Mutex::new(State {
                completions: completions.into(),
                requests: Vec::new(),
            })),
        }
    }

    pub fn snapshot(&self) -> ScriptedProviderSnapshot {
        let state = self.state.lock().expect("scripted provider mutex poisoned");
        ScriptedProviderSnapshot {
            model: self.model.clone(),
            requests: state.requests.clone(),
            remaining_completions: state.completions.len(),
        }
    }

    pub fn remaining(&self) -> usize {
        self.state
            .lock()
            .expect("scripted provider mutex poisoned")
            .completions
            .len()
    }

    fn next(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> Result<(usize, ScriptedCompletion)> {
        let mut state = self.state.lock().expect("scripted provider mutex poisoned");
        let sequence = state.requests.len() + 1;
        state.requests.push(ScriptedRequest {
            sequence,
            messages: messages.to_vec(),
            tool_names: tools
                .iter()
                .map(|schema| schema.function.name.clone())
                .collect(),
        });
        let completion = state.completions.pop_front().ok_or_else(|| {
            LlmError::Config(format!(
                "scripted provider exhausted after {} request(s)",
                sequence - 1
            ))
        })?;
        Ok((sequence, completion))
    }
}

fn chunks(text: &str, limit: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    if limit == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut chunk = String::new();
    for character in text.chars() {
        chunk.push(character);
        if chunk.chars().count() >= limit {
            out.push(std::mem::take(&mut chunk));
        }
    }
    if !chunk.is_empty() {
        out.push(chunk);
    }
    out
}

async fn deterministic_delay(delay_ms: u64, sink: &mut dyn StreamSink) -> Result<()> {
    let mut remaining = delay_ms;
    while remaining > 0 {
        if sink.is_cancelled() {
            return Err(LlmError::Incomplete);
        }
        let slice = remaining.min(10);
        tokio::time::sleep(Duration::from_millis(slice)).await;
        remaining -= slice;
    }
    if sink.is_cancelled() {
        return Err(LlmError::Incomplete);
    }
    Ok(())
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn complete(&self, messages: &[Message], tools: &[ToolSchema]) -> Result<Completion> {
        let (sequence, scripted) = self.next(messages, tools)?;
        if scripted.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(scripted.delay_ms)).await;
        }
        if let Some(error) = &scripted.api_error {
            return Err(LlmError::Api {
                status: error.status,
                body: error.body.clone(),
            });
        }
        Ok(scripted.materialize(sequence))
    }

    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> Result<Completion> {
        let (sequence, scripted) = self.next(messages, tools)?;
        deterministic_delay(scripted.delay_ms, sink).await?;
        if let Some(error) = &scripted.api_error {
            return Err(LlmError::Api {
                status: error.status,
                body: error.body.clone(),
            });
        }
        for chunk in chunks(
            scripted.reasoning.as_deref().unwrap_or_default(),
            scripted.chunk_chars,
        ) {
            if sink.is_cancelled() {
                return Err(LlmError::Incomplete);
            }
            sink.on_reasoning(&chunk);
        }
        for chunk in chunks(&scripted.content, scripted.chunk_chars) {
            if sink.is_cancelled() {
                return Err(LlmError::Incomplete);
            }
            sink.on_text(&chunk);
        }
        let completion = scripted.materialize(sequence);
        for (index, call) in completion.tool_calls.iter().enumerate() {
            sink.on_tool_call(index, &call.function.name, &call.function.arguments);
        }
        sink.on_usage(completion.usage.clone());
        Ok(completion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NullSink, Role};

    #[tokio::test]
    async fn consumes_fifo_and_records_exact_requests() {
        let provider = ScriptedProvider::new(
            "fixture",
            vec![ScriptedCompletion {
                tool_calls: vec![ScriptedToolCall {
                    name: "read".into(),
                    arguments: serde_json::json!({"path": "notes.txt"}),
                    ..ScriptedToolCall::default()
                }],
                ..ScriptedCompletion::default()
            }],
        );
        let mut sink = NullSink;
        let completion = provider
            .stream(
                &[Message::user("inspect")],
                &[ToolSchema::new("read", "read", serde_json::json!({}))],
                &mut sink,
            )
            .await
            .unwrap();
        assert_eq!(completion.tool_calls[0].function.name, "read");
        assert_eq!(completion.finish_reason.as_deref(), Some("tool_calls"));
        let snapshot = provider.snapshot();
        assert_eq!(snapshot.requests.len(), 1);
        assert_eq!(snapshot.requests[0].messages[0].role, Role::User);
        assert_eq!(snapshot.requests[0].tool_names, vec!["read"]);
        assert_eq!(snapshot.remaining_completions, 0);
    }

    #[tokio::test]
    async fn scripted_api_error_fails_one_step_and_advances_the_script() {
        let provider = ScriptedProvider::new(
            "fixture",
            vec![
                ScriptedCompletion {
                    api_error: Some(ScriptedApiError {
                        status: 400,
                        body: "maximum context length exceeded".into(),
                    }),
                    ..ScriptedCompletion::default()
                },
                ScriptedCompletion {
                    content: "recovered".into(),
                    ..ScriptedCompletion::default()
                },
            ],
        );
        let mut sink = NullSink;
        let error = provider
            .stream(&[Message::user("first")], &[], &mut sink)
            .await
            .unwrap_err();
        assert!(error.is_context_overflow(), "{error}");
        let completion = provider
            .complete(&[Message::user("second")], &[])
            .await
            .unwrap();
        assert_eq!(completion.content, "recovered");
        let snapshot = provider.snapshot();
        assert_eq!(snapshot.requests.len(), 2);
        assert_eq!(snapshot.remaining_completions, 0);
    }
}
