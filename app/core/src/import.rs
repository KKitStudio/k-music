//! 歌曲导入：从指定文件夹复制 / 移动到曲库，自动按 `config.library.file_scheme` 组织并入库。
//! - 普通音频：抽取标签 → 按 file_scheme 归位 → 登记（song_id=NULL，与扫描一致）。
//! - ncm 文件（有工具）：转换后按本地文件入库；原始 .ncm 归档 `download/ncm/`。
//! - ncm 文件（无工具）：暂存 `download/pendingNcm/`，不入库（待配置工具后转换）。
//!
//! FR-22：一次导入任务 = 一个**两阶段批次**（W3-A′）。阶段① 只收集候选（不写 DB、不归位、
//! 不弹窗），阶段② 由 `duplicates::finalize()` 统一判定「入库 / 静默跳过 / 挂起成重复组」。
//! `origin` 必须沿调用链**显式传递**：网易云同步与手工导入共用同一个 `import_folder_with_progress()`，
//! 仅靠 ImportMode 或源目录无法区分来源（P2 按来源分流的实施硬约束）。
use crate::config::Config;
use crate::duplicates::{self, Candidate, Disposition, FileMode, Origin};
use crate::library::convert_ncm;
use crate::lyrics;
use crate::progress::{EntryStatus, ImportAction, ProgressTracker};
use crate::Result;
use crate::is_playable_audio;
use anyhow::anyhow;
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
    /// FR-22：登记为「重复待决策」的文件数（挂起进 `.mt/pending-duplicates*`，等 GUI 批量对比）。
    pub held: u32,
}

/// 导入单文件的结果（供 serve / CLI 计数）。
pub enum ImportOutcome {
    Imported,
    Skipped,
    Pending,
    /// 重复项已挂起，等待 GUI 批量对比决策（CLI 无 UI 时不就地默认保留第一个）。
    Held,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    Audio,
    Ncm,
}

#[derive(Debug, Clone)]
struct SourceItem {
    path: PathBuf,
    fname: String,
    kind: SourceKind,
}

enum Prepared {
    Candidate {
        index: usize,
        temp: bool,
        fname: String,
        action: ImportAction,
    },
    PendingNcm {
        fname: String,
    },
    Failed {
        fname: String,
        message: String,
        action: ImportAction,
    },
}

pub fn import_folder(
    cfg: &Config,
    conn: &Connection,
    src: &Path,
    mode: ImportMode,
) -> Result<ImportResult> {
    import_folder_with_progress(cfg, conn, src, mode, Origin::Manual, &ProgressTracker::new())
}

/// 带进度上报的导入（FR-22：`origin` 由入口显式指定）。
///
/// 流程：先递归收集全部待处理源文件（进度条总数）→ 阶段① 逐个抽取元数据构造候选
/// （NCM 在此完成转换 / 暂存）→ 阶段② 批次结束统一判定并落地 → 逐文件回写进度与结果计数。
pub fn import_folder_with_progress(
    cfg: &Config,
    conn: &Connection,
    src: &Path,
    mode: ImportMode,
    origin: Origin,
    tracker: &ProgressTracker,
) -> Result<ImportResult> {
    import_batch(cfg, conn, collect_sources(src), mode, origin, tracker)
}

/// 递归收集源文件夹里的可播放音频与 .ncm。跳过 `.mt` / trash / 挂起目录（避免把曲库
/// 自身的数据目录当导入源）。排序保证批次内顺序稳定（但判重结果不依赖顺序，见 P4）。
fn collect_sources(dir: &Path) -> Vec<SourceItem> {
    let mut out = Vec::new();
    collect_sources_into(dir, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn collect_sources_into(dir: &Path, out: &mut Vec<SourceItem>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skip = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| matches!(n, ".mt" | "trash" | "pending-duplicates"))
                .unwrap_or(false);
            if !skip {
                collect_sources_into(&path, out);
            }
            continue;
        }
        let kind = if is_playable_audio(&path) {
            SourceKind::Audio
        } else if is_ncm(&path) {
            SourceKind::Ncm
        } else {
            continue;
        };
        out.push(SourceItem {
            fname: fname_of(&path).to_string_lossy().to_string(),
            path,
            kind,
        });
    }
}

/// 从「本地网易云音乐曲库目录」增量合并到 musicMT 曲库（本地操作，不联网、不登录）。
/// 单方向、只加不删：仅把曲库中没有的歌曲复制进来，绝不反向删除原曲库文件
/// （docs/02 FR-12、docs/05 §13）。`origin=netease-sync` 保留同大小静默跳过的快速通道（P2）。
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
    import_folder_with_progress(cfg, conn, dir, ImportMode::Copy, Origin::NeteaseSync, tracker)
}

/// 处理 pendingNcm 积压：把此前因「无工具」暂存的 .ncm 逐个按下载逻辑转换入库。
/// 调用时机：指定 NCM 工具成功之后（初始化向导 / 后续设置，任何时序统一走这里）。
/// `origin=ncm-backfill` 与网易云同步同属周期性批量场景，保留快速通道（P2）。
pub fn process_pending_ncm(cfg: &Config, conn: &Connection) -> ImportResult {
    process_pending_ncm_with_progress(cfg, conn, &ProgressTracker::new())
}

/// 带进度上报的 pendingNcm 回补。整批作为**一个批次**交给 `finalize()`（P4）。
pub fn process_pending_ncm_with_progress(
    cfg: &Config,
    conn: &Connection,
    tracker: &ProgressTracker,
) -> ImportResult {
    let pending_dir = cfg.paths.pending_ncm_dir();
    let Ok(rd) = std::fs::read_dir(&pending_dir) else {
        return ImportResult::default(); // 目录不存在 = 无积压
    };
    let mut items: Vec<SourceItem> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_ncm(p))
        .map(|p| SourceItem {
            fname: fname_of(&p).to_string_lossy().to_string(),
            path: p,
            kind: SourceKind::Ncm,
        })
        .collect();
    items.sort_by(|a, b| a.path.cmp(&b.path));
    match import_batch(cfg, conn, items, ImportMode::Move, Origin::NcmBackfill, tracker) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("pendingNcm 回补失败：{e}");
            ImportResult::default()
        }
    }
}

/// 导入一个 .ncm 文件（单文件批次）：有工具则转换后入库，无工具则暂存 pendingNcm。
pub fn import_ncm(
    cfg: &Config,
    conn: &Connection,
    src: &Path,
    mode: ImportMode,
    origin: Origin,
) -> Result<ImportOutcome> {
    import_ncm_with_progress(cfg, conn, src, mode, origin, &ProgressTracker::new())
}

/// 带进度上报的单文件 ncm 导入（见 [`import_ncm`]）。
pub fn import_ncm_with_progress(
    cfg: &Config,
    conn: &Connection,
    src: &Path,
    mode: ImportMode,
    origin: Origin,
    tracker: &ProgressTracker,
) -> Result<ImportOutcome> {
    let kind = if is_ncm(src) {
        SourceKind::Ncm
    } else {
        SourceKind::Audio
    };
    let items = vec![SourceItem {
        path: src.to_path_buf(),
        fname: fname_of(src).to_string_lossy().to_string(),
        kind,
    }];
    let res = import_batch(cfg, conn, items, mode, origin, tracker)?;
    if res.held > 0 {
        Ok(ImportOutcome::Held)
    } else if res.imported > 0 {
        Ok(ImportOutcome::Imported)
    } else if res.pending > 0 {
        Ok(ImportOutcome::Pending)
    } else if res.skipped > 0 {
        Ok(ImportOutcome::Skipped)
    } else {
        Err(anyhow!("导入失败"))
    }
}

/// 两阶段批次的公共实现：收集 → `duplicates::finalize()` → 进度结算。
fn import_batch(
    cfg: &Config,
    conn: &Connection,
    items: Vec<SourceItem>,
    mode: ImportMode,
    origin: Origin,
    tracker: &ProgressTracker,
) -> Result<ImportResult> {
    let mut res = ImportResult::default();
    tracker.set_total(items.len() as u32);
    let default_action = match mode {
        ImportMode::Copy => ImportAction::Copy,
        ImportMode::Move => ImportAction::Move,
    };
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut prepared: Vec<Prepared> = Vec::new();

    // ===== 阶段①：只收集，不写 DB / 不归位 / 不弹窗 =====
    for item in &items {
        tracker.begin_file(&item.fname);
        match item.kind {
            SourceKind::Audio => {
                tracker.set_action(default_action);
                match duplicates::candidate_from_audio(cfg, &item.path, mode.into(), origin) {
                    Ok(c) => {
                        let index = candidates.len();
                        candidates.push(c);
                        prepared.push(Prepared::Candidate {
                            index,
                            temp: false,
                            fname: item.fname.clone(),
                            action: default_action,
                        });
                    }
                    Err(e) => prepared.push(Prepared::Failed {
                        fname: item.fname.clone(),
                        message: e.to_string(),
                        action: default_action,
                    }),
                }
            }
            SourceKind::Ncm => {
                if !cfg.ncm.tool_ready() {
                    tracker.set_action(ImportAction::Stage);
                    match stage_pending_ncm(cfg, &item.path, mode, &item.fname) {
                        Ok(()) => prepared.push(Prepared::PendingNcm {
                            fname: item.fname.clone(),
                        }),
                        Err(e) => prepared.push(Prepared::Failed {
                            fname: item.fname.clone(),
                            message: e.to_string(),
                            action: ImportAction::Stage,
                        }),
                    }
                } else {
                    tracker.set_action(ImportAction::Convert);
                    match stage_ncm_audio(cfg, &item.path, mode, tracker, &item.fname) {
                        Ok(audio_out) => {
                            let out_name = fname_of(&audio_out).to_string_lossy().to_string();
                            match duplicates::candidate_from_audio(
                                cfg,
                                &audio_out,
                                FileMode::Move,
                                origin,
                            ) {
                                Ok(mut c) => {
                                    // 转换产物是临时文件：挂起时必须搬进 pending 目录，
                                    // 否则它会留在 download/ 被下次扫描重新入库。
                                    c.is_temp = true;
                                    let index = candidates.len();
                                    candidates.push(c);
                                    prepared.push(Prepared::Candidate {
                                        index,
                                        temp: true,
                                        fname: out_name,
                                        action: ImportAction::Move,
                                    });
                                }
                                Err(e) => {
                                    let _ = std::fs::remove_file(&audio_out);
                                    prepared.push(Prepared::Failed {
                                        fname: item.fname.clone(),
                                        message: e.to_string(),
                                        action: ImportAction::Convert,
                                    });
                                }
                            }
                        }
                        Err(e) => prepared.push(Prepared::Failed {
                            fname: item.fname.clone(),
                            message: e.to_string(),
                            action: ImportAction::Convert,
                        }),
                    }
                }
            }
        }
    }

    // ===== 阶段②：批次结束统一判定并落地 =====
    let outcome = duplicates::finalize(cfg, conn, &candidates)?;
    for p in prepared {
        match p {
            Prepared::Candidate {
                index,
                temp,
                fname,
                action,
            } => {
                let status = match &outcome.dispositions[index] {
                    Disposition::Imported | Disposition::Updated => {
                        res.imported += 1;
                        tracker.log(action, &fname, EntryStatus::Imported, "");
                        EntryStatus::Imported
                    }
                    Disposition::Skipped => {
                        res.skipped += 1;
                        cleanup_temp(&candidates[index], temp);
                        tracker.log(ImportAction::Skip, &fname, EntryStatus::Skipped, "曲库中已存在");
                        EntryStatus::Skipped
                    }
                    Disposition::Held => {
                        res.held += 1;
                        tracker.log(
                            ImportAction::Stage,
                            &fname,
                            EntryStatus::Pending,
                            "重复文件已挂起，等待对比决策",
                        );
                        EntryStatus::Pending
                    }
                    Disposition::Failed(msg) => {
                        res.failed += 1;
                        cleanup_temp(&candidates[index], temp);
                        tracker.log(action, &fname, EntryStatus::Failed, msg);
                        EntryStatus::Failed
                    }
                };
                tracker.end_file(status);
            }
            Prepared::PendingNcm { fname } => {
                res.pending += 1;
                tracker.log(
                    ImportAction::Stage,
                    &fname,
                    EntryStatus::Pending,
                    "无 NCM 工具，暂存待转换",
                );
                tracker.end_file(EntryStatus::Pending);
            }
            Prepared::Failed {
                fname,
                message,
                action,
            } => {
                res.failed += 1;
                eprintln!("导入失败 {fname}: {message}");
                tracker.log(action, &fname, EntryStatus::Failed, &message);
                tracker.end_file(EntryStatus::Failed);
            }
        }
    }
    Ok(res)
}

/// 跳过的转换产物（或抽取失败时的残留）清掉，避免 download/ 里堆垃圾。
fn cleanup_temp(c: &Candidate, temp: bool) {
    if !temp {
        return;
    }
    let _ = std::fs::remove_file(&c.source);
    if let Some(lrc) = &c.source_lrc {
        let _ = std::fs::remove_file(lrc);
    }
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

/// 有工具：把 .ncm 搬到 download/ 暂存 → 调外部工具解密 → 原始 .ncm 归档 download/ncm/，
/// 返回解出的音频路径（**不入库**，交给批次统一判定）。
fn stage_ncm_audio(
    cfg: &Config,
    src: &Path,
    mode: ImportMode,
    tracker: &ProgressTracker,
    fname: &str,
) -> Result<PathBuf> {
    let stage = cfg.paths.download_dir.join(fname);
    if let Some(p) = stage.parent() {
        std::fs::create_dir_all(p)?;
    }
    let _ = std::fs::remove_file(&stage);
    // 同名 .lrc 一并搬运到 download/（若源 .ncm 旁有）：转换产出的音频与它同目录同 stem。
    let sidecar = lyrics::find_sidecar(src);
    copy_or_move(mode, src, &stage)?;
    if let Some(lrc) = &sidecar {
        let _ = std::fs::copy(lrc, &stage.with_extension("lrc"));
    }
    let audio_out = convert_ncm(cfg, &stage)?;
    tracker.log(ImportAction::Convert, fname, EntryStatus::Imported, "转换完成");
    let ncm_archive = cfg.paths.ncm_raw_dir.join(fname);
    if let Some(p) = ncm_archive.parent() {
        std::fs::create_dir_all(p)?;
    }
    let _ = std::fs::remove_file(&ncm_archive);
    std::fs::rename(&stage, &ncm_archive)?;
    Ok(audio_out)
}

/// 无工具：原 .ncm 暂存到 download/pendingNcm/（不入库，待后续配置工具转换）。
/// 已在该目录内（例如 pendingNcm 回补重复处理）则直接算 Pending，避免重命名到自身报错。
fn stage_pending_ncm(
    cfg: &Config,
    src: &Path,
    mode: ImportMode,
    fname: &str,
) -> Result<()> {
    let pending_dir = cfg.paths.pending_ncm_dir();
    if src.starts_with(&pending_dir) {
        return Ok(());
    }
    std::fs::create_dir_all(&pending_dir)?;
    let pending = pending_dir.join(fname);
    let _ = std::fs::remove_file(&pending);
    copy_or_move(mode, src, &pending)?;
    Ok(())
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
