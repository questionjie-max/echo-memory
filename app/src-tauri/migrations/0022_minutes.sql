-- 会议纪要（v1.1 块3/块4）。
-- 纪要失败不阻塞录音入库：minutes 行状态独立于 records.status，
-- 生成失败记 failed（可重试），录音与逐字稿照常可用。
-- minutes_items 是条目级两态（块4）：未确认的条目绝不进入记忆库、MCP 读不到。
CREATE TABLE IF NOT EXISTS minutes (
    id            TEXT PRIMARY KEY,
    record_id     TEXT NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    status        TEXT NOT NULL CHECK (status IN ('pending','generating','completed','failed')),
    content_json  TEXT NOT NULL DEFAULT '{}',
    provider      TEXT,
    model         TEXT,
    error_message TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS minutes_record_idx ON minutes(record_id);
-- 一条记录一份纪要：UNIQUE 防并发 ensure/生成插入两行后读取不确定。
CREATE UNIQUE INDEX IF NOT EXISTS minutes_record_unique ON minutes(record_id);

CREATE TABLE IF NOT EXISTS minutes_items (
    id            TEXT PRIMARY KEY,
    minutes_id    TEXT NOT NULL REFERENCES minutes(id) ON DELETE CASCADE,
    record_id     TEXT NOT NULL,
    kind          TEXT NOT NULL CHECK (kind IN ('conclusion','action')),
    text          TEXT NOT NULL,
    owner_text    TEXT,
    evidence_json TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'provisional' CHECK (status IN ('provisional','confirmed')),
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS minutes_items_record_idx ON minutes_items(record_id, status);
