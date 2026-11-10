-- 术语变体映射（v1.1 块7）。
-- term = 规范写法（与 hotwords.term 语义对应，不强制外键：热词可能被删而变体保留），
-- variant = 逐字稿里识别出的错写/别名。UNIQUE(variant)：一个变体只归一个规范词。
-- 搜索时按变体反查规范词做 OR 扩展：错写也能命中。
CREATE TABLE IF NOT EXISTS term_variants (
    id         TEXT PRIMARY KEY,
    term       TEXT NOT NULL,
    variant    TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS term_variants_term_idx ON term_variants(term);
