//! 环境变量配置与 fail-closed 校验。

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;

use crate::cups::DEFAULT_CUPS_URI;

/// 默认监听地址。
pub const DEFAULT_ADDR: &str = "0.0.0.0:8080";
/// 默认静态前端目录（镜像内置路径）。
pub const DEFAULT_WEB_DIR: &str = "/usr/share/just-print/web";
/// 上传大小上限：64 MiB。
pub const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;
/// `LibreOffice` 转换超时。
pub const CONVERSION_TIMEOUT: Duration = Duration::from_mins(2);
/// 临时文件清理周期。
pub const CLEANUP_INTERVAL: Duration = Duration::from_mins(1);
/// 无引用临时文件的保留时长。
pub const TEMP_TTL: Duration = Duration::from_mins(30);

/// 应用配置。
#[derive(Debug, Clone)]
pub struct Config {
    /// HTTP 监听地址。
    pub addr: SocketAddr,
    /// 静态前端目录。
    pub web_dir: PathBuf,
    /// `Bearer` 准入令牌（已校验非空）。
    pub token: Arc<str>,
    /// CUPS 服务地址（`host:port`，供 CUPS 命令行工具使用）。
    pub cups_server: String,
    /// CUPS 服务 scheme（`ipp` / `ipps`，供 IPP 查询使用）。
    pub cups_scheme: String,
    /// 上传大小上限（字节）。
    pub max_upload_bytes: usize,
    /// `LibreOffice` 转换超时。
    pub conversion_timeout: Duration,
    /// 临时文件清理周期。
    pub cleanup_interval: Duration,
    /// 无引用临时文件保留时长。
    pub temp_ttl: Duration,
}

impl Config {
    /// 从环境变量加载配置；令牌缺失或为空时拒绝启动（fail-closed）。
    ///
    /// # Errors
    ///
    /// 令牌缺失/为空、`JUST_PRINT_ADDR` 无法解析为 `SocketAddr`，
    /// 或 `JUST_PRINT_CUPS_URI` 不是合法的 `http(s)://host[:port]` 时返回错误。
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
            max_upload_bytes: MAX_UPLOAD_BYTES,
            conversion_timeout: CONVERSION_TIMEOUT,
            cleanup_interval: CLEANUP_INTERVAL,
            temp_ttl: TEMP_TTL,
        })
    }
}

fn parse_cups_uri(uri: &str) -> Result<(String, String), String> {
    let (rest, default_port, scheme) = if let Some(rest) = uri.strip_prefix("http://") {
        (rest, "631", "ipp")
    } else if let Some(rest) = uri.strip_prefix("https://") {
        (rest, "632", "ipps")
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
