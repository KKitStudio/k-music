# TODO — 发布可执行包（直发个人 / GitHub Release 共用同一产物）

> 状态：结论已整理，暂缓执行。等当前版本（v0.4.0）暂时稳定后再逐项推进。
> 背景：2026-11-06 关于"如何发给别人 / 如何发 GitHub"的复盘结论。
> 关键澄清：**直发个人** 与 **GitHub Release** 的二进制产物是同一个清理后的 `dist` zip；
> GitHub 仅额外多一层"开源代码 + 官方下载页"。故两者合并为同一条流水线。

---

## 关键约束（执行前必读，避免返工）

1. **GUI 前端未内嵌**：`mt-gui.exe` 通过 axum 本地服务从磁盘 `frontend/` 目录加载页面
   （`app/core/src/serve.rs::resolve_frontend_dir` 第一候选 `exe_dir/frontend`；
   `app/gui/src/lib.rs` 用 `WebviewUrl::External(local_server)`）。
   → **只发 exe 必然空白/打不开，必须连同 `frontend/` 目录一起给。**
2. **依赖系统 WebView2 运行时**：Tauri 2 依赖 Edge WebView2。Win11 自带；Win10 干净安装/LTSC/老版本可能缺失。
   → 缺失则 GUI 窗口打不开，是头号风险点（仅影响最终用户，不影响 CI 构建）。
3. **MSVC 运行库**（`vcruntime140.dll`）：`x86_64-pc-windows-msvc` 动态链接，Win10/11 基本都有，风险低。
   加固可选：编译加 `-C target-feature=+crt-static` 静态链接。
4. **ncm 解密工具是可选外挂**：`detect_ncm_tool()` 仅探测 `<曲库>/.mt/tools/ncmdump.exe`；
   缺失仅关闭 NCM 功能，程序照常运行，不阻碍启动。
5. **已于 2026-11-06 初始化 git 仓库**（`main` 分支，本地提交身份 `kill33 <kill33@126.com>`）。`.gitignore` 忽略 `/target/`、`/dist/`、运行时 toml / DB / 日志、NCM 工具、可重建夹具；`.gitattributes` 用 `* -text` 禁用行尾转换（保证前端 md5 对齐与 `.bat` CRLF）。二进制仍走 Releases，不进仓库。
6. **发布前必须清理本机残留**：`.guest/`（含 `library.db`、`app.log`）与 `kqk-app.toml`
   是当前构建机的曲库路径与空库，带出去会让对方继承，必须删除后再打包。
   （本地手工构建若跑过 app 会留下；**CI 自动构建不跑 app，天然无此残留**。）

---

## 发布流水线（单一产物，两种投放方式共用）

核心产物 = 清理后的 `dist` 文件夹压缩包：`mt-gui.exe` + `mt-cli.exe` + `frontend/` + `README`。

- [ ] **1. 构建**：`cargo build --release -p mt-gui -p mt-cli`
      （产物在 `target/release/`；`mt-cli.exe` 由 GUI 内部调用，必须一起带。）
- [ ] **2. 组装暂存目录 `dist_release/`**：
      - 复制 `target/release/mt-gui.exe`、`target/release/mt-cli.exe`
      - 复制 `app/gui/frontend/`（app.js / index.html / styles.css）→ `dist_release/frontend/`
      - 复制 `tools/dist_readme.md` → `dist_release/README.txt`
- [ ] **3. 确保无本机残留**：暂存目录**不得含** `.guest/`、`kqk-app.toml`
      （本地构建若跑过 app 需手动删；CI 构建无需此步）。
- [ ] **4. 处理 WebView2 前置**（用户偏好「解压即用」+ 控制发布体积，**不随包附带任何安装器/bootstrapper**）：
      - 做法：仅在「安装与使用说明」文档中注明——
        若双击 `mt-gui.exe` 后窗口打不开/白屏，多为缺少 WebView2 运行时，
        请访问微软官网下载安装：<https://go.microsoft.com/fwlink/p/?LinkId=2124703>（Evergreen 引导器）。
      - 放置位置：① zip 内 `README.txt`（即 `tools/dist_readme.md` 的发布版）醒目处；② 仓库 `README.md` 的安装/使用章节。
      - 现状：Win11 默认已带 WebView2，多数直接双击即可；Win10 多数经 Windows Update 已具备，
        仅干净安装/离线/LTSC/老版本可能缺失。
      - 取舍：体积最小（zip 不含任何额外运行时），代价是缺失者需手动点链接安装一次；契合「解压即用 + 小体积」目标。
      - [x] **已实现（2026-11-06）**：GUI 启动预检 WebView2——`app/gui/src/lib.rs` 的 `webview2_check::ensure()` 在创建 Tauri 窗口前调用微软官方 API `GetAvailableCoreWebView2BrowserVersionString`（来自 `webview2-com-sys`，与 Tauri 同源静态链接 `WebView2LoaderStatic`，零新增体积），缺失则原生 `MessageBoxW` 弹窗 + 自动打开下载页后退出。本机单元测试 `local_machine_has_webview2` 验证不误杀已装用户。随下次发版生效。
- [ ] **5. 打 zip**：`kquku-<版本>-win64.zip`
- [ ] **6. 投放（二选一或都做）**：
      - **直发个人**：把 zip 发微信/邮件/U盘 + WebView2 提示；
      - **GitHub Release**：先 `git init` + 提交源码与文档（见下），再把同一 zip 传到 Releases。
        ⚠️ 仓库只放源码/文档，**不放二进制**；二进制集中在 Releases。

---

## GitHub 开源与自动构建（可行，建议纳入）

- [x] **A1. 初始化本地仓库**（2026-11-06 已完成）：
      - `git init -b main` + 仓库级提交身份 `kill33 <kill33@126.com>`；`.gitignore` 已忽略 `/target/`、`/dist/`、`.guest/`、
        本机 `kqk-app.toml`、真实曲库、`tools/ncm/{bin,source}`、`demo/.run/`、可重建夹具；
        新增 `.gitattributes`（`* -text`，禁用行尾自动转换）。
      - 首次提交：`chore: initialize repository with v0.4.0 source and docs`（源码 + 文档 + 脚本，不含任何二进制 / 运行时数据）。
- [ ] **A2. 公开代码**（待执行，需先确认合规范围）：
      - ⚠️ **与原计划的偏差**：`tools/ncm/source/`（上游 ncmdump 完整克隆）与 `tools/ncm/bin/`（工具二进制）**不入库**。
        理由：① 合规承诺「不公开分享 NCM 工具」（见 `docs/04`、`MEMORY.md`）；② `source/` 含嵌套 `.git`，
        直接 `git add` 只会生成悬空 gitlink。仅保留 `tools/ncm/README.md` 作溯源 / 许可证说明。
      - 提交范围：`app/`、`docs/`、`demo/`（源码与脚本）、`tools/`（脚本 + `ncm/README.md`）、`Cargo.toml`、`Cargo.lock`、
        `README.md`、`VERSION.md`、`TODO.md`、`CODEBUDDY.md`、`.gitignore`、`.gitattributes`、`.vscode/`、`.workbuddy/memory/`。
      - 写/补 `README.md`：项目简介、依赖（Rust stable + WebView2）、编译命令、
        运行方式（`mt-gui.exe` 双击 或 `mt-cli.exe serve`）、`frontend/` 须与 exe 同级、ncm 可选说明。（现状已基本覆盖，发布前复核即可。）
- [ ] **B. 新增 `.github/workflows/release.yml` 自动出包**（标准可行）：
      - Runner：`windows-latest`（预装 MSVC 工具链 + Windows SDK，可编译 Tauri）。
      - 步骤：装 Rust → `cargo build --release -p mt-gui -p mt-cli`
        → 组装 `frontend/` + exe + README 到暂存目录 → 打 zip
        → `softprops/action-gh-release` 上传为 Release asset。
      - 触发：打 tag（`v*`）时自动跑；产物即上面的 zip。
      - 优势：CI 不运行 app，**天然无 `.guest/`/`kqk-app.toml` 残留**，比本地手工更干净。
      - 注意：WebView2 是用户运行时依赖，CI 无需也不处理。

---

## 可选代码/工程改进（推荐但非阻塞）

- [ ] **C-1**（可选，非默认）：改 `resolve_frontend_dir` 增加 Tauri `resource_dir()` 回退，支撑 `cargo tauri build` 出 MSI 安装包。
      ⚠️ 当前用户偏好「解压即用」，安装器非主路径，仅当需要"下一步安装"体验时才做。
- [ ] **C-2**：新增/改造 `make_dist.bat`：构建 → 复制 exe+frontend → 删 `.guest/`、`kqk-app.toml` → 产出干净 zip（本地一键打包）。
- [ ] **C-3**：（可选加固）Release 编译加 `-C target-feature=+crt-static`，消除 MSVC 运行库依赖。
- [ ] **C-4｜启动 Logo（优先级极低）**：启动瞬间仍可能露出未着色空窗（WebView2 首次绘制之前）。
      **仅在日后出现启动过慢时，才提高本项优先级，用启动 Logo 增加用户体验**（缓解等待感）。
      ⚠️ 技术约束：写在 HTML/CSS 里的 Logo **遮不住**这段空窗——此时页面尚未绘制，
      必须是**原生层 splash**：① 纯 GDI 无边框小窗口（几乎零开销，推荐）；
      ② 额外的 WebView 窗口（多一次 WebView2 初始化 + 渲染进程，反而拖慢启动，不推荐）；
      ③ 延后 `show()` 到首帧也能遮住，但窗口会晚几十～几百 ms 出现，与「不拖慢启动」冲突。
      背景：2026-11-06 曾试「注入 `<head>` 早期 `<style>` + builder `background_color`」三层兜底，
      真机仍会黑一下，且该改动无持续系统消耗、仅 +0.008 ms/次请求（见 `VERSION.md` 11-06）；
      已按用户决定回退，此项留待以后与原生 splash 一并解决。

---

## 备注
- 以上均基于 2026-11-06 对源码与 `dist/` 的实际核查，非臆测。
- 直发个人 ≡ GitHub Release 的二进制部分；GitHub 仅多"开源代码 + 官方下载页"。
- 建议执行顺序：先做步骤 1–5 产出干净 zip（直发即可用）→ 再做 A/B 公开代码 + 自动 Release → 最后视需要做 C 增强。
