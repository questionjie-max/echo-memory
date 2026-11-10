-- 块4：导入内容确认门。
-- 未确认的导入内容（document/chat）对 MCP 不可见，与录音纪要条目的
-- 「暂定→确认→才可被 MCP 读取」同一语义（隐私红线：未确认内容绝不给 Agent）。
-- 只加列 + 回填，不重建父表（外键级联教训：绝不走重建路线）。
ALTER TABLE records ADD COLUMN import_confirmed_at TEXT;
-- 存量导入记录视为已确认：用户既有资料库的 MCP 可见性保持不变，只有新导入走确认门。
UPDATE records SET import_confirmed_at = imported_at WHERE source_type IN ('document', 'chat');
