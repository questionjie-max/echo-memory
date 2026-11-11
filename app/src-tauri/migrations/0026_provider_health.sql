-- Phase 0：第三方连接健康快照。
-- 记录每个槽位（analysis/transcription）最后一次真实验证的结果与配置指纹。
-- 「已验证」只有在 provider/base_url/model/credential_revision/catalog_version
-- 全部匹配当前配置时才成立；任一变化即「需重新验证」。
-- 全部字段非敏感：不含 Key、Secret 或任何可离线反推凭据的材料。
CREATE TABLE IF NOT EXISTS provider_health (
  target TEXT PRIMARY KEY,
  provider_id TEXT NOT NULL,
  base_url TEXT NOT NULL,
  model TEXT NOT NULL,
  credential_revision INTEGER NOT NULL,
  catalog_version INTEGER NOT NULL,
  stage TEXT NOT NULL,
  status TEXT NOT NULL,
  error_code TEXT,
  verified_at TEXT NOT NULL
);
