//! mt-core：musicMT 的纯 Rust 核心库（UI 无关）。
//! 涵盖配置、SQLite 存储、音频元数据、曲库扫描、导入、NCM 转换、播放文件服务。
//! 网易云服务端登录/下载已于 2026-10-29 移除；未来若恢复同步将基于官方开发者 API 另起模块。
pub mod app;
pub mod audio_meta;
pub mod config;
pub mod db;
pub mod downloads;
pub mod duplicates;
pub mod import;
pub mod library;
pub mod log;
pub mod lyrics;
pub mod playlist_io;
pub mod playstate;
pub mod progress;
pub mod serve;

pub type Result<T> = anyhow::Result<T>;

/// 支持的可播放音频扩展名（用于扫描/导入/播放范围判定）。
pub const PLAYABLE_EXTS: &[&str] = &["mp3", "flac", "m4a", "wav", "ogg", "opus", "aac"];
/// NCM 加密文件扩展名（未转换前不可直接播放）。
pub const NCM_EXT: &str = "ncm";

/// 判断路径扩展名是否可播放音频。
pub fn is_playable_audio(path: &std::path::Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => PLAYABLE_EXTS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// 创建一个**不弹控制台窗口**的子进程命令。
///
/// ## 背景
/// GUI 进程（`windows_subsystem = "windows"`，无自有控制台）调用外部 exe 时，
/// Windows 会为该子进程**新建一个控制台窗口**并显示出来。每转换一首 NCM 就闪一次，
/// 歌曲多时形成持续闪烁，观感很差。
///
/// ## 解决
/// Windows 提供 `CREATE_NO_WINDOW`（0x0800_0000）创建标志：进程仍正常运行，
/// 但不分配/显示控制台窗口。配合 `stdout`/`stderr` 捕获（`Command::output()`），
/// 子进程既不闪窗，输出也不会污染宿主终端。
///
/// 非 Windows 平台无需处理（POSIX 无此概念），仅原样返回命令。
pub fn silent_command(program: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW：子进程不创建控制台窗口。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
