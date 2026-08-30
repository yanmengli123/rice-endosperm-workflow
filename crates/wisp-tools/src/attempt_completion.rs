//! `attempt_completion` — signal the task is done and present the final result.

use crate::env::{ToolEnv, ToolResult};
use crate::tool::{arg_str, Tool};
use async_trait::async_trait;
use serde_json::json;
use wisp_llm::ToolSchema;

pub struct AttemptCompletionTool;

#[async_trait]
impl Tool for AttemptCompletionTool {
    fn name(&self) -> &str {
        "attempt_completion"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "attempt_completion",
            "Indicate that the task is complete and provide the final result/answer to the user.",
            json!({
                "type": "object",
                "properties": {
                    "result": { "type": "string", "description": "The final result or summary of the completed task" }
                },
                "required": ["result"]
            }),
        )
    }
    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let result = match arg_str(args, "result") {
            Ok(r) => r,
            Err(e) => return ToolResult::fail(e),
        };
        // An empty result would end the turn with no visible answer — the UI
        // hides this tool row and only promotes non-empty result text, so the
        // user sees a bare "Processed" block (#798). Refuse it so the model
        // retries with the actual final answer.
        if result.trim().is_empty() {
            return ToolResult::fail(
                "attempt_completion error: 'result' must contain the final answer text for the \
                 user — it is the only text shown as your response. Call attempt_completion \
                 again with the complete result.",
            );
        }
        ToolResult::ok(result).stop_turn()
    }
}

#[cfg(test)]
mod tests {
    use super::AttemptCompletionTool;
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

    #[tokio::test]
    async fn non_empty_result_stops_the_turn() {
        let result = AttemptCompletionTool
            .run(&json!({ "result": "All 12 samples aligned." }), &NoEnv)
            .await;
        assert!(result.success);
        assert_eq!(result.control, ToolControl::StopTurn);
        assert_eq!(result.content, "All 12 samples aligned.");
    }

    // #798: an empty result used to end the turn "successfully" while the UI
    // had nothing to show — the user saw a blank "Processed" row. It must be
    // a tool failure that keeps the loop running so the model supplies the
    // real answer.
    #[tokio::test]
    async fn empty_result_is_rejected_instead_of_ending_the_turn() {
        for args in [
            json!({}),
            json!({ "result": "" }),
            json!({ "result": "  \n" }),
        ] {
            let result = AttemptCompletionTool.run(&args, &NoEnv).await;
            assert!(!result.success, "{args} must not be accepted");
            assert_eq!(
                result.control,
                ToolControl::Continue,
                "a rejected completion must not stop the turn"
            );
        }
    }
}
