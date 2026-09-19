//! 进程内唯一 id 生成。
//!
//! 使用操作系统 CSPRNG（`uuid` v4）生成 128 位随机值，输出 32 位十六进制
//! 字符串。文件 id 就是访问句柄，随机不可预测比时间戳更安全。

use uuid::Uuid;

/// 生成一个随机的 32 位十六进制 id。
#[must_use]
pub fn new_id() -> String {
    Uuid::new_v4().simple().to_string()
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
