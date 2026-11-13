-- v1.3 包8：记录产物——口播稿改写 / show notes / 社媒文案等派生内容。
-- 只加表（外键级联教训：绝不重建父表）；record 删除时产物随行删除。
-- 产物全部由用户显式触发生成；provider/model 如实记录，Key 等敏感信息绝不入库。
CREATE TABLE IF NOT EXISTS record_artifacts (
  id TEXT PRIMARY KEY,
  record_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  title TEXT NOT NULL,
  content TEXT NOT NULL,
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (record_id) REFERENCES records(id) ON DELETE CASCADE
);
-- 同一条记录同一 kind 唯一（重新生成覆盖，不堆积历史）。
CREATE UNIQUE INDEX IF NOT EXISTS idx_record_artifacts_record_kind
  ON record_artifacts(record_id, kind);
