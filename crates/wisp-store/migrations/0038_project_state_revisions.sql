CREATE TABLE IF NOT EXISTS project_state_revisions (
    id                        TEXT PRIMARY KEY,
    project_id                TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    frame_id                  TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    turn_index                INTEGER NOT NULL CHECK(turn_index >= 0),
    message_seq               INTEGER NOT NULL CHECK(message_seq > 0),
    ui_event_seq              INTEGER NOT NULL CHECK(ui_event_seq >= 0),
    parent_revision_id        TEXT REFERENCES project_state_revisions(id) ON DELETE CASCADE,
    workspace_snapshot_id     TEXT NOT NULL REFERENCES workspace_snapshots(id) ON DELETE CASCADE,
    workspace_manifest_sha256 TEXT NOT NULL,
    workspace_delta_json      TEXT NOT NULL DEFAULT '[]',
    artifact_heads_json       TEXT NOT NULL DEFAULT '[]',
    entities_json             TEXT NOT NULL DEFAULT '[]',
    run_ids_json              TEXT NOT NULL DEFAULT '[]',
    decision_ids_json         TEXT NOT NULL DEFAULT '[]',
    external_effects_json     TEXT NOT NULL DEFAULT '[]',
    context_archive_id        TEXT NOT NULL REFERENCES context_archives(id) ON DELETE CASCADE,
    state_generation          INTEGER NOT NULL,
    is_full                   INTEGER NOT NULL DEFAULT 0 CHECK(is_full IN (0,1)),
    created_at                INTEGER NOT NULL,
    UNIQUE(frame_id, turn_index)
);
CREATE INDEX IF NOT EXISTS ix_project_state_revisions_project_created
    ON project_state_revisions(project_id, created_at, id);
CREATE INDEX IF NOT EXISTS ix_project_state_revisions_frame_turn
    ON project_state_revisions(frame_id, turn_index);
