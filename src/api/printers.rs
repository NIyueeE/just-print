//! 打印机列表接口。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::registry::PrinterView;
use crate::state::AppState;

/// 打印机列表响应。
#[derive(Debug, Serialize)]
pub struct PrinterListResponse {
    /// 打印机列表。
    pub printers: Vec<PrinterView>,
}

/// 返回当前打印机列表与能力信息。
pub async fn list(State(state): State<Arc<AppState>>) -> Json<PrinterListResponse> {
    Json(PrinterListResponse {
        printers: state.printers.list().await,
    })
}
