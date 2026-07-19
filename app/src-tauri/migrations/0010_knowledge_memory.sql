CREATE TABLE IF NOT EXISTS analysis_templates (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    description         TEXT NOT NULL DEFAULT '',
    focus_instructions  TEXT NOT NULL DEFAULT '',
    custom_sections_json TEXT NOT NULL DEFAULT '[]',
    is_builtin          INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

INSERT OR IGNORE INTO analysis_templates
    (id, name, description, focus_instructions, custom_sections_json, is_builtin, created_at, updated_at)
VALUES
    ('builtin-standard', '标准会议', '提取通用会议中的结论与行动。', '优先识别会议目标、讨论脉络、已经确认的决定和明确行动。', '[]', 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
    ('builtin-interview', '客户访谈', '归纳客户目标、痛点与需求证据。', '关注受访者的目标、现状、痛点、替代方案和原话证据。', '[{"key":"needs","title":"核心需求","format":"list","instruction":"提取有原话证据的客户需求"},{"key":"pain_points","title":"痛点","format":"list","instruction":"提取当前工作中的具体困难"}]', 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
    ('builtin-retrospective', '项目复盘', '整理成果、问题、原因与改进。', '区分事实、原因判断和后续改进，不把猜测写成结论。', '[{"key":"wins","title":"有效做法","format":"list","instruction":"哪些做法产生了正向结果"},{"key":"lessons","title":"经验教训","format":"list","instruction":"可以复用的经验和需要避免的问题"}]', 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
    ('builtin-brainstorm', '头脑风暴', '收敛想法、约束和下一步验证。', '保留不同方向，不把尚未选择的想法写成决定。', '[{"key":"ideas","title":"候选想法","format":"list","instruction":"不同的可选方向"},{"key":"constraints","title":"约束条件","format":"list","instruction":"明确出现的资源、时间或技术约束"}]', 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP),
    ('builtin-sales', '销售沟通', '整理客户背景、机会、异议和跟进。', '关注客户原话、购买条件、异议、承诺和下一步。', '[{"key":"opportunities","title":"机会信号","format":"list","instruction":"客户表达的价值、预算或采购信号"},{"key":"objections","title":"异议与风险","format":"list","instruction":"明确提出的顾虑和阻碍"}]', 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);

ALTER TABLE records ADD COLUMN analysis_template_id TEXT DEFAULT 'builtin-standard';
ALTER TABLE analyses ADD COLUMN template_id TEXT;
ALTER TABLE analyses ADD COLUMN template_snapshot_json TEXT NOT NULL DEFAULT '';
ALTER TABLE action_items ADD COLUMN analysis_id TEXT;

UPDATE analyses
SET template_id = 'builtin-standard',
    template_snapshot_json = (SELECT json_object(
        'id', id,
        'name', name,
        'description', description,
        'focusInstructions', focus_instructions,
        'customSections', json(custom_sections_json),
        'isBuiltin', json('true')
    ) FROM analysis_templates WHERE id = 'builtin-standard')
WHERE template_id IS NULL;

UPDATE action_items
SET analysis_id = (
    SELECT analyses.id FROM analyses
    WHERE analyses.record_id = action_items.record_id
    ORDER BY analyses.created_at DESC LIMIT 1
)
WHERE analysis_id IS NULL;

CREATE TABLE IF NOT EXISTS knowledge_chunks (
    id                  TEXT PRIMARY KEY,
    record_id           TEXT NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    project_id          TEXT REFERENCES projects(id) ON DELETE SET NULL,
    transcript_version_id TEXT NOT NULL REFERENCES transcript_versions(id) ON DELETE CASCADE,
    segment_ids_json    TEXT NOT NULL,
    body                TEXT NOT NULL,
    start_ms            INTEGER NOT NULL,
    end_ms              INTEGER NOT NULL,
    speaker_label       TEXT,
    content_hash        TEXT NOT NULL,
    embedding_model     TEXT NOT NULL,
    embedding_dimensions INTEGER NOT NULL,
    embedding_blob      BLOB NOT NULL,
    updated_at          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS knowledge_chunks_scope_idx ON knowledge_chunks(project_id, record_id);
CREATE INDEX IF NOT EXISTS knowledge_chunks_version_idx ON knowledge_chunks(transcript_version_id);

CREATE TABLE IF NOT EXISTS knowledge_index_state (
    scope_key       TEXT PRIMARY KEY,
    status          TEXT NOT NULL DEFAULT 'not_built',
    total_records   INTEGER NOT NULL DEFAULT 0,
    processed_records INTEGER NOT NULL DEFAULT 0,
    chunk_count     INTEGER NOT NULL DEFAULT 0,
    embedding_model TEXT NOT NULL DEFAULT 'qwen3-embedding:0.6b',
    last_error      TEXT,
    updated_at      TEXT NOT NULL
);

INSERT OR IGNORE INTO app_settings (key, value, updated_at) VALUES
    ('transcription_language', 'zh', CURRENT_TIMESTAMP),
    ('whisper_model_path', '', CURRENT_TIMESTAMP),
    ('analysis_model', 'qwen2.5:7b', CURRENT_TIMESTAMP),
    ('embedding_model', 'qwen3-embedding:0.6b', CURRENT_TIMESTAMP);
