//! 导入进度上报：把「正在处理哪一首、做什么动作」实时暴露给前端。
//!
//! ## 背景
//! 早先的导入是一次性同步调用：HTTP 请求阻塞到全部完成才返回，前端只能显示
//! 「正在更新…」。曲库有上千首时，用户既不知道做到哪了，也不知道是在复制、
//! 移动还是调用 NCM 工具转换。
//!
//! ## 设计
//! - `ProgressTracker` 是**线程安全的共享状态**（内部 `Arc<Mutex<..>>`），可克隆后
//!   传入导入流程各处；导入在 `spawn_blocking` 线程里跑，REST handler 读同一份状态。
//! - 前端在导入请求进行期间**轮询** `GET /api/import-progress` 拉取快照渲染进度面板。
//! - 计数分两类，刻意区分：
//!   - **进度条**按**源文件**计（`processed / total`），一个 ncm 源文件算一个；
//!   - **结果计数**（入库/跳过/待转换/失败）也按源文件计，反映最终归宿；
//!   - **动作日志**（`recent`）按真实发生的动作记（转换 + 入库可能是两条），
//!     用来向用户交代「这一首具体做了什么」。
//!
//! 三个概念分开，用户看到的每一行都有确切含义，不会出现「进度条走了 2 格但
//! 只入了 1 首」这类困惑。

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 导入过程中对单个文件执行的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportAction {
    /// 复制到曲库。
    Copy,
    /// 移动到曲库。
    Move,
    /// 调用 NCM 工具转换（解密转码）。
    Convert,
    /// 无 NCM 工具，暂存到 `download/pendingNcm/`。
    Stage,
    /// 目标已存在且大小一致，跳过。
    Skip,
}

impl ImportAction {
    /// 供日志/调试使用的稳定键。
    pub fn as_str(&self) -> &'static str {
        match self {
            ImportAction::Copy => "copy",
            ImportAction::Move => "move",
            ImportAction::Convert => "convert",
            ImportAction::Stage => "stage",
            ImportAction::Skip => "skip",
        }
    }
}

/// 单个源文件的最终归宿。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryStatus {
    /// 已入库。
    Imported,
    /// 已存在，跳过。
    Skipped,
    /// 待转换（无 NCM 工具，暂存 pendingNcm）。
    Pending,
    /// 重复文件已挂起，等待批量对比决策。
    Held,
    /// 处理失败。
    Failed,
}

/// 动作日志条目。
#[derive(Debug, Clone, Serialize)]
pub struct ProgressEntry {
    /// **单调递增**的日志序号。
    ///
    /// 日志列表是「新的在前」，因此消费者无法靠下标判断哪些是新增的
    /// （插入会挤动既有元素的下标；列表达到上限后旧条目还会被丢弃）。
    /// 用单调序号做游标，消费方只需记住「已处理到哪个 seq」即可精确取到增量，
    /// 不会漏、不会重。
    pub seq: u64,
    /// 动作类型。
    pub action: ImportAction,
    /// 文件名（不含目录，便于阅读）。
    pub name: String,
    /// 结果状态。
    pub status: EntryStatus,
    /// 补充说明（失败原因等）。
    pub message: String,
}

/// 导入进度快照（可直接 JSON 序列化返回给前端）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportProgress {
    /// 是否正在导入。
    pub running: bool,
    /// 本次导入的来源标签（如「网易云曲库」「文件夹 xxx」）。
    pub source: String,
    /// 待处理的源文件总数。
    pub total: u32,
    /// 已处理完的源文件数。
    pub processed: u32,
    /// 当前处理中的文件名。
    pub current_file: String,
    /// 当前动作。
    pub current_action: Option<ImportAction>,
    /// 结果计数：入库。
    pub imported: u32,
    /// 结果计数：跳过。
    pub skipped: u32,
    /// 结果计数：待转换。
    pub pending: u32,
    /// 结果计数：重复挂起（等待对比决策）。
    pub held: u32,
    /// 结果计数：失败。
    pub failed: u32,
    /// 最近的动作日志（新的在前，最多保留 100 条）。
    pub recent: Vec<ProgressEntry>,
    /// 已分配的最大日志序号。消费方记录此值后，用 `seq > last_seq` 精确取增量。
    pub latest_seq: u64,
    /// 最近一次错误描述。
    pub last_error: String,
    /// 用户已请求停止（取消剩余文件、保留已入库）。前端据此显示「已停止」。
    pub cancelled: bool,
    /// 用户已请求暂停（协作式挂起导入线程）。前端据此显示「已暂停」。
    pub paused: bool,
}

/// 日志保留上限（防止长时间导入导致内存与响应体无限膨胀）。
const RECENT_LIMIT: usize = 100;

/// 进度跟踪器：可克隆的共享状态句柄。
#[derive(Clone)]
pub struct ProgressTracker {
    inner: Arc<Mutex<ImportProgress>>,
    /// 日志序号计数器（与 inner 分开存放，`start` 重置时同步归零）。
    seq: Arc<Mutex<u64>>,
    /// 停止请求标志（与 DB 锁无关，`finalize` 分桶循环间以 Relaxed 读取）。
    cancel: Arc<AtomicBool>,
    /// 暂停请求标志（与 DB 锁无关，导入检查点以 Relaxed 读取后挂起等待）。
    pause: Arc<AtomicBool>,
    /// 可选事件回调（CLI 用它逐行打印；GUI 用轮询，故不设）。
    on_event: Option<Arc<dyn Fn(&ImportProgress) + Send + Sync>>,
}

impl Default for ProgressTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressTracker {
    /// 新建一个空闲的跟踪器。
    pub fn new() -> Self {
        ProgressTracker {
            inner: Arc::new(Mutex::new(ImportProgress::default())),
            seq: Arc::new(Mutex::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            on_event: None,
        }
    }

    /// 新建跟踪器并附带事件回调：每次状态变化时以新快照调用一次。
    /// 用于 CLI 逐行打印进度。
    pub fn with_callback<F>(f: F) -> Self
    where
        F: Fn(&ImportProgress) + Send + Sync + 'static,
    {
        ProgressTracker {
            inner: Arc::new(Mutex::new(ImportProgress::default())),
            seq: Arc::new(Mutex::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            on_event: Some(Arc::new(f)),
        }
    }

    /// 开始一次导入：重置计数并置 `running=true`，同时清掉上一次的停止/暂停请求。
    pub fn start(&self, source: &str) {
        self.cancel.store(false, Ordering::Relaxed);
        self.pause.store(false, Ordering::Relaxed);
        {
            let mut g = self.inner.lock().unwrap();
            *g = ImportProgress {
                running: true,
                source: source.to_string(),
                ..Default::default()
            };
        }
        *self.seq.lock().unwrap() = 0;
        self.emit();
    }

    /// 更新任务来源文案：重组等长任务用它切换「扫描 / 移动 / 清理 / 回滚」阶段显示。
    pub fn set_source(&self, source: &str) {
        {
            let mut g = self.inner.lock().unwrap();
            g.source = source.to_string();
        }
        self.emit();
    }

    /// 请求停止：只置标志，不回滚已入库结果；分桶循环下一轮检查后跳过剩余候选。
    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        {
            let mut g = self.inner.lock().unwrap();
            g.cancelled = true;
        }
        self.emit();
    }

    /// 是否已请求停止（Relaxed 足够：仅作为协作式取消的提示，无数据依赖）。
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// 请求暂停：置标志供快照显示，导入线程在下一个检查点挂起等待恢复。
    /// 与停止不同，暂停不丢弃任何候选；恢复后从挂起点继续。
    pub fn request_pause(&self) {
        self.pause.store(true, Ordering::Relaxed);
        {
            let mut g = self.inner.lock().unwrap();
            g.paused = true;
        }
        self.emit();
    }

    /// 请求继续：清暂停标志并唤醒等待中的导入线程。
    pub fn request_resume(&self) {
        self.pause.store(false, Ordering::Relaxed);
        {
            let mut g = self.inner.lock().unwrap();
            g.paused = false;
        }
        self.emit();
    }

    /// 是否已请求暂停。
    pub fn is_paused(&self) -> bool {
        self.pause.load(Ordering::Relaxed)
    }

    /// 协作式暂停检查点：若已请求暂停则阻塞等待，直到恢复或收到停止请求。
    /// 每 100ms 轮询一次标志；一旦停止请求到达也立即放行，避免暂停卡死停止操作。
    pub fn wait_if_paused(&self) {
        while self.is_paused() && !self.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// 结束导入：`running=false`。
    pub fn finish(&self) {
        {
            let mut g = self.inner.lock().unwrap();
            g.running = false;
            g.current_action = None;
        }
        self.emit();
    }

    /// 设置待处理源文件总数。
    pub fn set_total(&self, total: u32) {
        {
            let mut g = self.inner.lock().unwrap();
            g.total = total;
        }
        self.emit();
    }

    /// 标记开始处理某个源文件（只影响「当前文件」显示，不计数）。
    pub fn begin_file(&self, name: &str) {
        {
            let mut g = self.inner.lock().unwrap();
            g.current_file = name.to_string();
            g.current_action = None;
        }
        self.emit();
    }

    /// 设置当前动作（转换/复制/移动/暂存…）。
    pub fn set_action(&self, action: ImportAction) {
        {
            let mut g = self.inner.lock().unwrap();
            g.current_action = Some(action);
        }
        self.emit();
    }

    /// 追加一条动作日志。**不改变**任何计数（计数由 `end_file` 统一结算）。
    pub fn log(&self, action: ImportAction, name: &str, status: EntryStatus, message: &str) {
        let seq = {
            let mut s = self.seq.lock().unwrap();
            *s += 1;
            *s
        };
        {
            let mut g = self.inner.lock().unwrap();
            g.recent.insert(
                0,
                ProgressEntry {
                    seq,
                    action,
                    name: name.to_string(),
                    status,
                    message: message.to_string(),
                },
            );
            if g.recent.len() > RECENT_LIMIT {
                g.recent.truncate(RECENT_LIMIT);
            }
            if status == EntryStatus::Failed && !message.is_empty() {
                g.last_error = message.to_string();
            }
            g.latest_seq = seq;
        }
        self.emit();
    }

    /// 取出**序号大于 `after_seq`** 的新增日志条目，按序号升序返回。
    ///
    /// 这是消费方（CLI / GUI）读取增量的正确方式：日志列表虽然是「新的在前」，
    /// 且达到上限后旧条目会被丢弃，但序号单调递增，因此按 seq 过滤既不会漏
    /// 也不会重复。
    pub fn entries_after(&self, after_seq: u64) -> Vec<ProgressEntry> {
        let g = self.inner.lock().unwrap();
        let mut v: Vec<ProgressEntry> = g
            .recent
            .iter()
            .filter(|e| e.seq > after_seq)
            .cloned()
            .collect();
        v.sort_by_key(|e| e.seq);
        v
    }

    /// 推进进度分子：源文件进入处理流水线即 +1（阶段①收集完一个文件就调用）。
    /// 这样耗时最长的收集/转换阶段也能实时显示 progress，不再卡在 0/N。
    pub fn advance(&self) {
        {
            let mut g = self.inner.lock().unwrap();
            g.processed += 1;
        }
        self.emit();
    }

    /// 结算一个源文件的最终归宿：只更新结果计数，不再推进进度分子
    /// （进度分子由 `advance` 在收集阶段统一推进，避免重复计数）。
    pub fn end_file(&self, status: EntryStatus) {
        {
            let mut g = self.inner.lock().unwrap();
            match status {
                EntryStatus::Imported => g.imported += 1,
                EntryStatus::Skipped => g.skipped += 1,
                EntryStatus::Pending => g.pending += 1,
                EntryStatus::Held => g.held += 1,
                EntryStatus::Failed => g.failed += 1,
            }
        }
        self.emit();
    }

    /// 取当前快照。
    pub fn snapshot(&self) -> ImportProgress {
        self.inner.lock().unwrap().clone()
    }

    fn emit(&self) {
        if let Some(cb) = &self.on_event {
            cb(&self.snapshot());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_are_per_source_file_not_per_action() {
        let t = ProgressTracker::new();
        t.start("测试");
        t.set_total(1);
        t.begin_file("a.ncm");
        // 一次 ncm 源文件会先「转换」再「入库」两条动作日志……
        t.log(ImportAction::Convert, "a.ncm", EntryStatus::Imported, "转换完成");
        t.log(ImportAction::Move, "a.mp3", EntryStatus::Imported, "已入库");
        // ……但只结算为 1 个源文件：阶段①收集时推进一次进度，阶段②只记结果。
        t.advance();
        t.end_file(EntryStatus::Imported);
        let s = t.snapshot();
        assert_eq!(s.processed, 1, "进度应按源文件计（由 advance 推进）");
        assert_eq!(s.imported, 1, "结果计数应按源文件计");
        assert_eq!(s.recent.len(), 2, "动作日志保留两条");
        assert_eq!(s.total, 1);
    }

    #[test]
    fn start_resets_previous_state() {
        let t = ProgressTracker::new();
        t.start("第一次");
        t.set_total(10);
        t.end_file(EntryStatus::Imported);
        t.finish();
        // 第二次导入必须从零开始，不能残留上一次的计数。
        t.start("第二次");
        let s = t.snapshot();
        assert!(s.running);
        assert_eq!(s.source, "第二次");
        assert_eq!(s.processed, 0);
        assert_eq!(s.imported, 0);
        assert_eq!(s.total, 0);
        assert!(s.recent.is_empty());
    }

    #[test]
    fn recent_log_is_capped_and_newest_first() {
        let t = ProgressTracker::new();
        t.start("x");
        for i in 0..(RECENT_LIMIT + 10) {
            t.log(ImportAction::Copy, &format!("f{i}.mp3"), EntryStatus::Imported, "");
        }
        let s = t.snapshot();
        assert_eq!(s.recent.len(), RECENT_LIMIT, "日志长度应受限");
        assert_eq!(s.recent[0].name, format!("f{}.mp3", RECENT_LIMIT + 9), "新的在前");
    }

    #[test]
    fn failure_records_last_error() {
        let t = ProgressTracker::new();
        t.start("x");
        t.log(ImportAction::Convert, "x.ncm", EntryStatus::Failed, "转换超时");
        assert_eq!(t.snapshot().last_error, "转换超时");
    }

    #[test]
    fn entries_after_returns_increment_in_seq_order() {
        let t = ProgressTracker::new();
        t.start("x");
        t.log(ImportAction::Copy, "a.mp3", EntryStatus::Imported, "");
        t.log(ImportAction::Convert, "b.ncm", EntryStatus::Imported, "");
        t.log(ImportAction::Move, "b.mp3", EntryStatus::Imported, "");
        // 增量游标：先取前两条。
        let first = t.entries_after(0);
        assert_eq!(first.len(), 3);
        assert_eq!(first[0].name, "a.mp3");
        assert_eq!(first[1].name, "b.ncm");
        assert_eq!(first[2].name, "b.mp3");
        // 游标推进到第 2 条后，只应拿到第 3 条（不重不漏）。
        let second = t.entries_after(first[1].seq);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].name, "b.mp3");
        // 游标在最新之后 → 无新增。
        assert!(t.entries_after(t.snapshot().latest_seq).is_empty());
    }

    #[test]
    fn seq_survives_recent_trimming() {
        let t = ProgressTracker::new();
        t.start("x");
        for i in 0..(RECENT_LIMIT + 20) {
            t.log(ImportAction::Copy, &format!("f{i}.mp3"), EntryStatus::Imported, "");
        }
        // 旧的条目已被裁掉，但游标机制仍能精确取到最新的那一条。
        let last_seq = t.snapshot().latest_seq;
        let fresh = t.entries_after(last_seq - 1);
        assert_eq!(fresh.len(), 1, "应只取到最后一条");
        assert_eq!(fresh[0].name, format!("f{}.mp3", RECENT_LIMIT + 19));
    }

    #[test]
    fn start_resets_seq_counter() {
        let t = ProgressTracker::new();
        t.start("第一次");
        t.log(ImportAction::Copy, "a.mp3", EntryStatus::Imported, "");
        assert!(t.snapshot().latest_seq > 0);
        // 第二次导入：序号从 1 重新开始，消费者的旧游标不会误判为「有新增」。
        t.start("第二次");
        assert_eq!(t.snapshot().latest_seq, 0);
        assert!(t.entries_after(0).is_empty());
    }

    #[test]
    fn cancel_flag_is_reset_by_start_and_exposed_in_snapshot() {
        let t = ProgressTracker::new();
        t.start("第一次");
        assert!(!t.is_cancelled());
        assert!(!t.snapshot().cancelled);
        t.request_cancel();
        assert!(t.is_cancelled(), "停止请求应立即可见");
        assert!(t.snapshot().cancelled, "快照需带 cancelled 供前端显示");
        // 已入库的计数不受停止影响（不回滚）。
        t.end_file(EntryStatus::Imported);
        assert_eq!(t.snapshot().imported, 1);
        // 新一轮导入必须清掉上一轮停止请求。
        t.start("第二次");
        assert!(!t.is_cancelled());
        assert!(!t.snapshot().cancelled);
        assert_eq!(t.snapshot().imported, 0);
    }

    #[test]
    fn pause_flag_is_reset_by_start_and_exposed_in_snapshot() {
        let t = ProgressTracker::new();
        t.start("第一次");
        assert!(!t.is_paused());
        assert!(!t.snapshot().paused);
        t.request_pause();
        assert!(t.is_paused(), "暂停请求应立即可见");
        assert!(t.snapshot().paused, "快照需带 paused 供前端显示");
        t.request_resume();
        assert!(!t.is_paused());
        assert!(!t.snapshot().paused);
        // 新一轮导入必须清掉上一轮暂停请求。
        t.request_pause();
        t.start("第二次");
        assert!(!t.is_paused());
        assert!(!t.snapshot().paused);
    }

    #[test]
    fn advance_only_bumps_progress_end_file_only_bumps_result() {
        let t = ProgressTracker::new();
        t.start("测试");
        t.set_total(2);
        t.advance();
        assert_eq!(t.snapshot().processed, 1, "advance 推进进度分子");
        assert_eq!(t.snapshot().imported, 0, "advance 不动结果计数");
        t.end_file(EntryStatus::Imported);
        assert_eq!(t.snapshot().processed, 1, "end_file 不再重复推进进度");
        assert_eq!(t.snapshot().imported, 1, "end_file 推进结果计数");
    }
}
