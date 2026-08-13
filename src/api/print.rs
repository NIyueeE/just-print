//! 提交打印任务。

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::cups::{CupsError, OptionKind};
use crate::error::AppError;
use crate::state::AppState;

/// 打印请求。
#[derive(Debug, Deserialize)]
pub struct PrintRequest {
    /// 已上传文件 id。
    pub file_id: String,
    /// 目标 CUPS 打印机名。
    pub printer_id: String,
    /// CUPS 选项（可选，键/值必须来自打印机选项查询）。
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

/// 打印提交响应。
#[derive(Debug, Serialize)]
pub struct PrintResponse {
    /// CUPS 任务 id（`printer-job` 形式）。
    pub job_id: String,
}

/// 校验选项并直接提交给 CUPS，返回 CUPS 任务 id。
pub async fn submit(
    State(state): State<Arc<AppState>>,
    body: Result<Json<PrintRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<PrintResponse>), AppError> {
    let Json(request) = body
        .map_err(|error| AppError::BadRequest(format!("JSON 解析失败: {}", error.body_text())))?;
    let printers = state.cups.list_printers().await.map_err(AppError::from)?;
    if !printers
        .iter()
        .any(|printer| printer.name == request.printer_id)
    {
        return Err(AppError::PrinterNotFound);
    }

    let printer_options = state
        .cups
        .list_options(&request.printer_id)
        .await
        .map_err(AppError::from)?;
    validate_options(&printer_options, &request.options)?;

    let file = state
        .files
        .guard(&request.file_id)
        .ok_or(AppError::FileNotFound)?;
    let job_id = state
        .cups
        .submit(&request.printer_id, file.path(), &request.options)
        .await
        .map_err(map_submit_error)?;
    Ok((StatusCode::ACCEPTED, Json(PrintResponse { job_id })))
}

fn validate_options(
    printer_options: &BTreeMap<String, crate::cups::OptionSpec>,
    requested: &BTreeMap<String, String>,
) -> Result<(), AppError> {
    for (key, value) in requested {
        let spec = printer_options
            .get(key)
            .ok_or_else(|| AppError::InvalidControls(format!("打印机未提供参数 {key}")))?;
        let valid = match &spec.kind {
            OptionKind::Enumerated { values } => values.iter().any(|candidate| candidate == value),
            OptionKind::Range { min, max } => value
                .parse::<i64>()
                .is_ok_and(|number| number >= *min && number <= *max),
        };
        if !valid {
            return Err(AppError::InvalidControls(format!(
                "参数 {key} 的取值 {value} 不在合法范围内"
            )));
        }
    }
    Ok(())
}

fn map_submit_error(error: CupsError) -> AppError {
    match error {
        CupsError::Command { stderr, .. } => {
            let lower = stderr.to_ascii_lowercase();
            if lower.contains("unable to connect") || lower.contains("connection failed") {
                AppError::Internal(format!("CUPS 不可用: {stderr}"))
            } else {
                AppError::PrinterUnavailable(stderr)
            }
        }
        other => AppError::Internal(format!("CUPS 提交失败: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::cups::{OptionKind, OptionSpec};

    use super::validate_options;

    #[test]
    fn rejects_unknown_and_invalid_option_values() {
        let mut options = BTreeMap::new();
        options.insert(
            "Duplex".to_string(),
            OptionSpec {
                default: Some("None".to_string()),
                kind: OptionKind::Enumerated {
                    values: vec![
                        "DuplexTumble".to_string(),
                        "DuplexNoTumble".to_string(),
                        "None".to_string(),
                    ],
                },
            },
        );
        options.insert(
            "copies".to_string(),
            OptionSpec {
                default: None,
                kind: OptionKind::Range { min: 1, max: 999 },
            },
        );

        let mut requested = BTreeMap::new();
        requested.insert("Duplex".to_string(), "DuplexTumble".to_string());
        requested.insert("copies".to_string(), "5".to_string());
        assert!(validate_options(&options, &requested).is_ok());

        requested.insert("Duplex".to_string(), "SIDEWAYS".to_string());
        assert!(validate_options(&options, &requested).is_err());
        requested.insert("copies".to_string(), "9999".to_string());
        assert!(validate_options(&options, &requested).is_err());
        requested.remove("Duplex");
        requested.insert("HACK".to_string(), "1".to_string());
        assert!(validate_options(&options, &requested).is_err());
    }
}
