//! 打印机选项目录：从 IPP 打印机的 `*-supported` / `*-default` 属性中提取
//! 常用 job 模板选项，并把用户请求的字符串值编码为 IPP job 属性。
//!
//! 选项键全部使用标准 IPP 属性名（如 `media`、`sides`、`print-quality`），
//! 不再使用 PPD 的 `lpoptions` 名称，保证与 RFC 8011 及 IPP Everywhere 一致。

use std::collections::BTreeMap;

use ipp::attribute::{IppAttribute, IppAttributes};
use ipp::model::DelimiterTag;
use ipp::value::IppValue;
use serde::Serialize;
use thiserror::Error;

/// 向 `Get-Printer-Attributes` 请求的选项属性列表。
pub const REQUESTED_ATTRIBUTES: [&str; 22] = [
    "media-supported",
    "media-default",
    "sides-supported",
    "sides-default",
    "print-color-mode-supported",
    "print-color-mode-default",
    "print-quality-supported",
    "print-quality-default",
    "printer-resolution-supported",
    "printer-resolution-default",
    "copies-supported",
    "copies-default",
    "number-up-supported",
    "number-up-default",
    "media-type-supported",
    "media-type-default",
    "output-bin-supported",
    "output-bin-default",
    "orientation-requested-supported",
    "orientation-requested-default",
    "finishings-supported",
    "finishings-default",
];

/// 选项取值类型与合法范围。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OptionKind {
    /// IPP `keyword` 取值。
    Keyword {
        /// 合法取值。
        values: Vec<String>,
    },
    /// IPP `enum` 取值，附带可读名称。
    Enum {
        /// 合法取值。
        values: Vec<EnumValue>,
    },
    /// 整数区间取值。
    Integer {
        /// 最小值。
        min: i64,
        /// 最大值。
        max: i64,
    },
    /// 离散整数取值。
    IntegerChoices {
        /// 合法取值。
        values: Vec<i64>,
    },
    /// 分辨率取值（`WxHdpi` / `WxHdpcm`）。
    Resolution {
        /// 合法取值。
        values: Vec<ResolutionValue>,
    },
}

/// 单个 IPP 枚举值。
#[derive(Debug, Clone, Serialize)]
pub struct EnumValue {
    /// 数值。
    pub value: i32,
    /// 标准名称。
    pub name: String,
}

/// 单个 IPP 分辨率值。
#[derive(Debug, Clone, Serialize)]
pub struct ResolutionValue {
    /// 横向分辨率。
    pub cross_feed: i32,
    /// 纵向分辨率。
    pub feed: i32,
    /// 单位（3 = dpi，4 = dpcm）。
    pub units: i8,
    /// 展示用字符串，如 `600x600dpi`。
    pub label: String,
}

/// 单个打印选项。
#[derive(Debug, Clone, Serialize)]
pub struct OptionSpec {
    /// 打印机默认值（字符串形式）；缺失时为 `None`。
    pub default: Option<String>,
    /// 取值类型与合法范围。
    #[serde(flatten)]
    pub kind: OptionKind,
}

/// 选项解析与编码错误。
#[derive(Debug, Error)]
pub enum OptionError {
    /// 打印机未提供该选项。
    #[error("打印机未提供参数 {0}")]
    Unknown(String),
    /// 取值不在合法范围内。
    #[error("参数 {key} 的取值 {value} 不合法")]
    Invalid {
        /// 选项名。
        key: String,
        /// 非法取值。
        value: String,
    },
    /// IPP 属性编码失败。
    #[error("参数编码失败: {0}")]
    Encode(String),
}

impl From<ipp::parser::IppParseError> for OptionError {
    fn from(error: ipp::parser::IppParseError) -> Self {
        Self::Encode(error.to_string())
    }
}

/// 从 `Get-Printer-Attributes` 响应中提取打印选项。
#[must_use]
pub fn parse_options(attributes: &IppAttributes) -> BTreeMap<String, OptionSpec> {
    let mut options = BTreeMap::new();

    if let Some(spec) = keyword_spec(attributes, "media") {
        options.insert("media".to_string(), spec);
    }
    if let Some(spec) = keyword_spec(attributes, "sides") {
        options.insert("sides".to_string(), spec);
    }
    if let Some(spec) = keyword_spec(attributes, "print-color-mode") {
        options.insert("print-color-mode".to_string(), spec);
    }
    if let Some(spec) = keyword_spec(attributes, "media-type") {
        options.insert("media-type".to_string(), spec);
    }
    if let Some(spec) = keyword_spec(attributes, "output-bin") {
        options.insert("output-bin".to_string(), spec);
    }
    if let Some(spec) = enum_spec(attributes, "print-quality", &PRINT_QUALITY_NAMES) {
        options.insert("print-quality".to_string(), spec);
    }
    if let Some(spec) = enum_spec(attributes, "orientation-requested", &ORIENTATION_NAMES) {
        options.insert("orientation-requested".to_string(), spec);
    }
    if let Some(spec) = enum_spec(attributes, "finishings", &FINISHINGS_NAMES) {
        options.insert("finishings".to_string(), spec);
    }
    if let Some(spec) = resolution_spec(attributes, "printer-resolution") {
        options.insert("printer-resolution".to_string(), spec);
    }
    if let Some(spec) = integer_spec(attributes, "copies") {
        options.insert("copies".to_string(), spec);
    }
    if let Some(spec) = integer_spec(attributes, "number-up") {
        options.insert("number-up".to_string(), spec);
    }

    options
}

/// 把用户请求的字符串选项编码为 `Print-Job` 的 job 属性。
///
/// 除标准 IPP 属性外，分辨率还会额外附带一个 CUPS PPD 兼容属性（见
/// [`resolution_ppd_attribute`]）。
///
/// # Errors
///
/// 选项不在目录中、取值不合法或 IPP 值编码失败时返回 [`OptionError`]。
pub fn encode_job_attributes(
    catalog: &BTreeMap<String, OptionSpec>,
    requested: &BTreeMap<String, String>,
) -> Result<Vec<IppAttribute>, OptionError> {
    let mut attributes = Vec::with_capacity(requested.len());
    for (key, raw) in requested {
        let spec = catalog
            .get(key)
            .ok_or_else(|| OptionError::Unknown(key.clone()))?;
        let value = encode_value(&spec.kind, key, raw)?;
        attributes.push(IppAttribute::with_name(key.as_str(), value)?);
        if let Some(compat) = resolution_ppd_attribute(&spec.kind, raw) {
            attributes.push(compat);
        }
    }
    Ok(attributes)
}

/// CUPS PPD 队列的分辨率兼容属性。
///
/// CUPS 在 PPD 队列上把 IPP `printer-resolution` 交给 `cupsMarkOptions()`，
/// 后者只用取值去匹配 PPD choice 名（`Resolution`/`SetResolution`/`JCLResolution`），
/// 而 cupsd 自己序列化 IPP 分辨率时写的是 `1200x1200dpi`，PPD 里却是 `1200dpi`，
/// 于是标准属性被静默忽略、分辨率退回 PPD 默认值。
///
/// 因此这里在标准属性之外再发一个同值的 PPD 风格属性 `Resolution`：
/// PPD 队列凭 choice 名生效；没有该 PPD 选项的队列/打印机按未知属性忽略。
fn resolution_ppd_attribute(kind: &OptionKind, raw: &str) -> Option<IppAttribute> {
    let OptionKind::Resolution { values } = kind else {
        return None;
    };
    let value = values.iter().find(|candidate| candidate.label == raw)?;
    let name = ppd_resolution_choice(value);
    IppAttribute::with_name("Resolution", IppValue::new_keyword(name.as_str()).ok()?).ok()
}

/// 把分辨率值写成 PPD choice 的常见形式：`1200dpi` 或 `600x1200dpi`。
fn ppd_resolution_choice(value: &ResolutionValue) -> String {
    // IPP 分辨率单位：3 = 每英寸点数，4 = 每厘米点数。
    let unit = match value.units {
        4 => "dpcm",
        _ => "dpi",
    };
    if value.cross_feed == value.feed {
        format!("{}{unit}", value.cross_feed)
    } else {
        format!("{}x{}{unit}", value.cross_feed, value.feed)
    }
}

fn encode_value(kind: &OptionKind, key: &str, raw: &str) -> Result<IppValue, OptionError> {
    let invalid = || OptionError::Invalid {
        key: key.to_string(),
        value: raw.to_string(),
    };
    match kind {
        OptionKind::Keyword { values } => {
            if !values.iter().any(|candidate| candidate == raw) {
                return Err(invalid());
            }
            Ok(IppValue::new_keyword(raw)?)
        }
        OptionKind::Enum { values } => {
            let found = values
                .iter()
                .find(|candidate| candidate.name == raw || candidate.value.to_string() == raw);
            let value = found.ok_or_else(invalid)?.value;
            Ok(IppValue::new_enum(value)?)
        }
        OptionKind::Integer { min, max } => {
            let parsed = raw.parse::<i64>().map_err(|_| invalid())?;
            if parsed < *min || parsed > *max {
                return Err(invalid());
            }
            let parsed = i32::try_from(parsed).map_err(|_| invalid())?;
            Ok(IppValue::new_integer(parsed))
        }
        OptionKind::IntegerChoices { values } => {
            let parsed = raw.parse::<i64>().map_err(|_| invalid())?;
            if !values.contains(&parsed) {
                return Err(invalid());
            }
            let parsed = i32::try_from(parsed).map_err(|_| invalid())?;
            Ok(IppValue::new_integer(parsed))
        }
        OptionKind::Resolution { values } => {
            let found = values
                .iter()
                .find(|candidate| candidate.label == raw)
                .ok_or_else(invalid)?;
            Ok(IppValue::new_resolution(
                found.cross_feed,
                found.feed,
                found.units,
            ))
        }
    }
}

fn group_value<'a>(attributes: &'a IppAttributes, name: &str) -> Option<&'a IppValue> {
    attributes
        .groups_of(DelimiterTag::PrinterAttributes)
        .find_map(|group| group.get(name))
        .map(IppAttribute::value)
}

fn keyword_spec(attributes: &IppAttributes, name: &str) -> Option<OptionSpec> {
    let values = value_strings(group_value(attributes, &format!("{name}-supported"))?);
    if values.is_empty() {
        return None;
    }
    let default = group_value(attributes, &format!("{name}-default"))
        .and_then(|value| value_strings(value).into_iter().next())
        .filter(|candidate| values.contains(candidate));
    Some(OptionSpec {
        default,
        kind: OptionKind::Keyword { values },
    })
}

fn enum_spec(attributes: &IppAttributes, name: &str, names: &[(i32, &str)]) -> Option<OptionSpec> {
    let raw = value_enums(group_value(attributes, &format!("{name}-supported"))?);
    if raw.is_empty() {
        return None;
    }
    let mut values = Vec::with_capacity(raw.len());
    for value in raw {
        values.push(EnumValue {
            value,
            name: enum_name(names, value),
        });
    }
    let default = group_value(attributes, &format!("{name}-default"))
        .and_then(|value| value_enums(value).into_iter().next())
        .map(|value| enum_name(names, value));
    Some(OptionSpec {
        default,
        kind: OptionKind::Enum { values },
    })
}

fn resolution_spec(attributes: &IppAttributes, name: &str) -> Option<OptionSpec> {
    let values = value_resolutions(group_value(attributes, &format!("{name}-supported"))?);
    if values.is_empty() {
        return None;
    }
    let default = group_value(attributes, &format!("{name}-default"))
        .and_then(|value| value_resolutions(value).into_iter().next())
        .map(|value| value.label);
    Some(OptionSpec {
        default,
        kind: OptionKind::Resolution { values },
    })
}

fn integer_spec(attributes: &IppAttributes, name: &str) -> Option<OptionSpec> {
    let supported = group_value(attributes, &format!("{name}-supported"))?;
    let kind = match supported {
        IppValue::RangeOfInteger { min, max } => OptionKind::Integer {
            min: i64::from(*min),
            max: i64::from(*max),
        },
        IppValue::Integer(value) => OptionKind::Integer {
            min: i64::from(*value),
            max: i64::from(*value),
        },
        IppValue::Array(_) => {
            let values = value_int_choices(supported);
            if values.is_empty() {
                return None;
            }
            OptionKind::IntegerChoices { values }
        }
        _ => return None,
    };
    let default = group_value(attributes, &format!("{name}-default"))
        .and_then(|value| value_int_choices(value).into_iter().next())
        .map(|value| value.to_string());
    Some(OptionSpec { default, kind })
}

fn value_strings(value: &IppValue) -> Vec<String> {
    match value {
        IppValue::Array(items) => items.iter().flat_map(value_strings).collect(),
        IppValue::Keyword(item)
        | IppValue::NameWithoutLanguage(item)
        | IppValue::MimeMediaType(item) => vec![item.as_str().to_string()],
        IppValue::Uri(item) => vec![item.as_str().to_string()],
        IppValue::TextWithoutLanguage(item) => vec![item.to_string()],
        _ => Vec::new(),
    }
}

fn value_enums(value: &IppValue) -> Vec<i32> {
    match value {
        IppValue::Array(items) => items.iter().flat_map(value_enums).collect(),
        IppValue::Enum(item) => vec![*item],
        _ => Vec::new(),
    }
}

fn value_resolutions(value: &IppValue) -> Vec<ResolutionValue> {
    match value {
        IppValue::Array(items) => items.iter().flat_map(value_resolutions).collect(),
        IppValue::Resolution {
            cross_feed,
            feed,
            units,
        } => vec![ResolutionValue {
            cross_feed: *cross_feed,
            feed: *feed,
            units: *units,
            label: resolution_label(*cross_feed, *feed, *units),
        }],
        _ => Vec::new(),
    }
}

fn value_int_choices(value: &IppValue) -> Vec<i64> {
    match value {
        IppValue::Array(items) => items.iter().flat_map(value_int_choices).collect(),
        IppValue::Integer(item) | IppValue::Enum(item) => vec![i64::from(*item)],
        IppValue::RangeOfInteger { min, max } => {
            // 范围选项用 min..=max 展开会很大，这里只保留端点，前端按区间输入。
            vec![i64::from(*min), i64::from(*max)]
        }
        _ => Vec::new(),
    }
}

fn resolution_label(cross_feed: i32, feed: i32, units: i8) -> String {
    let suffix = match units {
        3 => "dpi",
        4 => "dpcm",
        _ => "",
    };
    format!("{cross_feed}x{feed}{suffix}")
}

fn enum_name(names: &[(i32, &str)], value: i32) -> String {
    names
        .iter()
        .find(|(candidate, _)| *candidate == value)
        .map_or_else(|| value.to_string(), |(_, name)| (*name).to_string())
}

/// IPP `print-quality` 枚举名称。
const PRINT_QUALITY_NAMES: [(i32, &str); 3] = [(3, "draft"), (4, "normal"), (5, "high")];

/// IPP `orientation-requested` 枚举名称。
const ORIENTATION_NAMES: [(i32, &str); 4] = [
    (3, "portrait"),
    (4, "landscape"),
    (5, "reverse-landscape"),
    (6, "reverse-portrait"),
];

/// IPP `finishings` 基础枚举名称（仅保留常用值，其余回落为数字）。
const FINISHINGS_NAMES: [(i32, &str); 7] = [
    (3, "none"),
    (4, "staple"),
    (5, "punch"),
    (6, "cover"),
    (7, "bind"),
    (8, "saddle-stitch"),
    (9, "edge-stitch"),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ipp::attribute::{IppAttribute, IppAttributes};
    use ipp::model::DelimiterTag;
    use ipp::value::IppValue;

    use super::{OptionKind, encode_job_attributes, parse_options};

    fn keyword(value: &str) -> IppValue {
        IppValue::new_keyword(value).unwrap_or(IppValue::NoValue)
    }

    fn attrs(items: Vec<(&str, IppValue)>) -> IppAttributes {
        let mut attributes = IppAttributes::new();
        for (name, value) in items {
            if let Ok(attribute) = IppAttribute::with_name(name, value) {
                attributes.add(DelimiterTag::PrinterAttributes, attribute);
            }
        }
        attributes
    }

    #[test]
    fn parses_keyword_enum_resolution_and_integer_options() {
        let attributes = attrs(vec![
            (
                "media-supported",
                IppValue::Array(vec![
                    keyword("iso_a4_210x297mm"),
                    keyword("na_letter_8.5x11in"),
                ]),
            ),
            ("media-default", keyword("iso_a4_210x297mm")),
            (
                "sides-supported",
                IppValue::Array(vec![
                    keyword("one-sided"),
                    keyword("two-sided-long-edge"),
                    keyword("two-sided-short-edge"),
                ]),
            ),
            ("sides-default", keyword("one-sided")),
            (
                "print-quality-supported",
                IppValue::Array(vec![
                    IppValue::new_enum(3).unwrap_or(IppValue::NoValue),
                    IppValue::new_enum(5).unwrap_or(IppValue::NoValue),
                ]),
            ),
            (
                "copies-supported",
                IppValue::RangeOfInteger { min: 1, max: 999 },
            ),
            (
                "printer-resolution-supported",
                IppValue::Array(vec![IppValue::new_resolution(600, 600, 3)]),
            ),
            (
                "printer-resolution-default",
                IppValue::new_resolution(600, 600, 3),
            ),
        ]);

        let options = parse_options(&attributes);
        assert!(options.contains_key("media"));
        assert!(options.contains_key("sides"));
        assert!(options.contains_key("print-quality"));
        assert!(options.contains_key("copies"));
        assert!(options.contains_key("printer-resolution"));

        let media = options.get("media");
        assert!(
            matches!(media.map(|spec| &spec.kind), Some(OptionKind::Keyword { values }) if values.len() == 2)
        );
        let quality = options.get("print-quality");
        assert!(
            matches!(quality.map(|spec| &spec.kind), Some(OptionKind::Enum { values }) if values.first().map(|v| v.name.as_str()) == Some("draft"))
        );
        let copies = options.get("copies");
        assert!(matches!(
            copies.map(|spec| &spec.kind),
            Some(OptionKind::Integer { min: 1, max: 999 })
        ));
        let resolution = options.get("printer-resolution");
        assert!(
            matches!(resolution.map(|spec| &spec.kind), Some(OptionKind::Resolution { values }) if values.first().map(|v| v.label.as_str()) == Some("600x600dpi"))
        );
    }

    #[test]
    fn encodes_valid_requested_values() {
        let attributes = attrs(vec![
            (
                "sides-supported",
                IppValue::Array(vec![keyword("one-sided"), keyword("two-sided-long-edge")]),
            ),
            (
                "copies-supported",
                IppValue::RangeOfInteger { min: 1, max: 99 },
            ),
            (
                "print-quality-supported",
                IppValue::Array(vec![IppValue::new_enum(4).unwrap_or(IppValue::NoValue)]),
            ),
            (
                "printer-resolution-supported",
                IppValue::Array(vec![IppValue::new_resolution(600, 600, 3)]),
            ),
        ]);
        let catalog = parse_options(&attributes);
        let mut requested = BTreeMap::new();
        requested.insert("sides".to_string(), "two-sided-long-edge".to_string());
        requested.insert("copies".to_string(), "3".to_string());
        requested.insert("print-quality".to_string(), "normal".to_string());
        requested.insert("printer-resolution".to_string(), "600x600dpi".to_string());
        let result = encode_job_attributes(&catalog, &requested);
        assert!(result.is_ok());
        // 4 个标准属性 + 1 个 CUPS PPD 分辨率兼容属性。
        assert_eq!(result.map_or(0, |items| items.len()), 5);
    }

    #[test]
    fn adds_ppd_resolution_alias_for_cups_queues() {
        let attributes = attrs(vec![(
            "printer-resolution-supported",
            IppValue::Array(vec![
                IppValue::new_resolution(600, 600, 3),
                IppValue::new_resolution(600, 1200, 3),
                IppValue::new_resolution(47, 47, 4),
            ]),
        )]);
        let catalog = parse_options(&attributes);

        for (label, expected) in [
            ("600x600dpi", "600dpi"),
            ("600x1200dpi", "600x1200dpi"),
            ("47x47dpcm", "47dpcm"),
        ] {
            let mut requested = BTreeMap::new();
            requested.insert("printer-resolution".to_string(), label.to_string());
            let Ok(encoded) = encode_job_attributes(&catalog, &requested) else {
                return;
            };
            let alias = encoded
                .iter()
                .find(|item| item.name().as_ref() == "Resolution");
            assert!(
                matches!(alias.map(ipp::attribute::IppAttribute::value), Some(IppValue::Keyword(value)) if value.as_ref() == expected),
                "分辨率 {label} 应附带 PPD 兼容属性 Resolution={expected}"
            );
        }
    }

    #[test]
    fn rejects_unknown_and_invalid_values() {
        let attributes = attrs(vec![(
            "sides-supported",
            IppValue::Array(vec![keyword("one-sided")]),
        )]);
        let catalog = parse_options(&attributes);

        let mut unknown = BTreeMap::new();
        unknown.insert("HACK".to_string(), "1".to_string());
        assert!(encode_job_attributes(&catalog, &unknown).is_err());

        let mut invalid = BTreeMap::new();
        invalid.insert("sides".to_string(), "sideways".to_string());
        assert!(encode_job_attributes(&catalog, &invalid).is_err());
    }
}
