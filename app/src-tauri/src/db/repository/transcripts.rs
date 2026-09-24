use super::LibraryRepository;
use crate::error::{AppError, AppResult};
use crate::transcript::{build_blocks, normalize_chinese, NORMALIZATION_VERSION};
use crate::types::{TranscriptBlock, TranscriptSegment, TranscriptSegmentInput, TranscriptVersion};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Row};
use std::path::PathBuf;
use uuid::Uuid;

impl LibraryRepository {
    pub fn audio_path_for_record(&self, record_id: &str) -> AppResult<PathBuf> {
        self.connect()?
            .query_row(
                "SELECT audio_path FROM records WHERE id = ?1",
                params![record_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(PathBuf::from)
            .ok_or_else(|| AppError::NotFound(format!("record {record_id}")))
    }

    pub fn save_transcript(
        &self,
        record_id: &str,
        provider: &str,
        model: &str,
        segments: &[TranscriptSegmentInput],
    ) -> AppResult<(TranscriptVersion, Vec<TranscriptSegment>)> {
        self.save_transcript_with_metadata(
            record_id,
            provider,
            model,
            "zh",
            "legacy-v1",
            "{}",
            segments,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_transcript_with_metadata(
        &self,
        record_id: &str,
        provider: &str,
        model: &str,
        language: &str,
        pipeline_version: &str,
        preprocessing_json: &str,
        segments: &[TranscriptSegmentInput],
    ) -> AppResult<(TranscriptVersion, Vec<TranscriptSegment>)> {
        self.get_record(record_id)?;
        if segments.is_empty() {
            return Err(AppError::Invalid("逐字稿不能没有片段".into()));
        }
        let now = Utc::now().to_rfc3339();
        let version = TranscriptVersion {
            id: Uuid::new_v4().to_string(),
            record_id: record_id.to_owned(),
            provider: provider.to_owned(),
            model: model.to_owned(),
            status: "completed".to_owned(),
            language: language.to_owned(),
            pipeline_version: pipeline_version.to_owned(),
            preprocessing_json: preprocessing_json.to_owned(),
            created_at: now.clone(),
        };
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM record_search WHERE source_id IN (SELECT id FROM transcript_segments WHERE record_id = ?1)",
            params![record_id],
        )?;
        Self::invalidate_record_knowledge(&transaction, record_id, true)?;
        transaction.execute(
            "INSERT INTO transcript_versions (id, record_id, provider, model, status, language, pipeline_version, preprocessing_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![version.id, version.record_id, version.provider, version.model, version.status, version.language, version.pipeline_version, version.preprocessing_json, version.created_at],
        )?;
        let mut saved = Vec::with_capacity(segments.len());
        for (sequence, segment) in segments.iter().enumerate() {
            if segment.end_ms < segment.start_ms || segment.original_text.trim().is_empty() {
                return Err(AppError::Invalid("逐字稿片段无效".into()));
            }
            let saved_segment = TranscriptSegment {
                id: Uuid::new_v4().to_string(),
                record_id: record_id.to_owned(),
                sequence: sequence as i64,
                speaker_label: segment.speaker_label.clone(),
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                original_text: segment.original_text.trim().to_owned(),
                normalized_text: Some(normalize_chinese(segment.original_text.trim())),
                normalization_version: Some(NORMALIZATION_VERSION.to_owned()),
                edited_text: None,
            };
            transaction.execute(
                "INSERT INTO transcript_segments (id, transcript_version_id, record_id, sequence, speaker_label, start_ms, end_ms, original_text, normalized_text, normalization_version, edited_text, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, ?11)",
                params![saved_segment.id, version.id, saved_segment.record_id, saved_segment.sequence, saved_segment.speaker_label, saved_segment.start_ms, saved_segment.end_ms, saved_segment.original_text, saved_segment.normalized_text, saved_segment.normalization_version, now],
            )?;
            saved.push(saved_segment);
        }
        transaction.commit()?;
        Ok((version, saved))
    }

    pub fn list_transcript_segments(&self, record_id: &str) -> AppResult<Vec<TranscriptSegment>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT segments.id, segments.record_id, segments.sequence, segments.speaker_label, segments.start_ms, segments.end_ms, segments.original_text, segments.normalized_text, segments.normalization_version, segments.edited_text \
             FROM transcript_segments AS segments \
             WHERE segments.transcript_version_id = ( \
               SELECT versions.id FROM transcript_versions AS versions \
               WHERE versions.record_id = ?1 AND versions.status = 'completed' \
               ORDER BY versions.created_at DESC LIMIT 1 \
             ) ORDER BY segments.sequence ASC",
        )?;
        let segments = statement
            .query_map(params![record_id], Self::map_segment)?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(segments)
    }

    pub fn list_transcript_blocks(&self, record_id: &str) -> AppResult<Vec<TranscriptBlock>> {
        Ok(build_blocks(&self.list_transcript_segments(record_id)?))
    }

    pub fn update_segment_text(
        &self,
        id: &str,
        edited_text: Option<&str>,
    ) -> AppResult<TranscriptSegment> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let record_id: Option<String> = transaction
            .query_row(
                "SELECT record_id FROM transcript_segments WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        let record_id = record_id.ok_or_else(|| AppError::NotFound(format!("segment {id}")))?;
        let affected = transaction.execute(
            "UPDATE transcript_segments SET edited_text = ?2, updated_at = ?3 WHERE id = ?1",
            params![
                id,
                edited_text.map(str::trim).filter(|text| !text.is_empty()),
                Utc::now().to_rfc3339()
            ],
        )?;
        debug_assert_eq!(affected, 1);
        Self::invalidate_record_knowledge(&transaction, &record_id, true)?;
        transaction.commit()?;
        self.connect()?.query_row(
            "SELECT id, record_id, sequence, speaker_label, start_ms, end_ms, original_text, normalized_text, normalization_version, edited_text FROM transcript_segments WHERE id = ?1",
            params![id], Self::map_segment,
        ).map_err(Into::into)
    }

    pub(super) fn backfill_normalized_transcripts(&self) -> AppResult<()> {
        let mut connection = self.connect()?;
        let rows = {
            let mut statement = connection.prepare(
                "SELECT id, original_text FROM transcript_segments WHERE normalized_text IS NULL OR normalization_version IS NULL",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, rusqlite::Error>>()?;
            rows
        };
        if rows.is_empty() {
            return Ok(());
        }
        let transaction = connection.transaction()?;
        for (id, original) in rows {
            transaction.execute(
                "UPDATE transcript_segments SET normalized_text = ?2, normalization_version = ?3 WHERE id = ?1",
                params![id, normalize_chinese(&original), NORMALIZATION_VERSION],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// 将 LLM 校对后的文本写入 normalized 层（edited 层保留给用户手改，原始层不动）。
    pub fn set_segment_normalized_texts(
        &self,
        record_id: &str,
        corrections: &[(String, String)],
    ) -> AppResult<u32> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let mut changed = 0_u32;
        for (segment_id, text) in corrections {
            let updated = transaction.execute(
                "UPDATE transcript_segments SET normalized_text = ?2, normalization_version = 'llm-corrected-v1', updated_at = ?4 WHERE id = ?1 AND record_id = ?3",
                params![segment_id, text, record_id, Utc::now().to_rfc3339()],
            )?;
            changed += updated as u32;
        }
        // 逐字稿文本变了：分析结论引用的原文可能对不上，知识索引也要重建。
        if changed > 0 {
            Self::invalidate_record_knowledge(&transaction, record_id, true)?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    fn map_segment(row: &Row<'_>) -> rusqlite::Result<TranscriptSegment> {
        Ok(TranscriptSegment {
            id: row.get(0)?,
            record_id: row.get(1)?,
            sequence: row.get(2)?,
            speaker_label: row.get(3)?,
            start_ms: row.get(4)?,
            end_ms: row.get(5)?,
            original_text: row.get(6)?,
            normalized_text: row.get(7)?,
            normalization_version: row.get(8)?,
            edited_text: row.get(9)?,
        })
    }
}
