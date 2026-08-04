//! 全局共享状态与临时目录管理。

use std::path::PathBuf;
use std::sync::Arc;

use crate::config::Config;
use crate::conversion::CONVERSION_SLOTS;
use crate::cups::CupsClient;
use crate::store::FileStore;
use tokio::sync::Semaphore;

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
    /// CUPS 客户端（打印机枚举、选项、提交与任务查询）。
    pub cups: CupsClient,
    /// 限制 `LibreOffice` 并发转换的信号量。
    pub conversion_slots: Arc<Semaphore>,
    /// 临时文件目录。
    pub temp_dir: PathBuf,
}

impl AppState {
    /// 组装共享状态。
    #[must_use]
    pub fn new(config: Config, temp_dir: PathBuf) -> Self {
        Self {
            files: Arc::new(FileStore::default()),
            cups: CupsClient::new(config.cups_server.clone(), config.cups_scheme.clone()),
            conversion_slots: Arc::new(Semaphore::new(CONVERSION_SLOTS)),
            config,
            temp_dir,
        }
    }
}
