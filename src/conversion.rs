//! 文档转 `PDF`：`PDF` 原样校验，其余格式由镜像内置 `LibreOffice` 转换。

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;
use tokio::process::Command;

/// 支持的上传扩展名。
pub const SUPPORTED_EXTENSIONS: [&str; 9] = [
    "pdf", "docx", "xlsx", "pptx", "odt", "ods", "odp", "md", "txt",
];

/// `LibreOffice` 并发转换上限（`CPU` 密集操作）。
pub const CONVERSION_SLOTS: usize = 2;

/// 文档转换错误。
#[derive(Debug, Error)]
pub enum ConversionError {
    /// 扩展名不在支持列表。
    #[error("不支持的扩展名: {0}")]
    UnsupportedExtension(String),
    /// PDF 魔数校验失败。
    #[error("不是有效的 PDF 文件")]
    InvalidPdf,
    /// 读取上传文件失败。
    #[error("读取上传文件失败: {0}")]
    Read(#[source] std::io::Error),
    /// 启动 soffice 失败。
    #[error("启动 soffice 失败: {0}")]
    Spawn(#[source] std::io::Error),
    /// 等待 soffice 退出失败。
    #[error("等待 soffice 失败: {0}")]
    Wait(#[source] std::io::Error),
    /// 转换超时。
    #[error("转换超时（{0:?}）")]
    Timeout(Duration),
    /// soffice 非零退出。
    #[error("soffice 转换失败，退出码: {0:?}")]
    NonZeroExit(Option<i32>),
    /// 未找到输出 PDF。
    #[error("未找到转换输出 PDF")]
    MissingOutput,
    /// 输出 PDF 为空。
    #[error("转换输出为空")]
    EmptyOutput,
}

/// 将上传文件转换为 PDF。
///
/// `.pdf` 校验 `%PDF-` 魔数后原样返回；其它格式调用
/// `soffice --headless --convert-to pdf` 转换。每次转换使用独立的
/// `UserInstallation` 目录，避免并行实例争用配置锁。
///
/// # Errors
///
/// 扩展名不支持、PDF 魔数不合法、soffice 缺失/退出码非零/超时、
/// 或未生成有效输出时返回 [`ConversionError`]。
pub async fn convert_to_pdf(
    source: &Path,
    output_dir: &Path,
    timeout: Duration,
) -> Result<PathBuf, ConversionError> {
    let extension = source
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ConversionError::UnsupportedExtension("无扩展名".to_string()))?;
    if !SUPPORTED_EXTENSIONS.contains(&extension.as_str()) {
        return Err(ConversionError::UnsupportedExtension(extension));
    }

    if extension == "pdf" {
        let bytes = tokio::fs::read(source)
            .await
            .map_err(ConversionError::Read)?;
        if !bytes.starts_with(b"%PDF-") {
            return Err(ConversionError::InvalidPdf);
        }
        return Ok(source.to_path_buf());
    }

    let profile_dir = output_dir.join(format!("lo-profile-{}", crate::ids::new_id()));
    let profile_url = format!("file://{}", profile_dir.display());
    let mut child = Command::new("soffice")
        .arg(format!("-env:UserInstallation={profile_url}"))
        .arg("--headless")
        .arg("--convert-to")
        .arg("pdf")
        .arg("--outdir")
        .arg(output_dir)
        .arg(source)
        .kill_on_drop(true)
        .spawn()
        .map_err(ConversionError::Spawn)?;

    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Err(_) => {
            let _ = child.kill().await;
            let _ = tokio::fs::remove_dir_all(&profile_dir).await;
            return Err(ConversionError::Timeout(timeout));
        }
        Ok(result) => result.map_err(ConversionError::Wait)?,
    };
    let _ = tokio::fs::remove_dir_all(&profile_dir).await;
    if !status.success() {
        return Err(ConversionError::NonZeroExit(status.code()));
    }

    let stem = source
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("document");
    let output = output_dir.join(format!("{stem}.pdf"));
    let metadata = tokio::fs::metadata(&output)
        .await
        .map_err(|_| ConversionError::MissingOutput)?;
    if metadata.len() == 0 {
        return Err(ConversionError::EmptyOutput);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use crate::ids;

    use super::{SUPPORTED_EXTENSIONS, convert_to_pdf};

    #[tokio::test]
    async fn pdf_passthrough_requires_magic() {
        let temp = std::env::temp_dir().join(format!("just-print-test-{}", ids::new_id()));
        assert!(std::fs::create_dir_all(&temp).is_ok());
        let source = temp.join("sample.pdf");
        assert!(std::fs::write(&source, b"%PDF-1.7 not really a pdf").is_ok());
        let result = convert_to_pdf(&source, &temp, std::time::Duration::from_secs(10)).await;
        assert!(result.is_ok());
        let invalid = temp.join("broken.pdf");
        assert!(std::fs::write(&invalid, b"not a pdf").is_ok());
        assert!(
            convert_to_pdf(&invalid, &temp, std::time::Duration::from_secs(10))
                .await
                .is_err()
        );
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn extension_list_is_stable() {
        assert!(SUPPORTED_EXTENSIONS.contains(&"pdf"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"docx"));
    }
}
