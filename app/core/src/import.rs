//! 歌曲导入：从指定文件夹复制 / 移动到曲库，自动按 `config.library.file_scheme` 组织并入库。
//! - 普通音频：抽取标签 → 按 file_scheme 归位 → 登记（song_id=NULL，与扫描一致）。
//! - ncm 文件（有工具）：转换前按 `(曲源 id, 原始 .ncm 文件名)` 短路判重，转换后按本地文件入库。
//! - ncm 文件（无工具）：v0.14.0 起直接跳过、留在源目录（不复制不暂存），下次更新曲源重扫再处理。
//!
//! FR-22：一次导入任务 = 一个**两阶段批次**（W3-A′）。阶段① 只收集候选（不写 DB、不归位、
//! 不弹窗），阶段② 由 `duplicates::finalize()` 统一判定「入库 / 静默跳过 / 挂起成重复组」。
//! `origin` 必须沿调用链**显式传递**：网易云同步与手工导入共用同一个 `import_folder_with_progress()`，
//! 仅靠 ImportMode 或源目录无法区分来源（P2 按来源分流的实施硬约束）。
use crate::config::Config;
use crate::db;
use crate::duplicates::{self, Candidate, Disposition, FileMode, Origin};
use crate::library::convert_ncm;
use crate::lyrics;
use crate::progress::{EntryStatus, ImportAction, ProgressTracker};
use crate::Result;
use crate::is_playable_audio;
use anyhow::anyhow;
use rusqlite::Connection;
use serde::Serialize;
use std::cell::RefCell;
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
    /// v0.14.0：skipped 中因未配置 NCM 工具而跳过的加密文件数（留在源目录，不复制不暂存）。
    pub ncm_skipped: u32,
    pub failed: u32,
    /// FR-22：登记为「重复待决策」的文件数（挂起进 `.mt/pending-duplicates*`，等 GUI 批量对比）。
    pub held: u32,
    /// 歌曲筛选设置过滤掉的文件数（大小 / 时长不达标）。
    pub filtered: u32,
    /// 同源质量等价的自身重复，自动选优后静默跳过的数量。
    pub intra_source_skipped: u32,
    /// 新文件质量明确更好，已自动替换库内旧版本的数量。
    pub upgraded: u32,
    /// 新文件质量明确更差，静默忽略的数量。
    pub lower_quality_ignored: u32,
    /// v0.17.7：曲源重导命中不可访问记录、原位恢复（清标）的数量。
    pub restored: u32,
}

/// 导入单文件的结果（供 serve / CLI 计数）。
pub enum ImportOutcome {
    Imported,
    Skipped,
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

// 只有「没有进入 candidates」的源文件需要阶段②单独结算进度；
// 成功构造成候选的曲目一律由 finalize_with_hook 的钩子逐首结算。
enum Prepared {
    /// v0.14.0：源文件未进入候选而直接跳过（无 NCM 工具 / 已转换过）。
    /// 需要在阶段②单独结算 skipped 并 end_file，否则进度分子不收敛（v0.12.18 修复口径）。
    Skipped {
        fname: String,
        action: ImportAction,
        reason: String,
    },
    /// 阶段①被歌曲筛选阈值拦下：不进候选，也不参与进度二次结算。
    Filtered {
        fname: String,
        action: ImportAction,
        reason: String,
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
    import_batch(cfg, conn, collect_sources(src), mode, origin, None, tracker)
}

/// 带曲源归属的导入：`source_id` 写入每首候选，最终落到 `download_records.source_kind`。
/// 手工选择文件夹 / 扫描 / NCM 回补走 `import_folder_with_progress`（传 None）。
pub fn import_folder_with_progress_from_source(
    cfg: &Config,
    conn: &Connection,
    src: &Path,
    mode: ImportMode,
    origin: Origin,
    source_id: Option<&str>,
    tracker: &ProgressTracker,
) -> Result<ImportResult> {
    import_batch(
        cfg,
        conn,
        collect_sources(src),
        mode,
        origin,
        source_id,
        tracker,
    )
}

/// 「持续监听」只读扫描：统计曲源目录内仍可导入的音频文件数。
/// 零新增线程、不写库、不复制、不联网；结果仅用于「发现 N 首新歌」提示，
/// 实际入库仍须用户确认后走既有判重链路。
/// v0.14.0：`.ncm` 不计入 found（无工具时只提示配置工具，不进入「新增歌曲」口径）。
pub fn count_source_audio_files(dir: &Path) -> u32 {
    count_source_audio_and_ncm(dir).0
}

/// 同一次只读扫描分别返回（可导入音频数, `.ncm` 文件数）。
/// `.ncm` 单列供前端提示「待配置 NCM 工具」，永不并入音频数、也不计入 check 的 total。
pub fn count_source_audio_and_ncm(dir: &Path) -> (u32, u32) {
    let items = collect_sources(dir);
    let audio = items.iter().filter(|i| matches!(i.kind, SourceKind::Audio)).count() as u32;
    let ncm = items.iter().filter(|i| matches!(i.kind, SourceKind::Ncm)).count() as u32;
    (audio, ncm)
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

/// 探测遍历上限：只读、有短路，避免超大目录拖慢 UI。
const PROBE_MAX_ENTRIES: usize = 2000;
const PROBE_MAX_DEPTH: usize = 3;

/// 来源目录的只读探测结果（自动判定网易云曲库 + 条件显示 NCM 引导）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct SourceProbe {
    /// 是否按网易云曲库处理：仅由 `.ncm` 探测信号得出，界面可手动勾选覆盖。
    pub is_netease: bool,
    /// 目录（含有限深度子目录）里是否存在 `.ncm` 文件。
    pub ncm_found: bool,
    /// 建议的来源名称（默认取目录名）。
    pub suggested_name: String,
}

/// 探测时跳过的目录：不把曲库自身的数据目录当成来源特征。
fn probe_skip_dir(name: &str) -> bool {
    // v0.17.3：中间态收敛到 .mt/convert（被 ".mt" 规则覆盖）；旧的 download / pendingncm /
    // ncm_raw 目录不再产生，但保留在跳过清单以兼容探测仍含旧目录的曲源。
    matches!(name, ".mt" | "trash" | "pendingncm" | "download" | "tools")
        || name.starts_with('.')
}

/// 只读探测来源目录（启发式，不联网、不写盘、不解析音频）。
///
/// 判定依据：**仅**目录内出现 `.ncm` 文件才视为网易云强信号；目录名（哪怕含
/// `cloudmusic` / `netease` / `网易云`）一律不作为判据，杜绝按名字误判。
/// 用户仍可在界面手动勾选 / 取消 `is_netease` 覆盖探测结果；不联网、不写盘。
/// 遍历有上限（约 2000 条目、3 层），命中 .ncm 后继续统计，不再按名字短路。
pub fn probe_source(dir: &Path) -> SourceProbe {
    let mut probe = SourceProbe {
        suggested_name: dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("新来源")
            .to_string(),
        ..Default::default()
    };
    if !dir.is_dir() {
        return probe;
    }

    let mut queue: std::collections::VecDeque<(PathBuf, usize)> = std::collections::VecDeque::new();
    queue.push_back((dir.to_path_buf(), 0));
    let mut visited = 0usize;
    while let Some((cur, depth)) = queue.pop_front() {
        let Ok(rd) = std::fs::read_dir(&cur) else {
            continue;
        };
        for entry in rd.flatten() {
            visited += 1;
            if visited >= PROBE_MAX_ENTRIES {
                probe.is_netease = probe.ncm_found;
                return probe;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_lowercase();
            if lower.ends_with(".ncm") {
                probe.ncm_found = true;
            }
            let p = entry.path();
            if p.is_dir() && depth < PROBE_MAX_DEPTH && !probe_skip_dir(&lower) {
                queue.push_back((p, depth + 1));
            }
        }
    }
    probe.is_netease = probe.ncm_found;
    probe
}

/// 按来源执行导入：网易云来源沿用原逻辑（只加不删、强制复制、`Origin::NeteaseSync`），
/// 普通来源按用户所选 `Copy` / `Move`（`Origin::Manual`）。
pub fn import_source_with_progress(
    cfg: &Config,
    conn: &Connection,
    source_dir: &Path,
    is_netease: bool,
    mode: ImportMode,
    source_id: &str,
    tracker: &ProgressTracker,
) -> Result<ImportResult> {
    if !source_dir.is_dir() {
        anyhow::bail!("来源目录不存在或不可访问：{}", source_dir.display());
    }
    if is_netease {
        return import_folder_with_progress_from_source(
            cfg,
            conn,
            source_dir,
            ImportMode::Copy,
            Origin::NeteaseSync,
            Some(source_id),
            tracker,
        );
    }
    import_folder_with_progress_from_source(
        cfg,
        conn,
        source_dir,
        mode,
        Origin::Manual,
        Some(source_id),
        tracker,
    )
}

/// 导入一个 .ncm 文件（单文件批次）：有工具则转换后入库、原件保留；无工具则跳过留源目录。
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
    let res = import_batch(cfg, conn, items, mode, origin, None, tracker)?;
    if res.held > 0 {
        Ok(ImportOutcome::Held)
    } else if res.imported > 0 {
        Ok(ImportOutcome::Imported)
    } else if res.skipped > 0 {
        Ok(ImportOutcome::Skipped)
    } else {
        Err(anyhow!("导入失败"))
    }
}

/// 与 `candidates` 下标对齐的显示信息，供批次钩子实时上报进度。
struct CandidateMeta {
    fname: String,
    action: ImportAction,
    temp: bool,
}

/// 把 `finalize_with_hook` 的逐首回调接到 `ProgressTracker`：
/// 每首开始归位时刷新「当前文件/动作」，结果落定时立刻写日志并结算计数，
/// 从而消除「阶段②全部复制完才批量结算」造成的 0/N 假死。
struct ImportHook<'a> {
    tracker: &'a ProgressTracker,
    /// 写连接：落库后按 target 路径回读完整曲目，作为列表增量明细推给前端。
    conn: &'a Connection,
    meta: Vec<CandidateMeta>,
    res: RefCell<ImportResult>,
}

impl<'a> ImportHook<'a> {
    /// 曲目已落库后回读完整 TrackView 推入进度快照，供前端分批刷新歌曲列表。
    /// 回读失败仅静默跳过本条明细（不影响导入结果本身）。
    fn push_imported_track(&self, c: &Candidate) {
        if let Ok(Some(view)) = db::get_track_by_path(self.conn, &c.target) {
            self.tracker.push_import(view);
        }
    }
}

impl duplicates::BatchHook for ImportHook<'_> {
    fn on_begin(&self, index: usize, _c: &Candidate) {
        // 每首归位前检查暂停：阶段②复制大文件同样可耗时，需能被暂停挂起。
        self.tracker.wait_if_paused();
        let Some(m) = self.meta.get(index) else { return };
        self.tracker.begin_file(&m.fname);
        self.tracker.set_action(m.action);
    }

    fn on_settled(&self, index: usize, c: &Candidate, disposition: &Disposition) {
        let Some(m) = self.meta.get(index) else { return };
        let mut res = self.res.borrow_mut();
        let status = match disposition {
            Disposition::Imported | Disposition::Updated => {
                res.imported += 1;
                self.push_imported_track(c);
                self.tracker.log(m.action, &m.fname, EntryStatus::Imported, "");
                EntryStatus::Imported
            }
            Disposition::Skipped => {
                res.skipped += 1;
                cleanup_temp(c, m.temp);
                let msg = if self.tracker.is_cancelled() {
                    "已停止，未处理"
                } else {
                    "曲库中已存在"
                };
                self.tracker
                    .log(ImportAction::Skip, &m.fname, EntryStatus::Skipped, msg);
                EntryStatus::Skipped
            }
            Disposition::IntraSourceSkipped => {
                res.intra_source_skipped += 1;
                cleanup_temp(c, m.temp);
                self.tracker.log(
                    ImportAction::Skip,
                    &m.fname,
                    EntryStatus::Skipped,
                    "同源质量等价，已自动选优",
                );
                EntryStatus::Skipped
            }
            Disposition::Upgraded => {
                res.upgraded += 1;
                self.push_imported_track(c);
                self.tracker.log(
                    m.action,
                    &m.fname,
                    EntryStatus::Imported,
                    "质量更好，已自动替换旧版本",
                );
                EntryStatus::Imported
            }
            Disposition::LowerQualityIgnored => {
                res.lower_quality_ignored += 1;
                cleanup_temp(c, m.temp);
                self.tracker.log(
                    ImportAction::Skip,
                    &m.fname,
                    EntryStatus::Skipped,
                    "质量低于库内版本，已静默忽略",
                );
                EntryStatus::Skipped
            }
            Disposition::Restored => {
                res.restored += 1;
                self.push_imported_track(c);
                self.tracker.log(
                    m.action,
                    &m.fname,
                    EntryStatus::Imported,
                    "曲源重导：已恢复不可访问曲目",
                );
                EntryStatus::Imported
            }
            Disposition::Held => {
                res.held += 1;
                self.tracker.log(
                    ImportAction::Stage,
                    &m.fname,
                    EntryStatus::Held,
                    "重复文件已挂起，等待对比决策",
                );
                EntryStatus::Held
            }
            Disposition::Failed(msg) => {
                res.failed += 1;
                cleanup_temp(c, m.temp);
                self.tracker
                    .log(m.action, &m.fname, EntryStatus::Failed, msg);
                EntryStatus::Failed
            }
        };
        self.tracker.end_file(status);
    }

    fn is_cancelled(&self) -> bool {
        self.tracker.is_cancelled()
    }
}

/// 歌曲筛选未通过时给出可读原因；通过返回 None。
fn filter_reason(cfg: &Config, c: &Candidate) -> Option<String> {
    if cfg.import.passes_filter(c.size, c.duration_ms) {
        return None;
    }
    let mut reasons: Vec<String> = Vec::new();
    if cfg.import.min_size_kb > 0 && c.size < cfg.import.min_size_kb as i64 * 1024 {
        reasons.push(format!("小于 {}KB", cfg.import.min_size_kb));
    }
    if cfg.import.min_duration_sec > 0 {
        match c.duration_ms {
            None => reasons.push("时长未知".to_string()),
            Some(ms) if ms / 1000 < cfg.import.min_duration_sec as i64 => {
                reasons.push(format!("短于 {} 秒", cfg.import.min_duration_sec));
            }
            _ => {}
        }
    }
    Some(format!("歌曲筛选未通过：{}", reasons.join("，")))
}

/// 把单首 finalize 的局部结果合并到总结果（流式逐首累加）。
fn merge_result(dst: &mut ImportResult, s: ImportResult) {
    dst.imported += s.imported;
    dst.skipped += s.skipped;
    dst.ncm_skipped += s.ncm_skipped;
    dst.failed += s.failed;
    dst.held += s.held;
    dst.filtered += s.filtered;
    dst.intra_source_skipped += s.intra_source_skipped;
    dst.upgraded += s.upgraded;
    dst.lower_quality_ignored += s.lower_quality_ignored;
    dst.restored += s.restored;
}

/// v0.17.4 流式批次的公共实现：逐首「收集 → 判重 → 归位 → 结算」，一首落定再处理下一首。
/// - 每个源文件恰好 `advance()` 一次（进度分子），归位结果由单首 hook 的 `end_file` 结算；
/// - 库内命中且质量并非更好 → 复用 FR-22 `stage_duplicate_of` 挂进 `.mt/pending-duplicates/`
///   （后到重复不丢、不锁死低质量版本，交 GUI 对比决策）；
/// - 库内未命中或质量更好 → 单首 `finalize_with_hook` 直接入库 / 自动替换。
/// 这样 `new_imports` 在整个导入过程中持续推送，前端歌曲列表实时增长。
fn import_batch(
    cfg: &Config,
    conn: &Connection,
    items: Vec<SourceItem>,
    mode: ImportMode,
    origin: Origin,
    source_id: Option<&str>,
    tracker: &ProgressTracker,
) -> Result<ImportResult> {
    tracker.set_total(items.len() as u32);
    let default_action = match mode {
        ImportMode::Copy => ImportAction::Copy,
        ImportMode::Move => ImportAction::Move,
    };
    let mut res = ImportResult::default();

    for item in &items {
        tracker.wait_if_paused();
        tracker.begin_file(&item.fname);

        // 构造本首候选（Audio / Ncm 两条路径），失败或无工具走就地结算。
        let mut candidate: Option<(Candidate, CandidateMeta)> = None;
        let mut prepared: Option<Prepared> = None;
        match item.kind {
            SourceKind::Audio => {
                tracker.set_action(default_action);
                match duplicates::candidate_from_audio(cfg, &item.path, mode.into(), origin) {
                    Ok(mut c) => {
                        if let Some(reason) = filter_reason(cfg, &c) {
                            prepared = Some(Prepared::Filtered {
                                fname: item.fname.clone(),
                                action: default_action,
                                reason,
                            });
                        } else {
                            c.source_id = source_id.map(|s| s.to_string());
                            candidate = Some((c, CandidateMeta {
                                fname: item.fname.clone(),
                                action: default_action,
                                temp: false,
                            }));
                        }
                    }
                    Err(e) => prepared = Some(Prepared::Failed {
                        fname: item.fname.clone(),
                        message: e.to_string(),
                        action: default_action,
                    }),
                }
            }
            SourceKind::Ncm => {
                if !cfg.ncm.tool_ready() {
                    tracker.set_action(ImportAction::Skip);
                    prepared = Some(Prepared::Skipped {
                        fname: item.fname.clone(),
                        action: ImportAction::Skip,
                        reason: "未配置 NCM 工具，跳过加密文件（留在源目录）".to_string(),
                    });
                } else {
                    tracker.set_action(ImportAction::Convert);
                    // v0.17.5：无曲源 id 的入口（散落整理 / 手工导入 / import-netease）用
                    // .ncm 所在目录路径作短路键，与曲源 id 的键空间自然隔离；同目录同名
                    // 同 size 才算已转换。源 size 取不到时不短路（宁可多转不丢歌）。
                    let dedup_id = source_id.map(|s| s.to_string()).unwrap_or_else(|| {
                        item.path.parent()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default()
                    });
                    let ncm_size = std::fs::metadata(&item.path).ok().map(|m| m.len() as i64);
                    // v0.17.8：短路改查独立缓存表 ncm_processed —— 不再依赖 download_records
                    // 的 ncm_origin 三列（旧库为 NULL，且判重 Skip 路径不会写那三列 → 死循环）。
                    let already = ncm_size.and_then(|sz| {
                        match db::ncm_processed_exists(conn, &dedup_id, &item.fname, sz) {
                            Ok(v) => Some(v),
                            Err(e) => {
                                eprintln!("NCM 转换判重查询失败（降级为不跳过）：{e}");
                                Some(false)
                            }
                        }
                    }).unwrap_or(false);
                    if already {
                        prepared = Some(Prepared::Skipped {
                            fname: item.fname.clone(),
                            action: ImportAction::Skip,
                            reason: "该 .ncm 已转换入库（同名同大小），跳过".to_string(),
                        });
                    } else {
                        match stage_ncm_audio(cfg, &item.path, tracker, &item.fname) {
                            Ok(audio_out) => {
                                // 历史库该 (id,name) 的 size 为 NULL → 首次转换后回填（溯源用）。
                                if let Some(sz) = ncm_size {
                                    if let Err(e) = db::backfill_ncm_original_size(
                                        conn, &dedup_id, &item.fname, sz,
                                    ) {
                                        eprintln!("NCM 历史 size 回填失败（不影响本次入库）：{e}");
                                    }
                                    // v0.17.8：无条件写短路缓存 —— 关键！判重 Skip（曲库中已存在）
                                    // 不入库、不写 ncm_origin，但转换已发生，必须记一笔，
                                    // 否则下次同版本 .ncm 还会重新转换（283 转换/0 入库的死循环）。
                                    if let Err(e) = db::mark_ncm_processed(
                                        conn, &dedup_id, &item.fname, sz,
                                    ) {
                                        eprintln!("NCM 短路缓存写入失败（不影响本次入库）：{e}");
                                    }
                                }
                                let out_name = fname_of(&audio_out).to_string_lossy().to_string();
                                match duplicates::candidate_from_audio(
                                    cfg, &audio_out, FileMode::Move, origin,
                                ) {
                                    Ok(mut c) => {
                                        c.is_temp = true;
                                        c.source_id = source_id.map(|s| s.to_string());
                                        c.ncm_origin = Some((dedup_id.clone(), item.fname.clone()));
                                        c.ncm_source_size = ncm_size;
                                        if let Some(reason) = filter_reason(cfg, &c) {
                                            cleanup_temp(&c, true);
                                            prepared = Some(Prepared::Filtered {
                                                fname: item.fname.clone(),
                                                action: ImportAction::Convert,
                                                reason,
                                            });
                                        } else {
                                            candidate = Some((c, CandidateMeta {
                                                fname: out_name,
                                                action: ImportAction::Move,
                                                temp: true,
                                            }));
                                        }
                                    }
                                    Err(e) => {
                                        let _ = std::fs::remove_file(&audio_out);
                                        prepared = Some(Prepared::Failed {
                                            fname: item.fname.clone(),
                                            message: e.to_string(),
                                            action: ImportAction::Convert,
                                        });
                                    }
                                }
                            }
                            Err(e) => prepared = Some(Prepared::Failed {
                                fname: item.fname.clone(),
                                message: e.to_string(),
                                action: ImportAction::Convert,
                            }),
                        }
                    }
                }
            }
        }

        // 本首归位：流式单首 finalize，**完全复用现有整批判重逻辑**（不另造轮子）。
        // finalize_with_hook 单候选时的口径（duplicates.rs）：
        //   未命中          → Import
        //   命中+质量更好    → Better 自动替换（upgraded）
        //   命中+质量更差    → LowerQualityIgnored 静默忽略（不进 pending）
        //   命中+质量等价    → Skip（等价不进 pending，用户已确认）
        //   命中+质量无法判定 → Hold → register_group 挂进 .mt/pending-duplicates/ 待对比
        // 这样既满足「更好的自动入库、等价不进 pending」，又保持既有判重语义与计数口径。
        if let Some((c, meta)) = candidate {
            let hook = ImportHook {
                tracker,
                conn,
                meta: vec![meta],
                res: RefCell::new(ImportResult::default()),
            };
            duplicates::finalize_with_hook(cfg, conn, std::slice::from_ref(&c), Some(&hook))?;
            merge_result(&mut res, hook.res.into_inner());
        } else if let Some(p) = prepared {
            match p {
                Prepared::Skipped { fname, action, reason } => {
                    res.skipped += 1;
                    if reason.starts_with("未配置 NCM 工具") {
                        res.ncm_skipped += 1;
                    }
                    tracker.log(action, &fname, EntryStatus::Skipped, &reason);
                    tracker.end_file(EntryStatus::Skipped);
                }
                Prepared::Filtered { fname, action, reason } => {
                    res.filtered += 1;
                    tracker.log(action, &fname, EntryStatus::Skipped, &reason);
                    tracker.end_file(EntryStatus::Skipped);
                }
                Prepared::Failed { fname, message, action } => {
                    res.failed += 1;
                    eprintln!("导入失败 {fname}: {message}");
                    tracker.log(action, &fname, EntryStatus::Failed, &message);
                    tracker.end_file(EntryStatus::Failed);
                }
            }
        }

        // 本首（含候选/无候选）处理完毕 → 进度分子 +1。
        tracker.advance();
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

/// 有工具：把 .ncm **复制**一份到 `<曲库>/.mt/convert/` 中转 → 调外部工具解密 →
/// 清理中转副本，返回解出的音频路径（**不入库**，交给批次统一判定）。
/// v0.17.3：原始 .ncm 一律不删不挪（Copy 模式，无论用户选复制/移动），留源曲源；
/// 不再归档原始文件。同名 .lrc 只复制、不移动源文件。
fn stage_ncm_audio(
    cfg: &Config,
    src: &Path,
    tracker: &ProgressTracker,
    fname: &str,
) -> Result<PathBuf> {
    let stage = cfg.paths.convert_dir.join(fname);
    if let Some(p) = stage.parent() {
        std::fs::create_dir_all(p)?;
    }
    let _ = std::fs::remove_file(&stage);
    // 同名 .lrc 一并复制到中转目录（若源 .ncm 旁有）：转换产出的音频与它同目录同 stem。
    let sidecar = lyrics::find_sidecar(src);
    std::fs::copy(src, &stage)?;
    if let Some(lrc) = &sidecar {
        let _ = std::fs::copy(lrc, &stage.with_extension("lrc"));
    }
    let audio_out = convert_ncm(cfg, &stage)?;
    // v0.17.4：阶段①转换成功 ≠ 已入库；用 Converted 状态避免前端误显示「已入库」。
    tracker.log(ImportAction::Convert, fname, EntryStatus::Converted, "转换完成");
    // 只清理中转副本（含 sidecar 副本），原始 .ncm/.lrc 留在源曲源。
    let _ = std::fs::remove_file(&stage);
    if sidecar.is_some() {
        let _ = std::fs::remove_file(stage.with_extension("lrc"));
    }
    Ok(audio_out)
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

    /// 统计曲库根下、排除 .mt（含 convert 中转）的音频文件数。
    fn count_library_audio(root: &Path) -> usize {
        let staging = root.join(".mt");
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
    fn import_ncm_no_tool_skips_and_leaves_in_source() {
        let root = std::env::temp_dir().join(format!("mt_imp_{}_a", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let mut cfg = config::build_library_config(&root, "artist_album", "");
        cfg.import.min_size_kb = 0;
        cfg.import.min_duration_sec = 0;
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
        assert_eq!(res.skipped, 1, "无工具的 .ncm 应计为跳过");
        assert_eq!(res.ncm_skipped, 1, "应精确统计跳过的加密文件");
        assert_eq!(res.failed, 0);
        // 曲库正式位置有 1 个音频（普通 wav 已归位）。
        assert_eq!(count_library_audio(&root), 1, "曲库应有 1 个音频");
        // v0.17.3：.ncm 一律留在源目录，不写入任何中转/归档目录。
        assert!(ncm.is_file(), "无工具的 .ncm 应留在源目录");
        assert_eq!(
            std::fs::read_dir(&cfg.paths.convert_dir).unwrap().count(),
            0,
            "无工具不应写入 convert 中转目录"
        );

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_ncm_with_tool_converts_and_keeps_original() {
        let root = std::env::temp_dir().join(format!("mt_imp_{}_b", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let mut cfg = config::build_library_config(&root, "artist_album", "");
        cfg.import.min_size_kb = 0;
        cfg.import.min_duration_sec = 0;
        cfg.ensure_dirs().unwrap();
        let conn = db::open(&cfg).unwrap();

        // 准备假 ncmdump：把 sample.wav 复制到与输入同目录、同名（换扩展名 .wav）。
        // sample 与源文件夹都放在 root 之外，避免被曲库音频计数误计。
        let sample = std::env::temp_dir().join(format!("mt_imp_smp_{}.wav", std::process::id()));
        write_valid_wav(&sample);
        let tool_path = root.join("tools").join("fake_ncmdump.cmd");
        std::fs::create_dir_all(tool_path.parent().unwrap()).unwrap();
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
        assert_eq!(res.failed, 0);
        // 转换产物已按结构归位（全树计数，假样本无标签时艺人取父目录）。
        assert_eq!(count_audio_named(&root, "trackC"), 1, "应存在 1 个转换音频");
        // v0.17.3：原始 ncm 一律留在源曲源，不删不挪、不归档。
        assert!(ncm.is_file(), "原始 .ncm 应留在源目录");
        // convert 中转目录转换后应清空（临时副本已清理）。
        let left_in_convert = std::fs::read_dir(&cfg.paths.convert_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains("trackC"));
        assert!(!left_in_convert, "convert/ 不应残留 trackC 临时文件");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn count_source_audio_excludes_ncm_from_found() {
        let src = std::env::temp_dir().join(format!("mt_imp_src_{}_count", std::process::id()));
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&src).unwrap();
        write_valid_wav(&src.join("a.wav"));
        std::fs::write(src.join("b.ncm"), b"FAKE_NCM_BYTES").unwrap();

        let (found, ncm) = count_source_audio_and_ncm(&src);
        assert_eq!(found, 1, ".ncm 不得计入可导入音频");
        assert_eq!(ncm, 1, ".ncm 应单列计数");
        assert_eq!(count_source_audio_files(&src), 1);
        let _ = std::fs::remove_dir_all(&src);
    }

    #[test]
    fn converted_ncm_is_short_circuited_on_next_source_update() {
        let root = std::env::temp_dir().join(format!("mt_imp_{}_dedup", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut cfg = config::build_library_config(&root, "artist_album", "");
        cfg.import.min_size_kb = 0;
        cfg.import.min_duration_sec = 0;
        cfg.ensure_dirs().unwrap();
        let conn = db::open(&cfg).unwrap();

        let sample = std::env::temp_dir().join(format!("mt_imp_smp_{}_dedup.wav", std::process::id()));
        write_valid_wav(&sample);
        let tool_path = root.join("tools").join("fake_ncmdump.cmd");
        std::fs::create_dir_all(tool_path.parent().unwrap()).unwrap();
        std::fs::write(
            &tool_path,
            format!("@echo off\r\ncopy /Y \"{}\" \"%~dpn1.wav\" >nul\r\n", sample.display()),
        )
        .unwrap();
        cfg.ncm.tool_path = tool_path;
        cfg.ncm.command_template = r#"cmd /C "{tool}" "{input}""#.into();

        let src = std::env::temp_dir().join(format!("mt_imp_src_{}_dedup", std::process::id()));
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&src).unwrap();
        let ncm_bytes = b"FAKE_NCM_BYTES_D";
        std::fs::write(src.join("trackD.ncm"), ncm_bytes).unwrap();

        // 网易云来源强制 Copy：源 .ncm 保留，转换产物入库并记 ncm_origin + size。
        let first = import_source_with_progress(
            &cfg, &conn, &src, true, ImportMode::Copy, "src-dedup", &ProgressTracker::new(),
        )
        .unwrap();
        assert_eq!(first.imported, 1);
        assert!(src.join("trackD.ncm").is_file(), "Copy 模式源 ncm 应保留");
        // v0.17.8：短路缓存（独立于 ncm_origin 列）在转换后即写入。
        assert!(db::ncm_processed_exists(
            &conn, "src-dedup", "trackD.ncm", ncm_bytes.len() as i64,
        ).unwrap());

        // 第二次更新同一曲源：同名同 size 命中短路，不再调外部工具解码。
        let second = import_source_with_progress(
            &cfg, &conn, &src, true, ImportMode::Copy, "src-dedup", &ProgressTracker::new(),
        )
        .unwrap();
        assert_eq!(second.imported, 0, "已转换 ncm 不应重复入库");
        assert_eq!(second.skipped, 1);
        assert_eq!(second.ncm_skipped, 0, "已转换短路不是未配置工具跳过");

        // 同名 .ncm 被更高清版本覆盖（size 变大）：不得短路，重新转换入库。
        let hq_bytes = b"FAKE_NCM_BYTES_D_HIGHER_QUALITY_MUCH_LARGER";
        assert!(hq_bytes.len() > ncm_bytes.len());
        std::fs::write(src.join("trackD.ncm"), hq_bytes).unwrap();
        assert!(!db::ncm_processed_exists(
            &conn, "src-dedup", "trackD.ncm", hq_bytes.len() as i64,
        ).unwrap(), "高清版 size 不同，不应命中短路");
        let third = import_source_with_progress(
            &cfg, &conn, &src, true, ImportMode::Copy, "src-dedup", &ProgressTracker::new(),
        )
        .unwrap();
        // 假样本转换出的音频与库内等价 → 判重 Skipped（不应重复入库）；
        // 但转换已实际发生，size 必须回填，使下次同版本扫描直接短路。
        assert_eq!(third.imported, 0, "等价音频不应重复入库");
        // 关键守护：判重 Skip（未入库）也必须写缓存 —— 否则下次还会重复转换（旧死循环）。
        assert!(db::ncm_processed_exists(
            &conn, "src-dedup", "trackD.ncm", hq_bytes.len() as i64,
        ).unwrap(), "高清版判 Skip 后仍须写缓存，成为新短路键");
        // 第四次：同高清版再扫描 → 短路，无转换。
        let fourth = import_source_with_progress(
            &cfg, &conn, &src, true, ImportMode::Copy, "src-dedup", &ProgressTracker::new(),
        )
        .unwrap();
        assert_eq!(fourth.skipped, 1, "同版本高清 ncm 应命中短路");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_file(&sample);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ncm_short_circuit_works_without_source_id_by_parent_dir() {
        // v0.17.5：散落整理 / 手工导入（source_id=None）用父目录路径作短路键。
        let root = std::env::temp_dir().join(format!("mt_imp_{}_nosid", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut cfg = config::build_library_config(&root, "artist_album", "");
        cfg.import.min_size_kb = 0;
        cfg.import.min_duration_sec = 0;
        cfg.ensure_dirs().unwrap();
        let conn = db::open(&cfg).unwrap();

        let sample = std::env::temp_dir().join(format!("mt_imp_smp_{}_nosid.wav", std::process::id()));
        write_valid_wav(&sample);
        let tool_path = root.join("tools").join("fake_ncmdump.cmd");
        std::fs::create_dir_all(tool_path.parent().unwrap()).unwrap();
        std::fs::write(
            &tool_path,
            format!("@echo off\r\ncopy /Y \"{}\" \"%~dpn1.wav\" >nul\r\n", sample.display()),
        )
        .unwrap();
        cfg.ncm.tool_path = tool_path;
        cfg.ncm.command_template = r#"cmd /C "{tool}" "{input}""#.into();

        let src = std::env::temp_dir().join(format!("mt_imp_src_{}_nosid", std::process::id()));
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&src).unwrap();
        let bytes = b"FAKE_NCM_BYTES_NOSID";
        std::fs::write(src.join("trackE.ncm"), bytes).unwrap();

        // 无 source_id 的 import_folder 两次导入：首次入库，第二次按目录+名字+size 短路。
        let first = import_folder(&cfg, &conn, &src, ImportMode::Copy).unwrap();
        assert_eq!(first.imported, 1);
        let parent = src.to_string_lossy().to_string();
        assert!(db::ncm_processed_exists(&conn, &parent, "trackE.ncm", bytes.len() as i64).unwrap());

        let second = import_folder(&cfg, &conn, &src, ImportMode::Copy).unwrap();
        assert_eq!(second.imported, 0, "无 source_id 同目录同名同 size 应短路");
        assert_eq!(second.skipped, 1);

        // 同名不同 size：不短路，重新转换 + 缓存写入。
        let hq = b"FAKE_NCM_BYTES_NOSID_HIGH_QUALITY";
        std::fs::write(src.join("trackE.ncm"), hq).unwrap();
        let third = import_folder(&cfg, &conn, &src, ImportMode::Copy).unwrap();
        assert_eq!(third.imported, 0, "等价音频不重复入库");
        assert!(db::ncm_processed_exists(&conn, &parent, "trackE.ncm", hq.len() as i64).unwrap(),
            "无 source_id 场景高清版判 Skip 后也应写缓存");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_file(&sample);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn source_reimport_restores_unavailable_record_in_place() {
        // v0.17.7：曲源文件还在、本地归位文件被外部删除 → 重导走恢复分支，
        // 清标并写回新路径，保留 local_id（播放列表/收藏引用不断）。
        let root = std::env::temp_dir().join(format!("mt_imp_{}_restore", std::process::id()));
        let src = std::env::temp_dir().join(format!("mt_imp_src_{}_restore", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&src).unwrap();

        let mut cfg = config::build_library_config(&root, "artist_album", "");
        cfg.import.min_size_kb = 0;
        cfg.import.min_duration_sec = 0;
        cfg.ensure_dirs().unwrap();
        let conn = db::open(&cfg).unwrap();

        // 曲源文件（保留不删），首次导入 Copy 到曲库。
        let song = src.join("restore_song.wav");
        write_valid_wav(&song);
        let first = import_source_with_progress(
            &cfg, &conn, &src, true, ImportMode::Copy, "src-restore", &ProgressTracker::new(),
        )
        .unwrap();
        assert_eq!(first.imported, 1);
        let tracks = db::list_tracks(&conn, None, None, false, false).unwrap();
        assert_eq!(tracks.len(), 1);
        let local_id = tracks[0].local_id;
        let lib_path = tracks[0].file_path.clone();
        assert!(std::path::Path::new(&lib_path).is_file());

        // 模拟外部删除归位文件 + 标记不可访问。
        std::fs::remove_file(&lib_path).unwrap();
        db::mark_unavailable(&conn, local_id).unwrap();
        assert_eq!(db::list_unavailable_ids(&conn).unwrap(), vec![local_id]);

        // 曲源重新导入：命中恢复分支（非 Scan 候选 + 旧记录 unavailable）。
        let second = import_source_with_progress(
            &cfg, &conn, &src, true, ImportMode::Copy, "src-restore", &ProgressTracker::new(),
        )
        .unwrap();
        assert_eq!(second.restored, 1, "曲源重导应原位恢复 1 首");
        assert_eq!(second.imported, 0, "恢复不应计为新建");
        assert!(db::list_unavailable_ids(&conn).unwrap().is_empty(), "恢复后应清标");
        let after = db::list_tracks(&conn, None, None, false, false).unwrap();
        assert_eq!(after.len(), 1, "不产生重复记录");
        assert_eq!(after[0].local_id, local_id, "local_id 必须保留");
        assert!(std::path::Path::new(&after[0].file_path).is_file(), "新路径文件应存在");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn streaming_import_converges_progress_and_counts() {
        // v0.17.4：流式逐首入库必须保证每个源文件恰好 advance 一次、结果计数守恒。
        let root = std::env::temp_dir().join(format!("mt_stream_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut cfg = config::build_library_config(&root, "artist_album", "");
        cfg.import.min_size_kb = 0;
        cfg.import.min_duration_sec = 0;
        cfg.ensure_dirs().unwrap();
        let conn = db::open(&cfg).unwrap();

        let src = std::env::temp_dir().join(format!("mt_stream_src_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(&src).unwrap();
        // 两首不同身份的合法 wav：都应入库。
        write_valid_wav(&src.join("song_one.wav"));
        let p2 = src.join("song_two.wav");
        write_valid_wav(&p2);

        let tracker = ProgressTracker::new();
        let res = import_folder_with_progress(
            &cfg, &conn, &src, ImportMode::Copy, Origin::Manual, &tracker,
        ).unwrap();
        let snap = tracker.snapshot();
        assert_eq!(snap.processed, 2, "流式下每个源文件恰好推进一次进度");
        assert_eq!(snap.total, 2);
        assert_eq!(
            res.imported + res.skipped + res.held + res.failed + res.filtered,
            2,
            "流式结果计数必须守恒到源文件总数"
        );
        assert_eq!(snap.imported as u32, res.imported, "快照 imported 与结果一致");
        assert_eq!(snap.new_imports.len() as u32, res.imported, "每首入库都推 new_imports（列表实时刷新）");

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn probe_source_ignores_folder_name_and_uses_ncm_only() {
        let base = std::env::temp_dir().join(format!("mt_probe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        // 目录名带 CloudMusic 但只有普通音频：不得误判为网易云曲库。
        let plain = base.join("CloudMusic");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("song.mp3"), b"FAKE_MP3").unwrap();
        let p1 = probe_source(&plain);
        assert!(!p1.is_netease, "目录名带 CloudMusic 但无 .ncm 不应判网易云");
        assert!(!p1.ncm_found, "无 .ncm 不应报 ncm_found");

        // 目录名普通但含 .ncm：仅凭 .ncm 信号判为网易云强信号。
        let with_ncm = base.join("MySongs");
        std::fs::create_dir_all(&with_ncm).unwrap();
        std::fs::write(with_ncm.join("track.ncm"), b"FAKE_NCM").unwrap();
        let p2 = probe_source(&with_ncm);
        assert!(p2.ncm_found && p2.is_netease, "仅 .ncm 应触发网易云判定");

        let _ = std::fs::remove_dir_all(&base);
    }
}
