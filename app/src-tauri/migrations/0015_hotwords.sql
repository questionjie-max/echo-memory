-- 个人词汇库：注入 Whisper initial_prompt 与转写二次校对，提升中文专名识别率。
CREATE TABLE IF NOT EXISTS hotwords (
    id         TEXT PRIMARY KEY,
    term       TEXT NOT NULL UNIQUE,
    note       TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
