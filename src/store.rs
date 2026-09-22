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
    /// 原始文件名。
    pub name: String,
    /// 转换后 PDF 的字节数。
    pub size: u64,
    /// 创建时间。
    pub created_at: Instant,
    /// 引用计数：正在提交打印的任务与正在预览的请求各占一个引用。
    pub references: usize,
}

/// 预览/打印文件引用；析构时自动归还引用计数。
#[derive(Debug)]
pub struct FileGuard {
    store: Arc<FileStore>,
    id: String,
    path: PathBuf,
    name: String,
    released: bool,
}

impl FileGuard {
    /// 被引用文件的路径。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 原始文件名。
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
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
    pub fn insert(&self, id: String, name: String, path: PathBuf, size: u64) {
        let record = FileRecord {
            path,
            name,
            size,
            created_at: Instant::now(),
            references: 0,
        };
        self.lock().insert(id, record);
    }

    /// 移除一个无引用的文件记录并返回其路径。
    ///
    /// 文件仍被打印或预览引用时返回 `None`（由调用方决定冲突语义）。
    pub fn remove(&self, id: &str) -> Option<PathBuf> {
        let mut inner = self.lock();
        let references = inner.get(id).map(|record| record.references)?;
        if references > 0 {
            return None;
        }
        inner.remove(id).map(|record| record.path)
    }

    /// 文件是否存在。
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.lock().contains_key(id)
    }

    /// 返回登记的原始文件名。
    #[must_use]
    pub fn name(&self, id: &str) -> Option<String> {
        self.lock().get(id).map(|record| record.name.clone())
    }

    /// 当前登记的文件数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// 当前登记文件的总字节数。
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.lock().values().map(|record| record.size).sum()
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
            name: record.name.clone(),
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
        let mut paths = Vec::new();
        {
            let mut inner = self.lock();
            let now = Instant::now();
            inner.retain(|_, record| {
                if record.references == 0 && now.duration_since(record.created_at) >= ttl {
                    paths.push(record.path.clone());
                    false
                } else {
                    true
                }
            });
        }
        let removed = paths.len();
        for path in paths {
            let _ = std::fs::remove_file(&path);
        }
        removed
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, FileRecord>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::FileStore;

    fn store() -> Arc<FileStore> {
        Arc::new(FileStore::default())
    }

    #[test]
    fn guard_blocks_removal_and_cleanup_until_released() {
        let store = store();
        let path = std::env::temp_dir().join("jp-store-test.pdf");
        store.insert("f1".to_string(), "a.pdf".to_string(), path.clone(), 10);

        let guard = store.guard("f1");
        assert!(guard.is_some(), "存在的文件应能取到引用");
        assert!(store.remove("f1").is_none(), "持有引用的文件不能删除");
        assert_eq!(
            store.cleanup(Duration::ZERO),
            0,
            "持有引用的文件不应被 TTL 清理"
        );
        drop(guard);
        assert_eq!(
            store.remove("f1"),
            Some(path),
            "释放引用后应允许删除并返回路径"
        );
        assert!(!store.contains("f1"));
    }

    #[test]
    fn cleanup_removes_only_expired_unreferenced_files() {
        let store = store();
        store.insert(
            "old".to_string(),
            "old.pdf".to_string(),
            std::env::temp_dir().join("jp-store-old.pdf"),
            1,
        );
        store.insert(
            "new".to_string(),
            "new.pdf".to_string(),
            std::env::temp_dir().join("jp-store-new.pdf"),
            1,
        );
        // 人为把 old 的创建时间推到过去（记录字段仅测试可写）。
        {
            let mut inner = store.lock();
            if let Some(record) = inner.get_mut("old") {
                record.created_at = Instant::now()
                    .checked_sub(Duration::from_secs(120))
                    .unwrap_or(record.created_at);
            }
        }
        assert_eq!(
            store.cleanup(Duration::from_secs(60)),
            1,
            "只应清理过期文件"
        );
        assert!(store.contains("new"), "未过期文件必须保留");
        assert!(!store.contains("old"));
    }

    #[test]
    fn guard_of_missing_file_is_none() {
        let store = store();
        assert!(store.guard("nope").is_none());
        assert!(store.remove("nope").is_none());
        assert_eq!(store.cleanup(Duration::ZERO), 0);
    }

    #[test]
    fn release_is_saturating_and_counts_bytes() {
        let store = store();
        store.insert(
            "f1".to_string(),
            "a.pdf".to_string(),
            std::env::temp_dir().join("jp-store-bytes.pdf"),
            7,
        );
        store.insert(
            "f2".to_string(),
            "b.pdf".to_string(),
            std::env::temp_dir().join("jp-store-bytes2.pdf"),
            5,
        );
        assert_eq!(store.len(), 2);
        assert_eq!(store.total_bytes(), 12);
        // 未持引用就 release 不应下溢成巨大的引用计数。
        store.release("f1");
        assert!(store.remove("f1").is_some(), "无引用文件仍可删除");
        assert_eq!(store.len(), 1);
    }
}
