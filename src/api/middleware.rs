//! 横切中间件：请求 id 与追踪 span、指标采集、请求超时。

use std::sync::Arc;

use axum::extract::{MatchedPath, Request, State};
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

/// 请求 id 扩展；处理函数可读取以写入审计日志。
#[derive(Debug, Clone)]
pub struct RequestId(pub Arc<str>);

impl RequestId {
    /// 以字符串形式返回请求 id。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 为每个请求生成/透传 `x-request-id`，并建立带该 id 的追踪 span。
pub async fn request_context(mut request: Request, next: Next) -> Response {
    let id = incoming_request_id(&request).unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    let id: Arc<str> = Arc::from(id);
    request.extensions_mut().insert(RequestId(Arc::clone(&id)));
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let span = tracing::info_span!(
        "http_request",
        request_id = %id,
        method = %method,
        path = %path
    );
    let mut response = next.run(request).instrument(span).await;
    if let Ok(value) = HeaderValue::from_str(&id) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-request-id"), value);
    }
    response
}

fn incoming_request_id(request: &Request) -> Option<String> {
    let value = request.headers().get("x-request-id")?.to_str().ok()?;
    let valid = !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|ch| !ch.is_control() && ch != '"');
    valid.then(|| value.to_string())
}

/// 采集 HTTP 请求计数与耗时（按方法、路由模板、状态码）。
pub async fn track(State(state): State<Arc<AppState>>, request: Request, next: Next) -> Response {
    let method = request.method().as_str().to_string();
    let route = request.extensions().get::<MatchedPath>().map_or_else(
        || request.uri().path().to_string(),
        |path| path.as_str().to_string(),
    );
    let started = std::time::Instant::now();
    state.metrics.begin_request();
    let response = next.run(request).await;
    state.metrics.end_request();
    let status = response.status().as_u16().to_string();
    state.metrics.inc(
        "http_requests",
        &[("method", &method), ("route", &route), ("status", &status)],
    );
    state.metrics.observe(
        "request_duration",
        &[("route", &route)],
        started.elapsed().as_secs_f64(),
    );
    response
}

/// 请求级超时；超时返回 `504 gateway_timeout`。
pub async fn timeout(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    tokio::time::timeout(state.config.request_timeout, next.run(request))
        .await
        .map_err(|_| AppError::GatewayTimeout("请求处理超时".to_string()))
}
