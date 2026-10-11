-- L3 个人总述：memory_snapshots 的 view_kind CHECK 需要放行 'profile'。
-- SQLite 无法修改 CHECK，按标准做法重建表：数据平移、索引与触发器重建。
--
-- 外键陷阱：应用的业务连接开着 PRAGMA foreign_keys=ON。只要某个连接以
-- FK=ON 执行本文件，DROP TABLE memory_snapshots 就会触发
-- memory_feedback ON DELETE CASCADE，把用户全部确认/驳回记录清空
-- （FK 恒为 OFF 的迁移自测连接发现不了这件事）。所以先把反馈内容
-- 存进无外键的普通表，重建完成后再写回。
CREATE TABLE IF NOT EXISTS memory_feedback_preserved AS
    SELECT id, snapshot_id, item_id, decision, note, created_at, updated_at FROM memory_feedback;

CREATE TABLE IF NOT EXISTS memory_snapshots_rebuild (
    id                     TEXT PRIMARY KEY,
    view_kind              TEXT NOT NULL CHECK(view_kind IN ('timeline', 'map', 'evolution', 'profile')),
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

INSERT INTO memory_snapshots_rebuild
    (id, view_kind, scope_kind, scope_key, range_start, range_end, status, provider, model, source_record_ids_json, request_hash, result_json, quality_warning, error_message, is_stale, version, created_at, updated_at)
SELECT
    id, view_kind, scope_kind, scope_key, range_start, range_end, status, provider, model, source_record_ids_json, request_hash, result_json, quality_warning, error_message, is_stale, version, created_at, updated_at
FROM memory_snapshots;

-- 四个 stale 触发器挂在 records/transcript_versions/transcript_segments/analyses
-- 上、但函数体引用 memory_snapshots：必须先删触发器再删表，否则 DROP 时
-- SQLite 重解析触发器体会报 no such table。
DROP TRIGGER IF EXISTS memory_stale_record_update;
DROP TRIGGER IF EXISTS memory_stale_transcript_version_insert;
DROP TRIGGER IF EXISTS memory_stale_transcript_segment_update;
DROP TRIGGER IF EXISTS memory_stale_analysis_insert;

DROP TABLE memory_snapshots;
ALTER TABLE memory_snapshots_rebuild RENAME TO memory_snapshots;

CREATE INDEX IF NOT EXISTS memory_snapshots_scope_idx
    ON memory_snapshots(view_kind, scope_key, range_start, range_end, version DESC);
CREATE INDEX IF NOT EXISTS memory_snapshots_status_idx
    ON memory_snapshots(status, updated_at DESC);

-- 0012 的四个 stale 触发器随旧表删除，这里原样重建。
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

-- 写回被级联清空（或原样保留）的反馈记录；条目指向的快照 ID 全部平移未变。
DELETE FROM memory_feedback;
INSERT INTO memory_feedback (id, snapshot_id, item_id, decision, note, created_at, updated_at)
    SELECT id, snapshot_id, item_id, decision, note, created_at, updated_at FROM memory_feedback_preserved;
DROP TABLE memory_feedback_preserved;
