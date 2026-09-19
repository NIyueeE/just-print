//! 轻量 Prometheus 指标注册表（无第三方依赖）。
//!
//! 支持计数器、仪表和固定桶直方图，输出标准 Prometheus 文本格式。
//! 指标名统一加 `just_print_` 前缀，计数器自动加 `_total` 后缀。

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// 直方图默认桶（秒）。
const DURATION_BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// 应用指标。
#[derive(Debug)]
pub struct Metrics {
    started: Instant,
    counters: Mutex<HashMap<String, BTreeMap<String, u64>>>,
    gauges: Mutex<HashMap<String, BTreeMap<String, i64>>>,
    histograms: Mutex<HashMap<String, BTreeMap<String, Histogram>>>,
    in_flight: AtomicU64,
}

/// 单一直方图（固定桶）。
#[derive(Debug, Clone)]
struct Histogram {
    buckets: Vec<u64>,
    sum: f64,
    count: u64,
}

impl Histogram {
    fn new() -> Self {
        Self {
            buckets: vec![0; DURATION_BUCKETS.len()],
            sum: 0.0,
            count: 0,
        }
    }

    fn observe(&mut self, value: f64) {
        for (index, bound) in DURATION_BUCKETS.iter().enumerate() {
            if value <= *bound
                && let Some(bucket) = self.buckets.get_mut(index)
            {
                *bucket += 1;
            }
        }
        self.sum += value;
        self.count += 1;
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            counters: Mutex::new(HashMap::new()),
            gauges: Mutex::new(HashMap::new()),
            histograms: Mutex::new(HashMap::new()),
            in_flight: AtomicU64::new(0),
        }
    }
}

impl Metrics {
    /// 创建指标注册表。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 计数器加一（`name` 不带 `_total` 后缀）。
    pub fn inc(&self, name: &str, labels: &[(&str, &str)]) {
        self.add(name, labels, 1);
    }

    /// 计数器加指定值。
    pub fn add(&self, name: &str, labels: &[(&str, &str)], value: u64) {
        let metric = format!("just_print_{name}_total");
        let labels = render_labels(labels);
        let mut counters = self.lock_counters();
        let family = counters.entry(metric).or_default();
        *family.entry(labels).or_insert(0) += value;
    }

    /// 记录一次耗时观测（秒）。
    pub fn observe(&self, name: &str, labels: &[(&str, &str)], seconds: f64) {
        let metric = format!("just_print_{name}_seconds");
        let labels = render_labels(labels);
        let mut histograms = self.lock_histograms();
        let family = histograms.entry(metric).or_default();
        family
            .entry(labels)
            .or_insert_with(Histogram::new)
            .observe(seconds);
    }

    /// 设置仪表值。
    pub fn set_gauge(&self, name: &str, labels: &[(&str, &str)], value: i64) {
        let metric = format!("just_print_{name}");
        let labels = render_labels(labels);
        let mut gauges = self.lock_gauges();
        let family = gauges.entry(metric).or_default();
        family.insert(labels, value);
    }

    /// 正在处理的请求数加一。
    pub fn begin_request(&self) {
        self.in_flight.fetch_add(1, Ordering::Relaxed);
    }

    /// 正在处理的请求数减一。
    pub fn end_request(&self) {
        self.in_flight.fetch_sub(1, Ordering::Relaxed);
    }

    /// 渲染 Prometheus 文本格式。
    #[must_use]
    pub fn render(&self) -> String {
        let mut output = String::new();
        let uptime = self.started.elapsed().as_secs_f64();
        let in_flight = self.in_flight.load(Ordering::Relaxed);

        let _ = writeln!(output, "# TYPE just_print_uptime_seconds gauge");
        let _ = writeln!(output, "just_print_uptime_seconds {uptime}");
        let _ = writeln!(output, "# TYPE just_print_http_in_flight gauge");
        let _ = writeln!(output, "just_print_http_in_flight {in_flight}");

        for (metric, family) in self.lock_counters().iter() {
            let _ = writeln!(output, "# TYPE {metric} counter");
            for (labels, value) in family {
                let _ = writeln!(output, "{} {value}", sample(metric, labels));
            }
        }
        for (metric, family) in self.lock_gauges().iter() {
            let _ = writeln!(output, "# TYPE {metric} gauge");
            for (labels, value) in family {
                let _ = writeln!(output, "{} {value}", sample(metric, labels));
            }
        }
        for (metric, family) in self.lock_histograms().iter() {
            let _ = writeln!(output, "# TYPE {metric} histogram");
            for (labels, histogram) in family {
                for (index, bound) in DURATION_BUCKETS.iter().enumerate() {
                    let count = histogram.buckets.get(index).copied().unwrap_or(0);
                    let _ = writeln!(
                        output,
                        "{} {count}",
                        sample_with_extra(metric, labels, &format!("le=\"{bound}\""))
                    );
                }
                let _ = writeln!(
                    output,
                    "{} {}",
                    sample_with_extra(metric, labels, "le=\"+Inf\""),
                    histogram.count
                );
                let _ = writeln!(output, "{}_sum{} {}", metric, braces(labels), histogram.sum);
                let _ = writeln!(
                    output,
                    "{}_count{} {}",
                    metric,
                    braces(labels),
                    histogram.count
                );
            }
        }
        output
    }

    fn lock_counters(&self) -> std::sync::MutexGuard<'_, HashMap<String, BTreeMap<String, u64>>> {
        self.counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_gauges(&self) -> std::sync::MutexGuard<'_, HashMap<String, BTreeMap<String, i64>>> {
        self.gauges
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_histograms(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, BTreeMap<String, Histogram>>> {
        self.histograms
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn render_labels(labels: &[(&str, &str)]) -> String {
    let mut rendered = String::new();
    for (index, (label, value)) in labels.iter().enumerate() {
        if index > 0 {
            rendered.push(',');
        }
        let _ = write!(rendered, "{label}=\"{}\"", escape_label(value));
    }
    rendered
}

fn braces(labels: &str) -> String {
    if labels.is_empty() {
        String::new()
    } else {
        format!("{{{labels}}}")
    }
}

fn sample(metric: &str, labels: &str) -> String {
    format!("{metric}{}", braces(labels))
}

fn sample_with_extra(metric: &str, labels: &str, extra: &str) -> String {
    if labels.is_empty() {
        format!("{metric}_bucket{{{extra}}}")
    } else {
        format!("{metric}_bucket{{{labels},{extra}}}")
    }
}

fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::Metrics;

    #[test]
    fn renders_counters_gauges_and_histograms() {
        let metrics = Metrics::new();
        metrics.inc("http_requests", &[("method", "GET"), ("status", "200")]);
        metrics.inc("http_requests", &[("method", "GET"), ("status", "200")]);
        metrics.set_gauge("uploaded_files", &[], 3);
        metrics.observe("request_duration", &[("route", "/api/printers")], 0.02);
        let rendered = metrics.render();
        assert!(rendered.contains("# TYPE just_print_http_requests_total counter"));
        assert!(
            rendered.contains("just_print_http_requests_total{method=\"GET\",status=\"200\"} 2")
        );
        assert!(rendered.contains("just_print_uploaded_files 3"));
        assert!(
            rendered
                .contains("just_print_request_duration_seconds_count{route=\"/api/printers\"} 1")
        );
        assert!(rendered.contains(
            "just_print_request_duration_seconds_bucket{route=\"/api/printers\",le=\"+Inf\"} 1"
        ));
    }
}
