//! 最小迁移运行机制（M0 选型演示）。
//! 读取 migrations/ 下的有序 SQL 文件，按版本号幂等应用。
//! 应用启动时的库初始化与领域表在 M1 落地。

use crate::error::AppResult;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

pub mod repository;

/// 迁移文件目录：`<crate>/migrations`。
fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

/// 对给定数据库路径执行所有未应用的迁移，可重复调用（幂等）。
pub fn run_migrations(db_path: &Path) -> AppResult<()> {
    let mut conn = Connection::open(db_path)?;

    // 跟踪表（首次由 0001 创建；此处防御性确保存在）。
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version    INTEGER PRIMARY KEY,
            name       TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );",
    )?;

    let applied: Vec<i64> = conn
        .prepare("SELECT version FROM schema_migrations ORDER BY version ASC")?
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, rusqlite::Error>>()?;
    let max_applied = applied.into_iter().max().unwrap_or(0);

    // 收集并按版本号排序所有迁移文件。
    let mut entries: Vec<(i64, String, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(migrations_dir())? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        if let Some((ver_str, name)) = stem.split_once('_') {
            if let Ok(version) = ver_str.parse::<i64>() {
                entries.push((version, name.to_string(), path));
            }
        }
    }
    entries.sort_by_key(|(v, _, _)| *v);

    for (version, name, path) in entries {
        if version <= max_applied {
            continue;
        }
        let sql = std::fs::read_to_string(&path)?;
        let tx = conn.transaction()?;
        tx.execute_batch(&sql)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![version, name, chrono::Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_db() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut p = std::env::temp_dir().join("echo_memory_test");
        std::fs::create_dir_all(&p).unwrap();
        p.push(format!("migrate_{n}.db"));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn migrations_run_and_are_idempotent() {
        let p = temp_db();
        run_migrations(&p).expect("首次迁移失败");
        run_migrations(&p).expect("重复迁移应幂等");

        let conn = Connection::open(&p).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "0001 迁移应只记录一次");
    }

    #[test]
    fn library_schema_is_applied() {
        let p = temp_db();
        run_migrations(&p).expect("资料库迁移失败");
        let conn = Connection::open(&p).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('projects', 'records', 'processing_jobs')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn knowledge_migration_preserves_existing_library_data() {
        let p = temp_db();
        let mut conn = Connection::open(&p).unwrap();
        let mut entries = std::fs::read_dir(migrations_dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                let stem = path.file_stem()?.to_str()?.to_owned();
                let (version, name) = stem.split_once('_')?;
                let version = version.parse::<i64>().ok()?;
                let name = name.to_owned();
                (version < 10).then(|| (version, name, path))
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|(version, _, _)| *version);
        for (version, name, path) in entries {
            let transaction = conn.transaction().unwrap();
            transaction
                .execute_batch(&std::fs::read_to_string(path).unwrap())
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, '2026-01-01')",
                    rusqlite::params![version, name],
                )
                .unwrap();
            transaction.commit().unwrap();
        }
        conn.execute_batch(
            "INSERT INTO projects (id, name, status, created_at, updated_at) VALUES ('p1', '原知识库', 'active', '2026-01-01', '2026-01-01');
             INSERT INTO records (id, title, project_id, audio_path, audio_hash, audio_duration_ms, imported_at, processing_status, created_at, updated_at) VALUES ('r1', '原录音', 'p1', 'audio/a.m4a', 'hash', 1000, '2026-01-01', 'completed', '2026-01-01', '2026-01-01');
             INSERT INTO transcript_versions (id, record_id, provider, model, status, created_at) VALUES ('v1', 'r1', 'whisper.cpp', 'small', 'completed', '2026-01-01');
             INSERT INTO transcript_segments (id, transcript_version_id, record_id, sequence, start_ms, end_ms, original_text, created_at, updated_at) VALUES ('s1', 'v1', 'r1', 0, 0, 1000, '原逐字稿', '2026-01-01', '2026-01-01');
             INSERT INTO analyses (id, record_id, source_transcript_version_id, status, content_json, provider, model, template_version, created_at) VALUES ('a1', 'r1', 'v1', 'completed', '{\"summary\":\"原分析\"}', 'ollama', 'qwen2.5:7b', 'alpha-v1', '2026-01-01');
             INSERT INTO citations (id, analysis_id, item_path, transcript_segment_id, quote_text, verified) VALUES ('c1', 'a1', 'decisions[0]', 's1', '原逐字稿', 1);
             INSERT INTO action_items (id, record_id, project_id, title, status, source_segment_id) VALUES ('todo1', 'r1', 'p1', '原待办', 'open', 's1');
             INSERT INTO mcp_access_logs (id, tool_name, record_id, project_id, called_at) VALUES ('m1', 'search_records', 'r1', 'p1', '2026-01-01');",
        )
        .unwrap();
        drop(conn);

        run_migrations(&p).unwrap();
        let conn = Connection::open(&p).unwrap();
        for table in [
            "projects",
            "records",
            "transcript_segments",
            "analyses",
            "citations",
            "action_items",
            "mcp_access_logs",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 1, "{table} 数据应保留");
        }
        let snapshot: String = conn
            .query_row(
                "SELECT template_snapshot_json FROM analyses WHERE id = 'a1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(snapshot.contains("标准会议"));
        let analysis_id: String = conn
            .query_row(
                "SELECT analysis_id FROM action_items WHERE id = 'todo1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(analysis_id, "a1");
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
    }
}
