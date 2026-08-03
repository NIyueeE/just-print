//! 通过轮询 `/sys/class/usb/lp*` 发现 USB 打印机。
//!
//! 只读 sysfs：设备名称、制造商、序列号沿符号链接向上定位到 USB 设备目录读取，
//! 不调用 `lsusb`，不依赖 usbutils / udev。

use std::fs;
use std::path::{Path, PathBuf};

/// 一次轮询发现的一台打印机。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPrinter {
    /// `/dev/usb/lpN` 设备节点路径。
    pub path: PathBuf,
    /// sysfs `product`，通常为打印机型号名。
    pub name: String,
    /// sysfs `manufacturer`，可能缺失。
    pub manufacturer: Option<String>,
    /// sysfs `serial`，可能缺失；缺失时上层回退使用 `path` 作为身份。
    pub serial: Option<String>,
}

/// 从指定根目录扫描 `lp*` 条目；测试可指向伪 sysfs 与伪设备目录。
///
/// 目录不存在或不可读时返回空列表，不视为错误。
#[must_use]
pub fn scan_sysfs_from(root: &Path, device_dir: &Path) -> Vec<DiscoveredPrinter> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut printers = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if is_lp_entry(&name)
            && let Some(printer) = parse_lp_entry(&entry.path(), device_dir)
        {
            printers.push(printer);
        }
    }
    printers.sort_by_key(|printer| lp_number(&printer.path));
    printers
}

/// 生成稳定的打印机身份：优先序列号，缺失时回退到 lp 节点路径。
#[must_use]
pub fn printer_id(printer: &DiscoveredPrinter) -> String {
    printer
        .serial
        .clone()
        .unwrap_or_else(|| printer.path.to_string_lossy().into_owned())
}

/// 解析 `/sys/class/usb/lpN` 条目为 [`DiscoveredPrinter`]；无法识别时返回 `None`。
///
/// 单元测试可直接构造一个伪 sysfs 目录结构来覆盖此函数。
pub fn parse_lp_entry(entry: &Path, device_dir: &Path) -> Option<DiscoveredPrinter> {
    let name = entry.file_name()?.to_string_lossy().into_owned();
    if !is_lp_entry(&name) {
        return None;
    }
    let device_root = fs::canonicalize(entry).ok()?;
    let mut product = None;
    let mut manufacturer = None;
    let mut serial = None;
    for ancestor in device_root.ancestors() {
        if product.is_none() {
            product = read_sysfs_attr(ancestor, "product");
        }
        if manufacturer.is_none() {
            manufacturer = read_sysfs_attr(ancestor, "manufacturer");
        }
        if serial.is_none() {
            serial = read_sysfs_attr(ancestor, "serial");
        }
        if product.is_some() && manufacturer.is_some() && serial.is_some() {
            break;
        }
    }
    Some(DiscoveredPrinter {
        path: device_dir.join(&name),
        name: product.unwrap_or(name),
        manufacturer,
        serial,
    })
}

/// 判断 sysfs 条目名是否为 `lpN`。
fn is_lp_entry(name: &str) -> bool {
    name.strip_prefix("lp")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
}

/// 从 sysfs 设备目录读取单个属性文件并去除首尾空白。
fn read_sysfs_attr(dir: &Path, attr: &str) -> Option<String> {
    let value = fs::read_to_string(dir.join(attr)).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// 从 `/dev/usb/lpN` 路径提取 N；无法解析时使用最大值以便排到末尾。
fn lp_number(path: &Path) -> usize {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("lp"))
        .and_then(|suffix| suffix.parse::<usize>().ok())
        .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::ids;

    use super::{parse_lp_entry, scan_sysfs_from};
    use std::path::Path;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("just-print-sysfs-test-{}", ids::new_id()))
    }

    fn write_attr(dir: &PathBuf, name: &str, value: &str) {
        assert!(std::fs::create_dir_all(dir).is_ok());
        assert!(std::fs::write(dir.join(name), value).is_ok());
    }

    #[test]
    fn parses_sysfs_entry_and_falls_back_to_name() {
        let root = temp_root();
        let device = root.join("device0");
        write_attr(&device, "product", "HP LaserJet M203dw\n");
        write_attr(&device, "manufacturer", "HP\n");
        write_attr(&device, "serial", "CN12345678\n");
        let entry = root.join("lp0");
        let device_dir = root.join("dev");
        assert!(std::os::unix::fs::symlink(&device, &entry).is_ok());

        let parsed = parse_lp_entry(&entry, &device_dir);
        assert!(parsed.is_some());
        let Some(printer) = parsed else {
            return;
        };
        assert_eq!(printer.path, device_dir.join("lp0"));
        assert_eq!(printer.name, "HP LaserJet M203dw");
        assert_eq!(printer.manufacturer.as_deref(), Some("HP"));
        assert_eq!(printer.serial.as_deref(), Some("CN12345678"));
        assert_eq!(super::printer_id(&printer), "CN12345678");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_product_falls_back_to_entry_name() {
        let root = temp_root();
        let device = root.join("device1");
        write_attr(&device, "serial", "SN1\n");
        let entry = root.join("lp1");
        let device_dir = root.join("dev");
        assert!(std::os::unix::fs::symlink(&device, &entry).is_ok());

        let parsed = parse_lp_entry(&entry, &device_dir);
        assert!(parsed.is_some());
        let Some(printer) = parsed else {
            return;
        };
        assert_eq!(printer.name, "lp1");
        assert_eq!(printer.serial.as_deref(), Some("SN1"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scans_and_sorts_numerically() {
        let root = temp_root();
        for (name, serial) in [("lp0", "A"), ("lp10", "B"), ("lp2", "C")] {
            let device = root.join(format!("device-{name}"));
            write_attr(&device, "product", name);
            write_attr(&device, "serial", serial);
            assert!(std::os::unix::fs::symlink(&device, root.join(name)).is_ok());
        }
        assert!(std::fs::write(root.join("not-a-printer"), "x").is_ok());

        let printers = scan_sysfs_from(&root, &root);
        assert_eq!(printers.len(), 3);
        assert_eq!(
            printers
                .iter()
                .map(|printer| printer.serial.as_deref().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["A", "C", "B"]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_root_returns_empty() {
        let root = temp_root();
        assert!(scan_sysfs_from(&root, &root).is_empty());
    }

    #[test]
    fn rejects_non_lp_names() {
        assert!(parse_lp_entry(&PathBuf::from("/tmp/usb0"), Path::new("/dev/usb")).is_none());
    }
}
