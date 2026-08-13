//! 文件上传与 PDF 预览。

use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::multipart::MultipartRejection;
use axum::extract::{Multipart, Path as AxumPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
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
    let mut upload: Option<(String, Bytes)> = None;
    while let Some(field) = multipart
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
        let bytes = field
            .bytes()
            .await
            .map_err(|error| map_multipart_error(&error))?;
        if bytes.len() > state.config.max_upload_bytes {
            return Err(AppError::PayloadTooLarge);
        }
        if bytes.is_empty() {
            return Err(AppError::BadRequest("上传文件为空".to_string()));
        }
        upload = Some((name, bytes));
        break;
    }

    let (name, bytes) = upload.ok_or_else(|| AppError::BadRequest("缺少 file 字段".to_string()))?;
    let extension = Path::new(&name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| AppError::UnsupportedMediaType(name.clone()))?;
    if !conversion::SUPPORTED_EXTENSIONS.contains(&extension.as_str()) {
        return Err(AppError::UnsupportedMediaType(name.clone()));
    }

    let id = ids::new_id();
    let source_path = state.temp_dir.join(format!("{id}-upload.{extension}"));
    tokio::fs::write(&source_path, &bytes)
        .await
        .map_err(|error| AppError::Internal(format!("写入临时文件失败: {error}")))?;

    let permit = state
        .conversion_slots
        .acquire()
        .await
        .map_err(|_| AppError::Internal("转换通道已关闭".to_string()))?;
    let result = conversion::convert_to_pdf(
        &source_path,
        &state.temp_dir,
        state.config.conversion_timeout,
    )
    .await;
    drop(permit);

    let pdf_path = match result {
        Ok(path) => path,
        Err(error) => {
            let _ = tokio::fs::remove_file(&source_path).await;
            return Err(map_conversion_error(error));
        }
    };
    if pdf_path != source_path {
        let _ = tokio::fs::remove_file(&source_path).await;
    }
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    state.files.insert(id.clone(), pdf_path);
    Ok((StatusCode::CREATED, Json(UploadResponse { id, name, size })))
}

/// 返回转换后的 PDF 预览。
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

fn map_multipart_error(error: &axum::extract::multipart::MultipartError) -> AppError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::PayloadTooLarge
    } else {
        AppError::BadRequest(format!("multipart 解析失败: {error}"))
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
