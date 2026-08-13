//! 统一 Web API 错误类型与 HTTP 响应。

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use thiserror::Error;

/// 业务 API 错误；`IntoResponse` 输出统一的 JSON 错误信封。
#[derive(Debug, Error)]
pub enum AppError {
    /// 缺少或错误的 `Bearer` 令牌。
    #[error("未提供或错误的访问令牌")]
    Unauthorized,
    /// 请求体/参数不合法。
    #[error("{0}")]
    BadRequest(String),
    /// 文件不存在或已失效。
    #[error("文件不存在或已失效，服务可能已重启，请重新上传")]
    FileNotFound,
    /// 任务不存在或已被 CUPS 清理。
    #[error("任务不存在或已失效（可能已被 CUPS 清理）")]
    JobNotFound,
    /// 请求的 API 接口不存在。
    #[error("接口不存在")]
    NotFound,
    /// 打印机不存在。
    #[error("打印机不存在或已移除")]
    PrinterNotFound,
    /// 打印机存在但当前不可用。
    #[error("{0}")]
    PrinterUnavailable(String),
    /// 控制参数不是能力查询返回的合法值。
    #[error("控制参数不合法: {0}")]
    InvalidControls(String),
    /// 上传格式不在支持列表。
    #[error("上传格式不支持: {0}")]
    UnsupportedMediaType(String),
    /// 上传超过大小限制。
    #[error("请求体过大（超过 64 MiB）")]
    PayloadTooLarge,
    /// `LibreOffice` 转换失败。
    #[error("文档转换失败: {0}")]
    ConversionFailed(String),
    /// 内部错误。
    #[error("内部错误: {0}")]
    Internal(String),
}

impl From<crate::cups::CupsError> for AppError {
    fn from(error: crate::cups::CupsError) -> Self {
        Self::Internal(format!("CUPS 不可用: {error}"))
    }
}

impl AppError {
    /// 错误对应的 HTTP 状态码。
    fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::BadRequest(_) | Self::InvalidControls(_) => StatusCode::BAD_REQUEST,
            Self::FileNotFound | Self::JobNotFound | Self::PrinterNotFound | Self::NotFound => {
                StatusCode::NOT_FOUND
            }
            Self::PrinterUnavailable(_) => StatusCode::CONFLICT,
            Self::UnsupportedMediaType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::ConversionFailed(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// 稳定的机器可读错误码。
    fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::BadRequest(_) => "bad_request",
            Self::FileNotFound | Self::JobNotFound | Self::PrinterNotFound | Self::NotFound => {
                "not_found"
            }
            Self::PrinterUnavailable(_) => "printer_unavailable",
            Self::InvalidControls(_) => "invalid_controls",
            Self::UnsupportedMediaType(_) => "unsupported_media_type",
            Self::PayloadTooLarge => "payload_too_large",
            Self::ConversionFailed(_) => "conversion_failed",
            Self::Internal(_) => "internal",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = json!({ "error": { "code": self.code(), "message": self.to_string() } });
        (status, Json(body)).into_response()
    }
}
