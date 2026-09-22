//! 全链路集成测试：上传 PDF → 提交打印（含幂等）→ 任务查询/取消 → 文件删除。
//!
//! 用进程内假 IPP 服务端（HTTP + IPP 编解码）驱动完整路由，覆盖跨模块的
//! 状态流转，而不是只测纯函数。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ipp::attribute::IppAttribute;
use ipp::model::{DelimiterTag, IppVersion, Operation, StatusCode as IppStatus};
use ipp::parser::IppParser;
use ipp::prelude::IppRequestResponse;
use ipp::value::IppValue;
use tokio::net::TcpListener;
use tower::ServiceExt;

use crate::config::Config;
use crate::state::AppState;

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
    for (name, value) in [
        ("printer-name", keyword("FAKE")),
        (
            "printer-info",
            IppValue::new_text_without_language("Fake Printer").unwrap_or(IppValue::NoValue),
        ),
        (
            "printer-state",
            IppValue::new_enum(3).unwrap_or(IppValue::NoValue),
        ),
        ("printer-is-accepting-jobs", IppValue::new_boolean(true)),
        (
            "media-supported",
            IppValue::Array(vec![keyword("iso_a4_210x297mm")]),
        ),
        (
            "sides-supported",
            IppValue::Array(vec![keyword("one-sided"), keyword("two-sided-long-edge")]),
        ),
        (
            "copies-supported",
            IppValue::RangeOfInteger { min: 1, max: 99 },
        ),
    ] {
        add_attribute(response, DelimiterTag::PrinterAttributes, name, value);
    }
}

fn add_job_attributes(response: &mut IppRequestResponse, state: i32) {
    add_attribute(
        response,
        DelimiterTag::JobAttributes,
        "job-id",
        IppValue::new_integer(7),
    );
    add_attribute(
        response,
        DelimiterTag::JobAttributes,
        "job-name",
        IppValue::new_name_without_language("[abcd1234] report.pdf").unwrap_or(IppValue::NoValue),
    );
    add_attribute(
        response,
        DelimiterTag::JobAttributes,
        "job-state",
        IppValue::new_enum(state).unwrap_or(IppValue::NoValue),
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
        return (StatusCode::BAD_REQUEST, "parse error").into_response();
    };
    let operation = parsed.header().operation_or_status;
    let printer_uri = parsed
        .attributes()
        .first_of(DelimiterTag::OperationAttributes)
        .and_then(|group| group.get("printer-uri"))
        .and_then(|attribute| match attribute.value() {
            IppValue::Uri(uri) => Some(uri.as_str().to_string()),
            _ => None,
        })
        .unwrap_or_default();
    // 模拟「打印机已从 CUPS 移除」：对该打印机的任何操作返回 not-found。
    let gone = printer_uri.contains("/printers/GONE");
    if gone {
        let Ok(response) = IppRequestResponse::new_response(
            IppVersion::v1_1(),
            IppStatus::ClientErrorNotFound,
            parsed.header().request_id,
        ) else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "build error").into_response();
        };
        return (
            [(header::CONTENT_TYPE, "application/ipp")],
            response.to_bytes(),
        )
            .into_response();
    }
    let Ok(mut response) = IppRequestResponse::new_response(
        IppVersion::v1_1(),
        IppStatus::SuccessfulOk,
        parsed.header().request_id,
    ) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "build error").into_response();
    };
    if operation == Operation::CupsGetPrinters as i16
        || operation == Operation::GetPrinterAttributes as i16
    {
        add_printer_attributes(&mut response);
    }
    if operation == Operation::PrintJob as i16 || operation == Operation::GetJobAttributes as i16 {
        add_job_attributes(&mut response, 5); // processing
    }
    if operation == Operation::GetJobs as i16 {
        add_job_attributes(&mut response, 9); // completed
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
        let app = axum::Router::new().fallback(ipp_handler);
        let _ = axum::serve(listener, app).await;
    });
    Some(addr.to_string())
}

fn test_config(cups_server: &str, temp_dir: &std::path::Path) -> Config {
    Config {
        addr: std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        web_dir: temp_dir.to_path_buf(),
        token: Arc::from("test-token"),
        cups_server: cups_server.to_string(),
        cups_scheme: "ipp".to_string(),
        max_upload_bytes: 1024 * 1024,
        conversion_timeout: Duration::from_secs(5),
        conversion_slots: 1,
        cleanup_interval: Duration::from_secs(60),
        temp_ttl: Duration::from_secs(60),
        ipp_timeout: Duration::from_secs(5),
        printer_cache_ttl: Duration::from_secs(1),
        job_cache_ttl: Duration::from_secs(1),
        idempotency_ttl: Duration::from_secs(60),
        upload_slots: 2,
        request_timeout: Duration::from_secs(30),
    }
}

async fn test_state() -> Option<(Arc<AppState>, std::path::PathBuf)> {
    let server = serve_fake_ipp().await?;
    let temp = std::env::temp_dir().join(format!("just-print-flow-{}", crate::ids::new_id()));
    let _ = std::fs::create_dir_all(&temp);
    let _ = std::fs::write(temp.join("index.html"), "<div id=\"app\"></div>");
    let state = AppState::new(test_config(&server, &temp), temp.clone()).ok()?;
    Some((Arc::new(state), temp))
}

/// 构造带 Bearer 认证的请求建造器。
fn new_request(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, "Bearer test-token")
}

/// 构造 JSON POST 请求。
fn json_post(uri: &str, body: &str) -> Option<Request<Body>> {
    new_request("POST", uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .ok()
}

async fn call(
    app: &axum::Router,
    request: Request<Body>,
) -> (StatusCode, String, axum::http::HeaderMap) {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .unwrap_or_else(|never| match never {});
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await;
    let body = bytes
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    (status, body, headers)
}

fn multipart_pdf(filename: &str) -> (String, Vec<u8>) {
    let boundary = "probe";
    let mut payload = Vec::new();
    payload.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/pdf\r\n\r\n"
        )
        .as_bytes(),
    );
    payload.extend_from_slice(b"%PDF-1.4 minimal-but-valid");
    payload.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), payload)
}

async fn upload_pdf(app: &axum::Router) -> String {
    let (content_type, payload) = multipart_pdf("report.pdf");
    let Ok(request) = new_request("POST", "/api/files")
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(payload))
    else {
        return String::new();
    };
    let (status, body, _) = call(app, request).await;
    assert_eq!(status, StatusCode::CREATED, "upload failed: {body}");
    let value: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn print_body(file_id: &str) -> String {
    let mut options = BTreeMap::new();
    options.insert("sides".to_string(), "two-sided-long-edge".to_string());
    options.insert("copies".to_string(), "2".to_string());
    serde_json::json!({
        "file_id": file_id,
        "printer_id": "FAKE",
        "options": options,
    })
    .to_string()
}

async fn print_file(
    app: &axum::Router,
    file_id: &str,
    key: Option<&str>,
) -> (StatusCode, String, axum::http::HeaderMap) {
    let mut builder =
        new_request("POST", "/api/print").header(header::CONTENT_TYPE, "application/json");
    if let Some(key) = key {
        builder = builder.header("idempotency-key", key);
    }
    let Ok(request) = builder.body(Body::from(print_body(file_id))) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            String::new(),
            axum::http::HeaderMap::new(),
        );
    };
    call(app, request).await
}

/// 完整打印流程：上传 → 打印 → 列表 → 详情 → 取消 → 删除。
#[tokio::test]
async fn upload_print_list_cancel_delete_flow() {
    let Some((state, _temp)) = test_state().await else {
        return;
    };
    let app = super::router(&state);

    let file_id = upload_pdf(&app).await;

    // 提交打印（带幂等键）。
    let (status, body, headers) = print_file(&app, &file_id, Some("flow-key-1")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "print failed: {body}");
    assert!(
        headers.get("idempotency-replayed").is_none(),
        "首次提交不应回放"
    );
    let submitted: serde_json::Value =
        serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    assert_eq!(
        submitted.get("job_id").and_then(serde_json::Value::as_str),
        Some("FAKE-7")
    );
    assert_eq!(
        submitted.get("status").and_then(serde_json::Value::as_str),
        Some("printing")
    );

    // 同一幂等键重放：必须回放而不是再次提交。
    let (status, body, headers) = print_file(&app, &file_id, Some("flow-key-1")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(
        headers
            .get("idempotency-replayed")
            .and_then(|value| value.to_str().ok()),
        Some("true"),
        "重复提交必须回放首次响应"
    );

    // 任务列表：登记表 + CUPS 状态合并。
    let Ok(request) = new_request("GET", "/api/jobs").body(Body::empty()) else {
        return;
    };
    let (status, body, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    let jobs = listed
        .get("jobs")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(jobs.len(), 1, "应只有 1 个任务: {body}");
    let Some(job) = jobs.first() else {
        return;
    };
    assert_eq!(
        job.get("id").and_then(serde_json::Value::as_str),
        Some("FAKE-7")
    );
    assert_eq!(
        job.get("file_name").and_then(serde_json::Value::as_str),
        Some("report.pdf")
    );
    assert_eq!(
        job.get("options")
            .and_then(|value| value.get("copies"))
            .and_then(serde_json::Value::as_str),
        Some("2")
    );

    // 单个任务查询。
    let Ok(request) = new_request("GET", "/api/jobs/FAKE-7").body(Body::empty()) else {
        return;
    };
    let (status, body, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // 取消任务。
    let Ok(request) = new_request("DELETE", "/api/jobs/FAKE-7").body(Body::empty()) else {
        return;
    };
    let (status, _, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 取消后文件允许删除。
    let Ok(request) = new_request("DELETE", &format!("/api/files/{file_id}")).body(Body::empty())
    else {
        return;
    };
    let (status, _, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// 打印提交完成后 CUPS 已持有 spool 副本，临时文件允许删除（204）；
/// 预览可正常流式读取。
#[tokio::test]
async fn file_deletable_after_print_and_streamable_preview() {
    let Some((state, _temp)) = test_state().await else {
        return;
    };
    let app = super::router(&state);
    let file_id = upload_pdf(&app).await;
    let (status, body, _) = print_file(&app, &file_id, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "print failed: {body}");

    let Ok(request) = new_request("DELETE", &format!("/api/files/{file_id}")).body(Body::empty())
    else {
        return;
    };
    let (status, body, _) = call(&app, request).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "提交完成后文件应可删除: {body}"
    );

    // 删除后预览应 404（文件已不存在）。
    let Ok(request) = new_request("GET", &format!("/api/files/{file_id}")).body(Body::empty())
    else {
        return;
    };
    let (status, _, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// 预览可流式读取且带正确 Content-Type。
#[tokio::test]
async fn preview_streams_pdf() {
    let Some((state, _temp)) = test_state().await else {
        return;
    };
    let app = super::router(&state);
    let file_id = upload_pdf(&app).await;
    let Ok(request) = new_request("GET", &format!("/api/files/{file_id}")).body(Body::empty())
    else {
        return;
    };
    let (status, body, headers) = call(&app, request).await;
    assert_eq!(status, StatusCode::OK, "预览应可读取");
    assert!(body.contains("%PDF-"), "预览应返回 PDF 内容: {body}");
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/pdf")
    );
}

/// active=true 只返回未结束任务（假服务端 Get-Jobs 报 completed，应为空）。
#[tokio::test]
async fn active_filter_returns_only_unfinished_jobs() {
    let Some((state, _temp)) = test_state().await else {
        return;
    };
    let app = super::router(&state);
    let file_id = upload_pdf(&app).await;
    let (status, body, _) = print_file(&app, &file_id, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "print failed: {body}");

    let Ok(request) = new_request("GET", "/api/jobs?active=true").body(Body::empty()) else {
        return;
    };
    let (status, body, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    let jobs = listed
        .get("jobs")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(jobs.is_empty(), "active 列表应排除已完成任务: {body}");
}

/// 取消任务后重新打印：同一 `file_id` 仍可再次提交（幂等键不同）。
#[tokio::test]
async fn reprint_after_cancel_with_new_key() {
    let Some((state, _temp)) = test_state().await else {
        return;
    };
    let app = super::router(&state);
    let file_id = upload_pdf(&app).await;
    let (status, body, _) = print_file(&app, &file_id, Some("key-a")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "print failed: {body}");

    let Ok(request) = new_request("DELETE", "/api/jobs/FAKE-7").body(Body::empty()) else {
        return;
    };
    let (status, _, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 重新打印使用新幂等键：必须再次提交成功。
    let (status, body, headers) = print_file(&app, &file_id, Some("key-b")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "reprint failed: {body}");
    assert!(
        headers.get("idempotency-replayed").is_none(),
        "新幂等键不应回放"
    );

    // 重新打印提交完成后，文件同样允许删除。
    let Ok(request) = new_request("DELETE", &format!("/api/files/{file_id}")).body(Body::empty())
    else {
        return;
    };
    let (status, _, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// 打印不存在的文件必须 404，且不产生 CUPS 任务。
#[tokio::test]
async fn print_unknown_file_returns_404() {
    let Some((state, _temp)) = test_state().await else {
        return;
    };
    let app = super::router(&state);
    let body = r#"{"file_id":"missing","printer_id":"FAKE"}"#;
    let Some(request) = json_post("/api/print", body) else {
        return;
    };
    let (status, body, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// 上传空文件被拒（400），且不残留临时文件。
#[tokio::test]
async fn empty_upload_is_rejected() {
    let Some((state, temp)) = test_state().await else {
        return;
    };
    let app = super::router(&state);
    // 构造零字节正文的合法 multipart。
    let payload = "--probe\r\nContent-Disposition: form-data; name=\"file\"; filename=\"empty.pdf\"\r\nContent-Type: application/pdf\r\n\r\n\r\n--probe--\r\n"
        .as_bytes()
        .to_vec();
    let Ok(request) = new_request("POST", "/api/files")
        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=probe")
        .body(Body::from(payload))
    else {
        return;
    };
    let (status, body, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // 临时目录不应残留上传文件。
    let leftovers: Vec<_> = std::fs::read_dir(&temp)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .filter(|name| name.to_string_lossy().contains("-upload."))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "空上传不应残留临时文件: {leftovers:?}"
    );
}

/// 回归：打印机已从 CUPS 移除时，任务列表必须仍能返回（该打印机的任务按
/// `已清理` 展示），而不是整表 404。
#[tokio::test]
async fn job_list_survives_printer_removed_from_cups() {
    let Some((state, _temp)) = test_state().await else {
        return;
    };
    // 直接登记一个指向「已移除打印机」的任务。
    state.jobs.insert(crate::jobs::JobRecord {
        id: "GONE-12".to_string(),
        file_id: "file-gone".to_string(),
        file_name: "old.pdf".to_string(),
        printer_id: "GONE".to_string(),
        created_at_ms: 1_785_833_607_000,
        options: BTreeMap::new(),
    });
    let app = super::router(&state);

    let Ok(request) = new_request("GET", "/api/jobs").body(Body::empty()) else {
        return;
    };
    let (status, body, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::OK, "打印机被移除不应让列表失败: {body}");
    let listed: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    let jobs = listed
        .get("jobs")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(jobs.len(), 1, "{body}");
    let Some(job) = jobs.first() else {
        return;
    };
    assert_eq!(
        job.get("id").and_then(serde_json::Value::as_str),
        Some("GONE-12")
    );
    assert!(
        job.get("status").is_some_and(serde_json::Value::is_null),
        "CUPS 已不知悉的任务状态应为 null: {body}"
    );

    // active=true 时应把它过滤掉（无法确认仍在进行）。
    let Ok(request) = new_request("GET", "/api/jobs?active=true").body(Body::empty()) else {
        return;
    };
    let (status, body, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    let jobs = listed
        .get("jobs")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(jobs.is_empty(), "active 列表不应包含已清理任务: {body}");
}

/// 回归：纯任务号在唯一命中时应能解析（前端重打/取消只用展示 id）。
#[tokio::test]
async fn bare_job_number_resolves_when_unique() {
    let Some((state, _temp)) = test_state().await else {
        return;
    };
    let app = super::router(&state);
    let file_id = upload_pdf(&app).await;
    let (status, body, _) = print_file(&app, &file_id, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "print failed: {body}");

    let Ok(request) = new_request("GET", "/api/jobs/7").body(Body::empty()) else {
        return;
    };
    let (status, body, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::OK, "唯一任务号应可解析: {body}");
    let job: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    assert_eq!(
        job.get("id").and_then(serde_json::Value::as_str),
        Some("FAKE-7")
    );
}

/// 回归：任务号在多台打印机上重复时必须 400 提示使用完整 id。
#[tokio::test]
async fn ambiguous_job_number_is_rejected() {
    let Some((state, _temp)) = test_state().await else {
        return;
    };
    for printer in ["FAKE", "GONE"] {
        state.jobs.insert(crate::jobs::JobRecord {
            id: format!("{printer}-12"),
            file_id: "file".to_string(),
            file_name: "dup.pdf".to_string(),
            printer_id: printer.to_string(),
            created_at_ms: 1_785_833_607_000,
            options: BTreeMap::new(),
        });
    }
    let app = super::router(&state);
    let Ok(request) = new_request("GET", "/api/jobs/12").body(Body::empty()) else {
        return;
    };
    let (status, body, _) = call(&app, request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("多台打印机"), "应提示使用完整 id: {body}");
}
