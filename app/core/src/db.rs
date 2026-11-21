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
    /// 收藏标记（用户侧「已收藏」）。SQLite 存储为 INTEGER（0/1），读取时转 bool。
    pub favorite: bool,
    /// #28：曲级流派 / 发行年份（音频标签）。空串视为无流派，前端按「空字段不渲染」处理。
    pub genre: Option<String>,
    pub year: Option<i64>,
    /// 歌曲列表「有歌词」标记——与 `lyrics::get_lyrics` 同源：同名 `.lrc` 侧车或音频内嵌歌词命中即为 true。
    pub has_lyrics: bool,
    /// v0.17.7：文件当前不可访问（404 / 扫描判定），UI 以 ⚠ 标记展示；不代表记录被删除。
    pub unavailable: bool,
}

#[derive(Debug, Clone)]
pub struct ScanStats {
    pub scanned: u32,
    pub inserted: u32,
    pub updated: u32,
    /// 本次扫描清理的孤儿记录数（磁盘上已不存在的文件）。
    pub removed: u32,
    /// v0.17.7：本次扫描新标记为不可访问的曲目数（不删记录）。
    pub marked: u32,
    /// v0.17.7：本次扫描复核后恢复可访问的曲目数（清标）。
    pub recovered: u32,
    /// FR-22：本次扫描登记为「重复待决策」的文件数（P1① 静默登记，启动不弹窗）。
    pub held: u32,
    /// 扫描发现的散落 .ncm 文件（不在旧 ing/ncm 暂存目录 / .mt 内）。
    /// 由前端弹窗询问是否处理（v0.17.3：原件不删不挪，有工具才转换入库、无工具跳过）。
    pub loose_ncm: Vec<String>,
}

/// 连接级 PRAGMA：外键约束 + WAL 读并发所需的 busy_timeout。
/// WAL 是库文件级持久设置，写连接打开时开启一次即可；每条连接重设无害。
fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 5000;",
    )?;
    Ok(())
}

/// 按 config 初始化（打开写连接 + 建表）。开启外键约束与 WAL。
pub fn open(cfg: &Config) -> Result<Connection> {
    std::fs::create_dir_all(&cfg.paths.app_data)?;
    let db_path = cfg.paths.app_data.join("library.db");
    let conn = Connection::open(&db_path)?;
    apply_pragmas(&conn)?;
    init_schema(&conn)?;
    Ok(conn)
}

/// 打开只读连接：不跑 init_schema（避免读连接参与 DDL / 破坏性迁移分支），
/// 供纯读 handler 使用。WAL 下读不阻塞写，导入/扫描期间列表与播放可读。
/// 必须在写连接 `open()` 之后调用（库与表已就绪）。
pub fn open_read(cfg: &Config) -> Result<Connection> {
    let db_path = cfg.paths.app_data.join("library.db");
    let conn = Connection::open(&db_path)?;
    apply_pragmas(&conn)?;
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
            downloaded_at TEXT,
            ncm_origin_id TEXT,                  -- v0.14.0：NCM 转换产物的来源曲源 id（转换短路键的一部分）
            ncm_original_name TEXT,              -- v0.14.0：NCM 转换产物的原始 .ncm 文件名（不含路径）
            ncm_original_size INTEGER,           -- v0.17.5：原始 .ncm 文件大小（字节），短路键第三部分
            unavailable INTEGER DEFAULT 0,       -- v0.17.7：文件当前不可访问标记（0=可访问，1=不可访问）
            unavailable_at INTEGER               -- v0.17.7：首次标记为不可访问的时刻（UNIX 毫秒）
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
        -- v0.17.8：NCM 转换短路缓存。与 download_records 的 ncm_origin 三列解耦，
        -- 转换后无论入库还是判重跳过都记一条，避免旧库 NULL 列导致短路永不命中、
        -- 以及「转换→判 Skip→不写标记→重复转换」死循环。主键即短路三元组。
        CREATE TABLE IF NOT EXISTS ncm_processed (
            source_key TEXT NOT NULL,
            ncm_name TEXT NOT NULL,
            ncm_size INTEGER NOT NULL,
            processed_at INTEGER NOT NULL,
            PRIMARY KEY (source_key, ncm_name, ncm_size)
        );
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
    // 收藏标记：无破坏迁移，默认 0（未收藏）。旧记录无需回填。
    let has_fav: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('download_records') WHERE name='favorite'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);
    if !has_fav {
        conn.execute_batch(
            "ALTER TABLE download_records ADD COLUMN favorite INTEGER DEFAULT 0;",
        )?;
    }
    // #28：曲级流派 / 年份，无破坏迁移。旧记录两列保持 NULL，
    // 由扫描增量门控识别为「从未抽取过标签」而下探一次（用户拍板的回填策略）。
    // genre 约定：NULL=从未尝试；''=已尝试但文件无此标签；非空=实际流派。
    for (col, decl) in [("genre", "TEXT"), ("year", "INTEGER")] {
        let has_col: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('download_records') WHERE name=?1",
                params![col],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if !has_col {
            conn.execute_batch(&format!(
                "ALTER TABLE download_records ADD COLUMN {col} {decl};"
            ))?;
        }
    }
    // v0.14.0：NCM 转换来源两列，无破坏迁移。旧记录保持 NULL（非转换产物）。
    // v0.17.5：追加 ncm_original_size，历史转换产物 size 为 NULL —— 查询侧据此降级为
    // 「不跳过」（宁可多转一次），由导入侧转换成功后回填 size 闭环。
    for (col, decl) in [
        ("ncm_origin_id", "TEXT"),
        ("ncm_original_name", "TEXT"),
        ("ncm_original_size", "INTEGER"),
        ("unavailable", "INTEGER DEFAULT 0"),
        ("unavailable_at", "INTEGER"),
    ] {
        let has_col: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('download_records') WHERE name=?1",
                params![col],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if !has_col {
            conn.execute_batch(&format!(
                "ALTER TABLE download_records ADD COLUMN {col} {decl};"
            ))?;
        }
    }
    // v0.14.0：索引必须在补列迁移之后创建——旧库在建表批次时尚无这两列，
    // 提前建索引会报错并跳过整个迁移（由 legacy_schema_migrates_idempotently 守护）。
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_dr_ncm_origin
             ON download_records(ncm_origin_id, ncm_original_name);",
    )?;
    // v0.17.5：三列短路索引。老库的两列索引 IF NOT EXISTS 不会改列，故另建新名索引；
    // 查询 WHERE id+name+size 走此覆盖索引。旧索引保留不删（幂等、无破坏）。
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_dr_ncm_origin_size
             ON download_records(ncm_origin_id, ncm_original_name, ncm_original_size);",
    )?;
    // v0.17.7：不可访问标记的复核扫描/筛选走此索引。
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_dr_unavailable
             ON download_records(unavailable, status);",
    )?;
    Ok(())
}

/// 写入一条本地扫描/导入记录，幂等：同一 file_path 已存在则仅更新音质元数据。
/// 返回 true 表示新插入，false 表示已存在并更新。file_mtime 用于增量门控。
///
/// `created_at_ms` = 音频文件的文件系统创建时间（UNIX 毫秒）；`imported_at` 只在**新增**时
/// 由 SQL 取当前时刻写入，更新元数据时不覆盖 → 始终保留「首次加入曲库」的时间。
/// 「未指定来源」的持久化标记：历史歌曲 / 扫描 / NCM 回填 / 已删除曲源的孤儿归属。
/// 复用既有 `source_kind` 列（该列原本就硬编码为此值），避免 `ALTER TABLE` 与
/// `track_from_row()` 固定列序改动。
pub const UNSPECIFIED_SOURCE: &str = "local_scan";

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
    genre: Option<&str>,
    year: Option<i64>,
) -> Result<bool> {
    upsert_local_record_with_source(
        conn, file_path, name, artist, album, duration_ms, format, sample_rate, bit_depth,
        bitrate, channels, quality_tier, file_size, file_mtime, created_at_ms, genre, year, None,
        None, None,
    )
}

/// 与 `upsert_local_record` 相同，但可显式写曲源归属。
///
/// `source_id` 语义：
/// - `None` = 不改动已有归属（扫描 / 重扫 / 标签修正不得抹掉曲源信息）；
/// - `Some(id)` = 明确设为该曲源；替换决策用 `Some(UNSPECIFIED_SOURCE)` 归「未指定来源」。
///
/// 仅在 INSERT 时缺省回落 `UNSPECIFIED_SOURCE`。UPDATE 用 `COALESCE(?, source_kind)`
/// 一条语句同时满足「不改动」与「修正归属」两种语义，保留 `local_id` 及播放列表引用。
///
/// `ncm_origin`：v0.14.0 NCM 转换产物的来源信息 `(曲源 id, 原始 .ncm 文件名)`。
/// - `Some((id, name))` = 明确写入转换来源（转换短路键），INSERT/UPDATE 均写；
/// - `None` = 普通音频，不改动已有值（UPDATE 用 COALESCE 保留原值）。
///
/// `ncm_size`：v0.17.5 原始 .ncm 源文件大小（字节），与上面两列共同构成短路三元组。
/// `Some` 时 INSERT/UPDATE 均写；`None` 保留原值（普通音频 / 扫描刷新）。
pub fn upsert_local_record_with_source(
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
    genre: Option<&str>,
    year: Option<i64>,
    source_id: Option<&str>,
    ncm_origin: Option<(&str, &str)>,
    ncm_size: Option<i64>,
) -> Result<bool> {
    let (ncm_id, ncm_name) = match ncm_origin {
        Some((id, name)) => (Some(id), Some(name)),
        None => (None, None),
    };
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
                file_size=?11,file_mtime=?12,created_at=?13,genre=COALESCE(?14,''),year=?15,
                source_kind=COALESCE(?16,source_kind),
                ncm_origin_id=COALESCE(?18,ncm_origin_id),ncm_original_name=COALESCE(?19,ncm_original_name),
                ncm_original_size=COALESCE(?20,ncm_original_size)
             WHERE local_id=?17",
            params![
                name, artist, album, duration_ms, format, sample_rate, bit_depth, bitrate,
                channels, quality_tier, file_size, file_mtime, created_at_ms, genre, year,
                source_id, rec_id, ncm_id, ncm_name, ncm_size
            ],
        )?;
        Ok(false)
    } else {
        // 新增：song_id 显式 NULL（本地文件不关联网易云）。
        conn.execute(
            "INSERT INTO download_records(song_id,source_kind,status,name,artist,album,duration_ms,
                file_path,format,sample_rate,bit_depth,bitrate,channels,quality_tier,file_size,
                file_mtime,imported_at,created_at,downloaded_at,genre,year,ncm_origin_id,ncm_original_name,
                ncm_original_size)
             VALUES(NULL,COALESCE(?17,'local_scan'),'downloaded',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,
                    CAST(strftime('%s','now') AS INTEGER)*1000,?14,datetime('now'),COALESCE(?15,''),?16,?18,?19,?20)",
            params![
                name, artist, album, duration_ms, file_path, format, sample_rate, bit_depth,
                bitrate, channels, quality_tier, file_size, file_mtime, created_at_ms, genre, year,
                source_id, ncm_id, ncm_name, ncm_size
            ],
        )?;
        Ok(true)
    }
}

/// v0.14.0 / v0.17.5：NCM 转换前短路查询。判断该 (来源标识, 原始 .ncm 文件名, 原始大小)
/// 是否已有**同版本**转换产物入库；命中才跳过外部工具解码。
/// 带 size 是为了同名 .ncm 被更高清版本覆盖时（size 变大）不误跳过、让高清版正常入库。
/// 历史记录 size 为 NULL → `ncm_original_size=?3` 不匹配 → 降级为「不跳过」（宁可多转不丢歌）。
/// 查询走 idx_dr_ncm_origin_size 联合索引；查询失败由调用方降级为「不跳过」。
pub fn ncm_conversion_exists(
    conn: &Connection,
    source_id: &str,
    original_name: &str,
    original_size: i64,
) -> Result<bool> {
    let found = conn
        .query_row(
            "SELECT 1 FROM download_records
             WHERE status='downloaded'
               AND ncm_origin_id=?1 AND ncm_original_name=?2 AND ncm_original_size=?3
             LIMIT 1",
            params![source_id, original_name, original_size],
            |_| Ok(true),
        )
        .optional()?;
    Ok(found.is_some())
}

/// v0.17.5：NCM 转换产物源文件大小回填（短路闭环）。
/// 导入侧**每次实际转换成功后**按 (来源标识, 原始文件名) 把当前 .ncm 的 size 写回，
/// 使下次同版本扫描直接短路。覆盖两种场景：
/// - 老库记录 size=NULL → 回填后首次实现短路；
/// - 同名 .ncm 被新版覆盖且判重为等价/更差（不触发 upsert）→ 更新为新 size，
///   避免每次扫描都重复转换再跳过。
/// 库内同一 (id,name) 只有一条归位记录（upsert 按路径），不存在多版本共存，故可覆盖。
pub fn backfill_ncm_original_size(
    conn: &Connection,
    source_id: &str,
    original_name: &str,
    original_size: i64,
) -> Result<usize> {
    let n = conn.execute(
        "UPDATE download_records SET ncm_original_size=?1
         WHERE status='downloaded' AND ncm_origin_id=?2 AND ncm_original_name=?3",
        params![original_size, source_id, original_name],
    )?;
    Ok(n)
}

/// v0.17.8：NCM 转换是否已处理过（短路查询，基于独立缓存表）。
/// 与 download_records 的 ncm_origin 三列无关 —— 旧库那三列为 NULL 时本表依然能命中。
/// 三元组 (来源标识, 原始 .ncm 文件名, 源字节数) 全等才算已处理；同名高清版 size 变大不命中。
pub fn ncm_processed_exists(
    conn: &Connection,
    source_key: &str,
    ncm_name: &str,
    ncm_size: i64,
) -> Result<bool> {
    let found = conn
        .query_row(
            "SELECT 1 FROM ncm_processed
             WHERE source_key=?1 AND ncm_name=?2 AND ncm_size=?3
             LIMIT 1",
            params![source_key, ncm_name, ncm_size],
            |_| Ok(true),
        )
        .optional()?;
    Ok(found.is_some())
}

/// v0.17.8：记录一次 NCM 转换已处理（转换成功后**无条件**调用，不管后续判重是入库还是跳过）。
/// INSERT OR REPLACE：同三元组重复调用幂等；processed_at 取最新。
pub fn mark_ncm_processed(
    conn: &Connection,
    source_key: &str,
    ncm_name: &str,
    ncm_size: i64,
) -> Result<()> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    conn.execute(
        "INSERT OR REPLACE INTO ncm_processed(source_key, ncm_name, ncm_size, processed_at)
         VALUES(?1, ?2, ?3, ?4)",
        params![source_key, ncm_name, ncm_size, now_ms],
    )?;
    Ok(())
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

/// 判断曲目是否「有歌词」：复用 `lyrics::get_lyrics`，与详情页 / 全屏歌词完全同源。
// 同名 `.lrc` 侧车存在即命中（仅目录存在性检查，廉价）；否则读取音频内嵌歌词
// （ID3v2 USLT / Vorbis Comment），保证内嵌歌词的曲目也能在列表显示标记。
// 判定结果列表加载时逐曲计算一次，不产生额外网络请求。
fn track_has_lyrics(file_path: &str) -> bool {
    crate::lyrics::get_lyrics(std::path::Path::new(file_path)).is_some()
}

// 行 → TrackView 映射（多个查询复用，保证列序一致，避免各查询各自维护字段顺序）。
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
        favorite: row.get::<_, i64>(16)? != 0,
        genre: row.get(17)?,
        year: row.get(18)?,
        has_lyrics: track_has_lyrics(&row.get::<_, String>(13)?),
        unavailable: row.get::<_, i64>(19)? != 0,
    })
}

/// 查询曲库中可播放曲目（来自 download_records 中 status='downloaded' 且有文件路径者）。
/// 直接读 download_records 的冗余列（本地文件 song_id 为 NULL，无需关联其它表）。
/// 支持按关键词（曲名/艺人/专辑）、音质层级（quality）、仅看收藏（favorite_only）过滤。
pub fn list_tracks(
    conn: &Connection,
    q: Option<&str>,
    quality: Option<&str>,
    favorite_only: bool,
    unavailable_only: bool,
) -> Result<Vec<TrackView>> {
    let mut sql = String::from(
        "SELECT dr.local_id, dr.song_id, dr.name, dr.artist, dr.album, dr.format, dr.sample_rate,
                dr.bit_depth, dr.bitrate, dr.channels, dr.duration_ms, dr.quality_tier, dr.file_size, dr.file_path, dr.imported_at, dr.created_at, dr.favorite, dr.genre, dr.year, dr.unavailable
         FROM download_records dr
         WHERE dr.status='downloaded' AND dr.file_path IS NOT NULL",
    );
    // 动态 WHERE：关键词 LIKE / 音质 quality_tier / 收藏 favorite。
    // 用 Vec<&dyn ToSql> 持有参数引用，避免在迭代器存活期间因 Vec 重分配导致悬垂引用。
    let mut text_vals: Vec<String> = Vec::new();
    let mut fav_val: i64 = 0;
    let mut has_fav = false;
    let mut clauses: Vec<String> = Vec::new();
    let mut idx = 1usize;
    if let Some(kw) = q {
        let kw = kw.trim();
        if !kw.is_empty() {
            text_vals.push(format!("%{kw}%"));
            clauses.push(format!(
                "(dr.name LIKE ?{idx} OR dr.artist LIKE ?{idx} OR dr.album LIKE ?{idx})"
            ));
            idx += 1;
        }
    }
    match quality {
        Some("lossless") | Some("hires") => {
            text_vals.push(quality.unwrap().to_string());
            clauses.push(format!("dr.quality_tier = ?{idx}"));
            idx += 1;
        }
        Some("lossless_up") => {
            text_vals.push("lossless".to_string());
            text_vals.push("hires".to_string());
            clauses.push(format!(
                "dr.quality_tier IN (?{idx}, ?{idx2})",
                idx = idx,
                idx2 = idx + 1
            ));
            idx += 2;
        }
        _ => {}
    }
    // v0.17.7：仅看不可访问曲目（正向筛选；默认关闭时全部含 ⚠ 标记项）。
    if unavailable_only {
        clauses.push("dr.unavailable = 1".to_string());
    }
    if favorite_only {
        clauses.push(format!("dr.favorite = ?{idx}"));
        fav_val = 1;
        has_fav = true;
        // favorite 是最后一个带参数的过滤条件，idx 不再被读取。
    }
    for c in &clauses {
        sql.push_str(" AND ");
        sql.push_str(c);
    }
    sql.push_str(" ORDER BY dr.artist, dr.name LIMIT 2000");

    let mut args: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(text_vals.len() + 1);
    for v in &text_vals {
        args.push(v);
    }
    if has_fav {
        args.push(&fav_val);
    }

    let map = track_from_row;
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args), map)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
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
                    dr.bit_depth, dr.bitrate, dr.channels, dr.duration_ms, dr.quality_tier, dr.file_size, dr.file_path, dr.imported_at, dr.created_at, dr.favorite, dr.genre, dr.year, dr.unavailable
             FROM download_records dr
             WHERE dr.local_id=?1 AND dr.status='downloaded' AND dr.file_path IS NOT NULL
             ORDER BY dr.local_id DESC LIMIT 1",
            params![local_id],
            track_from_row,
        )
        .optional()?;
    Ok(opt)
}

/// 按归位后的文件路径取完整曲目视图（导入落库后回读增量明细用）。
/// 与 `get_track_by_local_id` 同一套列序，复用 `track_from_row`。
pub fn get_track_by_path(conn: &Connection, file_path: &str) -> Result<Option<TrackView>> {
    let opt: Option<TrackView> = conn
        .query_row(
            "SELECT dr.local_id, dr.song_id, dr.name, dr.artist, dr.album, dr.format, dr.sample_rate,
                    dr.bit_depth, dr.bitrate, dr.channels, dr.duration_ms, dr.quality_tier, dr.file_size, dr.file_path, dr.imported_at, dr.created_at, dr.favorite, dr.genre, dr.year, dr.unavailable
             FROM download_records dr
             WHERE dr.file_path=?1 AND dr.status='downloaded' AND dr.file_path IS NOT NULL
             ORDER BY dr.local_id DESC LIMIT 1",
            params![file_path],
            track_from_row,
        )
        .optional()?;
    Ok(opt)
}

/// 切换某曲目的收藏标记（按 local_id；只写 download_records.favorite 列）。
/// 返回受影响的行数（0 表示该 local_id 不存在）。
pub fn set_favorite(conn: &Connection, local_id: i64, favorite: bool) -> Result<usize> {
    let n = conn.execute(
        "UPDATE download_records SET favorite = ?1 WHERE local_id = ?2",
        params![favorite as i64, local_id],
    )?;
    Ok(n)
}

/// v0.17.7：扫描对账三段式的第②③段 —— 不可访问打标/复核恢复 + 孤儿清理。
/// 不再自动删库（本地文件暂时读不到可能是占用/写入中/权限问题）：
/// - `metadata` 失败或空文件 且当前未标记 → 打 `unavailable=1`（首次记录 `unavailable_at`）；
/// - 可访问（metadata 成功且非空）且当前已标记 → 清标（文件被放回 / 服务器恢复）；
/// - 孤儿清理只针对「真删记录」遗留的 playlist_items（trash_track 删索引后本会清，这里是历史兜底）。
/// 返回 `(新标记数, 恢复数)`。
pub fn reconcile_unavailable(conn: &Connection) -> Result<(u32, u32)> {
    let rows: Vec<(i64, String, i64)> = {
        let mut stmt = conn.prepare(
            "SELECT local_id, file_path, unavailable FROM download_records
             WHERE status='downloaded' AND file_path IS NOT NULL",
        )?;
        let mut out = Vec::new();
        let rows = stmt.query_map((), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
        })?;
        for r in rows {
            out.push(r?);
        }
        out
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut marked = 0u32;
    let mut recovered = 0u32;
    for (local_id, path, current) in rows {
        let accessible = std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false);
        if !accessible && current == 0 {
            conn.execute(
                "UPDATE download_records SET unavailable=1, unavailable_at=?1 WHERE local_id=?2",
                params![now_ms, local_id],
            )?;
            marked += 1;
        } else if accessible && current != 0 {
            conn.execute(
                "UPDATE download_records SET unavailable=0, unavailable_at=NULL WHERE local_id=?1",
                params![local_id],
            )?;
            recovered += 1;
        }
    }
    conn.execute(
        "DELETE FROM playlist_items
         WHERE NOT EXISTS (
             SELECT 1 FROM download_records dr WHERE dr.local_id = playlist_items.local_id
         )",
        [],
    )?;
    Ok((marked, recovered))
}

/// v0.17.7：单点打标（播放 404 / verify 判定文件不可访问时）。
/// `unavailable_at` 只在首次标记时写入（COALESCE），保留「首次失效时刻」语义。
pub fn mark_unavailable(conn: &Connection, local_id: i64) -> Result<()> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    conn.execute(
        "UPDATE download_records SET unavailable=1, unavailable_at=COALESCE(unavailable_at, ?1)
         WHERE local_id=?2",
        params![now_ms, local_id],
    )?;
    Ok(())
}

/// v0.17.7：单点清标（verify 复核 / 双击恢复成功）。
pub fn clear_unavailable(conn: &Connection, local_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE download_records SET unavailable=0, unavailable_at=NULL WHERE local_id=?1",
        params![local_id],
    )?;
    Ok(())
}

/// v0.17.7：原位恢复（曲源重新导入命中不可访问记录时）。
/// 把新文件的归位路径与新元数据写回**同一条**记录并清标，保留 local_id →
/// 播放列表 / 收藏 / 历史引用全部不断。source_id 与 NCM 短路来源一并不覆盖（保留原值）。
pub fn restore_unavailable_record(
    conn: &Connection,
    local_id: i64,
    file_path: &str,
    file_size: i64,
    file_mtime: i64,
    format: Option<&str>,
    sample_rate: Option<i64>,
    bit_depth: Option<i64>,
    bitrate: Option<i64>,
    channels: Option<i64>,
    duration_ms: Option<i64>,
    quality_tier: Option<&str>,
    genre: Option<&str>,
    year: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE download_records SET file_path=?1, file_size=?2, file_mtime=?3, format=?4,
             sample_rate=?5, bit_depth=?6, bitrate=?7, channels=?8, duration_ms=?9,
             quality_tier=?10, genre=?11, year=?12,
             unavailable=0, unavailable_at=NULL, status='downloaded'
         WHERE local_id=?13",
        params![
            file_path, file_size, file_mtime, format, sample_rate, bit_depth, bitrate, channels,
            duration_ms, quality_tier, genre, year, local_id
        ],
    )?;
    Ok(())
}

/// v0.17.7：列出所有当前不可访问的曲目 ID（供谓词/单测；扫描复核本身在 reconcile 内完成）。
pub fn list_unavailable_ids(conn: &Connection) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT local_id FROM download_records WHERE unavailable=1 AND status='downloaded'",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 删除曲目索引：连同所有播放列表里指向它的条目一起清掉（单个事务）。
///
/// 调用方（`library::trash_track`）必须先把音频文件移入回收站：只删索引而磁盘文件仍在，
/// 下一次扫描会把文件原样重新入库，等于没删。本函数不碰任何磁盘文件。
/// 返回受影响的播放列表条目数，供 UI 提示「已从 N 个列表移除」。
pub fn delete_track(conn: &Connection, local_id: i64) -> Result<u32> {
    let tx = conn.unchecked_transaction()?;
    // playlist_items 对 download_records 没有外键，必须显式删，否则留下孤儿
    // （左侧 track_count 仍计入、右侧 JOIN 取不到，播放队列也会撞上失效 id）。
    let items = tx.execute(
        "DELETE FROM playlist_items WHERE local_id=?1",
        params![local_id],
    )? as u32;
    let rows = tx.execute(
        "DELETE FROM download_records WHERE local_id=?1",
        params![local_id],
    )?;
    if rows == 0 {
        tx.rollback()?;
        bail!("曲目不存在或已被删除");
    }
    tx.commit()?;
    Ok(items)
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

/// #28：判断该文件的记录是否已抽取过流派 / 年份标签。
/// 约定 `genre IS NOT NULL` = 已尝试（文件无标签时落空串）；NULL = 迁移前的旧记录，
/// 扫描增量门控据此下探一次 lofty 抽取回填，之后不再产生额外 I/O。
pub fn tags_extracted(conn: &Connection, file_path: &str) -> Result<bool> {
    let missing = conn
        .query_row(
            "SELECT 1 FROM download_records
             WHERE file_path=?1 AND status='downloaded' AND genre IS NULL",
            params![file_path],
            |_| Ok(true),
        )
        .optional()?;
    Ok(!missing.unwrap_or(false))
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
    /// 收藏标记（用户侧「已收藏」）。SQLite 存 INTEGER（0/1），读取转 bool。
    pub favorite: bool,
    /// #28：曲级流派 / 发行年份（与 TrackView 同源）。
    pub genre: Option<String>,
    pub year: Option<i64>,
    /// 歌曲列表「有歌词」标记——与 `TrackView` 同源判定（`.lrc` 侧车或内嵌歌词）。
    pub has_lyrics: bool,
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
                dr.duration_ms, dr.quality_tier, dr.file_size, dr.file_path, dr.imported_at, dr.created_at, dr.favorite, dr.genre, dr.year
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
            favorite: r.get::<_, i64>(12)? != 0,
            genre: r.get(13)?,
            year: r.get(14)?,
            has_lyrics: track_has_lyrics(&r.get::<_, String>(9)?),
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

/// 分组视图（专辑 / 歌手）：分组名称 + 组内曲目数 + 代表曲目 local_id（用于取封面）。
/// 用于左侧导航二级列表。NULL 或空字符串折叠为统一的占位名称，便于浏览未标注元数据的曲目。
/// `cover_id` 由 `MIN(local_id)` 聚合得到（组内稳定代表曲目），前端据此请求 `/api/cover/:id`；
/// 加字段不新增查询、不改变分组语义，`renderGroupList` 只消费 name/track_count，忽略此字段。
#[derive(Debug, Clone, Serialize)]
pub struct GroupView {
    pub name: String,
    pub track_count: i64,
    pub cover_id: Option<i64>,
    /// #28：组内流派（字典序最小的非空流派）与年份范围，供歌手行 / 专辑卡片副信息展示。
    /// 全组无标签时为 None，前端按「空字段不渲染」处理。
    pub genre: Option<String>,
    pub year_min: Option<i64>,
    pub year_max: Option<i64>,
}

/// 列出全部专辑（按名称分组，含曲目数 / 代表曲目 id / 流派 / 年份范围）。
/// NULL/空 album 折叠为「未知专辑」；按专辑名称字典序。
/// #28 的聚合全部在同一次 GROUP BY 内完成，不新增查询。
pub fn list_albums(conn: &Connection) -> Result<Vec<GroupView>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(dr.album,''),'未知专辑') AS album, COUNT(*), MIN(dr.local_id),
                MIN(CASE WHEN dr.genre IS NOT NULL AND dr.genre <> '' THEN dr.genre END),
                MIN(dr.year), MAX(dr.year)
         FROM download_records dr
         WHERE dr.status='downloaded' AND dr.file_path IS NOT NULL
         GROUP BY COALESCE(NULLIF(dr.album,''),'未知专辑')
         ORDER BY album",
    )?;
    let rows = stmt.query_map((), |r| {
        Ok(GroupView {
            name: r.get(0)?,
            track_count: r.get(1)?,
            cover_id: r.get(2)?,
            genre: r.get(3)?,
            year_min: r.get(4)?,
            year_max: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 列出全部歌手（按整字段分组，含曲目数 / 代表曲目 id / 流派 / 年份范围）。
/// NULL/空 artist 折叠为「未知艺人」；艺人字段含多人协作分隔符时不拆分。
pub fn list_artists(conn: &Connection) -> Result<Vec<GroupView>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(dr.artist,''),'未知艺人') AS artist, COUNT(*), MIN(dr.local_id),
                MIN(CASE WHEN dr.genre IS NOT NULL AND dr.genre <> '' THEN dr.genre END),
                MIN(dr.year), MAX(dr.year)
         FROM download_records dr
         WHERE dr.status='downloaded' AND dr.file_path IS NOT NULL
         GROUP BY COALESCE(NULLIF(dr.artist,''),'未知艺人')
         ORDER BY artist",
    )?;
    let rows = stmt.query_map((), |r| {
        Ok(GroupView {
            name: r.get(0)?,
            track_count: r.get(1)?,
            cover_id: r.get(2)?,
            genre: r.get(3)?,
            year_min: r.get(4)?,
            year_max: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 取某专辑内曲目（按名称排序）；`album` 传入「未知专辑」时匹配 album 为 NULL/空的曲目。
pub fn get_tracks_by_album(conn: &Connection, album: &str) -> Result<Vec<TrackView>> {
    let mut stmt = conn.prepare(
        "SELECT dr.local_id, dr.song_id, dr.name, dr.artist, dr.album, dr.format, dr.sample_rate,
                dr.bit_depth, dr.bitrate, dr.channels, dr.duration_ms, dr.quality_tier, dr.file_size, dr.file_path, dr.imported_at, dr.created_at, dr.favorite, dr.genre, dr.year, dr.unavailable
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
                dr.bit_depth, dr.bitrate, dr.channels, dr.duration_ms, dr.quality_tier, dr.file_size, dr.file_path, dr.imported_at, dr.created_at, dr.favorite, dr.genre, dr.year, dr.unavailable
         FROM download_records dr
         WHERE dr.status='downloaded' AND dr.file_path IS NOT NULL
           AND COALESCE(NULLIF(dr.artist,''),'未知艺人')=?1
         ORDER BY dr.name",
    )?;
    let rows = stmt.query_map(params![artist], track_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ============================================================
// 曲源归属（复用既有 source_kind 列，不改 track_from_row 固定列序）
// ============================================================

/// 按 `source_kind` 统计曲目数。返回原始分组 key，由上层映射成曲源名称并把
/// 未知 key / 未指定来源合并为「未指定来源」组（零写库，删曲源不批量 UPDATE）。
pub fn source_counts(conn: &Connection) -> Result<Vec<(String, u32)>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(dr.source_kind,'local_scan'), COUNT(*)
         FROM download_records dr
         WHERE dr.status='downloaded' AND dr.file_path IS NOT NULL
         GROUP BY COALESCE(dr.source_kind,'local_scan')
         ORDER BY 1",
    )?;
    let rows = stmt.query_map((), |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 取某曲源导入的曲目（按名称排序）。
///
/// `source_id == UNSPECIFIED_SOURCE` 时取「未指定来源」：历史 `'local_scan'` +
/// 所有不在 `known_ids`（当前仍存在的曲源）里的孤儿归属。曲源删除后无需批量
/// UPDATE，歌曲自然归入该组。曲目列与 `track_from_row()` 严格一致。
pub fn get_tracks_by_source(
    conn: &Connection,
    source_id: &str,
    known_ids: &[String],
) -> Result<Vec<TrackView>> {
    let select = "SELECT dr.local_id, dr.song_id, dr.name, dr.artist, dr.album, dr.format, dr.sample_rate,
                dr.bit_depth, dr.bitrate, dr.channels, dr.duration_ms, dr.quality_tier, dr.file_size, dr.file_path, dr.imported_at, dr.created_at, dr.favorite, dr.genre, dr.year, dr.unavailable
         FROM download_records dr
         WHERE dr.status='downloaded' AND dr.file_path IS NOT NULL AND ";
    if source_id != UNSPECIFIED_SOURCE {
        let sql = format!("{select} dr.source_kind=?1 ORDER BY dr.name");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![source_id], track_from_row)?;
        return Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    }
    // 未指定来源：COALESCE 归一的 'local_scan' 或不属于任何现存曲源的 key。
    let mut sql = format!(
        "{select} (COALESCE(dr.source_kind,'local_scan')='{UNSPECIFIED_SOURCE}'"
    );
    if !known_ids.is_empty() {
        let placeholders = (1..=known_ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(
            " OR COALESCE(dr.source_kind,'local_scan') NOT IN ({placeholders})"
        ));
    }
    sql.push_str(") ORDER BY dr.name");
    let mut stmt = conn.prepare(&sql)?;
    let rows = if known_ids.is_empty() {
        stmt.query_map([], track_from_row)?
    } else {
        stmt.query_map(
            rusqlite::params_from_iter(known_ids.iter().map(|s| s.as_str())),
            track_from_row,
        )?
    };
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
    fn reconcile_unavailable_marks_not_deletes_and_clears_real_orphans() {
        let c = test_conn();
        let id = create_playlist(&c, "孤儿测试").unwrap();
        insert_missing_tracks(&c, &[1, 2, 3]);
        for i in 1..=3 {
            add_to_playlist(&c, id, i).unwrap();
        }
        assert_eq!(list_playlists(&c).unwrap()[0].track_count, 3);

        // v0.17.7：文件不存在只打标、不删记录 → 播放列表引用完整保留。
        let (marked, recovered) = reconcile_unavailable(&c).unwrap();
        assert_eq!(marked, 3, "3 条文件不存在的记录应被标记");
        assert_eq!(recovered, 0);
        assert_eq!(list_unavailable_ids(&c).unwrap().len(), 3);
        assert_eq!(list_playlists(&c).unwrap()[0].track_count, 3, "打标不删记录，引用保留");
        assert_eq!(get_playlist_tracks(&c, id).unwrap().len(), 3);

        // 再次对账：已标记的不重复计数。
        let (marked2, _) = reconcile_unavailable(&c).unwrap();
        assert_eq!(marked2, 0, "已标记记录不应重复计数");

        // 真删记录后（模拟用户删除），孤儿条目被清扫。
        c.execute("DELETE FROM download_records WHERE local_id=1", []).unwrap();
        let (_, _) = reconcile_unavailable(&c).unwrap();
        assert_eq!(list_playlists(&c).unwrap()[0].track_count, 2, "真删后孤儿引用清扫");

        // 历史孤儿（旧版本遗留，download_records 里已无对应行）也要被清掉。
        c.execute(
            "INSERT INTO playlist_items(playlist_id,local_id,position) VALUES(?1,999,4)",
            params![id],
        )
        .unwrap();
        assert_eq!(list_playlists(&c).unwrap()[0].track_count, 3);
        reconcile_unavailable(&c).unwrap();
        assert_eq!(list_playlists(&c).unwrap()[0].track_count, 2, "历史孤儿也要清扫");
    }

    #[test]
    fn unavailable_mark_clear_and_restore_preserves_local_id() {
        let c = test_conn();
        insert_missing_tracks(&c, &[10]);
        mark_unavailable(&c, 10).unwrap();
        assert_eq!(list_unavailable_ids(&c).unwrap(), vec![10]);
        // 首次标记时间不因重复打标而变。
        let t1: i64 = c.query_row("SELECT unavailable_at FROM download_records WHERE local_id=10", [], |r| r.get(0)).unwrap();
        mark_unavailable(&c, 10).unwrap();
        let t2: i64 = c.query_row("SELECT unavailable_at FROM download_records WHERE local_id=10", [], |r| r.get(0)).unwrap();
        assert_eq!(t1, t2, "unavailable_at 保留首次时刻");

        // 原位恢复：写回新路径 + 元数据 + 清标，local_id 不变。
        restore_unavailable_record(
            &c, 10, "/lib/restored.flac", 2048, 123, Some("flac"),
            Some(44100), Some(16), Some(1000), Some(2), Some(5000),
            Some("lossless"), Some("流行"), Some(2024),
        ).unwrap();
        assert!(list_unavailable_ids(&c).unwrap().is_empty());
        let p: String = c.query_row("SELECT file_path FROM download_records WHERE local_id=10", [], |r| r.get(0)).unwrap();
        assert_eq!(p, "/lib/restored.flac");

        // 清标幂等。
        clear_unavailable(&c, 10).unwrap();
        assert!(list_unavailable_ids(&c).unwrap().is_empty());
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

    #[test]
    fn delete_track_removes_index_and_all_playlist_refs() {
        let c = test_conn();
        insert_missing_tracks(&c, &[1, 2]);
        let p1 = create_playlist(&c, "A").unwrap();
        let p2 = create_playlist(&c, "B").unwrap();
        add_to_playlist(&c, p1, 1).unwrap();
        add_to_playlist(&c, p2, 1).unwrap();
        add_to_playlist(&c, p1, 2).unwrap();

        let affected = delete_track(&c, 1).unwrap();
        assert_eq!(affected, 2, "两个列表里的引用都应清掉");
        let exists: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM download_records WHERE local_id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0, "索引记录应消失");
        assert_eq!(get_playlist_tracks(&c, p1).unwrap().len(), 1, "另一首曲目不受影响");
        assert!(get_playlist_tracks(&c, p2).unwrap().is_empty());
        assert!(delete_track(&c, 1).is_err(), "重复删除应报错");
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

    /// 简化调用：`source_id` 之外全部给最小可用值。
    fn upsert(conn: &Connection, path: &str, name: &str, source_id: Option<&str>) -> bool {
        upsert_local_record_with_source(
            conn,
            path,
            name,
            Some("艺人"),
            None,
            None,
            Some("flac"),
            None,
            None,
            None,
            None,
            Some("lossless"),
            1000,
            0,
            None,
            None,
            None,
            source_id,
            None,
            None,
        )
        .unwrap()
    }

    fn kind_of(conn: &Connection, path: &str) -> String {
        conn.query_row(
            "SELECT source_kind FROM download_records WHERE file_path=?1",
            params![path],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn source_attribution_is_written_and_grouped() {
        let c = test_conn();
        assert!(upsert(&c, "/lib/a.flac", "曲一", Some("src-a")));
        assert!(upsert(&c, "/lib/b.flac", "曲二", Some("src-a")));
        assert!(upsert(&c, "/lib/c.flac", "曲三", Some("src-b")));
        // 历史 / 扫描记录：None 落「未指定来源」。
        assert!(upsert(&c, "/lib/d.flac", "曲四", None));

        let counts = source_counts(&c).unwrap();
        assert_eq!(
            counts.iter().find(|(k, _)| k == "src-a").map(|x| x.1),
            Some(2)
        );
        assert_eq!(
            counts.iter().find(|(k, _)| k == "src-b").map(|x| x.1),
            Some(1)
        );
        assert_eq!(
            counts
                .iter()
                .find(|(k, _)| k == UNSPECIFIED_SOURCE)
                .map(|x| x.1),
            Some(1)
        );

        let known = vec!["src-a".to_string(), "src-b".to_string()];
        assert_eq!(get_tracks_by_source(&c, "src-a", &known).unwrap().len(), 2);
        assert_eq!(get_tracks_by_source(&c, "src-b", &known).unwrap().len(), 1);
        assert_eq!(
            get_tracks_by_source(&c, UNSPECIFIED_SOURCE, &known)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn rescan_does_not_erase_attribution_but_replacement_fixes_it() {
        let c = test_conn();
        assert!(upsert(&c, "/lib/a.flac", "曲一", Some("src-a")));
        let first_id: i64 = c
            .query_row(
                "SELECT local_id FROM download_records WHERE file_path='/lib/a.flac'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        // 启动扫描 / 标签重扫传 None：不得抹掉曲源归属，也不得改写首次导入时间。
        assert!(!upsert(&c, "/lib/a.flac", "曲一(重扫)", None));
        assert_eq!(kind_of(&c, "/lib/a.flac"), "src-a");

        // 替换决策明确传新曲源：UPDATE 同一行，local_id 与播放列表引用保留。
        assert!(!upsert(&c, "/lib/a.flac", "曲一(替换)", Some("src-b")));
        assert_eq!(kind_of(&c, "/lib/a.flac"), "src-b");
        let after_id: i64 = c
            .query_row(
                "SELECT local_id FROM download_records WHERE file_path='/lib/a.flac'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(first_id, after_id, "替换必须复用同一行，不能 DELETE+INSERT");

        // 旧挂起组无来源信息：显式归「未指定来源」。
        assert!(!upsert(
            &c,
            "/lib/a.flac",
            "曲一(旧组)",
            Some(UNSPECIFIED_SOURCE)
        ));
        assert_eq!(kind_of(&c, "/lib/a.flac"), UNSPECIFIED_SOURCE);
    }

    #[test]
    fn deleted_source_orphans_fall_into_unspecified_group() {
        let c = test_conn();
        assert!(upsert(&c, "/lib/a.flac", "曲一", Some("src-gone")));
        assert!(upsert(&c, "/lib/b.flac", "曲二", None));
        // 曲源已从配置移除，只剩仍存在的曲源列表。
        let known = vec!["src-live".to_string()];
        let unknown = get_tracks_by_source(&c, UNSPECIFIED_SOURCE, &known).unwrap();
        assert_eq!(unknown.len(), 2, "孤儿归属 + 历史歌曲都归入未指定来源");
        assert!(get_tracks_by_source(&c, "src-gone", &known).unwrap().len() == 1);
    }
}

// ============================================================
// v0.14.0：NCM 转换来源两列 + 短路判重
// ============================================================
#[cfg(test)]
mod ncm_tests {
    use super::*;
    use rusqlite::Connection;

    fn columns(conn: &Connection) -> Vec<String> {
        conn.prepare("PRAGMA table_info(download_records)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|x| x.unwrap())
            .collect()
    }

    #[test]
    fn fresh_schema_has_ncm_origin_columns_and_index() {
        let c = Connection::open_in_memory().unwrap();
        init_schema(&c).unwrap();
        let cols = columns(&c);
        assert!(cols.iter().any(|x| x == "ncm_origin_id"));
        assert!(cols.iter().any(|x| x == "ncm_original_name"));
        assert!(cols.iter().any(|x| x == "ncm_original_size"));
        let idx: String = c
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_dr_ncm_origin'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, "idx_dr_ncm_origin");
        let size_idx: String = c
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_dr_ncm_origin_size'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(size_idx, "idx_dr_ncm_origin_size");
    }

    #[test]
    fn legacy_schema_migrates_idempotently() {
        let c = Connection::open_in_memory().unwrap();
        // 模拟旧库：无 genre/year 与 NCM 两列，init_schema 应无破坏补列。
        c.execute_batch(
            "CREATE TABLE download_records(local_id INTEGER PRIMARY KEY, song_id INTEGER,
             source_kind TEXT NOT NULL, status TEXT NOT NULL, file_path TEXT, name TEXT,
             artist TEXT, album TEXT, duration_ms INTEGER, format TEXT, sample_rate INTEGER,
             bit_depth INTEGER, bitrate INTEGER, channels INTEGER, quality_tier TEXT,
             file_size INTEGER, file_mtime INTEGER, imported_at INTEGER, created_at INTEGER,
             fail_reason TEXT, downloaded_at TEXT);",
        )
        .unwrap();
        init_schema(&c).unwrap();
        init_schema(&c).unwrap(); // 重复初始化必须幂等
        let cols = columns(&c);
        assert!(cols.iter().any(|x| x == "genre"));
        assert!(cols.iter().any(|x| x == "ncm_origin_id"));
        assert!(cols.iter().any(|x| x == "ncm_original_name"));
        assert!(cols.iter().any(|x| x == "ncm_original_size"));
    }

    #[test]
    fn ncm_conversion_exists_matches_exact_source_name_and_size() {
        let c = Connection::open_in_memory().unwrap();
        init_schema(&c).unwrap();
        c.execute(
            "INSERT INTO download_records(local_id,source_kind,status,file_path,name,
             ncm_origin_id,ncm_original_name,ncm_original_size)
             VALUES (1,'src-1','downloaded','/lib/a.flac','A','src-1','song.ncm',1024)",
            [],
        )
        .unwrap();
        assert!(ncm_conversion_exists(&c, "src-1", "song.ncm", 1024).unwrap());
        // 同名但更高清（size 变大）→ 不命中，让高清版正常转换入库。
        assert!(!ncm_conversion_exists(&c, "src-1", "song.ncm", 51200).unwrap());
        assert!(!ncm_conversion_exists(&c, "src-2", "song.ncm", 1024).unwrap());
        assert!(!ncm_conversion_exists(&c, "src-1", "other.ncm", 1024).unwrap());
        // 历史 size=NULL → 不命中（降级为多转一次）。
        c.execute(
            "UPDATE download_records SET ncm_original_size=NULL WHERE local_id=1",
            [],
        )
        .unwrap();
        assert!(!ncm_conversion_exists(&c, "src-1", "song.ncm", 1024).unwrap());
        // 回填后命中；再回填新版本 size 会覆盖（等价/更差判重不落库时避免重复转换）。
        assert_eq!(backfill_ncm_original_size(&c, "src-1", "song.ncm", 2048).unwrap(), 1);
        assert!(ncm_conversion_exists(&c, "src-1", "song.ncm", 2048).unwrap());
        assert_eq!(backfill_ncm_original_size(&c, "src-1", "song.ncm", 9999).unwrap(), 1);
        assert!(ncm_conversion_exists(&c, "src-1", "song.ncm", 9999).unwrap());
        assert!(!ncm_conversion_exists(&c, "src-1", "song.ncm", 2048).unwrap());
    }

    #[test]
    fn ncm_processed_cache_is_triple_keyed_and_independent_of_origin_columns() {
        // v0.17.8：缓存表不依赖 download_records 的 ncm_origin 三列（旧库为 NULL 也能用）。
        let c = Connection::open_in_memory().unwrap();
        init_schema(&c).unwrap();
        // 故意不插入任何 download_records 记录（模拟旧库无 ncm_origin 信息）。
        assert!(!ncm_processed_exists(&c, "CloudMusic", "song.ncm", 1024).unwrap());
        // 转换成功后无条件记一笔（哪怕后续判重 Skip 不入库）。
        mark_ncm_processed(&c, "CloudMusic", "song.ncm", 1024).unwrap();
        assert!(ncm_processed_exists(&c, "CloudMusic", "song.ncm", 1024).unwrap());
        // 三元组任一不同都不命中：来源不同 / 名字不同 / size 不同（高清版）。
        assert!(!ncm_processed_exists(&c, "other-source", "song.ncm", 1024).unwrap());
        assert!(!ncm_processed_exists(&c, "CloudMusic", "other.ncm", 1024).unwrap());
        assert!(!ncm_processed_exists(&c, "CloudMusic", "song.ncm", 51200).unwrap());
        // 同名高清版转换后另记一条，旧 size 仍在（各自独立短路）。
        mark_ncm_processed(&c, "CloudMusic", "song.ncm", 51200).unwrap();
        assert!(ncm_processed_exists(&c, "CloudMusic", "song.ncm", 1024).unwrap());
        assert!(ncm_processed_exists(&c, "CloudMusic", "song.ncm", 51200).unwrap());
        // 同三元组重复写幂等（主键约束）。
        mark_ncm_processed(&c, "CloudMusic", "song.ncm", 1024).unwrap();
        let cnt: i64 = c.query_row(
            "SELECT COUNT(*) FROM ncm_processed WHERE source_key='CloudMusic' AND ncm_name='song.ncm'",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(cnt, 2, "两个 size 各一条，重复写不新增");
    }
}

