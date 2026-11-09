//! 启动早期单实例锁（Windows）。
//!
//! ## 为什么不用 `tauri-plugin-single-instance`
//! 官方插件在 `Builder::build()` 阶段才判定第二个实例，而本项目在 Builder **之前**
//! 就已经 `db::open()`、启动 axum 并由 `serve::build_router` 触发 `spawn_boot_scan`
//! （全库扫描 + 孤儿清理，会写 DB）。插件拦得太晚：第二个窗口虽不出现，却已经产生
//! 第二份 DB 连接、重复扫描、端口占用与日志交错。因此把判定前移到 `run()` 最开头。
//!
//! ## 机制（全部在 GUI 进程内，零新增依赖、零设置开关）
//! 1. 命名互斥体 `Local\KQK_SINGLE_INSTANCE_<identifier>`。第一个进程持有到退出；
//!    第二个进程看到 `ERROR_ALREADY_EXISTS` 立即结束，此时**还没加载配置 / 开 DB / 起服务**。
//! 2. 第一个进程把 axum 实际端口写入 `<exe目录>/kqk-instance.port`（临时文件 + rename 原子替换）。
//! 3. 第二个进程读端口 → 本机 `GET /api/focus` → 已有实例经 `set_post_dialog_focus_sink`
//!    把主窗口带回前台 → 退出。调用 `AllowSetForegroundWindow(ASFW_ANY)` 解除前台锁，
//!    让已有实例（通常是后台进程）的置前请求不被 Windows 拒绝。
//! 4. 端口文件残留（进程崩溃）不影响正确性：互斥体随进程句柄关闭而销毁；读不到或连不上时
//!    第二个实例仍静默退出，**绝不双开**。
//!
//! ## 边界
//! - 只约束 GUI 二进制（发布入口 `k_music.exe`，构建产物 `mt-gui.exe`；判定基于命名互斥体，与文件名无关）。`mt-cli.exe serve` 不获取该锁，demo 端到端用例不受影响。
//! - 刻意**不做设置开关**：锁在进程启动时建立，运行期改开关无法同步到已启动的进程，
//!   只会引入「设置显示关、实际仍单实例」之类的状态不一致。
//! - 锁名绑定 `tauri.conf.json` 的 identifier（而非 exe 路径），`dist/` 与 `target/release/`
//!   两个副本也视为同一实例；非 Windows 平台暂不强制（保持可编译可运行）。

use std::path::{Path, PathBuf};
use std::time::Duration;

const PORT_FILE: &str = "kqk-instance.port";

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(attrs: *const std::ffi::c_void, initial_owner: i32, name: *const u16) -> isize;
    fn CloseHandle(handle: isize) -> i32;
    fn GetLastError() -> u32;
}

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn AllowSetForegroundWindow(process_id: u32) -> i32;
}

/// 单实例锁句柄 + 端口文件所有权。Guard 必须活到 `run()` 结束；Drop 时清理。
pub struct InstanceGuard {
    #[cfg(windows)]
    handle: isize,
    port_file: PathBuf,
    /// 只有确实持有互斥体的第一个实例才能写 / 删端口文件。
    /// 互斥体创建失败时保守放行（不能因锁故障导致应用起不来），但不得碰端口文件，
    /// 否则会删掉另一个正在运行的实例写下的端口。
    owns_port_file: bool,
}

impl InstanceGuard {
    /// 服务已可响应后发布实际端口，供第二个实例唤起本窗口。
    pub fn publish(&self, port: u16) {
        if !self.owns_port_file || port == 0 {
            return;
        }
        // 临时文件 + rename：避免第二个实例读到写了一半的内容。
        let tmp = self.port_file.with_extension("port.tmp");
        if std::fs::write(&tmp, port.to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.port_file);
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if self.owns_port_file {
            let _ = std::fs::remove_file(&self.port_file);
        }
        #[cfg(windows)]
        if self.handle != 0 {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

/// 端口文件路径：与 `kqk-window.state` 同目录，刻意不写进 `kqk-app.toml`
/// （后者由服务线程内存副本整段就地写 `[ui]`，混写会互相覆盖）。
fn port_path(exe_dir: Option<&Path>) -> PathBuf {
    let base = exe_dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    base.join(PORT_FILE)
}

#[cfg(windows)]
pub fn acquire(exe_dir: Option<&Path>) -> InstanceGuard {
    use std::os::windows::ffi::OsStrExt;

    const ERROR_ALREADY_EXISTS: u32 = 183;
    const ASFW_ANY: u32 = u32::MAX; // (DWORD)-1，允许任意进程置前
    const MUTEX_NAME: &str = "Local\\KQK_SINGLE_INSTANCE_com.musicmt.app";

    let port_file = port_path(exe_dir);
    let name: Vec<u16> = std::ffi::OsStr::new(MUTEX_NAME)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // bInitialOwner = FALSE：只靠 ERROR_ALREADY_EXISTS 判定，从不 WaitForSingleObject。
    // 这样第一个进程崩溃时内核对象随句柄关闭自动销毁，不会把后续启动永久挡住。
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle == 0 {
        let code = unsafe { GetLastError() };
        eprintln!("单实例互斥体创建失败（GetLastError={code}），本次按单实例放行");
        return InstanceGuard {
            handle: 0,
            port_file,
            owns_port_file: false,
        };
    }
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already_exists {
        unsafe {
            CloseHandle(handle);
            // 第二个进程此刻通常由 explorer 拉起、持有前台权限，放开前台锁后
            // 已有实例的 SetForegroundWindow 才不会被系统拒绝。
            AllowSetForegroundWindow(ASFW_ANY);
        }
        notify_existing(&port_file);
        // 无论能否成功唤起窗口都必须退出：绝不出现第二个窗口 / 第二份 DB 扫描。
        std::process::exit(0);
    }
    InstanceGuard {
        handle,
        port_file,
        owns_port_file: true,
    }
}

#[cfg(not(windows))]
pub fn acquire(exe_dir: Option<&Path>) -> InstanceGuard {
    // 项目面向 Windows PC；其它平台暂不强制单实例，保持可编译可运行。
    InstanceGuard {
        port_file: port_path(exe_dir),
        owns_port_file: false,
    }
}

/// 读端口并请求已有实例置前。第一个实例可能刚拿到锁、端口文件尚未落盘，
/// 因此最多等约 400ms；等不到就静默放弃（仍然退出，不双开）。
#[cfg(windows)]
fn notify_existing(path: &Path) {
    for _ in 0..8 {
        if let Some(port) = read_port(path) {
            request_focus(port);
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(windows)]
fn read_port(path: &Path) -> Option<u16> {
    let text = std::fs::read_to_string(path).ok()?;
    parse_port(&text)
}

#[cfg(windows)]
fn parse_port(text: &str) -> Option<u16> {
    let port: u16 = text.trim().parse().ok()?;
    (port != 0).then_some(port)
}

/// 极简本机 HTTP GET，避免为此引入 HTTP 客户端依赖。
/// 返回 true 表示已有实例确认收到请求（读到 `200` 状态行）。
#[cfg(windows)]
fn request_focus(port: u16) -> bool {
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_millis(300)) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(300)));
    if stream.write_all(focus_request(port).as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 64];
    let n = stream.read(&mut buf).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).contains(" 200 ")
}

#[cfg(windows)]
fn focus_request(port: u16) -> String {
    format!("GET /api/focus HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    #[test]
    fn parse_port_accepts_valid_and_rejects_noise() {
        assert_eq!(parse_port("59637"), Some(59637));
        assert_eq!(parse_port(" 1234 \n"), Some(1234));
        assert_eq!(parse_port("65535"), Some(u16::MAX));
        assert_eq!(parse_port("0"), None);
        assert_eq!(parse_port(""), None);
        assert_eq!(parse_port("65536"), None);
        assert_eq!(parse_port("abc"), None);
        assert_eq!(parse_port("12 34"), None);
    }

    #[test]
    fn focus_request_targets_local_focus_endpoint() {
        let req = focus_request(59637);
        assert!(req.starts_with("GET /api/focus HTTP/1.1\r\n"));
        assert!(req.contains("Host: 127.0.0.1:59637\r\n"));
        assert!(req.contains("Connection: close\r\n"));
        assert!(req.ends_with("\r\n\r\n"));
    }

    #[test]
    fn port_file_round_trip() {
        // 用进程 ID + 纳秒时间戳隔离并发测试，避免互相踩踏。
        let p = std::env::temp_dir().join(format!(
            "kqk-si-test-{}-{}.port",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let guard = InstanceGuard {
            handle: 0,
            port_file: p.clone(),
            owns_port_file: true,
        };
        guard.publish(0);
        assert_eq!(read_port(&p), None, "port=0 不得发布");
        guard.publish(61234);
        assert_eq!(read_port(&p), Some(61234));
        drop(guard);
        assert_eq!(read_port(&p), None, "Drop 必须清理端口文件");
    }

    #[test]
    fn named_mutex_detects_second_creator() {
        // 同进程内用唯一名字创建两次：第二次必须命中 ERROR_ALREADY_EXISTS。
        // 测试结束前关闭两个句柄，内核对象随之销毁，不污染全局命名。
        let unique = format!(
            "Local\\KQK_SI_TEST_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let name: Vec<u16> = std::ffi::OsStr::new(unique.as_str())
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let h1 = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            assert_ne!(h1, 0, "第一个互斥体必须创建成功");
            let first_err = GetLastError();
            let h2 = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            assert_ne!(h2, 0);
            let second_err = GetLastError();
            CloseHandle(h2);
            CloseHandle(h1);
            assert_eq!(first_err, 0, "新创建不应报 ALREADY_EXISTS");
            assert_eq!(second_err, 183, "第二次创建必须报 ALREADY_EXISTS");
        }
    }
}
