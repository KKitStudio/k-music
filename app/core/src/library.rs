//! 曲库扫描与文件整理入库。
//! - scan_library：遍历 download_root，抽取音质元数据并入库（本地文件 song_id 为 NULL）。
//! - 内部工具：derive_names / target_path / convert_ncm 同时供 import 模块复用。
//! 说明：网易云服务端登录与下载已于 2026-10-29 移除；同步能力未来若恢复将基于官方开发者 API
//! 另起授权模块，本文件不再保留 WEAPI 相关逻辑。
use crate::audio_meta::{extract, quality_tier_of, AudioMeta};
use crate::config::Config;
use crate::db;
use crate::Result;
use crate::is_playable_audio;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// 递归扫描曲库目录，写入 download_records，并清理磁盘上已不存在的孤儿记录。
pub fn scan_library(cfg: &Config, conn: &Connection) -> Result<db::ScanStats> {
    let mut stats = db::ScanStats {
        scanned: 0,
        inserted: 0,
        updated: 0,
        removed: 0,
        loose_ncm: Vec::new(),
    };
    scan_dir(&cfg.paths.download_root, &mut stats, conn)?;
    // 双向对账：删掉磁盘上已不存在的本地文件记录（用户在工具外删除/移动）。
    stats.removed = db::prune_missing(conn)?;
    Ok(stats)
}

fn scan_dir(root: &Path, stats: &mut db::ScanStats, conn: &Connection) -> Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .map(|n| n == "_ncm_raw" || n == "ing" || n == "ncm" || n == "pendingNcm")
                .unwrap_or(false)
            {
                continue; // 暂存/原始目录不参与可播放曲库扫描
            }
            scan_dir(&path, stats, conn)?;
        } else if is_playable_audio(&path) {
            scan_file(&path, stats, conn)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("ncm") {
            // 散落 .ncm（不在 ing/ncm/pendingNcm 暂存目录内）：不作为音频入库，仅记录路径，
            // 由前端扫描完成后弹窗询问是否移入 download 按下载逻辑处理。
            stats.loose_ncm.push(path.to_string_lossy().to_string());
        }
    }
    Ok(())
}

fn scan_file(path: &Path, stats: &mut db::ScanStats, conn: &Connection) -> Result<()> {
    stats.scanned += 1;
    let size = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0);
    let mtime = mtime_secs(path);
    let created_ms = file_created_ms(path);
    let path_s = path.to_string_lossy().to_string();
    // 增量门控：已有记录且 (file_size, file_mtime) 均未变 → 跳过 lofty 抽取（省去主要 I/O）。
    // mtime 不可得（0）或缓存为 NULL 时视为「未缓存」，下探抽取。
    if let Some((cached_size, cached_mtime)) = db::get_cached_file_info(conn, &path_s)? {
        if cached_size == size && cached_mtime == Some(mtime) && mtime != 0 {
            // 文件未变：仍校正「文件创建时间」。旧库迁移时该列回填的是 file_mtime 近似值，
            // 这里趁扫描把它换成真实 birth time；值没变化则 refresh 内部不会产生写入。
            if let Some(ms) = created_ms {
                db::refresh_created_at(conn, &path_s, ms)?;
            }
            return Ok(()); // 未变更，无需重抽
        }
    }
    let meta = extract(path)?;
    let tier = quality_tier_of(&meta);
    let (name, artist, album) = derive_names(path, &meta);
    let inserted = db::upsert_local_record(
        conn,
        &path_s,
        &name,
        artist.as_deref(),
        album.as_deref(),
        meta.duration_ms.map(|v| v as i64),
        Some(&meta.format),
        meta.sample_rate.map(|v| v as i64),
        meta.bit_depth.map(|v| v as i64),
        meta.bitrate.map(|v| v as i64),
        meta.channels.map(|v| v as i64),
        Some(&tier),
        size,
        mtime,
        created_ms,
    )?;
    if inserted {
        stats.inserted += 1;
    } else {
        stats.updated += 1;
    }
    Ok(())
}

/// 取文件修改时间（秒，UNIX 纪元）。不可得时返回 0（调用方据此跳过门控、下探抽取）。
fn mtime_secs(path: &Path) -> i64 {
    use std::time::UNIX_EPOCH;
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 取音频文件的**文件系统创建时间**（UNIX 毫秒），供列表「创建时间」列显示。
///
/// Windows 上 `Metadata::created()` 即资源管理器里的「创建时间」（birth time）。
/// 部分文件系统 / 平台不提供 birth time → 退化为文件修改时间，保证这一列不会整列为空；
/// 完全取不到（文件已消失等）时返回 None，前端按「隐藏而非占位」契约渲染空单元格。
pub fn file_created_ms(path: &Path) -> Option<i64> {
    use std::time::UNIX_EPOCH;
    let md = std::fs::metadata(path).ok()?;
    let t = md.created().ok().or_else(|| md.modified().ok())?;
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

/// 计算下载/导入目标路径：按 `cfg.library.file_scheme` 组织到 download_root 下。
/// 方案见 docs/05 §4a：artist_album | album | artist | flat。
pub fn target_path(cfg: &Config, artist: &str, album: &str, title: &str, ext: &str) -> PathBuf {
    let rel = crate::config::target_path(&cfg.library.file_scheme, artist, album, title);
    let mut p = cfg.paths.download_root.join(&rel);
    p.set_extension(ext);
    p
}

/// 从文件路径与音频标签推导（曲名, 艺人, 专辑）。标签优先，缺失时回退到目录/文件名。
pub fn derive_names(path: &Path, meta: &AudioMeta) -> (String, Option<String>, Option<String>) {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("未知曲目")
        .to_string();
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());
    let name = meta.title.clone().unwrap_or(stem);
    let artist = meta.artist.clone().or(parent);
    let album = meta.album.clone();
    (name, artist, album)
}

/// 调用 ncm 工具转换：在 `input` 同目录产出解码后的音频文件，返回其路径。
/// 通过 config.ncm.command_template 替换 {tool}/{input} 执行（默认 `{tool} "{input}"`）。
/// 提取为 pub 供 import 模块复用（导入 ncm 分支）。
pub fn convert_ncm(cfg: &Config, input: &Path) -> Result<PathBuf> {
    let tool = cfg.ncm.tool_path.to_string_lossy().to_string();
    let input_s = input.to_string_lossy().to_string();
    let cmd = cfg
        .ncm
        .command_template
        .replace("{tool}", &tool)
        .replace("{input}", &input_s);
    let parts = shell_split(&cmd);
    if parts.is_empty() {
        return Err(anyhow::anyhow!("ncm 命令模板为空"));
    }
    let parent = input
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    // 记录转换前目录内文件，用于定位新产出的音频。
    let before: std::collections::HashSet<PathBuf> = std::fs::read_dir(&parent)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    let mut command = crate::silent_command(&parts[0]);
    for a in &parts[1..] {
        command.arg(a);
    }
    // 捕获子进程输出：既避免污染宿主终端，也能在失败时把工具的 stderr 带回错误信息。
    // 配合 silent_command 的 CREATE_NO_WINDOW，转换过程中不会反复弹出控制台窗口。
    let output = command
        .output()
        .map_err(|e| anyhow::anyhow!("执行 ncm 工具失败：{e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            anyhow::anyhow!("ncm 工具返回非零退出码：{}", output.status)
        } else {
            anyhow::anyhow!("ncm 工具返回非零退出码：{}（{}）", output.status, detail)
        });
    }
    // 找新出现的音频文件（非 input 本身）。
    let after: Vec<PathBuf> = std::fs::read_dir(&parent)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    for p in &after {
        if p.as_path() == input {
            continue;
        }
        if is_playable_audio(p) && !before.contains(p) {
            return Ok(p.clone());
        }
    }
    Err(anyhow::anyhow!("ncm 转换后未找到产出音频文件"))
}

/// 极简命令行切分：按空白分割，支持双引号包裹的整段（可含空白）。
fn shell_split(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in s.chars() {
        match c {
            '"' => in_q = !in_q,
            c if c.is_whitespace() && !in_q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
