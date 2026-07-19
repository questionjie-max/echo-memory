-- Search and analysis use the latest completed transcript while older versions remain stored.
DELETE FROM record_search
WHERE source_id IN (SELECT id FROM transcript_segments);

INSERT INTO record_search (record_id, project_id, source_id, title, body)
SELECT segments.record_id,
       records.project_id,
       segments.id,
       records.title,
       COALESCE(segments.edited_text, segments.original_text)
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
