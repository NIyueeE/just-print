//! 文档转 `PDF`：`PDF` 原样校验，`Markdown` 先渲染为带打印样式的
//! `HTML`，其余格式由镜像内置 `LibreOffice` 转换。
//! 白名单与镜像内 `LibreOffice` 7.4.7 注册的 `IMPORT` 过滤器一致。

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pulldown_cmark::{Event, Options as MarkdownOptions, Parser, Tag, TagEnd};
use thiserror::Error;
use tokio::process::Command;

/// 支持的上传扩展名（含镜像内 `LibreOffice` 可导入的全部格式与 Markdown）。
pub const SUPPORTED_EXTENSIONS: [&str; 172] = [
    "123", "602", "abw", "bmp", "cdr", "cgm", "cmx", "csv", "cwk", "dbf", "dif", "doc", "docm",
    "docx", "dot", "dotm", "dotx", "dps", "dpt", "dxf", "emf", "emz", "eps", "et", "ett", "fb2",
    "fh", "fh1", "fh10", "fh11", "fh2", "fh3", "fh4", "fh5", "fh6", "fh7", "fh8", "fh9", "fodg",
    "fodp", "fods", "fodt", "gif", "gnm", "gnumeric", "htm", "html", "hwp", "jfif", "jif", "jpe",
    "jpeg", "jpg", "key", "lrf", "lwp", "mcw", "md", "met", "mov", "mp", "mw", "mwd", "numbers",
    "nx^d", "odc", "odg", "odm", "odp", "ods", "odt", "otg", "oth", "otm", "otp", "ots", "ott",
    "p65", "pages", "pbm", "pcd", "pct", "pcx", "pdb", "pdf", "pgm", "pict", "pm", "pm6", "pmd",
    "png", "pot", "potm", "potx", "ppm", "pps", "ppsx", "ppt", "pptm", "pptx", "psd", "psw", "pub",
    "qxd", "qxt", "ras", "rtf", "sda", "sdc", "sdd", "sdw", "slk", "stc", "std", "sti", "stw",
    "svg", "svgz", "svm", "sxc", "sxd", "sxg", "sxi", "sxs", "sxw", "sylk", "tab", "tga", "tif",
    "tiff", "tsv", "txt", "vdx", "vsd", "vsdm", "vsdx", "wb1", "wb2", "wdb", "webp", "wk1", "wk3",
    "wk4", "wks", "wmf", "wmz", "wn", "wpd", "wpg", "wps", "wpt", "wq1", "wq2", "wri", "xbm",
    "xhtml", "xlc", "xlk", "xlm", "xls", "xlsb", "xlsm", "xlsx", "xlt", "xltm", "xltx", "xlw",
    "xml", "xpm", "zabw", "zip", "zmf",
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
    /// Markdown 不是合法的 UTF-8。
    #[error("Markdown 不是有效的 UTF-8 文本")]
    InvalidUtf8,
    /// 写入渲染后的 HTML 失败。
    #[error("写入 Markdown 渲染结果失败: {0}")]
    Write(#[source] std::io::Error),
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

/// Markdown 渲染用的打印样式（`LibreOffice` HTML 导入支持的基础 CSS 子集）。
const MARKDOWN_STYLE: &str = r#"
body { font-family: "Noto Sans CJK SC", "Liberation Sans", sans-serif; font-size: 11pt; line-height: 1.45; color: #282828; }
h1, h2, h3, h4, h5, h6 { font-family: "Noto Sans CJK SC", "Liberation Sans", sans-serif; }
h1 { font-size: 18pt; border-bottom: 1px solid #a89984; padding-bottom: 2pt; margin-top: 14pt; }
h2 { font-size: 15pt; margin-top: 12pt; }
h3 { font-size: 13pt; margin-top: 10pt; }
h4, h5, h6 { font-size: 11pt; margin-top: 8pt; }
pre { font-family: "Liberation Mono", monospace; font-size: 9pt; background-color: #f2ede2; border: 1px solid #d5c4a1; padding: 6pt; white-space: pre-wrap; }
code { font-family: "Liberation Mono", monospace; font-size: 9pt; background-color: #f2ede2; }
blockquote { color: #504945; border-left: 3px solid #a89984; padding-left: 8pt; margin-left: 0; }
table { border-collapse: collapse; }
th, td { border: 1px solid #a89984; padding: 3pt 8pt; }
th { background-color: #f2ede2; }
a { color: #076678; text-decoration: underline; }
hr { border: none; border-top: 1px solid #a89984; }
"#;

/// 将文本转义为 HTML 实体（用于正文与属性值）。
fn push_escaped(output: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            _ => output.push(ch),
        }
    }
}

/// `LibreOffice` 7.4 的 `HTML` 导入会吞掉正文里第一个文本块（段落或标题），
/// 前置一个不可见的占位段落让它吞，保住真实内容。
const LO_GUARD_PARAGRAPH: &str = "<p style=\"font-size: 1pt; color: #ffffff;\">&nbsp;</p>";

/// 判断标签是否为块级元素。
fn is_block_start(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::List(_)
            | Tag::Table(_)
            | Tag::HtmlBlock
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
    )
}

/// 输出标签的开始标记（表格表头状态由 `in_table_header` 维护）。
fn push_markdown_start(output: &mut String, tag: Tag<'_>, in_table_header: &mut bool) {
    match tag {
        Tag::Paragraph => output.push_str("<p>"),
        Tag::Heading { level, .. } => {
            let _ = write!(output, "<{level}>");
        }
        Tag::BlockQuote(_) => output.push_str("<blockquote>"),
        Tag::CodeBlock(_) => output.push_str("<pre><code>"),
        Tag::List(start) => {
            if let Some(number) = start {
                let _ = write!(output, "<ol start=\"{number}\">");
            } else {
                output.push_str("<ul>");
            }
        }
        Tag::Item => output.push_str("<li>"),
        Tag::Table(_) => output.push_str("<table>"),
        Tag::TableHead => {
            *in_table_header = true;
            output.push_str("<thead><tr>");
        }
        Tag::TableRow => output.push_str("<tr>"),
        Tag::TableCell => {
            output.push_str(if *in_table_header { "<th>" } else { "<td>" });
        }
        Tag::Emphasis => output.push_str("<em>"),
        Tag::Strong => output.push_str("<strong>"),
        Tag::Strikethrough => output.push_str("<del>"),
        Tag::Superscript => output.push_str("<sup>"),
        Tag::Subscript => output.push_str("<sub>"),
        Tag::Link { dest_url, .. } => {
            output.push_str("<a href=\"");
            push_escaped(output, &dest_url);
            output.push_str("\">");
        }
        Tag::Image { dest_url, .. } => {
            output.push_str("<img src=\"");
            push_escaped(output, &dest_url);
            output.push_str("\" alt=\"");
        }
        Tag::HtmlBlock
        | Tag::FootnoteDefinition(_)
        | Tag::DefinitionList
        | Tag::DefinitionListTitle
        | Tag::DefinitionListDefinition
        | Tag::MetadataBlock(_) => {}
    }
}

/// 输出标签的结束标记（表格表头状态由 `in_table_header` 维护）。
fn push_markdown_end(output: &mut String, tag_end: TagEnd, in_table_header: &mut bool) {
    match tag_end {
        TagEnd::Paragraph => output.push_str("</p>"),
        TagEnd::Heading(level) => {
            let _ = write!(output, "</{level}>");
        }
        TagEnd::BlockQuote(_) => output.push_str("</blockquote>"),
        TagEnd::CodeBlock => output.push_str("</code></pre>"),
        TagEnd::List(ordered) => {
            output.push_str(if ordered { "</ol>" } else { "</ul>" });
        }
        TagEnd::Item => output.push_str("</li>"),
        TagEnd::Table => output.push_str("</table>"),
        TagEnd::TableHead => {
            *in_table_header = false;
            output.push_str("</tr></thead>");
        }
        TagEnd::TableRow => output.push_str("</tr>"),
        TagEnd::TableCell => {
            output.push_str(if *in_table_header { "</th>" } else { "</td>" });
        }
        TagEnd::Emphasis => output.push_str("</em>"),
        TagEnd::Strong => output.push_str("</strong>"),
        TagEnd::Strikethrough => output.push_str("</del>"),
        TagEnd::Superscript => output.push_str("</sup>"),
        TagEnd::Subscript => output.push_str("</sub>"),
        TagEnd::Link => output.push_str("</a>"),
        TagEnd::Image => output.push_str("\">"),
        TagEnd::HtmlBlock
        | TagEnd::FootnoteDefinition
        | TagEnd::DefinitionList
        | TagEnd::DefinitionListTitle
        | TagEnd::DefinitionListDefinition
        | TagEnd::MetadataBlock(_) => {}
    }
}

/// 将 `CommonMark` 事件流渲染为 `HTML` 正文（原样 `HTML` 一律转义）。
fn render_markdown_body(markdown: &str) -> String {
    let mut options = MarkdownOptions::empty();
    options.insert(
        MarkdownOptions::ENABLE_TABLES
            | MarkdownOptions::ENABLE_STRIKETHROUGH
            | MarkdownOptions::ENABLE_TASKLISTS,
    );
    let mut output = String::new();
    let mut in_table_header = false;
    let mut first_block = true;
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => {
                if first_block && is_block_start(&tag) {
                    if matches!(tag, Tag::Paragraph | Tag::Heading { .. }) {
                        output.push_str(LO_GUARD_PARAGRAPH);
                    }
                    first_block = false;
                }
                push_markdown_start(&mut output, tag, &mut in_table_header);
            }
            Event::End(tag_end) => push_markdown_end(&mut output, tag_end, &mut in_table_header),
            Event::Text(text) => push_escaped(&mut output, &text),
            Event::Code(text) => {
                output.push_str("<code>");
                push_escaped(&mut output, &text);
                output.push_str("</code>");
            }
            Event::Html(html) | Event::InlineHtml(html) => push_escaped(&mut output, &html),
            Event::SoftBreak => output.push('\n'),
            Event::HardBreak => output.push_str("<br>\n"),
            Event::Rule => output.push_str("<hr>\n"),
            Event::TaskListMarker(checked) => {
                output.push_str(if checked { "☑ " } else { "☐ " });
            }
            Event::FootnoteReference(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }
    output
}

/// 将 Markdown 渲染为带打印样式的 HTML 文件，返回生成的 HTML 路径。
///
/// 原样 HTML 会被转义，避免上传内容注入不受控的样式或结构。
///
/// # Errors
///
/// 源文件读取失败、不是 UTF-8，或写入 HTML 失败时返回 [`ConversionError`]。
async fn render_markdown_to_html(
    source: &Path,
    output_dir: &Path,
) -> Result<PathBuf, ConversionError> {
    let bytes = tokio::fs::read(source)
        .await
        .map_err(ConversionError::Read)?;
    let text = String::from_utf8(bytes).map_err(|_| ConversionError::InvalidUtf8)?;
    let body = render_markdown_body(&text);
    let stem = source
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("document");
    let html_path = output_dir.join(format!("{stem}.html"));
    let document = format!(
        "<!doctype html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n\
         <style>\n{MARKDOWN_STYLE}\n</style>\n</head>\n<body>\n{body}\n</body>\n</html>\n"
    );
    tokio::fs::write(&html_path, document)
        .await
        .map_err(ConversionError::Write)?;
    Ok(html_path)
}

/// 尽力删除转换失败时的残留产物：`UserInstallation` 配置目录、
/// 中间文件（`Markdown` 渲染出的 `HTML`）与可能的部分输出 `PDF`。
async fn cleanup_artifacts(profile_dir: &Path, intermediate: Option<&Path>, output: Option<&Path>) {
    let _ = tokio::fs::remove_dir_all(profile_dir).await;
    if let Some(path) = intermediate {
        let _ = tokio::fs::remove_file(path).await;
    }
    if let Some(path) = output {
        let _ = tokio::fs::remove_file(path).await;
    }
}

/// 将上传文件转换为 PDF。
///
/// `.pdf` 校验 `%PDF-` 魔数后原样返回；其它格式调用
/// `soffice --headless --convert-to pdf` 转换；`.md` 会先渲染为带样式的
/// `HTML` 再交给 `LibreOffice`，避免以纯文本源码形式打印。每次转换使用
/// 独立的 `UserInstallation` 目录，避免并行实例争用配置锁。
/// 最终打印语言由 CUPS 过滤链根据驱动 / PPD 决定，应用层不干预。
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

    let (convert_source, intermediate) = if extension == "md" {
        let html_path = render_markdown_to_html(source, output_dir).await?;
        (html_path.clone(), Some(html_path))
    } else {
        (source.to_path_buf(), None)
    };

    let profile_dir = output_dir.join(format!("lo-profile-{}", crate::ids::new_id()));
    let profile_url = format!("file://{}", profile_dir.display());
    let mut child = Command::new("soffice")
        .arg(format!("-env:UserInstallation={profile_url}"))
        .arg("--headless")
        .arg("--convert-to")
        .arg("pdf")
        .arg("--outdir")
        .arg(output_dir)
        .arg(&convert_source)
        .kill_on_drop(true)
        .spawn()
        .map_err(ConversionError::Spawn)?;

    let stem = convert_source
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("document");
    let output = output_dir.join(format!("{stem}.pdf"));

    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Err(_) => {
            let _ = child.kill().await;
            cleanup_artifacts(&profile_dir, intermediate.as_deref(), Some(&output)).await;
            return Err(ConversionError::Timeout(timeout));
        }
        Ok(result) => match result {
            Ok(status) => status,
            Err(error) => {
                cleanup_artifacts(&profile_dir, intermediate.as_deref(), Some(&output)).await;
                return Err(ConversionError::Wait(error));
            }
        },
    };
    if !status.success() {
        cleanup_artifacts(&profile_dir, intermediate.as_deref(), Some(&output)).await;
        return Err(ConversionError::NonZeroExit(status.code()));
    }

    let Ok(metadata) = tokio::fs::metadata(&output).await else {
        cleanup_artifacts(&profile_dir, intermediate.as_deref(), Some(&output)).await;
        return Err(ConversionError::MissingOutput);
    };
    if metadata.len() == 0 {
        cleanup_artifacts(&profile_dir, intermediate.as_deref(), Some(&output)).await;
        return Err(ConversionError::EmptyOutput);
    }
    cleanup_artifacts(&profile_dir, intermediate.as_deref(), None).await;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use crate::ids;

    use super::{SUPPORTED_EXTENSIONS, convert_to_pdf, render_markdown_to_html};

    #[tokio::test]
    async fn markdown_renders_headings_code_and_tables() {
        let temp = std::env::temp_dir().join(format!("just-print-test-{}", ids::new_id()));
        assert!(std::fs::create_dir_all(&temp).is_ok());
        let source = temp.join("notes.md");
        let markdown =
            "# 标题\n\n**加粗**\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        assert!(std::fs::write(&source, markdown).is_ok());
        let result = render_markdown_to_html(&source, &temp).await;
        assert!(result.is_ok());
        let Ok(html_path) = result else {
            return;
        };
        let html = std::fs::read_to_string(&html_path);
        assert!(html.is_ok());
        let Ok(html) = html else {
            return;
        };
        assert!(html.contains("<h1"));
        assert!(html.contains("加粗"));
        assert!(html.contains("<pre"));
        assert!(html.contains("fn main()"));
        assert!(html.contains("<table>"));
        assert!(html.contains("font-size: 18pt"));
        assert!(html.contains("font-size: 1pt; color: #ffffff;"));
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[tokio::test]
    async fn markdown_escapes_raw_html() {
        let temp = std::env::temp_dir().join(format!("just-print-test-{}", ids::new_id()));
        assert!(std::fs::create_dir_all(&temp).is_ok());
        let source = temp.join("notes.md");
        assert!(std::fs::write(&source, "# 标题\n\n<script>alert(1)</script>\n").is_ok());
        let result = render_markdown_to_html(&source, &temp).await;
        assert!(result.is_ok());
        let Ok(html_path) = result else {
            return;
        };
        let html = std::fs::read_to_string(&html_path);
        assert!(html.is_ok());
        let Ok(html) = html else {
            return;
        };
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        let _ = std::fs::remove_dir_all(&temp);
    }

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
        assert!(SUPPORTED_EXTENSIONS.contains(&"doc"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"xls"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"ppt"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"csv"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"html"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"rtf"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"txt"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"png"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"jpg"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"webp"));
    }
}
