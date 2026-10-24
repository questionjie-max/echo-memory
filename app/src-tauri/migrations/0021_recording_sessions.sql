-- 本地录音的持久化生命周期；不重建 records，避免外键级联丢失既有资料。
CREATE TABLE IF NOT EXISTS recording_sessions (
    id            TEXT PRIMARY KEY,
    status        TEXT NOT NULL, -- starting|recording|paused|finalizing|completed|failed|recoverable|canceled
    temp_path     TEXT NOT NULL, -- 资料库内相对路径，不存绝对路径
    title         TEXT NOT NULL DEFAULT '',
    project_id    TEXT REFERENCES projects(id) ON DELETE SET NULL,
    started_at    TEXT NOT NULL,
    ended_at      TEXT,
    duration_ms   INTEGER NOT NULL DEFAULT 0,
    paused_ms     INTEGER NOT NULL DEFAULT 0,
    input_device  TEXT,
    sample_rate   INTEGER NOT NULL DEFAULT 48000,
    channels      INTEGER NOT NULL DEFAULT 1,
    written_bytes INTEGER NOT NULL DEFAULT 0,
    record_id     TEXT REFERENCES records(id) ON DELETE SET NULL,
    last_error    TEXT,
    updated_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS recording_sessions_status_idx
    ON recording_sessions(status, updated_at);
CREATE INDEX IF NOT EXISTS recording_sessions_record_id_idx
    ON recording_sessions(record_id);
