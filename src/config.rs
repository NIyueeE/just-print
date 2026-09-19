//! 环境变量配置与 fail-closed 校验。

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;

/// 默认监听地址。
pub const DEFAULT_ADDR: &str = "0.0.0.0:8080";
/// 默认静态前端目录（镜像内置路径）。
pub const DEFAULT_WEB_DIR: &str = "/usr/share/just-print/web";
/// 默认 CUPS 服务地址。
pub const DEFAULT_CUPS_URI: &str = "http://127.0.0.1:631";
/// 上传大小上限：64 MiB。
pub const DEFAULT_MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;
/// `LibreOffice` 转换超时。
pub const DEFAULT_CONVERSION_TIMEOUT: Duration = Duration::from_mins(2);
/// `LibreOffice` 并发转换上限（`CPU` 密集操作）。
pub const DEFAULT_CONVERSION_SLOTS: usize = 2;
/// 临时文件清理周期。
pub const DEFAULT_CLEANUP_INTERVAL: Duration = Duration::from_mins(1);
/// 无引用临时文件的保留时长。
pub const DEFAULT_TEMP_TTL: Duration = Duration::from_mins(30);
/// 单次 IPP 请求超时。
pub const DEFAULT_IPP_TIMEOUT: Duration = Duration::from_secs(15);
/// 打印机快照缓存时长。
pub const DEFAULT_PRINTER_CACHE_TTL: Duration = Duration::from_secs(10);
/// 任务状态缓存时长。
pub const DEFAULT_JOB_CACHE_TTL: Duration = Duration::from_secs(2);
/// 幂等键保留时长。
pub const DEFAULT_IDEMPOTENCY_TTL: Duration = Duration::from_mins(10);
/// 并发上传上限。
pub const DEFAULT_UPLOAD_SLOTS: usize = 4;
/// 单个 HTTP 请求处理超时。
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_mins(5);

/// 应用配置。
#[derive(Debug, Clone)]
pub struct Config {
    /// HTTP 监听地址。
    pub addr: SocketAddr,
    /// 静态前端目录。
    pub web_dir: PathBuf,
    /// `Bearer` 准入令牌（已校验非空）。
    pub token: Arc<str>,
    /// CUPS 服务地址（`host:port`）。
    pub cups_server: String,
    /// CUPS 服务 scheme（`ipp` / `ipps`）。
    pub cups_scheme: String,
    /// 上传大小上限（字节）。
    pub max_upload_bytes: usize,
    /// `LibreOffice` 转换超时。
    pub conversion_timeout: Duration,
    /// `LibreOffice` 并发转换上限。
    pub conversion_slots: usize,
    /// 临时文件清理周期。
    pub cleanup_interval: Duration,
    /// 无引用临时文件保留时长。
    pub temp_ttl: Duration,
    /// 单次 IPP 请求超时。
    pub ipp_timeout: Duration,
    /// 打印机快照缓存时长。
    pub printer_cache_ttl: Duration,
    /// 任务状态缓存时长。
    pub job_cache_ttl: Duration,
    /// 幂等键保留时长。
    pub idempotency_ttl: Duration,
    /// 并发上传上限。
    pub upload_slots: usize,
    /// 单个 HTTP 请求处理超时。
    pub request_timeout: Duration,
}

impl Config {
    /// 从环境变量加载配置；令牌缺失或为空时拒绝启动（fail-closed）。
    ///
    /// # Errors
    ///
    /// 令牌缺失/为空、地址或 CUPS URI 不合法、数值型环境变量无法解析时返回
    /// [`ConfigError`]。
    pub fn from_env() -> Result<Self, ConfigError> {
        let token = env::var("JUST_PRINT_TOKEN").map_err(|_| ConfigError::MissingToken)?;
        let token = token.trim();
        if token.is_empty() {
            return Err(ConfigError::MissingToken);
        }
        let addr = env::var("JUST_PRINT_ADDR")
            .unwrap_or_else(|_| DEFAULT_ADDR.to_string())
            .parse::<SocketAddr>()
            .map_err(|error| ConfigError::InvalidAddr(error.to_string()))?;
        let web_dir = PathBuf::from(
            env::var("JUST_PRINT_WEB_DIR").unwrap_or_else(|_| DEFAULT_WEB_DIR.to_string()),
        );
        let cups_uri =
            env::var("JUST_PRINT_CUPS_URI").unwrap_or_else(|_| DEFAULT_CUPS_URI.to_string());
        let (cups_server, cups_scheme) =
            parse_cups_uri(&cups_uri).map_err(ConfigError::InvalidCupsUri)?;
        Ok(Self {
            addr,
            web_dir,
            token: Arc::from(token),
            cups_server,
            cups_scheme,
            max_upload_bytes: env_usize("JUST_PRINT_MAX_UPLOAD_BYTES", DEFAULT_MAX_UPLOAD_BYTES)?,
            conversion_timeout: env_duration(
                "JUST_PRINT_CONVERSION_TIMEOUT_SECS",
                DEFAULT_CONVERSION_TIMEOUT,
            )?,
            conversion_slots: env_usize("JUST_PRINT_CONVERSION_SLOTS", DEFAULT_CONVERSION_SLOTS)?,
            cleanup_interval: env_duration(
                "JUST_PRINT_CLEANUP_INTERVAL_SECS",
                DEFAULT_CLEANUP_INTERVAL,
            )?,
            temp_ttl: env_duration("JUST_PRINT_TEMP_TTL_SECS", DEFAULT_TEMP_TTL)?,
            ipp_timeout: env_duration("JUST_PRINT_IPP_TIMEOUT_SECS", DEFAULT_IPP_TIMEOUT)?,
            printer_cache_ttl: env_duration(
                "JUST_PRINT_PRINTER_CACHE_SECS",
                DEFAULT_PRINTER_CACHE_TTL,
            )?,
            job_cache_ttl: env_duration("JUST_PRINT_JOB_CACHE_SECS", DEFAULT_JOB_CACHE_TTL)?,
            idempotency_ttl: env_duration(
                "JUST_PRINT_IDEMPOTENCY_TTL_SECS",
                DEFAULT_IDEMPOTENCY_TTL,
            )?,
            upload_slots: env_usize("JUST_PRINT_UPLOAD_SLOTS", DEFAULT_UPLOAD_SLOTS)?,
            request_timeout: env_duration(
                "JUST_PRINT_REQUEST_TIMEOUT_SECS",
                DEFAULT_REQUEST_TIMEOUT,
            )?,
        })
    }
}

fn env_usize(key: &str, default: usize) -> Result<usize, ConfigError> {
    let Ok(raw) = env::var(key) else {
        return Ok(default);
    };
    let value = raw
        .trim()
        .parse::<usize>()
        .map_err(|_| ConfigError::InvalidNumber {
            key: key.to_string(),
            value: raw.clone(),
        })?;
    if value == 0 {
        return Err(ConfigError::InvalidNumber {
            key: key.to_string(),
            value: raw,
        });
    }
    Ok(value)
}

fn env_duration(key: &str, default: Duration) -> Result<Duration, ConfigError> {
    let Ok(raw) = env::var(key) else {
        return Ok(default);
    };
    let value = raw
        .trim()
        .parse::<u64>()
        .map_err(|_| ConfigError::InvalidNumber {
            key: key.to_string(),
            value: raw.clone(),
        })?;
    if value == 0 {
        return Err(ConfigError::InvalidNumber {
            key: key.to_string(),
            value: raw,
        });
    }
    Ok(Duration::from_secs(value))
}

fn parse_cups_uri(uri: &str) -> Result<(String, String), String> {
    let (rest, default_port, scheme) = if let Some(rest) = uri.strip_prefix("http://") {
        (rest, "631", "ipp")
    } else if let Some(rest) = uri.strip_prefix("https://") {
        (rest, "631", "ipps")
    } else {
        return Err("仅支持 http:// 或 https://".to_string());
    };
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) => (host.trim(), port.trim()),
        None => (rest.trim(), default_port),
    };
    if host.is_empty() {
        return Err("缺少主机名".to_string());
    }
    if !port.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("端口不是数字".to_string());
    }
    Ok((format!("{host}:{port}"), scheme.to_string()))
}

/// 配置加载错误。
#[derive(Debug, Error)]
pub enum ConfigError {
    /// 令牌未配置或为空。
    #[error("JUST_PRINT_TOKEN 未配置或为空，服务按 fail-closed 拒绝启动")]
    MissingToken,
    /// 监听地址不合法。
    #[error("JUST_PRINT_ADDR 不是合法的 SocketAddr: {0}")]
    InvalidAddr(String),
    /// CUPS 地址不合法。
    #[error("JUST_PRINT_CUPS_URI 不合法: {0}")]
    InvalidCupsUri(String),
    /// 数值型环境变量不合法。
    #[error("{key} 不是合法的正整数: {value}")]
    InvalidNumber {
        /// 环境变量名。
        key: String,
        /// 原始取值。
        value: String,
    },
}

#[cfg(test)]
mod tests {
    use super::parse_cups_uri;

    #[test]
    fn parses_cups_uri_with_and_without_port() {
        let result = parse_cups_uri("http://127.0.0.1:631");
        assert!(result.is_ok());
        let Ok((server, scheme)) = result else {
            return;
        };
        assert_eq!(server, "127.0.0.1:631");
        assert_eq!(scheme, "ipp");
        let result = parse_cups_uri("http://cups.internal");
        assert!(result.is_ok());
        let Ok((server, scheme)) = result else {
            return;
        };
        assert_eq!(server, "cups.internal:631");
        assert_eq!(scheme, "ipp");
        let result = parse_cups_uri("https://cups.internal:7443");
        assert!(result.is_ok());
        let Ok((server, scheme)) = result else {
            return;
        };
        assert_eq!(server, "cups.internal:7443");
        assert_eq!(scheme, "ipps");
        assert!(parse_cups_uri("cups://localhost").is_err());
    }
}
