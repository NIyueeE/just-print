//! PJL 包装层：设备发现、能力查询/解析、打印与复位会话。
//!
//! 本层只感知打印机设备与 PDF 字节流，不感知上传格式、任务队列或 Web API。
//! 同一台打印机的所有设备访问由上层 worker 串行化后调用本层函数。

pub mod capabilities;
pub mod discover;
pub mod session;

pub use capabilities::{
    PRACTICAL_VARIABLES, Variable, VariableKind, practical_variables, supports_pdf,
};
pub use discover::{DiscoveredPrinter, printer_id, scan_sysfs_from};
pub use session::{PjlError, print_pdf, query_capabilities, reset};
