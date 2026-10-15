-- 单条记录的分析「补充信息」：随每一次分析一起落库，重分析时回显、审计时可追溯。
-- 只做 ADD COLUMN，不重建表——重建父表会按 FK 级联清空子表（0019 事故教训）。
ALTER TABLE analyses ADD COLUMN analysis_instruction TEXT NOT NULL DEFAULT '';
