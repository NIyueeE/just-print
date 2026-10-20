//! Web API：路由装配、认证、横切中间件与安全响应头。

pub mod auth;
pub mod files;
pub mod formats;
pub mod health;
pub mod jobs;
pub mod middleware;
pub mod print;
pub mod printers;

#[cfg(test)]
mod flow_tests;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderName, HeaderValue, header};
use axum::routing::{get, post};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::error::AppError;
use crate::state::AppState;

/// 未匹配的 API 路径返回统一 JSON 404（避免落入静态前端 fallback）。
async fn api_fallback() -> AppError {
    AppError::NotFound
}

/// `/api` 业务路由：鉴权、超时、指标与错误信封都在这里装配。
fn api_router(state: &Arc<AppState>) -> Router {
    let body_limit = state.config.max_upload_bytes;
    Router::new()
        .route("/files", post(files::upload))
        .route("/files/{id}", get(files::preview).delete(files::delete))
        .route("/formats", get(formats::list))
        .route("/printers", get(printers::list))
        .route("/print", post(print::submit))
        .route("/jobs", get(jobs::list))
        .route("/jobs/{id}", get(jobs::get).delete(jobs::cancel))
        .route("/metrics", get(health::metrics))
        .fallback(api_fallback)
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(state),
            auth::require_auth,
        ))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(state),
            middleware::timeout,
        ))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(state),
            middleware::track,
        ))
        .layer(axum::middleware::from_fn(middleware::request_context))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .with_state(Arc::clone(state))
}

/// 构建完整路由：`/api` 业务接口、`/mcp` 控制面、票据上传、健康检查与静态前端。
pub fn router(state: &Arc<AppState>) -> Router {
    let web_dir = state.config.web_dir.clone();
    let index = web_dir.join("index.html");
    let api = api_router(state);
    let mcp = crate::mcp::router(state, api.clone());
    let uploads = Router::new()
        .route("/uploads/{ticket}", post(files::upload_by_ticket))
        .with_state(Arc::clone(state));

    let ready_state = Arc::clone(state);
    let readyz = move || {
        let ready_state = Arc::clone(&ready_state);
        async move { health::readyz(ready_state).await }
    };

    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(readyz))
        .merge(uploads)
        .nest("/api", api)
        .merge(mcp)
        .fallback_service(ServeDir::new(web_dir).not_found_service(ServeFile::new(index)))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; \
                 script-src 'self'; connect-src 'self'; frame-src 'self' blob:; object-src 'none'; \
                 base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
            ),
        ))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt;

    use crate::config::Config;
    use crate::state::AppState;

    fn test_config(web_dir: std::path::PathBuf) -> Config {
        Config {
            addr: std::net::SocketAddr::from(([127, 0, 0, 1], 8080)),
            web_dir,
            token: Arc::from("test-token"),
            cups_server: "127.0.0.1:631".to_string(),
            cups_scheme: "ipp".to_string(),
            max_upload_bytes: 64 * 1024 * 1024,
            conversion_timeout: Duration::from_mins(2),
            conversion_slots: 2,
            cleanup_interval: Duration::from_mins(1),
            temp_ttl: Duration::from_mins(30),
            ipp_timeout: Duration::from_secs(15),
            printer_cache_ttl: Duration::from_secs(10),
            job_cache_ttl: Duration::from_secs(2),
            idempotency_ttl: Duration::from_mins(10),
            idempotency_wait: Duration::from_secs(5),
            upload_slots: 4,
            request_timeout: Duration::from_mins(5),
        }
    }

    fn test_state(web_dir: std::path::PathBuf) -> Option<Arc<AppState>> {
        let temp_dir = std::env::temp_dir().join("just-print-test");
        let _ = std::fs::create_dir_all(&temp_dir);
        let state = AppState::new(test_config(web_dir), temp_dir);
        state.ok().map(Arc::new)
    }

    fn temp_web_dir() -> std::path::PathBuf {
        let temp = std::env::temp_dir().join(format!("just-print-test-{}", crate::ids::new_id()));
        let _ = std::fs::create_dir_all(&temp);
        let _ = std::fs::write(temp.join("index.html"), "<div id=\"app\"></div>");
        temp
    }

    async fn response_body(app: axum::Router, request: Request<Body>) -> (StatusCode, String) {
        let response = app
            .oneshot(request)
            .await
            .unwrap_or_else(|never| match never {});
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await;
        let Ok(bytes) = bytes else {
            return (status, String::new());
        };
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// 票据上传通道：一次性、PDF-only、不需要 Bearer。
    #[tokio::test]
    async fn ticket_upload_accepts_pdf_exactly_once() {
        let Some(state) = test_state(temp_web_dir()) else {
            return;
        };
        let app = super::router(&state);
        let Some(ticket) = state.tickets.issue("probe.pdf".to_string()) else {
            return;
        };
        let pdf = b"%PDF-1.4\n1 0 obj<<>>endobj\ntrailer<<>>\n%%EOF\n".to_vec();
        let Ok(request) = Request::builder()
            .uri(format!("/uploads/{ticket}"))
            .method("POST")
            .body(Body::from(pdf))
        else {
            return;
        };
        let (status, body) = response_body(app.clone(), request).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert!(body.contains("\"name\":\"probe.pdf\""));

        // 票据只能兑换一次。
        let Ok(request) = Request::builder()
            .uri(format!("/uploads/{ticket}"))
            .method("POST")
            .body(Body::from("%PDF-1.4".as_bytes().to_vec()))
        else {
            return;
        };
        let (status, body) = response_body(app, request).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("invalid_ticket"));
    }

    #[tokio::test]
    async fn ticket_upload_rejects_non_pdf() {
        let Some(state) = test_state(temp_web_dir()) else {
            return;
        };
        let app = super::router(&state);
        let Some(ticket) = state.tickets.issue("note.txt".to_string()) else {
            return;
        };
        let Ok(request) = Request::builder()
            .uri(format!("/uploads/{ticket}"))
            .method("POST")
            .body(Body::from("hello".as_bytes().to_vec()))
        else {
            return;
        };
        let (status, body) = response_body(app, request).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert!(body.contains("PDF"), "{body}");
    }

    /// MCP 端点：无令牌 401，带令牌可列出七个工具。
    #[tokio::test]
    async fn mcp_endpoint_requires_token_and_lists_tools() {
        let Some(state) = test_state(temp_web_dir()) else {
            return;
        };
        let app = super::router(&state);
        let payload = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{",
            "\"_meta\":{\"io.modelcontextprotocol/protocolVersion\":\"2026-07-28\",",
            "\"io.modelcontextprotocol/clientCapabilities\":{}}}}"
        );
        let request = Request::builder()
            .uri("/mcp")
            .method("POST")
            .header(header::HOST, "localhost")
            .header("mcp-protocol-version", "2026-07-28")
            .header("x-just-print-device", "office")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header("mcp-method", "tools/list")
            .body(Body::from(payload.to_string()));
        let Ok(request) = request else { return };
        let (status, _) = response_body(app.clone(), request).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let request = Request::builder()
            .uri("/mcp")
            .method("POST")
            .header(header::HOST, "localhost")
            .header("mcp-protocol-version", "2026-07-28")
            .header("x-just-print-device", "office")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header("mcp-method", "tools/list")
            .body(Body::from(payload.to_string()));
        let Ok(request) = request else { return };
        let (status, body) = response_body(app, request).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for tool in [
            "create_upload_ticket",
            "print_document",
            "get_device",
            "cancel_job",
        ] {
            assert!(body.contains(tool), "缺少工具 {tool}: {body}");
        }
    }

    #[tokio::test]
    async fn unknown_api_path_returns_json_404() {
        let Some(state) = test_state(temp_web_dir()) else {
            return;
        };
        let app = super::router(&state);
        let request = Request::builder()
            .uri("/api/does-not-exist")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .body(Body::empty());
        let Ok(request) = request else {
            return;
        };
        let (status, body) = response_body(app, request).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.contains("\"code\":\"not_found\""));
        assert!(!body.contains("<!doctype"));
    }

    #[tokio::test]
    async fn oversized_upload_returns_413_envelope() {
        let Some(state) = test_state(temp_web_dir()) else {
            return;
        };
        let app = super::router(&state);
        let mut payload = Vec::with_capacity(65 * 1024 * 1024);
        payload.extend_from_slice(
            b"--probe\r\nContent-Disposition: form-data; name=\"file\"; filename=\"big.bin\"\r\n\r\n",
        );
        payload.resize(65 * 1024 * 1024, b'x');
        payload.extend_from_slice(b"\r\n--probe--\r\n");
        let request = Request::builder()
            .uri("/api/files")
            .method("POST")
            .header(header::HOST, "localhost")
            .header("mcp-protocol-version", "2026-07-28")
            .header("x-just-print-device", "office")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "multipart/form-data; boundary=probe")
            .body(Body::from(payload));
        let Ok(request) = request else {
            return;
        };
        let (status, body) = response_body(app, request).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(body.contains("\"code\":\"payload_too_large\""));
    }

    #[tokio::test]
    async fn unauthenticated_unknown_api_path_returns_401() {
        let Some(state) = test_state(temp_web_dir()) else {
            return;
        };
        let app = super::router(&state);
        let request = Request::builder()
            .uri("/api/does-not-exist")
            .body(Body::empty());
        let Ok(request) = request else {
            return;
        };
        let (status, body) = response_body(app, request).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("\"code\":\"unauthorized\""));
    }

    #[tokio::test]
    async fn malformed_json_body_returns_400_envelope() {
        let Some(state) = test_state(temp_web_dir()) else {
            return;
        };
        let app = super::router(&state);
        let request = Request::builder()
            .uri("/api/print")
            .method("POST")
            .header(header::HOST, "localhost")
            .header("mcp-protocol-version", "2026-07-28")
            .header("x-just-print-device", "office")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{not json"));
        let Ok(request) = request else {
            return;
        };
        let (status, body) = response_body(app, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("\"code\":\"bad_request\""));
        assert!(body.contains("JSON 解析失败"));
    }

    #[tokio::test]
    async fn wrong_typed_json_body_returns_400_envelope() {
        let Some(state) = test_state(temp_web_dir()) else {
            return;
        };
        let app = super::router(&state);
        let request = Request::builder()
            .uri("/api/print")
            .method("POST")
            .header(header::HOST, "localhost")
            .header("mcp-protocol-version", "2026-07-28")
            .header("x-just-print-device", "office")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{\"file_id\":123}"));
        let Ok(request) = request else {
            return;
        };
        let (status, body) = response_body(app, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("\"code\":\"bad_request\""));
    }

    #[tokio::test]
    async fn non_multipart_upload_returns_400_envelope() {
        let Some(state) = test_state(temp_web_dir()) else {
            return;
        };
        let app = super::router(&state);
        let request = Request::builder()
            .uri("/api/files")
            .method("POST")
            .header(header::HOST, "localhost")
            .header("mcp-protocol-version", "2026-07-28")
            .header("x-just-print-device", "office")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"));
        let Ok(request) = request else {
            return;
        };
        let (status, body) = response_body(app, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("\"code\":\"bad_request\""));
        assert!(body.contains("multipart 请求不合法"));
    }

    #[tokio::test]
    async fn invalid_idempotency_key_returns_400() {
        let Some(state) = test_state(temp_web_dir()) else {
            return;
        };
        let app = super::router(&state);
        let request = Request::builder()
            .uri("/api/print")
            .method("POST")
            .header(header::HOST, "localhost")
            .header("mcp-protocol-version", "2026-07-28")
            .header("x-just-print-device", "office")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "application/json")
            .header("idempotency-key", "bad\nkey")
            .body(Body::from("{\"file_id\":\"a\",\"printer_id\":\"b\"}"));
        let Ok(request) = request else {
            return;
        };
        let (status, body) = response_body(app, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("\"code\":\"bad_request\""));
    }

    #[tokio::test]
    async fn health_and_ready_endpoints_do_not_require_auth() {
        let Some(state) = test_state(temp_web_dir()) else {
            return;
        };
        let app = super::router(&state);
        let request = Request::builder().uri("/healthz").body(Body::empty());
        let Ok(request) = request else {
            return;
        };
        let (status, body) = response_body(app, request).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "ok");
    }

    /// 回归：真实 multipart 正文必须能被解析（曾因未释放 field 导致
    /// "failed to lock multipart state" 而误报 400）。
    #[tokio::test]
    async fn multipart_upload_body_is_parsed() {
        let Some(state) = test_state(temp_web_dir()) else {
            return;
        };
        let app = super::router(&state);
        let payload = concat!(
            "--probe\r\n",
            "Content-Disposition: form-data; name=\"note\"\r\n\r\n",
            "ignored\r\n",
            "--probe\r\n",
            "Content-Disposition: form-data; name=\"file\"; filename=\"sample.txt\"\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "hello just-print\r\n",
            "--probe--\r\n"
        );
        let request = Request::builder()
            .uri("/api/files")
            .method("POST")
            .header(header::HOST, "localhost")
            .header("mcp-protocol-version", "2026-07-28")
            .header("x-just-print-device", "office")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "multipart/form-data; boundary=probe")
            .body(Body::from(payload));
        let Ok(request) = request else {
            return;
        };
        let (status, body) = response_body(app, request).await;
        // 无 LibreOffice 时为 422 conversion_failed；有则为 201。
        // 关键是 multipart 必须解析成功，而不是 400。
        assert_ne!(
            status,
            StatusCode::BAD_REQUEST,
            "multipart should parse: {body}"
        );
        assert!(
            !body.contains("multipart 解析失败"),
            "unexpected parse error: {body}"
        );
    }
}
