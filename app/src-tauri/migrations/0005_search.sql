CREATE VIRTUAL TABLE IF NOT EXISTS record_search USING fts5(
    record_id UNINDEXED,
    project_id UNINDEXED,
    source_id UNINDEXED,
    title,
    body,
    tokenize = 'trigram'
);

CREATE TRIGGER IF NOT EXISTS records_search_insert AFTER INSERT ON records BEGIN
  INSERT INTO record_search (record_id, project_id, source_id, title, body)
  VALUES (new.id, new.project_id, new.id, new.title, '');
END;
CREATE TRIGGER IF NOT EXISTS records_search_update AFTER UPDATE OF title, project_id ON records BEGIN
  UPDATE record_search SET project_id = new.project_id, title = new.title WHERE source_id = new.id;
END;
CREATE TRIGGER IF NOT EXISTS records_search_delete AFTER DELETE ON records BEGIN
  DELETE FROM record_search WHERE record_id = old.id;
END;
CREATE TRIGGER IF NOT EXISTS segments_search_insert AFTER INSERT ON transcript_segments BEGIN
  INSERT INTO record_search (record_id, project_id, source_id, title, body)
  SELECT new.record_id, records.project_id, new.id, records.title, COALESCE(new.edited_text, new.original_text)
  FROM records WHERE records.id = new.record_id;
END;
CREATE TRIGGER IF NOT EXISTS segments_search_update AFTER UPDATE OF original_text, edited_text ON transcript_segments BEGIN
  UPDATE record_search SET body = COALESCE(new.edited_text, new.original_text) WHERE source_id = new.id;
END;
CREATE TRIGGER IF NOT EXISTS segments_search_delete AFTER DELETE ON transcript_segments BEGIN
  DELETE FROM record_search WHERE source_id = old.id;
END;

INSERT INTO record_search (record_id, project_id, source_id, title, body)
SELECT id, project_id, id, title, '' FROM records
WHERE NOT EXISTS (SELECT 1 FROM record_search WHERE source_id = records.id);
INSERT INTO record_search (record_id, project_id, source_id, title, body)
SELECT segments.record_id, records.project_id, segments.id, records.title, COALESCE(segments.edited_text, segments.original_text)
FROM transcript_segments AS segments JOIN records ON records.id = segments.record_id
WHERE NOT EXISTS (SELECT 1 FROM record_search WHERE source_id = segments.id);
