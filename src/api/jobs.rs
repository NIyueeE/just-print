//! 任务状态查询。

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use serde::Serialize;

use crate::cups::JobStatus;
use crate::error::AppError;
use crate::state::AppState;

/// 任务状态响应。
#[derive(Debug, Serialize)]
pub struct JobView {
    /// CUPS 任务 id（`printer-job` 形式）。
    pub id: String,
    /// 目标打印机名。
    pub printer_id: String,
    /// 当前状态。
    pub status: JobStatus,
    /// 失败/取消原因。
    pub error: Option<String>,
    /// 创建时间（Unix 毫秒，UTC）。
    pub created_at_ms: u64,
}

/// 查询任务状态；状态直接来自 CUPS spool。
pub async fn get(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<JobView>, AppError> {
    let job = state
        .cups
        .job_status(&job_id)
        .await
        .map_err(|error| AppError::Internal(format!("CUPS 不可用: {error}")))?
        .ok_or(AppError::JobNotFound)?;
    Ok(Json(JobView {
        id: job.id,
        printer_id: job.printer_id,
        status: job.status,
        error: job.error,
        created_at_ms: job.created_at_ms,
    }))
}
