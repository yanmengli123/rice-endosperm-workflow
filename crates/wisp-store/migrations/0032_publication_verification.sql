CREATE TABLE IF NOT EXISTS reproduction_runs (
    id                        TEXT PRIMARY KEY,
    revision_id               TEXT NOT NULL
                                      REFERENCES publication_revisions(id) ON DELETE CASCADE,
    source_run_id             TEXT NOT NULL REFERENCES runs(id) ON DELETE RESTRICT,
    status                    TEXT NOT NULL
                                      CHECK(status IN ('running','completed','failed')),
    capability_level          TEXT NOT NULL
                                      CHECK(capability_level IN ('re_executable','reproduced')),
    command_sha256            TEXT NOT NULL,
    expected_environment_hash TEXT,
    actual_environment_json   TEXT NOT NULL,
    actual_environment_hash   TEXT NOT NULL,
    environment_matched       INTEGER NOT NULL DEFAULT 0,
    workspace_manifest_json   TEXT NOT NULL,
    stdout_tail               TEXT,
    stderr_tail               TEXT,
    exit_code                 INTEGER,
    error                     TEXT,
    created_at                INTEGER NOT NULL,
    started_at                INTEGER NOT NULL,
    completed_at              INTEGER
);
CREATE INDEX IF NOT EXISTS ix_reproduction_runs_revision
    ON reproduction_runs(revision_id, created_at DESC, id);
CREATE INDEX IF NOT EXISTS ix_reproduction_runs_source
    ON reproduction_runs(source_run_id, created_at DESC, id);

CREATE TABLE IF NOT EXISTS reproduction_results (
    id                           TEXT PRIMARY KEY,
    reproduction_run_id          TEXT NOT NULL
                                         REFERENCES reproduction_runs(id) ON DELETE CASCADE,
    output_id                    TEXT NOT NULL,
    output_path                  TEXT NOT NULL,
    expected_artifact_version_id TEXT NOT NULL
                                         REFERENCES artifact_versions(id) ON DELETE RESTRICT,
    comparator_kind              TEXT NOT NULL
                                         CHECK(comparator_kind IN
                                               ('sha256','text','json','numeric')),
    required                     INTEGER NOT NULL DEFAULT 1,
    expected_json                TEXT NOT NULL,
    actual_json                  TEXT NOT NULL,
    tolerance_json               TEXT NOT NULL DEFAULT '{}',
    passed                       INTEGER NOT NULL,
    report_json                  TEXT NOT NULL DEFAULT '{}',
    created_at                   INTEGER NOT NULL,
    UNIQUE(reproduction_run_id, output_id)
);
CREATE INDEX IF NOT EXISTS ix_reproduction_results_run
    ON reproduction_results(reproduction_run_id, output_id);
