//! MCP 服务：Streamable HTTP 端点 /mcp/{device}，一台设备一个连接。
//!
//! 终端优先的设计：MCP 只负责控制面（查设备、提交打印、管任务），文件字节由
//! 终端里的 curl 凭一次性票据推到 /uploads/{ticket}。令牌始终在 HTTP 头里，
//! 不会进入模型上下文。

pub mod backend;
pub mod tools;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::Extension;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use rmcp::transport::streamable_http_server::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::StreamableHttpServerConfig;
use tower::ServiceExt;

use self::backend::Backend;
use self::tools::McpServer;
use crate::api::auth;
use crate::error::AppError;
use crate::state::AppState;

/// 指定设备的请求头：配置里和设备名一样，是一个显式字段。
const DEVICE_HEADER: &str = "x-just-print-device";
/// 同时缓存的设备端点数上限：设备名来自请求头，必须封顶。
const MAX_DEVICES: usize = 32;
/// 设备名长度上限。
const MAX_DEVICE_LEN: usize = 64;

type DeviceService = StreamableHttpService<McpServer, LocalSessionManager>;

/// MCP 端点共享状态：每个设备一个会话管理器。
struct McpState {
    state: Arc<AppState>,
    backend: Arc<Backend>,
    services: Mutex<HashMap<String, DeviceService>>,
}

impl McpState {
    /// 取（或按需创建）某个设备的 MCP 服务；设备数超限时返回 None。
    fn service_for(&self, device: &str) -> Option<DeviceService> {
        let mut services = self.lock();
        if let Some(service) = services.get(device) {
            return Some(service.clone());
        }
        if services.len() >= MAX_DEVICES {
            return None;
        }
        let state = Arc::clone(&self.state);
        let backend = Arc::clone(&self.backend);
        let name: Arc<str> = Arc::from(device);
        let service = StreamableHttpService::new(
            move || {
                Ok(McpServer::new(
                    Arc::clone(&state),
                    Arc::clone(&backend),
                    Arc::clone(&name),
                ))
            },
            Arc::new(LocalSessionManager::default()),
            transport_config(),
        );
        services.insert(device.to_string(), service.clone());
        Some(service)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, DeviceService>> {
        self.services
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// MCP 传输配置。
fn transport_config() -> StreamableHttpServerConfig {
    StreamableHttpServerConfig::default()
        // 工具调用没有中途通知，用普通 JSON 响应比 SSE 更好调试。
        .with_json_response(true)
        // Host / Origin 白名单在反向代理与局域网自定义域名下不可靠；
        // 这里由 Bearer 令牌承担鉴权，而且服务不启用 CORS，
        // 浏览器跨站请求拿不到令牌。
        .disable_allowed_hosts()
        .disable_allowed_origins()
}

/// 处理一次 MCP 请求：设备由请求头指定，一个连接只对应一台设备。
async fn handle(
    State(_state): State<Arc<AppState>>,
    Extension(mcp): Extension<Arc<McpState>>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    let Some(raw) = headers
        .get(DEVICE_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return AppError::BadRequest(
            "缺少 X-Just-Print-Device 请求头（填写 CUPS 打印机名，例如 office）".to_string(),
        )
        .into_response();
    };
    let device = raw.trim();
    if device.is_empty() || device.len() > MAX_DEVICE_LEN || device.chars().any(char::is_control) {
        return AppError::BadRequest("X-Just-Print-Device 不合法".to_string()).into_response();
    }
    let Some(service) = mcp.service_for(device) else {
        return AppError::BadRequest("同时可用的设备端点已达上限，请稍后重试".to_string())
            .into_response();
    };
    match service.oneshot(request).await {
        Ok(response) => response.map(axum::body::Body::new),
        Err(never) => match never {},
    }
}

/// 构建 MCP 路由；内部通过已经装配好的 api 路由复用业务逻辑。
pub fn router(state: &Arc<AppState>, api: Router) -> Router {
    // 工具内部走 /api 前缀调用同一套路由，路径与对外 API 完全一致。
    let internal = Router::new().nest("/api", api);
    let mcp = Arc::new(McpState {
        backend: Arc::new(Backend::new(internal, Arc::clone(&state.config.token))),
        services: Mutex::new(HashMap::new()),
        state: Arc::clone(state),
    });
    Router::new()
        .route("/mcp", any(handle))
        .layer(Extension(mcp))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(state),
            auth::require_auth,
        ))
        .with_state(Arc::clone(state))
}
