//! MCP 工具：一个连接绑定一台设备，每个工具只做一件事。

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::{Method, header};
use rmcp::ErrorData;
use rmcp::RoleServer;
use rmcp::ServerHandler;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::mcp::backend::{self, Backend};
use crate::state::AppState;
use crate::tickets::TICKET_TTL;

/// 对外声明的服务名。
const SERVER_NAME: &str = "just-print";
/// 单次列任务的上限，与 API 契约一致。
const MAX_JOB_LIMIT: u32 = 200;
/// 文件名长度上限。
const MAX_NAME_LEN: usize = 128;

/// 设备视图。
#[derive(Debug, Serialize, JsonSchema)]
pub struct DeviceView {
    /// 绑定设备名。
    pub device: String,
    /// idle / printing / stopped / disabled。
    pub state: String,
    /// 是否接受新任务。
    pub accepting_jobs: bool,
    /// 型号。
    pub make_and_model: Option<String>,
    /// 位置。
    pub location: Option<String>,
    /// IPP 选项目录；options 的键与值必须取自这里。
    pub options: Value,
    /// 选项目录读取失败时的原因。
    pub options_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Printer {
    id: String,
    state: String,
    accepting_jobs: bool,
    make_and_model: Option<String>,
    location: Option<String>,
    #[serde(default)]
    options: Value,
    options_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PrinterList {
    printers: Vec<Printer>,
}

/// 票据参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TicketArgs {
    /// 上传文件的展示名，例如 report.pdf。
    pub filename: String,
}

/// 票据结果。
#[derive(Debug, Serialize, JsonSchema)]
pub struct TicketView {
    /// 一次性上传地址，5 分钟内有效、只能使用一次。
    pub upload_url: String,
    /// 可直接执行的命令模板：把 {path} 换成本机文件路径。
    pub curl: String,
    /// 剩余有效期（秒）。
    pub expires_in_secs: u64,
}

/// 打印参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrintArgs {
    /// 上传接口返回的 `file_id`。
    pub file_id: String,
    /// 打印选项；键与值必须来自 `get_device` 返回的 options。
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

/// 打印结果。
#[derive(Debug, Serialize, JsonSchema)]
pub struct PrintView {
    /// 任务 id，形如 office-8。
    pub job_id: String,
    /// 设备名。
    pub device: String,
    /// 任务状态。
    pub status: Option<String>,
    /// 文件名。
    pub file_name: String,
    /// 创建时间（毫秒时间戳）。
    pub created_at_ms: u64,
    /// 是否回放了已经受理过的相同提交。
    pub replayed: bool,
    /// 给模型读的补充说明。
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PrintAccepted {
    job_id: String,
    printer_id: String,
    status: Option<String>,
    created_at_ms: u64,
    file_name: String,
}

/// 列任务参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListJobsArgs {
    /// 返回条数上限，1-200，默认 20。
    pub limit: Option<u32>,
    /// 只看排队中/打印中的任务。
    pub active: Option<bool>,
}

/// 任务列表。
#[derive(Debug, Serialize, JsonSchema)]
pub struct JobsView {
    /// 绑定设备。
    pub device: String,
    /// 任务列表（新的在前）。
    pub jobs: Vec<Value>,
}

/// 单任务参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct JobArgs {
    /// 任务 id，形如 office-8。
    pub job_id: String,
}

/// 删文件参数。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteFileArgs {
    /// 上传接口返回的 `file_id`。
    pub file_id: String,
}

/// 操作确认。
#[derive(Debug, Serialize, JsonSchema)]
pub struct ActionAck {
    /// 是否成功。
    pub ok: bool,
    /// 说明。
    pub message: String,
}

/// 绑定到单台设备的 MCP 服务。
#[derive(Clone)]
pub struct McpServer {
    tool_router: ToolRouter<Self>,
    backend: Arc<Backend>,
    state: Arc<AppState>,
    device: Arc<str>,
}

#[tool_router]
impl McpServer {
    /// 构造服务。
    #[must_use]
    pub fn new(state: Arc<AppState>, backend: Arc<Backend>, device: Arc<str>) -> Self {
        Self {
            tool_router: Self::tool_router(),
            backend,
            state,
            device,
        }
    }

    /// 读取绑定设备的状态与选项目录。
    #[tool(
        description = "读取当前绑定设备的状态与可用打印选项。打印前先调用它：既确认设备在线，也拿到 options 的合法键与取值。"
    )]
    async fn get_device(&self) -> Result<Json<DeviceView>, ErrorData> {
        self.state
            .metrics
            .inc("mcp_calls", &[("tool", "get_device")]);
        let (status, headers, body) = self
            .backend
            .call(Method::GET, "/api/printers?refresh=true", None, None)
            .await?;
        let list: PrinterList = backend::parse(status, &headers, &body)?;
        let known: Vec<String> = list.printers.iter().map(|item| item.id.clone()).collect();
        let Some(printer) = list
            .printers
            .into_iter()
            .find(|item| item.id == self.device.as_ref())
        else {
            let hint = if known.is_empty() {
                "服务里没有任何打印机".to_string()
            } else {
                format!("当前可用: {}", known.join(", "))
            };
            return Err(ErrorData::invalid_params(
                format!("绑定的设备 {} 不存在，{hint}", self.device),
                None,
            ));
        };
        Ok(Json(DeviceView {
            device: printer.id,
            state: printer.state,
            accepting_jobs: printer.accepting_jobs,
            make_and_model: printer.make_and_model,
            location: printer.location,
            options: printer.options,
            options_error: printer.options_error,
        }))
    }

    /// 签发一次性上传票据。
    #[tool(
        description = "为一次文件上传签发一次性票据，返回可以直接执行的 curl 命令。你无法把文件字节写进对话，所以必须走这条路：1) 调用本工具，2) 在终端执行返回的命令（把 {path} 换成本机 PDF 路径），3) 用返回的 file_id 调 print_document。票据 5 分钟有效、只能用一次。"
    )]
    async fn create_upload_ticket(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(args): Parameters<TicketArgs>,
    ) -> Result<Json<TicketView>, ErrorData> {
        self.state
            .metrics
            .inc("mcp_calls", &[("tool", "create_upload_ticket")]);
        let Some(name) = sanitize_name(&args.filename) else {
            return Err(ErrorData::invalid_params(
                "filename 不合法：只需要文件名（如 report.pdf），不要带路径或控制字符".to_string(),
                None,
            ));
        };
        let Some(ticket) = self.state.tickets.issue(name) else {
            return Err(ErrorData::internal_error(
                "同时有效的上传票据已满，请稍后重试".to_string(),
                None,
            ));
        };
        let base = base_url(&ctx).unwrap_or_default();
        let url = format!("{base}/uploads/{ticket}");
        Ok(Json(TicketView {
            upload_url: url.clone(),
            curl: format!("curl -sS -X POST --data-binary @{{path}} '{url}'"),
            expires_in_secs: TICKET_TTL.as_secs(),
        }))
    }

    /// 提交打印任务。
    #[tool(
        description = "把已上传的文件提交到绑定设备打印。file_id 来自上传接口的返回；options 的键和值必须来自 get_device。相同 file_id 与选项在 10 分钟内重复调用会回放同一个任务、不会重复出纸；需要多份请用 copies 选项。"
    )]
    async fn print_document(
        &self,
        Parameters(args): Parameters<PrintArgs>,
    ) -> Result<Json<PrintView>, ErrorData> {
        self.state
            .metrics
            .inc("mcp_calls", &[("tool", "print_document")]);
        let key = idempotency_key(&args.file_id, &self.device, &args.options);
        let payload = json!({
            "file_id": args.file_id,
            "printer_id": self.device.as_ref(),
            "options": args.options,
        });
        let (status, headers, body) = self
            .backend
            .call(Method::POST, "/api/print", Some(&payload), Some(&key))
            .await?;
        let accepted: PrintAccepted = backend::parse(status, &headers, &body)?;
        let replayed = headers
            .get("idempotency-replayed")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        let note =
            replayed.then(|| "这次回放了 10 分钟内相同内容的提交，没有重复出纸。".to_string());
        Ok(Json(PrintView {
            job_id: accepted.job_id,
            device: accepted.printer_id,
            status: accepted.status,
            file_name: accepted.file_name,
            created_at_ms: accepted.created_at_ms,
            replayed,
            note,
        }))
    }

    /// 列出本设备任务。
    #[tool(description = "列出当前绑定设备的打印任务（新的在前）。看不到其它设备的任务。")]
    async fn list_jobs(
        &self,
        Parameters(args): Parameters<ListJobsArgs>,
    ) -> Result<Json<JobsView>, ErrorData> {
        self.state
            .metrics
            .inc("mcp_calls", &[("tool", "list_jobs")]);
        let limit = args.limit.unwrap_or(20).clamp(1, MAX_JOB_LIMIT);
        let fetch = limit.saturating_mul(4).min(MAX_JOB_LIMIT).max(limit);
        let mut path = format!("/api/jobs?limit={fetch}");
        if args.active.unwrap_or(false) {
            path.push_str("&active=true");
        }
        let (status, headers, body) = self.backend.call(Method::GET, &path, None, None).await?;
        let list: Value = backend::parse(status, &headers, &body)?;
        let mut jobs: Vec<Value> = list
            .get("jobs")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| {
                        item.get("printer_id").and_then(Value::as_str) == Some(self.device.as_ref())
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        jobs.truncate(usize::try_from(limit).unwrap_or(20));
        Ok(Json(JobsView {
            device: self.device.to_string(),
            jobs,
        }))
    }

    /// 查询单个任务。
    #[tool(description = "查询当前设备上某个任务的状态。")]
    async fn get_job(
        &self,
        Parameters(args): Parameters<JobArgs>,
    ) -> Result<Json<Value>, ErrorData> {
        self.state.metrics.inc("mcp_calls", &[("tool", "get_job")]);
        self.ensure_local_job(&args.job_id)?;
        let (status, headers, body) = self
            .backend
            .call(
                Method::GET,
                &format!("/api/jobs/{}", args.job_id),
                None,
                None,
            )
            .await?;
        let job: Value = backend::parse(status, &headers, &body)?;
        Ok(Json(job))
    }

    /// 取消任务。
    #[tool(description = "取消当前设备上排队中或打印中的任务。已经结束的任务不能取消。")]
    async fn cancel_job(
        &self,
        Parameters(args): Parameters<JobArgs>,
    ) -> Result<Json<ActionAck>, ErrorData> {
        self.state
            .metrics
            .inc("mcp_calls", &[("tool", "cancel_job")]);
        self.ensure_local_job(&args.job_id)?;
        let (status, headers, body) = self
            .backend
            .call(
                Method::DELETE,
                &format!("/api/jobs/{}", args.job_id),
                None,
                None,
            )
            .await?;
        if !status.is_success() {
            return Err(backend::to_error(status, &headers, &body));
        }
        Ok(Json(ActionAck {
            ok: true,
            message: format!("任务 {} 已取消", args.job_id),
        }))
    }

    /// 删除上传的文件。
    #[tool(
        description = "删除一个还没有打印的上传文件（上传接口返回的 file_id）。用于上传错了的场景。"
    )]
    async fn delete_file(
        &self,
        Parameters(args): Parameters<DeleteFileArgs>,
    ) -> Result<Json<ActionAck>, ErrorData> {
        self.state
            .metrics
            .inc("mcp_calls", &[("tool", "delete_file")]);
        let (status, headers, body) = self
            .backend
            .call(
                Method::DELETE,
                &format!("/api/files/{}", args.file_id),
                None,
                None,
            )
            .await?;
        if !status.is_success() {
            return Err(backend::to_error(status, &headers, &body));
        }
        Ok(Json(ActionAck {
            ok: true,
            message: format!("文件 {} 已删除", args.file_id),
        }))
    }

    fn ensure_local_job(&self, job_id: &str) -> Result<(), ErrorData> {
        let prefix = format!("{}-", self.device);
        let local = job_id
            .strip_prefix(&prefix)
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit()));
        if local {
            return Ok(());
        }
        Err(ErrorData::invalid_params(
            format!(
                "任务 {job_id} 不属于设备 {}，只能操作本设备的任务",
                self.device
            ),
            None,
        ))
    }
}

// 宏生成的默认方法体里没有 await，属实现细节，不影响工具语义。
#[allow(clippy::unused_async_trait_impl)]
#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        let instructions = format!(
            "你只能把文件打印到设备 {}。你无法把文件字节写进对话，所以上传必须走终端：create_upload_ticket 拿到一次性的 curl 命令 -> 在终端执行它（把 {{path}} 换成本机 PDF 路径）拿到 file_id -> print_document 提交。get_device 给出 options 的合法取值。工具都是原子的：上传成功不代表打印成功。",
            self.device
        );
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
            .with_instructions(instructions)
    }
}

/// 从请求里推断对外基地址；拿不到请求头时返回空串（调用方降级为相对地址）。
fn base_url(ctx: &RequestContext<RoleServer>) -> Option<String> {
    let parts = ctx.extensions.get::<axum::http::request::Parts>()?;
    let host = parts.headers.get(header::HOST)?.to_str().ok()?;
    let scheme = parts
        .headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("http");
    Some(format!("{scheme}://{host}"))
}

/// 只保留文件名部分，拒绝路径与控制字符。
fn sanitize_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_NAME_LEN {
        return None;
    }
    let name = std::path::Path::new(trimmed).file_name()?.to_str()?;
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    if name.chars().any(char::is_control) {
        return None;
    }
    Some(name.to_string())
}

/// 由内容派生幂等键：相同文件、设备与选项在保留期内映射到同一个键。
fn idempotency_key(file_id: &str, device: &str, options: &BTreeMap<String, String>) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file_id.hash(&mut hasher);
    device.hash(&mut hasher);
    options.hash(&mut hasher);
    format!("mcp-{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{McpServer, idempotency_key, sanitize_name};
    use crate::state::AppState;

    fn server() -> Option<McpServer> {
        let temp = std::env::temp_dir().join("just-print-mcp-unit");
        std::fs::create_dir_all(&temp).ok()?;
        let config = crate::config::Config {
            addr: std::net::SocketAddr::from(([127, 0, 0, 1], 8080)),
            web_dir: temp.clone(),
            token: std::sync::Arc::from("token"),
            cups_server: "127.0.0.1:631".to_string(),
            cups_scheme: "ipp".to_string(),
            max_upload_bytes: 1024,
            conversion_timeout: std::time::Duration::from_secs(1),
            conversion_slots: 1,
            cleanup_interval: std::time::Duration::from_secs(1),
            temp_ttl: std::time::Duration::from_secs(1),
            ipp_timeout: std::time::Duration::from_secs(1),
            printer_cache_ttl: std::time::Duration::from_secs(1),
            job_cache_ttl: std::time::Duration::from_secs(1),
            idempotency_ttl: std::time::Duration::from_secs(60),
            idempotency_wait: std::time::Duration::from_secs(1),
            upload_slots: 1,
            request_timeout: std::time::Duration::from_secs(60),
        };
        let state = AppState::new(config, temp).ok()?;
        let state = std::sync::Arc::new(state);
        let backend = std::sync::Arc::new(crate::mcp::backend::Backend::new(
            axum::Router::new(),
            std::sync::Arc::from("token"),
        ));
        Some(McpServer::new(
            state,
            backend,
            std::sync::Arc::from("office"),
        ))
    }

    #[test]
    fn exposes_the_expected_tool_surface() {
        let names: Vec<String> = McpServer::tool_router()
            .list_all()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        assert_eq!(
            names,
            vec![
                "cancel_job",
                "create_upload_ticket",
                "delete_file",
                "get_device",
                "get_job",
                "list_jobs",
                "print_document",
            ]
        );
    }

    #[test]
    fn every_tool_has_a_description() {
        for tool in McpServer::tool_router().list_all() {
            let description = tool.description.as_deref().unwrap_or_default();
            assert!(!description.is_empty(), "{} 缺少 description", tool.name);
        }
    }

    #[test]
    fn idempotency_key_is_stable_and_option_sensitive() {
        let mut options = BTreeMap::new();
        options.insert("media".to_string(), "iso_a4".to_string());
        let first = idempotency_key("abc", "office", &options);
        assert_eq!(first, idempotency_key("abc", "office", &options));
        assert_ne!(first, idempotency_key("abc", "lobby", &options));
        options.insert("copies".to_string(), "2".to_string());
        assert_ne!(first, idempotency_key("abc", "office", &options));
        assert!(first.starts_with("mcp-"));
    }

    #[test]
    fn file_names_are_reduced_to_a_basename() {
        assert_eq!(sanitize_name("report.pdf").as_deref(), Some("report.pdf"));
        assert_eq!(
            sanitize_name("/tmp/dir/report.pdf").as_deref(),
            Some("report.pdf")
        );
        assert_eq!(sanitize_name(".."), None);
        assert_eq!(sanitize_name("   "), None);
        assert_eq!(sanitize_name("a\u{0}b.pdf"), None);
    }

    #[test]
    fn job_ids_from_other_devices_are_rejected() {
        let Some(server) = server() else { return };
        assert!(server.ensure_local_job("office-12").is_ok());
        assert!(server.ensure_local_job("lobby-12").is_err());
        assert!(server.ensure_local_job("office-").is_err());
        assert!(server.ensure_local_job("office-12x").is_err());
    }
}
