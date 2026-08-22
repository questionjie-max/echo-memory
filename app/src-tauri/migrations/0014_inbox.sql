-- 音频收件箱：监听文件夹与已见文件（去重与状态跟踪）。
CREATE TABLE IF NOT EXISTS inbox_watch_folders (
    id         TEXT PRIMARY KEY,
    path       TEXT NOT NULL UNIQUE,
    label      TEXT NOT NULL DEFAULT '',
    enabled    INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS inbox_seen_files (
    id            TEXT PRIMARY KEY,
    source_kind   TEXT NOT NULL, -- folder | volume
    source_path   TEXT NOT NULL, -- 监听目录或卷挂载点
    file_path     TEXT NOT NULL UNIQUE,
    file_name     TEXT NOT NULL,
    file_size     INTEGER NOT NULL DEFAULT 0,
    mtime_ms      INTEGER NOT NULL DEFAULT 0,
    sha256        TEXT,
    status        TEXT NOT NULL DEFAULT 'pending', -- pending|importing|imported|duplicate|skipped|failed
    record_id     TEXT,
    error_message TEXT,
    seen_at       TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_inbox_seen_status ON inbox_seen_files(status, seen_at);

INSERT OR IGNORE INTO app_settings (key, value, updated_at)
VALUES ('inbox_usb_detection', 'false', CURRENT_TIMESTAMP);
