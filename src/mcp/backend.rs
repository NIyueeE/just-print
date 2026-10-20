//! MCP 工具的内部控制面：直接调用已装配的 /api 路由。
//!
//! 走内部路由而不是复制业务逻辑，是为了让 MCP 与 Web 端共享同一套鉴权、参数
//! 校验、幂等键与错误信封——两边行为不会漂移。

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use rmcp::ErrorData;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tower::ServiceExt;

/// 内部响应体读取上限：打印目录与任务列表都很小，超过说明响应异常。
const MAX_INTERNAL_BODY: usize = 8 * 1024 * 1024;

/// 内部调用器。
#[derive(Clone)]
pub struct Backend {
    app: Router,
    token: Arc<str>,
}

impl Backend {
    /// 用已装配的 /api 路由与令牌构造。
    #[must_use]
    pub fn new(app: Router, token: Arc<str>) -> Self {
        Self { app, token }
    }

    /// 发起一次内部请求，返回状态码、响应头与响应体。
    ///
    /// # Errors
    ///
    /// 请求构造失败或响应体读取失败时返回 MCP 内部错误。
    pub async fn call(
        &self,
        method: Method,
        uri: &str,
        body: Option<&Value>,
        idempotency_key: Option<&str>,
    ) -> Result<(StatusCode, HeaderMap, Vec<u8>), ErrorData> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.token));
        if let Some(key) = idempotency_key {
            builder = builder.header("idempotency-key", key);
        }
        let request = match body {
            Some(value) => {
                let bytes = serde_json::to_vec(value).map_err(|error| {
                    ErrorData::internal_error(format!("序列化请求失败: {error}"), None)
                })?;
                builder
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(bytes))
            }
            None => builder.body(Body::empty()),
        }
        .map_err(|error| ErrorData::internal_error(format!("构造内部请求失败: {error}"), None))?;

        let response = match self.app.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        };
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), MAX_INTERNAL_BODY)
            .await
            .map_err(|error| {
                ErrorData::internal_error(format!("读取内部响应失败: {error}"), None)
            })?;
        Ok((status, headers, bytes.to_vec()))
    }
}

/// 解析成功响应；非 2xx 转成带自纠提示的 MCP 错误。
///
/// # Errors
///
/// 状态码非 2xx 或响应无法反序列化时返回 [`ErrorData`]。
pub fn parse<T: DeserializeOwned>(
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<T, ErrorData> {
    if !status.is_success() {
        return Err(to_error(status, headers, body));
    }
    serde_json::from_slice(body)
        .map_err(|error| ErrorData::internal_error(format!("解析响应失败: {error}"), None))
}

/// 把 /api 的错误信封翻译成模型能自行纠正的说明。
#[must_use]
pub fn to_error(status: StatusCode, headers: &HeaderMap, body: &[u8]) -> ErrorData {
    let (code, message) = envelope(body).unwrap_or_else(|| (status_code(status), String::new()));
    let hint = match code.as_str() {
        "invalid_controls" => "（调用 get_device 查看该设备的合法选项与取值）",
        "idempotency_conflict" => "（相同内容的提交正在处理中，稍后用相同参数重试）",
        "printer_unavailable" => "（设备已停止或拒绝任务，请让用户检查打印机）",
        "service_unavailable" | "gateway_timeout" => "（打印服务暂时不可用，稍后可用相同参数重试）",
        "unauthorized" => "（令牌无效，请检查 MCP 配置里的 Authorization）",
        "not_found" => "（对象不存在或已过期；文件可能已被清理）",
        _ => "",
    };
    let retry = headers
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map_or_else(String::new, |seconds| format!("，建议 {seconds} 秒后重试"));
    let text = format!("{code}: {message}{retry}{hint}");
    match status.as_u16() {
        400 | 404 | 409 | 413 | 415 | 422 => ErrorData::invalid_params(text, None),
        _ => ErrorData::internal_error(text, None),
    }
}

fn envelope(body: &[u8]) -> Option<(String, String)> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let error = value.get("error")?;
    let code = error.get("code")?.as_str()?.to_string();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some((code, message))
}

fn status_code(status: StatusCode) -> String {
    match status.as_u16() {
        400 => "bad_request",
        401 => "unauthorized",
        404 => "not_found",
        409 => "conflict",
        413 => "payload_too_large",
        415 => "unsupported_media_type",
        422 => "conversion_failed",
        502 => "bad_gateway",
        503 => "service_unavailable",
        504 => "gateway_timeout",
        _ => "internal",
    }
    .to_string()
}
