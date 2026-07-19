CREATE TABLE IF NOT EXISTS transcript_versions (
    id          TEXT PRIMARY KEY,
    record_id   TEXT NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    provider    TEXT NOT NULL,
    model       TEXT NOT NULL,
    status      TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS transcript_segments (
    id                    TEXT PRIMARY KEY,
    transcript_version_id TEXT NOT NULL REFERENCES transcript_versions(id) ON DELETE CASCADE,
    record_id             TEXT NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    sequence              INTEGER NOT NULL,
    speaker_label         TEXT,
    start_ms              INTEGER NOT NULL,
    end_ms                INTEGER NOT NULL,
    original_text         TEXT NOT NULL,
    edited_text           TEXT,
    created_at            TEXT NOT NULL,
    updated_at            TEXT NOT NULL,
    UNIQUE(transcript_version_id, sequence)
);

CREATE INDEX IF NOT EXISTS transcript_versions_record_id_idx ON transcript_versions(record_id, created_at DESC);
CREATE INDEX IF NOT EXISTS transcript_segments_record_position_idx ON transcript_segments(record_id, start_ms);
