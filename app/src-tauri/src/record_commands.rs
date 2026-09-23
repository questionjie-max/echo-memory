use crate::state::AppState;
use crate::types::DeleteRecordsResult;
use tauri::{AppHandle, State};

/// 批量移动：把多条记录一次性归到某个知识库（传 null 表示移出到「未归档」）。
/// 单条移动已经能在详情页做，这里是工作区勾选多条后的批量版本。
#[tauri::command]
pub fn move_records(
    app: AppHandle,
    state: State<AppState>,
    record_ids: Vec<String>,
    knowledge_base_id: Option<String>,
) -> Result<u32, String> {
    let repository = state.library.repository();
    let moved = repository
        .move_records(&record_ids, knowledge_base_id.as_deref())
        .map_err(|error| error.to_frontend())?;
    if moved > 0 {
        crate::commands::spawn_index_metadata_refresh(app, state.library.root().to_path_buf());
        crate::commands::schedule_memory_update(state.inner().clone());
    }
    Ok(moved)
}

/// 批量删除：删库内行（外键级联带走转写、分析、待办、知识分片、搜索项）、
/// 磁盘上的原始音频和预处理产物。返回实际删除的条数。
#[tauri::command]
pub fn delete_records(
    app: AppHandle,
    state: State<AppState>,
    record_ids: Vec<String>,
) -> Result<DeleteRecordsResult, String> {
    let library_root = state.library.root().to_path_buf();
    let repository = state.library.repository();
    let deleted_records = repository
        .delete_records(&record_ids)
        .map_err(|error| error.to_frontend())?;
    let deleted_count = deleted_records.len() as u32;
    let file_cleanup_failures = repository
        .cleanup_deleted_record_files(&library_root, &deleted_records)
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    if deleted_count > 0 {
        crate::commands::spawn_index_metadata_refresh(app, library_root);
        crate::commands::schedule_memory_update(state.inner().clone());
    }
    Ok(DeleteRecordsResult {
        deleted_count,
        file_cleanup_failures,
    })
}
