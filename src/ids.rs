//! 进程内唯一 id 生成。
//!
//! 不使用随机数依赖：由创建时间（Unix 毫秒）、进程内原子计数器和进程 id
//! 组合成 128 位值，输出 32 位十六进制字符串。服务重启后时间戳不同，
//! 仍可保证全局唯一。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 生成一个进程内唯一的 32 位十六进制 id。
#[must_use]
pub fn new_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let millis = u128::from(u64::try_from(millis).unwrap_or(u64::MAX));
    let counter = u128::from(COUNTER.fetch_add(1, Ordering::Relaxed));
    let pid = u128::from(std::process::id());
    let value = (millis << 64) | (counter << 32) | (pid & 0xffff_ffff);
    format!("{value:032x}")
}

#[cfg(test)]
mod tests {
    use super::new_id;

    #[test]
    fn ids_are_unique_and_hex() {
        let first = new_id();
        let second = new_id();
        assert_ne!(first, second);
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|ch| ch.is_ascii_hexdigit()));
    }
}
