//! 打印机注册表：5 秒轮询发现、热插拔 diff、每打印机 worker 生命周期。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::ids;
use serde::Serialize;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::config::{DEFAULT_SYSFS_ROOT, FALLBACK_SYSFS_ROOT};
use crate::error::AppError;
use crate::pjl::{
    PRACTICAL_VARIABLES, Variable, VariableKind, practical_variables, print_language, printer_id,
    scan_sysfs_from_many, supports_pcl, supports_pdf, supports_postscript,
};
use crate::store::{FileStore, JobStatus, JobStore};
use crate::workers::{WorkerCommand, WorkerHandle, spawn_worker};

/// 暴露给前端的单变量能力视图。
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityView {
    /// 打印机报告的当前/默认值。
    pub default: Option<String>,
    /// `enumerated` 或 `range`。
    pub kind: &'static str,
    /// 枚举取值（仅枚举类型）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    /// 范围下界（仅范围类型）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    /// 范围上界（仅范围类型）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
}

/// 暴露给前端的打印机视图。
#[derive(Debug, Clone, Serialize)]
pub struct PrinterView {
    /// 稳定身份（序列号或 lp 节点路径）。
    pub id: String,
    /// 型号名称。
    pub name: String,
    /// 制造商。
    pub manufacturer: Option<String>,
    /// 序列号。
    pub serial: Option<String>,
    /// 是否支持 PDF 输出；能力未知时为 `false`。
    pub pdf_supported: bool,
    /// 是否支持 PostScript 输出（PDF 不可用时的回退语言）。
    pub postscript_supported: bool,
    /// 是否支持 PCL 输出（PDF 不可用时的回退语言）。
    pub pcl_supported: bool,
    /// 实用能力表；`None` 表示尚未查询到。
    pub capabilities: Option<BTreeMap<String, CapabilityView>>,
}

/// 打印机注册表。
pub struct PrinterRegistry {
    printers: RwLock<HashMap<String, WorkerHandle>>,
    queued: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    jobs: Arc<JobStore>,
    files: Arc<FileStore>,
    session_timeout: Duration,
    conversion_timeout: Duration,
    discovery_interval: Duration,
    sysfs_root: std::path::PathBuf,
    device_dir: std::path::PathBuf,
}

impl PrinterRegistry {
    /// 创建注册表。
    #[must_use]
    pub fn new(
        jobs: Arc<JobStore>,
        files: Arc<FileStore>,
        session_timeout: Duration,
        conversion_timeout: Duration,
        discovery_interval: Duration,
        sysfs_root: std::path::PathBuf,
        device_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            printers: RwLock::new(HashMap::new()),
            queued: Arc::new(Mutex::new(HashMap::new())),
            jobs,
            files,
            session_timeout,
            conversion_timeout,
            discovery_interval,
            sysfs_root,
            device_dir,
        }
    }

    /// 后台发现循环：每 `discovery_interval` 轮询一次 sysfs 并做 diff。
    pub async fn run(self: Arc<Self>) {
        loop {
            self.refresh().await;
            tokio::time::sleep(self.discovery_interval).await;
        }
    }

    /// 快照当前打印机列表。
    pub async fn list(&self) -> Vec<PrinterView> {
        let printers = self.printers.read().await;
        let handles: Vec<WorkerHandle> = printers.values().cloned().collect();
        drop(printers);

        let mut views = Vec::with_capacity(handles.len());
        for handle in handles {
            let capabilities = handle.capabilities.read().await.clone();
            let pdf_supported = capabilities.as_ref().is_some_and(supports_pdf);
            views.push(PrinterView {
                id: printer_id(&handle.printer),
                name: handle.printer.name.clone(),
                manufacturer: handle.printer.manufacturer.clone(),
                serial: handle.printer.serial.clone(),
                pdf_supported,
                postscript_supported: capabilities.as_ref().is_some_and(supports_postscript),
                pcl_supported: capabilities.as_ref().is_some_and(supports_pcl),
                capabilities: capabilities.as_ref().map(practical_view),
            });
        }
        views.sort_by(|left, right| left.id.cmp(&right.id));
        views
    }

    /// 校验控制信息并把打印任务入队，返回任务 id。
    ///
    /// # Errors
    ///
    /// 文件/打印机不存在、打印机未就绪或不支持 PDF / PCL / PostScript、控制信息非法、
    /// 或入队失败时返回 [`AppError`]。
    pub async fn submit(
        &self,
        printer_id: &str,
        file_id: &str,
        controls: BTreeMap<String, String>,
    ) -> Result<String, AppError> {
        let handle = {
            let printers = self.printers.read().await;
            printers.get(printer_id).cloned()
        }
        .ok_or(AppError::PrinterNotFound)?;

        let capabilities = handle.capabilities.read().await.clone().ok_or_else(|| {
            AppError::PrinterUnavailable("打印机能力尚未加载，请稍后重试".to_string())
        })?;
        let language = print_language(&capabilities).ok_or_else(|| {
            AppError::PrinterUnavailable(
                "打印机不支持或未报告可打印语言（PDF / PCL / PostScript），能力可能未完整加载"
                    .to_string(),
            )
        })?;
        validate_controls(&capabilities, &controls)?;

        let pdf_path = self.files.retain(file_id).ok_or(AppError::FileNotFound)?;
        let job_id = ids::new_id();
        self.jobs
            .insert(job_id.clone(), printer_id.to_string(), file_id.to_string());
        self.track(printer_id, &job_id);

        let command = WorkerCommand::Print {
            job_id: job_id.clone(),
            file_id: file_id.to_string(),
            pdf_path,
            controls,
            language,
        };
        if handle.tx.send(command).await.is_err() {
            self.untrack(printer_id, &job_id);
            self.jobs
                .set_status(&job_id, JobStatus::Failed, Some("打印机已移除".to_string()));
            self.files.release(file_id);
            return Err(AppError::PrinterUnavailable(
                "打印机已移除，任务未入队".to_string(),
            ));
        }
        Ok(job_id)
    }

    async fn refresh(&self) {
        let default_root = Path::new(DEFAULT_SYSFS_ROOT);
        let roots = if self.sysfs_root == default_root {
            vec![default_root, Path::new(FALLBACK_SYSFS_ROOT)]
        } else {
            vec![self.sysfs_root.as_path()]
        };
        let found = scan_sysfs_from_many(&roots, &self.device_dir);
        let mut printers = self.printers.write().await;
        let mut next = HashMap::with_capacity(found.len());

        for printer in found {
            let id = printer_id(&printer);
            let existing = printers.remove(&id);
            match existing {
                Some(handle) if handle.printer.path == printer.path => {
                    let missing = handle.capabilities.read().await.is_none();
                    if missing {
                        let _ = handle.tx.send(WorkerCommand::Query).await;
                    }
                    next.insert(id, handle);
                }
                Some(handle) => {
                    warn!(printer = %id, "设备路径变化，重建 worker");
                    let _ = handle.tx.send(WorkerCommand::Shutdown).await;
                    let handle = spawn_worker(
                        printer,
                        Arc::clone(&self.jobs),
                        Arc::clone(&self.files),
                        Arc::clone(&self.queued),
                        self.session_timeout,
                        self.conversion_timeout,
                    );
                    let _ = handle.tx.send(WorkerCommand::Query).await;
                    next.insert(id, handle);
                }
                None => {
                    info!(printer = %id, "发现新打印机");
                    let handle = spawn_worker(
                        printer,
                        Arc::clone(&self.jobs),
                        Arc::clone(&self.files),
                        Arc::clone(&self.queued),
                        self.session_timeout,
                        self.conversion_timeout,
                    );
                    let _ = handle.tx.send(WorkerCommand::Query).await;
                    next.insert(id, handle);
                }
            }
        }

        for (id, handle) in printers.drain() {
            info!(printer = %id, "打印机已移除");
            self.fail_printer(&id);
            let _ = handle.tx.send(WorkerCommand::Shutdown).await;
        }
        *printers = next;
    }

    fn track(&self, printer_id: &str, job_id: &str) {
        self.queued
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(printer_id.to_string())
            .or_default()
            .insert(job_id.to_string());
    }

    fn untrack(&self, printer_id: &str, job_id: &str) {
        let mut queued = self
            .queued
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(jobs) = queued.get_mut(printer_id) {
            jobs.remove(job_id);
        }
    }

    fn fail_printer(&self, printer_id: &str) {
        let queued = self
            .queued
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(printer_id)
            .unwrap_or_default();
        for job_id in queued {
            if let Some(record) = self.jobs.get(&job_id) {
                self.files.release(&record.file_id);
            }
            self.jobs
                .set_status(&job_id, JobStatus::Failed, Some("打印机已移除".to_string()));
        }
    }
}

fn practical_view(capabilities: &BTreeMap<String, Variable>) -> BTreeMap<String, CapabilityView> {
    practical_variables(capabilities)
        .into_iter()
        .map(|(name, variable)| (name, capability_view(&variable)))
        .collect()
}

fn capability_view(variable: &Variable) -> CapabilityView {
    match &variable.kind {
        VariableKind::Enumerated { values } => CapabilityView {
            default: variable.default.clone(),
            kind: "enumerated",
            values: Some(values.clone()),
            min: None,
            max: None,
        },
        VariableKind::Range { min, max } => CapabilityView {
            default: variable.default.clone(),
            kind: "range",
            values: None,
            min: Some(*min),
            max: Some(*max),
        },
    }
}

fn validate_controls(
    capabilities: &BTreeMap<String, Variable>,
    controls: &BTreeMap<String, String>,
) -> Result<(), AppError> {
    for (key, value) in controls {
        if !PRACTICAL_VARIABLES.contains(&key.as_str()) {
            return Err(AppError::InvalidControls(format!("不允许的控制参数 {key}")));
        }
        let variable = capabilities
            .get(key)
            .ok_or_else(|| AppError::InvalidControls(format!("打印机未报告参数 {key}")))?;
        let valid = match &variable.kind {
            VariableKind::Enumerated { values } => {
                values.iter().any(|candidate| candidate == value)
            }
            VariableKind::Range { min, max } => value
                .parse::<i64>()
                .is_ok_and(|number| number >= *min && number <= *max),
        };
        if !valid {
            return Err(AppError::InvalidControls(format!(
                "参数 {key} 的取值 {value} 不在合法范围内"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::pjl::{Variable, VariableKind};

    use super::validate_controls;

    #[test]
    fn rejects_unknown_and_invalid_values() {
        let mut capabilities = BTreeMap::new();
        capabilities.insert(
            "DUPLEX".to_string(),
            Variable {
                default: Some("OFF".to_string()),
                kind: VariableKind::Enumerated {
                    values: vec!["OFF".to_string(), "ON".to_string()],
                },
            },
        );
        capabilities.insert(
            "DENSITY".to_string(),
            Variable {
                default: Some("0".to_string()),
                kind: VariableKind::Range { min: -6, max: 6 },
            },
        );
        let mut controls = BTreeMap::new();
        controls.insert("DUPLEX".to_string(), "ON".to_string());
        controls.insert("DENSITY".to_string(), "3".to_string());
        assert!(validate_controls(&capabilities, &controls).is_ok());

        controls.insert("DUPLEX".to_string(), "SIDEWAYS".to_string());
        assert!(validate_controls(&capabilities, &controls).is_err());
        controls.insert("DENSITY".to_string(), "99".to_string());
        assert!(validate_controls(&capabilities, &controls).is_err());
        controls.remove("DUPLEX");
        controls.insert("HACK".to_string(), "1".to_string());
        assert!(validate_controls(&capabilities, &controls).is_err());
    }
}
