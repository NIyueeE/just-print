//! `Idempotency-Key` 支持（IETF draft-ietf-httpapi-idempotency-key-header 语义）。
//!
//! 打印是物理副作用，必须防止重复出纸：同一幂等键在保留期内只产生一个
//! CUPS 任务，后续请求回放首次响应；同键不同负载返回冲突；并发重复请求在
//! 首次未完成前也返回冲突并提示重试。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

/// 幂等键开始处理的结果。
#[derive(Debug)]
pub enum Begin {
    /// 首次见到该键，可以执行；`token` 用于失败后的对账。
    Fresh {
        /// 本次尝试的唯一标记（写入 `job-name` 以便对账）。
        token: String,
    },
    /// 已完成，回放既有响应。
    Replay(Value),
    /// 同键但请求负载不同。
    Conflict,
    /// 同键请求正在处理中。
    InProgress,
}

/// 幂等键条目。
#[derive(Debug)]
enum Entry {
    /// 正在处理。
    InProgress { fingerprint: String, at: Instant },
    /// 已成功完成。
    Completed {
        fingerprint: String,
        response: Value,
        at: Instant,
    },
}

impl Entry {
    fn at(&self) -> Instant {
        match self {
            Self::InProgress { at, .. } | Self::Completed { at, .. } => *at,
        }
    }
}

/// 幂等键存储（内存，带 TTL 与容量上限）。
#[derive(Debug)]
pub struct IdempotencyStore {
    inner: Mutex<HashMap<String, Entry>>,
    ttl: Duration,
    capacity: usize,
}

impl IdempotencyStore {
    /// 创建幂等键存储。
    #[must_use]
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl,
            capacity: capacity.max(1),
        }
    }

    /// 尝试开始处理一个幂等键。
    pub fn begin(&self, key: &str, fingerprint: &str) -> Begin {
        let mut inner = self.lock();
        self.prune_locked(&mut inner);
        match inner.get(key) {
            Some(Entry::InProgress {
                fingerprint: existing,
                ..
            }) => {
                if existing == fingerprint {
                    Begin::InProgress
                } else {
                    Begin::Conflict
                }
            }
            Some(Entry::Completed {
                fingerprint: existing,
                response,
                ..
            }) => {
                if existing == fingerprint {
                    Begin::Replay(response.clone())
                } else {
                    Begin::Conflict
                }
            }
            None => {
                self.enforce_capacity(&mut inner);
                let token = crate::ids::new_id();
                inner.insert(
                    key.to_string(),
                    Entry::InProgress {
                        fingerprint: fingerprint.to_string(),
                        at: Instant::now(),
                    },
                );
                Begin::Fresh { token }
            }
        }
    }

    /// 标记幂等键处理成功，保存响应以便回放。
    pub fn complete(&self, key: &str, fingerprint: &str, response: Value) {
        let mut inner = self.lock();
        inner.insert(
            key.to_string(),
            Entry::Completed {
                fingerprint: fingerprint.to_string(),
                response,
                at: Instant::now(),
            },
        );
    }

    /// 放弃幂等键，允许客户端重试。
    pub fn abort(&self, key: &str) {
        let mut inner = self.lock();
        if matches!(inner.get(key), Some(Entry::InProgress { .. })) {
            inner.remove(key);
        }
    }

    /// 清理过期条目。
    pub fn prune(&self) {
        let mut inner = self.lock();
        self.prune_locked(&mut inner);
    }

    /// 当前条目数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    fn prune_locked(&self, inner: &mut HashMap<String, Entry>) {
        let ttl = self.ttl;
        inner.retain(|_, entry| entry.at().elapsed() < ttl);
    }

    fn enforce_capacity(&self, inner: &mut HashMap<String, Entry>) {
        while inner.len() >= self.capacity {
            // 只淘汰已完成条目：进行中的条目对应「请求已受理、响应未返回」的
            // 真实打印。它一旦被淘汰，同键重试会拿到 Fresh 并再次提交，
            // 幂等保护失效、造成重复出纸。全部条目都在进行中时停止淘汰，
            // 让容量被并发量短暂突破，由 TTL 兜底回收。
            let oldest = inner
                .iter()
                .filter(|(_, entry)| matches!(entry, Entry::Completed { .. }))
                .min_by_key(|(_, entry)| entry.at())
                .map(|(key, _)| key.clone());
            match oldest {
                Some(key) => {
                    inner.remove(&key);
                }
                None => break,
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// 校验客户端提供的 `Idempotency-Key`。
///
/// # Errors
///
/// 键为空、超过 255 字节或包含控制字符时返回 `Err`。
pub fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("Idempotency-Key 不能为空".to_string());
    }
    if key.len() > 255 {
        return Err("Idempotency-Key 超过 255 字节".to_string());
    }
    if key.chars().any(char::is_control) {
        return Err("Idempotency-Key 含控制字符".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::{Begin, IdempotencyStore, validate_key};

    #[test]
    fn replays_completed_response() {
        let store = IdempotencyStore::new(Duration::from_mins(1), 16);
        assert!(matches!(store.begin("k", "fp"), Begin::Fresh { .. }));
        store.complete("k", "fp", json!({"job_id": "PDF-1"}));
        match store.begin("k", "fp") {
            Begin::Replay(value) => assert_eq!(
                value.get("job_id").and_then(serde_json::Value::as_str),
                Some("PDF-1")
            ),
            other => assert!(
                format!("{other:?}").starts_with("Replay"),
                "expected replay response"
            ),
        }
    }

    #[test]
    fn rejects_conflicting_and_concurrent_requests() {
        let store = IdempotencyStore::new(Duration::from_mins(1), 16);
        assert!(matches!(store.begin("k", "fp"), Begin::Fresh { .. }));
        assert!(matches!(store.begin("k", "fp"), Begin::InProgress));
        assert!(matches!(store.begin("k", "other"), Begin::Conflict));
        store.abort("k");
        assert!(matches!(store.begin("k", "other"), Begin::Fresh { .. }));
    }

    #[test]
    fn in_progress_entries_survive_capacity_pressure() {
        // 回归：容量淘汰曾按时间驱逐最旧条目，可能吃掉「进行中」的键，
        // 之后同键重试拿到 Fresh 并重复提交打印。
        let store = IdempotencyStore::new(Duration::from_mins(1), 4);
        for index in 0..4 {
            assert!(
                matches!(store.begin(&format!("k{index}"), "fp"), Begin::Fresh { .. }),
                "前 4 个键都应首次受理"
            );
        }
        // 第 5 个键触发淘汰：只能淘汰已完成条目，而这里全部在进行中。
        assert!(matches!(store.begin("k4", "fp"), Begin::Fresh { .. }));
        assert!(
            matches!(store.begin("k0", "fp"), Begin::InProgress),
            "进行中的幂等键不得被容量淘汰"
        );
    }

    #[test]
    fn completed_entries_are_evicted_when_full() {
        let store = IdempotencyStore::new(Duration::from_mins(1), 2);
        assert!(matches!(store.begin("a", "fp"), Begin::Fresh { .. }));
        store.complete("a", "fp", json!({"ok": true}));
        assert!(matches!(store.begin("b", "fp"), Begin::Fresh { .. }));
        store.complete("b", "fp", json!({"ok": true}));
        // 已满且全部完成：新键应淘汰最旧的 "a" 而不是拒绝服务。
        assert!(matches!(store.begin("c", "fp"), Begin::Fresh { .. }));
        assert!(
            matches!(store.begin("a", "fp"), Begin::Fresh { .. }),
            "已完成的旧键应被淘汰"
        );
    }

    #[test]
    fn validates_keys() {
        assert!(validate_key("abc-123").is_ok());
        assert!(validate_key("").is_err());
        assert!(validate_key(&"x".repeat(256)).is_err());
        assert!(validate_key("bad\nkey").is_err());
    }
}
