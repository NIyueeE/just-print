//! 每打印机 FIFO worker：同一台打印机的所有设备访问严格串行。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::conversion;
use crate::pjl::{DiscoveredPrinter, Variable, print_document, query_capabilities, reset};
use crate::store::{FileStore, JobStatus, JobStore};
use tokio::sync::{RwLock, mpsc};
use tracing::{info, warn};

/// 一台打印机的 worker 句柄。
#[derive(Debug, Clone)]
pub struct WorkerHandle {
    /// 设备信息（路径、名称、序列号）。
    pub printer: DiscoveredPrinter,
    /// 已缓存的能力表；`None` 表示尚未查询成功。
    pub capabilities: Arc<RwLock<Option<BTreeMap<String, Variable>>>>,
    /// 命令发送端。
    pub tx: mpsc::Sender<WorkerCommand>,
}

/// 发送给 worker 的设备会话命令。
#[derive(Debug)]
pub enum WorkerCommand {
    /// 查询并缓存能力。
    Query,
    /// 打印一个 PDF。
    Print {
        /// 任务 id。
        job_id: String,
        /// 关联文件 id（终态后归还引用）。
        file_id: String,
        /// PDF 文件路径。
        pdf_path: PathBuf,
        /// 已校验的控制信息。
        controls: BTreeMap<String, String>,
        /// 打印语言（`PDF` 或 `POSTSCRIPT`，由能力表选择）。
        language: &'static str,
    },
    /// 停止 worker（打印机已移除）。
    Shutdown,
}

/// worker 依赖的共享上下文。
struct WorkerContext {
    jobs: Arc<JobStore>,
    files: Arc<FileStore>,
    queued: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    capabilities: Arc<RwLock<Option<BTreeMap<String, Variable>>>>,
    session_timeout: Duration,
    conversion_timeout: Duration,
    printer_id: String,
}

/// 为打印机创建 worker 并返回句柄。
#[must_use]
pub fn spawn_worker(
    printer: DiscoveredPrinter,
    jobs: Arc<JobStore>,
    files: Arc<FileStore>,
    queued: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    session_timeout: Duration,
    conversion_timeout: Duration,
) -> WorkerHandle {
    let (tx, rx) = mpsc::channel(64);
    let capabilities = Arc::new(RwLock::new(None));
    let printer_id = crate::pjl::printer_id(&printer);
    let context = WorkerContext {
        jobs,
        files,
        queued,
        capabilities: Arc::clone(&capabilities),
        session_timeout,
        conversion_timeout,
        printer_id: printer_id.clone(),
    };
    let loop_printer = printer.clone();
    tokio::spawn(worker_loop(loop_printer, rx, context));
    WorkerHandle {
        printer,
        capabilities,
        tx,
    }
}

async fn worker_loop(
    printer: DiscoveredPrinter,
    mut rx: mpsc::Receiver<WorkerCommand>,
    context: WorkerContext,
) {
    let mut needs_reset = false;
    while let Some(command) = rx.recv().await {
        match command {
            WorkerCommand::Query => {
                match query_capabilities(&printer.path, context.session_timeout).await {
                    Ok(variables) => {
                        *context.capabilities.write().await = Some(variables);
                        needs_reset = false;
                        info!(printer = %context.printer_id, "打印机能力查询成功");
                    }
                    Err(error) => {
                        warn!(
                            printer = %context.printer_id,
                            error = %error,
                            "能力查询失败，将在下个轮询周期重试"
                        );
                        needs_reset = true;
                    }
                }
            }
            WorkerCommand::Print {
                job_id,
                file_id,
                pdf_path,
                controls,
                language,
            } => {
                {
                    let mut queued = context
                        .queued
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Some(jobs) = queued.get_mut(&context.printer_id) {
                        jobs.remove(&job_id);
                    }
                }

                context.jobs.set_status(&job_id, JobStatus::Printing, None);
                if needs_reset {
                    info!(printer = %context.printer_id, job_id = %job_id, "会话前发送 UEL 复位");
                    let _ = reset(&printer.path, context.session_timeout).await;
                }

                let result: Result<(), String> = (async {
                    let document = tokio::fs::read(&pdf_path)
                        .await
                        .map_err(|error| format!("读取 PDF 失败: {error}"))?;
                    let document = match language {
                        "POSTSCRIPT" => conversion::convert_pdf_to_postscript(
                            &document,
                            context.conversion_timeout,
                        )
                        .await
                        .map_err(|error| error.to_string())?,
                        "PCL" => {
                            conversion::convert_pdf_to_pcl(&document, context.conversion_timeout)
                                .await
                                .map_err(|error| error.to_string())?
                        }
                        _ => document,
                    };
                    tokio::time::timeout(
                        context.session_timeout,
                        print_document(
                            &printer.path,
                            &document,
                            language,
                            &controls,
                            context.session_timeout,
                        ),
                    )
                    .await
                    .map_err(|_| crate::pjl::PjlError::Timeout("print").to_string())?
                    .map_err(|error| error.to_string())
                })
                .await;

                match result {
                    Ok(()) => {
                        context.jobs.set_status(&job_id, JobStatus::Success, None);
                        info!(printer = %context.printer_id, job_id = %job_id, "打印任务成功");
                    }
                    Err(message) => {
                        context
                            .jobs
                            .set_status(&job_id, JobStatus::Failed, Some(message.clone()));
                        needs_reset = true;
                        warn!(printer = %context.printer_id, job_id = %job_id, error = %message, "打印任务失败");
                    }
                }
                context.files.release(&file_id);
            }
            WorkerCommand::Shutdown => break,
        }
    }
    warn!(printer = %context.printer_id, "worker 已停止");
}
