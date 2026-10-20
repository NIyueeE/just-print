//! 一次性上传票据。
//!
//! 终端优先的关键：模型不能把文件字节写进对话，所以它只签发一张票据，由终端里的
//! curl 把字节直接推到 /uploads/{ticket}。票据一次性、短时效、只能创建一个文件，
//! 泄露也换不到别的能力。

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// 票据保留时长。
pub const TICKET_TTL: Duration = Duration::from_mins(5);
/// 同时有效的票据上限。
pub const TICKET_CAPACITY: usize = 64;

#[derive(Debug)]
struct Ticket {
    name: String,
    created_at: Instant,
}

/// 票据存储：id 到文件名的映射，带 TTL 与容量上限。
#[derive(Debug)]
pub struct TicketStore {
    inner: Mutex<HashMap<String, Ticket>>,
    ttl: Duration,
    capacity: usize,
}

impl TicketStore {
    /// 构造票据存储。
    #[must_use]
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl,
            capacity,
        }
    }

    /// 签发一张票据；容量已满时返回 None，由调用方转成 503。
    #[must_use]
    pub fn issue(&self, name: String) -> Option<String> {
        let mut inner = self.lock();
        let now = Instant::now();
        inner.retain(|_, ticket| now.duration_since(ticket.created_at) < self.ttl);
        if inner.len() >= self.capacity {
            return None;
        }
        let id = crate::ids::new_id();
        inner.insert(
            id.clone(),
            Ticket {
                name,
                created_at: now,
            },
        );
        Some(id)
    }

    /// 兑换票据：有效则返回绑定的文件名，并立即失效（一次性）。
    #[must_use]
    pub fn redeem(&self, id: &str) -> Option<String> {
        let mut inner = self.lock();
        let ticket = inner.remove(id)?;
        if Instant::now().duration_since(ticket.created_at) >= self.ttl {
            return None;
        }
        Some(ticket.name)
    }

    /// 当前登记的有效票据数（仅测试使用）。
    #[cfg(test)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Ticket>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::TicketStore;

    #[test]
    fn ticket_can_be_redeemed_only_once() {
        let store = TicketStore::new(Duration::from_mins(5), 4);
        let Some(ticket) = store.issue("report.pdf".to_string()) else {
            return;
        };
        assert_eq!(store.redeem(&ticket).as_deref(), Some("report.pdf"));
        assert_eq!(store.redeem(&ticket), None, "票据必须一次性");
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn expired_ticket_is_rejected() {
        let store = TicketStore::new(Duration::ZERO, 4);
        let Some(ticket) = store.issue("a.pdf".to_string()) else {
            return;
        };
        assert_eq!(store.redeem(&ticket), None, "过期票据不应被接受");
    }

    #[test]
    fn capacity_is_enforced_and_sweeps_expired() {
        let store = TicketStore::new(Duration::from_mins(5), 2);
        assert!(store.issue("a.pdf".to_string()).is_some());
        assert!(store.issue("b.pdf".to_string()).is_some());
        assert_eq!(store.issue("c.pdf".to_string()), None, "容量满时应拒绝");
        let expired = TicketStore::new(Duration::ZERO, 1);
        assert!(expired.issue("a.pdf".to_string()).is_some());
        assert!(
            expired.issue("b.pdf".to_string()).is_some(),
            "签发时应顺带清理过期票据"
        );
    }
}
