//! 曲库扫描与文件整理入库。
//! - scan_library：遍历 download_root，抽取音质元数据并入库（本地文件 song_id 为 NULL）。
//! - 内部工具：derive_names / target_path / convert_ncm 同时供 import 模块复用。
//! 说明：网易云服务端登录与下载已于 2026-10-29 移除；同步能力未来若恢复将基于官方开发者 API
//! 另起授权模块，本文件不再保留 WEAPI 相关逻辑。
use crate::audio_meta::AudioMeta;
use crate::config::Config;
use crate::db;
use crate::duplicates::{self, Candidate, FileMode, Origin};
use crate::Result;
use crate::is_playable_audio;
use anyhow::{anyhow, bail};
use rusqlite::{Connection, OptionalExtension};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// 递归扫描曲库目录，写入 download_records，并清理磁盘上已不存在的孤儿记录。
///
/// FR-22 W3-A′ 两阶段：阶段① 只把「新增/变更」文件收集成候选（`(size,mtime)` 门控仍生效），
/// 阶段② 由 `duplicates::finalize()` 按三元组分桶统一判定「正常入库 / 静默跳过 / 挂起成重复组」。
/// 启动 boot scan 只静默登记挂起组（P1①），不弹窗；角标由 `/api/status` 读当前曲库的未决组数。
pub fn scan_library(cfg: &Config, conn: &Connection) -> Result<db::ScanStats> {
    let mut stats = db::ScanStats {
        scanned: 0,
        inserted: 0,
        updated: 0,
        removed: 0,
        held: 0,
        loose_ncm: Vec::new(),
    };
    let mut batch: Vec<Candidate> = Vec::new();
    scan_dir(&cfg.paths.download_root, cfg, conn, &mut stats, &mut batch)?;
    let outcome = duplicates::finalize(cfg, conn, &batch)?;
    stats.inserted += outcome.imported;
    stats.updated += outcome.updated;
    stats.held = outcome.held;
    // 双向对账：删掉磁盘上已不存在的本地文件记录（用户在工具外删除/移动）。
    stats.removed = db::prune_missing(conn)?;
    Ok(stats)
}

/// 「删除歌曲」的结果摘要（对应 docs/01 FR-21）。
#[derive(Debug, Clone)]
pub struct TrashOutcome {
    pub name: String,
    /// 文件已移入回收站时的目标路径；源文件本就不存在时为 None（仅清理索引）。
    pub trashed_path: Option<String>,
    /// 被一并清掉的播放列表引用条数。
    pub playlists_affected: u32,
}

/// 把一首曲目移入曲库回收站 `<曲库>/.mt/trash/`，并从索引与所有播放列表移除。
///
/// - **不做不可逆删除**：`std::fs::rename` 到回收站，用户可手动取回；同名自动加 `_1/_2` 后缀。
/// - **先移文件、后删索引**；索引删除失败时把文件移回，保证「文件在回收站 ⟺ 曲库里没有它」。
/// - 只删索引是无效的（文件还在磁盘，下次扫描会重新入库），故两者必须同时发生，
///   且扫描端必须跳过 `.mt`（见 `is_excluded_dir`）。
/// - 路径必须落在当前曲库根内，防止 DB 里的 file_path 被篡改成指向任意位置的文件。
pub fn trash_track(cfg: &Config, conn: &Connection, local_id: i64) -> Result<TrashOutcome> {
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT name, file_path FROM download_records WHERE local_id=?1",
            [local_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (name, file_path) = row.ok_or_else(|| anyhow!("曲目不存在或已被删除"))?;
    let name = name.unwrap_or_default();

    // 无路径的历史占位记录：没有文件可移，只清索引。
    let Some(file_path) = file_path.filter(|s| !s.trim().is_empty()) else {
        let affected = db::delete_track(conn, local_id)?;
        return Ok(TrashOutcome {
            name,
            trashed_path: None,
            playlists_affected: affected,
        });
    };

    let src = ensure_within_library(cfg, Path::new(&file_path))?;
    let trash_dir = cfg.paths.app_data.join("trash");
    std::fs::create_dir_all(&trash_dir)?;

    let mut moved: Option<PathBuf> = None;
    if src.exists() {
        let dest = unique_destination(&trash_dir, src.file_name().unwrap_or_else(|| OsStr::new("track")));
        std::fs::rename(&src, &dest).map_err(|e| {
            anyhow!("无法把文件移入回收站：{e}（文件可能被播放器占用，曲目索引保持不变）")
        })?;
        moved = Some(dest);
    }

    match db::delete_track(conn, local_id) {
        Ok(affected) => Ok(TrashOutcome {
            name,
            trashed_path: moved.map(|p| p.to_string_lossy().to_string()),
            playlists_affected: affected,
        }),
        Err(e) => {
            // 索引没删掉 → 把文件放回原位，避免「磁盘上没了但曲库还列着」。
            if let Some(dest) = &moved {
                let _ = std::fs::rename(dest, &src);
            }
            Err(e)
        }
    }
}

/// 校验并规范化曲目路径：必须是绝对（或相对曲库根）且词法上落在当前曲库根内。
/// 供 `trash_track` 与 `/api/tracks/reveal` 共用，避免越界操作曲库外文件。
pub fn ensure_within_library(cfg: &Config, path: &Path) -> Result<PathBuf> {
    let root = lexical_clean(&cfg.paths.download_root);
    let abs = if path.is_absolute() {
        lexical_clean(path)
    } else {
        lexical_clean(&root.join(path))
    };
    if !abs.starts_with(&root) {
        bail!("文件不在当前曲库目录内，拒绝操作：{}", abs.display());
    }
    Ok(abs)
}

/// 词法消除 `.` / `..`（不访问磁盘，对已不存在的文件同样有效）。
fn lexical_clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 回收站里同名文件自动避让：`<stem>_1.<ext>`、`<stem>_2.<ext>`……
pub(crate) fn unique_destination(dir: &Path, name: &OsStr) -> PathBuf {
    let base = Path::new(name);
    let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
    let ext = base.extension().and_then(|s| s.to_str()).unwrap_or("");
    let direct = dir.join(name);
    if !direct.exists() {
        return direct;
    }
    for i in 1..1000 {
        let candidate = if ext.is_empty() {
            dir.join(format!("{stem}_{i}"))
        } else {
            dir.join(format!("{stem}_{i}.{ext}"))
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    dir.join(format!("{stem}_{ms}.{ext}"))
}

/// 曲库扫描要跳过的目录名：暂存目录 + **元数据目录 `.mt`** + 回收站目录 `trash`。
///
/// `.mt` 里放 config.toml / library.db / playstate.json / trash/，本就不是音乐目录；
/// 尤其 `trash/` 存的是「已删除」的音频，若不跳过，下次扫描会把它们**重新入库**，
/// 右键「删除歌曲」就会失效。FR-22 的 `pending-duplicates/` 同理（挂起文件不能重新入库）。
/// 曲库根即扫描根（`cfg.paths.download_root`），故这层判断是必需的。
fn is_excluded_dir(name: &OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(".mt")
            | Some("_ncm_raw")
            | Some("ing")
            | Some("ncm")
            | Some("pendingNcm")
            | Some("pending-duplicates")
            | Some("trash")
    )
}

fn scan_dir(
    root: &Path,
    cfg: &Config,
    conn: &Connection,
    stats: &mut db::ScanStats,
    batch: &mut Vec<Candidate>,
) -> Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().map(is_excluded_dir).unwrap_or(false) {
                continue; // 暂存 / 元数据 / 回收站 / 挂起目录不参与可播放曲库扫描
            }
            scan_dir(&path, cfg, conn, stats, batch)?;
        } else if is_playable_audio(&path) {
            scan_file(&path, cfg, conn, stats, batch)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("ncm") {
            // 散落 .ncm（不在 ing/ncm/pendingNcm 暂存目录内）：不作为音频入库，仅记录路径，
            // 由前端扫描完成后弹窗询问是否移入 download 按下载逻辑处理。
            stats.loose_ncm.push(path.to_string_lossy().to_string());
        }
    }
    Ok(())
}

/// 阶段①：只收集候选，不写 DB、不搬文件。判重与入库统一在批次结束后做（P4 / W3-A′）。
fn scan_file(
    path: &Path,
    cfg: &Config,
    conn: &Connection,
    stats: &mut db::ScanStats,
    batch: &mut Vec<Candidate>,
) -> Result<()> {
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
    match duplicates::candidate_from_audio(cfg, path, FileMode::Scan, Origin::Scan) {
        Ok(c) => batch.push(c),
        Err(e) => eprintln!("扫描抽取元数据失败 {}: {e}", path.display()),
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

#[cfg(test)]
mod trash_tests {
    use super::*;

    #[test]
    fn excluded_dirs_include_metadata_and_trash() {
        for n in [
            ".mt",
            "trash",
            "pending-duplicates",
            "_ncm_raw",
            "ing",
            "ncm",
            "pendingNcm",
        ] {
            assert!(is_excluded_dir(OsStr::new(n)), "{n} 必须跳过扫描");
        }
        for n in ["artist", "范特西", "download", "周杰伦"] {
            assert!(!is_excluded_dir(OsStr::new(n)), "{n} 是普通音乐目录，不能跳过");
        }
    }

    #[test]
    fn lexical_clean_resolves_parent_dirs() {
        assert_eq!(
            lexical_clean(Path::new("D:/lib/artist/../album/a.flac")),
            PathBuf::from("D:/lib/album/a.flac")
        );
        assert_eq!(
            lexical_clean(Path::new("D:/lib/./x//y.flac")),
            PathBuf::from("D:/lib/x/y.flac")
        );
    }

    #[test]
    fn unique_destination_avoids_existing_names() {
        let dir = std::env::temp_dir().join(format!("kqk-trash-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("song.flac"), b"x").unwrap();
        std::fs::write(dir.join("song_1.flac"), b"x").unwrap();
        let got = unique_destination(&dir, OsStr::new("song.flac"));
        assert_eq!(got, dir.join("song_2.flac"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
