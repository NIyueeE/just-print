//! 文件上传、PDF 预览与删除。
//!
//! 上传使用 `field.chunk()` 流式落盘，不会把整个文件读进内存；并发上传由信号量
//! 限制，避免磁盘与内存被瞬时打满。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::multipart::{Field, MultipartRejection};
use axum::extract::{Multipart, Path as AxumPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use crate::conversion::{self, ConversionError};
use crate::error::AppError;
use crate::ids;
use crate::state::AppState;

/// 上传成功响应。
#[derive(Debug, Serialize)]
pub struct UploadResponse {
    /// 文件 id（也是预览/打印的引用）。
    pub id: String,
    /// 原始文件名。
    pub name: String,
    /// 原始字节数。
    pub size: u64,
}

/// 上传文件并转换为 PDF。
pub async fn upload(
    State(state): State<Arc<AppState>>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, Json<UploadResponse>), AppError> {
    let mut multipart = multipart.map_err(|error| {
        AppError::BadRequest(format!("multipart 请求不合法: {}", error.body_text()))
    })?;
    let upload_permit = state
        .upload_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::ServiceUnavailable("上传通道已关闭".to_string()))?;

    let id = ids::new_id();
    let mut pending: Option<(String, PathBuf, u64)> = None;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|error| map_multipart_error(&error))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let name = field
            .file_name()
            .ok_or_else(|| AppError::BadRequest("缺少文件名".to_string()))?
            .to_string();
        let extension = extension_of(&name)?;
        if !conversion::SUPPORTED_EXTENSIONS.contains(&extension.as_str()) {
            return Err(AppError::UnsupportedMediaType(name));
        }
        let path = state.temp_dir.join(format!("{id}-upload.{extension}"));
        let size = write_field(&mut field, &path, state.config.max_upload_bytes).await?;
        // 必须先释放当前 field（multer 在同一时刻只允许一个 field 持有状态锁），
        // 否则后续 next_field() 会返回 "failed to lock multipart state"。
        drop(field);
        pending = Some((name, path, size));
        // 读掉剩余的 multipart 部分，保持连接可复用。
        while multipart
            .next_field()
            .await
            .map_err(|error| map_multipart_error(&error))?
            .is_some()
        {}
        break;
    }
    drop(upload_permit);

    let (name, source_path, original_size) =
        pending.ok_or_else(|| AppError::BadRequest("缺少 file 字段".to_string()))?;

    let conversion_permit = state
        .conversion_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::ServiceUnavailable("转换通道已关闭".to_string()))?;
    let result = conversion::convert_to_pdf(
        &source_path,
        &state.temp_dir,
        state.config.conversion_timeout,
    )
    .await;
    drop(conversion_permit);

    let pdf_path = match result {
        Ok(path) => path,
        Err(error) => {
            let _ = tokio::fs::remove_file(&source_path).await;
            state.metrics.inc("conversions", &[("outcome", "failed")]);
            state.metrics.inc("uploads", &[("outcome", "failed")]);
            return Err(map_conversion_error(error));
        }
    };
    state.metrics.inc("conversions", &[("outcome", "ok")]);
    if pdf_path != source_path {
        let _ = tokio::fs::remove_file(&source_path).await;
    }
    let pdf_size = tokio::fs::metadata(&pdf_path)
        .await
        .map_err(|error| AppError::Internal(format!("读取转换结果失败: {error}")))?
        .len();
    state
        .files
        .insert(id.clone(), name.clone(), pdf_path, pdf_size);
    state.metrics.inc("uploads", &[("outcome", "ok")]);
    tracing::info!(file_id = %id, name = %name, size = original_size, "文档转换完成");
    Ok((
        StatusCode::CREATED,
        Json(UploadResponse {
            id,
            name,
            size: original_size,
        }),
    ))
}

/// 返回转换后的 PDF 预览（流式）。
pub async fn preview(
    State(state): State<Arc<AppState>>,
    AxumPath(file_id): AxumPath<String>,
) -> Result<Response, AppError> {
    let guard = state.files.guard(&file_id).ok_or(AppError::FileNotFound)?;
    let metadata = tokio::fs::metadata(guard.path())
        .await
        .map_err(|error| AppError::Internal(format!("读取预览文件失败: {error}")))?;
    let file = tokio::fs::File::open(guard.path())
        .await
        .map_err(|error| AppError::Internal(format!("读取预览文件失败: {error}")))?;
    drop(guard);
    let content_length = metadata.len().to_string();
    let headers = [
        (header::CONTENT_TYPE, "application/pdf"),
        (
            header::CONTENT_DISPOSITION,
            "inline; filename=\"preview.pdf\"",
        ),
        (header::CONTENT_LENGTH, content_length.as_str()),
    ];
    Ok((headers, Body::from_stream(ReaderStream::new(file))).into_response())
}

/// 删除一个尚未被打印引用的上传文件。
pub async fn delete(
    State(state): State<Arc<AppState>>,
    AxumPath(file_id): AxumPath<String>,
) -> Result<StatusCode, AppError> {
    if !state.files.contains(&file_id) {
        return Err(AppError::FileNotFound);
    }
    match state.files.remove(&file_id) {
        Some(path) => {
            let _ = tokio::fs::remove_file(&path).await;
            state.metrics.inc("files_deleted", &[]);
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err(AppError::Conflict(
            "文件正在打印或预览中，暂时无法删除".to_string(),
        )),
    }
}

/// 把单个 multipart 字段流式写入临时文件，超过上限即中止并清理。
async fn write_field(
    field: &mut Field<'_>,
    path: &Path,
    max_bytes: usize,
) -> Result<u64, AppError> {
    let max = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(|error| AppError::Internal(format!("写入临时文件失败: {error}")))?;
    let mut size: u64 = 0;
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|error| map_multipart_error(&error))?
    {
        size = size.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        if size > max {
            drop(file);
            let _ = tokio::fs::remove_file(path).await;
            return Err(AppError::PayloadTooLarge);
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| AppError::Internal(format!("写入临时文件失败: {error}")))?;
    }
    file.flush()
        .await
        .map_err(|error| AppError::Internal(format!("写入临时文件失败: {error}")))?;
    drop(file);
    if size == 0 {
        let _ = tokio::fs::remove_file(path).await;
        return Err(AppError::BadRequest("上传文件为空".to_string()));
    }
    Ok(size)
}

fn extension_of(name: &str) -> Result<String, AppError> {
    Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| AppError::UnsupportedMediaType(name.to_string()))
}

fn map_multipart_error(error: &axum::extract::multipart::MultipartError) -> AppError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::PayloadTooLarge
    } else {
        AppError::BadRequest(format!("multipart 解析失败: {}", error.body_text()))
    }
}

fn map_conversion_error(error: ConversionError) -> AppError {
    match error {
        ConversionError::UnsupportedExtension(extension) => {
            AppError::UnsupportedMediaType(extension)
        }
        other => AppError::ConversionFailed(other.to_string()),
    }
}
