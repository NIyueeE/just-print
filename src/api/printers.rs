//! 打印机列表接口。

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use tracing::warn;

use crate::cups::OptionSpec;
use crate::error::AppError;
use crate::state::AppState;

/// 暴露给前端的打印机视图。
#[derive(Debug, Clone, Serialize)]
pub struct PrinterView {
    /// CUPS 打印机名（提交任务时的 `printer_id`）。
    pub id: String,
    /// 展示名称（优先使用 CUPS 描述）。
    pub name: String,
    /// 打印机状态（`idle` / `printing` / `disabled` / `stopped`）。
    pub state: Option<String>,
    /// `lpoptions -l` 提供的合法控制项。
    pub options: BTreeMap<String, OptionSpec>,
}

/// 打印机列表响应。
#[derive(Debug, Serialize)]
pub struct PrinterListResponse {
    /// 打印机列表。
    pub printers: Vec<PrinterView>,
}

/// 返回 CUPS 打印机列表及控制项。
pub async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PrinterListResponse>, AppError> {
    let printers = state.cups.list_printers().await.map_err(AppError::from)?;
    let mut views = Vec::with_capacity(printers.len());
    for printer in printers {
        let options = match state.cups.list_options(&printer.name).await {
            Ok(options) => options,
            Err(error) => {
                warn!(printer = %printer.name, error = %error, "读取打印机选项失败，按空选项返回");
                BTreeMap::new()
            }
        };
        views.push(PrinterView {
            name: printer.description.unwrap_or_else(|| printer.name.clone()),
            id: printer.name,
            state: Some(printer.state),
            options,
        });
    }
    views.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(Json(PrinterListResponse { printers: views }))
}
