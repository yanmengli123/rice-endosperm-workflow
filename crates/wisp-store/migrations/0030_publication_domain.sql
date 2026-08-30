CREATE TABLE IF NOT EXISTS publications (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_publications_project
    ON publications(project_id, updated_at DESC, id);

CREATE TABLE IF NOT EXISTS publication_revisions (
    id                  TEXT PRIMARY KEY,
    publication_id      TEXT NOT NULL REFERENCES publications(id) ON DELETE CASCADE,
    parent_revision_id  TEXT REFERENCES publication_revisions(id) ON DELETE SET NULL,
    revision_number     INTEGER NOT NULL CHECK(revision_number > 0),
    label               TEXT NOT NULL,
    state               TEXT NOT NULL DEFAULT 'draft'
                            CHECK(state IN ('draft','freezing','frozen','published','deleting')),
    capability_level    TEXT NOT NULL DEFAULT 'archived'
                            CHECK(capability_level IN
                                  ('archived','traceable','re_executable','reproduced')),
    manifest_json       TEXT,
    manifest_sha256     TEXT,
    frozen_at           INTEGER,
    published_at        INTEGER,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    CHECK(state NOT IN ('frozen','published')
          OR (manifest_json IS NOT NULL AND length(manifest_sha256)=64 AND frozen_at IS NOT NULL)),
    CHECK(state <> 'published' OR published_at IS NOT NULL),
    UNIQUE(publication_id, revision_number)
);
CREATE INDEX IF NOT EXISTS ix_publication_revisions_publication
    ON publication_revisions(publication_id, revision_number DESC);

CREATE TABLE IF NOT EXISTS publication_items (
    id             TEXT PRIMARY KEY,
    revision_id    TEXT NOT NULL REFERENCES publication_revisions(id) ON DELETE CASCADE,
    parent_item_id TEXT REFERENCES publication_items(id) ON DELETE CASCADE,
    kind           TEXT NOT NULL
                       CHECK(kind IN ('section','claim','figure','table','methods','supplement')),
    title          TEXT NOT NULL,
    content        TEXT NOT NULL DEFAULT '',
    ordinal        INTEGER NOT NULL,
    metadata_json  TEXT NOT NULL DEFAULT '{}',
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_publication_items_revision_order
    ON publication_items(revision_id, parent_item_id, ordinal, id);
CREATE UNIQUE INDEX IF NOT EXISTS ux_publication_items_sibling_order
    ON publication_items(revision_id, COALESCE(parent_item_id,''), ordinal);

CREATE TABLE IF NOT EXISTS publication_item_links (
    id             TEXT PRIMARY KEY,
    revision_id    TEXT NOT NULL REFERENCES publication_revisions(id) ON DELETE CASCADE,
    source_item_id TEXT NOT NULL REFERENCES publication_items(id) ON DELETE CASCADE,
    target_item_id TEXT NOT NULL REFERENCES publication_items(id) ON DELETE CASCADE,
    relation       TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    CHECK(source_item_id <> target_item_id),
    UNIQUE(revision_id, source_item_id, target_item_id, relation)
);
CREATE INDEX IF NOT EXISTS ix_publication_item_links_revision
    ON publication_item_links(revision_id, source_item_id, target_item_id);

CREATE TABLE IF NOT EXISTS evidence_bindings (
    id                      TEXT PRIMARY KEY,
    revision_id             TEXT NOT NULL REFERENCES publication_revisions(id) ON DELETE CASCADE,
    item_id                 TEXT REFERENCES publication_items(id) ON DELETE CASCADE,
    source_kind             TEXT NOT NULL
                                CHECK(source_kind IN
                                      ('artifact_version','run','execution_log','message_span',
                                       'tool_call','code_cell','external_resource')),
    source_id               TEXT NOT NULL,
    artifact_version_id     TEXT REFERENCES artifact_versions(id) ON DELETE RESTRICT,
    run_id                  TEXT REFERENCES runs(id) ON DELETE RESTRICT,
    external_resource_id    TEXT REFERENCES external_resources(id) ON DELETE RESTRICT,
    purpose                 TEXT NOT NULL DEFAULT '',
    supported_claim_item_id TEXT REFERENCES publication_items(id) ON DELETE SET NULL,
    selection_state         TEXT NOT NULL DEFAULT 'candidate'
                                CHECK(selection_state IN ('candidate','selected','rejected')),
    review_state            TEXT NOT NULL DEFAULT 'unreviewed'
                                CHECK(review_state IN ('unreviewed','reviewed')),
    reproduction_state      TEXT NOT NULL DEFAULT 'not_run'
                                CHECK(reproduction_state IN
                                      ('not_run','passed','failed','not_applicable')),
    visibility              TEXT NOT NULL DEFAULT 'private'
                                CHECK(visibility IN ('public','restricted','private')),
    source_snapshot_json    TEXT NOT NULL,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    CHECK(
        (source_kind='artifact_version'
         AND artifact_version_id IS NOT NULL AND source_id=artifact_version_id
         AND run_id IS NULL AND external_resource_id IS NULL)
        OR
        (source_kind='run'
         AND run_id IS NOT NULL AND source_id=run_id
         AND artifact_version_id IS NULL AND external_resource_id IS NULL)
        OR
        (source_kind='external_resource'
         AND external_resource_id IS NOT NULL AND source_id=external_resource_id
         AND artifact_version_id IS NULL AND run_id IS NULL)
        OR
        (source_kind IN ('execution_log','message_span','tool_call','code_cell')
         AND artifact_version_id IS NULL AND run_id IS NULL AND external_resource_id IS NULL)
    )
);
CREATE INDEX IF NOT EXISTS ix_evidence_bindings_revision
    ON evidence_bindings(revision_id, item_id, selection_state, id);
CREATE INDEX IF NOT EXISTS ix_evidence_bindings_artifact_version
    ON evidence_bindings(artifact_version_id);
CREATE INDEX IF NOT EXISTS ix_evidence_bindings_run
    ON evidence_bindings(run_id);

CREATE TABLE IF NOT EXISTS evidence_reviews (
    id               TEXT PRIMARY KEY,
    binding_id       TEXT NOT NULL REFERENCES evidence_bindings(id) ON DELETE CASCADE,
    reviewer         TEXT NOT NULL,
    method           TEXT NOT NULL,
    verified_at      INTEGER NOT NULL,
    environment_json TEXT NOT NULL DEFAULT '{}',
    comparator_json  TEXT NOT NULL DEFAULT '{}',
    tolerance_json   TEXT NOT NULL DEFAULT '{}',
    result           TEXT NOT NULL,
    report_json      TEXT NOT NULL DEFAULT '{}',
    created_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_evidence_reviews_binding
    ON evidence_reviews(binding_id, verified_at, id);

CREATE TABLE IF NOT EXISTS evidence_supersessions (
    id             TEXT PRIMARY KEY,
    revision_id    TEXT NOT NULL REFERENCES publication_revisions(id) ON DELETE CASCADE,
    old_binding_id TEXT NOT NULL REFERENCES evidence_bindings(id) ON DELETE CASCADE,
    new_binding_id TEXT NOT NULL REFERENCES evidence_bindings(id) ON DELETE CASCADE,
    reason         TEXT NOT NULL DEFAULT '',
    created_at     INTEGER NOT NULL,
    CHECK(old_binding_id <> new_binding_id),
    UNIQUE(revision_id, old_binding_id)
);
CREATE INDEX IF NOT EXISTS ix_evidence_supersessions_revision
    ON evidence_supersessions(revision_id, old_binding_id);

CREATE TABLE IF NOT EXISTS publication_readiness_reports (
    id                TEXT PRIMARY KEY,
    revision_id       TEXT NOT NULL UNIQUE
                          REFERENCES publication_revisions(id) ON DELETE CASCADE,
    capability_level  TEXT NOT NULL
                          CHECK(capability_level IN
                                ('archived','traceable','re_executable','reproduced')),
    blockers_json     TEXT NOT NULL DEFAULT '[]',
    warnings_json     TEXT NOT NULL DEFAULT '[]',
    omissions_json    TEXT NOT NULL DEFAULT '[]',
    manifest_json     TEXT NOT NULL,
    manifest_sha256   TEXT NOT NULL,
    created_at        INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS publication_waivers (
    id           TEXT PRIMARY KEY,
    revision_id  TEXT NOT NULL REFERENCES publication_revisions(id) ON DELETE CASCADE,
    finding_code TEXT NOT NULL,
    author       TEXT NOT NULL,
    reason       TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    UNIQUE(revision_id, finding_code)
);
CREATE INDEX IF NOT EXISTS ix_publication_waivers_revision
    ON publication_waivers(revision_id, finding_code);

CREATE TABLE IF NOT EXISTS capsule_builds (
    id                       TEXT PRIMARY KEY,
    revision_id              TEXT NOT NULL
                                 REFERENCES publication_revisions(id) ON DELETE CASCADE,
    format                   TEXT NOT NULL,
    visibility               TEXT NOT NULL
                                 CHECK(visibility IN ('public','restricted','private')),
    status                   TEXT NOT NULL,
    output_path              TEXT,
    revision_manifest_sha256 TEXT NOT NULL,
    archive_sha256           TEXT,
    error                    TEXT,
    created_at               INTEGER NOT NULL,
    completed_at             INTEGER
);
CREATE INDEX IF NOT EXISTS ix_capsule_builds_revision
    ON capsule_builds(revision_id, created_at DESC, id);
