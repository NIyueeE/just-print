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
/// 默认 sysfs 设备发现根目录。
pub const DEFAULT_SYSFS_ROOT: &str = "/sys/class/usb";
/// 默认设备节点目录。
pub const DEFAULT_DEVICE_DIR: &str = "/dev/usb";
/// 上传大小上限：64 MiB。
pub const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;
/// `LibreOffice` 转换超时。
pub const CONVERSION_TIMEOUT: Duration = Duration::from_mins(2);
/// 单次设备会话超时。
pub const SESSION_TIMEOUT: Duration = Duration::from_mins(1);
/// 设备发现轮询周期。
pub const DISCOVERY_INTERVAL: Duration = Duration::from_secs(5);
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
    /// 设备发现使用的 sysfs 根目录。
    pub sysfs_root: PathBuf,
    /// 打印机设备节点目录。
    pub device_dir: PathBuf,
    /// `Bearer` 准入令牌（已校验非空）。
    pub token: Arc<str>,
    /// 上传大小上限（字节）。
    pub max_upload_bytes: usize,
    /// `LibreOffice` 转换超时。
    pub conversion_timeout: Duration,
    /// 单次设备会话超时。
    pub session_timeout: Duration,
    /// 设备发现轮询周期。
    pub discovery_interval: Duration,
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
    /// 令牌缺失/为空，或 `JUST_PRINT_ADDR` 无法解析为 `SocketAddr` 时返回错误。
    pub fn from_env() -> Result<Self, ConfigError> {
        let token = env::var("JUST_PRINT_TOKEN").map_err(|_| ConfigError::MissingToken)?;
        if token.trim().is_empty() {
            return Err(ConfigError::MissingToken);
        }
        let addr = env::var("JUST_PRINT_ADDR")
            .unwrap_or_else(|_| DEFAULT_ADDR.to_string())
            .parse::<SocketAddr>()
            .map_err(|error| ConfigError::InvalidAddr(error.to_string()))?;
        let web_dir = PathBuf::from(
            env::var("JUST_PRINT_WEB_DIR").unwrap_or_else(|_| DEFAULT_WEB_DIR.to_string()),
        );
        let sysfs_root = PathBuf::from(
            env::var("JUST_PRINT_SYSFS_DIR").unwrap_or_else(|_| DEFAULT_SYSFS_ROOT.to_string()),
        );
        let device_dir = PathBuf::from(
            env::var("JUST_PRINT_DEVICE_DIR").unwrap_or_else(|_| DEFAULT_DEVICE_DIR.to_string()),
        );
        Ok(Self {
            addr,
            web_dir,
            sysfs_root,
            device_dir,
            token: Arc::from(token),
            max_upload_bytes: MAX_UPLOAD_BYTES,
            conversion_timeout: CONVERSION_TIMEOUT,
            session_timeout: SESSION_TIMEOUT,
            discovery_interval: DISCOVERY_INTERVAL,
            cleanup_interval: CLEANUP_INTERVAL,
            temp_ttl: TEMP_TTL,
        })
    }
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
}
