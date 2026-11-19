//! FR-22：重复文件挂起与批量对比决策。
//!
//! 设计（见 `docs/01-需求分析.md` FR-22）：
//! - **判重口径 C2** = 歌手 + 歌曲 + 格式（格式是小写扩展名，flac / mp3 不算重复）。
//! - **W3-A′ 两阶段**：批次内先只收集候选（不写 DB / 不归位 / 不弹窗），批次结束统一按桶判定。
//! - **P2 按来源分流**：周期性同步（`netease-sync` / `ncm-backfill`）保留「同大小静默跳过」快速通道，
//!   手工导入与扫描一律挂起对比。
//! - 挂起单位是「重复组 cluster」，落在 `<曲库>/.mt/pending-duplicates.json`；
//!   scan / NCM 临时产物的挂起文件搬进 `<曲库>/.mt/pending-duplicates/`。
//! - **独立同步 / 导入日志** `<曲库>/.mt/sync-import.jsonl`（不是 `app.log`），
//!   记录全部 origin 的同步 / 导入 / 跳过 / 挂起 / 决策 / 失败事件；决策历史只用作弹窗默认预选（C 语义）。
use crate::audio_meta::{extract, quality_tier_of};
use crate::config::Config;
use crate::db;
use crate::library::{derive_names, file_created_ms, target_path, unique_destination};
use crate::{is_playable_audio, Result};
use anyhow::{anyhow, Context};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================
// 基础枚举与键
// ============================================================

/// 一次文件的来源（FR-22 22.3）。**必须沿调用链显式传递**，不能由 ImportMode / 源目录推断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    /// GUI `/api/import-netease` + CLI `import-netease`：整目录 Copy 增量合并。
    NeteaseSync,
    /// `/api/ncm-tool` 指定工具后的 pendingNcm 批量回补（含散落 .ncm 整理）。
    NcmBackfill,
    /// GUI `/api/import` + CLI `import`（Copy / Move 均算）。
    Manual,
    /// boot scan / 手动「扫描曲库」。
    Scan,
}

impl Origin {
    /// 周期性同步：保留「同大小静默跳过」快速通道（P2）。
    pub fn is_periodic(self) -> bool {
        matches!(self, Origin::NeteaseSync | Origin::NcmBackfill)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::NeteaseSync => "netease-sync",
            Origin::NcmBackfill => "ncm-backfill",
            Origin::Manual => "manual",
            Origin::Scan => "scan",
        }
    }
}

/// 候选文件的搬运语义。Scan = 文件本来就在曲库里（不需要归位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileMode {
    Copy,
    Move,
    Scan,
}

impl From<crate::import::ImportMode> for FileMode {
    fn from(m: crate::import::ImportMode) -> Self {
        match m {
            crate::import::ImportMode::Copy => FileMode::Copy,
            crate::import::ImportMode::Move => FileMode::Move,
        }
    }
}

/// 判重三元组（历史 C2：歌手+歌曲+格式）。`key()` 统一小写。
/// v0.14.0 起判重身份改为 `identity_key()`（歌手+歌曲+时长秒级桶，与曲源/格式无关），
/// Triple 仅保留供旧决策历史兼容与 UI 展示，不再作为分组/查库键。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Triple {
    pub artist: String,
    pub title: String,
    pub format: String,
}

/// 时长秒级桶：同一首歌的不同编码容器时长接近，用秒取整作为身份消歧项。
/// 时长缺失（None）返回 -1，作为独立桶，避免所有无时长歌曲互相撞。
pub fn duration_bucket(duration_ms: Option<i64>) -> i64 {
    match duration_ms {
        Some(ms) if ms >= 0 => ms / 1000,
        _ => -1,
    }
}

/// v0.14.0 统一身份键：lower(artist) + lower(title) + 时长秒级桶。
/// 与曲源、格式、码率无关，用于全来源（scan/manual/netease-sync/ncm-backfill）共用判重。
pub fn identity_key(artist: &str, title: &str, duration_ms: Option<i64>) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}",
        artist.to_lowercase(),
        title.to_lowercase(),
        duration_bucket(duration_ms)
    )
}

impl Triple {
    pub fn new(artist: &str, title: &str, format: &str) -> Self {
        Triple {
            artist: artist.to_string(),
            title: title.to_string(),
            format: format.to_ascii_lowercase(),
        }
    }
    pub fn key(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            self.artist.to_lowercase(),
            self.title.to_lowercase(),
            self.format.to_lowercase()
        )
    }
}

/// v0.14.0 质量等价判定：同一身份下，格式相同且码率档位接近视为「同一质量版本」→ 自动跳过。
/// 格式不同（如 mp3 vs flac）或码率档位差超过阈值 → 视为「不同质量版本」→ 挂起让用户选。
/// 码率未知时退化为「格式相同即等价」（保守：不误判不同版本为等价）。
pub fn quality_equivalent(
    cand_format: &str,
    cand_bitrate: Option<i64>,
    cand_quality: &str,
    exist_format: Option<&str>,
    exist_bitrate: Option<i64>,
    exist_quality: Option<&str>,
) -> bool {
    let ef = match exist_format {
        Some(f) => f.to_ascii_lowercase(),
        None => return false,
    };
    if ef != cand_format.to_ascii_lowercase() {
        return false;
    }
    match (cand_bitrate, exist_bitrate) {
        (Some(a), Some(b)) => {
            // 相对差 ≤12% 视为同档（覆盖编码器标称抖动），否则不同质量版本。
            let hi = a.max(b).max(1);
            let lo = a.min(b);
            (hi - lo) as f64 / hi as f64 <= 0.12
        }
        _ => cand_quality == exist_quality.unwrap_or(""),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn mtime_secs(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 稳定哈希（用于 candidate id / group key）。同一「路径 + 大小」跨批次得到同一个 id，
/// 决策历史才能按 id 命中（C 语义默认预选）。
fn stable_hash(s: &str) -> String {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

pub fn candidate_id(source: &Path, size: i64) -> String {
    stable_hash(&format!("{}\u{1f}{}", source.to_string_lossy().to_lowercase(), size))
}

// ============================================================
// 候选 / 组 / 持久化
// ============================================================

/// 阶段① 收集到的一个待入库文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    /// 音频文件当前所在路径。scan / NCM 临时产物在挂起后会被改写为 pending 目录内的路径。
    pub source: String,
    /// 同名 .lrc（W14 跟随被采用的一方）。
    #[serde(default)]
    pub source_lrc: Option<String>,
    /// 归位目标路径；Scan 时为文件原本的曲库路径。
    pub target: String,
    pub name: String,
    pub artist: String,
    pub album: Option<String>,
    pub format: String,
    pub size: i64,
    pub mtime: i64,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub bitrate: Option<i64>,
    pub channels: Option<i64>,
    pub duration_ms: Option<i64>,
    pub quality_tier: String,
    /// #28：音频标签流派 / 年份，随候选一并带到入库阶段（不参与三元组判重）。
    /// `#[serde(default)]` 保证旧 pending JSON 反序列化兼容。
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub year: Option<i64>,
    pub mode: FileMode,
    pub origin: Origin,
    /// NCM 转换产物是临时文件；挂起时必须搬进 pending 目录，否则会被下次 scan 重新入库。
    #[serde(default)]
    pub is_temp: bool,
    /// 曲源 id（app 级 `[[sources]]` 的稳定 id）。`None` = 未指定来源（扫描 /
    /// NCM 回填 / 库内文件）。落库写入 `download_records.source_kind`；替换决策
    /// 时以新文件的归属覆盖旧行，实现「从原曲源列表移到新曲源列表」。
    /// `serde(default)` 保证旧 `pending-duplicates.json` 反序列化兼容。
    #[serde(default)]
    pub source_id: Option<String>,
    /// v0.14.0：NCM 转换产物的来源信息 `(曲源 id, 原始 .ncm 文件名)`。
    /// 落库写入 download_records.ncm_origin_id / ncm_original_name，供转换前短路判重。
    /// 仅 NCM 转换候选携带 Some，其它来源为 None。serde(default) 兼容旧 pending JSON。
    #[serde(default)]
    pub ncm_origin: Option<(String, String)>,
}

impl Candidate {
    pub fn triple(&self) -> Triple {
        Triple::new(&self.artist, &self.name, &self.format)
    }
    /// v0.14.0 统一身份键（歌手+歌曲+时长秒级桶），与曲源/格式无关。
    pub fn identity(&self) -> String {
        identity_key(&self.artist, &self.name, self.duration_ms)
    }
    /// 挂起时是否需要把文件搬进 `.mt/pending-duplicates/`。
    /// manual 的 Copy / Move 源文件**原位不动**（W1①），只有曲库内文件 / NCM 临时产物才搬。
    pub fn stage_on_hold(&self) -> bool {
        self.origin == Origin::Scan || self.is_temp
    }
}

/// 库内已有的那份（按三元组命中，排除批次自身路径）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Existing {
    pub local_id: i64,
    pub file_path: String,
    pub name: String,
    pub artist: Option<String>,
    pub format: Option<String>,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub bitrate: Option<i64>,
    pub channels: Option<i64>,
    pub duration_ms: Option<i64>,
    pub quality_tier: Option<String>,
    pub file_size: i64,
    pub file_mtime: Option<i64>,
}

/// 一个未决重复组（P4 的 cluster）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub id: String,
    /// 幂等登记键：三元组 + 所有候选 (源路径, 大小)。
    pub key: String,
    pub artist: String,
    pub title: String,
    pub format: String,
    pub origin: Origin,
    pub created_at: i64,
    pub existing: Option<Existing>,
    pub candidates: Vec<Candidate>,
    /// C 语义：上次决策作为**默认预选**（仍挂起、仍弹窗，不静默处置）。
    #[serde(default)]
    pub suggested: Option<String>,
    /// 「上次你选了…」的展示文案。
    #[serde(default)]
    pub last_choice: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PendingStore {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub groups: Vec<DuplicateGroup>,
}

pub fn pending_dir(cfg: &Config) -> PathBuf {
    cfg.paths.app_data.join("pending-duplicates")
}
pub fn pending_file(cfg: &Config) -> PathBuf {
    cfg.paths.app_data.join("pending-duplicates.json")
}
pub fn sync_log_file(cfg: &Config) -> PathBuf {
    cfg.paths.app_data.join("sync-import.jsonl")
}

pub fn read_store(cfg: &Config) -> PendingStore {
    let Ok(text) = std::fs::read_to_string(pending_file(cfg)) else {
        return PendingStore::default();
    };
    // 文件损坏 / 旧版本结构不兼容时降级为空，绝不因挂起文件阻断扫描与导入。
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn write_store(cfg: &Config, store: &PendingStore) -> Result<()> {
    let path = pending_file(cfg);
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(store)?;
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("写入挂起组失败：{}", path.display()))?;
    Ok(())
}

/// W15 角标口径：只统计当前曲库、按未决组数。
pub fn pending_count(cfg: &Config) -> usize {
    read_store(cfg).groups.len()
}

pub fn pending_groups(cfg: &Config) -> Vec<DuplicateGroup> {
    read_store(cfg).groups
}

// ============================================================
// 独立同步 / 导入日志（不复用 app.log）
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub ts: i64,
    pub origin: String,
    /// sync / import / skip / hold / decision / failed
    pub action: String,
    pub result: String,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub group_key: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub size: Option<i64>,
    #[serde(default)]
    pub note: Option<String>,
    /// decision 事件：采用 `"existing"` 或某个 candidate id。
    #[serde(default)]
    pub choice: Option<String>,
    /// decision 事件：失效校验快照。
    #[serde(default)]
    pub existing_size: Option<i64>,
    #[serde(default)]
    pub existing_mtime: Option<i64>,
    #[serde(default)]
    pub candidate_sizes: Vec<i64>,
}

/// append-only JSONL。日志不是业务判定依据（除决策预选），写失败不阻断主流程。
pub fn append_log(cfg: &Config, entry: LogEntry) {
    let path = sync_log_file(cfg);
    let Some(parent) = path.parent().map(|p| p.to_path_buf()) else {
        return;
    };
    if std::fs::create_dir_all(&parent).is_err() {
        return;
    }
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    if let Ok(line) = serde_json::to_string(&entry) {
        let _ = writeln!(f, "{line}");
    }
}

fn log_for_candidate(cfg: &Config, action: &str, c: &Candidate, result: &str, note: Option<&str>) {
    append_log(
        cfg,
        LogEntry {
            ts: now_ms(),
            origin: c.origin.as_str().to_string(),
            action: action.to_string(),
            result: result.to_string(),
            group_id: None,
            group_key: None,
            source: Some(c.source.clone()),
            target: Some(c.target.clone()),
            artist: Some(c.artist.clone()),
            title: Some(c.name.clone()),
            format: Some(c.format.clone()),
            size: Some(c.size),
            note: note.map(str::to_string),
            choice: None,
            existing_size: None,
            existing_mtime: None,
            candidate_sizes: Vec::new(),
        },
    );
}

fn log_lines(cfg: &Config) -> Vec<LogEntry> {
    let Ok(text) = std::fs::read_to_string(sync_log_file(cfg)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str::<LogEntry>(l).ok())
        .collect()
}

/// 取某个组键的最后一条决策（读时取最后一条，写入永远 append）。
fn last_decision(cfg: &Config, key: &str) -> Option<LogEntry> {
    log_lines(cfg)
        .into_iter()
        .rev()
        .find(|e| e.action == "decision" && e.group_key.as_deref() == Some(key))
}

/// 失效校验：库内那份 (size, mtime) 或任一候选大小变化 → 历史决策作废，重新挂起对比。
fn decision_still_valid(entry: &LogEntry, group: &DuplicateGroup) -> bool {
    match (&group.existing, entry.existing_size) {
        (Some(ex), Some(sz)) => {
            if sz != ex.file_size || entry.existing_mtime != ex.file_mtime {
                return false;
            }
        }
        (None, None) => {}
        _ => return false,
    }
    let mut a: Vec<i64> = group.candidates.iter().map(|c| c.size).collect();
    let mut b = entry.candidate_sizes.clone();
    a.sort_unstable();
    b.sort_unstable();
    a == b
}

/// C 语义：命中历史决策时只给出默认预选 + 「上次你选了…」文案，不静默处置。
fn decision_hint(cfg: &Config, group: &DuplicateGroup) -> (Option<String>, Option<String>) {
    let Some(entry) = last_decision(cfg, &group.key) else {
        return (None, None);
    };
    if !decision_still_valid(&entry, group) {
        return (None, None);
    }
    let Some(choice) = entry.choice.clone() else {
        return (None, None);
    };
    let label = if choice == "existing" {
        if group.existing.is_none() {
            return (None, None);
        }
        "上次你选择了：保留曲库内的版本".to_string()
    } else {
        let Some(c) = group.candidates.iter().find(|c| c.id == choice) else {
            return (None, None);
        };
        format!("上次你选择了：{}（{}）", file_name_of(&c.source), c.format)
    };
    (Some(choice), Some(label))
}

/// 设置面板「清除同步/导入历史」：清空整份独立日志（决策预选随之失效）。
pub fn clear_history(cfg: &Config) -> Result<()> {
    let path = sync_log_file(cfg);
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(&path, "")?;
    Ok(())
}

fn file_name_of(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string())
}

// ============================================================
// 阶段①：候选收集 / 库内命中查询
// ============================================================

/// 从音频文件抽取标签、音质、归位目标，构造候选。**只读，不写 DB、不搬文件**。
pub fn candidate_from_audio(
    cfg: &Config,
    audio: &Path,
    mode: FileMode,
    origin: Origin,
) -> Result<Candidate> {
    let meta = extract(audio)?;
    let (name, artist, album) = derive_names(audio, &meta);
    let artist = artist.unwrap_or_default();
    let format = audio
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let target = if mode == FileMode::Scan {
        audio.to_path_buf()
    } else {
        target_path(
            cfg,
            &artist,
            &album.clone().unwrap_or_default(),
            &name,
            &format,
        )
    };
    let md = std::fs::metadata(audio)?;
    let size = md.len() as i64;
    Ok(Candidate {
        id: candidate_id(audio, size),
        source: audio.to_string_lossy().to_string(),
        source_lrc: crate::lyrics::find_sidecar(audio)
            .map(|p| p.to_string_lossy().to_string()),
        target: target.to_string_lossy().to_string(),
        name,
        artist,
        album,
        format,
        size,
        mtime: mtime_secs(&md),
        sample_rate: meta.sample_rate.map(|v| v as i64),
        bit_depth: meta.bit_depth.map(|v| v as i64),
        bitrate: meta.bitrate.map(|v| v as i64),
        channels: meta.channels.map(|v| v as i64),
        duration_ms: meta.duration_ms.map(|v| v as i64),
        quality_tier: quality_tier_of(&meta),
        genre: meta.genre.clone(),
        year: meta.year,
        mode,
        origin,
        is_temp: false,
        // 曲源归属由 import.rs 阶段①按入口显式覆盖；扫描 / NCM 回填保持 None。
        source_id: None,
        // NCM 来源由 import.rs 转换后在候选上显式覆盖；其它来源保持 None。
        ncm_origin: None,
    })
}

/// v0.14.0：按统一身份（artist + title + 时长秒级桶）查库内已有记录；
/// `exclude` 用于排除批次自身路径（scan 兜底：否则每个变更文件都跟自己撞）。
/// 时长用 `(duration_ms/1000)` 精确匹配；NULL 时长用 `duration_ms IS NULL` 归入 -1 桶。
pub fn existing_for(
    conn: &Connection,
    artist: &str,
    title: &str,
    dur_bucket: i64,
    exclude: &[String],
) -> Result<Option<Existing>> {
    let dur_sql = if dur_bucket < 0 {
        "duration_ms IS NULL"
    } else {
        "duration_ms/1000 = ?3"
    };
    let sql = format!(
        "SELECT local_id, name, artist, format, sample_rate, bit_depth, bitrate, channels,
                duration_ms, quality_tier, file_size, file_path, file_mtime
         FROM download_records
         WHERE status='downloaded' AND file_path IS NOT NULL
           AND lower(COALESCE(artist,''))=lower(?1)
           AND lower(COALESCE(name,''))=lower(?2)
           AND {dur_sql}
         ORDER BY local_id ASC LIMIT 32"
    );
    let mut stmt = conn.prepare(&sql)?;
    let map_row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Existing> {
        Ok(Existing {
            local_id: r.get(0)?,
            name: r.get(1)?,
            artist: r.get(2)?,
            format: r.get(3)?,
            sample_rate: r.get(4)?,
            bit_depth: r.get(5)?,
            bitrate: r.get(6)?,
            channels: r.get(7)?,
            duration_ms: r.get(8)?,
            quality_tier: r.get(9)?,
            file_size: r.get(10)?,
            file_path: r.get(11)?,
            file_mtime: r.get(12)?,
        })
    };
    let rows: Vec<Existing> = if dur_bucket < 0 {
        stmt.query_map(params![artist, title], map_row)?.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map(params![artist, title, dur_bucket], map_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut first: Option<Existing> = None;
    for e in rows {
        // 路径已被排除（批次自身）或落在挂起/回收站目录内 → 不是「库内那份」。
        if exclude.iter().any(|p| p == &e.file_path)
            || e.file_path.contains("pending-duplicates")
            || e.file_path.contains(".mt")
        {
            continue;
        }
        if first.is_none() {
            first = Some(e);
        }
    }
    Ok(first)
}

// ============================================================
// W3-A′ 判定表（纯函数，便于单测）
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketPlan {
    Hold,
    Skip,
    Import,
}

/// v0.14.0 全来源统一判定树：同一「身份」下按质量决定自动跳过或挂起让用户选。
/// - `bucket_count >= 2`：同批多个同身份文件 → Hold（不让 readdir 顺序决定，交用户选版本）
/// - 库内命中 + 质量等价 → Skip（自动跳过，同一首歌从不同曲源导入也走这里）
/// - 库内命中 + 质量不同 → Hold（列出不同质量版本让用户选留哪个）
/// - 库内未命中 → Import
/// `target_same_size` 保留参数但仅作为「无法取元数据时的质量等价兜底」（归位目标已存在且同大小）。
pub fn plan_bucket(
    bucket_count: usize,
    has_existing: bool,
    quality_equiv: bool,
    target_same_size: bool,
) -> BucketPlan {
    if bucket_count >= 2 {
        return BucketPlan::Hold;
    }
    if !has_existing {
        return BucketPlan::Import;
    }
    if quality_equiv || target_same_size {
        BucketPlan::Skip
    } else {
        BucketPlan::Hold
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    Imported,
    Updated,
    Skipped,
    Held,
    Failed(String),
}

#[derive(Debug, Default)]
pub struct BatchOutcome {
    pub dispositions: Vec<Disposition>,
    pub imported: u32,
    pub updated: u32,
    pub skipped: u32,
    pub held: u32,
    pub failed: u32,
}

// ============================================================
// 阶段②：批次结束统一判定 + 落地
// ============================================================

/// 批次钩子：让调用方在「每首开始归位 / 每首结果落定」时实时上报进度，
/// 并在分桶循环之间协作式检查停止请求。
///
/// 约束：`finalize_with_hook` 对每个候选索引**恰好**触发一次 `on_begin` +
/// `on_settled`（含 Hold 桶的多个索引与取消后未处理的剩余索引），否则
/// `ProgressTracker` 的 `processed` 无法收敛到 `total`。
pub trait BatchHook {
    fn on_begin(&self, index: usize, c: &Candidate);
    fn on_settled(&self, index: usize, c: &Candidate, disposition: &Disposition);
    fn is_cancelled(&self) -> bool;
}

/// 把候选按三元组分桶，批次结束统一判定；只在这一步写 DB / 搬文件 / 登记挂起组。
/// 保留原签名作为薄包装，启动扫描（`library.rs`）与其它调用方零改动。
pub fn finalize(
    cfg: &Config,
    conn: &Connection,
    candidates: &[Candidate],
) -> Result<BatchOutcome> {
    finalize_with_hook(cfg, conn, candidates, None)
}

/// 带可选钩子的批次实现。`hook=None` 时与原 `finalize` 行为逐字等价。
///
/// 停止语义：分桶循环开始前检查取消标志，命中则**跳过剩余桶**，已入库结果不回滚；
/// 剩余索引在补结算循环里按预填的 `Skipped` 结算，保证进度收敛到总数。
pub fn finalize_with_hook(
    cfg: &Config,
    conn: &Connection,
    candidates: &[Candidate],
    hook: Option<&dyn BatchHook>,
) -> Result<BatchOutcome> {
    let n = candidates.len();
    let mut out = BatchOutcome {
        dispositions: vec![Disposition::Skipped; n],
        ..Default::default()
    };
    let mut settled = vec![false; n];
    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, c) in candidates.iter().enumerate() {
        // v0.14.0：按统一身份（artist+title+时长桶）分桶，跨曲源同歌同身份。
        buckets.entry(c.identity()).or_default().push(i);
    }
    let mut store = read_store(cfg);
    let mut dirty = false;

    for (_key, idxs) in buckets {
        if hook.is_some_and(|h| h.is_cancelled()) {
            break;
        }
        let exclude: Vec<String> = idxs.iter().map(|&i| candidates[i].source.clone()).collect();
        let first = &candidates[idxs[0]];
        let existing = existing_for(
            conn,
            &first.artist,
            &first.name,
            duration_bucket(first.duration_ms),
            &exclude,
        )?;
        let target_same_size = Path::new(&first.target)
            .metadata()
            .map(|m| m.len() as i64 == first.size)
            .unwrap_or(false);
        // 统一质量判定：身份命中的前提下比较格式 + 码率档。
        let quality_equiv = existing.as_ref().is_some_and(|e| {
            quality_equivalent(
                &first.format,
                first.bitrate,
                &first.quality_tier,
                e.format.as_deref(),
                e.bitrate,
                e.quality_tier.as_deref(),
            )
        });
        let plan = plan_bucket(
            idxs.len(),
            existing.is_some(),
            quality_equiv,
            target_same_size,
        );
        match plan {
            BucketPlan::Import => {
                let i = idxs[0];
                if let Some(h) = hook {
                    h.on_begin(i, &candidates[i]);
                }
                match import_candidate(cfg, conn, &candidates[i]) {
                    Ok(inserted) => {
                        out.dispositions[i] = if inserted {
                            out.imported += 1;
                            Disposition::Imported
                        } else {
                            out.updated += 1;
                            Disposition::Updated
                        };
                        log_for_candidate(cfg, "import", &candidates[i], "ok", None);
                    }
                    Err(e) => {
                        out.failed += 1;
                        out.dispositions[i] = Disposition::Failed(e.to_string());
                        log_for_candidate(cfg, "failed", &candidates[i], "error", Some(&e.to_string()));
                    }
                }
                if let Some(h) = hook {
                    h.on_settled(i, &candidates[i], &out.dispositions[i]);
                }
                settled[i] = true;
            }
            BucketPlan::Skip => {
                let i = idxs[0];
                if let Some(h) = hook {
                    h.on_begin(i, &candidates[i]);
                }
                out.skipped += 1;
                out.dispositions[i] = Disposition::Skipped;
                log_for_candidate(cfg, "skip", &candidates[i], "ok", Some("同大小，静默跳过"));
                if let Some(h) = hook {
                    h.on_settled(i, &candidates[i], &out.dispositions[i]);
                }
                settled[i] = true;
            }
            BucketPlan::Hold => {
                for &i in &idxs {
                    if let Some(h) = hook {
                        h.on_begin(i, &candidates[i]);
                    }
                }
                match register_group(cfg, &candidates, &idxs, existing, &mut store) {
                    Ok(_) => {
                        for &i in &idxs {
                            out.held += 1;
                            out.dispositions[i] = Disposition::Held;
                            log_for_candidate(cfg, "hold", &candidates[i], "ok", Some("重复组待决策"));
                        }
                        dirty = true;
                    }
                    Err(e) => {
                        for &i in &idxs {
                            out.failed += 1;
                            out.dispositions[i] = Disposition::Failed(e.to_string());
                            log_for_candidate(cfg, "failed", &candidates[i], "error", Some(&e.to_string()));
                        }
                    }
                }
                for &i in &idxs {
                    if let Some(h) = hook {
                        h.on_settled(i, &candidates[i], &out.dispositions[i]);
                    }
                    settled[i] = true;
                }
            }
        }
    }
    // 补结算：取消跳过的桶 / 任何未被上面覆盖的索引，按预填的 Skipped 收敛。
    for i in 0..n {
        if settled[i] {
            continue;
        }
        if let Some(h) = hook {
            h.on_begin(i, &candidates[i]);
            h.on_settled(i, &candidates[i], &out.dispositions[i]);
        }
    }
    if dirty {
        write_store(cfg, &store)?;
    }
    Ok(out)
}

/// 幂等登记（未决期）：同一 (三元组 + 源路径 + 大小) 组已存在时不重复追加。
fn register_group(
    cfg: &Config,
    candidates: &[Candidate],
    idxs: &[usize],
    existing: Option<Existing>,
    store: &mut PendingStore,
) -> Result<()> {
    let first = &candidates[idxs[0]];
    let mut parts: Vec<String> = idxs
        .iter()
        .map(|&i| format!("{}\u{1f}{}", candidates[i].source.to_lowercase(), candidates[i].size))
        .collect();
    parts.sort();
    let key = stable_hash(&format!("{}\u{1f}{}", first.triple().key(), parts.join("\u{1e}")));
    if store.groups.iter().any(|g| g.key == key) {
        return Ok(()); // 幂等：不重复追加
    }

    let mut staged: Vec<Candidate> = Vec::new();
    for &i in idxs {
        let mut c = candidates[i].clone();
        if c.stage_on_hold() {
            stage_candidate_into_pending(cfg, &mut c)?;
        }
        staged.push(c);
    }
    let group = DuplicateGroup {
        id: stable_hash(&format!("{key}\u{1f}{}", now_ms())),
        key,
        artist: first.artist.clone(),
        title: first.name.clone(),
        format: first.format.clone(),
        origin: first.origin,
        created_at: now_ms(),
        existing: existing.clone(),
        candidates: staged,
        suggested: None,
        last_choice: None,
    };
    let (suggested, last_choice) = decision_hint(cfg, &group);
    let mut group = group;
    group.suggested = suggested;
    group.last_choice = last_choice;
    store.groups.push(group);
    Ok(())
}

/// 把曲库内文件 / NCM 临时产物 rename 进 `.mt/pending-duplicates/`（S1），同名 .lrc 一并搬走（W14）。
fn stage_candidate_into_pending(cfg: &Config, c: &mut Candidate) -> Result<()> {
    let dir = pending_dir(cfg);
    std::fs::create_dir_all(&dir)?;
    let src = PathBuf::from(&c.source);
    if !src.is_file() {
        return Ok(()); // 文件已不在（外部删掉）：只登记，不阻断整组
    }
    let dest = unique_destination(&dir, src.file_name().unwrap_or_default());
    std::fs::rename(&src, &dest)
        .with_context(|| format!("无法挂起重复文件：{}", src.display()))?;
    let mut lrc = None;
    if let Some(p) = c.source_lrc.clone() {
        let lp = PathBuf::from(&p);
        if lp.is_file() {
            let ld = unique_destination(&dir, lp.file_name().unwrap_or_default());
            std::fs::rename(&lp, &ld)?;
            lrc = Some(ld.to_string_lossy().to_string());
        }
    }
    c.source_lrc = lrc;
    c.source = dest.to_string_lossy().to_string();
    Ok(())
}

/// 正常入库：ImportMode 决定 copy / move，Scan 原地不动只刷新元数据；随后 upsert（幂等键 = file_path）。
fn import_candidate(_cfg: &Config, conn: &Connection, c: &Candidate) -> Result<bool> {
    let src = Path::new(&c.source);
    let target = PathBuf::from(&c.target);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if c.mode != FileMode::Scan && src != target.as_path() {
        match c.mode {
            FileMode::Copy => {
                std::fs::copy(src, &target)?;
            }
            FileMode::Move => {
                std::fs::rename(src, &target)?;
            }
            FileMode::Scan => {}
        }
    }
    // W14：同名 .lrc 随被采用的音频一起归位。
    if let Some(lrc) = &c.source_lrc {
        let lp = PathBuf::from(lrc);
        if lp.is_file() && c.mode != FileMode::Scan {
            let dst = target.with_extension("lrc");
            let moved = if c.mode == FileMode::Move {
                std::fs::rename(&lp, &dst).is_ok()
            } else {
                false
            };
            if !moved {
                let _ = std::fs::copy(&lp, &dst);
            }
        }
    }
    let md = std::fs::metadata(&target)?;
    let inserted = db::upsert_local_record_with_source(
        conn,
        &target.to_string_lossy(),
        &c.name,
        Some(&c.artist),
        c.album.as_deref(),
        c.duration_ms,
        Some(&c.format),
        c.sample_rate,
        c.bit_depth,
        c.bitrate,
        c.channels,
        Some(&c.quality_tier),
        md.len() as i64,
        mtime_secs(&md),
        file_created_ms(&target),
        c.genre.as_deref(),
        c.year,
        c.source_id.as_deref(),
        c.ncm_origin.as_ref().map(|(id, name)| (id.as_str(), name.as_str())),
    )?;
    Ok(inserted)
}

// ============================================================
// 批量决策
// ============================================================

#[derive(Debug, Clone, Deserialize)]
pub struct DecisionInput {
    pub group_id: String,
    /// `"existing"` = 保留库内那份；否则为某个 candidate.id = 采用新文件。
    pub choice: String,
}

#[derive(Debug, Default)]
pub struct DecisionSummary {
    pub applied: u32,
    pub failed: Vec<String>,
}

/// 提交决策：只处理传入的组，未列出的组保持挂起（「稍后处理」/关弹窗 = 不调此接口）。
pub fn apply_decisions(
    cfg: &Config,
    conn: &Connection,
    inputs: &[DecisionInput],
) -> Result<DecisionSummary> {
    let mut store = read_store(cfg);
    let mut out = DecisionSummary::default();
    for input in inputs {
        let Some(pos) = store.groups.iter().position(|g| g.id == input.group_id) else {
            out.failed.push(format!("{}：重复组不存在或已处理", input.group_id));
            continue;
        };
        let group = store.groups[pos].clone();
        match apply_one(cfg, conn, &group, &input.choice) {
            Ok(()) => {
                store.groups.remove(pos);
                out.applied += 1;
                append_log(
                    cfg,
                    LogEntry {
                        ts: now_ms(),
                        origin: group.origin.as_str().to_string(),
                        action: "decision".to_string(),
                        result: "ok".to_string(),
                        group_id: Some(group.id.clone()),
                        group_key: Some(group.key.clone()),
                        source: None,
                        target: group.existing.as_ref().map(|e| e.file_path.clone()),
                        artist: Some(group.artist.clone()),
                        title: Some(group.title.clone()),
                        format: Some(group.format.clone()),
                        size: None,
                        note: None,
                        choice: Some(input.choice.clone()),
                        existing_size: group.existing.as_ref().map(|e| e.file_size),
                        existing_mtime: group.existing.as_ref().and_then(|e| e.file_mtime),
                        candidate_sizes: group.candidates.iter().map(|c| c.size).collect(),
                    },
                );
            }
            Err(e) => out.failed.push(format!("{}：{e}", input.group_id)),
        }
    }
    write_store(cfg, &store)?;
    Ok(out)
}

fn apply_one(cfg: &Config, conn: &Connection, group: &DuplicateGroup, choice: &str) -> Result<()> {
    if choice == "existing" {
        // W1① / P3：manual 的 Copy / Move 源文件原位不动；scan / NCM 临时产物进 `.mt/trash/`。
        for c in &group.candidates {
            if c.stage_on_hold() {
                discard_candidate(cfg, c)?;
            }
        }
        return Ok(());
    }
    let chosen = group
        .candidates
        .iter()
        .find(|c| c.id == choice)
        .ok_or_else(|| anyhow!("所选文件不在该重复组内"))?;

    // C3①：库内已有记录 → 原地替换其 file_path，保留 local_id 与全部播放列表引用。
    // W2①：组内全是新文件 → 选中的按正常归位路径新建 local_id。
    let target = crate::library::ensure_within_library(
        cfg,
        Path::new(
            &group
                .existing
                .as_ref()
                .map(|e| e.file_path.clone())
                .unwrap_or_else(|| chosen.target.clone()),
        ),
    )?;
    let src = PathBuf::from(&chosen.source);
    if !src.is_file() {
        return Err(anyhow!("所选文件已不在磁盘：{}", src.display()));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // 被替换的旧文件 + 它的 .lrc 一起进回收站（不做不可逆删除）。
    if target.exists() && target != src {
        move_to_trash(cfg, &target)?;
        let old_lrc = target.with_extension("lrc");
        if old_lrc.is_file() {
            move_to_trash(cfg, &old_lrc)?;
        }
    }
    match chosen.mode {
        FileMode::Copy => {
            std::fs::copy(&src, &target)?;
        }
        _ => {
            std::fs::rename(&src, &target)?;
        }
    }
    // W14：采用新的 → 同名 .lrc 随新文件归位；Copy 模式下源 .lrc 留在原处。
    if let Some(lrc) = &chosen.source_lrc {
        let lp = PathBuf::from(lrc);
        if lp.is_file() {
            let dst = target.with_extension("lrc");
            let moved = if chosen.mode == FileMode::Copy {
                false
            } else {
                std::fs::rename(&lp, &dst).is_ok()
            };
            if !moved {
                let _ = std::fs::copy(&lp, &dst);
            }
        }
    }
    // 未采用方：scan / NCM 临时产物连同 .lrc 进 trash；manual 源文件保持原位。
    for c in &group.candidates {
        if c.id != chosen.id && c.stage_on_hold() {
            discard_candidate(cfg, c)?;
        }
    }
    let md = std::fs::metadata(&target)?;
    // C3①：复用旧 file_path 走 UPDATE，保留 local_id 与播放列表引用；同时把归属
    // 改成新文件的曲源，前端重新分组即「从原列表删除、在新列表添加」。
    // 旧 pending JSON 无来源信息时显式归「未指定来源」（不能传 None，那会保留旧归属）。
    let source_id = chosen
        .source_id
        .as_deref()
        .unwrap_or(db::UNSPECIFIED_SOURCE);
    db::upsert_local_record_with_source(
        conn,
        &target.to_string_lossy(),
        &chosen.name,
        Some(&chosen.artist),
        chosen.album.as_deref(),
        chosen.duration_ms,
        Some(&chosen.format),
        chosen.sample_rate,
        chosen.bit_depth,
        chosen.bitrate,
        chosen.channels,
        Some(&chosen.quality_tier),
        md.len() as i64,
        mtime_secs(&md),
        file_created_ms(&target),
        chosen.genre.as_deref(),
        chosen.year,
        Some(source_id),
        chosen.ncm_origin.as_ref().map(|(id, name)| (id.as_str(), name.as_str())),
    )?;
    Ok(())
}

fn discard_candidate(cfg: &Config, c: &Candidate) -> Result<()> {
    move_to_trash(cfg, Path::new(&c.source))?;
    if let Some(lrc) = &c.source_lrc {
        let p = Path::new(lrc);
        if p.is_file() {
            move_to_trash(cfg, p)?;
        }
    }
    Ok(())
}

fn move_to_trash(cfg: &Config, path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let dir = cfg.paths.app_data.join("trash");
    std::fs::create_dir_all(&dir)?;
    let dest = unique_destination(&dir, path.file_name().unwrap_or_default());
    std::fs::rename(path, &dest)
        .with_context(|| format!("无法移入回收站：{}", path.display()))?;
    Ok(())
}

/// 试听寻址：只按 candidate id 从挂起组里取路径，不接受请求方传入任意路径。
pub fn preview_path(cfg: &Config, candidate_id: &str) -> Option<PathBuf> {
    let groups = pending_groups(cfg);
    let c = groups
        .iter()
        .flat_map(|g| g.candidates.iter())
        .find(|c| c.id == candidate_id)?;
    let p = PathBuf::from(&c.source);
    if p.is_file() && is_playable_audio(&p) {
        Some(p)
    } else {
        None
    }
}

// ============================================================
// 单测
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_periodic_channel_is_only_for_sync_sources() {
        assert!(Origin::NeteaseSync.is_periodic());
        assert!(Origin::NcmBackfill.is_periodic());
        assert!(!Origin::Manual.is_periodic());
        assert!(!Origin::Scan.is_periodic());
        assert_eq!(Origin::NeteaseSync.as_str(), "netease-sync");
        assert_eq!(Origin::Manual.as_str(), "manual");
    }

    #[test]
    fn triple_key_ignores_case_but_distinguishes_format() {
        let a = Triple::new("Alan Walker", "Faded", "FLAC").key();
        let b = Triple::new("alan walker", "FADED", "flac").key();
        let c = Triple::new("Alan Walker", "Faded", "mp3").key();
        assert_eq!(a, b, "大小写不应产生不同重复组");
        assert_ne!(a, c, "flac / mp3 不算重复（C2）");
    }

    #[test]
    fn plan_bucket_unified_identity_quality_table() {
        use BucketPlan::*;
        // A：同批 ≥2 个同身份文件 → Hold（不区分曲源，不让 readdir 顺序决定版本）
        assert_eq!(plan_bucket(2, false, false, false), Hold);
        assert_eq!(plan_bucket(2, true, true, false), Hold);
        assert_eq!(plan_bucket(3, true, false, true), Hold);
        // B：库内命中 + 质量等价 → Skip（同一首歌从不同曲源导入自动跳过）
        assert_eq!(plan_bucket(1, true, true, false), Skip);
        assert_eq!(plan_bucket(1, true, true, true), Skip);
        // 元数据不足时：归位目标已存在且同大小 → 兜底 Skip
        assert_eq!(plan_bucket(1, true, false, true), Skip);
        // C：库内命中但质量不同 → Hold（列出不同质量版本让用户选留哪个）
        assert_eq!(plan_bucket(1, true, false, false), Hold);
        // D：库内未命中 → Import
        assert_eq!(plan_bucket(1, false, false, false), Import);
        assert_eq!(plan_bucket(1, false, false, true), Import);
    }

    #[test]
    fn identity_key_unifies_sources_and_formats() {
        // 身份 = lower(artist) + lower(title) + 时长秒桶，与曲源 / 格式 / 码率无关
        assert_eq!(
            identity_key("Alan Walker", "Faded", Some(210_400)),
            identity_key("alan walker", "FADED", Some(210_999)),
        );
        // 不同秒桶 → 不同身份
        assert_ne!(
            identity_key("Alan Walker", "Faded", Some(210_000)),
            identity_key("Alan Walker", "Faded", Some(212_000)),
        );
        // 时长缺失独立桶 -1，不与 0 秒冲突
        assert_eq!(identity_key("A", "T", None), identity_key("a", "t", None));
        assert_ne!(identity_key("A", "T", None), identity_key("A", "T", Some(0)));
        assert_eq!(duration_bucket(Some(1_999)), 1);
        assert_eq!(duration_bucket(None), -1);
    }

    #[test]
    fn quality_equivalent_distinguishes_format_and_bitrate() {
        // 同格式 + 码率相对差 ≤12% → 同一质量版本
        assert!(quality_equivalent(
            "flac", Some(1000), "lossless",
            Some("FLAC"), Some(1050), Some("lossless"),
        ));
        // 同格式但码率档差明显 → 不同质量版本
        assert!(!quality_equivalent(
            "mp3", Some(128), "lossy",
            Some("mp3"), Some(320), Some("lossy"),
        ));
        // 格式不同 → 不同质量版本
        assert!(!quality_equivalent(
            "flac", Some(1000), "lossless",
            Some("mp3"), Some(320), Some("lossy"),
        ));
        // 码率未知时以 quality_tier 兜底
        assert!(quality_equivalent("flac", None, "lossless", Some("flac"), None, Some("lossless")));
        assert!(!quality_equivalent("flac", None, "lossless", Some("flac"), None, Some("lossy")));
        // 库内无格式信息 → 不等价（保守，不误自动跳过）
        assert!(!quality_equivalent("flac", Some(1000), "lossless", None, Some(1000), Some("lossless")));
    }

    #[test]
    fn candidate_id_is_stable_and_distinguishes_size() {
        let p = Path::new("D:/music/Faded.flac");
        assert_eq!(candidate_id(p, 100), candidate_id(p, 100));
        assert_ne!(candidate_id(p, 100), candidate_id(p, 200));
    }

    #[test]
    fn store_roundtrip_and_corrupt_fallback() {
        let dir = std::env::temp_dir().join(format!("kqk-dup-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".mt")).unwrap();
        let cfg = crate::config::build_library_config(&dir, "artist_album", "");
        let mut store = PendingStore::default();
        store.groups.push(DuplicateGroup {
            id: "g1".into(),
            key: "k1".into(),
            artist: "A".into(),
            title: "T".into(),
            format: "flac".into(),
            origin: Origin::Manual,
            created_at: 1,
            existing: None,
            candidates: Vec::new(),
            suggested: Some("existing".into()),
            last_choice: None,
        });
        write_store(&cfg, &store).unwrap();
        let back = read_store(&cfg);
        assert_eq!(back.groups.len(), 1);
        assert_eq!(back.groups[0].suggested.as_deref(), Some("existing"));
        std::fs::write(pending_file(&cfg), "{ broken").unwrap();
        assert!(read_store(&cfg).groups.is_empty(), "损坏文件必须降级为空");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decision_history_is_last_one_and_invalidates_on_size_change() {
        let dir = std::env::temp_dir().join(format!("kqk-dup-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".mt")).unwrap();
        let cfg = crate::config::build_library_config(&dir, "artist_album", "");
        let base = LogEntry {
            ts: 1,
            origin: "manual".into(),
            action: "decision".into(),
            result: "ok".into(),
            group_id: Some("g".into()),
            group_key: Some("k".into()),
            source: None,
            target: None,
            artist: None,
            title: None,
            format: None,
            size: None,
            note: None,
            choice: Some("existing".into()),
            existing_size: Some(100),
            existing_mtime: Some(7),
            candidate_sizes: vec![100, 200],
        };
        append_log(&cfg, base.clone());
        let mut later = base.clone();
        later.ts = 2;
        later.choice = Some("cand-2".into());
        append_log(&cfg, later);
        let last = last_decision(&cfg, "k").unwrap();
        assert_eq!(last.choice.as_deref(), Some("cand-2"), "必须取最后一条决策");

        let group = DuplicateGroup {
            id: "g".into(),
            key: "k".into(),
            artist: "A".into(),
            title: "T".into(),
            format: "flac".into(),
            origin: Origin::Manual,
            created_at: 1,
            existing: Some(Existing {
                local_id: 1,
                file_path: "D:/lib/A/T.flac".into(),
                name: "T".into(),
                artist: Some("A".into()),
                format: Some("flac".into()),
                sample_rate: None,
                bit_depth: None,
                bitrate: None,
                channels: None,
                duration_ms: None,
                quality_tier: None,
                file_size: 100,
                file_mtime: Some(7),
            }),
            candidates: vec![
                Candidate {
                    id: "cand-2".into(), source: "x".into(), source_lrc: None, target: "t".into(),
                    name: "T".into(), artist: "A".into(), album: None, format: "flac".into(),
                    size: 100, mtime: 0, sample_rate: None, bit_depth: None, bitrate: None,
                    channels: None, duration_ms: None, quality_tier: "lossless".into(),
                    genre: None, year: None,
                    mode: FileMode::Copy, origin: Origin::Manual, is_temp: false, source_id: None,
                    ncm_origin: None,
                },
                Candidate {
                    id: "cand-3".into(), source: "y".into(), source_lrc: None, target: "t".into(),
                    name: "T".into(), artist: "A".into(), album: None, format: "flac".into(),
                    size: 200, mtime: 0, sample_rate: None, bit_depth: None, bitrate: None,
                    channels: None, duration_ms: None, quality_tier: "lossless".into(),
                    genre: None, year: None,
                    mode: FileMode::Copy, origin: Origin::Manual, is_temp: false, source_id: None,
                    ncm_origin: None,
                },
            ],
            suggested: None,
            last_choice: None,
        };
        assert!(decision_still_valid(&last, &group));
        let hint = decision_hint(&cfg, &group);
        assert_eq!(hint.0.as_deref(), Some("cand-2"));
        assert!(hint.1.unwrap().contains("上次你选择了"));

        let mut changed = group.clone();
        changed.existing.as_mut().unwrap().file_size = 999;
        assert!(!decision_still_valid(&last, &changed), "库内那份大小变化应作废");
        assert_eq!(decision_hint(&cfg, &changed).0, None);

        clear_history(&cfg).unwrap();
        assert!(last_decision(&cfg, "k").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 造一个不落磁盘的候选：Manual + Copy，`stage_on_hold()` 为 false，
    /// 因此 Hold 桶只登记 pending 组，不会搬文件，适合纯逻辑单测。
    fn hook_candidate(name: &str, source: &str) -> Candidate {
        Candidate {
            id: format!("cand-{source}"),
            source: source.into(),
            source_lrc: None,
            target: format!("/lib/{name}.flac"),
            name: name.into(),
            artist: "A".into(),
            album: None,
            format: "flac".into(),
            size: 100,
            mtime: 0,
            sample_rate: None,
            bit_depth: None,
            bitrate: None,
            channels: None,
            duration_ms: None,
            quality_tier: "lossless".into(),
            genre: None,
            year: None,
            mode: FileMode::Copy,
            origin: Origin::Manual,
            is_temp: false,
            source_id: Some("src-a".into()),
            ncm_origin: None,
        }
    }

    struct RecordingHook {
        /// 第几次 `is_cancelled()` 起返回 true；`None` = 永不取消。
        cancel_at: Option<usize>,
        checks: std::cell::Cell<usize>,
        begun: std::cell::RefCell<Vec<usize>>,
        settled: std::cell::RefCell<Vec<usize>>,
    }

    impl RecordingHook {
        fn new(cancel_at: Option<usize>) -> Self {
            RecordingHook {
                cancel_at,
                checks: std::cell::Cell::new(0),
                begun: std::cell::RefCell::new(Vec::new()),
                settled: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl BatchHook for RecordingHook {
        fn on_begin(&self, index: usize, _c: &Candidate) {
            self.begun.borrow_mut().push(index);
        }
        fn on_settled(&self, index: usize, _c: &Candidate, _d: &Disposition) {
            self.settled.borrow_mut().push(index);
        }
        fn is_cancelled(&self) -> bool {
            let n = self.checks.get() + 1;
            self.checks.set(n);
            self.cancel_at.is_some_and(|k| n >= k)
        }
    }

    #[test]
    fn finalize_hook_settles_every_index_once_when_cancelled_upfront() {
        use crate::db;
        let dir = std::env::temp_dir().join(format!("kqk-hook-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".mt")).unwrap();
        let cfg = crate::config::build_library_config(&dir, "artist_album", "");
        let conn = db::open(&cfg).unwrap();
        let candidates: Vec<Candidate> = (0..5)
            .map(|i| hook_candidate(&format!("T{i}"), &format!("/src/{i}.flac")))
            .collect();
        let hook = RecordingHook::new(Some(1));
        let out = finalize_with_hook(&cfg, &conn, &candidates, Some(&hook)).unwrap();

        // 分桶循环立即 break，5 个索引全部走补结算：一个不漏、一个不重。
        let mut settled = hook.settled.borrow().clone();
        settled.sort_unstable();
        assert_eq!(settled, vec![0, 1, 2, 3, 4]);
        let mut begun = hook.begun.borrow().clone();
        begun.sort_unstable();
        assert_eq!(begun, vec![0, 1, 2, 3, 4]);
        assert_eq!(out.dispositions.len(), 5);
        assert!(out
            .dispositions
            .iter()
            .all(|d| *d == Disposition::Skipped));
        assert_eq!(out.imported, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finalize_hook_hold_bucket_settles_all_members_and_cancel_keeps_done() {
        use crate::db;
        let dir = std::env::temp_dir().join(format!("kqk-hook-hold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".mt")).unwrap();
        let cfg = crate::config::build_library_config(&dir, "artist_album", "");
        let conn = db::open(&cfg).unwrap();
        // 两个三元组不同的桶，各 2 个候选 → 都判 Hold；第二次检查时请求停止。
        let candidates = vec![
            hook_candidate("T1", "/src/1.flac"),
            hook_candidate("T1", "/src/2.flac"),
            hook_candidate("T2", "/src/3.flac"),
            hook_candidate("T2", "/src/4.flac"),
        ];
        let hook = RecordingHook::new(Some(2));
        let out = finalize_with_hook(&cfg, &conn, &candidates, Some(&hook)).unwrap();

        let mut settled = hook.settled.borrow().clone();
        settled.sort_unstable();
        assert_eq!(settled, vec![0, 1, 2, 3], "Hold 桶的每个成员都要结算");
        // 第一个桶已登记（held=2）不回滚；第二个桶跳过，仍结算为 Skipped。
        assert_eq!(out.held, 2);
        assert_eq!(out.dispositions.len(), 4);
        assert_eq!(
            out.dispositions.iter().filter(|d| **d == Disposition::Held).count(),
            2
        );
        assert_eq!(
            out.dispositions
                .iter()
                .filter(|d| **d == Disposition::Skipped)
                .count(),
            2
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
