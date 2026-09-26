use super::LibraryRepository;
use crate::error::AppResult;
use crate::types::SpeakerSummary;
use rusqlite::params;

impl LibraryRepository {
    pub fn list_record_speakers(&self, record_id: &str) -> AppResult<Vec<SpeakerSummary>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT segments.speaker_label, COUNT(*) FROM transcript_segments AS segments \
             WHERE segments.transcript_version_id = ( \
               SELECT versions.id FROM transcript_versions AS versions \
               WHERE versions.record_id = ?1 ORDER BY versions.created_at DESC LIMIT 1 ) \
             GROUP BY segments.speaker_label ORDER BY COUNT(*) DESC",
        )?;
        let rows = statement
            .query_map(params![record_id], |row| {
                Ok(SpeakerSummary {
                    label: row.get(0)?,
                    segment_count: row.get::<_, i64>(1)? as u32,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn rename_record_speaker(
        &self,
        record_id: &str,
        from_label: &str,
        to_label: &str,
    ) -> AppResult<u32> {
        let to_label = to_label.trim();
        if to_label.is_empty() || to_label.chars().count() > 24 {
            return Err(crate::error::AppError::Invalid("说话人名称无效".to_owned()));
        }
        let changed = self.connect()?.execute(
            "UPDATE transcript_segments SET speaker_label = ?3 \
             WHERE record_id = ?1 AND speaker_label = ?2",
            params![record_id, from_label, to_label],
        )?;
        Ok(changed as u32)
    }
}
