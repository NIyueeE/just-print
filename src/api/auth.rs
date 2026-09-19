//! Bearer 令牌中间件：常量时间比较，令牌未配置时服务不会启动。

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;
use subtle::ConstantTimeEq;

use crate::error::AppError;
use crate::state::AppState;

/// 校验 `Authorization: Bearer <token>`（scheme 大小写不敏感）；通过后放行。
pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let provided = bearer_token(&request);
    let expected = state.config.token.as_bytes();
    let authorized = provided.is_some_and(|provided| {
        let provided = provided.as_bytes();
        provided.len() == expected.len() && bool::from(expected.ct_eq(provided))
    });
    if authorized {
        Ok(next.run(request).await)
    } else {
        state.metrics.inc("auth_failures", &[]);
        Err(AppError::Unauthorized)
    }
}

/// 从 `Authorization` 头中提取 Bearer 令牌。
fn bearer_token(request: &Request) -> Option<&str> {
    let value = request.headers().get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    if scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() {
        Some(token)
    } else {
        None
    }
}
