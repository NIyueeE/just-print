//! 健康检查、就绪探针与 Prometheus 指标端点。

use std::sync::Arc;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// 存活探针：进程能响应即返回 `ok`（不依赖 CUPS）。
pub async fn healthz() -> &'static str {
    "ok"
}

/// 就绪探针：确认 CUPS 能在探针预算内响应 IPP 请求。
pub async fn readyz(state: Arc<AppState>) -> Response {
    match tokio::time::timeout(state.config.ready_timeout, state.cups.ready()).await {
        Ok(Ok(())) => {
            state.metrics.inc("readiness_checks", &[("outcome", "ok")]);
            (StatusCode::OK, "ready").into_response()
        }
        Ok(Err(error)) => {
            state
                .metrics
                .inc("readiness_checks", &[("outcome", "error")]);
            tracing::warn!(error = %error, "就绪探针失败");
            (StatusCode::SERVICE_UNAVAILABLE, "cups unavailable").into_response()
        }
        Err(_) => {
            state
                .metrics
                .inc("readiness_checks", &[("outcome", "timeout")]);
            tracing::warn!(
                timeout_secs = state.config.ready_timeout.as_secs(),
                "就绪探针超时"
            );
            (StatusCode::SERVICE_UNAVAILABLE, "cups unavailable").into_response()
        }
    }
}

/// Prometheus 文本格式指标。
pub async fn metrics(State(state): State<Arc<AppState>>) -> Response {
    state.metrics.set_gauge(
        "uploaded_files",
        &[],
        i64::try_from(state.files.len()).unwrap_or(i64::MAX),
    );
    state.metrics.set_gauge(
        "uploaded_bytes",
        &[],
        i64::try_from(state.files.total_bytes()).unwrap_or(i64::MAX),
    );
    state.metrics.set_gauge(
        "registered_jobs",
        &[],
        i64::try_from(state.jobs.len()).unwrap_or(i64::MAX),
    );
    state.metrics.set_gauge(
        "idempotency_entries",
        &[],
        i64::try_from(state.idempotency.len()).unwrap_or(i64::MAX),
    );
    state.metrics.set_gauge(
        "active_tickets",
        &[],
        i64::try_from(state.tickets.len()).unwrap_or(i64::MAX),
    );
    state.metrics.set_gauge(
        "in_flight_print_tasks",
        &[],
        i64::try_from(state.print_tasks.len()).unwrap_or(i64::MAX),
    );
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics.render(),
    )
        .into_response()
}
