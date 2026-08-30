CREATE TABLE IF NOT EXISTS agent_workflow_run_activities (
    attempt_id    TEXT PRIMARY KEY
                  REFERENCES agent_workflow_attempts(id) ON DELETE CASCADE,
    run_id        TEXT NOT NULL UNIQUE
                  REFERENCES runs(id) ON DELETE RESTRICT,
    activity      TEXT NOT NULL,
    state_json    TEXT NOT NULL DEFAULT '{}',
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS ix_agent_workflow_run_activities_run
    ON agent_workflow_run_activities(run_id);
