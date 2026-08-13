//! Web API：路由装配与 Bearer 令牌保护。

pub mod auth;
pub mod files;
pub mod jobs;
pub mod print;
pub mod printers;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{get, post};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::error::AppError;
use crate::state::AppState;

/// 未匹配的 API 路径返回统一 JSON 404（避免落入静态前端 fallback）。
async fn api_fallback() -> AppError {
    AppError::NotFound
}

/// 构建完整路由：`/api` 前缀业务接口 + 静态前端 fallback。
pub fn router(state: Arc<AppState>) -> Router {
    let web_dir = state.config.web_dir.clone();
    let index = web_dir.join("index.html");
    let api = Router::new()
        .route("/files", post(files::upload))
        .route("/files/{id}", get(files::preview))
        .route("/printers", get(printers::list))
        .route("/print", post(print::submit))
        .route("/jobs/{id}", get(jobs::get))
        .fallback(api_fallback)
        .layer(DefaultBodyLimit::max(state.config.max_upload_bytes))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth::require_auth,
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .with_state(state);
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api", api)
        .fallback_service(ServeDir::new(web_dir).not_found_service(ServeFile::new(index)))
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

    fn test_state(web_dir: std::path::PathBuf) -> Arc<AppState> {
        let config = Config {
            addr: std::net::SocketAddr::from(([127, 0, 0, 1], 8080)),
            web_dir,
            token: Arc::from("test-token"),
            cups_server: "127.0.0.1:631".to_string(),
            cups_scheme: "ipp".to_string(),
            max_upload_bytes: 64 * 1024 * 1024,
            conversion_timeout: Duration::from_mins(2),
            cleanup_interval: Duration::from_mins(1),
            temp_ttl: Duration::from_mins(30),
        };
        Arc::new(AppState::new(
            config,
            std::env::temp_dir().join("just-print-test"),
        ))
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

    #[tokio::test]
    async fn unknown_api_path_returns_json_404() {
        let app = super::router(test_state(temp_web_dir()));
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
        let app = super::router(test_state(temp_web_dir()));
        let mut payload = Vec::with_capacity(65 * 1024 * 1024);
        payload.extend_from_slice(
            b"--probe\r\nContent-Disposition: form-data; name=\"file\"; filename=\"big.bin\"\r\n\r\n",
        );
        payload.resize(65 * 1024 * 1024, b'x');
        payload.extend_from_slice(b"\r\n--probe--\r\n");
        let request = Request::builder()
            .uri("/api/files")
            .method("POST")
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
        let app = super::router(test_state(temp_web_dir()));
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
        let app = super::router(test_state(temp_web_dir()));
        let request = Request::builder()
            .uri("/api/print")
            .method("POST")
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
        let app = super::router(test_state(temp_web_dir()));
        let request = Request::builder()
            .uri("/api/print")
            .method("POST")
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
        let app = super::router(test_state(temp_web_dir()));
        let request = Request::builder()
            .uri("/api/files")
            .method("POST")
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
}
