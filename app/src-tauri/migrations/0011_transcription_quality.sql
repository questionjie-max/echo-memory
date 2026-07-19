ALTER TABLE transcript_segments ADD COLUMN normalized_text TEXT;
ALTER TABLE transcript_segments ADD COLUMN normalization_version TEXT;

ALTER TABLE transcript_versions ADD COLUMN language TEXT NOT NULL DEFAULT 'zh';
ALTER TABLE transcript_versions ADD COLUMN pipeline_version TEXT NOT NULL DEFAULT 'legacy-v1';
ALTER TABLE transcript_versions ADD COLUMN preprocessing_json TEXT NOT NULL DEFAULT '{}';

ALTER TABLE processing_jobs ADD COLUMN stage TEXT;
ALTER TABLE processing_jobs ADD COLUMN progress_current INTEGER NOT NULL DEFAULT 0;
ALTER TABLE processing_jobs ADD COLUMN progress_total INTEGER NOT NULL DEFAULT 0;

UPDATE analysis_templates
SET focus_instructions = focus_instructions || ' 所有输出使用简体中文。普通陈述不得写成待办；决策必须是已经确认的选择；待办必须有明确的未来行动证据；各栏目不得互相复制。',
    updated_at = CURRENT_TIMESTAMP
WHERE is_builtin = 1 AND focus_instructions NOT LIKE '%普通陈述不得写成待办%';

DROP TRIGGER IF EXISTS segments_search_insert;
DROP TRIGGER IF EXISTS segments_search_update;

CREATE TRIGGER segments_search_insert AFTER INSERT ON transcript_segments BEGIN
  INSERT INTO record_search (record_id, project_id, source_id, title, body)
  SELECT new.record_id, records.project_id, new.id, records.title,
         COALESCE(new.edited_text, new.normalized_text, new.original_text)
  FROM records WHERE records.id = new.record_id;
END;

CREATE TRIGGER segments_search_update AFTER UPDATE OF original_text, normalized_text, edited_text ON transcript_segments BEGIN
  UPDATE record_search
  SET body = COALESCE(new.edited_text, new.normalized_text, new.original_text)
  WHERE source_id = new.id;
END;

DELETE FROM record_search
WHERE source_id IN (SELECT id FROM transcript_segments);

INSERT INTO record_search (record_id, project_id, source_id, title, body)
SELECT segments.record_id,
       records.project_id,
       segments.id,
       records.title,
       COALESCE(segments.edited_text, segments.normalized_text, segments.original_text)
FROM transcript_segments AS segments
JOIN transcript_versions AS versions ON versions.id = segments.transcript_version_id
JOIN records ON records.id = segments.record_id
WHERE versions.id = (
  SELECT latest.id
  FROM transcript_versions AS latest
  WHERE latest.record_id = segments.record_id AND latest.status = 'completed'
  ORDER BY latest.created_at DESC
  LIMIT 1
);
