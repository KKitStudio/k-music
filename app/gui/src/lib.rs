//! K曲库 GUI（Tauri 壳）。
//! 启动流程：加载配置 → 打开 SQLite → 启动 Rust localhost HTTP 服务（REST + 前端 + Range 音频流）
//! → 接收服务实际端口 → 打开 WebView 窗口加载该地址（同源，前端用相对路径调用 /api）。
//! 网易云服务端登录/下载已于 2026-10-29 移除；本地导入源仅「从本地网易云音乐曲库更新」按钮。
use mt_core::{config, db, log, serve};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

mod window_state;

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

pub fn run() {
    // 创建 Tauri 窗口前预检 WebView2 运行时：缺失时原生弹窗提示并打开下载页后退出，
    // 避免 WebView 创建失败导致的白屏/崩溃（此时 WebView 自身还不可用，必须用原生 MessageBox）。
    #[cfg(windows)]
    webview2_check::ensure();
    // exe 所在目录：作为前端/资源解析与 launcher 记忆的基准（双击运行时 CWD 不可靠）。
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
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

    tauri::Builder::default()
        .setup(move |app| {
            // 先注册钩子（不依赖 HWND）：前端可能在窗口创建完成前就上报颜色，
            // apply() 会暂存 LAST_COLOR，待 bind() 时补刷，避免启动时标题栏来不及染色。
            #[cfg(windows)]
            serve::set_titlebar_color_sink(native_titlebar::apply);
            // 原生对话框（rfd）关闭后恢复主窗口焦点，避免「主窗口无响应 / 点击不生效」。
            #[cfg(windows)]
            serve::set_post_dialog_focus_sink(native_titlebar::refocus);
            let uri = tauri::Url::parse(&url).expect("无效的服务地址");
            // 恢复上次窗口位置：先校验坐标仍落在当前某台显示器上（外接屏拔掉 /
            // 改分辨率后旧坐标会跑到屏幕外），不通过则回退系统默认居中。
            let monitors = app.available_monitors().unwrap_or_default();
            let geom = saved_geometry.filter(|g| window_state::visible_on_any_monitor(g, &monitors));
            let win = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(uri))
                .title(mt_core::app::title())
                .inner_size(1100.0, 720.0)
                .min_inner_size(900.0, 600.0)
                // 先隐藏：定位 / 恢复尺寸 / 最大化完成后再显示，避免位置跳变被看见。
                .visible(false)
                .build()?;
            if let Some(g) = geom {
                window_state::restore(&win, &g);
            }
            window_state::track(&win, window_state_path.clone());
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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
                "无法启动 K曲库：当前系统缺少 WebView2 运行时（窗口需要它来显示界面）。\n\n\
请安装后重新打开本程序（仅首次需要，装好即永久可用）：\n{}",
                LINK
            );
            MessageBoxW(
                std::ptr::null_mut(),
                to_wide(&msg).as_ptr(),
                to_wide("K曲库 - 缺少 WebView2 运行时").as_ptr(),
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
