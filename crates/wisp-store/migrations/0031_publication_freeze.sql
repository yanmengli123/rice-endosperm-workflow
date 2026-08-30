CREATE TABLE IF NOT EXISTS publication_freeze_attempts (
    id                TEXT PRIMARY KEY,
    revision_id       TEXT NOT NULL UNIQUE
                          REFERENCES publication_revisions(id) ON DELETE CASCADE,
    target_visibility TEXT NOT NULL
                          CHECK(target_visibility IN ('public','restricted','private')),
    policy_json       TEXT NOT NULL,
    started_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_publication_freeze_attempts_started
    ON publication_freeze_attempts(started_at, revision_id);
