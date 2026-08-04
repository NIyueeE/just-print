//! 文档转 `PDF`：`PDF` 原样校验，其余格式由镜像内置 `LibreOffice` 转换。

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
    /// 启动 gs 失败。
    #[error("启动 Ghostscript 失败: {0}")]
    GsSpawn(#[source] io::Error),
    /// 与 gs 子进程通信失败。
    #[error("与 Ghostscript 通信失败: {0}")]
    GsIo(#[source] io::Error),
    /// gs 转换超时。
    #[error("Ghostscript 转换超时（{0:?}）")]
    GsTimeout(Duration),
    /// gs 非零退出。
    #[error("Ghostscript 转换失败，退出码: {0:?}")]
    GsNonZeroExit(Option<i32>),
    /// gs 输出为空。
    #[error("Ghostscript 输出为空")]
    GsEmptyOutput,
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

/// 将 PDF 字节流转换为 PostScript（Ghostscript `ps2write`），供不支持 PDF 的
/// 打印机回退使用。
///
/// # Errors
///
/// gs 缺失、管道不可用、转换超时、非零退出或输出为空时返回 [`ConversionError`]。
pub async fn convert_pdf_to_postscript(
    pdf: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, ConversionError> {
    convert_pdf_with_gs(pdf, "ps2write", "ps", timeout).await
}

/// 将 PDF 字节流转换为 PCL5e（Ghostscript `ljet4`），供不支持 PDF 的 PCL
/// 打印机使用。
///
/// # Errors
///
/// gs 缺失、管道不可用、转换超时、非零退出或输出为空时返回 [`ConversionError`]。
pub async fn convert_pdf_to_pcl(pdf: &[u8], timeout: Duration) -> Result<Vec<u8>, ConversionError> {
    convert_pdf_with_gs(pdf, "ljet4", "pcl", timeout).await
}

/// 用 Ghostscript 把 PDF 转换为指定输出设备的数据流。
///
/// PDF 从 stdin 输入，输出写入唯一的临时文件后读回并清理，避免
/// `-sOutputFile=-` 把 gs 诊断信息混入 stdout 数据流。
async fn convert_pdf_with_gs(
    pdf: &[u8],
    device: &str,
    extension: &str,
    timeout: Duration,
) -> Result<Vec<u8>, ConversionError> {
    let output_path =
        std::env::temp_dir().join(format!("just-print-{}.{extension}", crate::ids::new_id()));
    let result = tokio::time::timeout(timeout, async {
        let mut child = Command::new("gs")
            .args([
                "-q",
                "-dNOPAUSE",
                "-dBATCH",
                &format!("-sDEVICE={device}"),
                &format!("-sOutputFile={}", output_path.display()),
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(ConversionError::GsSpawn)?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ConversionError::GsSpawn(io::Error::other("gs stdin 不可用")))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| ConversionError::GsSpawn(io::Error::other("gs stderr 不可用")))?;
        let stderr_task = tokio::spawn(async move {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text).await;
            text
        });
        stdin.write_all(pdf).await.map_err(ConversionError::GsIo)?;
        drop(stdin);
        let status = child.wait().await.map_err(ConversionError::GsIo)?;
        let _stderr_text = stderr_task.await.unwrap_or_default();
        if !status.success() {
            return Err(ConversionError::GsNonZeroExit(status.code()));
        }
        let output = tokio::fs::read(&output_path)
            .await
            .map_err(ConversionError::GsIo)?;
        if output.is_empty() {
            return Err(ConversionError::GsEmptyOutput);
        }
        Ok(output)
    })
    .await;
    let _ = tokio::fs::remove_file(&output_path).await;
    match result {
        Err(_) => Err(ConversionError::GsTimeout(timeout)),
        Ok(inner) => inner,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::ids;

    use super::{
        SUPPORTED_EXTENSIONS, convert_pdf_to_pcl, convert_pdf_to_postscript, convert_to_pdf,
    };

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

    #[tokio::test]
    async fn converts_pdf_to_postscript_when_gs_available() {
        let Ok(probe) = tokio::process::Command::new("gs")
            .arg("--version")
            .output()
            .await
        else {
            return;
        };
        if !probe.status.success() {
            return;
        }

        let pdf = b"%PDF-1.4\n1 0 obj<</Type/Catalog>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
        let result = convert_pdf_to_postscript(pdf, Duration::from_secs(20)).await;
        assert!(result.is_ok(), "gs conversion failed: {result:?}");
        let Ok(ps) = result else {
            return;
        };
        assert!(ps.starts_with(b"%!PS"));
    }

    #[tokio::test]
    async fn converts_pdf_to_pcl_when_gs_available() {
        let Ok(probe) = tokio::process::Command::new("gs")
            .arg("--version")
            .output()
            .await
        else {
            return;
        };
        if !probe.status.success() {
            return;
        }

        let temp = std::env::temp_dir().join(format!("just-print-pcl-test-{}", ids::new_id()));
        assert!(std::fs::create_dir_all(&temp).is_ok());
        let ps_path = temp.join("page.ps");
        let pdf_path = temp.join("page.pdf");
        assert!(std::fs::write(
            &ps_path,
            b"%!PS\n/Helvetica findfont 12 scalefont setfont\n72 720 moveto (x) show\nshowpage\n",
        )
        .is_ok());
        let Ok(status) = tokio::process::Command::new("gs")
            .args([
                "-q",
                "-dNOPAUSE",
                "-dBATCH",
                "-sDEVICE=pdfwrite",
                &format!("-sOutputFile={}", pdf_path.display()),
                &ps_path.to_string_lossy(),
            ])
            .status()
            .await
        else {
            let _ = std::fs::remove_dir_all(&temp);
            return;
        };
        if !status.success() {
            let _ = std::fs::remove_dir_all(&temp);
            return;
        }
        let Ok(pdf) = tokio::fs::read(&pdf_path).await else {
            let _ = std::fs::remove_dir_all(&temp);
            return;
        };
        let result = convert_pdf_to_pcl(&pdf, Duration::from_secs(20)).await;
        let _ = std::fs::remove_dir_all(&temp);
        assert!(result.is_ok(), "gs pcl conversion failed: {result:?}");
        let Ok(pcl) = result else {
            return;
        };
        assert!(pcl.starts_with(b"\x1bE"));
    }
}
