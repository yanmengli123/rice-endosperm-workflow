//! `ask_user` — the agent's question channel to the user.
//!
//! `propose_plan`-style (see plan.rs): the tool result IS the question card's
//! body, the registry streams it to the UI, and the agent loop persists it as
//! the tool message that pairs with the call. No store handle, no event of its
//! own. The user's answer arrives as the NEXT user message — the tool result
//! tells the agent to end its turn and wait.

use crate::env::{ToolEnv, ToolResult};
use crate::tool::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use wisp_llm::ToolSchema;

pub const ASK_USER: &str = "ask_user";

const ASK_USER_NOTE: &str = "Question submitted; it is now waiting for the user. \
     End your turn here — the answer arrives as the next user message.";

/// Validate the arguments into the canonical `{question, options[],
/// allow_freeform}` card body. Shared with the ACP bridge so both sources
/// persist the exact same shape the UI parses.
pub fn question_body(args: &Value) -> Result<Value, String> {
    let question = args
        .get("question")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or("ask_user error: 'question' must be a non-empty string")?;
    let options = match args.get("options") {
        None | Some(Value::Null) => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or("ask_user error: 'options' must be an array of {label, description}")?
            .iter()
            .enumerate()
            .map(|(i, option)| {
                let label = option
                    .get("label")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .ok_or_else(|| {
                        format!("ask_user error: option {} is missing 'label' text", i + 1)
                    })?;
                let description = option
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .unwrap_or_default();
                Ok(json!({ "label": label, "description": description }))
            })
            .collect::<Result<Vec<_>, String>>()?,
    };
    let allow_freeform = args
        .get("allow_freeform")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if options.is_empty() && !allow_freeform {
        return Err(
            "ask_user error: a question with no options must allow a freeform answer".into(),
        );
    }
    Ok(json!({
        "v": 1,
        "source": "native",
        "question": question,
        "options": options,
        "allow_freeform": allow_freeform,
        "note": ASK_USER_NOTE,
    }))
}

/// Registered for every built-in session, not just plan mode: a fork mid-
/// execution deserves a question as much as one mid-planning.
pub struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        ASK_USER
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            ASK_USER,
            "Ask the user a question and wait for their decision. Use it when you hit a real fork \
             only they can settle — a destructive step, a choice between approaches, missing \
             requirements — not for confirmations you can infer or questions your own research can \
             answer. Offer the plausible choices as options and leave freeform on unless the answer \
             must be one of them. The question reaches the user as a card; end your turn right after \
             this call — their answer arrives as the next user message.",
            json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The question, complete and self-contained."
                    },
                    "options": {
                        "type": "array",
                        "description": "Suggested answers the user can pick with one click.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string", "description": "Short answer text; picking it sends exactly this." },
                                "description": { "type": "string", "description": "Optional one-line consequence or context." }
                            },
                            "required": ["label"]
                        }
                    },
                    "allow_freeform": {
                        "type": "boolean",
                        "description": "Let the user type their own answer. Defaults to true."
                    }
                },
                "required": ["question"]
            }),
        )
    }
    fn preview(&self, args: &Value) -> String {
        args.get("question")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .chars()
            .take(80)
            .collect()
    }
    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        match question_body(args) {
            Ok(body) => ToolResult::ok(body.to_string()).stop_turn(),
            Err(error) => ToolResult::fail(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{question_body, AskUserTool, ASK_USER};
    use crate::env::{ToolControl, ToolEnv, ToolEvent};
    use crate::tool::Tool;
    use serde_json::json;
    use std::path::Path;

    struct NoEnv;

    #[async_trait::async_trait]
    impl ToolEnv for NoEnv {
        fn project_root(&self) -> &Path {
            Path::new(".")
        }

        async fn confirm(&self, _message: &str) -> bool {
            false
        }

        async fn emit(&self, _event: ToolEvent) {}
    }

    #[test]
    fn builds_the_persisted_card_shape() {
        let body = question_body(&json!({
            "question": "  Which reference genome?  ",
            "options": [
                { "label": " GRCh38 ", "description": "current" },
                { "label": "T2T-CHM13" },
            ],
        }))
        .unwrap();
        assert_eq!(body["v"], 1);
        assert_eq!(body["source"], "native");
        assert_eq!(body["question"], "Which reference genome?");
        assert_eq!(
            body["options"],
            json!([
                { "label": "GRCh38", "description": "current" },
                { "label": "T2T-CHM13", "description": "" },
            ])
        );
        assert_eq!(body["allow_freeform"], true, "freeform defaults on");
        assert!(
            body["note"].as_str().unwrap().contains("End your turn"),
            "the result has to stop the agent, not invite it to keep going"
        );
        assert_eq!(AskUserTool.name(), ASK_USER);
    }

    #[tokio::test]
    async fn successful_question_is_a_hard_turn_boundary() {
        let result = AskUserTool
            .run(&json!({ "question": "Continue?" }), &NoEnv)
            .await;
        assert!(result.success);
        assert_eq!(result.control, ToolControl::StopTurn);
    }

    #[test]
    fn options_are_optional_but_freeform_only_questions_need_freeform() {
        let body = question_body(&json!({ "question": "Proceed how?" })).unwrap();
        assert_eq!(body["options"], json!([]));
        assert!(
            question_body(&json!({ "question": "Proceed how?", "allow_freeform": false })).is_err()
        );
    }

    #[test]
    fn rejects_junk() {
        for args in [
            json!({}),
            json!({ "question": "   " }),
            json!({ "question": "x", "options": "nope" }),
            json!({ "question": "x", "options": [{ "description": "no label" }] }),
            json!({ "question": "x", "options": [{ "label": "  " }] }),
        ] {
            assert!(question_body(&args).is_err(), "{args} should be refused");
        }
    }
}
