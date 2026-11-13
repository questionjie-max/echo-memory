-- v1.4 包5：录音打点标记。
-- 录音过程中用户点的「标记一下」，存为记录上的时间戳清单（相对录音开始的毫秒）。
-- 只加列（外键级联教训：绝不重建父表）；JSON 数组 [ms1, ms2, ...] 升序；
-- 未打过标记的存量记录为 NULL，读取时按空数组处理。
ALTER TABLE records ADD COLUMN marks_json TEXT;
