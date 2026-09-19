//! 支持的上传格式清单（前后端单一来源）。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::conversion::SUPPORTED_EXTENSIONS;
use crate::state::AppState;

/// 上传格式响应。
#[derive(Debug, Serialize)]
pub struct FormatsResponse {
    /// 支持的扩展名（小写，不含点）。
    pub extensions: Vec<&'static str>,
    /// 单文件上传大小上限（字节）。
    pub max_upload_bytes: usize,
}

/// 返回合法的上传扩展名与大小上限。
pub async fn list(State(state): State<Arc<AppState>>) -> Json<FormatsResponse> {
    Json(FormatsResponse {
        extensions: SUPPORTED_EXTENSIONS.to_vec(),
        max_upload_bytes: state.config.max_upload_bytes,
    })
}
