//! 任务状态查询。

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use serde::Serialize;

use crate::error::AppError;
use crate::state::AppState;
use crate::store::JobStatus;

/// 任务状态响应。
#[derive(Debug, Serialize)]
pub struct JobView {
    /// 任务 id。
    pub id: String,
    /// 目标打印机 id。
    pub printer_id: String,
    /// 当前状态。
    pub status: JobStatus,
    /// 失败原因。
    pub error: Option<String>,
    /// 创建时间（Unix 毫秒）。
    pub created_at_ms: u64,
}

/// 查询任务状态。
pub async fn get(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<JobView>, AppError> {
    let record = state.jobs.get(&job_id).ok_or(AppError::JobNotFound)?;
    Ok(Json(JobView {
        id: record.id,
        printer_id: record.printer_id,
        status: record.status,
        error: record.error,
        created_at_ms: record.created_at_ms,
    }))
}
