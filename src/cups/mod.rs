//! CUPS 集成：打印机枚举、选项读取、任务提交与状态查询。
//!
//! 通过 `lp` / `lpstat` / `lpoptions` 命令行工具与 CUPS 交互，应用层不再
//! 实现 PJL 会话、设备发现或每打印机队列。所有子进程强制使用 `C` locale，
//! 保证输出格式稳定。

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Output;
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;
use tokio::process::Command;

/// 默认 CUPS 服务地址。
pub const DEFAULT_CUPS_URI: &str = "http://127.0.0.1:631";
/// 单条 CUPS 命令超时。
pub const CUPS_TIMEOUT: Duration = Duration::from_secs(10);

/// CUPS 集成错误。
#[derive(Debug, Error)]
pub enum CupsError {
    /// 子进程启动失败。
    #[error("CUPS 命令启动失败: {0}")]
    Spawn(String),
    /// 子进程非零退出。
    #[error("CUPS 命令失败（{program}）: {stderr}")]
    Command {
        /// 失败的命令名。
        program: &'static str,
        /// CUPS 输出到 stderr 的错误信息。
        stderr: String,
    },
    /// 子进程超时。
    #[error("CUPS 命令超时（{0}）")]
    Timeout(&'static str),
    /// 输出解析失败。
    #[error("CUPS 输出解析失败: {0}")]
    Parse(String),
}

/// CUPS 客户端；所有调用均通过子进程完成。
#[derive(Debug, Clone)]
pub struct CupsClient {
    server: String,
    scheme: String,
    timeout: Duration,
}

impl CupsClient {
    /// 创建 CUPS 客户端。
    ///
    /// `server` 为 `host:port` 形式，将写入子进程的 `CUPS_SERVER` 环境变量；
    /// `scheme` 为 `ipp` / `ipps`，用于构造 IPP 查询 URI。
    #[must_use]
    pub fn new(server: String, scheme: String) -> Self {
        Self {
            server,
            scheme,
            timeout: CUPS_TIMEOUT,
        }
    }

    /// 列出 CUPS 中已配置的打印机（含状态与描述）。
    ///
    /// # Errors
    ///
    /// `lpstat` 失败或输出无法解析时返回 [`CupsError`]。
    pub async fn list_printers(&self) -> Result<Vec<PrinterInfo>, CupsError> {
        let output = self.run("lpstat", &["-p", "-l"]).await?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_printers(&stdout).map_err(CupsError::Parse)
    }

    /// 读取打印机的 `lpoptions -l` 选项表。
    ///
    /// # Errors
    ///
    /// `lpoptions` 失败或输出无法解析时返回 [`CupsError`]。
    pub async fn list_options(
        &self,
        printer: &str,
    ) -> Result<BTreeMap<String, OptionSpec>, CupsError> {
        let output = self.run("lpoptions", &["-p", printer, "-l"]).await?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_options(&stdout))
    }

    /// 通过 `lp` 提交打印任务，返回 CUPS 任务 id（`printer-job` 形式）。
    ///
    /// # Errors
    ///
    /// `lp` 失败、超时或输出无法解析时返回 [`CupsError`]。
    pub async fn submit(
        &self,
        printer: &str,
        file: &Path,
        options: &BTreeMap<String, String>,
    ) -> Result<String, CupsError> {
        let mut args = vec!["-d".to_string(), printer.to_string()];
        for (key, value) in options {
            args.push("-o".to_string());
            args.push(format!("{key}={value}"));
        }
        args.push(file.to_string_lossy().into_owned());
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self.run("lp", &arg_refs).await?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_job_id(&stdout)
            .ok_or_else(|| CupsError::Parse(format!("无法解析 lp 输出中的任务 id: {stdout}")))
    }

    /// 查询单个任务状态；CUPS 中不存在时返回 `None`。
    ///
    /// # Errors
    ///
    /// `lpstat` / `ipptool` 失败或输出无法解析时返回 [`CupsError`]。
    pub async fn job_status(&self, job_id: &str) -> Result<Option<JobInfo>, CupsError> {
        let Some((printer_id, job_number)) = self.find_job_destination(job_id).await? else {
            return Ok(None);
        };
        let request = format!(
            "{{\n  OPERATION Get-Job-Attributes\n  GROUP operation-attributes-tag\n  \
             ATTR charset attributes-charset utf-8\n  \
             ATTR language attributes-natural-language en\n  \
             ATTR uri printer-uri {}\n  ATTR integer job-id {}\n}}\n",
            self.printer_uri(&printer_id),
            job_number
        );
        let request_path =
            std::env::temp_dir().join(format!("just-print-job-{}.ipp", crate::ids::new_id()));
        tokio::fs::write(&request_path, request)
            .await
            .map_err(|error| CupsError::Spawn(error.to_string()))?;
        let output = self
            .run(
                "ipptool",
                &[
                    "-tv",
                    &self.printer_uri(&printer_id),
                    &request_path.to_string_lossy(),
                ],
            )
            .await?;
        let _ = tokio::fs::remove_file(&request_path).await;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_ipp_job(&stdout, &printer_id, &job_number))
    }

    async fn find_job_destination(
        &self,
        job_id: &str,
    ) -> Result<Option<(String, String)>, CupsError> {
        let output = self.run("lpstat", &["-W", "all", "-o"]).await?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let Some(key) = line.split_whitespace().next() else {
                continue;
            };
            let Some((printer, job_number)) = split_job_key(key) else {
                continue;
            };
            if job_number == job_id || format!("{printer}-{job_number}") == job_id {
                return Ok(Some((printer, job_number)));
            }
        }
        Ok(None)
    }

    fn printer_uri(&self, printer: &str) -> String {
        format!("{}://{}/printers/{printer}", self.scheme, self.server)
    }

    async fn run(&self, program: &'static str, args: &[&str]) -> Result<Output, CupsError> {
        let mut command = Command::new(program);
        command
            .args(args)
            .env("CUPS_SERVER", &self.server)
            .env("CUPS_ENCRYPTION", "IfRequested")
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("TZ", "UTC")
            .kill_on_drop(true);
        let output = tokio::time::timeout(self.timeout, command.output())
            .await
            .map_err(|_| CupsError::Timeout(program))?
            .map_err(|error| CupsError::Spawn(error.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(CupsError::Command { program, stderr });
        }
        Ok(output)
    }
}

/// CUPS 打印机基本信息。
#[derive(Debug, Clone)]
pub struct PrinterInfo {
    /// CUPS 打印机名（同时也是提交任务时的 `printer_id`）。
    pub name: String,
    /// 打印机状态（`idle` / `printing` / `disabled` / `stopped`）。
    pub state: String,
    /// PPD 或配置中的描述；缺失时为 `None`。
    pub description: Option<String>,
}

/// 暴露给前端的单个 CUPS 选项。
#[derive(Debug, Clone, Serialize)]
pub struct OptionSpec {
    /// 选项默认值（CUPS 以 `*` 标记；范围选项通常无标记）。
    pub default: Option<String>,
    /// 选项取值类型与合法范围。
    #[serde(flatten)]
    pub kind: OptionKind,
}

/// CUPS 选项取值类型。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum OptionKind {
    /// 枚举值列表。
    Enumerated {
        /// 合法取值。
        values: Vec<String>,
    },
    /// 整数范围。
    Range {
        /// 最小值。
        min: i64,
        /// 最大值。
        max: i64,
    },
}

/// CUPS 任务状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    /// 排队中（含 held）。
    Queued,
    /// 打印中。
    Printing,
    /// 已完成。
    Completed,
    /// 失败（aborted / stopped）。
    Failed,
    /// 已取消。
    Canceled,
}

/// CUPS 任务视图。
#[derive(Debug, Clone, Serialize)]
pub struct JobInfo {
    /// CUPS 任务 id（`printer-job` 形式）。
    pub id: String,
    /// 目标打印机名。
    pub printer_id: String,
    /// 当前状态。
    pub status: JobStatus,
    /// 失败/取消原因；其它状态为 `None`。
    pub error: Option<String>,
    /// 创建时间（Unix 毫秒）。
    pub created_at_ms: u64,
}

fn parse_printers(output: &str) -> Result<Vec<PrinterInfo>, String> {
    let mut printers: Vec<PrinterInfo> = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("printer ") {
            let (name, state_part) = rest
                .split_once(" is ")
                .ok_or_else(|| format!("无法解析打印机行: {line}"))?;
            let state = state_part
                .split_whitespace()
                .next()
                .unwrap_or("unknown")
                .trim_end_matches('.')
                .to_string();
            printers.push(PrinterInfo {
                name: name.trim().to_string(),
                state,
                description: None,
            });
        } else if let Some(description) = trimmed.strip_prefix("Description:")
            && let Some(printer) = printers.last_mut()
        {
            printer.description = Some(description.trim().to_string());
        }
    }
    if printers.is_empty() {
        return Err("lpstat -p -l 未返回任何打印机".to_string());
    }
    Ok(printers)
}

fn parse_options(output: &str) -> BTreeMap<String, OptionSpec> {
    let mut options = BTreeMap::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name_part, value_part)) = line.split_once(':') else {
            continue;
        };
        let name = name_part
            .split('/')
            .next()
            .unwrap_or(name_part)
            .trim()
            .to_string();
        if name.is_empty() || value_part.trim().is_empty() {
            continue;
        }
        let choices = split_choices(value_part);
        if choices.is_empty() {
            continue;
        }

        if let Some((value, is_default)) = choices.first() {
            let value = value.as_str();
            let is_default = *is_default;
            if let Some((min, max)) = parse_range(value) {
                let default = is_default.then(|| value.to_string());
                options.insert(
                    name,
                    OptionSpec {
                        default,
                        kind: OptionKind::Range { min, max },
                    },
                );
                continue;
            }
        }

        let mut values = Vec::with_capacity(choices.len());
        let mut default = None;
        for (value, is_default) in choices {
            if is_default {
                default = Some(value.clone());
            }
            values.push(value);
        }
        options.insert(
            name,
            OptionSpec {
                default,
                kind: OptionKind::Enumerated { values },
            },
        );
    }
    options
}

fn split_choices(value: &str) -> Vec<(String, bool)> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut is_default = false;
    for ch in value.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ch if ch.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    result.push((std::mem::take(&mut current), is_default));
                    is_default = false;
                }
            }
            '*' if current.is_empty() && !in_quotes => is_default = true,
            ch => current.push(ch),
        }
    }
    if !current.is_empty() {
        result.push((current, is_default));
    }
    result
}

fn parse_range(value: &str) -> Option<(i64, i64)> {
    let (min, max) = value.split_once('-')?;
    let min = min.parse::<i64>().ok()?;
    let max = max.parse::<i64>().ok()?;
    (min <= max).then_some((min, max))
}

fn parse_job_id(output: &str) -> Option<String> {
    let line = output.lines().next()?;
    let rest = line.strip_prefix("request id is ")?;
    rest.split_whitespace().next().map(str::to_string)
}

fn split_job_key(key: &str) -> Option<(String, String)> {
    let index = key.rfind('-')?;
    let (printer, dash_job) = key.split_at(index);
    let job = dash_job.strip_prefix('-')?;
    if printer.is_empty() || !job.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((printer.to_string(), job.to_string()))
}

fn map_job_status(state: &str) -> JobStatus {
    match state {
        "pending" | "pending-held" => JobStatus::Queued,
        "processing" | "processing-stopped" => JobStatus::Printing,
        "completed" => JobStatus::Completed,
        "canceled" => JobStatus::Canceled,
        _ => JobStatus::Failed,
    }
}

fn parse_ipp_job(output: &str, printer_id: &str, job_number: &str) -> Option<JobInfo> {
    let mut state = None;
    let mut reasons = None;
    let mut created_at_ms = 0;
    for line in output.lines() {
        let line = line.trim();
        if let Some(status) = line.strip_prefix("status-code = ") {
            if !status.starts_with("successful-ok") {
                return None;
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if let Some(name) = key.strip_suffix(" (enum)") {
            if name.trim() == "job-state" {
                state = Some(value.to_string());
            }
        } else if let Some(name) = key.strip_suffix(" (keyword)") {
            if name.trim() == "job-state-reasons" {
                reasons = Some(value.to_string());
            }
        } else if let Some(name) = key.strip_suffix(" (integer)")
            && name.trim() == "time-at-creation"
            && let Ok(seconds) = value.parse::<u64>()
        {
            created_at_ms = seconds.saturating_mul(1000);
        }
    }
    let status = state.as_deref().map_or(JobStatus::Failed, map_job_status);
    let error = match status {
        JobStatus::Failed | JobStatus::Canceled => {
            reasons.filter(|value| value != "none" && !value.is_empty())
        }
        _ => None,
    };
    Some(JobInfo {
        id: format!("{printer_id}-{job_number}"),
        printer_id: printer_id.to_string(),
        status,
        error,
        created_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        JobStatus, OptionKind, parse_ipp_job, parse_job_id, parse_options, parse_printers,
        split_choices, split_job_key,
    };

    #[test]
    fn parses_lpstat_printer_blocks() {
        let output = "\
printer PDF is idle.  enabled since Tue 04 Aug 2026 16:20:00 UTC
\tDescription: CUPS-PDF Printer
\tLocation: Local
\tDriver: cups-pdf (grayscale, 2-sided printing)
\tConnection: direct
\tDefaults: media=iso_a4_210x297mm sides=one-sided

printer Office is printing.  enabled since Mon 03 Aug 2026 08:00:00 UTC
\tDescription: Network Office Printer
\tConnection: ipp://office.local:631/ipp/print
";
        let result = parse_printers(output);
        assert!(result.is_ok());
        let Ok(printers) = result else {
            return;
        };
        assert_eq!(printers.len(), 2);
        let first = printers.first();
        assert!(first.is_some());
        if let Some(first) = first {
            assert_eq!(first.name, "PDF");
            assert_eq!(first.state, "idle");
            assert_eq!(first.description.as_deref(), Some("CUPS-PDF Printer"));
        }
        let second = printers.get(1);
        assert!(second.is_some());
        if let Some(second) = second {
            assert_eq!(second.state, "printing");
        }
    }

    #[test]
    fn parses_lpoptions_enumerated_and_ranges() {
        let output = "\
PageSize/Page Size: *Letter A4 A5 \"Plain Paper\"
copies/Copies: 1-9999
Duplex/Duplex Printing: DuplexTumble DuplexNoTumble *None
";
        let options = parse_options(output);
        let page_size = options.get("PageSize");
        assert!(page_size.is_some());
        if let Some(page_size) = page_size {
            assert_eq!(page_size.default.as_deref(), Some("Letter"));
            assert!(matches!(
                &page_size.kind,
                OptionKind::Enumerated { values } if values.len() == 4
            ));
        }
        let copies = options.get("copies");
        assert!(copies.is_some());
        if let Some(copies) = copies {
            assert!(matches!(
                &copies.kind,
                OptionKind::Range { min: 1, max: 9999 }
            ));
        }
        let duplex = options.get("Duplex");
        assert!(duplex.is_some());
        if let Some(duplex) = duplex {
            assert_eq!(duplex.default.as_deref(), Some("None"));
        }
    }

    #[test]
    fn split_choices_handles_quotes_and_default_markers() {
        assert_eq!(
            split_choices("*A4 \"Plain Paper\" 11x17"),
            vec![
                ("A4".to_string(), true),
                ("Plain Paper".to_string(), false),
                ("11x17".to_string(), false),
            ]
        );
    }

    #[test]
    fn parses_lp_job_id() {
        assert_eq!(
            parse_job_id("request id is PDF-8 (1 file(s))\n").as_deref(),
            Some("PDF-8")
        );
    }

    #[test]
    fn parses_ipp_job_attributes() {
        let output = "\
        status-code = successful-ok (successful-ok)
        job-state (enum) = completed
        job-state-reasons (keyword) = processing-to-stop-point
        time-at-creation (integer) = 1785833607
";
        let Some(job) = parse_ipp_job(output, "PDF", "8") else {
            return;
        };
        assert_eq!(job.id, "PDF-8");
        assert_eq!(job.printer_id, "PDF");
        assert_eq!(job.status, JobStatus::Completed);
        assert!(job.error.is_none());
        assert_eq!(job.created_at_ms, 1_785_833_607_000);
    }

    #[test]
    fn parses_failed_and_canceled_ipp_jobs() {
        let failed = "\
        status-code = successful-ok (successful-ok)
        job-state (enum) = aborted
        job-state-reasons (keyword) = job-hold-until-specified
";
        let Some(job) = parse_ipp_job(failed, "PDF", "6") else {
            return;
        };
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("job-hold-until-specified"));

        let canceled = "\
        status-code = successful-ok (successful-ok)
        job-state (enum) = canceled
        job-state-reasons (keyword) = job-canceled-by-user
";
        let Some(job) = parse_ipp_job(canceled, "PDF", "5") else {
            return;
        };
        assert_eq!(job.status, JobStatus::Canceled);
    }

    #[test]
    fn ipp_not_found_returns_none() {
        let output = "\
        status-code = client-error-not-found (Job #999 does not exist.)
";
        assert!(parse_ipp_job(output, "PDF", "999").is_none());
    }

    #[test]
    fn job_key_split_handles_hyphenated_printer_names() {
        let result = split_job_key("My-Office-PDF-12");
        assert_eq!(
            result,
            Some(("My-Office-PDF".to_string(), "12".to_string()))
        );
    }
}
