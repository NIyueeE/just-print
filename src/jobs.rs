//! 应用层任务登记表。
//!
//! CUPS 是任务的真实来源，但不会保存文件关联、原始文件名与请求选项。这里
//! 保存这些元数据，供 `/api/jobs`、重打和审计日志使用；容量与 TTL 有上限，
//! 服务重启后清空（任务本身仍在 CUPS 中，可重新对账）。

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 任务元数据。
#[derive(Debug, Clone)]
pub struct JobRecord {
    /// 应用层任务 id：`<printer>-<job-id>`。
    pub id: String,
    /// 关联的上传文件 id。
    pub file_id: String,
    /// 原始文件名。
    pub file_name: String,
    /// 目标打印机名。
    pub printer_id: String,
    /// 提交时间（Unix 毫秒）。
    pub created_at_ms: u64,
    /// 提交时使用的选项。
    pub options: BTreeMap<String, String>,
}

/// 进程内任务登记表。
#[derive(Debug)]
pub struct JobRegistry {
    inner: Mutex<Inner>,
    capacity: usize,
    ttl: Duration,
}

#[derive(Debug)]
struct Inner {
    order: VecDeque<String>,
    map: HashMap<String, JobRecord>,
}

impl JobRegistry {
    /// 创建登记表。
    #[must_use]
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                order: VecDeque::new(),
                map: HashMap::new(),
            }),
            capacity: capacity.max(1),
            ttl,
        }
    }

    /// 登记任务；同一 id 重复登记时覆盖并保持顺序。
    pub fn insert(&self, record: JobRecord) {
        let mut inner = self.lock();
        if inner.map.contains_key(&record.id) {
            inner.map.insert(record.id.clone(), record);
            return;
        }
        inner.order.push_back(record.id.clone());
        inner.map.insert(record.id.clone(), record);
        while inner.map.len() > self.capacity {
            if let Some(oldest) = inner.order.pop_front() {
                inner.map.remove(&oldest);
            }
        }
    }

    /// 查询单个任务。
    #[must_use]
    pub fn get(&self, id: &str) -> Option<JobRecord> {
        self.lock().map.get(id).cloned()
    }

    /// 列出任务（新提交的在前，最多 `limit` 条）。
    #[must_use]
    pub fn list(&self, limit: usize) -> Vec<JobRecord> {
        let inner = self.lock();
        inner
            .order
            .iter()
            .rev()
            .take(limit)
            .filter_map(|id| inner.map.get(id).cloned())
            .collect()
    }

    /// 当前任务数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().map.len()
    }

    /// 移除任务元数据。
    pub fn remove(&self, id: &str) {
        let mut inner = self.lock();
        if inner.map.remove(id).is_some() {
            inner.order.retain(|candidate| candidate != id);
        }
    }

    /// 清理超过 TTL 的任务元数据。
    pub fn prune(&self) {
        let ttl_ms = u64::try_from(self.ttl.as_millis()).unwrap_or(u64::MAX);
        let now = now_ms();
        let mut inner = self.lock();
        let expired: Vec<String> = inner
            .map
            .values()
            .filter(|record| now.saturating_sub(record.created_at_ms) >= ttl_ms)
            .map(|record| record.id.clone())
            .collect();
        for id in expired {
            inner.map.remove(&id);
        }
        let keep: std::collections::HashSet<String> = inner.map.keys().cloned().collect();
        inner.order.retain(|candidate| keep.contains(candidate));
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// 当前 Unix 毫秒时间戳。
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::{JobRecord, JobRegistry, now_ms};

    fn record(id: &str) -> JobRecord {
        JobRecord {
            id: id.to_string(),
            file_id: "file".to_string(),
            file_name: "report.pdf".to_string(),
            printer_id: "PDF".to_string(),
            created_at_ms: now_ms(),
            options: BTreeMap::new(),
        }
    }

    #[test]
    fn inserts_lists_and_removes() {
        let registry = JobRegistry::new(Duration::from_mins(10), 4);
        registry.insert(record("PDF-1"));
        registry.insert(record("PDF-2"));
        assert_eq!(registry.len(), 2);
        let listed = registry.list(10);
        assert_eq!(listed.first().map(|entry| entry.id.as_str()), Some("PDF-2"));
        registry.remove("PDF-2");
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn enforces_capacity() {
        let registry = JobRegistry::new(Duration::from_mins(10), 2);
        registry.insert(record("PDF-1"));
        registry.insert(record("PDF-2"));
        registry.insert(record("PDF-3"));
        assert_eq!(registry.len(), 2);
        assert!(registry.get("PDF-1").is_none());
    }
}
