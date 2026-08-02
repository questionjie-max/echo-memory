-- External-AI memory views. Generated inference is versioned and never written back to source facts.
CREATE TABLE IF NOT EXISTS memory_snapshots (
    id                     TEXT PRIMARY KEY,
    view_kind              TEXT NOT NULL CHECK(view_kind IN ('timeline', 'map', 'evolution')),
    scope_kind             TEXT NOT NULL CHECK(scope_kind IN ('all', 'project', 'unfiled')),
    scope_key              TEXT NOT NULL,
    range_start            TEXT,
    range_end              TEXT,
    status                 TEXT NOT NULL CHECK(status IN ('generating', 'completed', 'partial', 'failed', 'cancelled')),
    provider               TEXT NOT NULL DEFAULT 'openai-compatible',
    model                  TEXT NOT NULL,
    source_record_ids_json TEXT NOT NULL DEFAULT '[]',
    request_hash           TEXT NOT NULL,
    result_json            TEXT NOT NULL DEFAULT '{}',
    quality_warning        TEXT,
    error_message          TEXT,
    is_stale               INTEGER NOT NULL DEFAULT 0,
    version                INTEGER NOT NULL,
    created_at             TEXT NOT NULL,
    updated_at             TEXT NOT NULL,
    UNIQUE(view_kind, scope_key, range_start, range_end, version)
);

CREATE INDEX IF NOT EXISTS memory_snapshots_scope_idx
    ON memory_snapshots(view_kind, scope_key, range_start, range_end, version DESC);
CREATE INDEX IF NOT EXISTS memory_snapshots_status_idx
    ON memory_snapshots(status, updated_at DESC);

CREATE TABLE IF NOT EXISTS memory_feedback (
    id          TEXT PRIMARY KEY,
    snapshot_id TEXT NOT NULL REFERENCES memory_snapshots(id) ON DELETE CASCADE,
    item_id     TEXT NOT NULL,
    decision    TEXT NOT NULL CHECK(decision IN ('confirmed', 'rejected')),
    note        TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    UNIQUE(snapshot_id, item_id)
);

CREATE INDEX IF NOT EXISTS memory_feedback_snapshot_idx
    ON memory_feedback(snapshot_id, updated_at DESC);

-- Any source mutation keeps history but marks affected generated snapshots as potentially stale.
CREATE TRIGGER IF NOT EXISTS memory_stale_record_update
AFTER UPDATE OF title, project_id ON records
BEGIN
    UPDATE memory_snapshots SET is_stale = 1, updated_at = datetime('now')
    WHERE EXISTS (
        SELECT 1 FROM json_each(memory_snapshots.source_record_ids_json)
        WHERE json_each.value = NEW.id
    );
END;

CREATE TRIGGER IF NOT EXISTS memory_stale_transcript_version_insert
AFTER INSERT ON transcript_versions
BEGIN
    UPDATE memory_snapshots SET is_stale = 1, updated_at = datetime('now')
    WHERE EXISTS (
        SELECT 1 FROM json_each(memory_snapshots.source_record_ids_json)
        WHERE json_each.value = NEW.record_id
    );
END;

CREATE TRIGGER IF NOT EXISTS memory_stale_transcript_segment_update
AFTER UPDATE OF edited_text, original_text, normalized_text ON transcript_segments
BEGIN
    UPDATE memory_snapshots SET is_stale = 1, updated_at = datetime('now')
    WHERE EXISTS (
        SELECT 1 FROM json_each(memory_snapshots.source_record_ids_json)
        WHERE json_each.value = NEW.record_id
    );
END;

CREATE TRIGGER IF NOT EXISTS memory_stale_analysis_insert
AFTER INSERT ON analyses
BEGIN
    UPDATE memory_snapshots SET is_stale = 1, updated_at = datetime('now')
    WHERE EXISTS (
        SELECT 1 FROM json_each(memory_snapshots.source_record_ids_json)
        WHERE json_each.value = NEW.record_id
    );
END;
