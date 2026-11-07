-- 双路录音（v1.1 块8）：麦克风 + 系统内放声同一时间轴合并。
-- 主会话（麦克风）通过 paired_session_id 指向副会话（系统声）；
-- alignment_offset_ms 记录两路启动的实测偏移（毫秒），对齐误差可见可测。
-- 副会话最终状态用 canceled：0021 建表时的状态注释未含「已并入」枚举，
-- 重建表有 FK 级联风险（0019 教训），不动结构——副会话 canceled + record_id
-- 指向合并后的记录，即「已并入」的对账语义。
ALTER TABLE recording_sessions ADD COLUMN paired_session_id TEXT;
ALTER TABLE recording_sessions ADD COLUMN alignment_offset_ms INTEGER;
