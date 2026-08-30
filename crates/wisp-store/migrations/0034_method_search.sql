CREATE TABLE IF NOT EXISTS method_search_runs (
    run_id                      TEXT PRIMARY KEY
                                REFERENCES runs(id) ON DELETE CASCADE,
    spec_artifact_version_id    TEXT NOT NULL
                                REFERENCES artifact_versions(id) ON DELETE RESTRICT,
    spec_sha256                 TEXT NOT NULL,
    activity_version            INTEGER NOT NULL,
    checkpoint_json             TEXT NOT NULL DEFAULT '{}',
    result_status               TEXT,
    created_at                  INTEGER NOT NULL,
    updated_at                  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS method_candidate_blobs (
    id              TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL REFERENCES method_search_runs(run_id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,
    checksum        TEXT NOT NULL,
    size_bytes      INTEGER NOT NULL,
    storage_path    TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    UNIQUE(run_id, kind, checksum)
);

CREATE TABLE IF NOT EXISTS method_candidates (
    id                      TEXT PRIMARY KEY,
    run_id                  TEXT NOT NULL REFERENCES method_search_runs(run_id) ON DELETE CASCADE,
    parent_candidate_id     TEXT REFERENCES method_candidates(id) ON DELETE RESTRICT,
    sequence                INTEGER NOT NULL,
    strategy_key            TEXT NOT NULL,
    family                  TEXT NOT NULL DEFAULT 'unknown',
    status                  TEXT NOT NULL,
    primary_score           REAL,
    utility                 REAL,
    metrics_json            TEXT NOT NULL DEFAULT '{}',
    runtime_ms              INTEGER,
    source_sha256           TEXT NOT NULL,
    patch_sha256            TEXT NOT NULL,
    source_blob_id          TEXT REFERENCES method_candidate_blobs(id) ON DELETE RESTRICT,
    patch_blob_id           TEXT REFERENCES method_candidate_blobs(id) ON DELETE RESTRICT,
    changed_lines           INTEGER,
    dependency_count        INTEGER,
    rationale               TEXT,
    diagnostic_summary      TEXT,
    error                   TEXT,
    created_at              INTEGER NOT NULL,
    finished_at             INTEGER,
    UNIQUE(run_id, sequence)
);

CREATE INDEX IF NOT EXISTS ix_method_candidates_run_status
    ON method_candidates(run_id, status, sequence);
CREATE INDEX IF NOT EXISTS ix_method_candidates_parent
    ON method_candidates(parent_candidate_id);

CREATE TABLE IF NOT EXISTS method_strategy_stats (
    run_id              TEXT NOT NULL REFERENCES method_search_runs(run_id) ON DELETE CASCADE,
    strategy_key        TEXT NOT NULL,
    category            TEXT NOT NULL,
    weight              REAL NOT NULL,
    attempts            INTEGER NOT NULL,
    improvements        INTEGER NOT NULL,
    cumulative_reward   REAL NOT NULL,
    summary             TEXT NOT NULL,
    source_refs_json    TEXT NOT NULL DEFAULT '[]',
    updated_at          INTEGER NOT NULL,
    PRIMARY KEY(run_id, strategy_key)
);

CREATE INDEX IF NOT EXISTS ix_method_strategy_stats_run
    ON method_strategy_stats(run_id, category, strategy_key);
