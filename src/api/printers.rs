//! 打印机列表接口。

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};

use crate::cups::{OptionSpec, PrinterStateView};
use crate::error::AppError;
use crate::state::AppState;

/// 打印机列表查询参数。
#[derive(Debug, Deserialize)]
pub struct PrinterListQuery {
    /// 为 `true` 时跳过服务端缓存，强制向 CUPS 重新拉取。
    pub refresh: Option<bool>,
}

/// 暴露给前端的打印机视图。
#[derive(Debug, Clone, Serialize)]
pub struct PrinterView {
    /// CUPS 打印机名（提交任务时的 `printer_id`）。
    pub id: String,
    /// 展示名称（优先使用 CUPS `printer-info`）。
    pub name: String,
    /// 打印机状态。
    pub state: PrinterStateView,
    /// 是否接受新任务。
    pub accepting_jobs: bool,
    /// 厂商与型号。
    pub make_and_model: Option<String>,
    /// 物理位置。
    pub location: Option<String>,
    /// 标准 IPP job 模板选项目录。
    pub options: BTreeMap<String, OptionSpec>,
    /// 读取选项失败时的提示；成功时为 `null`。
    pub options_error: Option<String>,
}

/// 打印机列表响应。
#[derive(Debug, Serialize)]
pub struct PrinterListResponse {
    /// 打印机列表。
    pub printers: Vec<PrinterView>,
}

/// 返回 CUPS 打印机列表及控制项（带服务端缓存）。
pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PrinterListQuery>,
) -> Result<Json<PrinterListResponse>, AppError> {
    if query.refresh == Some(true) {
        state.cups.invalidate_printers().await;
    }
    let printers = state.cups.list_printers().await.map_err(AppError::from)?;
    let views = printers
        .iter()
        .map(|printer| PrinterView {
            id: printer.name.clone(),
            name: printer.display_name.clone(),
            state: printer.state,
            accepting_jobs: printer.accepting_jobs,
            make_and_model: printer.make_and_model.clone(),
            location: printer.location.clone(),
            options: printer.options.clone(),
            options_error: printer.options_error.clone(),
        })
        .collect();
    Ok(Json(PrinterListResponse { printers: views }))
}
