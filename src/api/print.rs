//! 提交打印任务。

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::state::AppState;

/// 打印请求。
#[derive(Debug, Deserialize)]
pub struct PrintRequest {
    /// 已上传文件 id。
    pub file_id: String,
    /// 目标打印机 id。
    pub printer_id: String,
    /// 控制信息（可选，值必须来自能力查询）。
    #[serde(default)]
    pub controls: BTreeMap<String, String>,
}

/// 打印提交响应。
#[derive(Debug, Serialize)]
pub struct PrintResponse {
    /// 任务 id。
    pub job_id: String,
}

/// 校验并入队打印任务，立即返回任务 id。
pub async fn submit(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PrintRequest>,
) -> Result<(StatusCode, Json<PrintResponse>), AppError> {
    let job_id = state
        .printers
        .submit(&request.printer_id, &request.file_id, request.controls)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(PrintResponse { job_id })))
}
