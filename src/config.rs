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
/// 已转换文件占用临时目录的总字节上限。
pub const DEFAULT_MAX_TEMP_BYTES: u64 = 1024 * 1024 * 1024;
/// 停机时等待已受理打印提交完成的预算。
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
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
/// 同键重试等待在途请求完成的最长时间。
pub const DEFAULT_IDEMPOTENCY_WAIT: Duration = Duration::from_secs(15);
/// 并发上传上限。
pub const DEFAULT_UPLOAD_SLOTS: usize = 4;
/// 单个 HTTP 请求处理超时。
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_mins(5);
/// 就绪探针查询 CUPS 的超时；必须短于容器健康检查的超时。
pub const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(3);

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
    /// 已转换文件占用临时目录的总字节上限。
    pub max_temp_bytes: u64,
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
    /// 同键重试等待在途请求完成的最长时间。
    pub idempotency_wait: Duration,
    /// 并发上传上限。
    pub upload_slots: usize,
    /// 单个 HTTP 请求处理超时。
    pub request_timeout: Duration,
    /// 停机时等待在途打印提交的上限。
    pub shutdown_timeout: Duration,
    /// 就绪探针查询 CUPS 的超时。
    pub ready_timeout: Duration,
    /// MCP 对外基地址；配置后优先于请求头，用于反向代理和 TLS 部署。
    pub public_base_url: Option<String>,
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
        let public_base_url = env::var("JUST_PRINT_PUBLIC_BASE_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| parse_public_base_url(&value).map_err(ConfigError::InvalidPublicBaseUrl))
            .transpose()?;
        let config = Self {
            addr,
            web_dir,
            token: Arc::from(token),
            cups_server,
            cups_scheme,
            max_upload_bytes: env_usize("JUST_PRINT_MAX_UPLOAD_BYTES", DEFAULT_MAX_UPLOAD_BYTES)?,
            max_temp_bytes: env_u64("JUST_PRINT_MAX_TEMP_BYTES", DEFAULT_MAX_TEMP_BYTES)?,
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
            idempotency_wait: env_duration(
                "JUST_PRINT_IDEMPOTENCY_WAIT_SECS",
                DEFAULT_IDEMPOTENCY_WAIT,
            )?,
            upload_slots: env_usize("JUST_PRINT_UPLOAD_SLOTS", DEFAULT_UPLOAD_SLOTS)?,
            request_timeout: env_duration(
                "JUST_PRINT_REQUEST_TIMEOUT_SECS",
                DEFAULT_REQUEST_TIMEOUT,
            )?,
            shutdown_timeout: env_duration(
                "JUST_PRINT_SHUTDOWN_TIMEOUT_SECS",
                DEFAULT_SHUTDOWN_TIMEOUT,
            )?,
            ready_timeout: env_duration("JUST_PRINT_READY_TIMEOUT_SECS", DEFAULT_READY_TIMEOUT)?,
            public_base_url,
        };
        config.validate_consistency()?;
        Ok(config)
    }

    /// 校验相互依赖的限额。配置可单独调整时，错误的组合会破坏
    /// 幂等或清理语义，必须在启动时 fail-closed。
    fn validate_consistency(&self) -> Result<(), ConfigError> {
        // 一次 Print-Job 最多经历打印查找、提交、响应丢失后的对账三次 IPP；
        // 幂等条目若比这个窗口还短，重试会被当成一次新提交而重复出纸。
        let submission_window = self.ipp_timeout.saturating_mul(3);
        if self.idempotency_ttl < submission_window {
            return Err(ConfigError::InvalidCombination(format!(
                "JUST_PRINT_IDEMPOTENCY_TTL_SECS 必须至少为 3×IPP 超时（{} 秒）",
                submission_window.as_secs()
            )));
        }
        if self.temp_ttl <= self.conversion_timeout {
            return Err(ConfigError::InvalidCombination(
                "JUST_PRINT_TEMP_TTL_SECS 必须大于转换超时，否则清理可能删除仍在转换的中间文件"
                    .to_string(),
            ));
        }
        if self.max_temp_bytes < u64::try_from(self.max_upload_bytes).unwrap_or(u64::MAX) {
            return Err(ConfigError::InvalidCombination(
                "JUST_PRINT_MAX_TEMP_BYTES 不能小于单文件上传上限，否则文件永远无法登记"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

fn env_u64(key: &str, default: u64) -> Result<u64, ConfigError> {
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
    Ok(value)
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

/// 校验 MCP 对外基地址，只接受无路径、无查询的 HTTP(S) origin。
fn parse_public_base_url(value: &str) -> Result<String, String> {
    use axum::http::Uri;

    let uri = value
        .parse::<Uri>()
        .map_err(|error| format!("无法解析: {error}"))?;
    match uri.scheme_str() {
        Some("http" | "https") => {}
        _ => return Err("仅支持 http:// 或 https://".to_string()),
    }
    let Some(authority) = uri.authority() else {
        return Err("缺少主机名".to_string());
    };
    let authority = authority.as_str();
    if authority.contains('@') {
        return Err("不允许包含用户信息".to_string());
    }
    if !authority
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ':' | '[' | ']'))
    {
        return Err("主机名包含不允许的字符".to_string());
    }
    if uri.host().is_none_or(str::is_empty) {
        return Err("缺少主机名".to_string());
    }
    if let Some(path) = uri.path_and_query()
        && (path.path() != "/" || path.query().is_some())
    {
        return Err("只允许 origin，不能包含路径或查询串".to_string());
    }
    Ok(value.trim_end_matches('/').to_string())
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
    /// MCP 对外基地址不合法。
    #[error("JUST_PRINT_PUBLIC_BASE_URL 不合法: {0}")]
    InvalidPublicBaseUrl(String),
    /// 数值型环境变量不合法。
    #[error("{key} 不是合法的正整数: {value}")]
    InvalidNumber {
        /// 环境变量名。
        key: String,
        /// 原始取值。
        value: String,
    },
    /// 多个限额之间的组合不满足运行时不变式。
    #[error("配置组合不合法: {0}")]
    InvalidCombination(String),
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::{
        Config, DEFAULT_CLEANUP_INTERVAL, DEFAULT_CONVERSION_SLOTS, DEFAULT_CONVERSION_TIMEOUT,
        DEFAULT_CUPS_URI, DEFAULT_IDEMPOTENCY_TTL, DEFAULT_IDEMPOTENCY_WAIT, DEFAULT_IPP_TIMEOUT,
        DEFAULT_JOB_CACHE_TTL, DEFAULT_MAX_TEMP_BYTES, DEFAULT_MAX_UPLOAD_BYTES,
        DEFAULT_PRINTER_CACHE_TTL, DEFAULT_READY_TIMEOUT, DEFAULT_REQUEST_TIMEOUT,
        DEFAULT_SHUTDOWN_TIMEOUT, DEFAULT_TEMP_TTL, DEFAULT_UPLOAD_SLOTS, parse_cups_uri,
        parse_public_base_url,
    };

    fn base_config() -> Config {
        let (cups_server, cups_scheme) = parse_cups_uri(DEFAULT_CUPS_URI)
            .unwrap_or_else(|_| ("127.0.0.1:631".to_string(), "ipp".to_string()));
        Config {
            addr: "127.0.0.1:8080"
                .parse()
                .unwrap_or_else(|_| std::net::SocketAddr::from(([127, 0, 0, 1], 8080))),
            web_dir: std::path::PathBuf::from("/tmp"),
            token: Arc::from("token"),
            cups_server,
            cups_scheme,
            max_upload_bytes: DEFAULT_MAX_UPLOAD_BYTES,
            max_temp_bytes: DEFAULT_MAX_TEMP_BYTES,
            conversion_timeout: DEFAULT_CONVERSION_TIMEOUT,
            conversion_slots: DEFAULT_CONVERSION_SLOTS,
            cleanup_interval: DEFAULT_CLEANUP_INTERVAL,
            temp_ttl: DEFAULT_TEMP_TTL,
            ipp_timeout: DEFAULT_IPP_TIMEOUT,
            printer_cache_ttl: DEFAULT_PRINTER_CACHE_TTL,
            job_cache_ttl: DEFAULT_JOB_CACHE_TTL,
            idempotency_ttl: DEFAULT_IDEMPOTENCY_TTL,
            idempotency_wait: DEFAULT_IDEMPOTENCY_WAIT,
            upload_slots: DEFAULT_UPLOAD_SLOTS,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
            ready_timeout: DEFAULT_READY_TIMEOUT,
            public_base_url: None,
        }
    }

    #[test]
    fn rejects_inconsistent_limit_combinations() {
        let config = base_config();
        assert!(config.validate_consistency().is_ok());

        let mut short_idempotency = config.clone();
        short_idempotency.idempotency_ttl = Duration::from_secs(1);
        assert!(short_idempotency.validate_consistency().is_err());

        let mut short_temp_ttl = config.clone();
        short_temp_ttl.temp_ttl = Duration::from_secs(1);
        assert!(short_temp_ttl.validate_consistency().is_err());

        let mut tiny_temp = config;
        tiny_temp.max_temp_bytes = 1;
        assert!(tiny_temp.validate_consistency().is_err());
    }

    #[test]
    fn parses_only_safe_public_base_urls() {
        assert_eq!(
            parse_public_base_url("https://print.example.com"),
            Ok("https://print.example.com".to_string())
        );
        assert_eq!(
            parse_public_base_url("http://192.168.1.10:8080/"),
            Ok("http://192.168.1.10:8080".to_string())
        );
        for invalid in [
            "ftp://print.example.com",
            "https://user@print.example.com",
            "https://print.example.com/base",
            "https://print.example.com?x=1",
            "https://print.example.com';touch /tmp/pwn;'",
            "print.example.com",
        ] {
            assert!(
                parse_public_base_url(invalid).is_err(),
                "{invalid} 不应被接受"
            );
        }
    }

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
