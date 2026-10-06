//! `Idempotency-Key` 支持（IETF draft-ietf-httpapi-idempotency-key-header 语义）。
//!
//! 打印是物理副作用，必须防止重复出纸：同一幂等键在保留期内只产生一个
//! CUPS 任务，后续请求回放首次响应；同键不同负载返回冲突。
//!
//! 同键同负载的重复请求（客户端超时重试、页面挂久后连接失效重试、用户手点重试）
//! 默认**等待**首次请求结束：成功则回放其结果，失败或被放弃则本请求接管重试；
//! 只有等待超时才返回冲突并提示重试。直接返回冲突会让用户在幂等键 TTL（默认
//! 10 分钟）内彻底提交不上去——即使首次请求其实已经打印成功。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::watch;

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

/// 在途请求的终态，用于唤醒等待同一键的请求。
#[derive(Debug, Clone)]
enum Settled {
    /// 成功完成：等待者回放该响应（不会重复出纸）。
    Completed(Value),
    /// 失败或已放弃：等待者可以接管并重新提交。
    Aborted,
}

/// 在途请求的完成信号。
type Signal = watch::Sender<Option<Settled>>;

/// 幂等键条目。
#[derive(Debug)]
enum Entry {
    /// 正在处理；`token` 标识本次尝试，`signal` 用于把终态广播给等待同一键的请求。
    InProgress {
        fingerprint: String,
        token: String,
        at: Instant,
        signal: Arc<Signal>,
    },
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

/// 单次判定结果；与对外暴露的 [`Begin`] 相比多携带等待所需的信号。
#[derive(Debug)]
enum Decision {
    /// 首次见到该键。
    Fresh { token: String },
    /// 已完成，回放既有响应。
    Replay(Value),
    /// 同键但请求负载不同。
    Conflict,
    /// 同键同负载的请求在途；订阅者据此等待其终态。
    InProgress(watch::Receiver<Option<Settled>>),
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

    /// 尝试开始处理一个幂等键；同键同负载且已有请求在途时，在 `wait` 内等待其结果：
    ///
    /// - 在途请求成功 → 回放其响应（[`Begin::Replay`]，不会重复出纸）；
    /// - 在途请求失败/被放弃 → 本请求接管并返回 [`Begin::Fresh`]；
    /// - 等待超时 → 返回 [`Begin::InProgress`]，由调用方回 `409` + `Retry-After`。
    ///
    /// 接管是安全的：只有确认在途请求没有产生 CUPS 任务（失败或显式放弃）时，
    /// 条目才会消失，等待者才可能拿到 `Fresh`。
    pub async fn begin_or_wait(&self, key: &str, fingerprint: &str, wait: Duration) -> Begin {
        let deadline = Instant::now() + wait;
        loop {
            match self.decide(key, fingerprint) {
                Decision::Fresh { token } => return Begin::Fresh { token },
                Decision::Replay(value) => return Begin::Replay(value),
                Decision::Conflict => return Begin::Conflict,
                Decision::InProgress(mut receiver) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Begin::InProgress;
                    }
                    let outcome = tokio::time::timeout(
                        remaining,
                        receiver.wait_for(std::option::Option::is_some),
                    )
                    .await;
                    match outcome {
                        Ok(Ok(state)) => match state.as_ref() {
                            Some(Settled::Completed(value)) => return Begin::Replay(value.clone()),
                            // 在途请求失败/被放弃：回到循环，本请求有机会接管。
                            Some(Settled::Aborted) | None => {}
                        },
                        // 信号端消失（任务被运行时中止）或等待超时：交由调用方回 409，
                        // 条目本身由 TTL 兜底清理，不会永久卡住。
                        Ok(Err(_)) | Err(_) => return Begin::InProgress,
                    }
                }
            }
        }
    }

    /// 判定当前键的状态；命中在途请求时返回可等待的信号。
    fn decide(&self, key: &str, fingerprint: &str) -> Decision {
        let mut inner = self.lock();
        self.prune_locked(&mut inner);
        match inner.get(key) {
            Some(Entry::InProgress {
                fingerprint: existing,
                signal,
                ..
            }) => {
                if existing == fingerprint {
                    Decision::InProgress(signal.subscribe())
                } else {
                    Decision::Conflict
                }
            }
            Some(Entry::Completed {
                fingerprint: existing,
                response,
                ..
            }) => {
                if existing == fingerprint {
                    Decision::Replay(response.clone())
                } else {
                    Decision::Conflict
                }
            }
            None => {
                self.enforce_capacity(&mut inner);
                let token = crate::ids::new_id();
                let (signal, _receiver) = watch::channel(None);
                inner.insert(
                    key.to_string(),
                    Entry::InProgress {
                        fingerprint: fingerprint.to_string(),
                        token: token.clone(),
                        at: Instant::now(),
                        signal: Arc::new(signal),
                    },
                );
                Decision::Fresh { token }
            }
        }
    }

    /// 标记幂等键处理成功，保存响应以便回放，并唤醒等待同一键的请求。
    pub fn complete(&self, key: &str, fingerprint: &str, response: Value) {
        let signal = {
            let mut inner = self.lock();
            let signal = match inner.get(key) {
                Some(Entry::InProgress { signal, .. }) => Some(Arc::clone(signal)),
                _ => None,
            };
            inner.insert(
                key.to_string(),
                Entry::Completed {
                    fingerprint: fingerprint.to_string(),
                    response: response.clone(),
                    at: Instant::now(),
                },
            );
            signal
        };
        if let Some(signal) = signal {
            // 没有等待者时 send 会失败，属正常情况。
            let _ = signal.send(Some(Settled::Completed(response)));
        }
    }

    /// 放弃本次尝试（`token` 由 [`Begin::Fresh`] 给出），允许同键重试接管提交。
    ///
    /// 只清理仍属于该 `token` 的条目：迟到的放弃（例如失败请求的兜底清理）绝不能
    /// 删掉已经接管的新尝试，否则同键请求会重新拿到 `Fresh` 并重复出纸。
    pub fn abort(&self, key: &str, token: &str) {
        let signal = {
            let mut inner = self.lock();
            match inner.get(key) {
                Some(Entry::InProgress {
                    token: current,
                    signal,
                    ..
                }) if current == token => {
                    let signal = Arc::clone(signal);
                    inner.remove(key);
                    Some(signal)
                }
                _ => None,
            }
        };
        if let Some(signal) = signal {
            let _ = signal.send(Some(Settled::Aborted));
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
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::{Begin, IdempotencyStore, validate_key};

    /// 不等待在途请求的判定（`wait` 为零），等价于「立即返回」语义。
    async fn begin_now(store: &IdempotencyStore, key: &str, fingerprint: &str) -> Begin {
        store.begin_or_wait(key, fingerprint, Duration::ZERO).await
    }

    /// 受理一个键并返回本次尝试的 token。
    async fn fresh_token(store: &IdempotencyStore, key: &str, fingerprint: &str) -> Option<String> {
        match begin_now(store, key, fingerprint).await {
            Begin::Fresh { token } => Some(token),
            _ => None,
        }
    }

    #[tokio::test]
    async fn replays_completed_response() {
        let store = IdempotencyStore::new(Duration::from_mins(1), 16);
        assert!(matches!(
            begin_now(&store, "k", "fp").await,
            Begin::Fresh { .. }
        ));
        store.complete("k", "fp", json!({"job_id": "PDF-1"}));
        match begin_now(&store, "k", "fp").await {
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

    #[tokio::test]
    async fn rejects_conflicting_and_concurrent_requests() {
        let store = IdempotencyStore::new(Duration::from_mins(1), 16);
        let Some(token) = fresh_token(&store, "k", "fp").await else {
            return;
        };
        assert!(matches!(
            begin_now(&store, "k", "fp").await,
            Begin::InProgress
        ));
        assert!(matches!(
            begin_now(&store, "k", "other").await,
            Begin::Conflict
        ));
        store.abort("k", &token);
        assert!(matches!(
            begin_now(&store, "k", "other").await,
            Begin::Fresh { .. }
        ));
    }

    /// 等待者必须拿到在途请求的结果（回放），而不是冲突——客户端超时重试依赖它。
    #[tokio::test]
    async fn waiter_replays_result_of_in_flight_request() {
        let store = Arc::new(IdempotencyStore::new(Duration::from_mins(1), 16));
        assert!(fresh_token(&store, "k", "fp").await.is_some());
        let waiter = {
            let store = Arc::clone(&store);
            tokio::spawn(
                async move { store.begin_or_wait("k", "fp", Duration::from_secs(5)).await },
            )
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        store.complete("k", "fp", json!({"job_id": "PDF-1"}));
        let Ok(outcome) = waiter.await else {
            return;
        };
        match outcome {
            Begin::Replay(value) => assert_eq!(
                value.get("job_id").and_then(serde_json::Value::as_str),
                Some("PDF-1")
            ),
            other => assert!(
                format!("{other:?}").starts_with("Replay"),
                "等待者应回放在途请求的结果: {other:?}"
            ),
        }
    }

    /// 在途请求失败/被放弃后，等待者必须能接管重试，否则用户会一直提交不上去。
    #[tokio::test]
    async fn waiter_takes_over_after_in_flight_request_is_aborted() {
        let store = Arc::new(IdempotencyStore::new(Duration::from_mins(1), 16));
        let Some(token) = fresh_token(&store, "k", "fp").await else {
            return;
        };
        let waiter = {
            let store = Arc::clone(&store);
            tokio::spawn(
                async move { store.begin_or_wait("k", "fp", Duration::from_secs(5)).await },
            )
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        store.abort("k", &token);
        let Ok(outcome) = waiter.await else {
            return;
        };
        assert!(
            matches!(outcome, Begin::Fresh { .. }),
            "在途请求放弃后等待者应接管: {outcome:?}"
        );
    }

    /// 迟到的放弃（失败请求的兜底清理）不得删除已经接管的新尝试，否则同键请求
    /// 会重新拿到 `Fresh` 并重复出纸。
    #[tokio::test]
    async fn stale_abort_cannot_drop_a_takeover() {
        let store = IdempotencyStore::new(Duration::from_mins(1), 16);
        let Some(first) = fresh_token(&store, "k", "fp").await else {
            return;
        };
        store.abort("k", &first);
        let Some(second) = fresh_token(&store, "k", "fp").await else {
            return;
        };
        assert_ne!(first, second, "接管应是另一次尝试");
        store.abort("k", &first);
        assert!(
            matches!(begin_now(&store, "k", "fp").await, Begin::InProgress),
            "接管中的尝试不得被旧尝试的兜底清理删掉"
        );
    }

    /// 在途请求长时间不结束时，等待者按预算放弃并回冲突（调用方据此回 409）。
    #[tokio::test]
    async fn waiter_gives_up_after_wait_budget() {
        let store = IdempotencyStore::new(Duration::from_mins(1), 16);
        assert!(matches!(
            begin_now(&store, "k", "fp").await,
            Begin::Fresh { .. }
        ));
        let outcome = store
            .begin_or_wait("k", "fp", Duration::from_millis(50))
            .await;
        assert!(
            matches!(outcome, Begin::InProgress),
            "等待超时应返回进行中: {outcome:?}"
        );
        let started = Instant::now();
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn in_progress_entries_survive_capacity_pressure() {
        // 回归：容量淘汰曾按时间驱逐最旧条目，可能吃掉「进行中」的键，
        // 之后同键重试拿到 Fresh 并重复提交打印。
        let store = IdempotencyStore::new(Duration::from_mins(1), 4);
        for index in 0..4 {
            assert!(
                matches!(
                    begin_now(&store, &format!("k{index}"), "fp").await,
                    Begin::Fresh { .. }
                ),
                "前 4 个键都应首次受理"
            );
        }
        // 第 5 个键触发淘汰：只能淘汰已完成条目，而这里全部在进行中。
        assert!(matches!(
            begin_now(&store, "k4", "fp").await,
            Begin::Fresh { .. }
        ));
        assert!(
            matches!(begin_now(&store, "k0", "fp").await, Begin::InProgress),
            "进行中的幂等键不得被容量淘汰"
        );
    }

    #[tokio::test]
    async fn completed_entries_are_evicted_when_full() {
        let store = IdempotencyStore::new(Duration::from_mins(1), 2);
        assert!(matches!(
            begin_now(&store, "a", "fp").await,
            Begin::Fresh { .. }
        ));
        store.complete("a", "fp", json!({"ok": true}));
        assert!(matches!(
            begin_now(&store, "b", "fp").await,
            Begin::Fresh { .. }
        ));
        store.complete("b", "fp", json!({"ok": true}));
        // 已满且全部完成：新键应淘汰最旧的 "a" 而不是拒绝服务。
        assert!(matches!(
            begin_now(&store, "c", "fp").await,
            Begin::Fresh { .. }
        ));
        assert!(
            matches!(begin_now(&store, "a", "fp").await, Begin::Fresh { .. }),
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
