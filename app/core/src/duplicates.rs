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

/// 判重三元组（C2）。`key()` 统一小写，大小写差异不产生两个重复组。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Triple {
    pub artist: String,
    pub title: String,
    pub format: String,
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
}

impl Candidate {
    pub fn triple(&self) -> Triple {
        Triple::new(&self.artist, &self.name, &self.format)
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
    })
}

/// 按三元组查库内已有记录；`exclude` 用于排除批次自身路径（scan 兜底：否则每个变更文件都跟自己撞）。
pub fn existing_for(
    conn: &Connection,
    triple: &Triple,
    exclude: &[String],
) -> Result<Option<Existing>> {
    let mut stmt = conn.prepare(
        "SELECT local_id, name, artist, format, sample_rate, bit_depth, bitrate, channels,
                duration_ms, quality_tier, file_size, file_path, file_mtime
         FROM download_records
         WHERE status='downloaded' AND file_path IS NOT NULL
           AND lower(COALESCE(artist,''))=lower(?1)
           AND lower(COALESCE(name,''))=lower(?2)
           AND lower(COALESCE(format,''))=lower(?3)
         ORDER BY local_id ASC LIMIT 32",
    )?;
    let rows = stmt.query_map(
        params![triple.artist, triple.title, triple.format],
        |r| {
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
        },
    )?;
    let mut first: Option<Existing> = None;
    for row in rows {
        let e = row?;
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

/// `existing_size` = Some(库内那份的大小) 表示三元组命中；`target_same_size` = 归位目标已存在且同大小
/// （兼容旧行为：网易云同步的库内记录可能因重扫后标签漂移而三元组不命中）。
pub fn plan_bucket(
    new_count: usize,
    origin: Origin,
    existing_size: Option<i64>,
    candidate_size: i64,
    target_same_size: bool,
) -> BucketPlan {
    if new_count >= 2 {
        return BucketPlan::Hold; // A 分支：不让 read_dir / 选文件顺序决定谁入库
    }
    match existing_size {
        Some(sz) => {
            if origin.is_periodic() && (sz == candidate_size || target_same_size) {
                BucketPlan::Skip // B 分支：周期性同步的快速通道
            } else {
                BucketPlan::Hold // C 分支 / manual / scan 一律挂起
            }
        }
        None => BucketPlan::Import, // D 分支
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

/// 把候选按三元组分桶，批次结束统一判定；只在这一步写 DB / 搬文件 / 登记挂起组。
pub fn finalize(
    cfg: &Config,
    conn: &Connection,
    candidates: &[Candidate],
) -> Result<BatchOutcome> {
    let n = candidates.len();
    let mut out = BatchOutcome {
        dispositions: vec![Disposition::Skipped; n],
        ..Default::default()
    };
    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, c) in candidates.iter().enumerate() {
        buckets.entry(c.triple().key()).or_default().push(i);
    }
    let mut store = read_store(cfg);
    let mut dirty = false;

    for (_key, idxs) in buckets {
        let exclude: Vec<String> = idxs.iter().map(|&i| candidates[i].source.clone()).collect();
        let first = &candidates[idxs[0]];
        let existing = existing_for(conn, &first.triple(), &exclude)?;
        let target_same_size = Path::new(&first.target)
            .metadata()
            .map(|m| m.len() as i64 == first.size)
            .unwrap_or(false);
        let plan = plan_bucket(
            idxs.len(),
            first.origin,
            existing.as_ref().map(|e| e.file_size),
            first.size,
            target_same_size,
        );
        match plan {
            BucketPlan::Import => {
                let i = idxs[0];
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
            }
            BucketPlan::Skip => {
                let i = idxs[0];
                out.skipped += 1;
                out.dispositions[i] = Disposition::Skipped;
                log_for_candidate(cfg, "skip", &candidates[i], "ok", Some("同大小，静默跳过"));
            }
            BucketPlan::Hold => {
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
            }
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
    let inserted = db::upsert_local_record(
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
    db::upsert_local_record(
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
    fn plan_bucket_matches_w3_a_prime_table() {
        use BucketPlan::*;
        // A：同批 ≥2，任意 origin 一律挂起
        for o in [Origin::NeteaseSync, Origin::NcmBackfill, Origin::Manual, Origin::Scan] {
            assert_eq!(plan_bucket(2, o, None, 100, false), Hold);
            assert_eq!(plan_bucket(3, o, Some(100), 100, false), Hold);
        }
        // B：周期性同步 + 1 个新文件 + 库内命中 + 同大小 → 静默跳过
        assert_eq!(plan_bucket(1, Origin::NeteaseSync, Some(100), 100, false), Skip);
        assert_eq!(plan_bucket(1, Origin::NcmBackfill, Some(100), 100, false), Skip);
        // 兼容旧行为：目标文件同大小亦跳过
        assert_eq!(plan_bucket(1, Origin::NeteaseSync, Some(999), 100, true), Skip);
        // C：周期性同步但大小不同 → 挂起
        assert_eq!(plan_bucket(1, Origin::NeteaseSync, Some(999), 100, false), Hold);
        // manual / scan 命中即挂起，即使同大小
        assert_eq!(plan_bucket(1, Origin::Manual, Some(100), 100, false), Hold);
        assert_eq!(plan_bucket(1, Origin::Scan, Some(100), 100, false), Hold);
        assert_eq!(plan_bucket(1, Origin::Scan, Some(100), 100, true), Hold);
        // D：无命中 → 正常入库
        assert_eq!(plan_bucket(1, Origin::Manual, None, 100, false), Import);
        assert_eq!(plan_bucket(1, Origin::Scan, None, 100, false), Import);
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
                    mode: FileMode::Copy, origin: Origin::Manual, is_temp: false,
                },
                Candidate {
                    id: "cand-3".into(), source: "y".into(), source_lrc: None, target: "t".into(),
                    name: "T".into(), artist: "A".into(), album: None, format: "flac".into(),
                    size: 200, mtime: 0, sample_rate: None, bit_depth: None, bitrate: None,
                    channels: None, duration_ms: None, quality_tier: "lossless".into(),
                    genre: None, year: None,
                    mode: FileMode::Copy, origin: Origin::Manual, is_temp: false,
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
}
