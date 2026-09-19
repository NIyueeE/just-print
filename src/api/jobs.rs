//! 任务查询与取消。

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::cups::{CupsError, JobInfo, JobStatus};
use crate::error::AppError;
use crate::jobs::JobRecord;
use crate::state::AppState;

/// 任务列表查询参数。
#[derive(Debug, Deserialize)]
pub struct JobListQuery {
    /// 返回条数上限（1–200，默认 50）。
    pub limit: Option<usize>,
    /// 只返回未结束（排队/打印中）的任务。
    pub active: Option<bool>,
}

/// 任务视图。
#[derive(Debug, Serialize)]
pub struct JobView {
    /// 应用层任务 id：`<printer>-<job-id>`。
    pub id: String,
    /// 目标打印机名。
    pub printer_id: String,
    /// CUPS 中的任务名。
    pub name: Option<String>,
    /// 关联的上传文件 id（来自登记表）。
    pub file_id: Option<String>,
    /// 原始文件名（来自登记表）。
    pub file_name: Option<String>,
    /// 当前状态；CUPS 已清理时未知。
    pub status: Option<JobStatus>,
    /// 失败/取消原因。
    pub error: Option<String>,
    /// 创建时间（Unix 毫秒）。
    pub created_at_ms: u64,
    /// 提交时使用的选项。
    pub options: BTreeMap<String, String>,
}

/// 任务列表响应。
#[derive(Debug, Serialize)]
pub struct JobListResponse {
    /// 任务列表（新提交的在前）。
    pub jobs: Vec<JobView>,
}

/// 查询任务列表，并用 CUPS 刷新每个任务的最新状态。
pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(query): Query<JobListQuery>,
) -> Result<Json<JobListResponse>, AppError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let records = state.jobs.list(limit);

    // 每台打印机只向 CUPS 查一次任务列表，避免按任务数放大为 N 次 IPP 请求。
    let mut by_printer: HashMap<String, HashMap<i32, JobInfo>> = HashMap::new();
    for record in &records {
        if let Some((printer, _)) = split_job_id(&record.id)
            && !by_printer.contains_key(&printer)
        {
            let jobs = state
                .cups
                .list_jobs(&printer)
                .await
                .map_err(AppError::from)?;
            let map = jobs.into_iter().map(|job| (job.cups_job_id, job)).collect();
            by_printer.insert(printer, map);
        }
    }

    let mut views = Vec::with_capacity(records.len());
    for record in records {
        let info = split_job_id(&record.id).and_then(|(printer, cups_job_id)| {
            by_printer
                .get(&printer)
                .and_then(|jobs| jobs.get(&cups_job_id))
                .cloned()
        });
        if query.active == Some(true)
            && !matches!(
                info.as_ref().map(|job| job.status),
                Some(JobStatus::Queued | JobStatus::Printing)
            )
        {
            continue;
        }
        views.push(view_from(Some(record), info.as_ref()));
    }
    Ok(Json(JobListResponse { jobs: views }))
}

/// 查询单个任务状态。
pub async fn get(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<JobView>, AppError> {
    let (record, printer, cups_job_id) = resolve(&state, &job_id)?;
    let info = state
        .cups
        .job_status(&printer, cups_job_id)
        .await
        .map_err(AppError::from)?
        .ok_or(AppError::JobNotFound)?;
    Ok(Json(view_from(record, Some(&info))))
}

/// 取消任务。
pub async fn cancel(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<StatusCode, AppError> {
    let (record, printer, cups_job_id) = resolve(&state, &job_id)?;
    state
        .cups
        .cancel_job(&printer, cups_job_id)
        .await
        .map_err(|error| match error {
            CupsError::NotFound => AppError::JobNotFound,
            other => AppError::from(other),
        })?;
    if let Some(record) = record {
        state.jobs.remove(&record.id);
    }
    state.metrics.inc("print_jobs", &[("outcome", "canceled")]);
    tracing::info!(job_id = %job_id, "打印任务已取消");
    Ok(StatusCode::NO_CONTENT)
}

/// 解析任务 id：登记表优先，其次 `<printer>-<job>` 形式，纯数字仅在唯一时接受。
fn resolve(state: &AppState, job_id: &str) -> Result<(Option<JobRecord>, String, i32), AppError> {
    if let Some(record) = state.jobs.get(job_id)
        && let Some((printer, cups_job_id)) = split_job_id(&record.id)
    {
        return Ok((Some(record), printer, cups_job_id));
    }
    if let Some((printer, cups_job_id)) = split_job_id(job_id) {
        return Ok((None, printer, cups_job_id));
    }
    if job_id.chars().all(|ch| ch.is_ascii_digit()) {
        let matches: Vec<JobRecord> = state
            .jobs
            .list(usize::MAX)
            .into_iter()
            .filter(|record| {
                record
                    .id
                    .rsplit_once('-')
                    .is_some_and(|(_, number)| number == job_id)
            })
            .collect();
        match matches.len() {
            1 => {
                if let Some(record) = matches.into_iter().next()
                    && let Some((printer, cups_job_id)) = split_job_id(&record.id)
                {
                    return Ok((Some(record), printer, cups_job_id));
                }
            }
            0 => {}
            _ => {
                return Err(AppError::BadRequest(
                    "任务号在多台打印机上重复，请使用 <printer>-<job> 形式".to_string(),
                ));
            }
        }
    }
    Err(AppError::JobNotFound)
}

fn split_job_id(id: &str) -> Option<(String, i32)> {
    let (printer, number) = id.rsplit_once('-')?;
    if printer.is_empty() {
        return None;
    }
    let cups_job_id = number.parse::<i32>().ok()?;
    if cups_job_id <= 0 {
        return None;
    }
    Some((printer.to_string(), cups_job_id))
}

fn view_from(record: Option<JobRecord>, info: Option<&JobInfo>) -> JobView {
    let id = info
        .map(JobInfo::id)
        .or_else(|| record.as_ref().map(|record| record.id.clone()))
        .unwrap_or_default();
    let printer_id = info
        .map(|job| job.printer_id.clone())
        .or_else(|| record.as_ref().map(|record| record.printer_id.clone()))
        .unwrap_or_default();
    JobView {
        id,
        printer_id,
        name: info.and_then(|job| job.name.clone()),
        file_id: record.as_ref().map(|record| record.file_id.clone()),
        file_name: record.as_ref().map(|record| record.file_name.clone()),
        status: info.map(|job| job.status),
        error: info.and_then(|job| job.error.clone()),
        created_at_ms: info
            .map_or(0, |job| job.created_at_ms)
            .max(record.as_ref().map_or(0, |record| record.created_at_ms)),
        options: record.map_or_else(BTreeMap::new, |record| record.options),
    }
}

#[cfg(test)]
mod tests {
    use super::split_job_id;

    #[test]
    fn splits_hyphenated_printer_names() {
        assert_eq!(
            split_job_id("My-Office-PDF-12"),
            Some(("My-Office-PDF".to_string(), 12))
        );
        assert_eq!(split_job_id("PDF-8"), Some(("PDF".to_string(), 8)));
        assert_eq!(split_job_id("PDF-abc"), None);
        assert_eq!(split_job_id("8"), None);
    }
}
