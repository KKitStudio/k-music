//! SQLite 存储（rusqlite，bundled）。对应 docs/03 §2 的表结构草案。
use crate::config::Config;
use crate::Result;
use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, params};
/// 再导出连接类型，便于 CLI/GUI 在不直接依赖 rusqlite 的情况下引用。
pub use rusqlite::Connection as SqliteConnection;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TrackView {
    /// 下载记录自增主键，恒有值；播放/详情/封面一律以此检索（本地文件 song_id 可空）。
    pub local_id: i64,
    pub song_id: Option<i64>,
    pub name: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub format: Option<String>,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub bitrate: Option<i64>,
    pub channels: Option<i64>,
    pub duration_ms: Option<i64>,
    pub quality_tier: Option<String>,
    pub file_size: i64,
    pub file_path: String,
    /// 首次入库（导入）时刻，UNIX **毫秒**；更新元数据时不覆盖，保留「首次加入曲库」的时间。
    /// 旧库迁移时由 `downloaded_at` / `file_mtime` 回填，可能为 NULL。
    pub imported_at: Option<i64>,
    /// 音频文件的**文件系统创建时间**，UNIX 毫秒。
    /// Windows 取 birth time；某些平台拿不到创建时间时退化为文件修改时间。
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ScanStats {
    pub scanned: u32,
    pub inserted: u32,
    pub updated: u32,
    /// 本次扫描清理的孤儿记录数（磁盘上已不存在的文件）。
    pub removed: u32,
    /// 扫描发现的散落 .ncm 文件（不在 ing/ncm/pendingNcm 等暂存目录内）。
    /// 由前端弹窗询问是否移入 download 按下载逻辑处理（有工具即转换入库）。
    pub loose_ncm: Vec<String>,
}

/// 按 config 初始化（打开连接 + 建表）。开启外键约束。
pub fn open(cfg: &Config) -> Result<Connection> {
    std::fs::create_dir_all(&cfg.paths.app_data)?;
    let db_path = cfg.paths.app_data.join("library.db");
    let conn = Connection::open(&db_path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<()> {
    // 迁移：本清理阶段（2026-11）重建了 playlists / playlist_items 表，去掉网易云时代遗留的
    // kind / playlist_id 等列。对已存在的旧结构表（早期开发版/测试库）直接丢弃并重建——
    // 旧 playlists 属于网易云同步，业务已移除，无保留价值；playlist_items 随之级联丢弃。
    let legacy: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('playlists') WHERE name='kind'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if legacy > 0 {
        // 先删子表再删父表（外键开启时顺序敏感）。
        conn.execute_batch(
            "DROP TABLE IF EXISTS playlist_items; DROP TABLE IF EXISTS playlists;",
        )?;
    }
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS download_records (
            local_id      INTEGER PRIMARY KEY AUTOINCREMENT,
            song_id       INTEGER,                -- 可空：保留列（历史/未来关联），本地文件为 NULL
            source_kind   TEXT NOT NULL,
            status        TEXT NOT NULL,
            file_path     TEXT,
            name          TEXT,                   -- 冗余存储，供列表/详情直取（本地文件 song_id 为 NULL）
            artist        TEXT,
            album         TEXT,
            duration_ms   INTEGER,
            format        TEXT,
            sample_rate   INTEGER,
            bit_depth     INTEGER,
            bitrate       INTEGER,
            channels      INTEGER,
            quality_tier  TEXT,
            file_size     INTEGER,
            file_mtime    INTEGER,                -- 增量门控：文件修改时间（秒），用于跳过未变文件的 lofty 抽取
            imported_at   INTEGER,                -- 首次入库时刻（UNIX 毫秒）；UPDATE 不覆盖
            created_at    INTEGER,                -- 音频文件创建时间（UNIX 毫秒）；文件系统 birth time
            fail_reason   TEXT,
            downloaded_at TEXT
        );
        CREATE TABLE IF NOT EXISTS playlists (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            name          TEXT NOT NULL,
            created_at    TEXT DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS playlist_items (
            playlist_id   INTEGER NOT NULL,
            local_id      INTEGER NOT NULL,
            position      INTEGER NOT NULL,
            added_at      TEXT DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (playlist_id, local_id),
            FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_dr_path ON download_records(file_path);
        CREATE INDEX IF NOT EXISTS idx_pi_playlist ON playlist_items(playlist_id);
        "#,
    )?;
    // 增量门控需要 file_mtime 列；对已有库做无破坏迁移（CREATE TABLE IF NOT EXISTS 不会补列）。
    let has_mtime: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('download_records') WHERE name='file_mtime'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);
    if !has_mtime {
        conn.execute_batch("ALTER TABLE download_records ADD COLUMN file_mtime INTEGER;")?;
    }
    // 时间列（导入时间 / 文件创建时间）：同样对已有库无破坏迁移。
    // 旧记录没有这两列 → 回填近似值，保证升级后列表立刻有内容可显示：
    //   imported_at ← downloaded_at（datetime('now') 是 UTC 文本，strftime('%s') 按 UTC 解析），
    //                 缺失时退到 file_mtime；
    //   created_at  ← file_mtime（文件修改时间，秒）；后续扫描会把门控命中的记录
    //                 逐步校正为真实的文件系统创建时间（见 refresh_created_at）。
    for col in ["imported_at", "created_at"] {
        let has: bool = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM pragma_table_info('download_records') WHERE name='{col}'"
                ),
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if has {
            continue;
        }
        conn.execute_batch(&format!(
            "ALTER TABLE download_records ADD COLUMN {col} INTEGER;"
        ))?;
        let backfill = if col == "imported_at" {
            "UPDATE download_records SET imported_at =
                 COALESCE(CAST(strftime('%s', downloaded_at) AS INTEGER) * 1000,
                          file_mtime * 1000)
             WHERE imported_at IS NULL;"
        } else {
            "UPDATE download_records SET created_at = file_mtime * 1000
             WHERE created_at IS NULL;"
        };
        conn.execute_batch(backfill)?;
    }
    Ok(())
}

/// 写入一条本地扫描/导入记录，幂等：同一 file_path 已存在则仅更新音质元数据。
/// 返回 true 表示新插入，false 表示已存在并更新。file_mtime 用于增量门控。
///
/// `created_at_ms` = 音频文件的文件系统创建时间（UNIX 毫秒）；`imported_at` 只在**新增**时
/// 由 SQL 取当前时刻写入，更新元数据时不覆盖 → 始终保留「首次加入曲库」的时间。
pub fn upsert_local_record(
    conn: &Connection,
    file_path: &str,
    name: &str,
    artist: Option<&str>,
    album: Option<&str>,
    duration_ms: Option<i64>,
    format: Option<&str>,
    sample_rate: Option<i64>,
    bit_depth: Option<i64>,
    bitrate: Option<i64>,
    channels: Option<i64>,
    quality_tier: Option<&str>,
    file_size: i64,
    file_mtime: i64,
    created_at_ms: Option<i64>,
) -> Result<bool> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT local_id FROM download_records WHERE file_path=?1 AND status='downloaded'",
            params![file_path],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(rec_id) = existing {
        // 已存在：仅更新元数据（本地文件不再补建 songs 行，song_id 保持 NULL）。
        // imported_at 刻意不进 SET 列表：曲目重扫/标签修正不应改写首次导入时间。
        conn.execute(
            "UPDATE download_records SET name=?1,artist=?2,album=?3,duration_ms=?4,format=?5,
                sample_rate=?6,bit_depth=?7,bitrate=?8,channels=?9,quality_tier=?10,
                file_size=?11,file_mtime=?12,created_at=?13
             WHERE local_id=?14",
            params![
                name, artist, album, duration_ms, format, sample_rate, bit_depth, bitrate,
                channels, quality_tier, file_size, file_mtime, created_at_ms, rec_id
            ],
        )?;
        Ok(false)
    } else {
        // 新增：song_id 显式 NULL（本地文件不关联网易云）。
        conn.execute(
            "INSERT INTO download_records(song_id,source_kind,status,name,artist,album,duration_ms,
                file_path,format,sample_rate,bit_depth,bitrate,channels,quality_tier,file_size,
                file_mtime,imported_at,created_at,downloaded_at)
             VALUES(NULL,'local_scan','downloaded',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,
                    CAST(strftime('%s','now') AS INTEGER)*1000,?14,datetime('now'))",
            params![
                name, artist, album, duration_ms, file_path, format, sample_rate, bit_depth,
                bitrate, channels, quality_tier, file_size, file_mtime, created_at_ms
            ],
        )?;
        Ok(true)
    }
}

/// 只在该值与库中不同（或库中为空）时刷新文件创建时间。
///
/// 增量门控命中「文件未变更」时也会调用，用于把旧库迁移时回填的 `file_mtime` 近似值
/// 逐步校正为真实的文件系统创建时间；值没变化则不会产生任何写入。
pub fn refresh_created_at(conn: &Connection, file_path: &str, created_ms: i64) -> Result<()> {
    conn.execute(
        "UPDATE download_records SET created_at=?1
         WHERE file_path=?2 AND status='downloaded'
           AND (created_at IS NULL OR created_at<>?1)",
        params![created_ms, file_path],
    )?;
    Ok(())
}

/// 更新某曲目的文件落点（切换文件结构方案重组时调用，docs/05 §4c）。
pub fn update_file_path(conn: &Connection, local_id: i64, file_path: &str) -> Result<()> {
    conn.execute(
        "UPDATE download_records SET file_path=?1 WHERE local_id=?2",
        params![file_path, local_id],
    )?;
    Ok(())
}

/// 行 → TrackView 映射（多个查询复用，保证列序一致，避免各查询各自维护字段顺序）。
fn track_from_row(row: &rusqlite::Row) -> rusqlite::Result<TrackView> {
    Ok(TrackView {
        local_id: row.get(0)?,
        song_id: row.get(1)?,
        name: row.get(2)?,
        artist: row.get(3)?,
        album: row.get(4)?,
        format: row.get(5)?,
        sample_rate: row.get(6)?,
        bit_depth: row.get(7)?,
        bitrate: row.get(8)?,
        channels: row.get(9)?,
        duration_ms: row.get(10)?,
        quality_tier: row.get(11)?,
        file_size: row.get(12)?,
        file_path: row.get(13)?,
        imported_at: row.get(14)?,
        created_at: row.get(15)?,
    })
}

/// 查询曲库中可播放曲目（来自 download_records 中 status='downloaded' 且有文件路径者）。
/// 直接读 download_records 的冗余列（本地文件 song_id 为 NULL，无需关联其它表）。
/// 支持按关键词（曲名/艺人/专辑）过滤。
pub fn list_tracks(conn: &Connection, q: Option<&str>) -> Result<Vec<TrackView>> {
    let mut sql = String::from(
        "SELECT dr.local_id, dr.song_id, dr.name, dr.artist, dr.album, dr.format, dr.sample_rate,
                dr.bit_depth, dr.bitrate, dr.channels, dr.duration_ms, dr.quality_tier, dr.file_size, dr.file_path, dr.imported_at, dr.created_at
         FROM download_records dr
         WHERE dr.status='downloaded' AND dr.file_path IS NOT NULL",
    );
    if let Some(kw) = q {
        if !kw.trim().is_empty() {
            sql.push_str(" AND (dr.name LIKE ?1 OR dr.artist LIKE ?1 OR dr.album LIKE ?1)");
        }
    }
    sql.push_str(" ORDER BY dr.artist, dr.name LIMIT 2000");

    let map = track_from_row;

    let mut out = Vec::new();
    if let Some(kw) = q {
        if !kw.trim().is_empty() {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![format!("%{}%", kw)], map)?;
            for r in rows {
                out.push(r?);
            }
        } else {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map((), map)?;
            for r in rows {
                out.push(r?);
            }
        }
    } else {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map((), map)?;
        for r in rows {
            out.push(r?);
        }
    }
    Ok(out)
}

/// 取某本地记录的当前可播放文件路径（播放/封面服务用，按 local_id）。
pub fn lookup_path_by_local_id(conn: &Connection, local_id: i64) -> Result<Option<String>> {
    let p: Option<String> = conn
        .query_row(
            "SELECT file_path FROM download_records WHERE local_id=?1 AND status='downloaded'
             AND file_path IS NOT NULL ORDER BY local_id DESC LIMIT 1",
            params![local_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(p)
}

/// 取某曲目的元数据（供详情面板），按 local_id 检索。
pub fn get_track_by_local_id(conn: &Connection, local_id: i64) -> Result<Option<TrackView>> {
    let opt: Option<TrackView> = conn
        .query_row(
            "SELECT dr.local_id, dr.song_id, dr.name, dr.artist, dr.album, dr.format, dr.sample_rate,
                    dr.bit_depth, dr.bitrate, dr.channels, dr.duration_ms, dr.quality_tier, dr.file_size, dr.file_path, dr.imported_at, dr.created_at
             FROM download_records dr
             WHERE dr.local_id=?1 AND dr.status='downloaded' AND dr.file_path IS NOT NULL
             ORDER BY dr.local_id DESC LIMIT 1",
            params![local_id],
            track_from_row,
        )
        .optional()?;
    Ok(opt)
}

/// 孤儿清理：删除 download_records 中 status='downloaded' 且 file_path 指向的文件在磁盘上
/// 已不存在的记录。用于检测用户在工具外删除/移动的文件，使曲库索引与磁盘一致。
///
/// 直接校验磁盘存在性（而非按路径字符串比对），规避 Windows 下大小写/分隔符差异导致的误判。
pub fn prune_missing(conn: &Connection) -> Result<u32> {
    let paths: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT file_path FROM download_records WHERE status='downloaded' AND file_path IS NOT NULL",
        )?;
        let mut out = Vec::new();
        let rows = stmt.query_map((), |r| r.get::<_, String>(0))?;
        for r in rows {
            out.push(r?);
        }
        out
    };
    let mut removed = 0u32;
    for p in &paths {
        if std::fs::metadata(p).is_err() {
            conn.execute(
                "DELETE FROM download_records WHERE file_path=?1 AND status='downloaded'",
                params![p],
            )?;
            removed += 1;
        }
    }
    // 曲目记录消失后，playlist_items 里指向它的条目就成孤儿：
    //   - 左侧 track_count 用 COUNT(*) 仍计入，右侧 get_playlist_tracks 用 JOIN 取不到 → 计数与列表不符；
    //   - 播放队列遇到失效 local_id 会中断（FR-18 要求跳过并提示）。
    // 这里按 NOT EXISTS 全量清扫，顺带修掉旧版本只删 download_records 留下的历史孤儿。
    // playlist_items 对 download_records 没有外键，必须显式清理。
    conn.execute(
        "DELETE FROM playlist_items
         WHERE NOT EXISTS (
             SELECT 1 FROM download_records dr WHERE dr.local_id = playlist_items.local_id
         )",
        [],
    )?;
    Ok(removed)
}

/// 取某 file_path 已缓存的 (file_size, file_mtime)，供扫描增量门控判断是否需重新抽取音质元数据。
/// file_mtime 可能为 NULL（旧记录），此时返回 None 的 mtime，调用方应视为「未缓存」而下探抽取。
pub fn get_cached_file_info(conn: &Connection, file_path: &str) -> Result<Option<(i64, Option<i64>)>> {
    let opt = conn
        .query_row(
            "SELECT file_size, file_mtime FROM download_records WHERE file_path=?1 AND status='downloaded'",
            params![file_path],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .optional()?;
    Ok(opt)
}

/// 播放列表（本地曲库内用户自建，local_id 体系，与网易云同步无关）。
#[derive(Debug, Clone, Serialize)]
pub struct PlaylistView {
    pub id: i64,
    pub name: String,
    pub track_count: i64,
}

/// 播放列表内的曲目（含排序位与曲目元数据快照）。
#[derive(Debug, Clone, Serialize)]
pub struct PlaylistItemView {
    pub position: i64,
    pub local_id: i64,
    pub name: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub format: Option<String>,
    pub duration_ms: Option<i64>,
    pub quality_tier: Option<String>,
    pub file_size: i64,
    pub file_path: String,
    pub imported_at: Option<i64>,
    pub created_at: Option<i64>,
}

/// 播放列表名称上限（按 Unicode 字符计）。
pub const MAX_PLAYLIST_NAME_CHARS: usize = 60;

/// 名称规范：去首尾空白、非空、长度受限。
fn normalize_playlist_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("播放列表名称不能为空");
    }
    if name.chars().count() > MAX_PLAYLIST_NAME_CHARS {
        bail!("播放列表名称不能超过 {} 个字符", MAX_PLAYLIST_NAME_CHARS);
    }
    Ok(name.to_string())
}

/// 同名检查（大小写不敏感）；`except_id` 用于重命名时排除自身。
fn ensure_playlist_name_available(conn: &Connection, name: &str, except_id: i64) -> Result<()> {
    let dup: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM playlists WHERE lower(name)=lower(?1) AND id<>?2)",
        params![name, except_id],
        |r| r.get(0),
    )?;
    if dup {
        bail!("已存在同名播放列表：{name}");
    }
    Ok(())
}

/// 播放列表是否存在（供删除 / 加曲 / 移除给出明确错误，而非静默成功）。
pub fn playlist_exists(conn: &Connection, playlist_id: i64) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM playlists WHERE id=?1)",
        params![playlist_id],
        |r| r.get::<_, bool>(0),
    )?)
}

/// 新建播放列表，返回其自增 id。名称非空 / ≤60 字符 / 不允许同名。
pub fn create_playlist(conn: &Connection, name: &str) -> Result<i64> {
    let name = normalize_playlist_name(name)?;
    ensure_playlist_name_available(conn, &name, -1)?;
    conn.execute("INSERT INTO playlists(name) VALUES(?1)", params![name])?;
    Ok(conn.last_insert_rowid())
}

/// 重命名播放列表（校验规则与新建一致）。
pub fn rename_playlist(conn: &Connection, playlist_id: i64, name: &str) -> Result<()> {
    let name = normalize_playlist_name(name)?;
    ensure_playlist_name_available(conn, &name, playlist_id)?;
    let n = conn.execute(
        "UPDATE playlists SET name=?1 WHERE id=?2",
        params![name, playlist_id],
    )?;
    if n == 0 {
        bail!("播放列表不存在或已被删除");
    }
    Ok(())
}

/// 列出全部播放列表（含各自曲目数）。
pub fn list_playlists(conn: &Connection) -> Result<Vec<PlaylistView>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, (SELECT COUNT(*) FROM playlist_items WHERE playlist_id=playlists.id)
         FROM playlists ORDER BY id",
    )?;
    let rows = stmt.query_map((), |r| {
        Ok(PlaylistView {
            id: r.get(0)?,
            name: r.get(1)?,
            track_count: r.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 删除播放列表（级联删除其条目）。
pub fn delete_playlist(conn: &Connection, playlist_id: i64) -> Result<()> {
    let n = conn.execute("DELETE FROM playlists WHERE id=?1", params![playlist_id])?;
    if n == 0 {
        bail!("播放列表不存在或已被删除");
    }
    Ok(())
}

/// 取播放列表内的曲目（按 position 排序），关联 download_records 取元数据。
pub fn get_playlist_tracks(conn: &Connection, playlist_id: i64) -> Result<Vec<PlaylistItemView>> {
    let mut stmt = conn.prepare(
        "SELECT pi.position, dr.local_id, dr.name, dr.artist, dr.album, dr.format,
                dr.duration_ms, dr.quality_tier, dr.file_size, dr.file_path, dr.imported_at, dr.created_at
         FROM playlist_items pi
         JOIN download_records dr ON dr.local_id = pi.local_id
         WHERE pi.playlist_id=?1
         ORDER BY pi.position",
    )?;
    let rows = stmt.query_map(params![playlist_id], |r| {
        Ok(PlaylistItemView {
            position: r.get(0)?,
            local_id: r.get(1)?,
            name: r.get(2)?,
            artist: r.get(3)?,
            album: r.get(4)?,
            format: r.get(5)?,
            duration_ms: r.get(6)?,
            quality_tier: r.get(7)?,
            file_size: r.get(8)?,
            file_path: r.get(9)?,
            imported_at: r.get(10)?,
            created_at: r.get(11)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 将曲目加入播放列表（追加到末尾）；已存在则忽略（幂等）。
/// 主键 `(playlist_id, local_id)` 禁止同一曲目重复出现（FR-19 已拍板，不做破坏性迁移）。
pub fn add_to_playlist(conn: &Connection, playlist_id: i64, local_id: i64) -> Result<()> {
    if !playlist_exists(conn, playlist_id)? {
        bail!("播放列表不存在或已被删除");
    }
    let max_pos: Option<i64> = conn
        .query_row(
            "SELECT MAX(position) FROM playlist_items WHERE playlist_id=?1",
            params![playlist_id],
            |r| r.get::<_, Option<i64>>(0),
        )?;
    let pos = max_pos.unwrap_or(0) + 1;
    conn.execute(
        "INSERT OR IGNORE INTO playlist_items(playlist_id, local_id, position) VALUES(?1,?2,?3)",
        params![playlist_id, local_id, pos],
    )?;
    Ok(())
}

/// 从播放列表移除曲目。
pub fn remove_from_playlist(conn: &Connection, playlist_id: i64, local_id: i64) -> Result<()> {
    let n = conn.execute(
        "DELETE FROM playlist_items WHERE playlist_id=?1 AND local_id=?2",
        params![playlist_id, local_id],
    )?;
    if n == 0 {
        bail!("该曲目不在此播放列表中");
    }
    Ok(())
}

/// 在列表内上移 / 下移一首（`delta = -1` 上移 / `+1` 下移）。
///
/// `position` 在移除曲目后会留下空洞，所以按「当前有序序列」定位相邻项并交换两者的
/// `position` 值，而不是简单 `position ± 1`。返回 `false` 表示已在边界或曲目不在列表内，
/// 属于 UI 可静默忽略的情况，不报错。
pub fn move_playlist_item(
    conn: &Connection,
    playlist_id: i64,
    local_id: i64,
    delta: i64,
) -> Result<bool> {
    if delta != -1 && delta != 1 {
        return Ok(false);
    }
    let rows: Vec<(i64, i64)> = {
        let mut stmt = conn.prepare(
            "SELECT local_id, position FROM playlist_items
             WHERE playlist_id=?1 ORDER BY position, local_id",
        )?;
        let rows = stmt
            .query_map(params![playlist_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let idx = match rows.iter().position(|(id, _)| *id == local_id) {
        Some(i) => i as i64,
        None => return Ok(false),
    };
    let target = idx + delta;
    if target < 0 || target >= rows.len() as i64 {
        return Ok(false);
    }
    let (_, own_pos) = rows[idx as usize];
    let (other_id, other_pos) = rows[target as usize];
    if own_pos == other_pos {
        return Ok(false);
    }
    // 事务内交换，避免中途失败留下两行同 position 的不确定顺序。
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE playlist_items SET position=?1 WHERE playlist_id=?2 AND local_id=?3",
        params![other_pos, playlist_id, local_id],
    )?;
    tx.execute(
        "UPDATE playlist_items SET position=?1 WHERE playlist_id=?2 AND local_id=?3",
        params![own_pos, playlist_id, other_id],
    )?;
    tx.commit()?;
    Ok(true)
}

/// 分组视图（专辑 / 歌手）：分组名称 + 组内曲目数。
/// 用于左侧导航二级列表。NULL 或空字符串折叠为统一的占位名称，便于浏览未标注元数据的曲目。
#[derive(Debug, Clone, Serialize)]
pub struct GroupView {
    pub name: String,
    pub track_count: i64,
}

/// 列出全部专辑（按名称分组，含各专辑曲目数），NULL/空 album 折叠为「未知专辑」。
/// 排序：按专辑名称字典序。
pub fn list_albums(conn: &Connection) -> Result<Vec<GroupView>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(album,''),'未知专辑') AS album, COUNT(*)
         FROM download_records
         WHERE status='downloaded' AND file_path IS NOT NULL
         GROUP BY COALESCE(NULLIF(album,''),'未知专辑')
         ORDER BY album",
    )?;
    let rows = stmt.query_map((), |r| {
        Ok(GroupView {
            name: r.get(0)?,
            track_count: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 列出全部歌手（按整字段分组，含各歌手曲目数），NULL/空 artist 折叠为「未知艺人」。
/// 注：艺人字段可能含多人协作分隔符（如「A / B」），此处按整字段分组，不做拆分。
pub fn list_artists(conn: &Connection) -> Result<Vec<GroupView>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(artist,''),'未知艺人') AS artist, COUNT(*)
         FROM download_records
         WHERE status='downloaded' AND file_path IS NOT NULL
         GROUP BY COALESCE(NULLIF(artist,''),'未知艺人')
         ORDER BY artist",
    )?;
    let rows = stmt.query_map((), |r| {
        Ok(GroupView {
            name: r.get(0)?,
            track_count: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 取某专辑内曲目（按名称排序）；`album` 传入「未知专辑」时匹配 album 为 NULL/空的曲目。
pub fn get_tracks_by_album(conn: &Connection, album: &str) -> Result<Vec<TrackView>> {
    let mut stmt = conn.prepare(
        "SELECT dr.local_id, dr.song_id, dr.name, dr.artist, dr.album, dr.format, dr.sample_rate,
                dr.bit_depth, dr.bitrate, dr.channels, dr.duration_ms, dr.quality_tier, dr.file_size, dr.file_path, dr.imported_at, dr.created_at
         FROM download_records dr
         WHERE dr.status='downloaded' AND dr.file_path IS NOT NULL
           AND COALESCE(NULLIF(dr.album,''),'未知专辑')=?1
         ORDER BY dr.name",
    )?;
    let rows = stmt.query_map(params![album], track_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 取某歌手内曲目（按名称排序）；`artist` 传入「未知艺人」时匹配 artist 为 NULL/空的曲目。
pub fn get_tracks_by_artist(conn: &Connection, artist: &str) -> Result<Vec<TrackView>> {
    let mut stmt = conn.prepare(
        "SELECT dr.local_id, dr.song_id, dr.name, dr.artist, dr.album, dr.format, dr.sample_rate,
                dr.bit_depth, dr.bitrate, dr.channels, dr.duration_ms, dr.quality_tier, dr.file_size, dr.file_path, dr.imported_at, dr.created_at
         FROM download_records dr
         WHERE dr.status='downloaded' AND dr.file_path IS NOT NULL
           AND COALESCE(NULLIF(dr.artist,''),'未知艺人')=?1
         ORDER BY dr.name",
    )?;
    let rows = stmt.query_map(params![artist], track_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod playlist_tests {
    use super::*;
    use rusqlite::Connection;

    /// 构造内存库并建表（含播放列表迁移后的新结构）。
    fn test_conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        init_schema(&c).unwrap();
        c
    }

    #[test]
    fn playlist_full_crud() {
        let c = test_conn();
        // 创建。
        let id = create_playlist(&c, "我的歌单").unwrap();
        assert!(id > 0, "新建播放列表应返回正 id");

        // 列出：初建曲目数为 0。
        let pls = list_playlists(&c).unwrap();
        assert_eq!(pls.len(), 1);
        assert_eq!(pls[0].name, "我的歌单");
        assert_eq!(pls[0].track_count, 0);

        // 准备一条 download_records 行（local_id=10），否则 get_playlist_tracks 的 JOIN 取不到元数据。
        c.execute_batch(
            "INSERT INTO download_records(local_id,song_id,source_kind,status,file_path,name,artist,album,format,duration_ms,file_size) \
             VALUES(10,NULL,'local_scan','downloaded','/x/test.flac','测试曲','测试人','测试辑','flac',180000,5000000)",
        )
        .unwrap();

        // 加入曲目：空列表首次加曲时 MAX(position) 为 NULL，必须正确处理（回归点）。
        add_to_playlist(&c, id, 10).unwrap();
        // 幂等：重复加入同一首应忽略。
        add_to_playlist(&c, id, 10).unwrap();

        let items = get_playlist_tracks(&c, id).unwrap();
        assert_eq!(items.len(), 1, "幂等加曲后应有且仅有一首");
        assert_eq!(items[0].local_id, 10);
        assert_eq!(items[0].name, "测试曲");
        assert_eq!(items[0].position, 1);

        // 列出计数应反映为 1。
        let pls = list_playlists(&c).unwrap();
        assert_eq!(pls[0].track_count, 1);

        // 移除。
        remove_from_playlist(&c, id, 10).unwrap();
        assert_eq!(get_playlist_tracks(&c, id).unwrap().len(), 0);

        // 删除歌单（级联删除条目）。
        delete_playlist(&c, id).unwrap();
        assert_eq!(list_playlists(&c).unwrap().len(), 0);
    }

    #[test]
    fn add_to_empty_playlist_first_position_is_one() {
        let c = test_conn();
        let id = create_playlist(&c, "X").unwrap();
        c.execute_batch(
            "INSERT INTO download_records(local_id,song_id,source_kind,status,file_path,name,file_size) \
             VALUES(7,NULL,'local_scan','downloaded','/x/a.flac','A',3000000)",
        )
        .unwrap();
        add_to_playlist(&c, id, 7).unwrap();
        let items = get_playlist_tracks(&c, id).unwrap();
        assert_eq!(items[0].position, 1, "首曲 position 应为 1 而非 0");
    }

    /// 造若干条「磁盘上不存在」的曲目记录，供孤儿清理测试使用。
    fn insert_missing_tracks(c: &Connection, ids: &[i64]) {
        for id in ids {
            c.execute(
                "INSERT INTO download_records(local_id,source_kind,status,file_path,name,file_size)
                 VALUES(?1,'local_scan','downloaded',?2,?3,1000)",
                params![id, format!("/definitely/missing/{id}.flac"), format!("曲{id}")],
            )
            .unwrap();
        }
    }

    #[test]
    fn prune_missing_clears_playlist_orphans() {
        let c = test_conn();
        let id = create_playlist(&c, "孤儿测试").unwrap();
        insert_missing_tracks(&c, &[1, 2, 3]);
        for i in 1..=3 {
            add_to_playlist(&c, id, i).unwrap();
        }
        assert_eq!(list_playlists(&c).unwrap()[0].track_count, 3);

        // 旧缺陷：只删 download_records，playlist_items 留下 3 条孤儿，
        // 左侧计数仍为 3、右侧 JOIN 取不到任何曲目。
        let removed = prune_missing(&c).unwrap();
        assert_eq!(removed, 3, "应清掉 3 条失效曲目记录");
        assert_eq!(list_playlists(&c).unwrap()[0].track_count, 0, "计数必须归零");
        assert!(get_playlist_tracks(&c, id).unwrap().is_empty());

        // 历史孤儿（旧版本遗留，download_records 里已无对应行）也要被清掉。
        c.execute(
            "INSERT INTO playlist_items(playlist_id,local_id,position) VALUES(?1,999,4)",
            params![id],
        )
        .unwrap();
        assert_eq!(list_playlists(&c).unwrap()[0].track_count, 1);
        prune_missing(&c).unwrap();
        assert_eq!(list_playlists(&c).unwrap()[0].track_count, 0, "历史孤儿也要清扫");
    }

    #[test]
    fn playlist_name_constraints_and_rename() {
        let c = test_conn();
        assert!(create_playlist(&c, "   ").is_err(), "空白名称应拒绝");
        assert!(create_playlist(&c, &"长".repeat(61)).is_err(), "超长名称应拒绝");
        let id = create_playlist(&c, "My List").unwrap();
        assert!(create_playlist(&c, "my list").is_err(), "同名（大小写不敏感）应拒绝");
        assert!(create_playlist(&c, "  My List  ").is_err(), "trim 后同名应拒绝");

        // 重命名为自身大小写变体不应误判为冲突。
        rename_playlist(&c, id, "MY LIST").unwrap();
        assert_eq!(list_playlists(&c).unwrap()[0].name, "MY LIST");
        let other = create_playlist(&c, "另一列表").unwrap();
        assert!(rename_playlist(&c, other, "my list").is_err(), "改成他人同名应拒绝");
        assert!(rename_playlist(&c, 9999, "新名").is_err(), "不存在的 id 应报错");
        assert!(rename_playlist(&c, id, &"长".repeat(61)).is_err(), "重命名也受长度约束");
    }

    #[test]
    fn move_playlist_item_swaps_neighbors() {
        let c = test_conn();
        let id = create_playlist(&c, "重排").unwrap();
        insert_missing_tracks(&c, &[1, 2, 3]);
        for i in 1..=3 {
            add_to_playlist(&c, id, i).unwrap();
        }
        let ids = |c: &Connection| {
            get_playlist_tracks(c, id)
                .unwrap()
                .into_iter()
                .map(|t| t.local_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&c), vec![1, 2, 3]);

        assert!(move_playlist_item(&c, id, 3, -1).unwrap(), "第 3 首上移应成功");
        assert_eq!(ids(&c), vec![1, 3, 2]);
        assert!(move_playlist_item(&c, id, 1, 1).unwrap(), "第 1 首下移应成功");
        assert_eq!(ids(&c), vec![3, 1, 2]);
        assert!(!move_playlist_item(&c, id, 3, -1).unwrap(), "首项上移应返回 false");
        assert!(!move_playlist_item(&c, id, 2, 1).unwrap(), "末项下移应返回 false");
        assert!(!move_playlist_item(&c, id, 999, -1).unwrap(), "不在列表内应返回 false");

        // 移除中间项留下 position 空洞后，重排仍按有序序列正确工作。
        remove_from_playlist(&c, id, 1).unwrap();
        assert!(move_playlist_item(&c, id, 2, -1).unwrap());
        assert_eq!(ids(&c), vec![2, 3]);
    }

    #[test]
    fn missing_playlist_id_returns_error() {
        let c = test_conn();
        assert!(delete_playlist(&c, 9999).is_err(), "删除不存在的列表应报错");
        assert!(add_to_playlist(&c, 9999, 1).is_err(), "向不存在的列表加曲应报错");
        let id = create_playlist(&c, "X").unwrap();
        assert!(remove_from_playlist(&c, id, 999).is_err(), "移除不在列表内的曲目应报错");
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;
    use rusqlite::Connection;

    fn test_conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        init_schema(&c).unwrap();
        c
    }

    /// 准备若干 download_records 行（含空 album/artist 以验证占位折叠）。
    fn seed(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO download_records(local_id,song_id,source_kind,status,file_path,name,artist,album,file_size) VALUES \
             (1,NULL,'local_scan','downloaded','/x/1.flac','曲一','周杰伦','范特西',1000), \
             (2,NULL,'local_scan','downloaded','/x/2.flac','曲二','周杰伦','范特西',2000), \
             (3,NULL,'local_scan','downloaded','/x/3.flac','曲三','陈奕迅','U87',3000), \
             (4,NULL,'local_scan','downloaded','/x/4.flac','曲四','','',4000), \
             (5,NULL,'local_scan','downloaded','/x/5.flac','曲五',NULL,NULL,5000)",
        )
        .unwrap();
    }

    #[test]
    fn albums_group_and_member_query() {
        let c = test_conn();
        seed(&c);
        let albums = list_albums(&c).unwrap();
        // 3 个分组：范特西(2) / U87(1) / 未知专辑(2)。
        assert_eq!(albums.len(), 3, "应得到 3 个专辑分组");
        let fan = albums.iter().find(|a| a.name == "范特西").unwrap();
        assert_eq!(fan.track_count, 2);
        let unknown = albums.iter().find(|a| a.name == "未知专辑").unwrap();
        assert_eq!(unknown.track_count, 2);

        // 成员查询：范特西 应包含曲一、曲二。
        let tracks = get_tracks_by_album(&c, "范特西").unwrap();
        assert_eq!(tracks.len(), 2);
        assert!(tracks.iter().all(|t| t.album.as_deref() == Some("范特西")));

        // 成员查询：未知专辑 应包含 file_path 为空 album 的两首（4、5）。
        let tracks = get_tracks_by_album(&c, "未知专辑").unwrap();
        assert_eq!(tracks.len(), 2);
    }

    #[test]
    fn artists_group_and_member_query() {
        let c = test_conn();
        seed(&c);
        let artists = list_artists(&c).unwrap();
        // 3 个分组：周杰伦(2) / 陈奕迅(1) / 未知艺人(2)。
        assert_eq!(artists.len(), 3);
        let jay = artists.iter().find(|a| a.name == "周杰伦").unwrap();
        assert_eq!(jay.track_count, 2);

        let tracks = get_tracks_by_artist(&c, "周杰伦").unwrap();
        assert_eq!(tracks.len(), 2);
        assert!(tracks.iter().all(|t| t.artist.as_deref() == Some("周杰伦")));

        let unknown = get_tracks_by_artist(&c, "未知艺人").unwrap();
        assert_eq!(unknown.len(), 2);
    }

    #[test]
    fn empty_library_yields_no_groups() {
        let c = test_conn();
        assert!(list_albums(&c).unwrap().is_empty());
        assert!(list_artists(&c).unwrap().is_empty());
        assert!(get_tracks_by_album(&c, "范特西").unwrap().is_empty());
    }
}

