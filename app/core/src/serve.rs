//! 本地 HTTP 服务（axum）：提供 REST 接口、静态前端、以及播放所需的 Range 音频流。
//! 设计要点（见 docs/02 §2.9、docs/01 FR-11）：
//! - 播放用原生 HTML5 <audio>，需要服务端支持 Range/206 断点续传（手写音频流实现）。
//! - 同一服务既承担 REST 数据接口，又承担音频文件分发，作为核心与 UI 的“文件 URL 接口”。
use crate::app;
use crate::config::{self, Config};
use crate::db;
use crate::downloads::DownloadManager;
use crate::duplicates::{self, DecisionInput, Origin};
use crate::import::{self, ImportMode};
use crate::library;
use crate::Result;
use axum::extract::{Path as AxumPath, Query, State};
use tower_http::services::fs::ServeDir;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::Router;
use rusqlite::Connection;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone)]
struct AppState {
    cfg: Arc<Mutex<Config>>,
    conn: Arc<Mutex<Connection>>,
    /// 前端磁盘目录：入口 HTML 需按当前 `[ui]` 外观内联注入首屏主题（见 index_handler）。
    frontend_dir: PathBuf,
    /// 当前曲库是否已初始化（.mt 已建）。驱动前端「初始化向导 / 主界面」两态。
    library_configured: Arc<Mutex<bool>>,
    /// 当前曲库配置路径（.mt/config.toml）。换库时更新，故用 Arc<Mutex> 包裹果。
    config_path: Arc<Mutex<PathBuf>>,
    /// app 级配置路径（exe 目录下的 kqk-app.toml）。存放应用自身设置 + 当前曲库路径。
    app_config_path: Arc<Mutex<PathBuf>>,
    /// 配置文件版本状态（生成版本 / 最低兼容版本 / 兼容判定 / 冲突原因）。
    config_status: Arc<Mutex<crate::config::ConfigStatus>>,
    /// 下载任务队列（内存 + 持久化 .mt/download_queue.json）。下载管理界面保留供未来复用
    /// （当前同步能力未启用；窗口与队列状态机保留）。
    downloads: Arc<DownloadManager>,
    /// 最近一次扫描（含启动自动扫描）发现的散落 .ncm 文件路径。
    /// status 返回给前端弹窗询问；organize 处理后清空。
    last_ncm_found: Arc<Mutex<Vec<String>>>,
    /// 导入进度（共享状态）：导入在 blocking 线程执行，前端轮询 /api/import-progress 拉取。
    import_progress: crate::progress::ProgressTracker,
}

#[derive(Deserialize)]
struct TracksQuery {
    q: Option<String>,
    /// 音质层级过滤：lossless / hires / lossless_up（无损+高解析）。空 = 不过滤。
    quality: Option<String>,
    /// 仅看已收藏。axum 的 Query 反序列化支持 bool（?favorite=true）。
    favorite: Option<bool>,
}

#[derive(Deserialize)]
struct NameQuery {
    name: String,
}

#[derive(Deserialize)]
struct LibraryDirectoryRequest {
    path: String,
    #[serde(default)]
    file_scheme: String,
    #[serde(default)]
    library_name: String,
}

/// 在阻塞线程上执行同步（DB / 网络）任务，避免阻塞 tokio 工作线程。
async fn blocking<F, T>(f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(v) => v,
        Err(e) => panic!("阻塞任务失败: {e}"),
    }
}

/// 服务启动后异步跑一次扫描（含孤儿清理），使曲库与磁盘一致（档位②：启动自动扫描）。
/// 失败仅告警，不阻断服务启动。散落 .ncm 结果写入共享列表，供前端加载时弹窗询问。
fn spawn_boot_scan(cfg: Config, conn: Arc<Mutex<Connection>>, ncm_found: Arc<Mutex<Vec<String>>>) {
    tokio::spawn(async move {
        match blocking(move || {
            let c = conn.lock().unwrap();
            library::scan_library(&cfg, &c)
        })
        .await
        {
            Ok(stats) => {
                *ncm_found.lock().unwrap() = stats.loose_ncm;
            }
            Err(e) => eprintln!("启动自动扫描失败：{e}"),
        }
    });
}

/// 统一错误 JSON 响应。
fn err_json(msg: &str) -> Response {
    axum::Json(serde_json::json!({ "ok": false, "error": msg })).into_response()
}

/// 原生窗口标题栏颜色回调（GUI 壳注册）。
///
/// WebView 内的 `.topbar` 由 CSS 变量 `--primary` 着色，而 Windows 原生标题栏由 DWM 绘制，
/// 两者默认割裂。前端在每次外观变化后把固定强调色（FR-29 起不再由配置 / 皮肤派生）
/// POST 到 `/api/appearance/titlebar`，本钩子转交给 GUI 壳调用 DWM 把标题栏染成同色。
/// 用无捕获的 `fn` 指针而非闭包，保证跨线程调用安全；CLI `serve` 模式未注册，接口静默空转。
static TITLEBAR_COLOR_SINK: OnceLock<fn(u32, bool)> = OnceLock::new();

/// 由 GUI 壳在窗口创建后注册；重复注册忽略（OnceLock 语义）。
pub fn set_titlebar_color_sink(f: fn(u32, bool)) {
    let _ = TITLEBAR_COLOR_SINK.set(f);
}

/// 原生文件夹/文件对话框（rfd）在 Windows 上关闭后，WebView 常未能自动重获焦点，
/// 表现为「主窗口无响应 / 点击不生效 / 交互紊乱」。此钩子在对话框结束后调用，
/// 由 GUI 壳把主窗口带回前台并恢复焦点。无捕获 `fn` 指针保证跨线程安全；
/// CLI `serve` 模式未注册则空转。
static POST_DIALOG_FOCUS_SINK: OnceLock<fn()> = OnceLock::new();

/// 由 GUI 壳在窗口创建后注册；重复注册忽略（OnceLock 语义）。
pub fn set_post_dialog_focus_sink(f: fn()) {
    let _ = POST_DIALOG_FOCUS_SINK.set(f);
}

/// 在原生对话框结束后调用：把焦点交还给主窗口（GUI 壳负责具体实现，未注册则空转）。
fn restore_focus_after_dialog() {
    if let Some(sink) = POST_DIALOG_FOCUS_SINK.get() {
        sink();
    }
}

/// #32：无边框窗口自绘控制条回调（GUI 壳注册）。
/// 白名单动作由 /api/window/control 校验后转交，CLI serve 模式未注册则静默空转。
static WINDOW_CONTROL_SINK: OnceLock<fn(&'static str)> = OnceLock::new();

pub fn set_window_control_sink(f: fn(&'static str)) {
    let _ = WINDOW_CONTROL_SINK.set(f);
}

/// 托盘状态（前端事件驱动推送，零轮询）。
#[derive(Debug, Clone, Deserialize)]
pub struct TrayState {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub muted: bool,
    pub has_track: bool,
}

/// 托盘菜单项文案 / 灰显状态由 GUI 壳经此 sink 更新（无捕获 `fn` 指针，跨线程安全）。
static TRAY_STATE_SINK: OnceLock<fn(TrayState)> = OnceLock::new();

/// 由 GUI 壳在创建托盘后注册（捕获托盘句柄）；重复注册忽略（OnceLock 语义）。
pub fn set_tray_state_sink(f: fn(TrayState)) {
    let _ = TRAY_STATE_SINK.set(f);
}

/// 推送托盘状态给已注册的 sink（未注册则空转）。
fn notify_tray_state(s: TrayState) {
    if let Some(sink) = TRAY_STATE_SINK.get() {
        sink(s);
    }
}

/// 隐藏态二次启动唤起 sink（GUI 壳注册：show + unminimize + set_focus）。
static FOCUS_WINDOW_SINK: OnceLock<fn()> = OnceLock::new();

/// 由 GUI 壳注册：隐藏态二次启动（/api/focus）时把主窗口显示并置前。
pub fn set_focus_window_sink(f: fn()) {
    let _ = FOCUS_WINDOW_SINK.set(f);
}

/// 托盘是否可用（GUI 壳在创建托盘后设置），经 /api/status 透出给前端做降级提示。
static TRAY_AVAILABLE_STATUS: OnceLock<bool> = OnceLock::new();

/// 由 GUI 壳注册：托盘创建成功与否（失败则前端显示降级提示，两开关强制按 false 生效）。
pub fn set_tray_available(available: bool) {
    let _ = TRAY_AVAILABLE_STATUS.set(available);
}

/// 解析 `#RRGGBB`（兼容 `#RGB`），返回 Windows COLORREF `0x00BBGGRR`。
fn parse_colorref(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    if !h.is_ascii() {
        return None; // 非 ASCII 会破坏字节切片边界，直接拒绝
    }
    let (r, g, b) = match h.len() {
        6 => (
            u8::from_str_radix(&h[0..2], 16).ok()?,
            u8::from_str_radix(&h[2..4], 16).ok()?,
            u8::from_str_radix(&h[4..6], 16).ok()?,
        ),
        3 => {
            let dup = |c: char| u8::from_str_radix(&format!("{c}{c}"), 16).ok();
            let mut it = h.chars();
            (dup(it.next()?)?, dup(it.next()?)?, dup(it.next()?)?)
        }
        _ => return None,
    };
    Some((b as u32) << 16 | (g as u32) << 8 | r as u32)
}

/// 外观同步端点：接收前端当前强调色与明暗，转交给 GUI 壳的 DWM 钩子。
async fn titlebar_color_handler(axum::Json(body): axum::Json<serde_json::Value>) -> Response {
    let parsed = body
        .get("color")
        .and_then(|v| v.as_str())
        .and_then(parse_colorref);
    let dark = body.get("dark").and_then(|v| v.as_bool()).unwrap_or(false);
    if let Some(color) = parsed {
        if let Some(sink) = TITLEBAR_COLOR_SINK.get() {
            sink(color, dark);
        }
    }
    axum::Json(serde_json::json!({ "ok": parsed.is_some() })).into_response()
}

/// #32：自绘窗口控制条。只接受四个固定动作，杜绝任意窗口操作被网页触发。
async fn window_control_handler(axum::Json(body): axum::Json<serde_json::Value>) -> Response {
    let action = match body.get("action").and_then(|v| v.as_str()) {
        Some("minimize") => "minimize",
        Some("toggle-maximize") => "toggle-maximize",
        Some("close") => "close",
        Some("drag") => "drag",
        _ => return axum::Json(serde_json::json!({ "ok": false })).into_response(),
    };
    if let Some(sink) = WINDOW_CONTROL_SINK.get() {
        sink(action);
    }
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

/// 解析前端（静态资源）目录：相对 exe 所在目录做级联查找。
/// 双击 exe 运行时 CWD 不可靠，不能写死 `./app/gui/frontend`。
/// 返回第一个含 index.html 的候选；都找不到时回退到 exe 同目录下的 frontend（ServeDir 将 404，但调用方应记录日志）。
pub fn resolve_frontend_dir(exe_dir: Option<&Path>, cwd: Option<&Path>) -> PathBuf {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(ed) = exe_dir {
        candidates.push(ed.join("frontend"));
        candidates.push(ed.join("app").join("gui").join("frontend"));
        // 运行于 target/release 时，项目根 = exe_dir/../..
        if let Some(pr) = ed.parent().and_then(|p| p.parent()) {
            candidates.push(pr.join("app").join("gui").join("frontend"));
        }
    }
    if let Some(c) = cwd {
        candidates.push(c.join("app").join("gui").join("frontend"));
    }
    candidates.push(PathBuf::from("app/gui/frontend"));
    for c in &candidates {
        if c.join("index.html").exists() {
            return c.clone();
        }
    }
    exe_dir
        .map(|d| d.join("frontend"))
        .unwrap_or_else(|| PathBuf::from("app/gui/frontend"))
}

/// 启动服务并阻塞（CLI `serve` 模式），Ctrl-C 优雅退出。
pub fn run_server_blocking(
    cfg: Config,
    conn: Arc<Mutex<Connection>>,
    frontend_dir: PathBuf,
    config_path: PathBuf,
    app_config_path: PathBuf,
    library_configured: bool,
    config_status: crate::config::ConfigStatus,
) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let ip: std::net::IpAddr = cfg
            .server
            .host
            .parse()
            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        let listener = tokio::net::TcpListener::bind((ip, cfg.server.port)).await?;
        let local = listener.local_addr()?;
        println!("musicMT 服务已启动：http://{}/", local);
        crate::log::event(&cfg, "serve", &format!("started http://{}/", local));
        let app = build_router(cfg, conn, frontend_dir, config_path, app_config_path, library_configured, config_status);
        axum::serve::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        Ok(())
    })
}

/// 在独立线程启动服务，并通过 channel 回传实际监听端口（GUI 模式用）。
pub fn spawn_server(
    port_tx: std::sync::mpsc::Sender<u16>,
    cfg: Config,
    conn: Arc<Mutex<Connection>>,
    frontend_dir: PathBuf,
    config_path: PathBuf,
    app_config_path: PathBuf,
    library_configured: bool,
    config_status: crate::config::ConfigStatus,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime 创建失败");
        rt.block_on(async move {
            let ip: std::net::IpAddr = cfg
                .server
                .host
                .parse()
                .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
            // 优先用配置端口；若被占用（例如上一次实例未退出）则回退到系统分配的空闲端口，避免 GUI 僵死。
            let bind_port = cfg.server.port;
            let listener = match tokio::net::TcpListener::bind((ip, bind_port)).await {
                Ok(l) => l,
                Err(_) => {
                    crate::log::write_to(
                        &cfg.log_path(),
                        "gui-serve",
                        &format!("port {} 被占用，回退到随机空闲端口", bind_port),
                    );
                    tokio::net::TcpListener::bind((ip, 0))
                        .await
                        .expect("端口绑定失败（含随机端口）")
                }
            };
            let local = listener.local_addr().expect("取本地地址失败");
            let _ = port_tx.send(local.port());
            crate::log::event(&cfg, "gui-serve", &format!("started http://{}/", local));
            let log_path = cfg.log_path();
            let app = build_router(cfg, conn, frontend_dir, config_path, app_config_path, library_configured, config_status);
            if let Err(e) = axum::serve::serve(listener, app).await {
                eprintln!("服务错误：{e}");
                crate::log::write_to(&log_path, "gui-serve", &format!("error: {e}"));
            }
        });
    });
}

fn build_router(
    cfg: Config,
    conn: Arc<Mutex<Connection>>,
    frontend_dir: PathBuf,
    config_path: PathBuf,
    app_config_path: PathBuf,
    library_configured: bool,
    config_status: crate::config::ConfigStatus,
) -> Router {
    let downloads = Arc::new(DownloadManager::new(&config_path));
    let last_ncm_found = Arc::new(Mutex::new(Vec::<String>::new()));
    // 启动自动扫描（含孤儿清理 + 散落 .ncm 收集），散件结果写入共享列表。
    spawn_boot_scan(cfg.clone(), conn.clone(), last_ncm_found.clone());
    let state = AppState {
        cfg: Arc::new(Mutex::new(cfg)),
        conn,
        frontend_dir: frontend_dir.clone(),
        library_configured: Arc::new(Mutex::new(library_configured)),
        config_path: Arc::new(Mutex::new(config_path)),
        app_config_path: Arc::new(Mutex::new(app_config_path)),
        config_status: Arc::new(Mutex::new(config_status)),
        downloads,
        last_ncm_found,
        import_progress: crate::progress::ProgressTracker::new(),
    };
    Router::new()
        .route("/api/tracks", get(tracks_handler))
        .route("/api/favorites", get(favorites_handler))
        .route("/api/window/control", post(window_control_handler))
        .route("/api/tracks/:id/favorite", post(track_favorite_handler))
        .route("/api/tracks/delete", post(track_delete_handler))
        .route("/api/tracks/reveal", post(track_reveal_handler))
        .route("/api/track/:id", get(track_handler))
        .route("/api/lyrics/:id", get(lyrics_handler))
        .route("/api/audio/:id", get(audio_handler))
        .route("/api/cover/:id", get(cover_handler))
        .route("/api/scan", post(scan_handler))
        // FR-22 重复文件挂起与批量对比决策（W15 角标 / W4 全部未决组一次展示）
        .route("/api/duplicates", get(duplicates_handler))
        .route("/api/duplicates/decide", post(duplicates_decide_handler))
        .route("/api/duplicates/preview/:id", get(duplicates_preview_handler))
        .route("/api/duplicates/history/clear", post(duplicates_history_clear_handler))
        .route("/api/pick-library-directory", post(pick_library_directory_handler))
        .route("/api/library-directory", post(library_directory_handler))
        .route("/api/library-name", post(library_name_handler))
        .route("/api/pick-folder", post(pick_folder_handler))
        .route("/api/pick-ncm-tool", post(pick_ncm_tool_handler))
        .route("/api/ncm-tool", post(ncm_tool_handler))
        .route("/api/ncm/organize", post(ncm_organize_handler))
        .route("/api/reorganize", post(reorganize_handler))
        .route("/api/columns", get(columns_get_handler).post(columns_post_handler))
        .route("/api/ui-prefs", get(ui_prefs_get_handler).post(ui_prefs_post_handler))
        .route("/api/import", post(import_handler))
        .route("/api/import-netease", post(import_netease_handler))
        .route("/api/import-progress", get(import_progress_handler))
        .route("/api/netease/library-dir", post(netease_library_dir_handler))
        // 曲库来源管理（app 级 [[sources]]）：静态段 /probe 先于动态段 /:id 注册。
        .route("/api/sources", get(sources_get_handler).post(sources_create_handler))
        .route("/api/sources/probe", post(sources_probe_handler))
        .route("/api/sources/:id", put(sources_update_handler).delete(sources_delete_handler))
        .route("/api/sources/:id/import", post(sources_import_handler))
        .route("/api/playlists", get(playlists_handler).post(playlist_create_handler))
        .route("/api/playlists/:id", delete(playlist_delete_handler))
        .route("/api/playlists/:id/rename", post(playlist_rename_handler))
        .route("/api/playlists/:id/tracks", get(playlist_tracks_handler))
        .route("/api/playlists/:id/add", post(playlist_add_handler))
        .route("/api/playlists/:id/remove", post(playlist_remove_handler))
        .route("/api/playlists/:id/move", post(playlist_move_handler))
        .route("/api/playlists/:id/export", post(playlist_export_handler))
        .route("/api/playlist-import", post(playlist_import_handler))
        .route("/api/pick-playlist-file", post(pick_playlist_file_handler))
        .route("/api/albums", get(albums_handler))
        .route("/api/albums/tracks", get(album_tracks_handler))
        .route("/api/artists", get(artists_handler))
        .route("/api/artists/tracks", get(artist_tracks_handler))
        .route("/api/status", get(status_handler))
        // FR-30：远程版本检测成功后写一行本地诊断日志（仅版本号，前端失败路径不发请求）。
        .route("/api/update-check/log", post(update_check_log_handler))
        .route("/api/playstate", get(playstate_get_handler).post(playstate_post_handler))
        .route("/api/focus", get(focus_handler))
        .route("/api/tray/state", post(tray_state_handler))
        .route("/api/about", get(about_handler))
        .route("/api/appearance/titlebar", post(titlebar_color_handler))
        .route("/api/downloads", get(downloads_handler))
        .route("/api/downloads/start", post(downloads_start_handler))
        .route("/api/downloads/pause", post(downloads_pause_handler))
        .route("/api/downloads/clear-completed", post(downloads_clear_completed_handler))
        .route("/api/downloads/clear", post(downloads_clear_handler))
        // 入口 HTML 由服务端注入首屏外观（权威源 = kqk-app.toml [ui]），
        // 其余静态资源仍交给 ServeDir。放在 with_state 之前以便读取 AppState。
        .route("/", get(index_handler))
        .route("/index.html", get(index_handler))
        .with_state(state)
        .fallback_service(ServeDir::new(frontend_dir))
}

/// 入口 HTML：把 app 级 `[ui]` 段的已保存外观内联进 `<head>`，让首帧就按用户主题渲染。
///
/// 背景：GUI 服务端口是动态的（端口占用时回退随机端口），而 `localStorage` 按 origin
/// 隔离（含端口），端口一变前端缓存就失效；若等 body 末尾的 `app.js` 或异步
/// `/api/status` 再应用外观，会先画出一帧 `styles.css` 的默认浅色（可见闪烁）。
///
/// 兼容：旧版 `dist/frontend/index.html` 没有占位符时原样返回，前端自然回退到
/// `localStorage` 缓存，不会因 build.rs 未同步而报错。
/// 注入值只做 JSON 序列化并把 `<` / `>` / `&` 转成 Unicode 转义，配置字符串无法提前闭合脚本块。
async fn index_handler(State(st): State<AppState>) -> Response {
    let Ok(html) = std::fs::read_to_string(st.frontend_dir.join("index.html")) else {
        return (StatusCode::NOT_FOUND, "index.html not found").into_response();
    };
    const MARKER: &str = "<!--KQK_BOOT_APPEARANCE-->";
    let html = if html.contains(MARKER) {
        let ui = st.cfg.lock().unwrap().ui.clone();
        let json = serde_json::to_string(&appearance_json(&ui)).unwrap_or_else(|_| "{}".to_string());
        let json = escape_for_inline_script(&json);
        html.replace(
            MARKER,
            &format!("<script>window.__KQK_BOOT_APPEARANCE__={json};</script>"),
        )
    } else {
        html
    };
    let mut headers = HeaderMap::new();
    // 主题变更后刷新不能拿到浏览器缓存的旧 HTML。
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    (headers, Html(html)).into_response()
}

/// 把 JSON 字面量中可能提前闭合 `<script>` 的字符转成 Unicode 转义。
/// 配置值来自用户可编辑的 TOML，不能假设它不含 HTML 元字符。
fn escape_for_inline_script(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

#[cfg(test)]
mod boot_appearance_tests {
    use super::escape_for_inline_script;

    #[test]
    fn script_terminator_is_escaped() {
        let s = escape_for_inline_script(r#"{"accent":"</script><script>alert(1)</script>&"}"#);
        assert!(!s.contains("</script>"), "不得出现可提前闭合脚本块的字面量");
        assert!(!s.contains("<script>"));
        assert!(s.contains("\\u0026"), "& 也应转义");
    }

    #[test]
    fn normal_json_is_unchanged() {
        let s = r##"{"theme":"dark","accent":"#7e57c2","saved":true}"##;
        assert_eq!(escape_for_inline_script(s), s);
    }
}

/// 单实例：第二个 GUI 进程请求「已有实例」把主窗口带回前台。
///
/// 复用 `restore_focus_after_dialog()`（原生对话框焦点恢复的 sink）——两者动作相同：
/// 由 GUI 壳调 `native_titlebar::refocus()` 执行 `SetForegroundWindow` / `SetFocus`。
/// CLI `serve` 未注册 sink 时为空操作，端点仍返回 ok，便于端到端冒烟。
/// 只监听 127.0.0.1 且无状态变更，GET 无副作用风险。
async fn focus_handler() -> Response {
    // 交互增强 b 级：隐藏态二次启动必须先把窗口显示出来再置前（否则 SetForegroundWindow 对隐藏窗口无效）。
    if let Some(sink) = FOCUS_WINDOW_SINK.get() {
        sink();
    }
    restore_focus_after_dialog();
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

/// 前端在播放状态变化（play / pause / ended / loadedmetadata / 选中曲目）时推送托盘状态。
/// 字段语义见 `TrayState`；事件驱动、零轮询。Rust 侧经 `TRAY_STATE_SINK` 更新托盘菜单项文案。
async fn tray_state_handler(
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let state = TrayState {
        title: body.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        artist: body.get("artist").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        playing: body.get("playing").and_then(|v| v.as_bool()).unwrap_or(false),
        muted: body.get("muted").and_then(|v| v.as_bool()).unwrap_or(false),
        has_track: body.get("has_track").and_then(|v| v.as_bool()).unwrap_or(false),
    };
    notify_tray_state(state);
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

async fn status_handler(State(st): State<AppState>) -> Response {
    let library_configured = *st.library_configured.lock().unwrap();
    let loose_ncm = st.last_ncm_found.lock().unwrap().clone();
    let cfg_guard = st.cfg.lock().unwrap();
    let library_dir = cfg_guard.paths.download_root.to_string_lossy().to_string();
    let file_scheme = cfg_guard.library.file_scheme.clone();
    let library_name = cfg_guard.library.name.clone();
    // NCM 开关 = tool_ready()（指定了工具即自动转换）。
    let ncm_ready = cfg_guard.ncm.tool_ready();
    let ncm_tool = cfg_guard.ncm.tool_path.to_string_lossy().to_string();
    let track_columns = cfg_guard.ui.track_columns.clone();
    let track_columns_saved = cfg_guard.ui.track_columns_saved;
    // 布局风格（4 视图各自独立）与歌曲列表列宽占比（%）。
    let layout_styles = cfg_guard.ui.layout_styles_map();
    let column_widths = cfg_guard.ui.widths_map();
    let auto_playlist_count = cfg_guard.ui.auto_playlist_count;
    // FR-24：侧栏宽度 / 收缩态 / 播放器音量（app 级 [ui]，不随曲库切换）。
    let sidebar_width = cfg_guard.ui.sidebar_width;
    let group_width = cfg_guard.ui.group_width;
    let sidebar_collapsed = cfg_guard.ui.sidebar_collapsed;
    let group_collapsed = cfg_guard.ui.group_collapsed;
    let volume = cfg_guard.ui.volume;
    let muted = cfg_guard.ui.muted;
    // FR-25：歌曲信息栏收起态（与侧栏 / 音量同属 app 级 [ui]）。
    let detail_collapsed = cfg_guard.ui.detail_collapsed;
    // FR-30：联网总闸 / 版本检测（默认全关；前端 refreshStatus() 据此决定是否发起唯一一次外部 GET）。
    let network_enabled = cfg_guard.ui.network_enabled;
    let update_check_enabled = cfg_guard.ui.update_check_enabled;
    let update_source = cfg_guard.ui.update_source.clone();
    let update_check_interval = cfg_guard.ui.update_check_interval.clone();
    // 外观 / 主题方案（配置文件为准；自定义皮肤图片本体不入配置，由前端 localStorage 兜底）。
    let appearance = serde_json::json!({
        "theme": cfg_guard.ui.theme,
        "accent": cfg_guard.ui.accent,
        "skin_type": cfg_guard.ui.skin_type,
        "skin_preset": cfg_guard.ui.skin_preset,
        "skin_accent": cfg_guard.ui.skin_accent,
        "saved": cfg_guard.ui.appearance_saved,
        "layout_skin": cfg_guard.ui.layout_skin,
        "list_density": cfg_guard.ui.list_density,
        "lyrics_opacity": cfg_guard.ui.lyrics_opacity,
        "skin_blur": cfg_guard.ui.skin_blur,
    });
    // 本地网易云曲库目录（导入源）：空 = 未设置。
    let netease_library_dir = cfg_guard.netease.library_dir.to_string_lossy().to_string();
    let pending_dir = cfg_guard.paths.pending_ncm_dir();
    // FR-22 W15：角标只读当前曲库的未决组数（换库不汇总、不自动清理）。
    let pending_duplicates = duplicates::pending_count(&cfg_guard);
    let pending_duplicates_dir = duplicates::pending_dir(&cfg_guard)
        .to_string_lossy()
        .to_string();
    drop(cfg_guard);
    // 配置文件版本状态（生成版本 / 最低兼容版本 / 兼容判定 / 冲突原因）。
    let cs = st.config_status.lock().unwrap().clone();
    // 轻量只读目录，直接在异步上下文中执行（read_dir 代价可忽略，避免引入额外future导致Handler推断失败）。
    let pending_ncm = std::fs::read_dir(&pending_dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("ncm"))
                .count()
        })
        .unwrap_or(0);
    axum::Json(
        serde_json::json!({
            "tray_available": TRAY_AVAILABLE_STATUS.get().copied().unwrap_or(false),
            "library_configured": library_configured,
            "library_dir": library_dir,
            "library_name": library_name,
            "netease_library_dir": netease_library_dir,
            "file_scheme": file_scheme,
            "ncm_ready": ncm_ready,
            "ncm_tool": ncm_tool,
            "pending_ncm": pending_ncm,
            "loose_ncm": loose_ncm,
            "pending_duplicates": pending_duplicates,
            "pending_duplicates_dir": pending_duplicates_dir,
            "track_columns": track_columns,
            "track_columns_saved": track_columns_saved,
            "layout_styles": layout_styles,
            "column_widths": column_widths,
            "appearance": appearance,
            "auto_playlist_count": auto_playlist_count,
            "sidebar_width": sidebar_width,
            "group_width": group_width,
            "sidebar_collapsed": sidebar_collapsed,
            "group_collapsed": group_collapsed,
            "volume": volume,
            "muted": muted,
            "detail_collapsed": detail_collapsed,
            "network_enabled": network_enabled,
            "update_check_enabled": update_check_enabled,
            "update_source": update_source,
            "update_check_interval": update_check_interval,
            "app_version": app::APP_VERSION,
            "min_compatible_version": app::MIN_COMPATIBLE_VERSION,
            "config_version": cs.app_config_version,
            "config_min_compatible_version": cs.app_min_compatible_version,
            "config_compat": cs.app_compat,
            "config_conflict_reason": cs.app_conflict_reason,
            "library_config_version": cs.library_config_version,
            "library_config_min_compatible_version": cs.library_min_compatible_version,
            "library_config_compat": cs.library_compat,
            "library_config_conflict_reason": cs.library_conflict_reason
        }),
    )
    .into_response()
}

/// 读取播放状态（跨重启恢复队列与位置）。失效 id → 后端返回 `state: null`，前端按「无队列」处理。
async fn playstate_get_handler(State(st): State<AppState>) -> Response {
    let cfg = st.cfg.clone();
    let conn = st.conn.clone();
    let state = blocking(move || {
        let g = cfg.lock().unwrap();
        let c = conn.lock().unwrap();
        crate::playstate::load(&g, &c)
    })
    .await;
    axum::Json(serde_json::json!({ "ok": true, "state": state })).into_response()
}

/// 写入播放状态；`{"clear": true}` 表示清空队列并删除状态文件。
/// 保存前逐个校验 queue 里的 local_id 仍在 download_records（DB 是唯一权威源）。
async fn playstate_post_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    if body.get("clear").and_then(|v| v.as_bool()) == Some(true) {
        let cfg = st.cfg.clone();
        let res = blocking(move || {
            let g = cfg.lock().unwrap();
            crate::playstate::clear(&g)
        })
        .await;
        return match res {
            Ok(()) => axum::Json(serde_json::json!({ "ok": true })).into_response(),
            Err(e) => err_json(&format!("清空播放状态失败：{e}")),
        };
    }
    let cfg = st.cfg.clone();
    let conn = st.conn.clone();
    let res = blocking(move || {
        let g = cfg.lock().unwrap();
        let c = conn.lock().unwrap();
        crate::playstate::save(&g, &c, &body)
    })
    .await;
    match res {
        Ok(()) => axum::Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => err_json(&format!("保存播放状态失败：{e}")),
    }
}

// ---------- 播放列表导入导出（FR-20，格式方案 A）----------

#[derive(Deserialize)]
struct PlaylistExportReq {
    dir: String,
}

/// 导出**单个列表的全量**（绝不导出当前播放队列，否则 n 的截断结果会污染文件）。
/// 一次产出 `.json` 无损备份 + `.m3u8` 通用交换格式（UTF-8 Extended M3U）。
async fn playlist_export_handler(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<i64>,
    axum::Json(body): axum::Json<PlaylistExportReq>,
) -> Response {
    let dir = PathBuf::from(body.dir.trim());
    if body.dir.trim().is_empty() {
        return err_json("导出目录不能为空");
    }
    let conn = st.conn.clone();
    let cfg = st.cfg.clone();
    let res = blocking(move || -> Result<serde_json::Value> {
        let c = conn.lock().unwrap();
        let g = cfg.lock().unwrap();
        let name: String = c
            .query_row("SELECT name FROM playlists WHERE id=?1", rusqlite::params![id], |r| r.get(0))
            .map_err(|_| anyhow::anyhow!("播放列表不存在或已被删除"))?;
        let tracks = db::get_playlist_tracks(&c, id)?;
        let root = g.paths.download_root.clone();
        let lib_name = g.library.name.clone();
        let backup = crate::playlist_io::build_backup(&name, &lib_name, &root, &tracks);
        let json_text = serde_json::to_string_pretty(&backup)?;
        let m3u8_text = crate::playlist_io::build_m3u8(&name, &tracks, &root);
        std::fs::create_dir_all(&dir)?;
        let (pj, pm) = crate::playlist_io::export_paths(&dir, &name);
        std::fs::write(&pj, json_text)?;
        std::fs::write(&pm, m3u8_text)?;
        Ok(serde_json::json!({
            "name": name,
            "track_count": tracks.len(),
            "files": [pj.to_string_lossy(), pm.to_string_lossy()],
        }))
    })
    .await;
    match res {
        Ok(mut v) => {
            v["ok"] = serde_json::json!(true);
            axum::Json(v).into_response()
        }
        Err(e) => err_json(&e.to_string()),
    }
}

#[derive(Deserialize)]
struct PlaylistImportReq {
    file: String,
    #[serde(default)]
    name: String,
}

/// 导入外部播放列表文件（M3U8 / JSON / CSV），匹配 → 两级去重 → **新建**列表并写 DB。
/// 未匹配跳过、不建占位；多副本取第一个并报告歧义；误合并逐条列出。
async fn playlist_import_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<PlaylistImportReq>,
) -> Response {
    let file = PathBuf::from(body.file.trim());
    let name_override = body.name.trim().to_string();
    let conn = st.conn.clone();
    let cfg = st.cfg.clone();
    let res = blocking(move || -> Result<serde_json::Value> {
        let parsed = crate::playlist_io::parse_file(&file)?;
        let c = conn.lock().unwrap();
        let g = cfg.lock().unwrap();
        let root = g.paths.download_root.clone();
        let resolved = crate::playlist_io::resolve(&c, &root, &parsed)?;
        if resolved.local_ids.is_empty() {
            anyhow::bail!("没有匹配到任何本地曲目，未创建播放列表");
        }
        let base = if name_override.is_empty() {
            parsed.name.clone().unwrap_or_default()
        } else {
            name_override
        };
        let name = crate::playlist_io::unique_name(&c, &base)?;
        let id = db::create_playlist(&c, &name)?;
        for lid in &resolved.local_ids {
            db::add_to_playlist(&c, id, *lid)?;
        }
        Ok(serde_json::json!({
            "playlist_id": id,
            "name": name,
            "total": parsed.entries.len(),
            "matched": resolved.report.matched,
            "report": resolved.report,
        }))
    })
    .await;
    match res {
        Ok(mut v) => {
            v["ok"] = serde_json::json!(true);
            axum::Json(v).into_response()
        }
        Err(e) => err_json(&e.to_string()),
    }
}

/// 原生文件对话框选择导入源（rfd）；关闭后恢复主窗口焦点，避免历史「对话框抢焦点」问题。
async fn pick_playlist_file_handler(State(st): State<AppState>) -> Response {
    let init_dir = st.cfg.lock().unwrap().paths.download_root.clone();
    let resp = match blocking(move || {
        rfd::FileDialog::new()
            .set_title("选择播放列表文件（.m3u8 / .m3u / .json / .csv）")
            .set_directory(init_dir)
            .add_filter("播放列表", &["m3u", "m3u8", "json", "csv"])
            .pick_file()
    })
    .await
    {
        Some(p) => axum::Json(serde_json::json!({
            "ok": true,
            "path": p.to_string_lossy()
        }))
        .into_response(),
        None => axum::Json(serde_json::json!({ "ok": true, "canceled": true })).into_response(),
    };
    restore_focus_after_dialog();
    resp
}

/// 「关于」信息：应用名称 / 版本号 / 作者 / 项目主页 / 许可证 / 技术栈。
/// 数据源为 `crate::app` 常量（编译期注入），与窗口标题、顶栏徽标同源，保证显示一致。
async fn about_handler() -> Response {
    axum::Json(serde_json::json!({
        "name": app::APP_NAME,
        "legacy_name": app::APP_LEGACY_NAME,
        "version": app::APP_VERSION,
        "author": app::APP_AUTHOR,
        "homepage": app::APP_HOMEPAGE,
        "license": app::APP_LICENSE,
        "description": app::APP_DESCRIPTION,
        "tech": app::APP_TECH,
    }))
    .into_response()
}

async fn tracks_handler(
    State(st): State<AppState>,
    Query(q): Query<TracksQuery>,
) -> Response {
    let conn = st.conn.clone();
    let kw = q.q.clone();
    let quality = q.quality.clone();
    let fav = q.favorite.unwrap_or(false);
    let rows = blocking(move || {
        let c = conn.lock().unwrap();
        db::list_tracks(&c, kw.as_deref(), quality.as_deref(), fav)
    })
    .await
    .unwrap_or_default();
    axum::Json(serde_json::json!({ "tracks": rows, "count": rows.len() })).into_response()
}

/// 「喜爱」独立智能列表（第 5 个一级视图）：实时从 download_records 推导收藏集合。
/// 与 FR-23 一致——只读、不写 playlists 表、不建有名列表；前端拿到的是 local_id[] 快照来源。
async fn favorites_handler(State(st): State<AppState>) -> Response {
    let conn = st.conn.clone();
    let rows = blocking(move || {
        let c = conn.lock().unwrap();
        db::list_tracks(&c, None, None, true)
    })
    .await
    .unwrap_or_default();
    axum::Json(serde_json::json!({ "tracks": rows, "count": rows.len() })).into_response()
}

async fn track_handler(State(st): State<AppState>, AxumPath(id): AxumPath<i64>) -> Response {
    let conn = st.conn.clone();
    let row = blocking(move || {
        let c = conn.lock().unwrap();
        db::get_track_by_local_id(&c, id)
    })
    .await
    .ok()
    .flatten();
    axum::Json(serde_json::json!({ "track": row })).into_response()
}

#[derive(Deserialize)]
struct TrackFavoriteReq {
    favorite: bool,
}

/// 切换某曲目的收藏标记（按 local_id，路径参数）。只写 download_records.favorite。
async fn track_favorite_handler(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<i64>,
    axum::Json(body): axum::Json<TrackFavoriteReq>,
) -> Response {
    if id <= 0 {
        return err_json("local_id 无效");
    }
    let conn = st.conn.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        db::set_favorite(&c, id, body.favorite)
    })
    .await;
    match res {
        Ok(n) if n > 0 => axum::Json(serde_json::json!({ "ok": true, "favorite": body.favorite }))
            .into_response(),
        Ok(_) => err_json("未找到该曲目"),
        Err(e) => err_json(&e.to_string()),
    }
}

#[derive(Deserialize)]
struct TrackActionReq {
    local_id: i64,
}

/// 删除歌曲（FR-21）：把文件移入 `<曲库>/.mt/trash/`，并清掉索引与所有播放列表引用。
/// 文件移动失败时索引保持不变（见 `library::trash_track` 的回滚），不做不可逆删除。
async fn track_delete_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<TrackActionReq>,
) -> Response {
    if body.local_id <= 0 {
        return err_json("local_id 无效");
    }
    let cfg = st.cfg.lock().unwrap().clone();
    let conn = st.conn.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        library::trash_track(&cfg, &c, body.local_id)
    })
    .await;
    match res {
        Ok(o) => axum::Json(serde_json::json!({
            "ok": true,
            "name": o.name,
            "trashed": o.trashed_path,
            "playlists_affected": o.playlists_affected,
        }))
        .into_response(),
        Err(e) => err_json(&e.to_string()),
    }
}

/// 在资源管理器中选中曲目文件。路径须落在当前曲库根内（`ensure_within_library` 防越界）。
/// 构造 explorer.exe 的定位参数（纯字符串，便于单测）。
///
/// explorer.exe 不按标准 `CommandLineToArgvW` 解析命令行：`Command::arg("/select,<path>")`
/// 在路径含空格时会把整段（含 `/select,` 前缀）加引号，explorer 于是把前缀当作路径的一部分，
/// 定位失败并落到默认位置 —— 这正是「打开的位置不对」的根因（曲库里 `Alan Walker`、
/// `Different World` 这类带空格的目录必然触发）。
/// 正确形式：`/select,"<path>"`，前缀在引号外、引号只包路径，用 `raw_arg` 原样传入。
/// 文件已不在磁盘时（索引残留 / 被外部移动）降级为只打开其父目录，保证至少落在正确目录。
#[cfg(windows)]
fn reveal_explorer_arg(abs: &Path, file_exists: bool) -> String {
    if file_exists {
        format!("/select,\"{}\"", abs.display())
    } else {
        let parent = abs.parent().unwrap_or(abs);
        format!("\"{}\"", parent.display())
    }
}

#[cfg(all(test, windows))]
mod reveal_tests {
    use super::reveal_explorer_arg;
    use std::path::Path;

    #[test]
    fn select_prefix_stays_outside_quotes_for_path_with_spaces() {
        let p = Path::new(r"D:\Music\Alan Walker\Different World\Faded.wav");
        let arg = reveal_explorer_arg(p, true);
        assert!(arg.starts_with("/select,\""), "前缀必须在引号外：{arg}");
        assert!(arg.ends_with('"'), "路径必须用引号闭合：{arg}");
        assert!(!arg.starts_with('"'), "整段参数不可被引号包裹：{arg}");
        assert!(arg.contains("Alan Walker\\Different World\\Faded.wav"));
    }

    #[test]
    fn missing_file_falls_back_to_parent_folder() {
        let p = Path::new(r"D:\Music\Alan Walker\Different World\Faded.wav");
        let arg = reveal_explorer_arg(p, false);
        assert!(!arg.contains("/select,"), "文件不存在时不应尝试选中：{arg}");
        assert!(arg.starts_with('"') && arg.ends_with('"'));
        assert!(arg.contains("Alan Walker\\Different World\""));
    }
}

async fn track_reveal_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<TrackActionReq>,
) -> Response {
    if body.local_id <= 0 {
        return err_json("local_id 无效");
    }
    let cfg = st.cfg.lock().unwrap().clone();
    let conn = st.conn.clone();
    let res = blocking(move || -> crate::Result<PathBuf> {
        let c = conn.lock().unwrap();
        let path = db::lookup_path_by_local_id(&c, body.local_id)
            .ok()
            .flatten()
            .ok_or_else(|| anyhow::anyhow!("曲目不存在或没有文件路径"))?;
        library::ensure_within_library(&cfg, Path::new(&path))
    })
    .await;
    match res {
        Ok(abs) => {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let mut cmd = crate::silent_command("explorer.exe");
                // raw_arg：/select, 前缀留在引号外，避免含空格路径被整段加引号后定位失败。
                cmd.raw_arg(reveal_explorer_arg(&abs, abs.is_file()));
                if let Err(e) = cmd.spawn() {
                    return err_json(&format!("无法打开资源管理器：{e}"));
                }
            }
            #[cfg(not(windows))]
            let _ = &abs;
            // 回带实际定位的绝对路径：便于端到端断言与真机排查「打开的位置不对」。
            axum::Json(serde_json::json!({
                "ok": true,
                "path": abs.display().to_string(),
            }))
            .into_response()
        }
        Err(e) => err_json(&e.to_string()),
    }
}

/// 歌词：按 local_id 取曲目文件路径 → 优先同名 `.lrc`，其次音频内嵌歌词。
/// 返回 `{ok, source, text, timed}`；无歌词时 `ok=true, text=""`（前端显示「暂无歌词」）。
async fn lyrics_handler(State(st): State<AppState>, AxumPath(id): AxumPath<i64>) -> Response {
    let conn = st.conn.clone();
    let result = blocking(move || {
        let c = conn.lock().unwrap();
        db::lookup_path_by_local_id(&c, id).ok().flatten()
    })
    .await;
    let Some(path) = result else {
        return axum::Json(serde_json::json!({ "ok": false, "text": "", "source": "" })).into_response();
    };
    let audio = PathBuf::from(&path);
    match crate::lyrics::get_lyrics(&audio) {
        Some(l) => axum::Json(serde_json::json!({
            "ok": true,
            "source": l.source.as_str(),
            "timed": l.timed,
            "text": l.text,
        }))
        .into_response(),
        None => axum::Json(serde_json::json!({
            "ok": true,
            "source": "",
            "timed": false,
            "text": "",
        }))
        .into_response(),
    }
}

/// 音频流：手写 Range/206 支持，满足 <audio> 拖拽进度。按 local_id 检索
/// （本地文件 song_id 可空，故不以 song_id 作为播放主键）。
type AudioChunk = (Vec<u8>, u64, bool, u64, u64, &'static str);

/// 手写 Range/206 读取。播放接口与 FR-22 挂起文件试听共用（见 `duplicates_preview_handler`）。
fn read_audio_range(pb: &Path, range: Option<&str>) -> Option<AudioChunk> {
    let meta = std::fs::metadata(pb).ok()?;
    let total = meta.len();
    let ext = pb
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ctype: &'static str = match ext.as_str() {
        "flac" => "audio/flac",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "ape" => "audio/x-ape",
        "wma" => "audio/x-ms-wma",
        _ => "application/octet-stream",
    };
    let (start, end, partial) = match range {
        Some(r) if r.starts_with("bytes=") => {
            let spec = &r["bytes=".len()..];
            let (s, e) = spec.split_once('-').unwrap_or(("", ""));
            let start = if s.is_empty() { None } else { s.trim().parse::<u64>().ok() };
            let end = if e.is_empty() { None } else { e.trim().parse::<u64>().ok() };
            match (start, end) {
                (Some(s), Some(e)) => (s, e.min(total.saturating_sub(1)), true),
                (Some(s), None) => (s, total.saturating_sub(1), true),
                (None, Some(e)) => (total.saturating_sub(e), total.saturating_sub(1), true),
                (None, None) => (0, total.saturating_sub(1), false),
            }
        }
        _ => (0, total.saturating_sub(1), false),
    };
    if total == 0 || start > end {
        return None;
    }
    let len = (end - start + 1) as usize;
    let mut f = std::fs::File::open(pb).ok()?;
    use std::io::{Read, Seek};
    if start > 0 && f.seek(std::io::SeekFrom::Start(start)).is_err() {
        return None;
    }
    let mut buf = vec![0u8; len];
    if f.read_exact(&mut buf).is_err() {
        return None;
    }
    Some((buf, total, partial, start, end, ctype))
}

fn audio_chunk_response(chunk: Option<AudioChunk>) -> Response {
    match chunk {
        Some((buf, total, partial, start, end, ctype)) => {
            let body = Body::from(Bytes::from(buf));
            let mut builder = Response::builder()
                .header(header::CONTENT_TYPE, ctype)
                .header(header::ACCEPT_RANGES, "bytes");
            if partial {
                builder = builder
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(
                        header::CONTENT_RANGE,
                        format!("bytes {}-{}/{}", start, end, total),
                    )
                    .header(header::CONTENT_LENGTH, end - start + 1);
            } else {
                builder = builder.status(StatusCode::OK).header(header::CONTENT_LENGTH, total);
            }
            builder.body(body).unwrap()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn audio_handler(
    State(st): State<AppState>,
    headers: HeaderMap,
    AxumPath(local_id): AxumPath<i64>,
) -> Response {
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let conn = st.conn.clone();
    let result = blocking(move || -> Option<AudioChunk> {
        let c = conn.lock().unwrap();
        let path = db::lookup_path_by_local_id(&c, local_id).ok().flatten()?;
        drop(c);
        read_audio_range(Path::new(&path), range.as_deref())
    })
    .await;
    audio_chunk_response(result)
}

/// 专辑封面：优先内嵌封面（lofty 提取），无则 204。按 local_id 检索。
async fn cover_handler(State(st): State<AppState>, AxumPath(local_id): AxumPath<i64>) -> Response {
    let conn = st.conn.clone();
    let path_opt = blocking(move || {
        let c = conn.lock().unwrap();
        db::lookup_path_by_local_id(&c, local_id).ok().flatten()
    })
    .await;
    match path_opt {
        Some(p) => match crate::audio_meta::extract_cover(std::path::Path::new(&p)) {
            Some(data) => {
                let ct = if data.len() > 3 && &data[..3] == b"\xff\xd8\xff" {
                    "image/jpeg"
                } else {
                    "image/png"
                };
                let mut resp = Response::new(axum::body::Body::from(data));
                resp.headers_mut().insert(
                    header::CONTENT_TYPE,
                    header::HeaderValue::from_static(ct),
                );
                resp
            }
            None => StatusCode::NO_CONTENT.into_response(),
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn scan_handler(State(st): State<AppState>) -> Response {
    let cfg = st.cfg.lock().unwrap().clone();
    let cfg_for_count = cfg.clone();
    let conn = st.conn.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        library::scan_library(&cfg, &c)
    })
    .await;
    match res {
        Ok(stats) => {
            // 散落 .ncm 更新共享列表（覆盖旧值），供 status/前端弹窗使用。
            *st.last_ncm_found.lock().unwrap() = stats.loose_ncm.clone();
            axum::Json(serde_json::json!({
                "ok": true, "scanned": stats.scanned,
                "inserted": stats.inserted, "updated": stats.updated, "removed": stats.removed,
                "held": stats.held,
                "pending_duplicates": duplicates::pending_count(&cfg_for_count),
                "ncm_found": stats.loose_ncm
            }))
            .into_response()
        }
        Err(e) => axum::Json(serde_json::json!({ "ok": false, "error": e.to_string() }))
            .into_response(),
    }
}

// ---------- FR-22 重复文件挂起与批量对比决策 ----------

/// 当前曲库的**全部未决组**（W4：弹窗一次展示；W15 角标按组数统计）。
/// 启动 boot scan 只静默登记，不自动弹窗（P1①）。
async fn duplicates_handler(State(st): State<AppState>) -> Response {
    let cfg = st.cfg.lock().unwrap().clone();
    let groups = duplicates::pending_groups(&cfg);
    axum::Json(serde_json::json!({
        "ok": true,
        "count": groups.len(),
        "groups": groups,
        "pending_dir": duplicates::pending_dir(&cfg).to_string_lossy(),
        "log_path": duplicates::sync_log_file(&cfg).to_string_lossy(),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct DuplicatesDecideRequest {
    #[serde(default)]
    decisions: Vec<DecisionInput>,
}

/// 提交批量决策；未列出的组保持挂起（「稍后处理」/关弹窗 = 不调此接口，W4）。
async fn duplicates_decide_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<DuplicatesDecideRequest>,
) -> Response {
    let cfg = st.cfg.lock().unwrap().clone();
    let conn = st.conn.clone();
    let decisions = body.decisions;
    let cfg2 = cfg.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        duplicates::apply_decisions(&cfg, &c, &decisions)
    })
    .await;
    match res {
        Ok(o) => axum::Json(serde_json::json!({
            "ok": true,
            "applied": o.applied,
            "failed": o.failed,
            "pending_duplicates": duplicates::pending_count(&cfg2),
        }))
        .into_response(),
        Err(e) => err_json(&e.to_string()),
    }
}

/// 挂起文件试听：只按 candidate id 从挂起组里查路径，不接受请求方传入任意路径；
/// Range/206 复用播放接口的实现。
async fn duplicates_preview_handler(
    State(st): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let cfg = st.cfg.lock().unwrap().clone();
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let result = blocking(move || -> Option<AudioChunk> {
        let p = duplicates::preview_path(&cfg, &id)?;
        read_audio_range(&p, range.as_deref())
    })
    .await;
    audio_chunk_response(result)
}

/// 设置面板「清除同步/导入历史」：清空独立的 sync-import.jsonl（决策预选随之失效）。
async fn duplicates_history_clear_handler(State(st): State<AppState>) -> Response {
    let cfg = st.cfg.lock().unwrap().clone();
    match duplicates::clear_history(&cfg) {
        Ok(()) => axum::Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => err_json(&e.to_string()),
    }
}

async fn library_directory_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<LibraryDirectoryRequest>,
) -> Response {
    let path = body.path.trim().to_string();
    if path.is_empty() {
        return err_json("曲库目录不能为空");
    }
    let scheme = if body.file_scheme.trim().is_empty() {
        "artist_album".to_string()
    } else {
        body.file_scheme.trim().to_string()
    };
    if !config::LibraryConfig::is_valid_scheme(&scheme) {
        return err_json(&format!("未知文件结构方案：{scheme}"));
    }

    let cfg_arc = st.cfg.clone();
    let conn_arc = st.conn.clone();
    let cp_arc = st.config_path.clone();
    let st_library_configured = st.library_configured.clone();

    match blocking(move || -> Result<(PathBuf, String)> {
        let requested = PathBuf::from(&path);
        std::fs::create_dir_all(&requested)?;
        if !requested.is_dir() {
            anyhow::bail!("指定路径不是目录");
        }
        // canonicalize 在 Windows 会返回带 \\?\ 前缀的 verbatim 路径，写入 config / UI / rfd 初始目录都不友好，去掉前缀。
        let canon = std::fs::canonicalize(&requested)?;
        let library_dir = match canon.to_string_lossy().strip_prefix(r"\\?\") {
            Some(s) => PathBuf::from(s),
            None => canon,
        };

        // 初始化曲库：建 .mt / config.toml / library.db / README / download 体系 / launcher 记忆。
        let new_cfg = config::init_library(&library_dir, &scheme, &body.library_name)?;
        let new_conn = db::open(&new_cfg)?;

        // 换库：更新运行态（cfg / conn / config_path / 已初始化标志）。
        *cfg_arc.lock().unwrap() = new_cfg.clone();
        *conn_arc.lock().unwrap() = new_conn;
        *cp_arc.lock().unwrap() = library_dir.join(".mt").join("config.toml");
        *st_library_configured.lock().unwrap() = true;

        Ok((library_dir, new_cfg.library.name.clone()))
    })
    .await
    {
        Ok((dir, name)) => axum::Json(serde_json::json!({
            "ok": true,
            "library_dir": dir.to_string_lossy(),
            "library_name": name
        }))
        .into_response(),
        Err(e) => err_json(&e.to_string()),
    }
}

#[derive(Deserialize)]
struct NcmToolRequest {
    path: String,
    #[serde(default)]
    copy_to_library: bool,
}

/// 指定 NCM 解密工具：copy_to_library=true 时复制到 .mt/tools 并用该路径；否则用原路径。
/// 路径写入 .mt/config.toml 的 [ncm]（docs/05 §6）。
async fn ncm_tool_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<NcmToolRequest>,
) -> Response {
    let src = body.path.trim().to_string();
    if src.is_empty() {
        return err_json("工具路径不能为空");
    }
    let src_p = PathBuf::from(&src);
    if !src_p.is_file() {
        return err_json("指定的文件不存在");
    }
    let cp = st.config_path.lock().unwrap().clone();
    // .mt/config.toml → parent(.mt) → parent(曲库根)
    let library_root = cp
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf());

    let final_path = if body.copy_to_library {
        let lib = match library_root {
            Some(l) => l,
            None => return err_json("无法确定曲库目录"),
        };
        let tools = lib.join(".mt").join("tools");
        if let Err(e) = std::fs::create_dir_all(&tools) {
            return err_json(&format!("创建 tools 目录失败：{e}"));
        }
        let dst = tools.join(src_p.file_name().unwrap_or_default());
        if let Err(e) = std::fs::copy(&src_p, &dst) {
            return err_json(&format!("复制工具失败：{e}"));
        }
        dst
    } else {
        src_p
    };

    let mut cfg = st.cfg.lock().unwrap().clone();
    cfg.ncm.tool_path = final_path.clone();
    // NCM 工具属 app 级设置 → 写 app 配置文件（kqk-app.toml），不写曲库配置。
    let app_cp = st.app_config_path.lock().unwrap().clone();
    if let Err(e) = cfg.save_app(&app_cp) {
        return err_json(&format!("保存配置失败：{e}"));
    }
    *st.cfg.lock().unwrap() = cfg.clone();

    // 指定了工具即自动转换：处理 pendingNcm 积压（此前无工具时暂存的 .ncm）。
    // 批量转换会逐首调用外部工具 → 接入进度上报，前端可显示「正在转换第 N 首」。
    let conn = st.conn.clone();
    let tracker = st.import_progress.clone();
    tracker.start("NCM 积压转换");
    let t = tracker.clone();
    let backlog = blocking(move || {
        let c = conn.lock().unwrap();
        import::process_pending_ncm_with_progress(&cfg, &c, &t)
    })
    .await;
    tracker.finish();

    axum::Json(serde_json::json!({
        "ok": true,
        "tool_path": final_path.to_string_lossy(),
        "tool_ready": true,
        "backlog": {
            "imported": backlog.imported,
            "skipped": backlog.skipped,
            "pending": backlog.pending,
            "failed": backlog.failed,
            "held": backlog.held
        }
    }))
    .into_response()
}

/// 原生文件选择：选 NCM 工具（.exe），返回绝对路径供前端再决定是否复制。
async fn pick_ncm_tool_handler(State(st): State<AppState>) -> Response {
    let init_dir: PathBuf = st
        .config_path
        .lock()
        .unwrap()
        .clone()
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let resp = match blocking(move || {
        rfd::FileDialog::new()
            .set_title("选择 NCM 解密工具（ncmdump.exe）")
            .set_directory(init_dir)
            .add_filter("可执行文件", &["exe"])
            .pick_file()
    })
    .await
    {
        Some(p) => axum::Json(serde_json::json!({
            "ok": true,
            "path": p.to_string_lossy()
        }))
        .into_response(),
        None => axum::Json(serde_json::json!({ "ok": true, "canceled": true })).into_response(),
    };
    restore_focus_after_dialog();
    resp
}

#[derive(Deserialize)]
struct NcmOrganizeRequest {
    /// 要处理的散落 .ncm 绝对路径列表（前端把扫描结果原样传回）。
    #[serde(default)]
    paths: Vec<String>,
}

/// 处理扫描发现的散落 .ncm：移入 download 体系并按下载逻辑处理
/// （有工具：转换 → 原始归档 download/ncm/ → 音频入库；无工具：暂存 download/pendingNcm/）。
/// 处理完成后清空共享散件列表；失败的文件仍在磁盘上，下次扫描会再次发现。
async fn ncm_organize_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<NcmOrganizeRequest>,
) -> Response {
    if body.paths.is_empty() {
        return err_json("未指定要处理的 .ncm 文件");
    }
    let cfg = st.cfg.lock().unwrap().clone();
    let conn = st.conn.clone();
    let paths = body.paths.clone();
    let tracker = st.import_progress.clone();
    tracker.start("散落 .ncm 整理");
    let t = tracker.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        let mut r = import::ImportResult::default();
        for p in &paths {
            let pb = PathBuf::from(p);
            if !pb.is_file() || pb.extension().and_then(|e| e.to_str()) != Some("ncm") {
                r.skipped += 1; // 已被移动/非 ncm：跳过（下次扫描自然消失）
                continue;
            }
            let fname = pb
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            t.begin_file(&fname);
            match import::import_ncm_with_progress(
                &cfg,
                &c,
                &pb,
                import::ImportMode::Move,
                Origin::NcmBackfill,
                &t,
            ) {
                Ok(import::ImportOutcome::Imported) => {
                    r.imported += 1;
                    t.end_file(crate::progress::EntryStatus::Imported);
                }
                Ok(import::ImportOutcome::Skipped) => {
                    r.skipped += 1;
                    t.end_file(crate::progress::EntryStatus::Skipped);
                }
                Ok(import::ImportOutcome::Pending) => {
                    r.pending += 1;
                    t.end_file(crate::progress::EntryStatus::Pending);
                }
                Ok(import::ImportOutcome::Held) => {
                    // 重复已挂起：落盘等 GUI 批量对比，不就地默认保留第一个（W7–W13）。
                    r.held += 1;
                    t.end_file(crate::progress::EntryStatus::Pending);
                }
                Err(e) => {
                    eprintln!("散落 ncm 处理失败 {}: {}", pb.display(), e);
                    r.failed += 1;
                    t.log(
                        crate::progress::ImportAction::Convert,
                        &fname,
                        crate::progress::EntryStatus::Failed,
                        &e.to_string(),
                    );
                    t.end_file(crate::progress::EntryStatus::Failed);
                }
            }
        }
        r
    })
    .await;
    tracker.finish();
    // 清空共享散件列表：已处理的自然消失；失败/用户遗留的由下次扫描重新发现。
    st.last_ncm_found.lock().unwrap().clear();
    axum::Json(serde_json::json!({
        "ok": true,
        "imported": res.imported,
        "skipped": res.skipped,
        "pending": res.pending,
        "failed": res.failed,
        "held": res.held
    }))
    .into_response()
}

#[derive(Deserialize)]
struct ReorganizeRequest {
    scheme: String,
}

/// 切换文件结构方案时全量重组（docs/05 §4c）：按新方案移动文件并更新 DB，失败回滚。
async fn reorganize_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<ReorganizeRequest>,
) -> Response {
    let scheme = body.scheme.trim().to_string();
    if !config::LibraryConfig::is_valid_scheme(&scheme) {
        return err_json(&format!("未知文件结构方案：{scheme}"));
    }
    let cfg = st.cfg.lock().unwrap().clone();
    let conn = st.conn.clone();
    let cp_arc = st.config_path.clone();
    let scheme_inner = scheme.clone();
    match blocking(move || -> Result<serde_json::Value> {
        let c = conn.lock().unwrap();
        let rows = db::list_tracks(&c, None, None, false)?;
        // 以新方案计算目标：先切换到新 scheme。
        let mut cfg = cfg;
        cfg.library.file_scheme = scheme_inner;
        let mut moved: u32 = 0;
        let mut mapping: Vec<(i64, PathBuf, PathBuf)> = Vec::new();
        for t in rows {
            let old = PathBuf::from(&t.file_path);
            if !old.exists() {
                continue;
            }
            let ext = old
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let new = library::target_path(
                &cfg,
                t.artist.as_deref().unwrap_or(""),
                t.album.as_deref().unwrap_or(""),
                &t.name,
                &ext,
            );
            if new == old {
                continue;
            }
            if let Some(parent) = new.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if let Err(e) = std::fs::rename(&old, &new) {
                rollback_reorg(&c, &mapping);
                anyhow::bail!("重组移动失败，已回滚：{e}");
            }
            if let Err(e) = db::update_file_path(&c, t.local_id, &new.to_string_lossy()) {
                let _ = std::fs::rename(&new, &old);
                rollback_reorg(&c, &mapping);
                anyhow::bail!("重组更新数据库失败，已回滚：{e}");
            }
            mapping.push((t.local_id, old, new));
            moved += 1;
        }
        drop(c);
        // 持久化新 scheme 到曲库配置 <曲库>/.mt/config.toml（曲库级）。
        let cp = cp_arc.lock().unwrap().clone();
        cfg.save_library(&cp)?;
        Ok(serde_json::json!({ "ok": true, "moved": moved }))
    })
    .await
    {
        Ok(v) => {
            // 同步运行态 scheme。
            let mut g = st.cfg.lock().unwrap();
            g.library.file_scheme = scheme;
            axum::Json(v).into_response()
        }
        Err(e) => err_json(&e.to_string()),
    }
}

/// 重组失败回滚：把已移动的文件还原并更新 DB。
fn rollback_reorg(conn: &Connection, mapping: &[(i64, PathBuf, PathBuf)]) {
    for (id, old, new) in mapping {
        let _ = std::fs::rename(new, old);
        let _ = db::update_file_path(conn, *id, &old.to_string_lossy());
    }
}

async fn pick_library_directory_handler(State(st): State<AppState>) -> Response {
    let current_dir = st.cfg.lock().unwrap().paths.download_root.clone();
    let resp = match blocking(move || {
        rfd::FileDialog::new()
            .set_title("选择本地曲库目录")
            .set_directory(current_dir)
            .pick_folder()
    })
    .await
    {
        Some(path) => axum::Json(serde_json::json!({
            "ok": true,
            "path": path.to_string_lossy()
        }))
        .into_response(),
        None => axum::Json(serde_json::json!({
            "ok": true,
            "canceled": true
        }))
        .into_response(),
    };
    restore_focus_after_dialog();
    resp
}

/// 读取当前歌曲列表显示列（有序可见列 key）。
async fn columns_get_handler(State(st): State<AppState>) -> Response {
    let cols = st.cfg.lock().unwrap().ui.track_columns.clone();
    axum::Json(serde_json::json!({ "ok": true, "columns": cols })).into_response()
}

/// 保存歌曲列表显示列（有序可见列 key 列表，顺序即显示顺序）。
async fn columns_post_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let allowed: std::collections::HashSet<String> =
        crate::config::all_track_column_keys().into_iter().collect();
    let cols: Vec<String> = body
        .get("columns")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .filter(|s| allowed.contains(s))
                .collect()
        })
        .unwrap_or_default();
    if cols.is_empty() {
        return axum::Json(serde_json::json!({ "ok": false, "error": "至少保留一列" }))
            .into_response();
    }
    // 显示列属 app 级偏好 → 写 app 配置文件（kqk-app.toml）。
    let app_cp = st.app_config_path.lock().unwrap().clone();
    {
        let mut g = st.cfg.lock().unwrap();
        g.ui.track_columns = cols.clone();
        // 打上「用户显式保存过」标记：后续启动时不再自动回填任何时间列，
        // 用户主动隐藏导入时间 / 主动打开创建时间的选择都能被完整保留。
        g.ui.track_columns_saved = true;
        g.ui.sanitize();
        // 与布局 / 列宽一致走「[ui] 段就地更新」：不要 save_app() 整份重写，
        // 否则用户在 app 配置里手写的注释 / 未知键会被这次勾选操作抹掉。
        if let Err(e) = config::save_ui_in_place(&app_cp, &g.ui) {
            return axum::Json(serde_json::json!({ "ok": false, "error": format!("保存失败：{e}") }))
                .into_response();
        }
    }
    axum::Json(serde_json::json!({ "ok": true, "columns": cols })).into_response()
}

// ---------- FR-30：版本检测诊断日志 ----------

/// 检测成功后把「本地版本 / 远端版本」写一行到 `<曲库>/.mt/app.log`。
///
/// 前端只在**检测成功**时发此请求；失败 / 超时 / 超限 / 非法 tag 一律静默不发，
/// 因此日志里不会出现网络噪声。请求体只接受两个三段数字版本号（可带 `v` 前缀、长度 ≤ 24），
/// 其它内容一律忽略——防止把任意文本写进本地日志，也杜绝任何曲库 / 机器 / 用户信息流入。
async fn update_check_log_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    fn clean_version(v: Option<&serde_json::Value>) -> Option<String> {
        let s = v?.as_str()?;
        if s.len() > 24 { return None; }
        let t = s.trim().strip_prefix('v').unwrap_or_else(|| s.trim());
        let parts: Vec<&str> = t.split('.').collect();
        let valid = parts.len() == 3
            && parts.iter().all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()));
        if valid { Some(t.to_string()) } else { None }
    }
    if let (Some(local), Some(remote)) = (clean_version(body.get("local")), clean_version(body.get("remote"))) {
        let cfg = st.cfg.lock().unwrap();
        crate::log::event(&cfg, "update-check", &format!("local={local} remote={remote}"));
    }
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

// ---------- 界面偏好（布局风格 + 列宽比例）----------

/// 读取界面偏好：各视图布局风格（1=横向 / 2=纵向）+ 歌曲列表各列宽度占比（%，总和 100）
/// + 外观主题方案（theme / skin_* / lyrics_opacity / skin_blur；accent / skin_accent 为 #37 恢复的主题色字段，
///   前端首屏脚本 / applyAppearance() 消费；自定义皮肤图片本体不落配置）+ FR-30 联网四键（network_enabled / update_check_enabled /
///   update_source / update_check_interval，默认全关）。
async fn ui_prefs_get_handler(State(st): State<AppState>) -> Response {
    let ui = st.cfg.lock().unwrap().ui.clone();
    axum::Json(serde_json::json!({
        "ok": true,
        "layout_styles": ui.layout_styles_map(),
        "column_widths": ui.widths_map(),
        "appearance": appearance_json(&ui),
        "auto_playlist_count": ui.auto_playlist_count,
        "sidebar_width": ui.sidebar_width,
        "group_width": ui.group_width,
        "sidebar_collapsed": ui.sidebar_collapsed,
        "group_collapsed": ui.group_collapsed,
        "volume": ui.volume,
        "muted": ui.muted,
        "detail_collapsed": ui.detail_collapsed,
        "lyrics_opacity": ui.lyrics_opacity,
        "skin_blur": ui.skin_blur,
        "layout_skin": ui.layout_skin,
        "list_density": ui.list_density,
        // FR-30：联网总闸 / 版本检测四键随偏好一起读回。
        "network_enabled": ui.network_enabled,
        "update_check_enabled": ui.update_check_enabled,
        "update_source": ui.update_source,
        "update_check_interval": ui.update_check_interval,
        // 交互增强 b 级：托盘最小化 / 关闭行为开关。
        "close_to_tray": ui.close_to_tray,
        "minimize_to_tray": ui.minimize_to_tray,
    }))
    .into_response()
}

/// 把 `UiConfig` 里的外观字段打包成前端可直接消费的对象。
fn appearance_json(ui: &config::UiConfig) -> serde_json::Value {
    serde_json::json!({
        "theme": ui.theme,
        "accent": ui.accent,
        "skin_type": ui.skin_type,
        "skin_preset": ui.skin_preset,
        "skin_accent": ui.skin_accent,
        "saved": ui.appearance_saved,
        // 布局：classic / modern（与底层背景皮肤 skin_type 正交）。
        "layout_skin": ui.layout_skin,
        "list_density": ui.list_density,
        // FR-29：两个可调数值也走同一份注入（首屏脚本据此预设 CSS 变量，避免启动闪烁）。
        "lyrics_opacity": ui.lyrics_opacity,
        "skin_blur": ui.skin_blur,
    })
}

/// 请求体（三段均可选，只提交需要变更的部分）：
/// ```json
/// { "layout_styles": { "library": 1, "playlists": 2 },
///   "column_widths": { "name": 34.5 },
///   "appearance": { "theme": "dark", "accent": "#7e57c2",
///                   "skin_type": "preset", "skin_preset": "purple", "skin_accent": "#7e57c2",
///                   "lyrics_opacity": 82, "skin_blur": 18 } }
/// ```
/// - `layout_styles`：view → 1|2，view 取 `library` / `playlists` / `albums` / `artists`；
///   非法 view 或非法风格值忽略（不报错，避免前端脏数据写坏配置）。
/// - `column_widths`：列 key → 占比（%）；写入后统一归一化（总和 100，单列 ≥ 4%）。
/// - `appearance`：主题方案；非法值交给 `UiConfig::sanitize()` 回退，空串表示恢复默认。
///   `accent` / `skin_accent` 自 #37 起恢复消费（手动主题色 / 皮肤自动取色，均为 #RRGGBB）；
///   `lyrics_opacity` / `skin_blur` 为数值型，缺失时保持原值，越界由 `config::clamp_*` 钳制。
/// - `network_enabled` / `update_check_enabled`：布尔总闸与子项（FR-30，默认 false）。
/// - `update_source` / `update_check_interval`：枚举字符串，非法值由 `config::clamp_update_*` 回退默认。
///   全部为部分更新语义，缺失键保持原值。
///
/// 落盘走 `config::save_ui_in_place`：只就地替换 `[ui]` 段，其余内容（注释 / 未知键 / 键序）原样保留。
async fn ui_prefs_post_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let app_cp = st.app_config_path.lock().unwrap().clone();
    let mut g = st.cfg.lock().unwrap();
    let mut ui = g.ui.clone();

    // 布局风格
    if let Some(obj) = body.get("layout_styles").and_then(|v| v.as_object()) {
        for (view, val) in obj {
            if let Some(style) = val.as_u64() {
                ui.set_layout(view, style as u8);
            }
        }
    }
    // 列宽占比
    if let Some(obj) = body.get("column_widths").and_then(|v| v.as_object()) {
        for (key, val) in obj {
            if let Some(w) = val.as_f64() {
                ui.set_width(key, w);
            }
        }
    }
    // 外观 / 主题方案：逐键取字符串，缺失的键保持原值（部分更新语义）。
    // 非法值不在这里硬拒绝，统一由 ui.sanitize() 回退到安全默认值。
    if let Some(obj) = body.get("appearance").and_then(|v| v.as_object()) {
        for (key, target) in [
            ("theme", &mut ui.theme),
            ("accent", &mut ui.accent),
            ("skin_type", &mut ui.skin_type),
            ("skin_preset", &mut ui.skin_preset),
            ("skin_accent", &mut ui.skin_accent),
        ] {
            if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
                *target = v.to_string();
            }
        }
        // FR-29：两个滑块数值（部分更新语义：缺失的键保持原值）。
        // 接受 Number / 数字字符串，坏值与越界由 config::clamp_* 统一钳制，不报错。
        if let Some(v) = obj.get("lyrics_opacity") {
            let n = match v {
                serde_json::Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
                serde_json::Value::String(s) => s.trim().parse::<f64>().unwrap_or(f64::NAN),
                _ => f64::NAN,
            };
            ui.lyrics_opacity = config::clamp_lyrics_opacity(n);
        }
        if let Some(v) = obj.get("skin_blur") {
            let n = match v {
                serde_json::Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
                serde_json::Value::String(s) => s.trim().parse::<f64>().unwrap_or(f64::NAN),
                _ => f64::NAN,
            };
            ui.skin_blur = config::clamp_skin_blur(n);
        }
        // 收到外观请求即视为「已显式保存过」，用于旧配置的一次性迁移判定。
        ui.appearance_saved = true;
    }
    // 自动播放列表上限 n：数字或数字字符串都接受，坏值回退默认 20、越界钳制到 [1, 200]。
    // 这是应用设置，与曲库无关，走同一处 [ui] 段就地更新。
    if let Some(v) = body.get("auto_playlist_count") {
        let n = match v {
            serde_json::Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
            serde_json::Value::String(s) => s.trim().parse::<f64>().unwrap_or(f64::NAN),
            _ => f64::NAN,
        };
        ui.auto_playlist_count = config::clamp_auto_playlist_count(n);
    }
    // FR-24：侧栏宽度 / 收缩态 / 音量，部分更新语义；越界值由 ui.sanitize() 回退。
    if let Some(v) = body.get("sidebar_width").and_then(|v| v.as_u64()) {
        ui.sidebar_width = v.min(u32::MAX as u64) as u32;
    }
    if let Some(v) = body.get("group_width").and_then(|v| v.as_u64()) {
        ui.group_width = v.min(u32::MAX as u64) as u32;
    }
    if let Some(v) = body.get("sidebar_collapsed").and_then(|v| v.as_bool()) {
        ui.sidebar_collapsed = v;
    }
    if let Some(v) = body.get("group_collapsed").and_then(|v| v.as_bool()) {
        ui.group_collapsed = v;
    }
    if let Some(v) = body.get("volume").and_then(|v| v.as_u64()) {
        ui.volume = v.min(u32::MAX as u64) as u32;
    }
    if let Some(v) = body.get("muted").and_then(|v| v.as_bool()) {
        ui.muted = v;
    }
    // FR-25：详情栏收缩态，部分更新语义（不影响其它 [ui] 键）。
    if let Some(v) = body.get("detail_collapsed").and_then(|v| v.as_bool()) {
        ui.detail_collapsed = v;
    }
    // FR-30：联网总闸 / 版本检测。部分更新语义；两个布尔只做严格 bool 判定，
    // 枚举字符串由 config::clamp_update_* 钳制（非法值回退已登记默认值），不会拼出意外 URL。
    // 注意：总闸关闭**不清零** update_check_enabled（A3：子项值保留、仅灰显禁用）。
    if let Some(v) = body.get("network_enabled").and_then(|v| v.as_bool()) {
        ui.network_enabled = v;
    }
    if let Some(v) = body.get("update_check_enabled").and_then(|v| v.as_bool()) {
        ui.update_check_enabled = v;
    }
    if let Some(v) = body.get("update_source").and_then(|v| v.as_str()) {
        ui.update_source = config::clamp_update_source(v);
    }
    if let Some(v) = body.get("update_check_interval").and_then(|v| v.as_str()) {
        ui.update_check_interval = config::clamp_update_interval(v);
    }
    // 交互增强 b 级：托盘最小化 / 关闭行为开关（部分更新语义，严格 bool 判定）。
    if let Some(v) = body.get("close_to_tray").and_then(|v| v.as_bool()) {
        ui.close_to_tray = v;
    }
    if let Some(v) = body.get("minimize_to_tray").and_then(|v| v.as_bool()) {
        ui.minimize_to_tray = v;
    }
    // 布局：classic / modern（部分更新语义，非法值由 sanitize() 回退默认）。
    if let Some(v) = body.get("layout_skin").and_then(|v| v.as_str()) {
        if matches!(v, "classic" | "modern") {
            ui.layout_skin = v.to_string();
        }
    }
    // 列表密度：comfortable / compact（部分更新语义，非法值由 sanitize() 回退默认）。
    if let Some(v) = body.get("list_density").and_then(|v| v.as_str()) {
        if matches!(v, "comfortable" | "compact") {
            ui.list_density = v.to_string();
        }
    }
    ui.sanitize();

    // 写盘（就地更新 [ui] 段）→ 失败则不污染内存状态。
    if let Err(e) = config::save_ui_in_place(&app_cp, &ui) {
        return axum::Json(serde_json::json!({ "ok": false, "error": format!("保存失败：{e}") }))
            .into_response();
    }
    g.ui = ui.clone();
    axum::Json(serde_json::json!({
        "ok": true,
        "layout_styles": ui.layout_styles_map(),
        "column_widths": ui.widths_map(),
        "appearance": appearance_json(&ui),
        "auto_playlist_count": ui.auto_playlist_count,
        "sidebar_width": ui.sidebar_width,
        "group_width": ui.group_width,
        "sidebar_collapsed": ui.sidebar_collapsed,
        "group_collapsed": ui.group_collapsed,
        "volume": ui.volume,
        "muted": ui.muted,
        "detail_collapsed": ui.detail_collapsed,
        "lyrics_opacity": ui.lyrics_opacity,
        "skin_blur": ui.skin_blur,
        "layout_skin": ui.layout_skin,
        "list_density": ui.list_density,
        // FR-30：联网总闸 / 版本检测四键随偏好一起读回。
        "network_enabled": ui.network_enabled,
        "update_check_enabled": ui.update_check_enabled,
        "update_source": ui.update_source,
        "update_check_interval": ui.update_check_interval,
        // 交互增强 b 级：托盘最小化 / 关闭行为开关。
        "close_to_tray": ui.close_to_tray,
        "minimize_to_tray": ui.minimize_to_tray,
    }))
    .into_response()
}

// ---------- 下载管理（队列控制）----------

async fn downloads_handler(State(st): State<AppState>) -> Response {
    let tasks = st.downloads.list();
    axum::Json(serde_json::json!({
        "tasks": tasks,
        "paused": st.downloads.is_paused()
    }))
    .into_response()
}

#[derive(Deserialize)]
struct DlControl {
    #[serde(default)]
    id: Option<String>,
}

async fn downloads_start_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<DlControl>,
) -> Response {
    match body.id {
        Some(id) => st.downloads.resume_one(&id),
        None => st.downloads.resume_all(),
    }
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

async fn downloads_pause_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<DlControl>,
) -> Response {
    match body.id {
        Some(id) => st.downloads.pause_one(&id),
        None => st.downloads.pause_all(),
    }
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

async fn downloads_clear_completed_handler(State(st): State<AppState>) -> Response {
    st.downloads.clear_completed();
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

async fn downloads_clear_handler(State(st): State<AppState>) -> Response {
    st.downloads.clear();
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

/// 通用文件夹选择：弹出原生目录选择对话框，返回绝对路径（导入源/曲库目录等复用）。
async fn pick_folder_handler(State(st): State<AppState>) -> Response {
    let current_dir = st.cfg.lock().unwrap().paths.download_root.clone();
    let resp = match blocking(move || {
        rfd::FileDialog::new()
            .set_title("选择文件夹")
            .set_directory(current_dir)
            .pick_folder()
    })
    .await
    {
        Some(path) => axum::Json(serde_json::json!({
            "ok": true,
            "path": path.to_string_lossy()
        }))
        .into_response(),
        None => axum::Json(serde_json::json!({
            "ok": true,
            "canceled": true
        }))
        .into_response(),
    };
    restore_focus_after_dialog();
    resp
}

/// 修改曲库名称并写回 .mt/config.toml（多曲库 / 多用户场景下区分曲库，docs/05）。
async fn library_name_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return err_json("曲库名称不能为空");
    }
    // 限制长度，避免极端输入影响 UI / 配置可读性。
    let name = name.chars().take(120).collect::<String>();
    // 曲库名称属曲库级设置 → 写 <曲库>/.mt/config.toml。
    let cp = st.config_path.lock().unwrap().clone();
    let mut cfg = st.cfg.lock().unwrap().clone();
    cfg.library.name = name.clone();
    if let Err(e) = cfg.save_library(&cp) {
        return err_json(&format!("保存配置失败：{e}"));
    }
    *st.cfg.lock().unwrap() = cfg.clone();
    axum::Json(serde_json::json!({
        "ok": true,
        "library_name": cfg.library.name
    }))
    .into_response()
}

/// 保存「本地网易云音乐曲库目录」（导入源）。
async fn netease_library_dir_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let dir = body
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if dir.is_empty() {
        return err_json("曲库目录不能为空");
    }
    let path = PathBuf::from(&dir);
    if !path.is_dir() {
        return err_json("指定路径不是已存在的目录");
    }
    // 本地网易云导入源属 app 级设置 → 写 app 配置文件（kqk-app.toml）。
    let app_cp = st.app_config_path.lock().unwrap().clone();
    let mut cfg = st.cfg.lock().unwrap().clone();
    cfg.netease.library_dir = path;
    if let Err(e) = cfg.save_app(&app_cp) {
        return err_json(&format!("保存配置失败：{e}"));
    }
    *st.cfg.lock().unwrap() = cfg.clone();
    axum::Json(serde_json::json!({
        "ok": true,
        "library_dir": cfg.netease.library_dir.to_string_lossy()
    }))
    .into_response()
}

/// 从本地网易云音乐曲库目录增量合并（本地操作，不联网/不登录）：
/// 仅把曲库中没有的歌曲复制进来，绝不反向删除原曲库文件（docs/02 FR-12、docs/05 §13）。
///
/// 导入在 blocking 线程执行，期间把逐文件进度写入共享 tracker，
/// 前端轮询 `GET /api/import-progress` 渲染实时进度。
async fn import_netease_handler(State(st): State<AppState>) -> Response {
    let cfg = st.cfg.lock().unwrap().clone();
    let conn = st.conn.clone();
    let tracker = st.import_progress.clone();
    tracker.start("本地网易云曲库");
    let t = tracker.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        import::import_netease_library_with_progress(&cfg, &c, &t)
    })
    .await;
    tracker.finish();
    match res {
        Ok(r) => axum::Json(serde_json::json!({
            "ok": true,
            "imported": r.imported,
            "skipped": r.skipped,
            "pending": r.pending,
            "failed": r.failed,
            "held": r.held
        }))
        .into_response(),
        Err(e) => axum::Json(serde_json::json!({ "ok": false, "error": e.to_string() }))
            .into_response(),
    }
}

/// 查询当前导入进度（前端轮询）。
async fn import_progress_handler(State(st): State<AppState>) -> Response {
    axum::Json(st.import_progress.snapshot()).into_response()
}

// ============================================================
// 曲库来源管理（app 级 [[sources]]，就地写入，旧字段 / 旧端点全部保留）
// ============================================================

/// 单条来源的 JSON 形态（前端列表 / 详情共用）。
fn source_json(s: &config::SourceEntry) -> serde_json::Value {
    serde_json::json!({
        "id": s.id,
        "name": s.name,
        "path": s.path,
        "is_netease": s.is_netease,
        "mode": s.mode,
        "enabled": s.enabled
    })
}

/// 生成稳定且不与现有来源冲突的 id（时间戳 + 后缀兜底）。
fn new_source_id(existing: &[config::SourceEntry]) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let base = format!("src-{nanos:x}");
    if !existing.iter().any(|s| s.id == base) {
        return base;
    }
    let mut n = 1usize;
    loop {
        let candidate = format!("{base}-{n}");
        if !existing.iter().any(|s| s.id == candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// 规范化一份来源数组（与落盘内容严格一致，避免内存 / 磁盘漂移）。
fn normalized_sources(sources: &[config::SourceEntry]) -> Vec<config::SourceEntry> {
    let mut v = sources.to_vec();
    for s in &mut v {
        config::normalize_source(s);
    }
    v
}

/// 「就地落盘 + 同步内存」公共范式（对齐 `ui_prefs_post_handler`）：
/// 先写盘，成功后才同步运行期配置；失败不污染内存。**绝不调用 `save_app()`**
/// （整份重写会抹掉注释 / 未知键 / `[sources_meta]` 迁移标记）。
fn persist_sources(
    st: &AppState,
    sources: &[config::SourceEntry],
) -> std::result::Result<Vec<config::SourceEntry>, String> {
    let app_cp = st.app_config_path.lock().unwrap().clone();
    let normalized = normalized_sources(sources);
    config::save_sources_in_place(&app_cp, &normalized)
        .map_err(|e| format!("保存失败：{e}"))?;
    st.cfg.lock().unwrap().sources = normalized.clone();
    Ok(normalized)
}

fn sources_error(msg: String) -> Response {
    axum::Json(serde_json::json!({ "ok": false, "error": msg })).into_response()
}

/// `GET /api/sources`：来源列表 + 角标所需的重复组数 / NCM 工具状态。
async fn sources_get_handler(State(st): State<AppState>) -> Response {
    let (sources, ncm_ready, pending_duplicates) = {
        let g = st.cfg.lock().unwrap();
        (
            g.sources.clone(),
            g.ncm.tool_ready(),
            duplicates::pending_count(&*g),
        )
    };
    axum::Json(serde_json::json!({
        "ok": true,
        "sources": sources.iter().map(source_json).collect::<Vec<_>>(),
        "ncm_ready": ncm_ready,
        "pending_duplicates": pending_duplicates
    }))
    .into_response()
}

/// `POST /api/sources/probe`：只读探测目录（自动判定网易云曲库 / 是否存在 .ncm）。
/// 探测失败静默降级为「未检测到」，不阻塞向导。
async fn sources_probe_handler(axum::Json(body): axum::Json<serde_json::Value>) -> Response {
    let path = body
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if path.is_empty() {
        return err_json("来源目录不能为空");
    }
    let p = PathBuf::from(&path);
    let probe = blocking(move || import::probe_source(&p)).await;
    axum::Json(serde_json::json!({
        "ok": true,
        "is_netease": probe.is_netease,
        "ncm_found": probe.ncm_found,
        "suggested_name": probe.suggested_name,
        "exists": Path::new(&path).is_dir()
    }))
    .into_response()
}

/// `POST /api/sources`：新增一条来源（自动检测网易云标记，允许前端显式覆盖）。
async fn sources_create_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let path = body
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if path.is_empty() {
        return err_json("来源目录不能为空");
    }
    if !Path::new(&path).is_dir() {
        return err_json("指定路径不是已存在的目录");
    }
    let probe_path = PathBuf::from(&path);
    let probe = blocking(move || import::probe_source(&probe_path)).await;

    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| probe.suggested_name.clone());
    let is_netease = body
        .get("is_netease")
        .and_then(|v| v.as_bool())
        .unwrap_or(probe.is_netease);
    let mode = body
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("copy")
        .to_string();
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let current = st.cfg.lock().unwrap().sources.clone();
    let mut entry = config::SourceEntry {
        id: new_source_id(&current),
        name,
        path,
        is_netease,
        mode,
        enabled,
    };
    config::normalize_source(&mut entry);
    let mut next = current;
    next.push(entry.clone());
    match persist_sources(&st, &next) {
        Ok(list) => axum::Json(serde_json::json!({
            "ok": true,
            "source": source_json(&entry),
            "sources": list.iter().map(source_json).collect::<Vec<_>>(),
            "ncm_found": probe.ncm_found
        }))
        .into_response(),
        Err(e) => sources_error(e),
    }
}

/// `PUT /api/sources/:id`：部分更新一条来源；改了路径且未显式覆盖标记时重新自动检测。
async fn sources_update_handler(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let current = st.cfg.lock().unwrap().sources.clone();
    let Some(mut entry) = current.iter().find(|s| s.id == id).cloned() else {
        return err_json("来源不存在");
    };
    let mut path_changed = false;
    if let Some(p) = body.get("path").and_then(|v| v.as_str()) {
        let p = p.trim().to_string();
        if p.is_empty() {
            return err_json("来源目录不能为空");
        }
        if !Path::new(&p).is_dir() {
            return err_json("指定路径不是已存在的目录");
        }
        path_changed = p != entry.path;
        entry.path = p;
    }
    if let Some(n) = body.get("name").and_then(|v| v.as_str()) {
        entry.name = n.to_string();
    }
    if let Some(v) = body.get("is_netease").and_then(|v| v.as_bool()) {
        entry.is_netease = v; // 用户手动覆盖误判
    } else if path_changed {
        let p = PathBuf::from(&entry.path);
        entry.is_netease = blocking(move || import::probe_source(&p)).await.is_netease;
    }
    if let Some(m) = body.get("mode").and_then(|v| v.as_str()) {
        entry.mode = m.to_string();
    }
    if let Some(v) = body.get("enabled").and_then(|v| v.as_bool()) {
        entry.enabled = v;
    }
    config::normalize_source(&mut entry);
    let next: Vec<config::SourceEntry> = current
        .iter()
        .map(|s| if s.id == id { entry.clone() } else { s.clone() })
        .collect();
    match persist_sources(&st, &next) {
        Ok(list) => {
            let saved = list.iter().find(|s| s.id == id).unwrap_or(&entry);
            axum::Json(serde_json::json!({
                "ok": true,
                "source": source_json(saved),
                "sources": list.iter().map(source_json).collect::<Vec<_>>()
            }))
            .into_response()
        }
        Err(e) => sources_error(e),
    }
}

/// `DELETE /api/sources/:id`：删除来源（空数组也会留下迁移标记，旧单值不会复活）。
async fn sources_delete_handler(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let current = st.cfg.lock().unwrap().sources.clone();
    if !current.iter().any(|s| s.id == id) {
        return err_json("来源不存在");
    }
    let next: Vec<config::SourceEntry> = current.into_iter().filter(|s| s.id != id).collect();
    match persist_sources(&st, &next) {
        Ok(list) => axum::Json(serde_json::json!({
            "ok": true,
            "sources": list.iter().map(source_json).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(e) => sources_error(e),
    }
}

/// `POST /api/sources/:id/import`：按来源导入。
/// 网易云来源在 `import_source_with_progress` 内强制复制 + `Origin::NeteaseSync`（只加不删）。
async fn sources_import_handler(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let (cfg, source) = {
        let g = st.cfg.lock().unwrap();
        (g.clone(), g.sources.iter().find(|s| s.id == id).cloned())
    };
    let Some(s) = source else {
        return err_json("来源不存在");
    };
    if !s.enabled {
        return err_json("该来源已停用，请先启用");
    }
    let mode = if s.mode == "move" && !s.is_netease {
        ImportMode::Move
    } else {
        ImportMode::Copy
    };
    let conn = st.conn.clone();
    let tracker = st.import_progress.clone();
    tracker.start(&format!("来源 {}", s.name));
    let t = tracker.clone();
    let path = PathBuf::from(&s.path);
    let is_netease = s.is_netease;
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        import::import_source_with_progress(&cfg, &c, &path, is_netease, mode, &t)
    })
    .await;
    tracker.finish();
    match res {
        Ok(r) => axum::Json(serde_json::json!({
            "ok": true,
            "imported": r.imported,
            "skipped": r.skipped,
            "pending": r.pending,
            "failed": r.failed,
            "held": r.held
        }))
        .into_response(),
        Err(e) => axum::Json(serde_json::json!({ "ok": false, "error": e.to_string() }))
            .into_response(),
    }
}

async fn import_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let cfg = st.cfg.lock().unwrap().clone();
    let conn = st.conn.clone();
    let src = body
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mode = if body.get("mode").and_then(|v| v.as_str()) == Some("move") {
        ImportMode::Move
    } else {
        ImportMode::Copy
    };
    if src.is_empty() {
        return axum::Json(serde_json::json!({ "ok": false, "error": "路径为空" })).into_response();
    }
    let tracker = st.import_progress.clone();
    let label = format!("文件夹 {}", fname_of_path(&src));
    tracker.start(&label);
    let t = tracker.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        import::import_folder_with_progress(
            &cfg,
            &c,
            std::path::Path::new(&src),
            mode,
            Origin::Manual,
            &t,
        )
    })
    .await;
    tracker.finish();
    match res {
        Ok(r) => axum::Json(serde_json::json!({
            "ok": true, "imported": r.imported, "skipped": r.skipped,
            "pending": r.pending, "failed": r.failed, "held": r.held
        }))
        .into_response(),
        Err(e) => axum::Json(serde_json::json!({ "ok": false, "error": e.to_string() }))
            .into_response(),
    }
}

/// 取路径的最后一段（用于给导入任务起可读标签）。
fn fname_of_path(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string())
}

// ---------- 播放列表（本地曲库内用户自建，local_id 体系）----------

/// 列出全部播放列表（含各自曲目数）。
async fn playlists_handler(State(st): State<AppState>) -> Response {
    let conn = st.conn.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        db::list_playlists(&c)
    })
    .await;
    // DB 异常必须如实返回，不能用空数组伪装成「暂无播放列表」。
    match res {
        Ok(rows) => axum::Json(serde_json::json!({ "ok": true, "playlists": rows })).into_response(),
        Err(e) => err_json(&format!("读取播放列表失败：{e}")),
    }
}

#[derive(Deserialize)]
struct PlaylistCreate {
    name: String,
}

/// 新建播放列表。
async fn playlist_create_handler(
    State(st): State<AppState>,
    axum::Json(body): axum::Json<PlaylistCreate>,
) -> Response {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return err_json("播放列表名称不能为空");
    }
    let conn = st.conn.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        db::create_playlist(&c, &name)
    })
    .await;
    match res {
        Ok(id) => axum::Json(serde_json::json!({ "ok": true, "id": id })).into_response(),
        Err(e) => err_json(&e.to_string()),
    }
}

/// 删除播放列表（级联删除其条目）。
async fn playlist_delete_handler(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<i64>,
) -> Response {
    let conn = st.conn.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        db::delete_playlist(&c, id)
    })
    .await;
    match res {
        Ok(()) => axum::Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => err_json(&e.to_string()),
    }
}

/// 取播放列表内的曲目（按 position 排序）。
async fn playlist_tracks_handler(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<i64>,
) -> Response {
    let conn = st.conn.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        db::get_playlist_tracks(&c, id)
    })
    .await;
    match res {
        Ok(rows) => axum::Json(serde_json::json!({ "ok": true, "tracks": rows })).into_response(),
        Err(e) => err_json(&format!("读取播放列表曲目失败：{e}")),
    }
}

#[derive(Deserialize)]
struct PlaylistTrackReq {
    local_id: i64,
}

/// 将曲目加入播放列表（追加到末尾；已存在则忽略）。
async fn playlist_add_handler(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<i64>,
    axum::Json(body): axum::Json<PlaylistTrackReq>,
) -> Response {
    if body.local_id <= 0 {
        return err_json("local_id 无效");
    }
    let conn = st.conn.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        db::add_to_playlist(&c, id, body.local_id)
    })
    .await;
    match res {
        Ok(()) => axum::Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => err_json(&e.to_string()),
    }
}

/// 从播放列表移除曲目。
async fn playlist_remove_handler(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<i64>,
    axum::Json(body): axum::Json<PlaylistTrackReq>,
) -> Response {
    let conn = st.conn.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        db::remove_from_playlist(&c, id, body.local_id)
    })
    .await;
    match res {
        Ok(()) => axum::Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => err_json(&e.to_string()),
    }
}

#[derive(Deserialize)]
struct PlaylistRename {
    name: String,
}

/// 重命名播放列表（名称非空 / ≤60 字符 / 同名拒绝，由 db 层统一校验）。
async fn playlist_rename_handler(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<i64>,
    axum::Json(body): axum::Json<PlaylistRename>,
) -> Response {
    let conn = st.conn.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        db::rename_playlist(&c, id, &body.name)
    })
    .await;
    match res {
        Ok(()) => axum::Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => err_json(&e.to_string()),
    }
}

#[derive(Deserialize)]
struct PlaylistMove {
    local_id: i64,
    /// -1 = 上移，+1 = 下移。
    delta: i64,
}

/// 列表内上移 / 下移一首（交换相邻 position）。
async fn playlist_move_handler(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<i64>,
    axum::Json(body): axum::Json<PlaylistMove>,
) -> Response {
    let conn = st.conn.clone();
    let res = blocking(move || {
        let c = conn.lock().unwrap();
        db::move_playlist_item(&c, id, body.local_id, body.delta)
    })
    .await;
    match res {
        Ok(moved) => axum::Json(serde_json::json!({ "ok": true, "moved": moved })).into_response(),
        Err(e) => err_json(&e.to_string()),
    }
}

// ---------- 专辑 / 歌手（本地曲库内按元数据维度的分组浏览）----------
// 与「播放列表」并列的二级导航：列表（专辑/歌手）→ 选中后展示其曲目（三级）。
// 分组用 COALESCE(NULLIF(字段,''),占位) 与成员查询保持一致，避免 NULL/空被漏分或无法点开。

/// 列出全部专辑（含各专辑曲目数）。
async fn albums_handler(State(st): State<AppState>) -> Response {
    let conn = st.conn.clone();
    let rows = blocking(move || {
        let c = conn.lock().unwrap();
        db::list_albums(&c)
    })
    .await
    .unwrap_or_default();
    axum::Json(serde_json::json!({ "albums": rows })).into_response()
}

/// 列出全部歌手（含各歌手曲目数）。
async fn artists_handler(State(st): State<AppState>) -> Response {
    let conn = st.conn.clone();
    let rows = blocking(move || {
        let c = conn.lock().unwrap();
        db::list_artists(&c)
    })
    .await
    .unwrap_or_default();
    axum::Json(serde_json::json!({ "artists": rows })).into_response()
}

/// 取某专辑内曲目（按名称排序）。`name` 为前端回传的分组名（含占位「未知专辑」）。
async fn album_tracks_handler(
    State(st): State<AppState>,
    Query(q): Query<NameQuery>,
) -> Response {
    if q.name.trim().is_empty() {
        return err_json("专辑名称不能为空");
    }
    let conn = st.conn.clone();
    let name = q.name.clone();
    let rows = blocking(move || {
        let c = conn.lock().unwrap();
        db::get_tracks_by_album(&c, &name)
    })
    .await
    .unwrap_or_default();
    axum::Json(serde_json::json!({ "tracks": rows, "count": rows.len() })).into_response()
}

/// 取某歌手内曲目（按名称排序）。`name` 为前端回传的分组名（含占位「未知艺人」）。
async fn artist_tracks_handler(
    State(st): State<AppState>,
    Query(q): Query<NameQuery>,
) -> Response {
    if q.name.trim().is_empty() {
        return err_json("歌手名称不能为空");
    }
    let conn = st.conn.clone();
    let name = q.name.clone();
    let rows = blocking(move || {
        let c = conn.lock().unwrap();
        db::get_tracks_by_artist(&c, &name)
    })
    .await
    .unwrap_or_default();
    axum::Json(serde_json::json!({ "tracks": rows, "count": rows.len() })).into_response()
}
