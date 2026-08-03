//! 解析并缓存 PJL `@PJL INFO VARIABLES` 输出。
//!
//! 只保留实用参数：双面/翻页、省墨、墨水浓度、纸张类型、分辨率；同时从
//! `PERSONALITY` 判断打印机是否支持 PDF。

use std::collections::BTreeMap;

use serde::Serialize;

/// 暴露给前端的实用控制参数（不含 `PERSONALITY`）。
pub const PRACTICAL_VARIABLES: [&str; 6] = [
    "DUPLEX",
    "BINDING",
    "ECONOMODE",
    "DENSITY",
    "MEDIATYPE",
    "RESOLUTION",
];

/// 从完整能力表筛选出 [`PRACTICAL_VARIABLES`] 中存在的参数。
#[must_use]
pub fn practical_variables(all: &BTreeMap<String, Variable>) -> BTreeMap<String, Variable> {
    PRACTICAL_VARIABLES
        .iter()
        .filter_map(|name| {
            all.get(*name)
                .map(|variable| (name.to_string(), variable.clone()))
        })
        .collect()
}

/// PJL 变量的取值方式。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VariableKind {
    /// 枚举值，`values` 为合法取值。
    Enumerated { values: Vec<String> },
    /// 整数范围，`min` / `max` 为闭区间边界。
    Range { min: i64, max: i64 },
}

/// 一个 PJL 变量的当前默认值与合法取值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Variable {
    /// 打印机报告的当前值，通常为默认值。
    pub default: Option<String>,
    /// 取值方式与合法值。
    #[serde(flatten)]
    pub kind: VariableKind,
}

/// 解析 `@PJL INFO VARIABLES` 响应文本为变量表。
///
/// 兼容示例中的两种格式：
/// - `DUPLEX=OFF [2 ENUMERATED]` + 缩进的取值列表
/// - `LPARM:PCL FONTSIZE=12.00 [2 RANGE]` + 缩进的最小/最大值
///
/// 解析结果保留全部变量（含 `LPARM:*`），由上层按需过滤。
#[must_use]
pub fn parse_info_variables(output: &str) -> BTreeMap<String, Variable> {
    let mut variables = BTreeMap::new();
    let mut pending: Option<Pending> = None;

    for raw in output.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        if let Some(header) = parse_header(raw) {
            if let Some(previous) = pending.take() {
                insert_pending(&mut variables, previous);
            }
            pending = Some(header);
            continue;
        }
        if (raw.starts_with(' ') || raw.starts_with('\t'))
            && let Some(entry) = pending.as_mut()
        {
            entry.values.push(raw.trim().to_string());
        }
    }
    if let Some(last) = pending {
        insert_pending(&mut variables, last);
    }
    variables
}

/// 解析中暂存的一条 PJL 变量。
#[derive(Debug)]
struct Pending {
    name: String,
    default: Option<String>,
    kind: PendingKind,
    values: Vec<String>,
}

/// 变量取值方式（解析阶段暂存）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingKind {
    Enumerated,
    Range,
}

/// 尝试把一行解析为 PJL 变量头。
fn parse_header(line: &str) -> Option<Pending> {
    let trimmed = line.trim();
    let (before_bracket, bracket) = trimmed.rsplit_once('[')?;
    let bracket = bracket.trim().trim_end_matches(']').trim();
    let (_count, kind) = bracket.split_once(' ')?;
    let (name, default) = before_bracket.trim().split_once('=')?;
    let default = (!default.trim().is_empty()).then(|| default.trim().to_string());
    let kind = match kind.to_ascii_uppercase().as_str() {
        "ENUMERATED" => PendingKind::Enumerated,
        "RANGE" => PendingKind::Range,
        _ => return None,
    };
    Some(Pending {
        name: name.trim().to_string(),
        default,
        kind,
        values: Vec::new(),
    })
}

/// 把暂存变量写入结果表；数据不完整时跳过。
fn insert_pending(variables: &mut BTreeMap<String, Variable>, pending: Pending) {
    let variable = match pending.kind {
        PendingKind::Enumerated => {
            if pending.values.is_empty() {
                return;
            }
            Variable {
                default: pending.default,
                kind: VariableKind::Enumerated {
                    values: pending.values,
                },
            }
        }
        PendingKind::Range => {
            let Some(min_text) = pending.values.first() else {
                return;
            };
            let Ok(min) = min_text.parse::<i64>() else {
                return;
            };
            let Some(max_text) = pending.values.get(1) else {
                return;
            };
            let Ok(max) = max_text.parse::<i64>() else {
                return;
            };
            Variable {
                default: pending.default,
                kind: VariableKind::Range { min, max },
            }
        }
    };
    variables.insert(pending.name, variable);
}

/// 判断能力表是否声明支持 PDF personality。
///
/// `PERSONALITY` 缺失时按支持处理（部分打印机不报告该变量但实际支持 PDF）；
/// 明确列出且不含 `PDF` 时返回 `false`。
#[must_use]
pub fn supports_pdf(capabilities: &BTreeMap<String, Variable>) -> bool {
    let Some(personality) = capabilities.get("PERSONALITY") else {
        return true;
    };
    let VariableKind::Enumerated { values } = &personality.kind else {
        return true;
    };
    values.iter().any(|value| value == "PDF")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{parse_info_variables, supports_pdf};
    use crate::pjl::{Variable, VariableKind};

    #[test]
    fn parses_enumerated_and_range() {
        let output =
            "DUPLEX=OFF [2 ENUMERATED]\r\n\tOFF\r\n\tON\r\nDENSITY=0 [2 RANGE]\r\n\t-6\r\n\t6\r\n";
        let vars = parse_info_variables(output);
        assert!(vars.contains_key("DUPLEX"));
        assert!(vars.contains_key("DENSITY"));

        let duplex = vars.get("DUPLEX").cloned();
        assert_eq!(
            duplex,
            Some(Variable {
                default: Some("OFF".to_string()),
                kind: VariableKind::Enumerated {
                    values: vec!["OFF".to_string(), "ON".to_string()],
                },
            })
        );

        let density = vars.get("DENSITY").cloned();
        assert_eq!(
            density,
            Some(Variable {
                default: Some("0".to_string()),
                kind: VariableKind::Range { min: -6, max: 6 },
            })
        );
    }

    #[test]
    fn parses_lparm_prefixes_and_ignores_decimal_ranges() {
        let output = "\
LPARM:PCL FONTSIZE=12.00 [2 RANGE]
        4.00
        999.75
LPARM:EPSON ORIENTATION=PORTRAIT [2 ENUMERATED]
        PORTRAIT
        LANDSCAPE
";
        let vars = parse_info_variables(output);
        assert!(vars.contains_key("LPARM:EPSON ORIENTATION"));
        assert!(!vars.contains_key("LPARM:PCL FONTSIZE"));
    }

    #[test]
    fn personality_without_pdf_is_unsupported() {
        let output = "PERSONALITY=LABEL [2 ENUMERATED]\r\n\tPCL\r\n\tPOSTSCRIPT\r\n";
        let vars = parse_info_variables(output);
        assert!(!supports_pdf(&vars));
    }

    #[test]
    fn parses_real_world_sample() {
        let output = "\
PORTRAIT
        LANDSCAPE
LPARM:IBM ORIENTATION=PORTRAIT [2 ENUMERATED]
        PORTRAIT
        LANDSCAPE
LPARM:EPSON ORIENTATION=PORTRAIT [2 ENUMERATED]
        PORTRAIT
        LANDSCAPE
LPARM:POSTSCRIPT ORIENTATION=PORTRAIT [2 ENUMERATED]
        PORTRAIT
        LANDSCAPE
LPARM:PCL FORMLINES=64 [2 RANGE]
        5
        128
LPARM:IBM FORMLINES=66 [2 RANGE]
        5
        128
LPARM:EPSON FORMLINES=66 [2 RANGE]
        5
        128
MANUALFEED=OFF [2 ENUMERATED]
        OFF
        ON
RESOLUTION=600 [6 ENUMERATED]
        300
        600
        900
        1200
        HQ1200
        TR1200
PERSONALITY=LABEL [5 ENUMERATED]
        PCL
        IBM
        EPSON
        POSTSCRIPT
        AUTO
AUTOCONT=ON [2 ENUMERATED]
        OFF
        ON
PASSWORD=DISABLED [2 RANGE]
        0
        65535
MEDIATYPE=REGULAR [14 ENUMERATED]
        REGULAR
        THICK
        THICK2
        THIN
        RECYCLED
        BOND
        ENVELOPES
        ENVTHICK
        ENVTHIN
        LABEL
        GLOSSY
        COLOR
        LETTERHEAD
        PREPUNCHED
ECONOMODE=ON [2 ENUMERATED]
        OFF
        ON
";
        let vars = parse_info_variables(output);
        let mut expected = BTreeMap::new();
        expected.insert(
            "RESOLUTION".to_string(),
            Variable {
                default: Some("600".to_string()),
                kind: VariableKind::Enumerated {
                    values: vec![
                        "300".to_string(),
                        "600".to_string(),
                        "900".to_string(),
                        "1200".to_string(),
                        "HQ1200".to_string(),
                        "TR1200".to_string(),
                    ],
                },
            },
        );
        assert_eq!(vars.get("RESOLUTION"), expected.get("RESOLUTION"));
        assert!(vars.contains_key("LPARM:PCL FORMLINES"));
        assert!(vars.contains_key("LPARM:EPSON ORIENTATION"));
        assert!(vars.contains_key("MEDIATYPE"));
        assert!(vars.contains_key("ECONOMODE"));
        assert!(!supports_pdf(&vars));
    }

    #[test]
    fn personality_missing_is_treated_as_pdf_supported() {
        let mut vars = BTreeMap::new();
        vars.insert(
            "DUPLEX".to_string(),
            Variable {
                default: Some("OFF".to_string()),
                kind: VariableKind::Enumerated {
                    values: vec!["OFF".to_string(), "ON".to_string()],
                },
            },
        );
        assert!(supports_pdf(&vars));
    }
}
