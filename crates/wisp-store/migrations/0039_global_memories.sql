CREATE TABLE IF NOT EXISTS global_memories (
    id                TEXT PRIMARY KEY,
    content           TEXT NOT NULL CHECK(length(trim(content)) > 0),
    source_frame_id   TEXT REFERENCES frames(id) ON DELETE SET NULL,
    source_turn_index INTEGER CHECK(source_turn_index IS NULL OR source_turn_index >= 0),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_global_memories_updated
    ON global_memories(updated_at DESC, id DESC);
