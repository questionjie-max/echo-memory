use crate::error::{AppError, AppResult};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Transaction};
use std::path::{Path, PathBuf};

pub(super) fn move_records(
    transaction: &mut Transaction<'_>,
    record_ids: &[String],
    project_id: Option<&str>,
) -> AppResult<u32> {
    if let Some(project_id) = project_id {
        let project_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)",
                params![project_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false);
        if !project_exists {
            return Err(AppError::Invalid(
                "目标知识库不存在，可能已被删除".to_owned(),
            ));
        }
    }

    let now = Utc::now().to_rfc3339();
    for record_id in record_ids {
        let affected = transaction.execute(
            "UPDATE records SET project_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![record_id, project_id, now],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("record {record_id}")));
        }
        transaction.execute(
            "UPDATE record_search SET project_id = ?2 WHERE record_id = ?1",
            params![record_id, project_id],
        )?;
        transaction.execute(
            "UPDATE action_items SET project_id = ?2 WHERE record_id = ?1",
            params![record_id, project_id],
        )?;
        transaction.execute(
            "UPDATE knowledge_chunks SET project_id = ?2 WHERE record_id = ?1",
            params![record_id, project_id],
        )?;
    }
    Ok(record_ids.len() as u32)
}

pub(super) fn delete_records(
    transaction: &mut Transaction<'_>,
    record_ids: &[String],
) -> AppResult<Vec<(String, PathBuf)>> {
    let mut deleted = Vec::with_capacity(record_ids.len());
    for record_id in record_ids {
        let audio_path: Option<String> = transaction
            .query_row(
                "SELECT audio_path FROM records WHERE id = ?1",
                params![record_id],
                |row| row.get(0),
            )
            .optional()?;
        let audio_path =
            audio_path.ok_or_else(|| AppError::NotFound(format!("record {record_id}")))?;
        transaction.execute("DELETE FROM records WHERE id = ?1", params![record_id])?;
        deleted.push((record_id.clone(), PathBuf::from(audio_path)));
    }
    Ok(deleted)
}

pub(super) fn cleanup_deleted_record_files(
    library_root: &Path,
    deleted_records: &[(String, PathBuf)],
) -> Vec<PathBuf> {
    let mut failed = Vec::new();
    for (record_id, relative_audio_path) in deleted_records {
        let audio = library_root.join(relative_audio_path);
        match std::fs::remove_file(&audio) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => failed.push(audio),
        }

        let raw = library_root.join("raw").join(record_id);
        if raw.exists() && std::fs::remove_dir_all(&raw).is_err() {
            failed.push(raw);
        }
    }
    failed
}
