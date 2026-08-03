//! 内存文件与任务存储：引用计数 + TTL 清理，服务重启后全部失效。

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// 默认保留的最大任务记录数。
pub const DEFAULT_JOB_CAP: usize = 1000;

/// 已上传文件记录。
#[derive(Debug)]
pub struct FileRecord {
    /// 转换后的 PDF 路径。
    pub path: PathBuf,
    /// 创建时间。
    pub created_at: Instant,
    /// 引用计数：排队的任务与正在预览的请求各占一个引用。
    pub references: usize,
}

/// 任务状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    /// 排队中。
    Queued,
    /// 打印中。
    Printing,
    /// 成功。
    Success,
    /// 失败。
    Failed,
}

/// 打印任务记录。
#[derive(Debug, Clone)]
pub struct JobRecord {
    /// 任务 id。
    pub id: String,
    /// 目标打印机身份。
    pub printer_id: String,
    /// 关联的文件 id。
    pub file_id: String,
    /// 当前状态。
    pub status: JobStatus,
    /// 失败原因；成功或排队中为 `None`。
    pub error: Option<String>,
    /// 创建时间（Unix 毫秒）。
    pub created_at_ms: u64,
}

/// 预览文件引用；析构时自动归还引用计数。
#[derive(Debug)]
pub struct FileGuard {
    store: Arc<FileStore>,
    id: String,
    path: PathBuf,
    released: bool,
}

impl FileGuard {
    /// 被引用文件的路径。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FileGuard {
    fn drop(&mut self) {
        if !self.released {
            self.store.release(&self.id);
        }
    }
}

/// 文件存储：id → 记录，带引用计数与 TTL 清理。
#[derive(Debug, Default)]
pub struct FileStore {
    inner: Mutex<HashMap<String, FileRecord>>,
}

impl FileStore {
    /// 登记一个新文件（引用计数为 0）。
    pub fn insert(&self, id: String, path: PathBuf) {
        let record = FileRecord {
            path,
            created_at: Instant::now(),
            references: 0,
        };
        self.lock().insert(id, record);
    }

    /// 增加引用计数并返回文件路径；文件不存在时返回 `None`。
    #[must_use]
    pub fn retain(&self, id: &str) -> Option<PathBuf> {
        let mut inner = self.lock();
        let record = inner.get_mut(id)?;
        record.references += 1;
        Some(record.path.clone())
    }

    /// 增加引用计数并返回 RAII 引用；文件不存在时返回 `None`。
    #[must_use]
    pub fn guard(self: &Arc<Self>, id: &str) -> Option<FileGuard> {
        let mut inner = self.lock();
        let record = inner.get_mut(id)?;
        record.references += 1;
        Some(FileGuard {
            store: Arc::clone(self),
            id: id.to_string(),
            path: record.path.clone(),
            released: false,
        })
    }

    /// 归还一个引用计数。
    pub fn release(&self, id: &str) {
        if let Some(record) = self.lock().get_mut(id) {
            record.references = record.references.saturating_sub(1);
        }
    }

    /// 清理无引用且超过 TTL 的文件，返回删除数量。
    pub fn cleanup(&self, ttl: Duration) -> usize {
        let mut removed = 0;
        let mut inner = self.lock();
        let now = Instant::now();
        inner.retain(|_, record| {
            if record.references == 0 && now.duration_since(record.created_at) >= ttl {
                let _ = std::fs::remove_file(&record.path);
                removed += 1;
                false
            } else {
                true
            }
        });
        removed
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, FileRecord>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// 任务存储：内存队列，超过容量时优先剪除已结束的任务。
#[derive(Debug)]
pub struct JobStore {
    inner: Mutex<HashMap<String, JobRecord>>,
    cap: usize,
}

impl Default for JobStore {
    fn default() -> Self {
        Self::new(DEFAULT_JOB_CAP)
    }
}

impl JobStore {
    /// 创建任务存储，最多保留 `cap` 条记录。
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            cap,
        }
    }

    /// 登记新任务（状态为排队中）。
    pub fn insert(&self, id: String, printer_id: String, file_id: String) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let created_at_ms = u64::try_from(now).unwrap_or(u64::MAX);
        let record = JobRecord {
            id,
            printer_id,
            file_id,
            status: JobStatus::Queued,
            error: None,
            created_at_ms,
        };
        self.lock().insert(record.id.clone(), record);
    }

    /// 查询任务记录。
    #[must_use]
    pub fn get(&self, id: &str) -> Option<JobRecord> {
        self.lock().get(id).cloned()
    }

    /// 更新任务状态。
    pub fn set_status(&self, id: &str, status: JobStatus, error: Option<String>) {
        if let Some(record) = self.lock().get_mut(id) {
            record.status = status;
            record.error = error;
        }
    }

    /// 剪除超过容量上限的已结束任务，返回删除数量。
    pub fn cleanup(&self) -> usize {
        let mut inner = self.lock();
        if inner.len() <= self.cap {
            return 0;
        }
        let mut removable: VecDeque<String> = inner
            .iter()
            .filter(|(_, record)| matches!(record.status, JobStatus::Success | JobStatus::Failed))
            .map(|(id, _)| id.clone())
            .collect();
        let mut pruned = 0;
        while inner.len() > self.cap {
            let Some(id) = removable.pop_front() else {
                break;
            };
            if inner.remove(&id).is_some() {
                pruned += 1;
            }
        }
        pruned
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, JobRecord>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
