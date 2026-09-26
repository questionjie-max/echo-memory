-- 记录归档只改变可见范围，不删除或改写转写、分析和知识库归属。
ALTER TABLE records ADD COLUMN archived_at TEXT;
CREATE INDEX IF NOT EXISTS records_archived_at_idx ON records(archived_at);
