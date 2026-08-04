//! 内存文件存储：引用计数 + TTL 清理，服务重启后全部失效。
//!
//! 打印任务本身由 CUPS 排队与跟踪，这里只管理上传转换后的临时 PDF。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// 已上传文件记录。
#[derive(Debug)]
pub struct FileRecord {
    /// 转换后的 PDF 路径。
    pub path: PathBuf,
    /// 创建时间。
    pub created_at: Instant,
    /// 引用计数：正在提交打印的任务与正在预览的请求各占一个引用。
    pub references: usize,
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
