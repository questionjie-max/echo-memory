CREATE TABLE IF NOT EXISTS analyses (
    id                           TEXT PRIMARY KEY,
    record_id                    TEXT NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    source_transcript_version_id TEXT NOT NULL REFERENCES transcript_versions(id) ON DELETE RESTRICT,
    status                       TEXT NOT NULL,
    content_json                 TEXT NOT NULL,
    provider                     TEXT NOT NULL,
    model                        TEXT NOT NULL,
    template_version             TEXT NOT NULL,
    created_at                   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS citations (
    id                    TEXT PRIMARY KEY,
    analysis_id           TEXT NOT NULL REFERENCES analyses(id) ON DELETE CASCADE,
    item_path             TEXT NOT NULL,
    transcript_segment_id TEXT NOT NULL REFERENCES transcript_segments(id) ON DELETE RESTRICT,
    quote_text            TEXT NOT NULL,
    verified              INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS action_items (
    id                TEXT PRIMARY KEY,
    record_id         TEXT NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    project_id        TEXT REFERENCES projects(id) ON DELETE SET NULL,
    title             TEXT NOT NULL,
    owner_text        TEXT NOT NULL DEFAULT '',
    due_text          TEXT NOT NULL DEFAULT '',
    status            TEXT NOT NULL DEFAULT 'open',
    source_segment_id TEXT REFERENCES transcript_segments(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS analyses_record_id_idx ON analyses(record_id, created_at DESC);
CREATE INDEX IF NOT EXISTS citations_analysis_id_idx ON citations(analysis_id);
CREATE INDEX IF NOT EXISTS action_items_project_status_idx ON action_items(project_id, status);
