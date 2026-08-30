CREATE TABLE IF NOT EXISTS project_state_counters (
    project_id          TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    mainline_generation INTEGER NOT NULL DEFAULT 0,
    updated_at          INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS workspace_snapshots (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    manifest_json   TEXT NOT NULL,
    manifest_sha256 TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_workspace_snapshots_project
    ON workspace_snapshots(project_id, created_at DESC);

CREATE TABLE IF NOT EXISTS context_archives (
    id           TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    frame_id     TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    storage_path TEXT NOT NULL,
    checksum     TEXT NOT NULL,
    created_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_context_archives_frame
    ON context_archives(frame_id, created_at DESC);

CREATE TABLE IF NOT EXISTS exploration_families (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    root_frame_id     TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    mainline_frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    generation        INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    UNIQUE(project_id, root_frame_id)
);
CREATE INDEX IF NOT EXISTS ix_exploration_families_mainline
    ON exploration_families(project_id, mainline_frame_id);

CREATE TABLE IF NOT EXISTS exploration_checkpoints (
    id                      TEXT PRIMARY KEY,
    family_id               TEXT NOT NULL REFERENCES exploration_families(id) ON DELETE CASCADE,
    project_id              TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    source_frame_id         TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    source_message_seq      INTEGER NOT NULL,
    source_frame_head_seq   INTEGER NOT NULL,
    source_ui_event_seq      INTEGER NOT NULL,
    source_family_generation INTEGER NOT NULL,
    source_state_generation  INTEGER NOT NULL,
    workspace_snapshot_id   TEXT NOT NULL REFERENCES workspace_snapshots(id) ON DELETE RESTRICT,
    context_archive_id      TEXT NOT NULL REFERENCES context_archives(id) ON DELETE RESTRICT,
    guard_hash              TEXT NOT NULL,
    entity_hash             TEXT NOT NULL,
    isolation_summary_json  TEXT NOT NULL DEFAULT '{}',
    created_at              INTEGER NOT NULL,
    UNIQUE(family_id, source_frame_id, source_message_seq, guard_hash)
);
CREATE INDEX IF NOT EXISTS ix_exploration_checkpoints_source
    ON exploration_checkpoints(source_frame_id, source_message_seq);

CREATE TABLE IF NOT EXISTS explorations (
    id               TEXT PRIMARY KEY,
    checkpoint_id    TEXT NOT NULL REFERENCES exploration_checkpoints(id) ON DELETE CASCADE,
    frame_id          TEXT NOT NULL UNIQUE REFERENCES frames(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    status            TEXT NOT NULL
                          CHECK(status IN ('creating','active','promoting','failed')),
    workspace_dir     TEXT NOT NULL,
    workspace_backend TEXT NOT NULL,
    scope_generation  INTEGER NOT NULL DEFAULT 0,
    warnings_json     TEXT NOT NULL DEFAULT '[]',
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_explorations_checkpoint_status
    ON explorations(checkpoint_id, status, created_at);

CREATE TABLE IF NOT EXISTS exploration_baseline_entities (
    checkpoint_id TEXT NOT NULL REFERENCES exploration_checkpoints(id) ON DELETE CASCADE,
    entity_kind   TEXT NOT NULL,
    entity_id     TEXT NOT NULL,
    version_id    TEXT,
    fingerprint   TEXT NOT NULL,
    PRIMARY KEY(checkpoint_id, entity_kind, entity_id)
);

CREATE TABLE IF NOT EXISTS exploration_baseline_artifact_heads (
    checkpoint_id       TEXT NOT NULL REFERENCES exploration_checkpoints(id) ON DELETE CASCADE,
    logical_key         TEXT NOT NULL,
    artifact_id         TEXT NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
    artifact_version_id TEXT NOT NULL REFERENCES artifact_versions(id) ON DELETE RESTRICT,
    fingerprint         TEXT NOT NULL,
    PRIMARY KEY(checkpoint_id, logical_key)
);

CREATE TABLE IF NOT EXISTS artifact_heads (
    project_id          TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    scope_key           TEXT NOT NULL,
    logical_key         TEXT NOT NULL,
    artifact_id         TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    artifact_version_id TEXT NOT NULL REFERENCES artifact_versions(id) ON DELETE CASCADE,
    updated_at          INTEGER NOT NULL,
    PRIMARY KEY(project_id, scope_key, logical_key)
);
CREATE INDEX IF NOT EXISTS ix_artifact_heads_artifact
    ON artifact_heads(artifact_id, artifact_version_id);

CREATE TABLE IF NOT EXISTS exploration_effects (
    id             TEXT PRIMARY KEY,
    exploration_id TEXT NOT NULL REFERENCES explorations(id) ON DELETE CASCADE,
    effect_kind    TEXT NOT NULL,
    recoverability TEXT NOT NULL,
    target_summary TEXT NOT NULL,
    metadata_json  TEXT NOT NULL DEFAULT '{}',
    created_at     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_exploration_effects_exploration
    ON exploration_effects(exploration_id, created_at);

CREATE TABLE IF NOT EXISTS exploration_promotions (
    id                  TEXT PRIMARY KEY,
    exploration_id      TEXT NOT NULL,
    expected_guard_hash TEXT NOT NULL,
    status              TEXT NOT NULL,
    diff_json           TEXT NOT NULL,
    journal_path        TEXT,
    error               TEXT,
    started_at          INTEGER NOT NULL,
    committed_at        INTEGER
);
CREATE INDEX IF NOT EXISTS ix_exploration_promotions_exploration
    ON exploration_promotions(exploration_id, started_at DESC);
