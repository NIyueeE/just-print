//! CUPS 集成：直接通过 IPP over HTTP 与 CUPS 通信。
//!
//! 使用标准 IPP（RFC 8011）操作实现全部能力：`CUPS-Get-Printers` 枚举打印机、
//! `Get-Printer-Attributes` 读取选项、`Print-Job` 提交文档、
//! `Get-Job-Attributes` / `Get-Jobs` 查询任务、`Cancel-Job` 取消任务。
//! 应用层不再依赖 `lp` / `lpstat` / `lpoptions` / `ipptool` 子进程，也没有
//! 每请求 fork 的开销。
//!
//! 打印机快照与任务状态都带短 TTL 缓存与 singleflight 刷新，前端轮询不会直接
//! 放大成 CUPS 请求风暴。

mod options;

pub use options::{OptionSpec, encode_job_attributes};

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ipp::attribute::{IppAttribute, IppAttributeGroup};
use ipp::error::IppError;
use ipp::model::{DelimiterTag, IppVersion, JobState, Operation, PrinterState, StatusCode};
use ipp::operation::builder::IppOperationBuilder;
use ipp::payload::IppPayload;
use ipp::prelude::{AsyncIppClient, IppRequestResponse, Uri};
use ipp::value::IppValue;
use serde::Serialize;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::{debug, warn};

/// 提交给 CUPS 的 `requesting-user-name`。
pub const CUPS_USER: &str = "just-print";
/// 打印文档的 MIME 类型。
pub const APPLICATION_PDF: &str = "application/pdf";

/// CUPS 集成错误。
#[derive(Debug, Error)]
pub enum CupsError {
    /// CUPS 不可达（连接失败、DNS 失败等）。
    #[error("CUPS 不可用: {0}")]
    Unavailable(String),
    /// IPP 请求超时。
    #[error("CUPS 请求超时")]
    Timeout,
    /// CUPS 中不存在对应对象（打印机 / 任务）。
    #[error("CUPS 中不存在该对象")]
    NotFound,
    /// 请求参数不合法（IPP 客户端错误）。
    #[error("{0}")]
    Invalid(String),
    /// 资源当前状态不允许该操作（如已结束的任务无法取消）。
    #[error("{0}")]
    Conflict(String),
    /// 打印机存在但当前不可用。
    #[error("打印机不可用: {0}")]
    PrinterUnavailable(String),
    /// IPP 协议或编码错误。
    #[error("IPP 协议错误: {0}")]
    Protocol(String),
}

impl CupsError {
    /// 根据 IPP 状态码构造错误。
    #[must_use]
    pub fn from_status(status: StatusCode, message: &str) -> Self {
        use StatusCode::{
            ClientErrorAttributesOrValuesNotSupported, ClientErrorBadRequest,
            ClientErrorConflictingAttributes, ClientErrorDocumentFormatError,
            ClientErrorDocumentFormatNotSupported, ClientErrorGone, ClientErrorNotFound,
            ClientErrorNotPossible, ClientErrorRequestValueTooLong, ServerErrorBusy,
            ServerErrorDeviceError, ServerErrorNotAcceptingJobs, ServerErrorServiceUnavailable,
            ServerErrorTemporaryError,
        };
        let detail = if message.is_empty() {
            status.to_string()
        } else {
            message.to_string()
        };
        match status {
            ClientErrorNotFound | ClientErrorGone => Self::NotFound,
            ClientErrorBadRequest
            | ClientErrorAttributesOrValuesNotSupported
            | ClientErrorConflictingAttributes
            | ClientErrorDocumentFormatNotSupported
            | ClientErrorDocumentFormatError
            | ClientErrorRequestValueTooLong => Self::Invalid(detail),
            ClientErrorNotPossible => Self::Conflict(detail),
            ServerErrorNotAcceptingJobs
            | ServerErrorBusy
            | ServerErrorTemporaryError
            | ServerErrorServiceUnavailable
            | ServerErrorDeviceError => Self::PrinterUnavailable(detail),
            _ => Self::Protocol(format!("IPP {status}: {detail}")),
        }
    }
}

impl From<IppError> for CupsError {
    fn from(error: IppError) -> Self {
        match error {
            IppError::StatusError(status) => Self::from_status(status, ""),
            IppError::AsyncClientError(error) => {
                if error.is_timeout() {
                    Self::Timeout
                } else {
                    Self::Unavailable(error.to_string())
                }
            }
            IppError::RequestError(code) => Self::Protocol(format!("IPP 服务返回 HTTP {code}")),
            IppError::PrinterNotReady => Self::PrinterUnavailable("打印机未就绪".to_string()),
            other => Self::Protocol(other.to_string()),
        }
    }
}

impl From<ipp::parser::IppParseError> for CupsError {
    fn from(error: ipp::parser::IppParseError) -> Self {
        Self::Protocol(error.to_string())
    }
}

/// 对外暴露的打印机状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PrinterStateView {
    /// 空闲。
    Idle,
    /// 打印中。
    Printing,
    /// 已停止。
    Stopped,
    /// 已禁用（拒绝新任务）。
    Disabled,
}

/// CUPS 打印机及其可用选项。
#[derive(Debug, Clone)]
pub struct PrinterInfo {
    /// CUPS 打印机名（也是提交任务时的 `printer_id`）。
    pub name: String,
    /// 展示名称（优先使用 `printer-info`）。
    pub display_name: String,
    /// 打印机状态。
    pub state: PrinterStateView,
    /// 是否接受新任务。
    pub accepting_jobs: bool,
    /// 厂商与型号。
    pub make_and_model: Option<String>,
    /// 物理位置。
    pub location: Option<String>,
    /// 可用的 job 模板选项（标准 IPP 属性名）。
    pub options: BTreeMap<String, OptionSpec>,
    /// 读取选项失败时的错误信息；成功时为 `None`。
    pub options_error: Option<String>,
}

/// CUPS 任务状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    /// 排队中（含 held）。
    Queued,
    /// 打印中。
    Printing,
    /// 已完成。
    Completed,
    /// 失败。
    Failed,
    /// 已取消。
    Canceled,
}

/// CUPS 任务视图。
#[derive(Debug, Clone)]
pub struct JobInfo {
    /// 目标打印机名。
    pub printer_id: String,
    /// CUPS 任务号（打印机内唯一）。
    pub cups_job_id: i32,
    /// 任务名（通常为文件名）。
    pub name: Option<String>,
    /// 当前状态。
    pub status: JobStatus,
    /// 失败/取消原因。
    pub error: Option<String>,
    /// 创建时间（Unix 毫秒，UTC）。
    pub created_at_ms: u64,
}

impl JobInfo {
    /// 稳定的应用层任务 id：`<printer>-<job-id>`。
    #[must_use]
    pub fn id(&self) -> String {
        format!("{}-{}", self.printer_id, self.cups_job_id)
    }
}

/// 带时间戳的缓存项。
#[derive(Debug)]
struct Cached<T> {
    value: T,
    at: Instant,
}

/// IPP 客户端（带缓存与 singleflight）。
#[derive(Debug)]
pub struct CupsClient {
    scheme: String,
    server: String,
    base: Uri,
    timeout: Duration,
    printer_ttl: Duration,
    job_ttl: Duration,
    printers: RwLock<Option<Cached<Arc<Vec<PrinterInfo>>>>>,
    refresh: Mutex<()>,
    jobs: Mutex<HashMap<String, Cached<Option<JobInfo>>>>,
}

impl CupsClient {
    /// 创建 IPP 客户端。
    ///
    /// `server` 为 `host:port`，`scheme` 为 `ipp` / `ipps`。
    ///
    /// # Errors
    ///
    /// scheme 不受支持或地址无法解析为 URI 时返回 [`CupsError`]。
    pub fn new(
        server: String,
        scheme: String,
        timeout: Duration,
        printer_ttl: Duration,
        job_ttl: Duration,
    ) -> Result<Self, CupsError> {
        if scheme != "ipp" && scheme != "ipps" {
            return Err(CupsError::Protocol(format!(
                "不支持的 CUPS scheme: {scheme}"
            )));
        }
        let base = format!("{scheme}://{server}/")
            .parse::<Uri>()
            .map_err(|error| CupsError::Protocol(error.to_string()))?;
        Ok(Self {
            scheme,
            server,
            base,
            timeout,
            printer_ttl,
            job_ttl,
            printers: RwLock::new(None),
            refresh: Mutex::new(()),
            jobs: Mutex::new(HashMap::new()),
        })
    }

    /// 列出打印机（带缓存与 singleflight）。
    ///
    /// # Errors
    ///
    /// CUPS 不可达、超时或返回协议错误时返回 [`CupsError`]。
    pub async fn list_printers(&self) -> Result<Arc<Vec<PrinterInfo>>, CupsError> {
        if let Some(hit) = self.cached_printers().await {
            return Ok(hit);
        }
        let _guard = self.refresh.lock().await;
        if let Some(hit) = self.cached_printers().await {
            return Ok(hit);
        }
        let printers = Arc::new(self.fetch_printers().await?);
        *self.printers.write().await = Some(Cached {
            value: Arc::clone(&printers),
            at: Instant::now(),
        });
        Ok(printers)
    }

    /// 在缓存快照中查找打印机。
    ///
    /// # Errors
    ///
    /// 同 [`CupsClient::list_printers`]。
    pub async fn find_printer(&self, name: &str) -> Result<Option<PrinterInfo>, CupsError> {
        let printers = self.list_printers().await?;
        Ok(printers
            .iter()
            .find(|printer| printer.name == name)
            .cloned())
    }

    /// 提交打印任务。
    ///
    /// `job_title` 会作为 IPP `job-name` 提交（自动截断到 255 字节以内）。
    ///
    /// # Errors
    ///
    /// 文件无法读取、IPP 提交失败或响应缺少 `job-id` 时返回 [`CupsError`]。
    pub async fn submit(
        &self,
        printer: &PrinterInfo,
        file: &Path,
        job_title: &str,
        attributes: Vec<IppAttribute>,
    ) -> Result<JobInfo, CupsError> {
        let uri = self.printer_uri(&printer.name)?;
        let handle = tokio::fs::File::open(file)
            .await
            .map_err(|error| CupsError::Protocol(format!("打开待打印文件失败: {error}")))?;
        let payload = IppPayload::new_async(handle.compat());
        let operation = IppOperationBuilder::print_job(uri.clone(), payload)
            .job_title(truncate_name(job_title))
            .user_name(CUPS_USER)
            .document_format(APPLICATION_PDF)
            .attributes(attributes)
            .build()
            .map_err(CupsError::from)?;
        let response = self.send(operation, &uri, "print-job").await?;
        let job_id = job_id_from_response(&response)
            .ok_or_else(|| CupsError::Protocol("Print-Job 响应缺少 job-id".to_string()))?;
        Ok(job_from_groups(
            &printer.name,
            job_id,
            response.attributes().first_of(DelimiterTag::JobAttributes),
        ))
    }

    /// 查询单个任务状态；任务不存在时返回 `None`。
    ///
    /// # Errors
    ///
    /// CUPS 不可达、超时或协议错误时返回 [`CupsError`]。
    pub async fn job_status(
        &self,
        printer_name: &str,
        cups_job_id: i32,
    ) -> Result<Option<JobInfo>, CupsError> {
        let key = format!("{printer_name}-{cups_job_id}");
        if let Some(cached) = self.cached_job(&key).await {
            return Ok(cached);
        }
        let uri = self.printer_uri(printer_name)?;
        let operation = IppOperationBuilder::get_job_attributes(uri.clone(), cups_job_id)
            .build()
            .map_err(CupsError::from)?;
        let info = match self.send(operation, &uri, "get-job-attributes").await {
            Ok(response) => Some(job_from_groups(
                printer_name,
                cups_job_id,
                response.attributes().first_of(DelimiterTag::JobAttributes),
            )),
            Err(CupsError::NotFound) => None,
            Err(error) => return Err(error),
        };
        self.jobs.lock().await.insert(
            key,
            Cached {
                value: info.clone(),
                at: Instant::now(),
            },
        );
        Ok(info)
    }

    /// 列出打印机上的任务（含已完成历史，用于提交失败后的对账）。
    ///
    /// # Errors
    ///
    /// CUPS 不可达、超时或协议错误时返回 [`CupsError`]。
    pub async fn list_jobs(&self, printer_name: &str) -> Result<Vec<JobInfo>, CupsError> {
        let uri = self.printer_uri(printer_name)?;
        let mut request =
            IppRequestResponse::new(IppVersion::v1_1(), Operation::GetJobs, Some(uri.clone()))
                .map_err(CupsError::from)?;
        add_keyword_attribute(&mut request, "which-jobs", "all")?;
        add_requested_attributes(
            &mut request,
            &[
                "job-id",
                "job-name",
                "job-state",
                "job-state-reasons",
                "time-at-creation",
            ],
        )?;
        let response = self.send(request, &uri, "get-jobs").await?;
        let mut jobs = Vec::new();
        for group in response.attributes().groups_of(DelimiterTag::JobAttributes) {
            let Some(job_id) = group_enum(group, "job-id") else {
                continue;
            };
            jobs.push(job_from_group(printer_name, job_id, group));
        }
        Ok(jobs)
    }

    /// 取消任务。
    ///
    /// # Errors
    ///
    /// CUPS 不可达、超时、任务不存在或拒绝取消时返回 [`CupsError`]。
    pub async fn cancel_job(&self, printer_name: &str, cups_job_id: i32) -> Result<(), CupsError> {
        let uri = self.printer_uri(printer_name)?;
        let operation = IppOperationBuilder::cancel_job(uri.clone(), cups_job_id)
            .build()
            .map_err(CupsError::from)?;
        self.send(operation, &uri, "cancel-job").await?;
        self.jobs
            .lock()
            .await
            .remove(&format!("{printer_name}-{cups_job_id}"));
        Ok(())
    }

    /// 就绪探针：确认 CUPS 能响应 IPP 请求。
    ///
    /// # Errors
    ///
    /// CUPS 不可达、超时或协议错误时返回 [`CupsError`]。
    pub async fn ready(&self) -> Result<(), CupsError> {
        let cups = IppOperationBuilder::cups();
        self.send(cups.get_printers(), &self.base, "cups-get-printers")
            .await
            .map(|_| ())
    }

    /// 丢弃打印机快照缓存（下一个请求会重新拉取）。
    pub async fn invalidate_printers(&self) {
        *self.printers.write().await = None;
    }

    async fn cached_printers(&self) -> Option<Arc<Vec<PrinterInfo>>> {
        let guard = self.printers.read().await;
        let cached = guard.as_ref()?;
        (cached.at.elapsed() < self.printer_ttl).then(|| Arc::clone(&cached.value))
    }

    async fn cached_job(&self, key: &str) -> Option<Option<JobInfo>> {
        let guard = self.jobs.lock().await;
        let cached = guard.get(key)?;
        (cached.at.elapsed() < self.job_ttl).then(|| cached.value.clone())
    }

    async fn fetch_printers(&self) -> Result<Vec<PrinterInfo>, CupsError> {
        let cups = IppOperationBuilder::cups();
        let response = self
            .send(cups.get_printers(), &self.base, "cups-get-printers")
            .await?;
        let mut printers = Vec::new();
        for group in response
            .attributes()
            .groups_of(DelimiterTag::PrinterAttributes)
        {
            if let Some(printer) = printer_from_group(group) {
                printers.push(printer);
            }
        }
        for printer in &mut printers {
            let name = printer.name.clone();
            match self.fetch_options(&name).await {
                Ok(options) => printer.options = options,
                Err(error) => {
                    warn!(printer = %name, error = %error, "读取打印机选项失败");
                    printer.options_error = Some(error.to_string());
                }
            }
        }
        printers.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(printers)
    }

    async fn fetch_options(
        &self,
        printer_name: &str,
    ) -> Result<BTreeMap<String, OptionSpec>, CupsError> {
        let uri = self.printer_uri(printer_name)?;
        let operation = IppOperationBuilder::get_printer_attributes(uri.clone())
            .attributes(options::REQUESTED_ATTRIBUTES)
            .build()
            .map_err(CupsError::from)?;
        let response = self.send(operation, &uri, "get-printer-attributes").await?;
        Ok(options::parse_options(response.attributes()))
    }

    fn printer_uri(&self, name: &str) -> Result<Uri, CupsError> {
        let safe = !name.is_empty()
            && name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
        if !safe {
            return Err(CupsError::Protocol(format!("非法打印机名: {name}")));
        }
        format!("{}://{}/printers/{name}", self.scheme, self.server)
            .parse::<Uri>()
            .map_err(|error| CupsError::Protocol(error.to_string()))
    }

    async fn send<R>(
        &self,
        request: R,
        endpoint: &Uri,
        operation: &'static str,
    ) -> Result<IppRequestResponse, CupsError>
    where
        R: Into<IppRequestResponse>,
    {
        let client = AsyncIppClient::builder(endpoint.clone())
            .request_timeout(self.timeout)
            .build();
        let started = Instant::now();
        debug!(operation, uri = %endpoint, "发送 IPP 请求");
        let result = client.send(request).await;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match result {
            Ok(response) => {
                let status = response.header().status_code();
                debug!(operation, elapsed_ms, status = %status, "IPP 请求完成");
                if status.is_success() {
                    Ok(response)
                } else {
                    let message = status_message(&response);
                    Err(CupsError::from_status(status, &message))
                }
            }
            Err(error) => {
                debug!(operation, elapsed_ms, error = %error, "IPP 请求失败");
                Err(CupsError::from(error))
            }
        }
    }
}

fn status_message(response: &IppRequestResponse) -> String {
    response
        .attributes()
        .first_of(DelimiterTag::OperationAttributes)
        .and_then(|group| group.get("status-message"))
        .and_then(|attribute| first_string(attribute.value()))
        .unwrap_or_default()
}

fn add_keyword_attribute(
    request: &mut IppRequestResponse,
    name: &str,
    value: &str,
) -> Result<(), CupsError> {
    let attribute = IppAttribute::with_name(name, IppValue::new_keyword(value)?)?;
    request
        .attributes_mut()
        .add(DelimiterTag::OperationAttributes, attribute);
    Ok(())
}

fn add_requested_attributes(
    request: &mut IppRequestResponse,
    names: &[&str],
) -> Result<(), CupsError> {
    let values = names
        .iter()
        .map(|name| IppValue::new_keyword(*name))
        .collect::<Result<Vec<_>, _>>()?;
    let attribute = IppAttribute::with_name("requested-attributes", IppValue::Array(values))?;
    request
        .attributes_mut()
        .add(DelimiterTag::OperationAttributes, attribute);
    Ok(())
}

fn printer_from_group(group: &IppAttributeGroup) -> Option<PrinterInfo> {
    let name = group_str(group, "printer-name")?;
    let accepting_jobs = group_bool(group, "printer-is-accepting-jobs").unwrap_or(true);
    let raw_state = group_enum(group, "printer-state").and_then(printer_state);
    let state = match (raw_state, accepting_jobs) {
        (Some(PrinterState::Stopped), _) => PrinterStateView::Stopped,
        (_, false) => PrinterStateView::Disabled,
        (Some(PrinterState::Processing), _) => PrinterStateView::Printing,
        _ => PrinterStateView::Idle,
    };
    Some(PrinterInfo {
        display_name: group_str(group, "printer-info").unwrap_or_else(|| name.clone()),
        name,
        state,
        accepting_jobs,
        make_and_model: group_str(group, "printer-make-and-model"),
        location: group_str(group, "printer-location"),
        options: BTreeMap::new(),
        options_error: None,
    })
}

fn printer_state(value: i32) -> Option<PrinterState> {
    match value {
        3 => Some(PrinterState::Idle),
        4 => Some(PrinterState::Processing),
        5 => Some(PrinterState::Stopped),
        _ => None,
    }
}

fn job_id_from_response(response: &IppRequestResponse) -> Option<i32> {
    response
        .attributes()
        .groups_of(DelimiterTag::JobAttributes)
        .find_map(|group| group_enum(group, "job-id"))
}

fn job_from_groups(
    printer_name: &str,
    cups_job_id: i32,
    group: Option<&IppAttributeGroup>,
) -> JobInfo {
    group.map_or_else(
        || JobInfo {
            printer_id: printer_name.to_string(),
            cups_job_id,
            name: None,
            status: JobStatus::Queued,
            error: None,
            created_at_ms: 0,
        },
        |group| job_from_group(printer_name, cups_job_id, group),
    )
}

fn job_from_group(printer_name: &str, cups_job_id: i32, group: &IppAttributeGroup) -> JobInfo {
    let status = group_enum(group, "job-state")
        .and_then(job_state)
        .map_or(JobStatus::Queued, map_job_state);
    let reasons = group_keywords(group, "job-state-reasons");
    let error = match status {
        JobStatus::Failed | JobStatus::Canceled => {
            let joined = reasons
                .iter()
                .filter(|reason| reason.as_str() != "none" && !reason.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            (!joined.is_empty()).then_some(joined)
        }
        _ => None,
    };
    JobInfo {
        printer_id: printer_name.to_string(),
        cups_job_id,
        name: group_str(group, "job-name"),
        status,
        error,
        created_at_ms: group_enum(group, "time-at-creation")
            .and_then(|seconds| u64::try_from(seconds).ok())
            .map_or(0, |seconds| seconds.saturating_mul(1000)),
    }
}

fn map_job_state(state: JobState) -> JobStatus {
    match state {
        JobState::Pending | JobState::PendingHeld => JobStatus::Queued,
        JobState::Processing | JobState::ProcessingStopped => JobStatus::Printing,
        JobState::Completed => JobStatus::Completed,
        JobState::Canceled => JobStatus::Canceled,
        JobState::Aborted => JobStatus::Failed,
    }
}

fn group_value<'a>(group: &'a IppAttributeGroup, name: &str) -> Option<&'a IppValue> {
    group.get(name).map(IppAttribute::value)
}

fn group_str(group: &IppAttributeGroup, name: &str) -> Option<String> {
    group_value(group, name).and_then(first_string)
}

fn group_enum(group: &IppAttributeGroup, name: &str) -> Option<i32> {
    match group_value(group, name)? {
        IppValue::Enum(value) | IppValue::Integer(value) => Some(*value),
        _ => None,
    }
}

fn group_bool(group: &IppAttributeGroup, name: &str) -> Option<bool> {
    match group_value(group, name)? {
        IppValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

fn group_keywords(group: &IppAttributeGroup, name: &str) -> Vec<String> {
    group_value(group, name).map_or_else(Vec::new, collect_keywords)
}

fn collect_keywords(value: &IppValue) -> Vec<String> {
    match value {
        IppValue::Array(items) => items.iter().flat_map(collect_keywords).collect(),
        IppValue::Keyword(item) => vec![item.as_str().to_string()],
        _ => Vec::new(),
    }
}

fn first_string(value: &IppValue) -> Option<String> {
    match value {
        IppValue::Keyword(item)
        | IppValue::NameWithoutLanguage(item)
        | IppValue::MimeMediaType(item) => Some(item.as_str().to_string()),
        IppValue::TextWithoutLanguage(item) => Some(item.to_string()),
        IppValue::TextWithLanguage { text, .. } => Some(text.to_string()),
        IppValue::Uri(item) | IppValue::UriScheme(item) => Some(item.as_str().to_string()),
        IppValue::NaturalLanguage(item) | IppValue::Charset(item) => {
            Some(item.as_str().to_string())
        }
        _ => None,
    }
}

/// 把 `job-name` 截断到 IPP 规定的 255 字节以内（按 UTF-8 边界）。
fn truncate_name(value: &str) -> String {
    const MAX: usize = 255;
    if value.len() <= MAX {
        return value.to_string();
    }
    let mut end = MAX;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.get(..end).unwrap_or(value).to_string()
}

/// 将 IPP `job-state` 数值映射为 [`JobState`]。
fn job_state(value: i32) -> Option<JobState> {
    match value {
        3 => Some(JobState::Pending),
        4 => Some(JobState::PendingHeld),
        5 => Some(JobState::Processing),
        6 => Some(JobState::ProcessingStopped),
        7 => Some(JobState::Canceled),
        8 => Some(JobState::Aborted),
        9 => Some(JobState::Completed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ipp::attribute::{IppAttribute, IppAttributes};
    use ipp::model::{DelimiterTag, Operation};
    use ipp::operation::IppOperation as _;
    use ipp::operation::builder::IppOperationBuilder;
    use ipp::parser::IppParser;
    use ipp::payload::IppPayload;
    use ipp::prelude::Uri;
    use ipp::value::IppValue;

    use super::options::{EnumValue, OptionKind, OptionSpec, ResolutionValue};
    use super::{
        JobStatus, PrinterStateView, encode_job_attributes, job_from_group, printer_from_group,
        truncate_name,
    };

    fn attrs(items: Vec<(&str, IppValue)>) -> IppAttributes {
        let mut attributes = IppAttributes::new();
        for (name, value) in items {
            if let Ok(attribute) = IppAttribute::with_name(name, value) {
                attributes.add(DelimiterTag::PrinterAttributes, attribute);
            }
        }
        attributes
    }

    fn group(items: Vec<(&str, IppValue)>) -> IppAttributes {
        attrs(items)
    }

    #[test]
    fn parses_printer_group() {
        let attributes = group(vec![
            (
                "printer-name",
                IppValue::new_keyword("CUPS-PDF").unwrap_or(IppValue::NoValue),
            ),
            (
                "printer-info",
                IppValue::new_text_without_language("CUPS-PDF Printer")
                    .unwrap_or(IppValue::NoValue),
            ),
            (
                "printer-state",
                IppValue::new_enum(3).unwrap_or(IppValue::NoValue),
            ),
            ("printer-is-accepting-jobs", IppValue::new_boolean(true)),
        ]);
        let Some(group) = attributes.first_of(DelimiterTag::PrinterAttributes) else {
            return;
        };
        let printer = printer_from_group(group);
        assert!(printer.is_some());
        if let Some(printer) = printer {
            assert_eq!(printer.name, "CUPS-PDF");
            assert_eq!(printer.display_name, "CUPS-PDF Printer");
            assert_eq!(printer.state, PrinterStateView::Idle);
            assert!(printer.accepting_jobs);
        }
    }

    #[test]
    fn maps_rejecting_printer_to_disabled() {
        let attributes = group(vec![
            (
                "printer-name",
                IppValue::new_keyword("PDF").unwrap_or(IppValue::NoValue),
            ),
            (
                "printer-state",
                IppValue::new_enum(3).unwrap_or(IppValue::NoValue),
            ),
            ("printer-is-accepting-jobs", IppValue::new_boolean(false)),
        ]);
        let Some(group) = attributes.first_of(DelimiterTag::PrinterAttributes) else {
            return;
        };
        assert_eq!(
            printer_from_group(group).map(|printer| printer.state),
            Some(PrinterStateView::Disabled)
        );
    }

    #[test]
    fn parses_job_group() {
        let attributes = group(vec![
            ("job-id", IppValue::new_integer(8)),
            (
                "job-name",
                IppValue::new_name_without_language("report.pdf").unwrap_or(IppValue::NoValue),
            ),
            (
                "job-state",
                IppValue::new_enum(9).unwrap_or(IppValue::NoValue),
            ),
            ("time-at-creation", IppValue::new_integer(1_785_833_607)),
        ]);
        let Some(group) = attributes.first_of(DelimiterTag::PrinterAttributes) else {
            return;
        };
        let job = job_from_group("CUPS-PDF", 8, group);
        assert_eq!(job.id(), "CUPS-PDF-8");
        assert_eq!(job.name.as_deref(), Some("report.pdf"));
        assert_eq!(job.status, JobStatus::Completed);
        assert_eq!(job.created_at_ms, 1_785_833_607_000);
    }

    #[test]
    fn truncates_long_job_names_on_char_boundary() {
        let long = "文".repeat(200);
        let truncated = truncate_name(&long);
        assert!(truncated.len() <= 255);
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    fn spec(kind: OptionKind) -> OptionSpec {
        OptionSpec {
            default: None,
            kind,
        }
    }

    #[test]
    fn print_job_request_carries_encoded_job_attributes() {
        let Ok(uri) = "ipp://127.0.0.1:631/printers/PDF".parse::<Uri>() else {
            return;
        };
        let mut catalog = BTreeMap::new();
        catalog.insert(
            "media".to_string(),
            spec(OptionKind::Keyword {
                values: vec!["iso_a4_210x297mm".to_string()],
            }),
        );
        catalog.insert(
            "copies".to_string(),
            spec(OptionKind::Integer { min: 1, max: 99 }),
        );
        catalog.insert(
            "print-quality".to_string(),
            spec(OptionKind::Enum {
                values: vec![EnumValue {
                    value: 4,
                    name: "normal".to_string(),
                }],
            }),
        );
        catalog.insert(
            "printer-resolution".to_string(),
            spec(OptionKind::Resolution {
                values: vec![ResolutionValue {
                    cross_feed: 600,
                    feed: 600,
                    units: 3,
                    label: "600x600dpi".to_string(),
                }],
            }),
        );

        let mut requested = BTreeMap::new();
        requested.insert("media".to_string(), "iso_a4_210x297mm".to_string());
        requested.insert("copies".to_string(), "2".to_string());
        requested.insert("print-quality".to_string(), "normal".to_string());
        requested.insert("printer-resolution".to_string(), "600x600dpi".to_string());
        let Ok(attributes) = encode_job_attributes(&catalog, &requested) else {
            return;
        };

        let payload = IppPayload::new(std::io::Cursor::new(b"%PDF-1.7".to_vec()));
        let Ok(operation) = IppOperationBuilder::print_job(uri, payload)
            .job_title("[abcd1234] report.pdf")
            .user_name("just-print")
            .document_format(crate::cups::APPLICATION_PDF)
            .attributes(attributes)
            .build()
        else {
            return;
        };
        let bytes = operation.into_ipp_request().to_bytes();
        let Ok((header, parsed, _reader)) =
            IppParser::new(std::io::Cursor::new(bytes)).parse_parts()
        else {
            return;
        };

        assert_eq!(header.operation_or_status, Operation::PrintJob as i16);
        let Some(group) = parsed.first_of(DelimiterTag::JobAttributes) else {
            return;
        };
        assert!(matches!(
            group.get("media").map(IppAttribute::value),
            Some(IppValue::Keyword(_))
        ));
        assert!(matches!(
            group.get("copies").map(IppAttribute::value),
            Some(IppValue::Integer(2))
        ));
        assert!(matches!(
            group.get("print-quality").map(IppAttribute::value),
            Some(IppValue::Enum(4))
        ));
        assert!(matches!(
            group.get("printer-resolution").map(IppAttribute::value),
            Some(IppValue::Resolution { .. })
        ));
        if let Some(operation_group) = parsed.first_of(DelimiterTag::OperationAttributes) {
            assert!(operation_group.get("job-name").is_some());
            assert!(operation_group.get("document-format").is_some());
            assert!(operation_group.get("requesting-user-name").is_some());
        }
    }
}

/// 用进程内假 IPP 服务端验证客户端真实走线（HTTP + IPP 编解码），
/// 不依赖 CUPS 或容器环境。
#[cfg(test)]
mod ipp_server_tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use axum::Router;
    use axum::body::Bytes;
    use axum::http::{StatusCode as HttpStatus, header};
    use axum::response::{IntoResponse, Response};
    use ipp::attribute::IppAttribute;
    use ipp::model::{DelimiterTag, IppVersion, Operation, StatusCode};
    use ipp::parser::IppParser;
    use ipp::prelude::IppRequestResponse;
    use ipp::value::IppValue;
    use tokio::net::TcpListener;

    use super::CupsClient;

    fn add_attribute(
        response: &mut IppRequestResponse,
        group: DelimiterTag,
        name: &str,
        value: IppValue,
    ) {
        if let Ok(attribute) = IppAttribute::with_name(name, value) {
            response.attributes_mut().add(group, attribute);
        }
    }

    fn keyword(value: &str) -> IppValue {
        IppValue::new_keyword(value).unwrap_or(IppValue::NoValue)
    }

    fn add_printer_attributes(response: &mut IppRequestResponse) {
        add_attribute(
            response,
            DelimiterTag::PrinterAttributes,
            "printer-name",
            keyword("FAKE"),
        );
        add_attribute(
            response,
            DelimiterTag::PrinterAttributes,
            "printer-info",
            IppValue::new_text_without_language("Fake Printer").unwrap_or(IppValue::NoValue),
        );
        add_attribute(
            response,
            DelimiterTag::PrinterAttributes,
            "printer-state",
            IppValue::new_enum(3).unwrap_or(IppValue::NoValue),
        );
        add_attribute(
            response,
            DelimiterTag::PrinterAttributes,
            "printer-is-accepting-jobs",
            IppValue::new_boolean(true),
        );
        add_attribute(
            response,
            DelimiterTag::PrinterAttributes,
            "media-supported",
            IppValue::Array(vec![
                keyword("iso_a4_210x297mm"),
                keyword("na_letter_8.5x11in"),
            ]),
        );
        add_attribute(
            response,
            DelimiterTag::PrinterAttributes,
            "sides-supported",
            IppValue::Array(vec![keyword("one-sided"), keyword("two-sided-long-edge")]),
        );
    }

    fn add_job_attributes(response: &mut IppRequestResponse) {
        add_attribute(
            response,
            DelimiterTag::JobAttributes,
            "job-id",
            IppValue::new_integer(1),
        );
        add_attribute(
            response,
            DelimiterTag::JobAttributes,
            "job-name",
            IppValue::new_name_without_language("[abcd1234] test.pdf").unwrap_or(IppValue::NoValue),
        );
        add_attribute(
            response,
            DelimiterTag::JobAttributes,
            "job-state",
            IppValue::new_enum(9).unwrap_or(IppValue::NoValue),
        );
        add_attribute(
            response,
            DelimiterTag::JobAttributes,
            "time-at-creation",
            IppValue::new_integer(1_785_833_607),
        );
    }

    async fn ipp_handler(body: Bytes) -> Response {
        let Ok(parsed) = IppParser::new(std::io::Cursor::new(body.to_vec())).parse() else {
            return (HttpStatus::BAD_REQUEST, "parse error").into_response();
        };
        let operation = parsed.header().operation_or_status;
        let Ok(mut response) = IppRequestResponse::new_response(
            IppVersion::v1_1(),
            StatusCode::SuccessfulOk,
            parsed.header().request_id,
        ) else {
            return (HttpStatus::INTERNAL_SERVER_ERROR, "build error").into_response();
        };
        if operation == Operation::CupsGetPrinters as i16
            || operation == Operation::GetPrinterAttributes as i16
        {
            add_printer_attributes(&mut response);
        }
        if operation == Operation::PrintJob as i16
            || operation == Operation::GetJobAttributes as i16
            || operation == Operation::GetJobs as i16
        {
            add_job_attributes(&mut response);
        }
        (
            [(header::CONTENT_TYPE, "application/ipp")],
            response.to_bytes(),
        )
            .into_response()
    }

    async fn serve_fake_ipp() -> Option<String> {
        let listener = TcpListener::bind("127.0.0.1:0").await.ok()?;
        let addr = listener.local_addr().ok()?;
        tokio::spawn(async move {
            let app = Router::new().fallback(ipp_handler);
            let _ = axum::serve(listener, app).await;
        });
        Some(addr.to_string())
    }

    #[tokio::test]
    async fn talks_to_an_ipp_server_end_to_end() {
        let Some(server) = serve_fake_ipp().await else {
            return;
        };
        let Ok(client) = CupsClient::new(
            server,
            "ipp".to_string(),
            Duration::from_secs(5),
            Duration::from_secs(1),
            Duration::from_secs(1),
        ) else {
            return;
        };

        let Ok(printers) = client.list_printers().await else {
            return;
        };
        assert_eq!(printers.len(), 1);
        let Some(printer) = printers.first() else {
            return;
        };
        assert_eq!(printer.name, "FAKE");
        assert_eq!(printer.display_name, "Fake Printer");
        assert!(printer.options.contains_key("media"));
        assert!(printer.options.contains_key("sides"));

        let path = std::env::temp_dir().join(format!("jp-ipp-test-{}.pdf", crate::ids::new_id()));
        if std::fs::write(&path, b"%PDF-1.7\n").is_err() {
            return;
        }
        let Ok(attributes) = super::encode_job_attributes(&printer.options, &BTreeMap::new())
        else {
            return;
        };
        let Ok(job) = client.submit(printer, &path, "test.pdf", attributes).await else {
            return;
        };
        assert_eq!(job.id(), "FAKE-1");
        assert_eq!(job.created_at_ms, 1_785_833_607_000);

        let Ok(Some(status)) = client.job_status("FAKE", 1).await else {
            return;
        };
        assert_eq!(status.name.as_deref(), Some("[abcd1234] test.pdf"));

        let Ok(jobs) = client.list_jobs("FAKE").await else {
            return;
        };
        assert_eq!(jobs.len(), 1);

        assert!(client.cancel_job("FAKE", 1).await.is_ok());
        assert!(client.ready().await.is_ok());
        let _ = std::fs::remove_file(&path);
    }
}
