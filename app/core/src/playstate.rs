//! 播放状态持久化（FR-18）：`<曲库根>/.mt/playstate.json`。
//!
//! - 普通 JSON 文件，**非 TOML、非关系表**；换曲库即换文件，队列不跨曲库串味。
//! - DB（`download_records`）仍是曲目唯一权威源。读取时逐个校验队列里的 `local_id`，
//!   任一失效即丢弃整份队列并删除文件（规格：id 失效则丢弃队列）。
//! - 只在切歌 / 暂停 / 退出时写盘，不做定时落盘；写盘用「临时文件 + rename」保证不写坏。

use crate::Result;
use crate::config::Config;
use anyhow::bail;
use rusqlite::{Connection, params};
use serde_json::Value;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "playstate.json";
/// 队列最大长度（与 `auto_playlist_count` 上限一致；有名列表全量可能更长，故放宽到 5000）。
const MAX_QUEUE_LEN: usize = 5000;

pub fn state_path(cfg: &Config) -> PathBuf {
    cfg.paths.app_data.join(FILE_NAME)
}

/// 校验播放状态：queue 必须是正整数 id 数组、index 落在范围内，且每个 id 仍在曲库里。
pub fn validate(conn: &Connection, state: &Value) -> bool {
    let queue = match state.get("queue").and_then(|v| v.as_array()) {
        Some(q) => q,
        None => return false,
    };
    if queue.is_empty() || queue.len() > MAX_QUEUE_LEN {
        return false;
    }
    for v in queue {
        let id = match v.as_i64() {
            Some(i) if i > 0 => i,
            _ => return false,
        };
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM download_records WHERE local_id=?1)",
                params![id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if !exists {
            return false;
        }
    }
    let idx = match state.get("index").and_then(|v| v.as_u64()) {
        Some(i) => i as usize,
        None => return false,
    };
    idx < queue.len()
}

fn save_to(path: &Path, conn: &Connection, state: &Value) -> Result<()> {
    if !validate(conn, state) {
        bail!("播放状态无效（队列为空、id 失效或 index 越界）");
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(state)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn load_from(path: &Path, conn: &Connection) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    let state: Value = serde_json::from_str(&text).ok()?;
    if !validate(conn, &state) {
        // 曲目已被清理 / 换过曲库 → 丢弃，避免恢复出一个播不动的队列。
        let _ = std::fs::remove_file(path);
        return None;
    }
    Some(state)
}

pub fn save(cfg: &Config, conn: &Connection, state: &Value) -> Result<()> {
    save_to(&state_path(cfg), conn, state)
}

pub fn load(cfg: &Config, conn: &Connection) -> Option<Value> {
    load_from(&state_path(cfg), conn)
}

/// 清空播放状态（用户点「清空队列」或队列被丢弃时）。
pub fn clear(cfg: &Config) -> Result<()> {
    let path = state_path(cfg);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conn_with(ids: &[i64]) -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE download_records(local_id INTEGER PRIMARY KEY, status TEXT, file_path TEXT);",
        )
        .unwrap();
        for id in ids {
            c.execute(
                "INSERT INTO download_records(local_id,status,file_path) VALUES(?1,'downloaded','x')",
                params![id],
            )
            .unwrap();
        }
        c
    }

    #[test]
    fn validate_rejects_missing_ids_and_bad_shape() {
        let c = conn_with(&[1, 2, 3]);
        assert!(validate(&c, &json!({ "queue": [1, 2, 3], "index": 1 })));
        assert!(!validate(&c, &json!({ "queue": [1, 99], "index": 0 })), "失效 id 应拒绝");
        assert!(!validate(&c, &json!({ "queue": [], "index": 0 })), "空队列应拒绝");
        assert!(!validate(&c, &json!({ "queue": [1, 2], "index": 5 })), "越界 index 应拒绝");
        assert!(!validate(&c, &json!({ "queue": "oops", "index": 0 })), "坏结构应拒绝");
        assert!(!validate(&c, &json!({ "queue": [0], "index": 0 })), "非正 id 应拒绝");
    }

    #[test]
    fn save_load_roundtrip_and_invalidation() {
        let dir = std::env::temp_dir().join(format!("mt_playstate_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("playstate.json");
        let _ = std::fs::remove_file(&path);
        let c = conn_with(&[1, 2, 3]);
        let st = json!({
            "queue": [1, 2, 3], "index": 1, "n": 20,
            "source": "library", "title": "全部曲库", "shuffle": false, "repeat": true,
        });
        save_to(&path, &c, &st).unwrap();
        let back = load_from(&path, &c).unwrap();
        assert_eq!(back["index"], 1);
        assert_eq!(back["queue"], json!([1, 2, 3]));

        // 曲目 2 从曲库消失 → 整份队列丢弃且文件被删。
        let c2 = conn_with(&[1, 3]);
        assert!(load_from(&path, &c2).is_none(), "失效 id 应导致队列被丢弃");
        assert!(!path.exists(), "失效状态文件应被清理");
        assert!(save_to(&path, &c2, &st).is_err(), "不允许写入含失效 id 的状态");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
