//! 播放列表导入导出（FR-20，格式方案 A）。
//!
//! **DB 是运行时唯一权威**，文件只在导入 / 导出时存在，不做双写：
//! - `.json`：内部无损备份（含 local_id / file_path / position / 时间字段），**可回读**；
//! - `.m3u8`：UTF-8 Extended M3U（`#EXTM3U` / `#EXTINF`，**不是 HLS 清单**），
//!   单向投影、**不回读**，仅给外部播放器 / 网易云本地导出文件做交换。
//!
//! 导入侧：外部文件没有 local_id，按「相对路径 → 绝对路径 → 文件名+艺人 → 标签」匹配；
//! 去重 ① `local_id` 级、② 内容级「文件名 + 艺人 + 时长」，**都发生在写 DB 之前**，
//! DB 主键 `(playlist_id, local_id)` + `INSERT OR IGNORE` 是最后一道兜底。

use crate::Result;
use crate::db::PlaylistItemView;
use anyhow::bail;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const BACKUP_FORMAT: &str = "kqk-playlist";
pub const BACKUP_VERSION: u32 = 1;

// ============================================================
// 导出：DB → JSON 备份 + M3U8 投影
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupTrack {
    pub position: i64,
    pub local_id: i64,
    pub name: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<i64>,
    pub format: Option<String>,
    pub quality_tier: Option<String>,
    pub file_size: i64,
    /// 相对曲库根的路径（换机器后仍可凭文件名 + 艺人重匹配）。
    pub file_path: String,
    pub imported_at: Option<i64>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistBackup {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub library_name: String,
    /// 导出时刻（UNIX 毫秒）。
    pub exported_at: i64,
    pub tracks: Vec<BackupTrack>,
}

/// 把绝对路径转成相对曲库根、统一用 `/` 分隔；不在曲库根下时退化为绝对路径。
pub fn relative_path(root: &Path, full: &Path) -> String {
    let f = full.to_string_lossy().replace('\\', "/");
    let r = root.to_string_lossy().replace('\\', "/");
    let r = r.trim_end_matches('/');
    if !r.is_empty() && f.to_ascii_lowercase().starts_with(&r.to_ascii_lowercase()) {
        let rest = f[r.len()..].trim_start_matches('/');
        if !rest.is_empty() {
            return rest.to_string();
        }
    }
    f
}

/// 文件名安全化：去掉路径分隔符与 Windows 非法字符，避免导出时路径穿越。
pub fn safe_file_stem(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim().trim_end_matches('.').trim().to_string();
    if s.is_empty() {
        "playlist".to_string()
    } else {
        s
    }
}

pub fn build_backup(
    name: &str,
    library_name: &str,
    root: &Path,
    tracks: &[PlaylistItemView],
) -> PlaylistBackup {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    PlaylistBackup {
        format: BACKUP_FORMAT.to_string(),
        version: BACKUP_VERSION,
        name: name.to_string(),
        library_name: library_name.to_string(),
        exported_at: now,
        tracks: tracks
            .iter()
            .map(|t| BackupTrack {
                position: t.position,
                local_id: t.local_id,
                name: t.name.clone(),
                artist: t.artist.clone(),
                album: t.album.clone(),
                duration_ms: t.duration_ms,
                format: t.format.clone(),
                quality_tier: t.quality_tier.clone(),
                file_size: t.file_size,
                file_path: relative_path(root, Path::new(&t.file_path)),
                imported_at: t.imported_at,
                created_at: t.created_at,
            })
            .collect(),
    }
}

/// M3U8 单向投影：只有 `#EXTINF` 与相对路径行，元数据有损、绝不回读。
pub fn build_m3u8(name: &str, tracks: &[PlaylistItemView], root: &Path) -> String {
    let mut out = String::from("#EXTM3U\n");
    if !name.trim().is_empty() {
        out.push_str("#PLAYLIST:");
        out.push_str(&name.replace(['\r', '\n'], " "));
        out.push('\n');
    }
    for t in tracks {
        let secs = t.duration_ms.unwrap_or(0) / 1000;
        let artist = t.artist.clone().unwrap_or_default();
        let title = if artist.trim().is_empty() {
            t.name.clone()
        } else {
            format!("{artist} - {}", t.name)
        };
        out.push_str(&format!(
            "#EXTINF:{},{}\n{}\n",
            secs.max(0),
            title.replace(['\r', '\n'], " "),
            relative_path(root, Path::new(&t.file_path)),
        ));
    }
    out
}

// ============================================================
// 导入：外部文件 → 条目 → 匹配 → 去重 → local_id[]
// ============================================================

#[derive(Debug, Clone, Default)]
pub struct ImportEntry {
    pub path: Option<String>,
    pub name: Option<String>,
    pub artist: Option<String>,
    pub duration_ms: Option<i64>,
}

impl ImportEntry {
    fn label(&self) -> String {
        let n = self.name.clone().unwrap_or_default();
        let a = self.artist.clone().unwrap_or_default();
        if n.is_empty() {
            self.path.clone().unwrap_or_else(|| "(空条目)".into())
        } else if a.is_empty() {
            n
        } else {
            format!("{a} - {n}")
        }
    }
}

#[derive(Debug, Default)]
pub struct ParsedPlaylist {
    pub name: Option<String>,
    pub entries: Vec<ImportEntry>,
}

/// 按扩展名分派解析：M3U8 / JSON / CSV。网易云本地导出常见为这三类。
pub fn parse_file(path: &Path) -> Result<ParsedPlaylist> {
    let text = std::fs::read_to_string(path)?;
    let text = text.trim_start_matches('\u{feff}'); // UTF-8 BOM 容错
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut parsed = match ext.as_str() {
        "m3u" | "m3u8" => parse_m3u(text),
        "json" => parse_json(text)?,
        "csv" => parse_csv(text),
        _ => bail!("不支持的播放列表格式：.{ext}（支持 .m3u / .m3u8 / .json / .csv）"),
    };
    if parsed.name.as_deref().unwrap_or("").trim().is_empty() {
        parsed.name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .filter(|s| !s.trim().is_empty());
    }
    if parsed.entries.is_empty() {
        bail!("播放列表文件里没有可导入的曲目");
    }
    Ok(parsed)
}

fn split_extinf_title(s: &str) -> (Option<String>, Option<String>) {
    // 常见写法「艺人 - 曲名」；没有分隔符时整体当曲名。
    for sep in [" - ", " – ", " — "] {
        if let Some(i) = s.find(sep) {
            return (
                Some(s[..i].trim().to_string()),
                Some(s[i + sep.len()..].trim().to_string()),
            );
        }
    }
    (None, Some(s.trim().to_string()))
}

fn parse_m3u(text: &str) -> ParsedPlaylist {
    let mut out = ParsedPlaylist::default();
    let mut pending: Option<ImportEntry> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#PLAYLIST:") {
            out.name = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let mut e = ImportEntry::default();
            if let Some((dur, title)) = rest.split_once(',') {
                if let Ok(secs) = dur.trim().parse::<f64>() {
                    e.duration_ms = Some((secs * 1000.0).round() as i64);
                }
                let (artist, name) = split_extinf_title(title.trim());
                e.artist = artist;
                e.name = name;
            }
            pending = Some(e);
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let mut e = pending.take().unwrap_or_default();
        e.path = Some(line.to_string());
        out.entries.push(e);
    }
    out
}

fn str_field(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v.get(k).and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                return Some(s.trim().to_string());
            }
        }
    }
    None
}

fn num_field(v: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    for k in keys {
        if let Some(x) = v.get(k) {
            let n = match x {
                serde_json::Value::Number(n) => n.as_f64(),
                serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
                _ => None,
            };
            if let Some(n) = n {
                // > 10000 视为毫秒（3 小时的歌才 10800 秒，不会误判）。
                return Some(if n > 10000.0 { n.round() as i64 } else { (n * 1000.0).round() as i64 });
            }
        }
    }
    None
}

fn entry_from_json(v: &serde_json::Value) -> ImportEntry {
    ImportEntry {
        path: str_field(v, &["file_path", "path", "file", "relative_path"]),
        name: str_field(v, &["name", "title", "song", "song_name"]),
        artist: str_field(v, &["artist", "artists", "artist_name", "singer"]),
        duration_ms: num_field(v, &["duration_ms", "duration", "dt"]),
    }
}

fn parse_json(text: &str) -> Result<ParsedPlaylist> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    let mut out = ParsedPlaylist::default();
    if v.is_object() {
        out.name = str_field(&v, &["name", "playlist_name", "title"]);
    }
    let arr = v
        .get("tracks")
        .or_else(|| v.get("songs"))
        .or_else(|| v.get("items"))
        .and_then(|x| x.as_array())
        .or_else(|| v.as_array());
    match arr {
        Some(list) => out.entries = list.iter().map(entry_from_json).collect(),
        None => bail!("JSON 里找不到曲目数组（tracks / songs / items）"),
    }
    Ok(out)
}

fn parse_csv(text: &str) -> ParsedPlaylist {
    let mut out = ParsedPlaylist::default();
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let head = match lines.next() {
        Some(h) => h.split(',').map(|s| s.trim().trim_matches('"').to_ascii_lowercase()).collect::<Vec<_>>(),
        None => return out,
    };
    let col = |names: &[&str]| -> Option<usize> {
        names.iter().find_map(|n| head.iter().position(|h| h == n))
    };
    let i_name = col(&["name", "title", "song", "曲名"]);
    let i_artist = col(&["artist", "artists", "艺人", "歌手"]);
    let i_path = col(&["path", "file", "file_path", "路径"]);
    let i_dur = col(&["duration", "duration_ms", "时长"]);
    for line in lines {
        let cells = line.split(',').map(|s| s.trim().trim_matches('"')).collect::<Vec<_>>();
        out.entries.push(ImportEntry {
            name: i_name.and_then(|i| cells.get(i)).map(|s| s.to_string()).filter(|s| !s.is_empty()),
            artist: i_artist.and_then(|i| cells.get(i)).map(|s| s.to_string()).filter(|s| !s.is_empty()),
            path: i_path.and_then(|i| cells.get(i)).map(|s| s.to_string()).filter(|s| !s.is_empty()),
            duration_ms: i_dur
                .and_then(|i| cells.get(i))
                .and_then(|s| s.parse::<f64>().ok())
                .map(|n| if n > 10000.0 { n as i64 } else { (n * 1000.0) as i64 }),
        });
    }
    out
}

#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    /// 实际写入列表的曲目数。
    pub matched: usize,
    pub unmatched: Vec<String>,
    /// ① local_id 级重复（保留首次出现位置）。
    pub duplicate_ids: Vec<String>,
    /// ② 内容级重复（文件名 + 艺人 + 时长），逐条列出，不静默丢弃。
    pub content_duplicates: Vec<String>,
    /// 命中多个本地副本：取第一个并报告歧义。
    pub ambiguous: Vec<String>,
}

#[derive(Debug)]
pub struct Resolved {
    pub local_ids: Vec<i64>,
    pub report: ImportReport,
}

#[derive(Debug, Clone)]
struct Candidate {
    local_id: i64,
    name: String,
    artist: String,
    duration_ms: i64,
    file_path: String,
}

fn norm_path(p: &str) -> String {
    p.replace('\\', "/").to_ascii_lowercase()
}

fn basename_stem(p: &str) -> String {
    let name = p.rsplit(['/', '\\']).next().unwrap_or(p);
    let stem = name.rsplit_once('.').map(|(a, _)| a).unwrap_or(name);
    stem.trim().to_ascii_lowercase()
}

fn key2(a: &str, b: &str) -> (String, String) {
    (a.trim().to_ascii_lowercase(), b.trim().to_ascii_lowercase())
}

/// 把外部条目匹配成本地 `local_id[]`，并在**写 DB 之前**完成两级去重。
pub fn resolve(conn: &Connection, root: &Path, parsed: &ParsedPlaylist) -> Result<Resolved> {
    let mut stmt = conn.prepare(
        "SELECT local_id, COALESCE(name,''), COALESCE(artist,''),
                COALESCE(duration_ms,0), file_path
         FROM download_records
         WHERE status='downloaded' AND file_path IS NOT NULL",
    )?;
    let candidates: Vec<Candidate> = stmt
        .query_map([], |r| {
            Ok(Candidate {
                local_id: r.get(0)?,
                name: r.get(1)?,
                artist: r.get(2)?,
                duration_ms: r.get(3)?,
                file_path: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    let mut by_path: HashMap<String, Vec<usize>> = HashMap::new();
    let mut by_title_artist: HashMap<(String, String), Vec<usize>> = HashMap::new();
    let mut by_basename_artist: HashMap<(String, String), Vec<usize>> = HashMap::new();
    let mut by_title: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, c) in candidates.iter().enumerate() {
        by_path.entry(norm_path(&c.file_path)).or_default().push(i);
        by_title_artist
            .entry(key2(&c.name, &c.artist))
            .or_default()
            .push(i);
        by_basename_artist
            .entry(key2(&basename_stem(&c.file_path), &c.artist))
            .or_default()
            .push(i);
        by_title.entry(c.name.trim().to_ascii_lowercase()).or_default().push(i);
    }

    let root_abs = root.to_string_lossy().replace('\\', "/");
    let mut report = ImportReport::default();
    let mut seen_ids: HashSet<i64> = HashSet::new();
    let mut seen_content: HashMap<String, String> = HashMap::new();
    let mut local_ids: Vec<i64> = Vec::new();

    for e in &parsed.entries {
        let mut hits: Vec<usize> = Vec::new();
        // ① 相对路径 → ② 绝对路径（两者都归一成 DB 里的绝对路径再查）。
        if let Some(p) = e.path.as_deref() {
            let p = p.trim();
            let is_abs = Path::new(p).is_absolute() || p.contains(':');
            let abs = if is_abs {
                p.replace('\\', "/")
            } else {
                format!("{}/{}", root_abs.trim_end_matches('/'), p.replace('\\', "/"))
            };
            if let Some(v) = by_path.get(&norm_path(&abs)) {
                hits = v.clone();
            }
        }
        // ③ 文件名 + 艺人
        if hits.is_empty() {
            if let (Some(p), Some(a)) = (e.path.as_deref(), e.artist.as_deref()) {
                if let Some(v) = by_basename_artist.get(&key2(&basename_stem(p), a)) {
                    hits = v.clone();
                }
            }
        }
        // ④ 标签：曲名 + 艺人；缺艺人时退化为仅曲名。
        if hits.is_empty() {
            if let Some(n) = e.name.as_deref() {
                if let Some(a) = e.artist.as_deref() {
                    if let Some(v) = by_title_artist.get(&key2(n, a)) {
                        hits = v.clone();
                    }
                }
                if hits.is_empty() {
                    if let Some(v) = by_title.get(&n.trim().to_ascii_lowercase()) {
                        hits = v.clone();
                    }
                }
            }
        }
        if hits.is_empty() {
            report.unmatched.push(format!("未匹配：「{}」", e.label()));
            continue;
        }
        if hits.len() > 1 {
            report.ambiguous.push(format!(
                "「{}」匹配到 {} 个本地副本，取第一个：{}",
                e.label(),
                hits.len(),
                candidates[hits[0]].file_path
            ));
        }
        let c = &candidates[hits[0]];
        if !seen_ids.insert(c.local_id) {
            report
                .duplicate_ids
                .push(format!("跳过重复曲目「{}」（local_id={}）", e.label(), c.local_id));
            continue;
        }
        // 内容级去重：文件名（去扩展名）+ 艺人 + 时长（秒）。
        let ckey = format!(
            "{}|{}|{}",
            basename_stem(&c.file_path),
            c.artist.trim().to_ascii_lowercase(),
            c.duration_ms / 1000
        );
        if let Some(first) = seen_content.get(&ckey) {
            report.content_duplicates.push(format!(
                "跳过「{}」：与「{}」内容重复（文件名+艺人+时长）；若属不同录音版本请手动加回",
                e.label(),
                first
            ));
            continue;
        }
        seen_content.insert(ckey, e.label());
        local_ids.push(c.local_id);
    }
    report.matched = local_ids.len();
    Ok(Resolved { local_ids, report })
}

/// 供 handler 生成不与现有列表重名的名称（同名时追加「 (导入 N)」）。
pub fn unique_name(conn: &Connection, base: &str) -> Result<String> {
    if base.trim().is_empty() {
        bail!("导入的播放列表名称为空");
    }
    let exists = |n: &str| -> Result<bool> {
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM playlists WHERE lower(name)=lower(?1))",
            rusqlite::params![n],
            |r| r.get::<_, bool>(0),
        )?)
    };
    if !exists(base)? {
        return Ok(base.to_string());
    }
    for i in 1..1000 {
        let cand = format!("{} (导入{})", base, if i == 1 { String::new() } else { format!(" {i}") });
        if !exists(&cand)? {
            return Ok(cand);
        }
    }
    bail!("无法生成唯一播放列表名称")
}

/// 导出目录 + 文件名：`<dir>/<stem>.json` 与 `<dir>/<stem>.m3u8`。
pub fn export_paths(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    let stem = safe_file_stem(name);
    (dir.join(format!("{stem}.json")), dir.join(format!("{stem}.m3u8")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stem_is_safe() {
        assert_eq!(safe_file_stem("我的/歌单:?"), "我的_歌单__");
        assert_eq!(safe_file_stem("..."), "playlist");
        assert_eq!(safe_file_stem("  A B  "), "A B");
    }

    #[test]
    fn m3u8_parse_and_backup_projection() {
        let txt = "#EXTM3U\n#PLAYLIST:测试列表\n#EXTINF:183,Alan Walker - Faded\nAlan Walker/Different World/Faded.flac\n#EXTINF:60,Solo\nsolo.mp3\n";
        let p = parse_m3u(txt);
        assert_eq!(p.name.as_deref(), Some("测试列表"));
        assert_eq!(p.entries.len(), 2);
        assert_eq!(p.entries[0].artist.as_deref(), Some("Alan Walker"));
        assert_eq!(p.entries[0].name.as_deref(), Some("Faded"));
        assert_eq!(p.entries[0].duration_ms, Some(183000));
        assert_eq!(p.entries[1].name.as_deref(), Some("Solo"));
    }

    #[test]
    fn json_backup_roundtrip_shape() {
        let backup: PlaylistBackup = serde_json::from_str(
            r#"{"format":"kqk-playlist","version":1,"name":"L","library_name":"K","exported_at":1,
                "tracks":[{"position":1,"local_id":9,"name":"曲","artist":"人","album":null,
                           "duration_ms":1000,"format":"flac","quality_tier":null,"file_size":10,
                           "file_path":"人/辑/曲.flac","imported_at":null,"created_at":null}]}"#,
        )
        .unwrap();
        assert_eq!(backup.tracks[0].local_id, 9);
        let p = parse_json(&serde_json::to_string(&backup).unwrap()).unwrap();
        assert_eq!(p.name.as_deref(), Some("L"));
        assert_eq!(p.entries.len(), 1);
        assert_eq!(p.entries[0].path.as_deref(), Some("人/辑/曲.flac"));
    }

    #[test]
    fn resolve_matches_by_four_keys_and_dedups_before_db() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE download_records(local_id INTEGER PRIMARY KEY, name TEXT, artist TEXT,
             duration_ms INTEGER, file_path TEXT, status TEXT);",
        )
        .unwrap();
        let ins = |id: i64, n: &str, a: &str, d: i64, p: &str| {
            c.execute(
                "INSERT INTO download_records VALUES(?1,?2,?3,?4,?5,'downloaded')",
                rusqlite::params![id, n, a, d, p],
            )
            .unwrap();
        };
        ins(1, "Faded", "Alan Walker", 180000, "D:/music/Alan/Faded.flac");
        // 同一首歌的 flac / mp3 两个副本：内容级去重应把它判为重复并写进报告。
        ins(2, "Faded", "Alan Walker", 180000, "D:/music/Copy/Faded.mp3");
        ins(3, "Spectre", "Iselin", 200000, "D:/music/I/Spectre.flac");

        let ent = |path: Option<&str>, name: Option<&str>, artist: Option<&str>| ImportEntry {
            path: path.map(str::to_string),
            name: name.map(str::to_string),
            artist: artist.map(str::to_string),
            duration_ms: None,
        };
        let parsed = ParsedPlaylist {
            name: None,
            entries: vec![
                ent(Some("Alan/Faded.flac"), None, None),        // ① 相对路径 → id 1
                ent(Some("Copy/Faded.mp3"), None, None),         // ② 内容级重复 → 报告
                ent(Some("Alan/Faded.flac"), None, None),        // ① local_id 重复 → 报告
                ent(None, Some("不存在"), Some("无人")),          // 未匹配 → 报告
                ent(None, Some("Spectre"), Some("Iselin")),      // ④ 标签 → id 3
            ],
        };
        let r = resolve(&c, Path::new("D:/music"), &parsed).unwrap();
        assert_eq!(r.local_ids, vec![1, 3], "去重后才写 DB 的 id 序列");
        assert_eq!(r.report.matched, 2);
        assert_eq!(r.report.unmatched.len(), 1);
        assert_eq!(r.report.duplicate_ids.len(), 1);
        assert_eq!(r.report.content_duplicates.len(), 1, "内容级重复必须逐条列出");

        // 一对多：取第一个并报告歧义。
        let p2 = ParsedPlaylist {
            name: None,
            entries: vec![ent(None, Some("Faded"), Some("Alan Walker"))],
        };
        let r2 = resolve(&c, Path::new("D:/music"), &p2).unwrap();
        assert_eq!(r2.local_ids.len(), 1);
        assert_eq!(r2.report.ambiguous.len(), 1, "多副本命中必须报告歧义");
    }

    #[test]
    fn relative_path_uses_library_root() {
        assert_eq!(
            relative_path(Path::new("D:/music"), Path::new("D:/music/A/B.flac")),
            "A/B.flac"
        );
        assert_eq!(
            relative_path(Path::new("D:\\music"), Path::new("D:\\music\\A\\B.flac")),
            "A/B.flac"
        );
        assert_eq!(
            relative_path(Path::new("D:/music"), Path::new("E:/other/A.flac")),
            "E:/other/A.flac"
        );
    }
}
