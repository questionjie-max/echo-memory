//! Tauri 托管应用状态。资料库只包含本地文件与 SQLite。

use crate::error::AppResult;
use crate::library::ManagedLibrary;
use crate::types::{MemoryGenerationJob, MemoryGenerationStatus, MemorySnapshot};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// 全局重任务（Whisper 转写、Ollama 分析、知识索引）并发上限。
/// 批量导入时防止同时启动多个本机模型任务拖垮整机。
/// 可用环境变量 `ECHO_MAX_HEAVY_JOBS`（1–8）覆盖。
const DEFAULT_MAX_HEAVY_JOBS: usize = 2;

/// 进程级重任务并发闸门：`acquire()` 阻塞直到拿到名额，守卫 drop 时释放。
/// 转写与分析运行在各自的后台线程里，同一线程内不会嵌套 acquire，因此不会自锁。
#[derive(Clone)]
pub struct JobLimiter {
    inner: Arc<JobLimiterInner>,
}

struct JobLimiterInner {
    max: usize,
    active: Mutex<usize>,
    idle: Condvar,
}

impl JobLimiter {
    pub fn new(max: usize) -> Self {
        Self {
            inner: Arc::new(JobLimiterInner {
                max,
                active: Mutex::new(0),
                idle: Condvar::new(),
            }),
        }
    }

    pub fn acquire(&self) -> JobPermit {
        let mut active = self.inner.active.lock().expect("重任务并发计数锁不可用");
        while *active >= self.inner.max {
            active = self
                .inner
                .idle
                .wait(active)
                .expect("重任务并发计数锁不可用");
        }
        *active += 1;
        JobPermit {
            inner: Arc::clone(&self.inner),
        }
    }

    pub fn max(&self) -> usize {
        self.inner.max
    }
}

/// `acquire` 返回的 RAII 名额：离开作用域自动释放并唤醒下一个等待线程。
pub struct JobPermit {
    inner: Arc<JobLimiterInner>,
}

impl Drop for JobPermit {
    fn drop(&mut self) {
        if let Ok(mut active) = self.inner.active.lock() {
            *active = active.saturating_sub(1);
            self.inner.idle.notify_one();
        }
    }
}

static HEAVY_JOB_LIMITER: OnceLock<JobLimiter> = OnceLock::new();

/// 进程全局重任务闸门。后台线程（含无法访问 `AppState` 的内部路径）统一从这里取名额。
pub fn heavy_job_limiter() -> &'static JobLimiter {
    HEAVY_JOB_LIMITER.get_or_init(|| {
        let max = std::env::var("ECHO_MAX_HEAVY_JOBS")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .map_or(DEFAULT_MAX_HEAVY_JOBS, |value| value.clamp(1, 8));
        JobLimiter::new(max)
    })
}

#[derive(Clone)]
pub struct AppState {
    pub library: ManagedLibrary,
    cancelled_generations: Arc<Mutex<HashSet<String>>>,
    generation_jobs: Arc<Mutex<HashMap<String, MemoryGenerationJob>>>,
    active_knowledge_indexes: Arc<Mutex<HashSet<String>>>,
    memory_update_revision: Arc<AtomicU64>,
}

impl AppState {
    /// 依据资料库根目录初始化迁移和受管理目录。
    pub fn initialize(library_root: PathBuf) -> AppResult<Self> {
        let library = ManagedLibrary::open(library_root)?;
        let repository = library.repository();
        repository.recover_interrupted_memory_snapshots()?;
        repository.recover_interrupted_knowledge_indexes()?;
        repository.recover_interrupted_processing_jobs()?;
        Ok(Self {
            library,
            cancelled_generations: Arc::new(Mutex::new(HashSet::new())),
            generation_jobs: Arc::new(Mutex::new(HashMap::new())),
            active_knowledge_indexes: Arc::new(Mutex::new(HashSet::new())),
            memory_update_revision: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn cancel_generation(&self, generation_id: &str) -> Option<MemoryGenerationJob> {
        if let Ok(mut cancelled) = self.cancelled_generations.lock() {
            cancelled.insert(generation_id.to_owned());
        }
        self.update_generation_job(generation_id, None, MemoryGenerationStatus::Cancelled, None)
    }

    pub fn clear_generation_cancel(&self, generation_id: &str) {
        if let Ok(mut cancelled) = self.cancelled_generations.lock() {
            cancelled.remove(generation_id);
        }
    }

    pub fn is_generation_cancelled(&self, generation_id: &str) -> bool {
        self.cancelled_generations
            .lock()
            .map(|cancelled| cancelled.contains(generation_id))
            .unwrap_or(false)
    }

    pub fn start_generation_job(&self, generation_id: &str) -> AppResult<MemoryGenerationJob> {
        if generation_id.trim().is_empty() {
            return Err(crate::error::AppError::Invalid(
                "生成任务 ID 不能为空".to_owned(),
            ));
        }
        self.clear_generation_cancel(generation_id);
        let mut jobs = self
            .generation_jobs
            .lock()
            .map_err(|_| crate::error::AppError::Invalid("生成任务状态不可用".to_owned()))?;
        if jobs
            .get(generation_id)
            .is_some_and(|job| job.status == MemoryGenerationStatus::Generating)
        {
            return Err(crate::error::AppError::Invalid(
                "该生成任务正在运行".to_owned(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        let job = MemoryGenerationJob {
            generation_id: generation_id.to_owned(),
            snapshot_id: None,
            status: MemoryGenerationStatus::Generating,
            error_message: None,
            started_at: now.clone(),
            updated_at: now,
        };
        jobs.insert(generation_id.to_owned(), job.clone());
        Ok(job)
    }

    pub fn generation_job(&self, generation_id: &str) -> Option<MemoryGenerationJob> {
        self.generation_jobs
            .lock()
            .ok()
            .and_then(|jobs| jobs.get(generation_id).cloned())
    }

    pub fn finish_generation_job(&self, generation_id: &str, snapshot: &MemorySnapshot) {
        self.update_generation_job(
            generation_id,
            Some(snapshot.id.clone()),
            snapshot.status.clone(),
            snapshot.error_message.clone(),
        );
    }

    pub fn fail_generation_job(&self, generation_id: &str, error_message: String) {
        self.update_generation_job(
            generation_id,
            None,
            MemoryGenerationStatus::Failed,
            Some(error_message),
        );
    }

    fn update_generation_job(
        &self,
        generation_id: &str,
        snapshot_id: Option<String>,
        status: MemoryGenerationStatus,
        error_message: Option<String>,
    ) -> Option<MemoryGenerationJob> {
        let mut jobs = self.generation_jobs.lock().ok()?;
        let job = jobs.get_mut(generation_id)?;
        if job.status == MemoryGenerationStatus::Cancelled
            && status != MemoryGenerationStatus::Cancelled
        {
            return Some(job.clone());
        }
        if snapshot_id.is_some() {
            job.snapshot_id = snapshot_id;
        }
        job.status = status;
        job.error_message = error_message;
        job.updated_at = Utc::now().to_rfc3339();
        Some(job.clone())
    }

    pub fn start_knowledge_index(&self, scope_key: &str) -> AppResult<()> {
        let mut active = self
            .active_knowledge_indexes
            .lock()
            .map_err(|_| crate::error::AppError::Invalid("知识索引任务状态不可用".to_owned()))?;
        if !active.insert(scope_key.to_owned()) {
            return Err(crate::error::AppError::Invalid(
                "该范围的知识索引正在建立，请稍候".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn finish_knowledge_index(&self, scope_key: &str) {
        if let Ok(mut active) = self.active_knowledge_indexes.lock() {
            active.remove(scope_key);
        }
    }

    pub fn schedule_memory_update(&self) -> u64 {
        self.memory_update_revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn is_latest_memory_update(&self, revision: u64) -> bool {
        self.memory_update_revision.load(Ordering::SeqCst) == revision
    }
}

/// 默认资料库根目录：优先 `ECHO_LIBRARY_ROOT`，否则用户主目录下
/// `Library/Application Support/回声记忆`（对齐「架构与数据.md」macOS 约定）。
pub fn default_library_root() -> PathBuf {
    if let Ok(custom) = std::env::var("ECHO_LIBRARY_ROOT") {
        return PathBuf::from(custom);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("回声记忆")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MemoryScope, MemorySnapshotResult, MemoryViewKind};
    use std::fs;
    use uuid::Uuid;

    fn test_state() -> (AppState, PathBuf) {
        let root = std::env::temp_dir().join(format!("echo-memory-state-{}", Uuid::new_v4()));
        (AppState::initialize(root.clone()).unwrap(), root)
    }

    fn snapshot(status: MemoryGenerationStatus) -> MemorySnapshot {
        let now = Utc::now().to_rfc3339();
        MemorySnapshot {
            id: "snapshot-1".to_owned(),
            view_kind: MemoryViewKind::Map,
            scope: MemoryScope {
                kind: "all".to_owned(),
                project_id: None,
            },
            range_start: None,
            range_end: None,
            status,
            provider: "openai-compatible".to_owned(),
            model: "test-model".to_owned(),
            source_record_ids: vec!["record-1".to_owned()],
            request_hash: "hash".to_owned(),
            result: MemorySnapshotResult::default(),
            quality_warning: None,
            error_message: None,
            is_stale: false,
            version: 1,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[test]
    fn initialize_recovers_snapshots_left_generating_by_a_previous_process() {
        let (state, root) = test_state();
        let scope = MemoryScope {
            kind: "all".to_owned(),
            project_id: None,
        };
        let snapshot = state
            .library
            .repository()
            .create_memory_snapshot(
                &MemoryViewKind::Map,
                &scope,
                None,
                None,
                "test-model",
                &[],
                "interrupted-hash",
            )
            .unwrap();
        drop(state);

        let recovered_state = AppState::initialize(root.clone()).unwrap();
        let recovered = recovered_state
            .library
            .repository()
            .get_memory_snapshot(&snapshot.id)
            .unwrap();
        assert_eq!(recovered.status, MemoryGenerationStatus::Failed);
        assert!(recovered
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("应用退出而中断")));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn generation_jobs_reach_terminal_states() {
        let (state, root) = test_state();

        let started = state.start_generation_job("success").unwrap();
        assert_eq!(started.status, MemoryGenerationStatus::Generating);
        state.finish_generation_job("success", &snapshot(MemoryGenerationStatus::Completed));
        let completed = state.generation_job("success").unwrap();
        assert_eq!(completed.status, MemoryGenerationStatus::Completed);
        assert_eq!(completed.snapshot_id.as_deref(), Some("snapshot-1"));

        state.start_generation_job("failure").unwrap();
        state.fail_generation_job("failure", "provider error".to_owned());
        let failed = state.generation_job("failure").unwrap();
        assert_eq!(failed.status, MemoryGenerationStatus::Failed);
        assert_eq!(failed.error_message.as_deref(), Some("provider error"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cancelled_job_cannot_be_overwritten_by_background_completion() {
        let (state, root) = test_state();
        state.start_generation_job("cancelled").unwrap();
        let cancelled = state.cancel_generation("cancelled").unwrap();
        assert_eq!(cancelled.status, MemoryGenerationStatus::Cancelled);

        state.finish_generation_job("cancelled", &snapshot(MemoryGenerationStatus::Completed));
        let final_job = state.generation_job("cancelled").unwrap();
        assert_eq!(final_job.status, MemoryGenerationStatus::Cancelled);
        assert!(final_job.snapshot_id.is_none());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_running_job_is_rejected_and_revisions_are_monotonic() {
        let (state, root) = test_state();
        state.start_generation_job("same").unwrap();
        assert!(state.start_generation_job("same").is_err());

        let first = state.schedule_memory_update();
        let second = state.schedule_memory_update();
        assert!(second > first);
        assert!(!state.is_latest_memory_update(first));
        assert!(state.is_latest_memory_update(second));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn job_limiter_releases_permits_and_unblocks_waiters() {
        let limiter = JobLimiter::new(1);
        assert_eq!(limiter.max(), 1);
        let first = limiter.acquire();

        let waiter = std::thread::spawn({
            let limiter = limiter.clone();
            move || {
                let _permit = limiter.acquire();
                "acquired"
            }
        });
        assert!(!waiter.is_finished());
        drop(first);
        assert_eq!(waiter.join().unwrap(), "acquired");
    }

    #[test]
    fn job_limiter_allows_up_to_max_concurrent_permits() {
        let limiter = JobLimiter::new(2);
        let _first = limiter.acquire();
        let second = limiter.acquire();
        let overflow = std::thread::spawn({
            let limiter = limiter.clone();
            move || limiter.acquire()
        });
        assert!(!overflow.is_finished());
        drop(second);
        let _released = overflow.join().unwrap();
    }

    #[test]
    fn global_heavy_job_limiter_has_bounded_capacity() {
        let limiter = heavy_job_limiter();
        assert!((1..=8).contains(&limiter.max()));
    }
}
