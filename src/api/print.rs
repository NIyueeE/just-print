//! 提交打印任务（标准 `Idempotency-Key` 语义）。

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::middleware::RequestId;
use crate::cups::{JobStatus, PrinterStateView, encode_job_attributes};
use crate::error::AppError;
use crate::idempotency::{Begin, IdempotencyStore, validate_key};
use crate::jobs::{JobRecord, now_ms};
use crate::state::AppState;

/// 打印请求。
#[derive(Debug, Deserialize)]
pub struct PrintRequest {
    /// 已上传文件 id。
    pub file_id: String,
    /// 目标 CUPS 打印机名。
    pub printer_id: String,
    /// 标准 IPP job 模板选项（可选，键/值必须来自打印机选项目录）。
    #[serde(default)]
    pub options: std::collections::BTreeMap<String, String>,
}

/// 打印提交响应。
#[derive(Debug, Clone, Serialize)]
pub struct PrintResponse {
    /// 应用层任务 id：`<printer>-<job-id>`。
    pub job_id: String,
    /// 目标打印机名。
    pub printer_id: String,
    /// 提交后的任务状态。
    pub status: JobStatus,
    /// 创建时间（Unix 毫秒）。
    pub created_at_ms: u64,
    /// 原始文件名。
    pub file_name: String,
}

/// 提交任务给 CUPS；带 `Idempotency-Key` 时保证同一键只产生一个任务。
///
/// 真正的提交在独立任务（[`run_submission`]）里执行：客户端断开——页面挂久后
/// 连接失效、浏览器超时中断、用户关掉标签页——只会丢弃等待，不会取消已经受理的
/// `Print-Job`。否则幂等键会永久停在「进行中」，之后所有同键重试都被 409 拒绝，
/// 用户看到的就是「任务提交不上去」，尽管打印其实已经成功。
pub async fn submit(
    State(state): State<Arc<AppState>>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    body: Result<Json<PrintRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let Json(request) = body
        .map_err(|error| AppError::BadRequest(format!("JSON 解析失败: {}", error.body_text())))?;

    let key = match headers.get("idempotency-key") {
        Some(value) => {
            let value = value
                .to_str()
                .map_err(|_| AppError::BadRequest("Idempotency-Key 不是合法字符串".to_string()))?;
            validate_key(value).map_err(AppError::BadRequest)?;
            Some(value.to_string())
        }
        None => None,
    };

    let options_json = serde_json::to_string(&request.options)
        .map_err(|error| AppError::Internal(format!("序列化选项失败: {error}")))?;
    let fingerprint = format!("{}|{}|{options_json}", request.file_id, request.printer_id);

    // 同键同负载的重复请求等在途请求的结果：成功回放（不重复出纸），失败接管重试。
    let token = match key.as_deref() {
        Some(key) => match state
            .idempotency
            .begin_or_wait(key, &fingerprint, state.config.idempotency_wait)
            .await
        {
            Begin::Fresh { token } => Some(token),
            Begin::Replay(value) => {
                state.metrics.inc("print_jobs", &[("outcome", "replayed")]);
                return Ok(replayed(value));
            }
            Begin::Conflict => {
                return Err(AppError::IdempotencyConflict(
                    "Idempotency-Key 已用于不同的请求".to_string(),
                ));
            }
            Begin::InProgress => {
                return Err(AppError::IdempotencyConflict(
                    "相同 Idempotency-Key 的请求仍在处理中，请稍后重试".to_string(),
                ));
            }
        },
        None => None,
    };

    let task = tokio::spawn(run_submission(
        Arc::clone(&state),
        request,
        key,
        fingerprint,
        token,
        request_id,
    ));
    match task.await {
        Ok(Ok(response)) => Ok((StatusCode::ACCEPTED, Json(response)).into_response()),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(AppError::Internal("打印任务执行中断".to_string())),
    }
}

/// 执行一次打印提交；生命周期与客户端连接无关（由 [`submit`] 派生为独立任务）。
async fn run_submission(
    state: Arc<AppState>,
    request: PrintRequest,
    key: Option<String>,
    fingerprint: String,
    token: Option<String>,
    request_id: RequestId,
) -> Result<PrintResponse, AppError> {
    // 兜底：任务被 panic 或运行时关闭打断时，不留下永远「进行中」的幂等键。
    let _pending = PendingKey {
        store: &state.idempotency,
        attempt: key.clone().zip(token.clone()),
    };

    let response = match submit_inner(&state, &request, token.as_deref(), &request_id).await {
        Ok(response) => response,
        Err(error) => {
            let reconciled = if token.is_some()
                && matches!(
                    &error,
                    AppError::GatewayTimeout(_) | AppError::ServiceUnavailable(_)
                ) {
                reconcile(&state, &request, token.as_deref()).await
            } else {
                None
            };
            if let Some(response) = reconciled {
                response
            } else {
                if let (Some(key), Some(token)) = (key.as_deref(), token.as_deref()) {
                    state.idempotency.abort(key, token);
                }
                return Err(error);
            }
        }
    };

    if let (Some(key), Some(_)) = (key.as_deref(), token.as_deref()) {
        let value = serde_json::to_value(&response)
            .map_err(|error| AppError::Internal(format!("序列化响应失败: {error}")))?;
        state.idempotency.complete(key, &fingerprint, value);
    }
    Ok(response)
}

/// 幂等键兜底守卫：任务提前结束（panic / 运行时关闭）时释放本次尝试占用的
/// 「进行中」条目，让同键重试可以接管，而不是被永久拒绝。释放带 `token`，
/// 因此不会误删已经接管的新尝试；提交成功后条目已是「已完成」，同样不受影响。
struct PendingKey<'a> {
    store: &'a IdempotencyStore,
    attempt: Option<(String, String)>,
}

impl Drop for PendingKey<'_> {
    fn drop(&mut self) {
        if let Some((key, token)) = self.attempt.take() {
            self.store.abort(&key, &token);
        }
    }
}

async fn submit_inner(
    state: &Arc<AppState>,
    request: &PrintRequest,
    token: Option<&str>,
    request_id: &RequestId,
) -> Result<PrintResponse, AppError> {
    let printer = state
        .cups
        .find_printer(&request.printer_id)
        .await
        .map_err(AppError::from)?
        .ok_or(AppError::PrinterNotFound)?;
    if !printer.accepting_jobs || printer.state == PrinterStateView::Stopped {
        return Err(AppError::PrinterUnavailable(format!(
            "打印机 {} 当前不接受任务",
            printer.display_name
        )));
    }

    let attributes = encode_job_attributes(&printer.options, &request.options)
        .map_err(|error| AppError::InvalidControls(error.to_string()))?;
    let file = state
        .files
        .guard(&request.file_id)
        .ok_or(AppError::FileNotFound)?;
    let file_name = file.name().to_string();
    // 标记放在最前面，保证长文件名被截断到 255 字节后仍可用于对账。
    let title = match token {
        Some(token) => format!("[{}] {file_name}", token_suffix(token)),
        None => file_name.clone(),
    };

    let submitted = state
        .cups
        .submit(&printer, file.path(), &title, attributes)
        .await;
    state.metrics.inc(
        "ipp_requests",
        &[
            ("operation", "print-job"),
            ("outcome", if submitted.is_ok() { "ok" } else { "error" }),
        ],
    );
    let job = submitted.map_err(AppError::from)?;
    let created_at_ms = if job.created_at_ms == 0 {
        now_ms()
    } else {
        job.created_at_ms
    };
    state.jobs.insert(JobRecord {
        id: job.id(),
        file_id: request.file_id.clone(),
        file_name: file_name.clone(),
        printer_id: printer.name.clone(),
        created_at_ms,
        options: request.options.clone(),
    });
    state.metrics.inc("print_jobs", &[("outcome", "submitted")]);
    tracing::info!(
        request_id = %request_id.as_str(),
        job_id = %job.id(),
        printer = %printer.name,
        file = %file_name,
        options = ?request.options,
        "打印任务已提交"
    );
    Ok(PrintResponse {
        job_id: job.id(),
        printer_id: printer.name.clone(),
        status: job.status,
        created_at_ms,
        file_name,
    })
}

/// 提交超时/连接失败后，按写入 `job-name` 的标记在 CUPS 中找回已创建的任务。
async fn reconcile(
    state: &Arc<AppState>,
    request: &PrintRequest,
    token: Option<&str>,
) -> Option<PrintResponse> {
    let token = token?;
    let marker = format!("[{}]", token_suffix(token));
    let jobs = state.cups.list_jobs(&request.printer_id).await.ok()?;
    let job = jobs.into_iter().find(|job| {
        job.name
            .as_deref()
            .is_some_and(|name| name.starts_with(&marker))
    })?;
    let created_at_ms = if job.created_at_ms == 0 {
        now_ms()
    } else {
        job.created_at_ms
    };
    let file_name = state
        .files
        .name(&request.file_id)
        .unwrap_or_else(|| "document.pdf".to_string());
    state.jobs.insert(JobRecord {
        id: job.id(),
        file_id: request.file_id.clone(),
        file_name: file_name.clone(),
        printer_id: request.printer_id.clone(),
        created_at_ms,
        options: request.options.clone(),
    });
    state
        .metrics
        .inc("print_jobs", &[("outcome", "reconciled")]);
    tracing::warn!(job_id = %job.id(), "提交响应丢失，已通过 job-name 对账找回任务");
    Some(PrintResponse {
        job_id: job.id(),
        printer_id: request.printer_id.clone(),
        status: job.status,
        created_at_ms,
        file_name,
    })
}

fn replayed(value: Value) -> Response {
    (
        StatusCode::ACCEPTED,
        [(
            HeaderName::from_static("idempotency-replayed"),
            "true".to_string(),
        )],
        Json(value),
    )
        .into_response()
}

fn token_suffix(token: &str) -> &str {
    token.get(..8).unwrap_or(token)
}

#[cfg(test)]
mod tests {
    use super::token_suffix;

    #[test]
    fn token_suffix_is_bounded() {
        assert_eq!(token_suffix("0123456789abcdef"), "01234567");
        assert_eq!(token_suffix("abc"), "abc");
    }
}
