//! Bearer 令牌中间件：常量时间比较，令牌未配置时服务不会启动。

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;
use subtle::ConstantTimeEq;

use crate::error::AppError;
use crate::state::AppState;

/// 校验 `Authorization: Bearer <token>`；通过后放行请求。
pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let header = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let Some(provided) = header.and_then(|value| value.strip_prefix("Bearer ")) else {
        return Err(AppError::Unauthorized);
    };
    let expected = state.config.token.as_bytes();
    let provided = provided.as_bytes();
    let matches = provided.len() == expected.len() && expected.ct_eq(provided).into();
    if matches {
        Ok(next.run(request).await)
    } else {
        Err(AppError::Unauthorized)
    }
}
