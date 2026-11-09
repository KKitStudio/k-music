//! 下载任务队列与状态机（内存，可选持久化到 `.mt/download_queue.json`）。
//!
//! 本模块是 UI 无关的纯状态机：只负责管理任务状态、暂停/取消标志与持久化；
//! 实际下载与落点由 serve 的 worker 线程驱动（未来基于官方开发者 API 恢复同步时复用本窗口）。
//! 这样队列逻辑可独立单元测试，不依赖联网。
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 任务状态机（docs/05 §12.1）：
/// `queued → downloading → done`（移入 `download/`）`→ imported`（整理入曲库）。
/// 另含控制相关的 `failed / paused / cancelled`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DlStatus {
    Queued,
    Downloading,
    Done,
    Imported,
    Failed,
    Paused,
    Cancelled,
}

impl DlStatus {
    pub fn as_zh(&self) -> &'static str {
        match self {
            DlStatus::Queued => "排队中",
            DlStatus::Downloading => "下载中",
            DlStatus::Done => "已完成",
            DlStatus::Imported => "已入库",
            DlStatus::Failed => "失败",
            DlStatus::Paused => "已暂停",
            DlStatus::Cancelled => "已取消",
        }
    }
    /// 是否终态（状态不再变化，可被「清除已完成」移除）。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            DlStatus::Done | DlStatus::Imported | DlStatus::Failed | DlStatus::Cancelled
        )
    }
}

/// 运行时任务结构。`cancel` 为取消标志，不持久化。
pub struct DlTask {
    pub id: String,
    pub song_id: i64,
    pub name: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<i64>,
    pub status: DlStatus,
    pub target_path: Option<String>,
    pub progress: i64,
    pub size: i64,
    pub error: Option<String>,
    // 运行期数据（不持久化）
    pub url: String,
    pub br: u32,
    pub ext: String,
    pub cancel: Arc<AtomicBool>,
}

impl Clone for DlTask {
    fn clone(&self) -> Self {
        DlTask {
            id: self.id.clone(),
            song_id: self.song_id,
            name: self.name.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration_ms: self.duration_ms,
            status: self.status,
            target_path: self.target_path.clone(),
            progress: self.progress,
            size: self.size,
            error: self.error.clone(),
            url: self.url.clone(),
            br: self.br,
            ext: self.ext.clone(),
            cancel: self.cancel.clone(),
        }
    }
}

/// 对外视图（API 返回）：含中文状态，不含 `cancel`/`url`/`br` 等运行细节。
#[derive(Clone, Serialize)]
pub struct DlTaskView {
    pub id: String,
    pub song_id: i64,
    pub name: String,
    pub artist: String,
    pub album: String,
    pub status: DlStatus,
    pub status_zh: String,
    pub target_path: Option<String>,
    pub progress: i64,
    pub size: i64,
    pub error: Option<String>,
}

impl DlTask {
    fn view(&self) -> DlTaskView {
        DlTaskView {
            id: self.id.clone(),
            song_id: self.song_id,
            name: self.name.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            status: self.status,
            status_zh: self.status.as_zh().to_string(),
            target_path: self.target_path.clone(),
            progress: self.progress,
            size: self.size,
            error: self.error.clone(),
        }
    }
}

/// 入队作业：由同步解析阶段产出（已解析好直链与元数据，未来恢复同步时填充）。
pub struct SyncJob {
    pub song_id: i64,
    pub name: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<i64>,
    pub url: String,
    pub br: u32,
    pub ext: String,
    pub size: i64,
}

/// 供 serve 的 worker 取出任务运行字段（避免暴露 DlTask 内部）。
pub struct TaskJob {
    pub song_id: i64,
    pub name: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<i64>,
    pub url: String,
    pub br: u32,
    pub ext: String,
    pub cancel: Arc<AtomicBool>,
}

/// 下载管理器：内存队列 + 全局暂停标志 + 持久化。
pub struct DownloadManager {
    tasks: Mutex<Vec<DlTask>>,
    paused: AtomicBool,
    file: PathBuf,
}

impl DownloadManager {
    pub fn new(config_path: &PathBuf) -> Self {
        let file = config_path
            .parent()
            .map(|p| p.join("download_queue.json"))
            .unwrap_or_else(|| PathBuf::from("download_queue.json"));
        let mut mgr = DownloadManager {
            tasks: Mutex::new(Vec::new()),
            paused: AtomicBool::new(false),
            file,
        };
        mgr.load();
        mgr
    }

    fn persist(&self) {
        if let Ok(g) = self.tasks.lock() {
            let serial: Vec<serde_json::Value> = g
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "id": t.id,
                        "song_id": t.song_id,
                        "name": t.name,
                        "artist": t.artist,
                        "album": t.album,
                        "duration_ms": t.duration_ms,
                        "status": t.status,
                        "target_path": t.target_path,
                        "progress": t.progress,
                        "size": t.size,
                        "error": t.error,
                        "url": t.url,
                        "br": t.br,
                        "ext": t.ext,
                    })
                })
                .collect();
            if let Ok(s) = serde_json::to_string_pretty(&serial) {
                let _ = std::fs::write(&self.file, s);
            }
        }
    }

    fn load(&mut self) {
        if let Ok(s) = std::fs::read_to_string(&self.file) {
            if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&s) {
                let mut tasks = Vec::new();
                for v in arr {
                    let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    if id.is_empty() {
                        continue;
                    }
                    let status = serde_json::from_value::<DlStatus>(
                        v.get("status").cloned().unwrap_or(serde_json::json!("Queued")),
                    )
                    .unwrap_or(DlStatus::Queued);
                    // 重启后把进行中的任务复位为排队（download/ing 残留会被落地逻辑覆盖重写）。
                    let status = if status == DlStatus::Downloading {
                        DlStatus::Queued
                    } else {
                        status
                    };
                    tasks.push(DlTask {
                        id,
                        song_id: v.get("song_id").and_then(|x| x.as_i64()).unwrap_or(0),
                        name: v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        artist: v.get("artist").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        album: v.get("album").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        duration_ms: v.get("duration_ms").and_then(|x| x.as_i64()),
                        status,
                        target_path: v
                            .get("target_path")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string()),
                        progress: v.get("progress").and_then(|x| x.as_i64()).unwrap_or(0),
                        size: v.get("size").and_then(|x| x.as_i64()).unwrap_or(0),
                        error: v.get("error").and_then(|x| x.as_str()).map(|s| s.to_string()),
                        url: v.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        br: v.get("br").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                        ext: v.get("ext").and_then(|x| x.as_str()).unwrap_or("mp3").to_string(),
                        cancel: Arc::new(AtomicBool::new(false)),
                    });
                }
                *self.tasks.lock().unwrap() = tasks;
            }
        }
    }

    /// 入队一批作业，返回实际新增数量（同 song_id 非终态任务去重）。
    pub fn enqueue(&self, jobs: &[SyncJob]) -> usize {
        let mut g = self.tasks.lock().unwrap();
        let mut queued = 0;
        for j in jobs {
            let dup = g.iter().any(|t| {
                t.song_id == j.song_id
                    && matches!(t.status, DlStatus::Queued | DlStatus::Downloading | DlStatus::Paused)
            });
            if dup {
                continue;
            }
            g.push(DlTask {
                id: format!("dl-{}-{}", j.song_id, now_ms()),
                song_id: j.song_id,
                name: j.name.clone(),
                artist: j.artist.clone(),
                album: j.album.clone(),
                duration_ms: j.duration_ms,
                status: DlStatus::Queued,
                target_path: None,
                progress: 0,
                size: j.size,
                error: None,
                url: j.url.clone(),
                br: j.br,
                ext: j.ext.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
            });
            queued += 1;
        }
        drop(g);
        self.persist();
        queued
    }

    /// 原子地取下一个待下载任务：`Queued → Downloading`，返回 id。暂停或空返回 None。
    pub fn take_next(&self) -> Option<String> {
        if self.paused.load(Ordering::Relaxed) {
            return None;
        }
        let mut g = self.tasks.lock().unwrap();
        for t in g.iter_mut() {
            if t.status == DlStatus::Queued {
                t.status = DlStatus::Downloading;
                t.progress = 0;
                return Some(t.id.clone());
            }
        }
        None
    }

    pub fn get(&self, id: &str) -> Option<DlTask> {
        self.tasks.lock().unwrap().iter().find(|t| t.id == id).cloned()
    }

    pub fn job_of(&self, id: &str) -> Option<TaskJob> {
        self.tasks
            .lock()
            .unwrap()
            .iter()
            .find(|t| t.id == id)
            .map(|t| TaskJob {
                song_id: t.song_id,
                name: t.name.clone(),
                artist: t.artist.clone(),
                album: t.album.clone(),
                duration_ms: t.duration_ms,
                url: t.url.clone(),
                br: t.br,
                ext: t.ext.clone(),
                cancel: t.cancel.clone(),
            })
    }

    pub fn set_progress(&self, id: &str, n: i64) {
        if let Some(t) = self.tasks.lock().unwrap().iter_mut().find(|t| t.id == id) {
            t.progress = n;
        }
    }

    pub fn set_size(&self, id: &str, size: i64) {
        if let Some(t) = self.tasks.lock().unwrap().iter_mut().find(|t| t.id == id) {
            t.size = size;
        }
    }

    pub fn set_status(
        &self,
        id: &str,
        status: DlStatus,
        target_path: Option<String>,
        error: Option<String>,
    ) {
        let mut g = self.tasks.lock().unwrap();
        if let Some(t) = g.iter_mut().find(|t| t.id == id) {
            t.status = status;
            if target_path.is_some() {
                t.target_path = target_path;
            }
            if error.is_some() {
                t.error = error;
            }
            if status.is_terminal() {
                t.progress = t.size;
            }
        }
        drop(g);
        if status.is_terminal() {
            self.persist();
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// 暂停全部：置全局暂停（take_next 不再派发），并对进行中任务置取消标志。
    pub fn pause_all(&self) {
        self.paused.store(true, Ordering::Relaxed);
        for t in self.tasks.lock().unwrap().iter() {
            if t.status == DlStatus::Downloading {
                t.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    /// 暂停单个：下载中→置取消标志（当前分块后停下转 Paused）；排队中→直接置 Paused。
    pub fn pause_one(&self, id: &str) {
        let mut g = self.tasks.lock().unwrap();
        if let Some(t) = g.iter_mut().find(|t| t.id == id) {
            match t.status {
                DlStatus::Downloading => t.cancel.store(true, Ordering::Relaxed),
                DlStatus::Queued => t.status = DlStatus::Paused,
                _ => {}
            }
        }
    }

    /// 开始全部：清全局暂停与所有取消标志，Paused→Queued。
    pub fn resume_all(&self) {
        self.paused.store(false, Ordering::Relaxed);
        let mut g = self.tasks.lock().unwrap();
        for t in g.iter_mut() {
            t.cancel.store(false, Ordering::Relaxed);
            if t.status == DlStatus::Paused {
                t.status = DlStatus::Queued;
            }
        }
        drop(g);
        self.persist();
    }

    /// 开始单个：清取消标志，Paused→Queued。
    pub fn resume_one(&self, id: &str) {
        let mut g = self.tasks.lock().unwrap();
        if let Some(t) = g.iter_mut().find(|t| t.id == id) {
            t.cancel.store(false, Ordering::Relaxed);
            if t.status == DlStatus::Paused {
                t.status = DlStatus::Queued;
            }
        }
        drop(g);
        self.persist();
    }

    /// 清空下载列表：取消进行中、移除全部任务。
    pub fn clear(&self) {
        self.paused.store(true, Ordering::Relaxed);
        {
            let g = self.tasks.lock().unwrap();
            for t in g.iter() {
                t.cancel.store(true, Ordering::Relaxed);
            }
        }
        *self.tasks.lock().unwrap() = Vec::new();
        self.persist();
    }

    /// 清除已完成（所有终态任务）。
    pub fn clear_completed(&self) {
        let mut g = self.tasks.lock().unwrap();
        g.retain(|t| !t.status.is_terminal());
        drop(g);
        self.persist();
    }

    /// 队列是否空闲（无排队/下载中），供前端在下载窗口展示空闲状态。
    pub fn is_idle(&self) -> bool {
        !self
            .tasks
            .lock()
            .unwrap()
            .iter()
            .any(|t| matches!(t.status, DlStatus::Queued | DlStatus::Downloading))
    }

    pub fn list(&self) -> Vec<DlTaskView> {
        self.tasks.lock().unwrap().iter().map(|t| t.view()).collect()
    }
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: i64, ext: &str) -> SyncJob {
        SyncJob {
            song_id: id,
            name: format!("Song {id}"),
            artist: "A".into(),
            album: "B".into(),
            duration_ms: Some(1000),
            url: format!("https://x/{id}.{ext}"),
            br: 320000,
            ext: ext.into(),
            size: 1000,
        }
    }

    /// 测试专用容器：为每个用例提供**独立队列文件**的 `DownloadManager`，并在结束时清理。
    ///
    /// 根因（曾导致单测偶发失败 ~40%）：`DownloadManager::new` 把队列持久化到
    /// `config_path.parent()/download_queue.json`。早先测试传入相对路径
    /// `"nonexistent_config.toml"`，其 `parent()` 为空 → 队列文件落到**当前工作目录**，
    /// 所有用例**共享同一个 `download_queue.json`**；Rust 单测默认并行执行，
    /// 一个用例入队的 song_id 被另一个用例读到 → 去重逻辑误判为重复，
    /// `enqueue` 返回 1 而不是 2，断言随机失败。
    ///
    /// 修复：每个用例使用 `std::env::temp_dir()` 下的**唯一目录**（用例名 + 进程号），
    /// 队列文件随之隔离；`Drop` 自动清理，不再在仓库里留下 `download_queue.json` 垃圾。
    struct TestMgr {
        mgr: DownloadManager,
        dir: PathBuf,
    }

    impl TestMgr {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("mt_dl_test_{}_{}", tag, std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::create_dir_all(&dir);
            let mgr = DownloadManager::new(&dir.join("config.toml"));
            TestMgr { mgr, dir }
        }
    }

    impl Drop for TestMgr {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn enqueue_and_take_next() {
        let t = TestMgr::new("enqueue_and_take_next");
        let m = &t.mgr;
        assert_eq!(m.enqueue(&[job(1, "mp3"), job(2, "flac")]), 2);
        // 去重：同 song_id 不应重复入队。
        assert_eq!(m.enqueue(&[job(1, "mp3")]), 0);
        let id = m.take_next().expect("应派发一个");
        let task = m.get(&id).unwrap();
        assert_eq!(task.status, DlStatus::Downloading);
        assert!(m.take_next().is_some());
        assert!(m.take_next().is_none());
    }

    #[test]
    fn pause_all_blocks_dispatch() {
        let t = TestMgr::new("pause_all_blocks_dispatch");
        let m = &t.mgr;
        m.enqueue(&[job(1, "mp3")]);
        m.pause_all();
        assert!(m.is_paused());
        assert!(m.take_next().is_none(), "暂停后不应派发新任务");
        m.resume_all();
        assert!(!m.is_paused());
        assert!(m.take_next().is_some());
    }

    #[test]
    fn pause_one_queued_becomes_paused() {
        let t = TestMgr::new("pause_one_queued_becomes_paused");
        let m = &t.mgr;
        let n = m.enqueue(&[job(1, "mp3"), job(2, "flac")]);
        assert_eq!(n, 2);
        // 取一个去下载（Downloading），另一个排队中（Queued）。
        let dl = m.take_next().unwrap();
        let queued = m
            .list()
            .into_iter()
            .find(|t| t.status == DlStatus::Queued)
            .unwrap();
        // 暂停进行中的那个：取消标志置位，状态仍 Downloading（worker 停下后才转 Paused）。
        m.pause_one(&dl);
        assert!(
            m.get(&dl).unwrap().cancel.load(Ordering::Relaxed),
            "进行中任务取消标志应置位"
        );
        assert_eq!(m.get(&dl).unwrap().status, DlStatus::Downloading);
        // 暂停排队的那个：直接转 Paused。
        m.pause_one(&queued.id);
        assert_eq!(m.get(&queued.id).unwrap().status, DlStatus::Paused);
        // 恢复全部：Paused → Queued，取消标志清除。
        m.resume_all();
        assert_eq!(m.get(&queued.id).unwrap().status, DlStatus::Queued);
        assert!(!m.get(&dl).unwrap().cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn clear_and_clear_completed() {
        let t = TestMgr::new("clear_and_clear_completed");
        let m = &t.mgr;
        m.enqueue(&[job(1, "mp3"), job(2, "flac")]);
        // 模拟两个完成
        let id = m.take_next().unwrap();
        m.set_status(&id, DlStatus::Imported, Some("/x/1.mp3".into()), None);
        // 清已完成
        m.clear_completed();
        assert_eq!(m.list().len(), 1, "应剩 1 个还在队列");
        // 清空
        m.clear();
        assert!(m.list().is_empty());
    }

    #[test]
    fn status_zh_labels() {
        assert_eq!(DlStatus::Queued.as_zh(), "排队中");
        assert_eq!(DlStatus::Imported.as_zh(), "已入库");
        assert!(DlStatus::Failed.is_terminal());
        assert!(!DlStatus::Downloading.is_terminal());
    }
}
