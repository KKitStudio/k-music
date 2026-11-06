//! 歌曲导入：从指定文件夹复制 / 移动到曲库，自动按 `config.library.file_scheme` 组织并入库。
//! - 普通音频：抽取标签 → 按 file_scheme 归位 → 登记（song_id=NULL，与扫描一致）。
//! - ncm 文件（有工具）：转换后按本地文件入库；原始 .ncm 归档 `download/ncm/`。
//! - ncm 文件（无工具）：暂存 `download/pendingNcm/`，不入库（待配置工具后转换）。
use crate::audio_meta::{extract, quality_tier_of};
use crate::config::Config;
use crate::db;
use crate::library::{convert_ncm, derive_names, file_created_ms, target_path};
use crate::lyrics;
use crate::progress::{EntryStatus, ImportAction, ProgressTracker};
use crate::Result;
use crate::is_playable_audio;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Copy,
    Move,
}

#[derive(Debug, Default)]
pub struct ImportResult {
    pub imported: u32,
    pub skipped: u32,
    /// 导入遇到 .ncm 但本地无 ncm 工具，已暂存到 download/pendingNcm/ 的数量（未入库）。
    pub pending: u32,
    pub failed: u32,
}

pub fn import_folder(
    cfg: &Config,
    conn: &Connection,
    src: &Path,
    mode: ImportMode,
) -> Result<ImportResult> {
    import_folder_with_progress(cfg, conn, src, mode, &ProgressTracker::new())
}

/// 带进度上报的导入：向 `tracker` 实时推送「当前处理哪一首、做什么动作」。
///
/// 流程：先递归扫描统计待处理文件总数（用于进度条百分比），再逐个导入。
/// 每个源文件在处理前后都会写入 tracker，前端据此渲染实时进度。
pub fn import_folder_with_progress(
    cfg: &Config,
    conn: &Connection,
    src: &Path,
    mode: ImportMode,
    tracker: &ProgressTracker,
) -> Result<ImportResult> {
    let mut res = ImportResult::default();
    // 先统计总数，前端才能显示百分比进度（否则只能显示不确定态的忙碌动画）。
    let total = count_importable(src);
    tracker.set_total(total);
    import_recursive(cfg, conn, src, mode, &mut res, tracker)?;
    Ok(res)
}

/// 递归统计目录下待导入的源文件数（可播放音频 + ncm）。
fn count_importable(dir: &Path) -> u32 {
    let mut n = 0u32;
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            n += count_importable(&path);
        } else if is_playable_audio(&path) || is_ncm(&path) {
            n += 1;
        }
    }
    n
}

/// 从「本地网易云音乐曲库目录」增量合并到 musicMT 曲库（本地操作，不联网、不登录）。
/// 单方向、只加不删：仅把曲库中没有的歌曲复制进来，绝不反向删除原曲库文件
/// （docs/02 FR-12、docs/05 §13）。`cfg.netease.library_dir` 为空时返回错误，提示前端先设置。
/// 复用 `import_folder` 的 Copy + 去重语义（目标已存在且大小一致则跳过）。
pub fn import_netease_library(cfg: &Config, conn: &Connection) -> Result<ImportResult> {
    import_netease_library_with_progress(cfg, conn, &ProgressTracker::new())
}

/// 带进度上报的网易云本地曲库增量合并（见 [`import_netease_library`]）。
pub fn import_netease_library_with_progress(
    cfg: &Config,
    conn: &Connection,
    tracker: &ProgressTracker,
) -> Result<ImportResult> {
    let dir = &cfg.netease.library_dir;
    if dir.as_os_str().is_empty() || !dir.is_dir() {
        anyhow::bail!("未设置有效的「本地网易云音乐曲库目录」，请先在设置或初始化向导中指定");
    }
    import_folder_with_progress(cfg, conn, dir, ImportMode::Copy, tracker)
}

/// 处理 pendingNcm 积压：把此前因「无工具」暂存的 .ncm 逐个按下载逻辑转换入库。
/// 调用时机：指定 NCM 工具成功之后（初始化向导 / 后续设置，任何时序统一走这里）。
/// 转换失败的文件留在 pendingNcm 原位，下次指定/重试可再处理。
pub fn process_pending_ncm(cfg: &Config, conn: &Connection) -> ImportResult {
    process_pending_ncm_with_progress(cfg, conn, &ProgressTracker::new())
}

/// 带进度上报的 pendingNcm 回补（见 [`process_pending_ncm`]）。
///
/// 这是**批量转换**场景（一次几十上百首），也是终端闪窗最明显的场景，
/// 因此进度上报与静默执行同样重要。
pub fn process_pending_ncm_with_progress(
    cfg: &Config,
    conn: &Connection,
    tracker: &ProgressTracker,
) -> ImportResult {
    let mut res = ImportResult::default();
    let pending_dir = cfg.paths.pending_ncm_dir();
    let rd = match std::fs::read_dir(&pending_dir) {
        Ok(rd) => rd,
        Err(_) => return res, // 目录不存在 = 无积压
    };
    // 先收集文件列表：既便于统计总数（进度条），也避免在遍历目录时同时修改它。
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_ncm(p))
        .collect();
    files.sort();
    tracker.set_total(files.len() as u32);
    for path in files {
        let fname = fname_of(&path).to_string_lossy().to_string();
        tracker.begin_file(&fname);
        match import_ncm_with_progress(cfg, conn, &path, ImportMode::Move, tracker) {
            Ok(ImportOutcome::Imported) => {
                res.imported += 1;
                tracker.end_file(EntryStatus::Imported);
            }
            Ok(ImportOutcome::Skipped) => {
                res.skipped += 1;
                tracker.end_file(EntryStatus::Skipped);
            }
            Ok(ImportOutcome::Pending) => {
                res.pending += 1; // 无工具时调用：原地保持暂存
                tracker.end_file(EntryStatus::Pending);
            }
            Err(e) => {
                eprintln!("pendingNcm 转换失败 {}: {}", path.display(), e);
                res.failed += 1;
                tracker.log(
                    ImportAction::Convert,
                    &fname,
                    EntryStatus::Failed,
                    &e.to_string(),
                );
                tracker.end_file(EntryStatus::Failed);
            }
        }
    }
    res
}

/// 导入单文件的结果（import_ncm 对外返回，serve/CLI 据此计数）。
pub enum ImportOutcome {
    Imported,
    Skipped,
    Pending,
}

fn import_recursive(
    cfg: &Config,
    conn: &Connection,
    dir: &Path,
    mode: ImportMode,
    res: &mut ImportResult,
    tracker: &ProgressTracker,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            import_recursive(cfg, conn, &path, mode, res, tracker)?;
        } else if is_playable_audio(&path) {
            let fname = fname_of(&path).to_string_lossy().to_string();
            tracker.begin_file(&fname);
            // 动作：复制 or 移动（由导入模式决定）。
            let action = match mode {
                ImportMode::Copy => ImportAction::Copy,
                ImportMode::Move => ImportAction::Move,
            };
            tracker.set_action(action);
            match import_one(cfg, conn, &path, mode) {
                Ok(true) => {
                    res.imported += 1;
                    tracker.log(action, &fname, EntryStatus::Imported, "");
                    tracker.end_file(EntryStatus::Imported);
                }
                Ok(false) => {
                    res.skipped += 1;
                    tracker.log(
                        ImportAction::Skip,
                        &fname,
                        EntryStatus::Skipped,
                        "曲库中已存在",
                    );
                    tracker.end_file(EntryStatus::Skipped);
                }
                Err(e) => {
                    eprintln!("导入失败 {}: {}", path.display(), e);
                    res.failed += 1;
                    tracker.log(action, &fname, EntryStatus::Failed, &e.to_string());
                    tracker.end_file(EntryStatus::Failed);
                }
            }
        } else if is_ncm(&path) {
            let fname = fname_of(&path).to_string_lossy().to_string();
            tracker.begin_file(&fname);
            match import_ncm_with_progress(cfg, conn, &path, mode, tracker) {
                Ok(ImportOutcome::Imported) => {
                    res.imported += 1;
                    tracker.end_file(EntryStatus::Imported);
                }
                Ok(ImportOutcome::Skipped) => {
                    res.skipped += 1;
                    tracker.end_file(EntryStatus::Skipped);
                }
                Ok(ImportOutcome::Pending) => {
                    res.pending += 1;
                    tracker.end_file(EntryStatus::Pending);
                }
                Err(e) => {
                    eprintln!("导入 ncm 失败 {}: {}", path.display(), e);
                    res.failed += 1;
                    tracker.log(ImportAction::Convert, &fname, EntryStatus::Failed, &e.to_string());
                    tracker.end_file(EntryStatus::Failed);
                }
            }
        }
    }
    Ok(())
}

fn is_ncm(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()) == Some("ncm")
}

/// 取文件名（含扩展名）为 PathBuf；缺失时返回空 PathBuf。
fn fname_of(p: &Path) -> PathBuf {
    p.file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default()
        .into()
}

/// 按模式复制/移动源文件到目标。
fn copy_or_move(mode: ImportMode, src: &Path, dst: &Path) -> Result<()> {
    if let ImportMode::Copy = mode {
        std::fs::copy(src, dst)?;
    } else {
        std::fs::rename(src, dst)?;
    }
    Ok(())
}

/// 导入一个可播放音频文件（普通文件 / ncm 转换产物）到曲库正式位置并入库。
/// 去重：目标已存在且大小一致 → 跳过（返回 false，不移动源文件）；否则复制/移动并登记。
/// 返回 Ok(true)=新入库；Ok(false)=已存在跳过。
fn import_audio_file(cfg: &Config, conn: &Connection, audio: &Path, mode: ImportMode) -> Result<bool> {
    let meta = extract(audio)?;
    let tier = quality_tier_of(&meta);
    let (name, artist, album) = derive_names(audio, &meta);
    let ext = audio
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let target = target_path(
        cfg,
        &artist.clone().unwrap_or_default(),
        &album.clone().unwrap_or_default(),
        &name,
        &ext,
    );

    // 同步导入同名 .lrc 歌词文件（若源文件旁存在）：歌词与音频同目录、同 stem。
    // 必须在移动源音频**之前**定位（移动后源路径已失效）。
    let sidecar = lyrics::find_sidecar(audio);

    if target.exists() {
        let existing_size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        let new_size = std::fs::metadata(audio).map(|m| m.len()).unwrap_or(0);
        if existing_size == new_size {
            // 已存在且大小一致 → 跳过音频；但若目标缺少歌词文件，仍补齐歌词。
            if let Some(src) = &sidecar {
                let dst = target.with_extension("lrc");
                if !dst.exists() {
                    let _ = std::fs::copy(src, &dst);
                }
            }
            return Ok(false); // 已存在且大小一致，跳过
        }
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match mode {
        ImportMode::Copy => {
            std::fs::copy(audio, &target)?;
        }
        ImportMode::Move => {
            std::fs::rename(audio, &target)?;
        }
    }
    // 歌词文件随音频一起落到曲库（与音频同目录、同 stem）。
    if let Some(src) = &sidecar {
        let dst = target.with_extension("lrc");
        let moved = match mode {
            ImportMode::Move => std::fs::rename(src, &dst).is_ok(),
            ImportMode::Copy => false,
        };
        if !moved {
            // 移动失败（如跨卷）时退回复制；复制模式直接复制。
            let _ = std::fs::copy(src, &dst);
        }
    }
    let size = std::fs::metadata(&target).map(|m| m.len() as i64).unwrap_or(0);
    let mtime = std::fs::metadata(&target)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // 归位后的文件创建时间：复制模式会得到「导入时刻」的新 birth time，
    // 移动（rename）模式则保留源文件的创建时间——这正是两列要区分的信息。
    let created_ms = file_created_ms(&target);
    let inserted = db::upsert_local_record(
        conn,
        &target.to_string_lossy(),
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
    Ok(inserted)
}

fn import_one(cfg: &Config, conn: &Connection, src: &Path, mode: ImportMode) -> Result<bool> {
    import_audio_file(cfg, conn, src, mode)
}

/// 导入一个 .ncm 文件：有工具则转换后入库（song_id=NULL），无工具则暂存 pendingNcm。
/// 「有工具」的唯一判据 = tool_ready()（指定了工具即自动转换，2026-10-28 与用户确认）。
pub fn import_ncm(cfg: &Config, conn: &Connection, src: &Path, mode: ImportMode) -> Result<ImportOutcome> {
    import_ncm_with_progress(cfg, conn, src, mode, &ProgressTracker::new())
}

/// 带进度上报的 ncm 导入：转换阶段会把「正在转换哪一首」推给前端。
///
/// 进度语义：一个 ncm 源文件会产生**两条**动作日志——「转换」（调用外部工具解密）
/// 与「入库」（把解出的音频归位到曲库）；但结果计数仍只算**一个**源文件。
pub fn import_ncm_with_progress(
    cfg: &Config,
    conn: &Connection,
    src: &Path,
    mode: ImportMode,
    tracker: &ProgressTracker,
) -> Result<ImportOutcome> {
    if cfg.ncm.tool_ready() {
        let fname = fname_of(src).to_string_lossy().to_string();
        tracker.begin_file(&fname);
        tracker.set_action(ImportAction::Convert);
        // 有工具：先把 .ncm 搬运到 download/ 暂存，避免污染用户源目录，且 convert_ncm 在同目录产出音频。
        let stage = cfg.paths.download_dir.join(&fname);
        if let Some(p) = stage.parent() {
            std::fs::create_dir_all(p)?;
        }
        let _ = std::fs::remove_file(&stage);
        // 同名 .lrc 一并搬运到 download/（若源 .ncm 旁有）：转换产出的音频与它同目录同 stem，
        // 后续 import_audio_file 即可命中歌词。
        let sidecar = lyrics::find_sidecar(src);
        copy_or_move(mode, src, &stage)?;
        if let Some(lrc) = &sidecar {
            let _ = std::fs::copy(lrc, &stage.with_extension("lrc"));
        }
        // 转换（在 download/ 同目录产出解码后的音频文件）。
        let audio_out = convert_ncm(cfg, &stage)?;
        tracker.log(
            ImportAction::Convert,
            &fname,
            EntryStatus::Imported,
            "转换完成",
        );
        // 原始 .ncm 归档到 download/ncm/（同名先清，避免占用）。
        let ncm_archive = cfg.paths.ncm_raw_dir.join(&fname);
        if let Some(p) = ncm_archive.parent() {
            std::fs::create_dir_all(p)?;
        }
        let _ = std::fs::remove_file(&ncm_archive);
        std::fs::rename(&stage, &ncm_archive)?;
        // 转换后音频整理入库（song_id=NULL，与本地扫描/导入一致）。转换产物是临时文件，用 Move 直接移走。
        tracker.set_action(ImportAction::Move);
        let out_name = fname_of(&audio_out).to_string_lossy().to_string();
        let inserted = import_audio_file(cfg, conn, &audio_out, ImportMode::Move)?;
        tracker.log(
            ImportAction::Move,
            &out_name,
            if inserted {
                EntryStatus::Imported
            } else {
                EntryStatus::Skipped
            },
            if inserted { "" } else { "曲库中已存在" },
        );
        // 去重跳过时 audio_out 残留 download/，清理以免占用。
        if !inserted {
            let _ = std::fs::remove_file(&audio_out);
        }
        Ok(if inserted {
            ImportOutcome::Imported
        } else {
            ImportOutcome::Skipped
        })
    } else {
        // 无工具：原 .ncm 暂存到 download/pendingNcm/（不入库，待后续配置工具转换）。
        // 已在该目录内（例如 process_pending_ncm 重复处理）则直接跳过，避免重命名到自身报错。
        let pending_dir = cfg.paths.pending_ncm_dir();
        if src.starts_with(&pending_dir) {
            return Ok(ImportOutcome::Pending);
        }
        let fname = fname_of(src).to_string_lossy().to_string();
        tracker.set_action(ImportAction::Stage);
        let pending = pending_dir.join(&fname);
        if let Some(p) = pending.parent() {
            std::fs::create_dir_all(p)?;
        }
        let _ = std::fs::remove_file(&pending);
        copy_or_move(mode, src, &pending)?;
        tracker.log(
            ImportAction::Stage,
            &fname,
            EntryStatus::Pending,
            "无 NCM 工具，已暂存待转换",
        );
        Ok(ImportOutcome::Pending)
    }
}

#[cfg(test)]
mod import_ncm_tests {
    use super::*;
    use crate::config;
    use crate::db;
    use std::path::Path;

    /// 写一段最小合法 PCM WAV（lofty 可解析），供假 ncmdump 产出与直接音频入库。
    fn write_valid_wav(path: &Path) {
        let sr: u32 = 44100;
        let ch: u16 = 2;
        let bps: u16 = 16;
        let data_len: u32 = sr * ch as u32 * (bps as u32 / 8); // 1 秒静音
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_len).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&ch.to_le_bytes());
        buf.extend_from_slice(&sr.to_le_bytes());
        buf.extend_from_slice(&(sr * ch as u32 * (bps as u32 / 8)).to_le_bytes());
        buf.extend_from_slice(&(ch * (bps / 8)).to_le_bytes());
        buf.extend_from_slice(&bps.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_len.to_le_bytes());
        buf.extend(std::iter::repeat(0u8).take(data_len as usize));
        std::fs::write(path, &buf).unwrap();
    }

    /// 统计曲库根下、排除 download/ 暂存树的音频文件数。
    fn count_library_audio(root: &Path) -> usize {
        let staging = root.join("download");
        fn walk(dir: &Path, staging: &Path, n: &mut usize) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p == *staging {
                        continue;
                    }
                    if p.is_dir() {
                        walk(&p, staging, n);
                    } else if crate::is_playable_audio(&p) {
                        *n += 1;
                    }
                }
            }
        }
        let mut n = 0;
        walk(root, &staging, &mut n);
        n
    }

    /// 统计曲库根下（含 download/ 暂存树）文件名包含 `needle` 的音频文件数。
    fn count_audio_named(root: &Path, needle: &str) -> usize {
        fn walk(dir: &Path, needle: &str, n: &mut usize) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, needle, n);
                    } else if crate::is_playable_audio(&p)
                        && p.file_name()
                            .map(|x| x.to_string_lossy().contains(needle))
                            .unwrap_or(false)
                    {
                        *n += 1;
                    }
                }
            }
        }
        let mut n = 0;
        walk(root, needle, &mut n);
        n
    }

    #[test]
    fn import_ncm_no_tool_branches_to_pending() {
        let root = std::env::temp_dir().join(format!("mt_imp_{}_a", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let cfg = config::build_library_config(&root, "artist_album", "");
        cfg.ensure_dirs().unwrap();
        let conn = db::open(&cfg).unwrap();

        // 源文件夹（放在 root 之外，避免被曲库音频计数误计）：一个普通 wav + 一个 ncm（当前无工具，enabled=false）。
        let src = std::env::temp_dir().join(format!("mt_imp_src_{}_a", std::process::id()));
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&src).unwrap();
        let wav = src.join("trackA.wav");
        write_valid_wav(&wav);
        let ncm = src.join("trackB.ncm");
        std::fs::write(&ncm, b"FAKE_NCM_BYTES").unwrap();

        let res = import_folder(&cfg, &conn, &src, ImportMode::Copy).unwrap();
        assert_eq!(res.imported, 1, "普通音频应入库 1");
        assert_eq!(res.pending, 1, "ncm 无工具应暂存 1");
        assert_eq!(res.failed, 0);
        // 曲库正式位置有 1 个音频（普通 wav 已归位）。
        assert_eq!(count_library_audio(&root), 1, "曲库应有 1 个音频");
        // ncm 应进入 download/pendingNcm/，原文件仍在 src（Copy 模式）。
        let pending = cfg.paths.pending_ncm_dir().join("trackB.ncm");
        assert!(pending.is_file(), "ncm 应暂存到 pendingNcm");
        assert!(ncm.is_file(), "Copy 模式原 ncm 应保留");
        // download/ncm 不应有归档（无工具不转换）。
        assert_eq!(
            std::fs::read_dir(cfg.paths.ncm_raw_dir).unwrap().count(),
            0,
            "无工具不应归档 ncm"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_ncm_with_tool_converts_and_archives() {
        let root = std::env::temp_dir().join(format!("mt_imp_{}_b", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let mut cfg = config::build_library_config(&root, "artist_album", "");
        cfg.ensure_dirs().unwrap();
        let conn = db::open(&cfg).unwrap();

        // 准备假 ncmdump：把 sample.wav 复制到与输入同目录、同名（换扩展名 .wav）。
        // sample 与源文件夹都放在 root 之外，避免被曲库音频计数误计。
        let sample = std::env::temp_dir().join(format!("mt_imp_smp_{}.wav", std::process::id()));
        write_valid_wav(&sample);
        let tool_path = cfg.paths.tools_dir.join("fake_ncmdump.cmd");
        let cmd = format!(
            "@echo off\r\ncopy /Y \"{}\" \"%~dpn1.wav\" >nul\r\n",
            sample.display()
        );
        std::fs::write(&tool_path, cmd).unwrap();
        cfg.ncm.tool_path = tool_path.clone();
        cfg.ncm.command_template = r#"cmd /C "{tool}" "{input}""#.into();

        // 源文件夹：一个 ncm（有工具）。
        let src = std::env::temp_dir().join(format!("mt_imp_src_{}_b", std::process::id()));
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&src).unwrap();
        let ncm = src.join("trackC.ncm");
        std::fs::write(&ncm, b"FAKE_NCM_BYTES_C").unwrap();

        let res = import_folder(&cfg, &conn, &src, ImportMode::Copy).unwrap();
        assert_eq!(res.imported, 1, "转换后音频应入库 1");
        assert_eq!(res.pending, 0);
        assert_eq!(res.failed, 0);
        // 转换产物已归位（trackC 命名的音频在曲库中存在 1 个；假样本无标签时艺人取父目录，
        // 故可能落在 download/ 子树下，这里全树计数而非排除暂存树）。
        assert_eq!(count_audio_named(&root, "trackC"), 1, "应存在 1 个转换音频");
        // 原始 ncm 应归档到 download/ncm/。
        let archive = cfg.paths.ncm_raw_dir.join("trackC.ncm");
        assert!(archive.is_file(), "原始 ncm 应归档到 download/ncm");
        // pendingNcm 不应有残留。
        assert_eq!(
            std::fs::read_dir(cfg.paths.pending_ncm_dir()).unwrap().count(),
            0,
            "有工具不应进 pendingNcm"
        );
        // download/ 暂存树不应残留 trackC 相关文件。
        let left_in_dl = std::fs::read_dir(cfg.paths.download_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains("trackC"));
        assert!(!left_in_dl, "download/ 不应残留 trackC 文件");

        let _ = std::fs::remove_dir_all(&root);
    }
}
