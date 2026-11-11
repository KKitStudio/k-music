//! K music GUI（Tauri 壳）。
//! 启动流程：加载配置 → 打开 SQLite → 启动 Rust localhost HTTP 服务（REST + 前端 + Range 音频流）
//! → 接收服务实际端口 → 打开 WebView 窗口加载该地址（同源，前端用相对路径调用 /api）。
//! 网易云服务端登录/下载已于 2026-10-29 移除；本地导入源仅「从本地网易云音乐曲库更新」按钮。
use mt_core::{config, db, log, serve};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;

mod single_instance;
mod window_state;

/// #32：无边框窗口的自绘控制条。前端 POST /api/window/control，本回调通过
/// Tauri 窗口 API 执行动作；start_dragging() 内部把请求投递到 UI 线程，
/// 因此无需手写 Win32、也不依赖 WebView 的 IPC 权限（CLI serve 模式未注册则空转）。
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

/// 托盘是否成功创建（创建失败则两开关强制按 false 生效，R1 降级）。
static TRAY_AVAILABLE: OnceLock<Arc<AtomicBool>> = OnceLock::new();
/// 应用配置路径（供关闭 / 最小化时实时读取托盘开关，避免过期内存副本）。
static APP_CFG_PATH: OnceLock<std::path::PathBuf> = OnceLock::new();

fn window_control(action: &'static str) {
    let Some(app) = APP_HANDLE.get() else { return };
    let Some(win) = app.get_webview_window("main") else { return };
    let tray_ok = TRAY_AVAILABLE.get().map(|a| a.load(Ordering::Relaxed)).unwrap_or(false);
    let (close_to_tray, minimize_to_tray) = match APP_CFG_PATH.get() {
        Some(p) => config::read_ui_tray_flags(p),
        None => (false, false),
    };
    match action {
        "minimize" => {
            if tray_ok && minimize_to_tray { let _ = win.hide(); }
            else { let _ = win.minimize(); }
        }
        "toggle-maximize" => {
            let _ = if win.is_maximized().unwrap_or(false) {
                win.unmaximize()
            } else {
                win.maximize()
            };
        }
        "close" => {
            if tray_ok && close_to_tray { let _ = win.hide(); }
            else { let _ = win.close(); }
        }
        "drag" => { let _ = win.start_dragging(); }
        _ => {}
    }
}

/// Windows 原生标题栏着色。
///
/// WebView 里的 `.topbar` 由 CSS `--primary` 渐变着色，窗口标题栏却由 DWM 按系统主题绘制，
/// 二者默认不一致。通过 `DwmSetWindowAttribute` 把标题栏背景/文字设为应用当前强调色，
/// 即可与顶栏统一；前端 `/api/appearance/titlebar` 负责在每次外观变化时上报颜色。
/// - Win11 21H2+：`DWMWA_CAPTION_COLOR` / `DWMWA_TEXT_COLOR` 支持任意颜色。
/// - Win10：上述两个属性不被支持（调用静默失败），退化为 `DWMWA_USE_IMMERSIVE_DARK_MODE`
///   跟随明暗，至少不与系统主题割裂。
#[cfg(windows)]
mod native_titlebar {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

    /// 主窗口 HWND 原始值；0 表示窗口尚未创建。
    static MAIN_HWND: AtomicUsize = AtomicUsize::new(0);
    /// 最近一次上报的 COLORREF，用于窗口创建前已收到上报时补刷。
    static LAST_COLOR: AtomicU32 = AtomicU32::new(0);
    static LAST_DARK: AtomicBool = AtomicBool::new(false);

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_CAPTION_COLOR: u32 = 35;
    const DWMWA_TEXT_COLOR: u32 = 36;

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: usize,
            attr: u32,
            value: *const c_void,
            len: u32,
        ) -> i32;
    }

    // 恢复焦点用：原生对话框（rfd）关闭后把主窗口带回前台并夺回键盘焦点。
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetForegroundWindow(hwnd: usize) -> i32;
        fn SetFocus(hwnd: usize) -> usize;
    }

    unsafe fn set_attr(hwnd: usize, attr: u32, value: *const c_void, len: u32) {
        let _ = DwmSetWindowAttribute(hwnd, attr, value, len);
    }

    /// 原生文件夹/文件对话框（rfd）在 Windows 上关闭后，WebView 常未自动重获焦点，
    /// 主窗口表现为「无响应 / 点击不生效」。此函数把主窗口带回前台并把键盘焦点设回它，
    /// 解决导入云音乐曲库等场景后主界面交互紊乱的问题。
    pub fn refocus() {
        let hwnd = MAIN_HWND.load(Ordering::Relaxed);
        if hwnd == 0 {
            return;
        }
        unsafe {
            // 先带回前台（突破前台锁，用户刚在同源对话框交互过，本进程持有前台权限）；
            // 再把键盘焦点设回主窗口，使 WebView 重新可交互。
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(hwnd);
        }
    }

    /// 由 axum 端点回调（可在任意线程）。
    pub fn apply(color: u32, dark: bool) {
        LAST_COLOR.store(color, Ordering::Relaxed);
        LAST_DARK.store(dark, Ordering::Relaxed);
        paint(color, dark);
    }

    /// 窗口创建完成后调用：登记 HWND 并补刷挂起的颜色。
    pub fn bind(hwnd: usize) {
        MAIN_HWND.store(hwnd, Ordering::Relaxed);
        let color = LAST_COLOR.load(Ordering::Relaxed);
        if color != 0 {
            paint(color, LAST_DARK.load(Ordering::Relaxed));
        }
    }

    fn paint(color: u32, dark: bool) {
        let hwnd = MAIN_HWND.load(Ordering::Relaxed);
        if hwnd == 0 {
            return;
        }
        unsafe {
            let mode: u32 = if dark { 1 } else { 0 };
            set_attr(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &mode as *const u32 as *const c_void,
                4,
            );
            set_attr(
                hwnd,
                DWMWA_CAPTION_COLOR,
                &color as *const u32 as *const c_void,
                4,
            );
            let white: u32 = 0x00FF_FFFF; // COLORREF 白色（0x00BBGGRR）
            set_attr(
                hwnd,
                DWMWA_TEXT_COLOR,
                &white as *const u32 as *const c_void,
                4,
            );
        }
    }
}

/// 隐藏态二次启动（单实例 /api/focus）必须先把窗口显示出来再置前，否则 SetForegroundWindow 对隐藏窗口无效。
fn focus_main_window() {
    let Some(app) = APP_HANDLE.get() else { return };
    let Some(win) = app.get_webview_window("main") else { return };
    let _ = win.show();
    let _ = win.unminimize();
    let _ = win.set_focus();
}

/// 托盘菜单项句柄（创建后存入，供 apply_tray_state 零 JS 反调更新文案 / 灰显）。
struct TrayMenuItems {
    title: tauri::menu::MenuItem<tauri::Wry>,
    show_hide: tauri::menu::MenuItem<tauri::Wry>,
    play_pause: tauri::menu::MenuItem<tauri::Wry>,
    prev: tauri::menu::MenuItem<tauri::Wry>,
    next: tauri::menu::MenuItem<tauri::Wry>,
    mute: tauri::menu::MenuItem<tauri::Wry>,
}
static TRAY_ITEMS: OnceLock<TrayMenuItems> = OnceLock::new();

/// 托盘菜单项文案 / 灰显状态（零 JS 反调，仅经已注册 sink 更新；sink 未注册时静默无操作）。
fn apply_tray_state(s: mt_core::serve::TrayState) {
    let Some(app) = APP_HANDLE.get() else { return };
    let Some(win) = app.get_webview_window("main") else { return };
    let Some(items) = TRAY_ITEMS.get() else { return };
    let title_text = if !s.has_track {
        "未在播放".to_string()
    } else if s.playing {
        format!("{} - {}", s.title, s.artist)
    } else {
        format!("已暂停：{} - {}", s.title, s.artist)
    };
    let _ = items.title.set_text(&title_text);
    let _ = items.title.set_enabled(false);
    let visible = win.is_visible().unwrap_or(true);
    let _ = items.show_hide.set_text(if visible { "隐藏主窗口" } else { "显示主窗口" });
    let _ = items.play_pause.set_text(if s.playing { "暂停" } else { "播放" });
    let _ = items.play_pause.set_enabled(s.has_track);
    let _ = items.prev.set_enabled(s.has_track);
    let _ = items.next.set_enabled(s.has_track);
    let _ = items.mute.set_text(if s.muted { "取消静音" } else { "静音" });
}

/// 由托盘菜单项触发的前端命令（白名单，计划 N2/N5）。仅 `eval` 顶层 `window.kqkCommand`。
fn tray_eval(app: &tauri::AppHandle, cmd: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.eval(&format!("window.kqkCommand('{}')", cmd));
    }
}

/// 创建系统托盘 + 右键菜单；失败返回 false（R1 降级：两开关强制按 false 生效）。
#[cfg(windows)]
fn create_tray(app: &tauri::AppHandle) -> bool {
    let icon = match app.default_window_icon() {
        Some(i) => i.clone(),
        None => { eprintln!("托盘创建失败：缺少窗口图标"); return false; }
    };
    let title_item = match MenuItem::with_id(app, "tray_title", "未在播放", false, None::<&str>) {
        Ok(m) => m,
        Err(e) => { eprintln!("托盘菜单创建失败: {e}"); return false; }
    };
    let show_hide_item = MenuItem::with_id(app, "show_hide", "隐藏主窗口", true, None::<&str>).unwrap();
    let sep1 = PredefinedMenuItem::separator(app).unwrap();
    let play_pause_item = MenuItem::with_id(app, "play_pause", "暂停", true, None::<&str>).unwrap();
    let prev_item = MenuItem::with_id(app, "prev", "上一首", true, None::<&str>).unwrap();
    let next_item = MenuItem::with_id(app, "next", "下一首", true, None::<&str>).unwrap();
    let sep2 = PredefinedMenuItem::separator(app).unwrap();
    let mute_item = MenuItem::with_id(app, "mute", "静音", true, None::<&str>).unwrap();
    let settings_item = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>).unwrap();
    let sep3 = PredefinedMenuItem::separator(app).unwrap();
    let quit_item = MenuItem::with_id(app, "quit", "退出 K music", true, None::<&str>).unwrap();
    let items: &[&dyn tauri::menu::IsMenuItem<tauri::Wry>] = &[
        &title_item,
        &show_hide_item,
        &sep1,
        &play_pause_item,
        &prev_item,
        &next_item,
        &sep2,
        &mute_item,
        &settings_item,
        &sep3,
        &quit_item,
    ];
    let menu = match Menu::with_items(app, items) {
        Ok(m) => m,
        Err(e) => { eprintln!("托盘菜单创建失败: {e}"); return false; }
    };
    let tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("K music")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            match event.id.as_ref() {
                "show_hide" => {
                    if let Some(w) = app.get_webview_window("main") {
                        if w.is_visible().unwrap_or(false) { let _ = w.hide(); }
                        else { let _ = w.show(); let _ = w.unminimize(); let _ = w.set_focus(); }
                    }
                }
                "play_pause" => tray_eval(app, "toggle-play"),
                "prev" => tray_eval(app, "prev"),
                "next" => tray_eval(app, "next"),
                "mute" => tray_eval(app, "toggle-mute"),
                "settings" => tray_eval(app, "show-settings"),
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .build(app);
    let tray = match tray {
        Ok(t) => t,
        Err(e) => { eprintln!("托盘图标创建失败: {e}"); return false; }
    };
    // 左键点击 = 显示/隐藏主窗口（右键由 .menu 弹出）。
    app.on_tray_icon_event(|app, event| {
        if let tauri::tray::TrayIconEvent::Click {
            button: tauri::tray::MouseButton::Left,
            button_state: tauri::tray::MouseButtonState::Up,
            ..
        } = event
        {
            if let Some(w) = app.get_webview_window("main") {
                if w.is_visible().unwrap_or(false) { let _ = w.hide(); }
                else { let _ = w.show(); let _ = w.unminimize(); let _ = w.set_focus(); }
            }
        }
    });
    let _ = TRAY_ITEMS.set(TrayMenuItems {
        title: title_item,
        show_hide: show_hide_item,
        play_pause: play_pause_item,
        prev: prev_item,
        next: next_item,
        mute: mute_item,
    });
    true
}

#[cfg(not(windows))]
fn create_tray(_app: &tauri::AppHandle) -> bool {
    false
}

pub fn run() {
    // 创建 Tauri 窗口前预检 WebView2 运行时：缺失时原生弹窗提示并打开下载页后退出，
    // 避免 WebView 创建失败导致的白屏/崩溃（此时 WebView 自身还不可用，必须用原生 MessageBox）。
    #[cfg(windows)]
    webview2_check::ensure();
    // exe 所在目录：作为前端/资源解析与 launcher 记忆的基准（双击运行时 CWD 不可靠）。
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    // 启动早期单实例锁：必须早于配置加载 / DB 打开 / axum 服务 / 全库后台扫描。
    // 第二个实例在此直接退出（并唤起已有窗口），不会产生第二份 DB 写入与重复扫描。
    let instance_guard = single_instance::acquire(exe_dir.as_deref());
    // 主窗口几何状态（独立于配置双文件，理由见 window_state 模块头注释）。
    let window_state_path = window_state::state_path(exe_dir.as_deref());
    let saved_geometry = window_state::load(&window_state_path);
    // 加载两份配置：app 级（exe 目录 kqk-app.toml，含当前曲库路径）+ 曲库级（<曲库>/.mt/config.toml）。
    let loaded = match config::load_startup() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("配置加载失败: {e}");
            return;
        }
    };
    let cfg = loaded.cfg.clone();
    let library_configured = loaded.library_configured;
    let cfg_path = loaded.library_path.clone();
    let app_cfg_path = loaded.app_path.clone();
    // 配置文件版本状态（生成版本 / 最低兼容版本 / 兼容判定 / 冲突原因）。
    let config_status = config::ConfigStatus::from_loaded(&loaded);
    // 存在逻辑冲突时明确告知（配置已被备份并重建，原文件内容保留在 .bak 中）。
    if !config_status.app_conflict_reason.is_empty() {
        eprintln!("app 配置存在逻辑冲突，已备份并重建：{}", config_status.app_conflict_reason);
    }
    if !config_status.library_conflict_reason.is_empty() {
        eprintln!("曲库配置存在逻辑冲突，已备份并重建：{}", config_status.library_conflict_reason);
    }
    log::event(&cfg, "gui", "run started");
    // 仅已配置曲库时才建目录（guest 态不污染发布目录）。
    if library_configured {
        if let Err(e) = cfg.ensure_dirs() {
            eprintln!("目录创建失败: {e}");
            return;
        }
    }
    let conn = match db::open(&cfg) {
        Ok(c) => Arc::new(Mutex::new(c)),
        Err(e) => {
            eprintln!("数据库打开失败: {e}");
            return;
        }
    };

    // 在独立线程启动 HTTP 服务，并通过 channel 回传实际监听端口。
    let (tx, rx) = mpsc::channel::<u16>();
    let server_cfg = cfg.clone();
    let server_conn = conn.clone();
    let server_cfg_path = cfg_path.clone();
    let server_app_cfg_path = app_cfg_path.clone();
    // 前端目录：相对 exe 级联解析（双击运行时 CWD 不可靠，不能写死 ./app/gui/frontend）。
    let frontend = serve::resolve_frontend_dir(exe_dir.as_deref(), std::env::current_dir().ok().as_deref());
    log::event(&cfg, "gui", &format!("frontend_dir={}", frontend.display()));
    serve::spawn_server(
        tx,
        server_cfg,
        server_conn,
        frontend,
        server_cfg_path,
        server_app_cfg_path,
        library_configured,
        config_status,
    );

    let port = rx.recv().unwrap_or(59637);
    let url = format!("http://127.0.0.1:{}/", port);
    // 等待后端真正开始响应，避免 WebView 打开瞬间连不上（旧实例残留/端口回退等）。
    wait_for_server(&url, Duration::from_secs(5));
    println!("GUI 窗口加载地址：{}", url);
    log::event(&cfg, "gui", &format!("window url={}", url));
    // 服务确认可响应后才发布端口，保证第二个实例 GET /api/focus 一定命中。
    instance_guard.publish(port);

    tauri::Builder::default()
        .setup(move |app| {
            // 先注册钩子（不依赖 HWND）：前端可能在窗口创建完成前就上报颜色，
            // apply() 会暂存 LAST_COLOR，待 bind() 时补刷，避免启动时标题栏来不及染色。
            #[cfg(windows)]
            serve::set_titlebar_color_sink(native_titlebar::apply);
            // 原生对话框（rfd）关闭后恢复主窗口焦点，避免「主窗口无响应 / 点击不生效」。
            #[cfg(windows)]
            serve::set_post_dialog_focus_sink(native_titlebar::refocus);
            // 交互增强 b 级：托盘状态 sink（前端推送）、隐藏态二次启动唤起 sink。
            serve::set_focus_window_sink(focus_main_window);
            serve::set_tray_state_sink(apply_tray_state);
            // #32：登记窗口句柄与自绘控制条回调，供 /api/window/control 调用。
            let _ = APP_HANDLE.set(app.handle().clone());
            let _ = APP_CFG_PATH.set(app_cfg_path.clone());
            serve::set_window_control_sink(window_control);
            let uri = tauri::Url::parse(&url).expect("无效的服务地址");
            // 恢复上次窗口位置：先校验坐标仍落在当前某台显示器上（外接屏拔掉 /
            // 改分辨率后旧坐标会跑到屏幕外），不通过则回退系统默认居中。
            let monitors = app.available_monitors().unwrap_or_default();
            let geom = saved_geometry.filter(|g| window_state::visible_on_any_monitor(g, &monitors));
            let win = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(uri))
                .title(mt_core::app::title())
                // #33：默认几何必须等于「能显示全」的硬下限，别再硬编码 1100（< MIN_W）。
                .inner_size(window_state::DEFAULT_W as f64, window_state::DEFAULT_H as f64)
                // #32：隐藏 Windows 原生标题栏（与 modern 深色 rail / 米色 stage 割裂），
                // 拖动与最小化 / 最大化 / 关闭改由前端自绘控制条经 REST 驱动。
                .decorations(false)
                // 重设计布局在 1280x720 以下必然溢出换行；这是「能显示全」的硬下限。
                .min_inner_size(window_state::MIN_W as f64, window_state::MIN_H as f64)
                // 先隐藏：定位 / 恢复尺寸 / 最大化完成后再显示，避免位置跳变被看见。
                .visible(false)
                .build()?;
            if let Some(g) = geom {
                window_state::restore(&win, &g);
            }
            // 交互增强 b 级：先建托盘状态原子（供 track 共享），再建托盘；
            // 关闭行为按「开关 + 托盘可用性」决策（R1：托盘失败则开关强制 false）。
            let tray_atomic = Arc::new(AtomicBool::new(false));
            let _ = TRAY_AVAILABLE.set(tray_atomic.clone());
            window_state::track(&win, window_state_path.clone(), tray_atomic.clone(), app_cfg_path.clone());
            let tray_ok = create_tray(app.handle());
            tray_atomic.store(tray_ok, Ordering::Relaxed);
            serve::set_tray_available(tray_ok);
            #[cfg(windows)]
            {
                // HWND 是 `windows` crate 的 newtype `HWND(pub *mut c_void)`，需取 .0 再转地址值。
                if let Ok(hwnd) = win.hwnd() {
                    native_titlebar::bind(hwnd.0 as usize);
                }
            }
            // 无条件显示：即使几何恢复失败，也不能把窗口留在隐藏态。
            let _ = win.show();
            Ok(())
        })
        .plugin(tauri_plugin_shell::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
    // 正常退出时删除端口文件并关闭互斥体；崩溃时句柄由内核回收，
    // 残留端口文件会在下一次启动时被连接探测判定为陈旧，不影响正确性。
    drop(instance_guard);
}

/// 轮询后端端口直至可连接（最多 `timeout`），避免 WebView 打开瞬间后端尚未就绪。
fn wait_for_server(url: &str, timeout: Duration) {
    use std::net::TcpStream;
    use std::time::Instant;
    let Some((_, hp)) = url.trim_start_matches("http://").split_once('/').map(|(h, _)| (h, h)) else {
        return;
    };
    let Some((_, port_str)) = hp.rsplit_once(':') else {
        return;
    };
    let Ok(port) = port_str.trim_end_matches('/').parse::<u16>() else {
        return;
    };
    let addr = std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port);
    let start = Instant::now();
    while start.elapsed() < timeout {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// WebView2 运行时预检（仅 Windows）。
///
/// 采用微软官方检测 API `GetAvailableCoreWebView2BrowserVersionString`（由 webview2-com-sys 暴露，
/// 与 Tauri 同源链接，零新增体积）。它能权威探测所有可用运行时：Win11 内置、Edge 随附、独立安装。
/// 不使用注册表探测——现代 WebView2（尤其 Edge 随附版）往往不写注册表，注册表方案会误杀已装用户。
///
/// 必须在创建 Tauri 窗口前调用：缺失时 WebView 自身还不可用，故用原生 `MessageBoxW` 弹窗，
/// 并自动打开微软官方下载页，随后退出，避免白屏/崩溃。
#[cfg(windows)]
mod webview2_check {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use webview2_com_sys::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString;
    use windows_core::{PCWSTR, PWSTR};

    const LINK: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(h: *mut c_void, text: *const u16, caption: *const u16, utype: u32) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(pv: *mut c_void);
    }

    fn to_wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// 入口：缺失 WebView2 则原生弹窗 + 打开下载页 + 退出；否则无副作用返回。
    pub fn ensure() {
        if is_missing() {
            show_missing_dialog();
        }
    }

    /// 探测系统是否缺少可用的 WebView2 运行时（`true` = 缺失）。
    ///
    /// 使用微软官方 API `GetAvailableCoreWebView2BrowserVersionString`（传 null 文件夹 =>
    /// 在系统默认位置查找已安装运行时），权威覆盖 Win11 内置 / Edge 随附 / 独立安装等所有来源。
    /// 不使用注册表探测——现代 WebView2（尤其 Edge 随附版）往往不写注册表，注册表方案会误杀已装用户。
    pub(crate) fn is_missing() -> bool {
        unsafe {
            let mut version: PWSTR = PWSTR(std::ptr::null_mut());
            let result = GetAvailableCoreWebView2BrowserVersionString(
                PCWSTR(std::ptr::null()),
                &mut version,
            );
            let installed = result.is_ok() && !version.is_null();
            // 成功时该 API 经 CoTaskMemAlloc 返回版本字符串，需释放（失败时为 null，释放无害）。
            if !version.is_null() {
                CoTaskMemFree(version.0 as *mut c_void);
            }
            !installed
        }
    }

    /// 原生弹窗提示并打开下载页后退出（缺失时调用；此时 WebView 自身还不可用，必须用原生 MessageBox）。
    fn show_missing_dialog() {
        unsafe {
            let msg = format!(
                "无法启动 K music：当前系统缺少 WebView2 运行时（窗口需要它来显示界面）。\n\n\
请安装后重新打开本程序（仅首次需要，装好即永久可用）：\n{}",
                LINK
            );
            MessageBoxW(
                std::ptr::null_mut(),
                to_wide(&msg).as_ptr(),
                to_wide("K music - 缺少 WebView2 运行时").as_ptr(),
                0x10, // MB_ICONERROR
            );
            // 直接打开微软官方 WebView2 下载页（Evergreen 引导器）。
            let _ = std::process::Command::new("cmd")
                .args(["/c", "start", "", LINK])
                .spawn();
            std::process::exit(1);
        }
    }

    #[cfg(all(windows, test))]
    mod tests {
        #[test]
        fn local_machine_has_webview2() {
            // 本机（开发机）已安装 WebView2（GUI 可正常运行），预检绝不可判定缺失而误杀已装用户。
            assert!(
                !super::is_missing(),
                "本机 WebView2 未被检测到，预检会误杀已装用户！"
            );
        }
    }
}
