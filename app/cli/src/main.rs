//! musicMT 命令行入口。子命令：scan / import / serve / ncm-organize / import-netease。
//! 说明：GUI 与 CLI 共享同一核心；CLI 主要用于 headless / 调试 / 定时任务。
//! 网易云服务端登录/下载已于 2026-10-29 移除；本地导入源仅 `import-netease`（从本地网易云曲库目录增量合并）。
use mt_core::{app, config, db, import, library, serve};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    // 版本信息：与 GUI 窗口标题、「关于」面板同源（mt_core::app 常量）。
    if cmd == "--version" || cmd == "-V" || cmd == "version" {
        println!("{}", app::full_name());
        println!("作者：{} <{}>", app::APP_AUTHOR, app::APP_EMAIL);
        println!("许可证：{}", app::APP_LICENSE);
        return Ok(());
    }

    // 「指定曲库」子命令：先初始化，再退出（docs/05 §4a/§5）。
    if cmd == "library" {
        let dir = args
            .get(2)
            .ok_or_else(|| anyhow::anyhow!("用法: mt-cli library <曲库目录> [方案] [名称]\n方案: artist_album|album|artist|flat（默认 artist_album）；名称可省略（默认用目录名）"))?;
        let scheme = args.get(3).map(|s| s.as_str()).unwrap_or("artist_album");
        let name = args.get(4).map(|s| s.as_str()).unwrap_or("");
        if !config::LibraryConfig::is_valid_scheme(scheme) {
            anyhow::bail!("未知文件结构方案：{scheme}");
        }
        let requested = PathBuf::from(dir);
        std::fs::create_dir_all(&requested)?;
        let canon = std::fs::canonicalize(&requested)?;
        let library_dir = match canon.to_string_lossy().strip_prefix(r"\\?\") {
            Some(s) => PathBuf::from(s),
            None => canon,
        };
        let cfg = config::init_library(&library_dir, scheme, name)?;
        println!("曲库已初始化：{}", library_dir.display());
        println!("  曲库名称：{}", cfg.library.name);
        println!("  文件结构方案：{}", cfg.library.file_scheme);
        println!("  配置：{}/.mt/config.toml", library_dir.display());
        return Ok(());
    }

    // 其余命令：加载两份配置（app 级 + 当前曲库级）；未指定曲库则引导。
    let loaded = config::load_startup()?;
    if !loaded.library_configured {
        eprintln!("尚未指定曲库，请先运行：mt-cli library <曲库目录>");
        std::process::exit(2);
    }
    let cfg = loaded.cfg.clone();
    let cfg_path = loaded.library_path.clone();
    let app_cfg_path = loaded.app_path.clone();
    cfg.ensure_dirs()?;
    let conn = Arc::new(Mutex::new(db::open(&cfg)?));

    match cmd {
        "scan" => {
            let c = conn.lock().unwrap();
            let stats = library::scan_library(&cfg, &c)?;
            println!(
                "扫描完成：scanned={} inserted={} updated={} removed={}",
                stats.scanned, stats.inserted, stats.updated, stats.removed
            );
            if !stats.loose_ncm.is_empty() {
                println!("发现曲库内散落的 .ncm 文件（不在 download 体系）：");
                for p in &stats.loose_ncm {
                    println!("  - {}", p);
                }
                println!("（可在 GUI 扫描后选择「移入 download 并处理」，或运行 `mt-cli ncm-organize` 处理）");
            }
        }
        "import" => {
            let src = args
                .get(2)
                .ok_or_else(|| anyhow::anyhow!("用法: mt-cli import <文件夹> [copy|move]"))?;
            let mode = match args.get(3).map(|s| s.as_str()) {
                Some("move") => import::ImportMode::Move,
                _ => import::ImportMode::Copy,
            };
            let c = conn.lock().unwrap();
            let tracker = cli_progress_tracker("导入");
            let res = import::import_folder_with_progress(
                &cfg,
                &c,
                PathBuf::from(src).as_path(),
                mode,
                &tracker,
            )?;
            tracker.finish();
            println!("导入完成：{:?}", res);
        }
        "serve" => {
            // 启动本地 HTTP 服务（REST + 静态前端 + Range 音频流）。
            let frontend = serve::resolve_frontend_dir(
                std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).as_deref(),
                std::env::current_dir().ok().as_deref(),
            );
            println!("前端目录：{}", frontend.display());
            serve::run_server_blocking(
                cfg,
                conn,
                frontend,
                cfg_path,
                app_cfg_path,
                true,
                config::ConfigStatus::from_loaded(&loaded),
            )?;
        }
        "import-netease" => {
            // 从本地网易云音乐曲库目录增量合并（本地操作，不联网/不登录，单方向只加不删）。
            let c = conn.lock().unwrap();
            let tracker = cli_progress_tracker("网易云曲库合并");
            let r = import::import_netease_library_with_progress(&cfg, &c, &tracker);
            tracker.finish();
            match r {
                Ok(res) => println!(
                    "从本地网易云曲库合并完成：入库 {}，跳过 {}，待转换 {}，失败 {}",
                    res.imported, res.skipped, res.pending, res.failed
                ),
                Err(e) => {
                    eprintln!("导入失败：{e}");
                    std::process::exit(1);
                }
            }
        }
        "ncm-organize" => {
            // 处理曲库内散落的 .ncm（移入 download 按下载逻辑处理）+ download/pendingNcm 积压。
            let res = cli_ncm_organize(&cfg, &*conn)?;
            println!(
                "散落 .ncm 处理完成：入库 {}，跳过 {}，待转换 {}，失败 {}",
                res.imported, res.skipped, res.pending, res.failed
            );
        }
        _ => {
            print_help();
        }
    }
    Ok(())
}

/// 创建一个带打印回调的进度跟踪器：把「正在处理哪一首、做什么动作」逐条打印到终端。
///
/// CLI 是单线程同步流程，直接打印比 GUI 轮询更合适。用 `\r` 覆盖同一行显示当前进度，
/// 避免大批量导入时刷屏（每行都换行会淹没终端）。
fn cli_progress_tracker(label: &str) -> mt_core::progress::ProgressTracker {
    use mt_core::progress::{EntryStatus, ImportAction};
    let label = label.to_string();
    // 日志游标：记录已打印到的 seq（后端日志「新的在前」，靠 seq 精确取增量，不重不漏）。
    let cursor = std::sync::Arc::new(std::sync::Mutex::new(0u64));
    // 上次打印的进度行文本：回调在每个事件点都会触发（begin_file/set_action/log/end_file），
    // 若每次都重画进度行，终端上会出现大量重复的「导入：N/M」。这里做去重：
    // 只有进度内容真正变化时才重画。
    let last_bar = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    mt_core::progress::ProgressTracker::with_callback(move |p| {
        let mut cursor = cursor.lock().unwrap();
        // 只对「新的动作日志」逐条打印；进度数字随动作行一起刷新。
        let mut fresh: Vec<&mt_core::progress::ProgressEntry> =
            p.recent.iter().filter(|e| e.seq > *cursor).collect();
        // 后端列表是新的在前，这里反转为「旧→新」顺序输出。
        fresh.reverse();
        for e in fresh {
            *cursor = e.seq;
            let verb = match e.action {
                ImportAction::Copy => "复制",
                ImportAction::Move => "移动",
                ImportAction::Convert => "转换",
                ImportAction::Stage => "暂存",
                ImportAction::Skip => "跳过",
            };
            let status = match e.status {
                EntryStatus::Imported => "已入库",
                EntryStatus::Skipped => "已跳过",
                EntryStatus::Pending => "待转换",
                EntryStatus::Failed => "失败",
            };
            let extra = if e.message.is_empty() || e.message == status {
                String::new()
            } else {
                format!("（{}）", e.message)
            };
            // 动作行前先清掉进度行（进度行用 \r 覆盖写在同一行），避免两行内容粘连。
            print!("\r\x1b[2K");
            println!("  [{verb}] {name} → {status}{extra}", name = e.name);
        }
        // 进度行（覆盖写）；仅在内容变化时重画，避免刷屏。
        if p.total > 0 {
            let bar = format!(
                "{}：{}/{}（入库 {} · 跳过 {} · 待转换 {} · 失败 {}）",
                label,
                p.processed,
                p.total,
                p.imported,
                p.skipped,
                p.pending,
                p.failed
            );
            let mut last = last_bar.lock().unwrap();
            if *last != bar {
                print!("\r\x1b[2K{bar}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
                *last = bar;
            }
        }
    })
}

/// 处理曲库内散落的 .ncm（移入 download 按下载逻辑处理）+ download/pendingNcm 积压。
/// - 有工具：转换 → 原始 .ncm 归档 download/ncm/ → 音频入库（song_id=NULL）。
/// - 无工具：暂存 download/pendingNcm/（待后续配置工具后自动处理）。
fn cli_ncm_organize(
    cfg: &config::Config,
    conn: &std::sync::Mutex<mt_core::db::SqliteConnection>,
) -> anyhow::Result<import::ImportResult> {
    let mut res = import::ImportResult::default();
    let c = conn.lock().unwrap();
    // 1) 先处理 download/pendingNcm 积压（此前无工具暂存的 .ncm），配置了工具即自动转换入库。
    let backlog = import::process_pending_ncm(cfg, &c);
    res.imported += backlog.imported;
    res.skipped += backlog.skipped;
    res.pending += backlog.pending;
    res.failed += backlog.failed;
    // 2) 再处理曲库内散落的 .ncm：移入 download 体系（有工具→转换入库，无工具→暂存 pendingNcm）。
    //    放在积压之后，避免刚移入 pendingNcm 的松散文件被本函数二次计数。
    let loose = library::scan_library(cfg, &c)?.loose_ncm;
    for p in &loose {
        let pb = PathBuf::from(p);
        if !pb.is_file() || pb.extension().and_then(|e| e.to_str()) != Some("ncm") {
            res.skipped += 1;
            continue;
        }
        match import::import_ncm(cfg, &c, &pb, import::ImportMode::Move) {
            Ok(import::ImportOutcome::Imported) => res.imported += 1,
            Ok(import::ImportOutcome::Skipped) => res.skipped += 1,
            Ok(import::ImportOutcome::Pending) => res.pending += 1,
            Err(e) => {
                eprintln!("散落 ncm 处理失败 {}: {}", pb.display(), e);
                res.failed += 1;
            }
        }
    }
    Ok(res)
}

fn print_help() {
    println!(
        "{} v{} 用法：\n\
         mt-cli library <目录> [方案] [名称]   指定/初始化曲库（方案: artist_album|album|artist|flat；名称可省略）\n\
         mt-cli scan                 扫描曲库并建索引\n\
         mt-cli import <文件夹> [copy|move]   从外部文件夹导入\n\
         mt-cli import-netease       从「本地网易云音乐曲库目录」增量合并（不联网/不登录）\n\
         mt-cli serve                启动本地 HTTP 服务（GUI 同源后端）\n\
         mt-cli ncm-organize         处理曲库内散落的 .ncm + download/pendingNcm 积压\n\
         mt-cli --version            显示版本号 / 作者联系方式",
        app::APP_NAME,
        app::APP_VERSION
    );
}
