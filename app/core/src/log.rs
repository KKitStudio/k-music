//! 极简运行日志：追加写入 `<曲库>/.mt/app.log`（路径由 cfg.log_path() 推导）。
//! 用途：GUI 模式下 stdout 不可见，真机验证时把关键事件/错误落盘到 `.mt/app.log` 便于诊断。
//! 设计：仅用标准库，零依赖、零风险；写入失败静默忽略，不影响主流程。
use crate::config::Config;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// 取当前 UNIX 秒（不可用则 0）。
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 追加一行到指定路径（serve 启动尚未持有完整 Config 时也能用）。
pub fn write_to(path: &Path, tag: &str, msg: &str) {
    let line = format!("[{}] {} {}\n", now_secs(), tag, msg);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// 追加一行到配置指定的日志路径。
pub fn event(cfg: &Config, tag: &str, msg: &str) {
    write_to(&cfg.log_path(), tag, msg);
}
