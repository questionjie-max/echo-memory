-- 确认过的记忆条目进入知识检索。独立于 knowledge_chunks：那张表的 record_id
-- 外键绑定录音/文档记录，而记忆条目活在快照 JSON 里、归属快照而非记录。
CREATE TABLE IF NOT EXISTS memory_chunks (
    id                   TEXT PRIMARY KEY,
    snapshot_id          TEXT NOT NULL REFERENCES memory_snapshots(id) ON DELETE CASCADE,
    view_kind            TEXT NOT NULL,
    item_id              TEXT NOT NULL,
    item_type            TEXT NOT NULL,
    topic                TEXT NOT NULL DEFAULT '',
    before_text          TEXT NOT NULL DEFAULT '',
    after_text           TEXT NOT NULL DEFAULT '',
    body                 TEXT NOT NULL,
    occurred_at          TEXT NOT NULL DEFAULT '',
    sources_json         TEXT NOT NULL DEFAULT '[]',
    content_hash         TEXT NOT NULL,
    embedding_model      TEXT NOT NULL,
    embedding_dimensions INTEGER NOT NULL,
    embedding_blob       BLOB NOT NULL,
    updated_at           TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS memory_chunks_snapshot_idx ON memory_chunks(snapshot_id);
CREATE INDEX IF NOT EXISTS memory_chunks_item_idx ON memory_chunks(item_id);

-- 与 record_search 同款 trigram 分词；由索引重建代码负责同步（记忆块成批整体替换，不走触发器）。
CREATE VIRTUAL TABLE IF NOT EXISTS memory_chunks_search USING fts5(
    chunk_id UNINDEXED,
    body,
    tokenize = 'trigram'
);

-- 记忆库是全局召回，不随项目过滤：插入常驻 scope 行，让索引状态条可见。
INSERT OR IGNORE INTO knowledge_index_state (scope_key, status, total_records, processed_records, chunk_count, embedding_model, updated_at)
VALUES ('memory', 'not_built', 0, 0, 0, 'qwen3-embedding:0.6b', datetime('now'));
