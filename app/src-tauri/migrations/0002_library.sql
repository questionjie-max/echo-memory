-- 0002 资料库领域表（回声记忆 M1a）
-- 由 Rust 迁移机制应用。所有表创建均可重复执行。

CREATE TABLE IF NOT EXISTS projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status      TEXT NOT NULL DEFAULT 'active',   -- active | archived
    created_at  TEXT NOT NULL,                    -- UTC ISO-8601
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS records (
    id                TEXT PRIMARY KEY,
    title             TEXT NOT NULL,
    project_id        TEXT REFERENCES projects(id) ON DELETE SET NULL,
    source_type       TEXT NOT NULL DEFAULT 'import',
    audio_path        TEXT NOT NULL,               -- 受管理目录内相对路径
    audio_hash        TEXT NOT NULL,               -- SHA-256，导入查重键
    audio_duration_ms INTEGER NOT NULL DEFAULT 0,  -- 毫秒整数
    imported_at       TEXT NOT NULL,
    processing_status TEXT NOT NULL DEFAULT 'queued',
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS processing_jobs (
    id            TEXT PRIMARY KEY,
    record_id     TEXT NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    job_type      TEXT NOT NULL,                   -- transcribe | analyze
    status        TEXT NOT NULL DEFAULT 'queued',  -- queued|preparing|transcribing|analyzing|completed|failed
    attempt_count INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS records_project_id_idx ON records(project_id);
CREATE INDEX IF NOT EXISTS records_audio_hash_idx ON records(audio_hash);
CREATE INDEX IF NOT EXISTS processing_jobs_record_id_idx ON processing_jobs(record_id);
CREATE INDEX IF NOT EXISTS processing_jobs_status_idx ON processing_jobs(status);
