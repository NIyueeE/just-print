//! 全局共享状态与临时目录管理。

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::config::Config;
use crate::cups::{CupsClient, CupsError};
use crate::idempotency::IdempotencyStore;
use crate::jobs::JobRegistry;
use crate::metrics::Metrics;
use crate::store::FileStore;

/// 幂等键存储容量上限。
pub const IDEMPOTENCY_CAPACITY: usize = 1024;
/// 任务登记表容量上限。
pub const JOB_REGISTRY_CAPACITY: usize = 512;
/// 任务元数据保留时长。
pub const JOB_REGISTRY_TTL: std::time::Duration = std::time::Duration::from_hours(24);

/// 容器内临时目录；启动时清空上一次运行的残留文件。
#[derive(Debug)]
pub struct TempDir {
    /// 临时目录根路径。
    pub root: PathBuf,
}

impl TempDir {
    /// 创建并清空临时目录。
    ///
    /// # Errors
    ///
    /// 目录创建或残留文件清理失败时返回 I/O 错误。
    pub fn prepare() -> Result<Self, std::io::Error> {
        let root = std::env::temp_dir().join("just-print");
        std::fs::create_dir_all(&root)?;
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                std::fs::remove_dir_all(path)?;
            } else {
                std::fs::remove_file(path)?;
            }
        }
        Ok(Self { root })
    }
}

/// 后端共享状态。
pub struct AppState {
    /// 应用配置。
    pub config: Config,
    /// 文件存储（引用计数 + TTL）。
    pub files: Arc<FileStore>,
    /// IPP 客户端（打印机枚举、选项、提交、任务查询与取消）。
    pub cups: Arc<CupsClient>,
    /// 任务元数据登记表。
    pub jobs: JobRegistry,
    /// 幂等键存储。
    pub idempotency: IdempotencyStore,
    /// Prometheus 指标。
    pub metrics: Arc<Metrics>,
    /// 限制 `LibreOffice` 并发转换的信号量。
    pub conversion_slots: Arc<Semaphore>,
    /// 限制并发上传的信号量。
    pub upload_slots: Arc<Semaphore>,
    /// 临时文件目录。
    pub temp_dir: PathBuf,
}

impl AppState {
    /// 组装共享状态。
    ///
    /// # Errors
    ///
    /// CUPS 地址不合法时返回 [`CupsError`]。
    pub fn new(config: Config, temp_dir: PathBuf) -> Result<Self, CupsError> {
        let cups = CupsClient::new(
            config.cups_server.clone(),
            config.cups_scheme.clone(),
            config.ipp_timeout,
            config.printer_cache_ttl,
            config.job_cache_ttl,
        )?;
        Ok(Self {
            files: Arc::new(FileStore::default()),
            cups: Arc::new(cups),
            jobs: JobRegistry::new(JOB_REGISTRY_TTL, JOB_REGISTRY_CAPACITY),
            idempotency: IdempotencyStore::new(config.idempotency_ttl, IDEMPOTENCY_CAPACITY),
            metrics: Arc::new(Metrics::new()),
            conversion_slots: Arc::new(Semaphore::new(config.conversion_slots)),
            upload_slots: Arc::new(Semaphore::new(config.upload_slots)),
            config,
            temp_dir,
        })
    }
}
