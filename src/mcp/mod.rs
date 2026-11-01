//! MCP 服务：Streamable HTTP 端点 /mcp/{device}，一台设备一个连接。
//!
//! 终端优先的设计：MCP 只负责控制面（查设备、提交打印、管任务），文件字节由
//! 终端里的 curl 凭一次性票据推到 /uploads/{ticket}。令牌始终在 HTTP 头里，
//! 不会进入模型上下文。

pub mod backend;
pub mod tools;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::Extension;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use rmcp::transport::streamable_http_server::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::StreamableHttpServerConfig;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use self::backend::Backend;
use self::tools::McpServer;
use crate::api::{auth, middleware};
use crate::cups::is_valid_printer_name;
use crate::error::AppError;
use crate::state::AppState;

/// 指定设备的请求头：配置里和设备名一样，是一个显式字段。
const DEVICE_HEADER: &str = "x-just-print-device";
/// 同时缓存的设备端点数上限：设备名来自请求头，必须封顶。
const MAX_DEVICES: usize = 32;
/// 设备名长度上限。
const MAX_DEVICE_LEN: usize = 64;
/// 空闲设备服务的回收时长；超过后下一次容量检查会关闭其会话。
const DEVICE_IDLE_TTL: Duration = Duration::from_mins(30);

type DeviceService = StreamableHttpService<McpServer, LocalSessionManager>;

/// 单个设备服务及其使用状态。
struct DeviceEntry {
    service: DeviceService,
    last_used: Instant,
    in_flight: usize,
    cancellation: CancellationToken,
}

/// 一次设备请求的租约；保证容量淘汰不会选中正在处理的条目。
struct DeviceLease {
    service: DeviceService,
    device: String,
    cache: Arc<McpState>,
}

impl Drop for DeviceLease {
    fn drop(&mut self) {
        if let Some(entry) = self.cache.lock().get_mut(&self.device) {
            entry.in_flight = entry.in_flight.saturating_sub(1);
            entry.last_used = Instant::now();
        }
    }
}

/// MCP 端点共享状态：每个设备一个会话管理器。
struct McpState {
    state: Arc<AppState>,
    backend: Arc<Backend>,
    services: Mutex<HashMap<String, DeviceEntry>>,
}

impl McpState {
    /// 取（或按需创建）某个设备的 MCP 服务。
    ///
    /// 超过容量时优先回收空闲最久且没有在途请求的设备；全部都在处理时
    /// 返回 `None`，由调用方给出可重试的 503，而不是永久拒绝新设备。
    fn service_for(self: &Arc<Self>, device: &str) -> Option<DeviceLease> {
        let mut services = self.lock();
        let now = Instant::now();
        let expired: Vec<String> = services
            .iter()
            .filter(|(_, entry)| {
                entry.in_flight == 0 && now.duration_since(entry.last_used) >= DEVICE_IDLE_TTL
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in expired {
            if let Some(entry) = services.remove(&name) {
                entry.cancellation.cancel();
            }
        }
        if let Some(entry) = services.get_mut(device) {
            entry.in_flight += 1;
            entry.last_used = now;
            return Some(DeviceLease {
                service: entry.service.clone(),
                device: device.to_string(),
                cache: Arc::clone(self),
            });
        }
        if services.len() >= MAX_DEVICES {
            let oldest = services
                .iter()
                .filter(|(_, entry)| entry.in_flight == 0)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(name, _)| name.clone());
            if let Some(name) = oldest
                && let Some(entry) = services.remove(&name)
            {
                entry.cancellation.cancel();
            } else {
                self.refresh_gauge(&services);
                return None;
            }
        }

        let cancellation = CancellationToken::new();
        let service = self.build_service(device, cancellation.clone());
        services.insert(
            device.to_string(),
            DeviceEntry {
                service: service.clone(),
                last_used: now,
                in_flight: 1,
                cancellation,
            },
        );
        self.refresh_gauge(&services);
        Some(DeviceLease {
            service,
            device: device.to_string(),
            cache: Arc::clone(self),
        })
    }

    fn build_service(&self, device: &str, cancellation: CancellationToken) -> DeviceService {
        let state = Arc::clone(&self.state);
        let backend = Arc::clone(&self.backend);
        let name: Arc<str> = Arc::from(device);
        StreamableHttpService::new(
            move || {
                Ok(McpServer::new(
                    Arc::clone(&state),
                    Arc::clone(&backend),
                    Arc::clone(&name),
                ))
            },
            Arc::new(LocalSessionManager::default()),
            transport_config(cancellation),
        )
    }

    fn refresh_gauge(&self, services: &HashMap<String, DeviceEntry>) {
        self.state.metrics.set_gauge(
            "mcp_devices",
            &[],
            i64::try_from(services.len()).unwrap_or(i64::MAX),
        );
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, DeviceEntry>> {
        self.services
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// MCP 传输配置。
fn transport_config(cancellation: CancellationToken) -> StreamableHttpServerConfig {
    StreamableHttpServerConfig::default()
        // 工具调用没有中途通知，用普通 JSON 响应比 SSE 更好调试。
        .with_json_response(true)
        .with_cancellation_token(cancellation)
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
    if device.is_empty() || device.len() > MAX_DEVICE_LEN || !is_valid_printer_name(device) {
        return AppError::BadRequest(
            "X-Just-Print-Device 不合法：只能包含字母、数字、点、下划线和连字符".to_string(),
        )
        .into_response();
    }
    let Some(lease) = mcp.service_for(device) else {
        return AppError::ServiceUnavailable("同时处理的设备端点已达上限，请稍后重试".to_string())
            .into_response();
    };
    let started = Instant::now();
    match lease.service.clone().oneshot(request).await {
        Ok(response) => {
            let status = response.status().as_u16().to_string();
            mcp.state
                .metrics
                .inc("mcp_requests", &[("status", &status)]);
            mcp.state
                .metrics
                .observe("mcp_request_duration", &[], started.elapsed().as_secs_f64());
            response.map(axum::body::Body::new)
        }
        Err(never) => match never {},
    }
}

/// 构建 MCP 路由；内部通过已经装配好的 api 路由复用业务逻辑。
pub fn router(state: &Arc<AppState>, api: Router) -> Router {
    // 工具内部走 /api 前缀调用同一套路由，路径与对外 API 完全一致。
    // 对外请求的 request_context 位于根路由，不会进入这里的 oneshot 调用；
    // 内部路由必须自行注入 RequestId，否则需要该扩展的 handler（如
    // `/api/print`）会在提取阶段直接返回空 500。
    let internal = Router::new()
        .nest("/api", api)
        .layer(axum::middleware::from_fn(middleware::request_context));
    let mcp = Arc::new(McpState {
        backend: Arc::new(Backend::new(internal, Arc::clone(&state.config.token))),
        services: Mutex::new(HashMap::new()),
        state: Arc::clone(state),
    });
    Router::new()
        .route("/mcp", any(handle))
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(state),
            auth::require_auth,
        ))
        .layer(Extension(mcp))
        .with_state(Arc::clone(state))
}
