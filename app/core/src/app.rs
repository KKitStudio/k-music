//! 应用元信息：名称、版本、作者与联系方式等。
//!
//! 这是**版本号与「关于」面板的唯一数据源**：窗口标题、顶栏品牌徽标、设置面板「关于」分区、
//! `mt-cli --version` 均从此处取值，三处显示永远一致。
//!
//! 版本号取自 Cargo 包版本（`env!("CARGO_PKG_VERSION")`），而 workspace 三个成员
//! （`mt-core`/`mt-cli`/`mt-gui`）统一继承根 `Cargo.toml` 的 `[workspace.package] version`，
//! 因此升级只需改该处一处版本号即可全局同步。

/// 产品名称（窗口标题 / 顶栏品牌 / 关于面板）。
pub const APP_NAME: &str = "K music";
/// 历史代号（2026-11-02 改名前为 musicMT；保留以便识别旧安装与旧文档）。
pub const APP_LEGACY_NAME: &str = "musicMT";
/// 版本号：取自 Cargo 包版本，与 `Cargo.toml` 的 `[workspace.package] version` 一致。
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// **最低可运行版本**：当前版本能正确解析的配置文件格式下限。
///
/// 写入配置文件时会记下两个版本号：
/// - `config_version`：生成该配置文件的构建版本（通常是 `APP_VERSION`）；
/// - `min_compatible_version`：能够正常解析该配置的最低应用版本（即本常量）。
///
/// 读取时用 `version_cmp` 比较二者判断兼容性：配置文件由更高版本生成（结构可能已变），
/// 或其最低兼容版本高于当前程序版本（当前程序太老）时判为**冲突**，需要重建配置。
pub const MIN_COMPATIBLE_VERSION: &str = "0.2.0";
/// 作者。
pub const APP_AUTHOR: &str = "kill33";
/// 作者联系方式（邮箱）。
pub const APP_EMAIL: &str = "kill33@126.com";
/// 开源许可证。
pub const APP_LICENSE: &str = "MIT";
/// 一句话简介。
pub const APP_DESCRIPTION: &str = "本地曲库管理 + 桌面播放器";
/// 技术栈摘要（关于面板展示用）。
pub const APP_TECH: &str = "Rust · Tauri 2 · SQLite(rusqlite) · lofty · axum";

/// 带版本号的窗口标题（例：`K music v0.2.0`）。
pub fn title() -> String {
    format!("{APP_NAME} v{APP_VERSION}")
}

/// 带版本号的应用标识（例：`K music v0.2.0`），供 CLI / 日志等场景复用。
pub fn full_name() -> String {
    title()
}

/// 比较两个语义化版本号：`a < b` → `Ordering::Less`，相等 → `Equal`，`a > b` → `Greater`。
///
/// 解析规则：按 `.` 拆分取前 3 段（major/minor/patch），逐段按数字比较；
/// 缺失的段按 0 处理；非数字段按 0 处理（不因格式怪异而 panic）。
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> (u32, u32, u32) {
        let mut it = s.split('.').map(|p| p.trim().parse::<u32>().unwrap_or(0));
        (
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
        )
    };
    parse(a).cmp(&parse(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_contains_name_and_version() {
        let t = title();
        assert!(t.starts_with(APP_NAME));
        assert!(t.contains(APP_VERSION));
        assert!(t.contains('v'));
    }

    #[test]
    fn version_matches_cargo_package() {
        // 防止常量与 Cargo.toml 脱节：两者必须同为语义化版本号。
        let parts: Vec<&str> = APP_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "版本号应为 MAJOR.MINOR.PATCH：{APP_VERSION}");
        for p in parts {
            assert!(p.chars().all(|c| c.is_ascii_digit()), "非法版本号：{APP_VERSION}");
        }
    }

    #[test]
    fn version_cmp_orders_semver() {
        use std::cmp::Ordering::*;
        assert_eq!(version_cmp("0.2.0", "0.2.0"), Equal);
        assert_eq!(version_cmp("0.2.0", "0.2.1"), Less);
        assert_eq!(version_cmp("0.3.0", "0.2.9"), Greater);
        assert_eq!(version_cmp("1.0.0", "0.99.99"), Greater);
        // 缺失/非法段按 0 处理，不 panic。
        assert_eq!(version_cmp("0.2", "0.2.0"), Equal);
        assert_eq!(version_cmp("0.2.x", "0.2.0"), Equal);
        assert_eq!(version_cmp("", "0.0.0"), Equal);
    }

    #[test]
    fn min_compatible_version_is_valid_and_not_above_current() {
        let parts: Vec<&str> = MIN_COMPATIBLE_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "最低兼容版本应为 MAJOR.MINOR.PATCH：{MIN_COMPATIBLE_VERSION}");
        assert!(
            version_cmp(MIN_COMPATIBLE_VERSION, APP_VERSION) != std::cmp::Ordering::Greater,
            "最低兼容版本不应高于当前版本 {APP_VERSION}"
        );
    }
}
