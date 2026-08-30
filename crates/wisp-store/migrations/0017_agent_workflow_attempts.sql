CREATE TABLE IF NOT EXISTS agent_workflow_attempts (
    id                    TEXT PRIMARY KEY,
    workflow_id           TEXT NOT NULL REFERENCES agent_workflows(id) ON DELETE CASCADE,
    step_id               TEXT NOT NULL REFERENCES agent_workflow_steps(id) ON DELETE CASCADE,
    attempt               INTEGER NOT NULL,
    request_id            TEXT NOT NULL UNIQUE,
    backend               TEXT NOT NULL,
    status                TEXT NOT NULL,
    request_json          TEXT NOT NULL,
    response_json         TEXT,
    output_json           TEXT NOT NULL DEFAULT '{}',
    artifact_ids_json     TEXT NOT NULL DEFAULT '[]',
    evidence_json         TEXT NOT NULL DEFAULT '[]',
    error                 TEXT,
    agent_session_id      TEXT,
    child_frame_id        TEXT,
    input_tokens          INTEGER NOT NULL DEFAULT 0,
    output_tokens         INTEGER NOT NULL DEFAULT 0,
    tool_calls            INTEGER NOT NULL DEFAULT 0,
    cost_microunits       INTEGER NOT NULL DEFAULT 0,
    cancel_requested      INTEGER NOT NULL DEFAULT 0,
    started_at            INTEGER,
    finished_at           INTEGER,
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL,
    UNIQUE(step_id, attempt)
);

CREATE INDEX IF NOT EXISTS ix_agent_workflow_attempts_workflow
    ON agent_workflow_attempts(workflow_id, status, created_at);
CREATE INDEX IF NOT EXISTS ix_agent_workflow_attempts_step
    ON agent_workflow_attempts(step_id, attempt DESC);
