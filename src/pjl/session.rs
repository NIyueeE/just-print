//! PJL 设备会话：能力查询、打印 PDF、UEL 复位。
//!
//! 会话字节序为：
//!
//! ```text
//! \x1B%-12345X            # UEL：进入 PJL 模式
//! @PJL SET DUPLEX=ON\r\n # 控制信息
//! @PJL ENTER LANGUAGE=PDF\r\n
//! <PDF 原始字节流>
//! \x1B%-12345X            # UEL：结束会话 / 复位
//! ```
//!
//! 设备读写使用非阻塞 fd + [`tokio::io::unix::AsyncFd`]，避免阻塞式
//! 写入导致超时失效。

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;
use tokio::io::unix::AsyncFd;

use super::capabilities::{Variable, parse_info_variables};

/// PJL UEL 转义序列。
const UEL: &[u8] = b"\x1B%-12345X";

/// 能力查询请求字节流。
const QUERY_REQUEST: &[u8] = b"\x1B%-12345X@PJL\r\n@PJL INFO VARIABLES\r\n\x1B%-12345X";

/// 单次读取缓冲区大小。
const READ_CHUNK_BYTES: usize = 4096;

/// 响应空闲判定：最后一次收到字节后等待这么久即认为响应结束。
const READ_IDLE_TIMEOUT: Duration = Duration::from_millis(300);

/// PJL 会话错误。
#[derive(Debug, Error)]
pub enum PjlError {
    /// 打开设备节点失败。
    #[error("open device {0}: {1}")]
    Open(PathBuf, io::Error),
    /// 设备读写失败。
    #[error("device io failed: {0}")]
    Io(io::Error),
    /// 会话超过时限。
    #[error("device session timed out during {0}")]
    Timeout(&'static str),
    /// PDF 字节流不合法（不以 `%PDF-` 开头）。
    #[error("invalid PDF payload")]
    InvalidPdf,
    /// 能力查询没有解析出任何变量。
    #[error("printer returned no PJL variables")]
    EmptyResponse,
}

/// 向设备发送 `@PJL INFO VARIABLES` 并读取完整响应，返回解析后的变量表。
///
/// 读取在收到 EOF 或连续 [`READ_IDLE_TIMEOUT`] 没有新数据后结束；整体仍受
/// `timeout` 约束。解析结果为空时返回 [`PjlError::EmptyResponse`]，上层会继续
/// 把能力视为未加载并在下个轮询周期重试。
pub async fn query_capabilities(
    path: &Path,
    timeout: Duration,
) -> Result<BTreeMap<String, Variable>, PjlError> {
    let device = open_device(path)?;
    write_all(&device, QUERY_REQUEST, timeout).await?;
    let response = read_response(&device, timeout, READ_IDLE_TIMEOUT).await?;
    let output = String::from_utf8_lossy(&response);
    let variables = parse_info_variables(output.as_ref());
    if variables.is_empty() {
        return Err(PjlError::EmptyResponse);
    }
    Ok(variables)
}

/// 向设备发送一个 PDF + 合法控制信息并完成会话。
///
/// `controls` 的键必须来自能力查询的合法值（由上层校验）；本函数只负责
/// 按 `@PJL SET {key}={value}` 逐行生成并发送。
pub async fn print_pdf(
    path: &Path,
    pdf: &[u8],
    controls: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<(), PjlError> {
    if !pdf.starts_with(b"%PDF-") {
        return Err(PjlError::InvalidPdf);
    }
    let mut payload = Vec::new();
    payload.extend_from_slice(UEL);
    for (key, value) in controls {
        payload.extend_from_slice(format!("@PJL SET {key}={value}\r\n").as_bytes());
    }
    payload.extend_from_slice(b"@PJL ENTER LANGUAGE=PDF\r\n");
    payload.extend_from_slice(pdf);
    payload.extend_from_slice(UEL);

    let device = open_device(path)?;
    write_all(&device, &payload, timeout).await
}

/// 发送 UEL 复位设备状态，用于超时/失败后的下一次会话前。
pub async fn reset(path: &Path, timeout: Duration) -> Result<(), PjlError> {
    let device = open_device(path)?;
    write_all(&device, UEL, timeout).await
}

/// 以非阻塞读写方式打开设备节点。
fn open_device(path: &Path) -> Result<AsyncFd<File>, PjlError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| PjlError::Open(path.to_path_buf(), error))?;
    AsyncFd::new(file).map_err(PjlError::Io)
}

/// 在超时约束下把完整字节序列写入设备，处理部分写入与 `WouldBlock`。
async fn write_all(fd: &AsyncFd<File>, bytes: &[u8], timeout: Duration) -> Result<(), PjlError> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut offset = 0usize;
    while offset < bytes.len() {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .ok_or(PjlError::Timeout("write"))?;
        let mut guard = tokio::time::timeout(remaining, fd.writable())
            .await
            .map_err(|_| PjlError::Timeout("write"))?
            .map_err(PjlError::Io)?;
        let chunk = bytes
            .get(offset..)
            .ok_or_else(|| PjlError::Io(io::Error::other("invalid write offset")))?;
        match guard.try_io(|inner| inner.get_ref().write(chunk)) {
            Ok(Ok(written)) if written > 0 => {
                offset = offset.saturating_add(written);
            }
            Ok(Ok(_)) => {
                return Err(PjlError::Io(io::Error::other(
                    "device write returned 0 bytes",
                )));
            }
            Ok(Err(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
            Ok(Err(error)) => return Err(PjlError::Io(error)),
            Err(_) => {}
        }
    }
    Ok(())
}

/// 读取设备响应，直到 EOF、空闲超时或整体超时。
async fn read_response(
    fd: &AsyncFd<File>,
    timeout: Duration,
    idle: Duration,
) -> Result<Vec<u8>, PjlError> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut response = Vec::new();
    let mut last_read = tokio::time::Instant::now();
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .ok_or(PjlError::Timeout("read"))?;
        let since_last_read = tokio::time::Instant::now().saturating_duration_since(last_read);
        let wait = std::cmp::min(remaining, idle.saturating_sub(since_last_read));
        if wait.is_zero() {
            break;
        }
        let mut guard = match tokio::time::timeout(wait, fd.readable()).await {
            Ok(result) => result.map_err(PjlError::Io)?,
            Err(_) => break,
        };
        let mut chunk = vec![0u8; READ_CHUNK_BYTES];
        match guard.try_io(|inner| inner.get_ref().read(&mut chunk)) {
            Ok(Ok(0)) => break,
            Ok(Ok(read)) => {
                chunk.truncate(read);
                response.extend_from_slice(&chunk);
                last_read = tokio::time::Instant::now();
            }
            Ok(Err(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
            Ok(Err(error)) => return Err(PjlError::Io(error)),
            Err(_) => {}
        }
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs::OpenOptions;
    use std::io::{Read, Write};
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Duration;

    use crate::ids;

    use super::{PjlError, UEL, print_pdf, query_capabilities, reset};

    fn temp_device() -> PathBuf {
        let path = std::env::temp_dir().join(format!("just-print-session-test-{}", ids::new_id()));
        let status = Command::new("mkfifo").arg(&path).status();
        assert!(status.is_ok());
        let Ok(status) = status else {
            return path;
        };
        assert!(status.success());
        path
    }

    fn read_fifo(path: &PathBuf) -> Vec<u8> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path);
        assert!(file.is_ok());
        let Ok(mut file) = file else {
            return Vec::new();
        };
        let mut bytes = Vec::new();
        loop {
            let mut chunk = vec![0u8; 4096];
            match file.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    chunk.truncate(read);
                    bytes.extend_from_slice(&chunk);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => return bytes,
            }
        }
        bytes
    }

    fn open_fifo() -> OpenOptions {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC);
        options
    }

    #[tokio::test]
    async fn reset_writes_uel_only() {
        let path = temp_device();
        let keeper = open_fifo().open(&path);
        assert!(keeper.is_ok());
        let Ok(_keeper) = keeper else {
            return;
        };
        let result = reset(&path, Duration::from_secs(1)).await;
        assert!(result.is_ok(), "reset failed: {result:?}");
        let bytes = read_fifo(&path);
        assert_eq!(bytes, UEL);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn print_writes_full_session_bytes() {
        let path = temp_device();
        let keeper = open_fifo().open(&path);
        assert!(keeper.is_ok());
        let Ok(_keeper) = keeper else {
            return;
        };
        let mut controls = BTreeMap::new();
        controls.insert("DUPLEX".to_string(), "ON".to_string());
        controls.insert("BINDING".to_string(), "LONGEDGE".to_string());
        let pdf = b"%PDF-1.7 fake";

        let result = print_pdf(&path, pdf, &controls, Duration::from_secs(1)).await;
        assert!(result.is_ok(), "print failed: {result:?}");

        let mut expected = Vec::new();
        expected.extend_from_slice(UEL);
        expected.extend_from_slice(b"@PJL SET BINDING=LONGEDGE\r\n");
        expected.extend_from_slice(b"@PJL SET DUPLEX=ON\r\n");
        expected.extend_from_slice(b"@PJL ENTER LANGUAGE=PDF\r\n");
        expected.extend_from_slice(pdf);
        expected.extend_from_slice(UEL);
        let bytes = read_fifo(&path);
        assert_eq!(bytes, expected);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn print_rejects_invalid_pdf() {
        let path = temp_device();
        let controls = BTreeMap::new();
        let result = print_pdf(&path, b"not a pdf", &controls, Duration::from_secs(1)).await;
        assert!(matches!(result, Err(PjlError::InvalidPdf)));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn query_without_response_returns_empty() {
        let path = temp_device();
        let result = query_capabilities(&path, Duration::from_secs(1)).await;
        assert!(
            matches!(result, Err(PjlError::EmptyResponse)),
            "unexpected query result: {result:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn query_parses_echoed_variables() {
        let path = temp_device();
        let keeper = open_fifo().open(&path);
        assert!(keeper.is_ok());
        let Ok(mut keeper) = keeper else {
            return;
        };
        let response = b"@PJL INFO VARIABLES\r\nDUPLEX=OFF [2 ENUMERATED]\r\n\tOFF\r\n\tON\r\n";
        assert!(keeper.write(response).is_ok());

        let result = query_capabilities(&path, Duration::from_secs(1)).await;
        assert!(result.is_ok(), "query failed: {result:?}");
        let Ok(variables) = result else {
            return;
        };
        assert!(variables.contains_key("DUPLEX"));
        let _ = std::fs::remove_file(&path);
    }
}
