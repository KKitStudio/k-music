//! 主窗口几何状态（位置 / 尺寸 / 最大化）的持久化。
//!
//! 独立于 `kqk-app.toml`：窗口几何由 GUI 进程独占写，而 `[ui]` 段由前端
//! `/api/ui-prefs` 从 axum 服务线程的内存副本整段就地写回；两者若写同一文件，
//! 任一方拿着旧副本整段落盘都会抹掉对方刚写入的值。单独一份
//! `kqk-window.state`（exe 目录）既消除竞态，也不改变「配置双文件」结构。
//!
//! 一律使用**物理像素**：Tauri 的 `outer_position()` / `inner_size()` 返回物理值，
//! 而 `WebviewWindowBuilder::position()` / `inner_size()` 接受逻辑值。若在 builder
//! 阶段直接把物理值当逻辑值传入，高 DPI（125% / 150%）下窗口会明显偏移，
//! 因此统一在窗口创建后用 `set_position(Position::Physical)` /
//! `set_size(Size::Physical)` 恢复。
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{Monitor, PhysicalPosition, PhysicalSize, Position, Size, WebviewWindow, WindowEvent};

pub const STATE_FILE: &str = "kqk-window.state";
/// #32：与 `WebviewWindowBuilder::min_inner_size(1280, 720)` 保持一致。
/// 重设计布局（236px rail + stage 顶栏 + 卡片内容 + 80px 播放器）在更小尺寸下
/// 会溢出换行，因此把窗口硬下限提到「能完整显示」的 1280x720。
pub const MIN_W: u32 = 1280;
pub const MIN_H: u32 = 720;
/// #33：首次启动 / 旧状态失效 / 取不到窗口尺寸时的默认几何，必须 ≥ 硬下限。
/// 曾硬编码 1100x720（小于 MIN_W），「不允许更小」在首次启动就不成立。
/// builder、current_geometry 与 parse() 共用这一组常量，避免出现第二套下限。
pub const DEFAULT_W: u32 = MIN_W;
pub const DEFAULT_H: u32 = MIN_H;
const MAX_W: u32 = 8192;
const MAX_H: u32 = 8192;
/// 窗口至少要有这么大的区域落在某台显示器上，才认为旧坐标仍然可用。
const MIN_VISIBLE_W: i64 = 160;
const MIN_VISIBLE_H: i64 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub maximized: bool,
}

/// 状态文件路径：exe 所在目录（双击运行时 CWD 不可靠），取不到时退回当前目录。
pub fn state_path(exe_dir: Option<&Path>) -> PathBuf {
    let base = exe_dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    base.join(STATE_FILE)
}

/// 解析极简 `key=value` 文本。任一必需字段缺失 / 非法 / 越界都判为无效，
/// 由调用方回退到默认几何——绝不把窗口恢复到屏幕外的坐标。
pub fn parse(text: &str) -> Option<Geometry> {
    let (mut x, mut y, mut w, mut h) = (None, None, None, None);
    let mut maximized = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "x" => x = v.parse::<i32>().ok(),
            "y" => y = v.parse::<i32>().ok(),
            "w" => w = v.parse::<u32>().ok(),
            "h" => h = v.parse::<u32>().ok(),
            "maximized" => maximized = v == "1" || v.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }
    let g = Geometry {
        x: x?,
        y: y?,
        w: w?,
        h: h?,
        maximized,
    };
    if !(MIN_W..=MAX_W).contains(&g.w) || !(MIN_H..=MAX_H).contains(&g.h) {
        return None;
    }
    Some(g)
}

pub fn load(path: &Path) -> Option<Geometry> {
    parse(&std::fs::read_to_string(path).ok()?)
}

/// 原子替换（临时文件 + rename），避免关闭瞬间被打断留下半截状态文件。
pub fn save(path: &Path, g: &Geometry) {
    let text = format!(
        "# K music 主窗口几何（物理像素，由 GUI 自动维护）\nx={}\ny={}\nw={}\nh={}\nmaximized={}\n",
        g.x,
        g.y,
        g.w,
        g.h,
        if g.maximized { 1 } else { 0 }
    );
    let tmp = path.with_extension("state.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// 旧坐标是否仍与当前任一显示器有足够交集。
/// 外接显示器被拔掉 / 分辨率变化后，直接恢复旧坐标会让窗口落在屏幕外且无法拖回。
/// 拿不到显示器列表时返回 true（不擅自否决用户保存的位置）。
pub fn visible_on_any_monitor(g: &Geometry, monitors: &[Monitor]) -> bool {
    if monitors.is_empty() {
        return true;
    }
    let (gx1, gy1) = (g.x as i64, g.y as i64);
    let (gx2, gy2) = (gx1 + g.w as i64, gy1 + g.h as i64);
    monitors.iter().any(|m| {
        let mx1 = m.position().x as i64;
        let my1 = m.position().y as i64;
        let mx2 = mx1 + m.size().width as i64;
        let my2 = my1 + m.size().height as i64;
        let iw = gx2.min(mx2) - gx1.max(mx1);
        let ih = gy2.min(my2) - gy1.max(my1);
        iw >= MIN_VISIBLE_W && ih >= MIN_VISIBLE_H
    })
}

fn current_geometry(win: &WebviewWindow) -> Geometry {
    let p = win.outer_position().ok();
    let s = win.inner_size().ok();
    Geometry {
        x: p.as_ref().map(|p| p.x).unwrap_or(0),
        y: p.as_ref().map(|p| p.y).unwrap_or(0),
        w: s.as_ref().map(|s| s.width).unwrap_or(DEFAULT_W),
        h: s.as_ref().map(|s| s.height).unwrap_or(DEFAULT_H),
        maximized: win.is_maximized().unwrap_or(false),
    }
}

/// 窗口创建完成后恢复几何（物理像素）。调用方应让窗口先处于隐藏态，恢复完再 `show()`，
/// 避免用户看到「先出现在默认位置、再跳到上次位置」。
pub fn restore(win: &WebviewWindow, g: &Geometry) {
    let _ = win.set_position(Position::Physical(PhysicalPosition::new(g.x, g.y)));
    let _ = win.set_size(Size::Physical(PhysicalSize::new(g.w, g.h)));
    if g.maximized {
        let _ = win.maximize();
    }
}

type SharedGeometry = Arc<Mutex<(Geometry, bool)>>;

fn persist(state: &SharedGeometry, path: &Path, win: &WebviewWindow) {
    let mut guard = state.lock().unwrap();
    // 第二项是 dirty 标记：用户从未动过窗口且没有历史状态时不写盘，保持系统默认居中。
    if !guard.1 {
        return;
    }
    guard.0.maximized = win.is_maximized().unwrap_or(false);
    save(path, &guard.0);
    guard.1 = false; // Destroyed 紧随 CloseRequested 时不重复写
}

/// 注册窗口事件：拖动 / 缩放更新缓存，关闭时一次性落盘。
/// 最大化期间跳过几何更新，保留最大化之前的正常矩形，避免把边框负偏移写进状态。
pub fn track(win: &WebviewWindow, path: PathBuf) {
    let loaded = load(&path);
    let dirty = loaded.is_some();
    let state: SharedGeometry =
        Arc::new(Mutex::new((loaded.unwrap_or_else(|| current_geometry(win)), dirty)));

    let event_win = win.clone();
    let (s_moved, s_resized, s_close, s_destroy) =
        (state.clone(), state.clone(), state.clone(), state.clone());
    let (p_close, p_destroy) = (path.clone(), path.clone());
    let (w_close, w_destroy) = (event_win.clone(), event_win.clone());

    win.on_window_event(move |event| match event {
        WindowEvent::Moved(p) => {
            if event_win.is_maximized().unwrap_or(false) {
                return;
            }
            let mut guard = s_moved.lock().unwrap();
            guard.0.x = p.x;
            guard.0.y = p.y;
            guard.1 = true;
        }
        WindowEvent::Resized(s) => {
            if event_win.is_maximized().unwrap_or(false) {
                return;
            }
            let mut guard = s_resized.lock().unwrap();
            if s.width >= MIN_W && s.height >= MIN_H {
                guard.0.w = s.width;
                guard.0.h = s.height;
                guard.1 = true;
            }
        }
        WindowEvent::CloseRequested { .. } => persist(&s_close, &p_close, &w_close),
        WindowEvent::Destroyed => persist(&s_destroy, &p_destroy, &w_destroy),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_geometry_is_the_full_layout_floor() {
        // #32：重设计布局的硬下限；builder 直接引用这两个常量，改变它即改变窗口下限。
        assert_eq!(MIN_W, 1280);
        assert_eq!(MIN_H, 720);
        assert!(MIN_W < MAX_W && MIN_H < MAX_H);
    }

    #[test]
    fn default_geometry_never_below_the_floor() {
        // #33：首次启动 / 回退默认时不能比硬下限更小，否则音量条等控件仍会被挤出。
        assert_eq!(DEFAULT_W, MIN_W);
        assert_eq!(DEFAULT_H, MIN_H);
        assert!(parse(&format!("x=0\ny=0\nw={}\nh={}\nmaximized=0\n", DEFAULT_W, DEFAULT_H)).is_some());
    }

    fn sample() -> Geometry {
        Geometry { x: 120, y: 80, w: 1280, h: 800, maximized: false }
    }

    #[test]
    fn parse_roundtrip_and_save() {
        let dir = std::env::temp_dir().join(format!("kqk-window-state-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(STATE_FILE);
        let g = Geometry { maximized: true, ..sample() };
        save(&path, &g);
        assert_eq!(load(&path), Some(g), "保存后应能原样读回");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_rejects_broken_or_out_of_range_values() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("x=1\ny=2\nw=1280\n"), None, "缺字段必须判无效");
        assert_eq!(parse("x=no\ny=2\nw=1280\nh=800\n"), None);
        assert_eq!(parse("x=1\ny=2\nw=10\nh=800\n"), None, "小于最小尺寸");
        assert_eq!(parse("x=1\ny=2\nw=99999\nh=800\n"), None, "超上限");
        // 坏行 / 注释 / 未知键应被忽略，不影响有效字段
        assert_eq!(
            parse("# comment\nbroken-line\nunknown=7\nx=-1920\ny=0\nw=1280\nh=720\nmaximized=0\n"),
            Some(Geometry { x: -1920, y: 0, w: 1280, h: 720, maximized: false }),
            "多显示器负坐标必须保留"
        );
    }

    #[test]
    fn parse_accepts_true_false_for_maximized() {
        assert_eq!(parse("x=0\ny=0\nw=1280\nh=720\nmaximized=true\n").unwrap().maximized, true);
        assert_eq!(parse("x=0\ny=0\nw=1280\nh=720\nmaximized=0\n").unwrap().maximized, false);
    }

    #[test]
    fn empty_monitor_list_does_not_veto() {
        assert!(visible_on_any_monitor(&sample(), &[]));
    }
}
