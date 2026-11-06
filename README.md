# K music — 本地曲库管理 + 桌面播放器

K music（原代号 musicMT）是一个 **PC 端音乐管理系统**（未来可能迁移飞牛 NAS / 安卓）。目标：
本地曲库扫描建库、桌面播放、外部曲库导入（含同名 `.lrc` 歌词同步导入）、歌词显示，
以及从**本地网易云音乐曲库目录**增量合并（`import-netease`，本地操作、不联网/不登录）。

> ⚠️ 设计要点：应用本质是**本地曲库管理 + 播放器**；网易云服务端登录/下载能力已于 2026-10-29 **彻底移除**（规避批量下载封号风险）。
> 当前仅保留「从本地网易云音乐曲库目录增量合并」的本地操作（`mt-cli import-netease`），不再有扫码登录 / 按歌单下载 / 配额等联网能力。

**当前版本：v0.5.1** —— 版本号取自根 `Cargo.toml` 的 `[workspace.package] version`；窗口标题、
「设置 → 关于」与 `mt-cli --version` 三处同源显示。**完整变更记录见 [`VERSION.md`](VERSION.md)**（升级版本号时须同步更新该文件）。

**关于**：作者 kill33 · 联系方式 kill33@126.com · 许可证 MIT。

## 技术栈

- **核心**：纯 Rust（`mt-core` crate，UI 无关）——配置、SQLite、音频元数据、曲库扫描、导入、歌词获取、本地 HTTP 播放服务。
- **CLI**：`mt-cli`（headless / 调试 / 定时任务）。
- **GUI 壳**：`mt-gui`（Tauri 2 + 原生 WebView，前端为原生 HTML/JS + Material 风样式）。
- **播放方案**：前端 HTML5 `<audio>` + Rust localhost HTTP 服务（支持 `Range/206` 断点续传），实现拖动进度与解耦的「文件 URL」接口。
- **关键依赖**：`rusqlite`（SQLite，bundled）、`lofty`（音频元数据与内嵌歌词）、`axum`（本地 HTTP 服务）、`tower-http`（静态资源）、`rfd`（原生文件选择）、`tokio`（异步运行时）、`encoding_rs`（歌词文本编码识别，含 GBK/GB18030）。
- **产物体积优化**：workspace `Cargo.toml` 的 `[profile.release]` 已启用 `opt-level="z"` / `lto="thin"` / `panic="abort"` / `strip=true` / `codegen-units=1`；并移除了 netease 移除后的死依赖（`ureq`+TLS 栈、`aes`/`cbc`/`num-bigint`/`hex`/`base64`/`rand`/`percent-encoding`）。

## 目录结构

```
musicMT/
├─ app/
│  ├─ core/            # mt-core：纯 Rust 核心库（src/ 下 app/audio_meta/config/db/import/library/lyrics/serve 等）
│  ├─ cli/             # mt-cli：命令行二进制
│  └─ gui/             # mt-gui：Tauri 壳 + frontend（index.html/app.js/styles.css）
├─ tools/ncm/          # NCM 转换工具（源码 + 二进制，个人本地使用，不随 release 分发）
├─ dist/               # 发布目录（k_music.exe + mt-cli.exe + frontend/ + README.md）
├─ VERSION.md          # 版本记录（与 app 版本号严格同步，升级时须同步更新）
└─ docs/               # 00 方案总览 / 01 需求 / 02 架构 / 03 数据模型与配置 / 04 技术选型 / 05 曲库数据目录 / 07 真机验证
```

## 构建前提

1. 安装 Rust 工具链：<https://rustup.rs>（stable）。
2. **Windows**：安装 **Visual Studio Build Tools（含 MSVC C++）**；`rusqlite(bundled)` 内置并编译 SQLite C 源码，需要 C 编译器与链接器（已移除 `ureq`/`openssl` 依赖，无需 libssl）。
3. **Linux**：`pkg-config` + `libssl-dev`（或 `openssl-devel`），以及 `gcc`/`clang`、构建工具。
4. **macOS**：Command Line Tools（`xcode-select --install`）。

> 本仓库的 workspace 已包含 `app/core`、`app/cli`、`app/gui` 三个成员。
> `cargo build --release -p mt-gui` 会在编译期自动把 `app/gui/frontend` 同步到 `dist/frontend`（前端不嵌入 exe，从磁盘提供）。

## 构建与运行

```bash
# 依赖解析与编译（首次较慢，需下载 crates.io 依赖）
cargo build --release -p mt-gui     # 构建 GUI
cargo build --release -p mt-cli     # 或仅构建 CLI
cargo test -p mt-core                # 单元测试

# 运行 GUI（自动启动本地 HTTP 服务并打开窗口）
cargo run -p mt-gui

# 运行 CLI 各子命令
cargo run -p mt-cli -- scan                       # 扫描曲库建索引
cargo run -p mt-cli -- import "D:/some/folder"    # 从外部文件夹导入（默认复制，同步导入同名 .lrc）
cargo run -p mt-cli -- import "D:/some/folder" move   # 剪切导入（破坏性，先写后删）
cargo run -p mt-cli -- import-netease            # 从本地网易云曲库目录增量合并（不联网/不登录）
cargo run -p mt-cli -- serve                      # 仅启动本地 HTTP 服务（浏览器访问 http://127.0.0.1:<端口>/）
cargo run -p mt-cli -- --version                  # 版本号 / 作者联系方式
cargo run -p mt-cli -- library "D:/Music/MyLib" artist_album "我的曲库"   # 指定曲库 + 名称 + 结构方案
```

发布目录 `dist/` 可直接双击 `k_music.exe` 使用（自带后端服务 + 桌面窗口，无需先开 CLI）。

> **入口命名**：GUI 的 crate / 构建产物仍为 `mt-gui`（`target/release/mt-gui.exe`），`tools/package_gui.bat` 在拷入 `dist/` 时统一改名为 `k_music.exe`，`mt-cli.exe` 保持不变。因此 `cargo build -p mt-gui`、`.vscode` 调试配置等**均不改**，只有「双击哪个文件」用新名；打包脚本还会清理旧版残留的 `dist/mt-gui.exe`，避免两个入口并存。

## 运行依赖（Windows GUI 用户）

GUI（`k_music.exe`）基于 **Tauri 2 + 系统 Edge WebView2**，需要本机已安装 **Microsoft Edge WebView2 运行时**：

- **Windows 11**：通常已自带，直接双击 `k_music.exe` 即可。
- **Windows 10**：多数经 Windows Update 已具备；仅**干净安装 / 离线 / LTSC / 老版本**可能缺失。
- **若双击后窗口打不开或白屏**：多为缺少 WebView2，请安装 **WebView2 Evergreen 引导器**：
  <https://go.microsoft.com/fwlink/p/?LinkId=2124703>（安装一次后永久可用，无需随包分发）。

## 配置：分两类存放（2026-11-04 拆分）

| 类别 | 位置 | 内容 |
|------|------|------|
| **app 级** | `<exe 目录>/kqk-app.toml` | 应用自身设置：NCM 工具路径、本地网易云导入源、扫描/导入策略、服务器、界面显示列偏好 |
| **曲库级** | `<曲库根>/.mt/config.toml` | 该曲库自身：曲库名称、文件结构方案、目录布局（`download` 体系 / `tools`） |
| **特例** | `<exe 目录>/kqk-app.toml` | **当前曲库路径** `last_library_dir`——唯一放在 app 目录的「曲库相关」项 |

运行期由 `config::Config::merge` 把两份合并为单一视图，业务代码无感知；写回时按段拆分
（`save_app()` / `save_library()`）。**旧版单文件配置会在首次启动时自动迁移**（见 docs/03 §3.3）。

### 配置文件版本兼容与保留策略（2026-11-04）

两份配置都带 `[meta]` 段，记录**生成时的版本号**与**可正常运行的最低版本号**：

```toml
[meta]
config_version = "0.5.1"        # 生成该配置时的构建版本号
min_compatible_version = "0.2.0" # 可正常解析该配置的最低应用版本号
```

- **版本兼容 → 既有配置与其内容完整保留**：只就地更新 `[meta]` 段，注释、手工添加的未知字段、
  键序全部原样不动；版本信息已是最新时**连文件都不触碰**。
- **逻辑冲突（结构/字段不兼容）→ 才允许替换**：先备份原文件为 `*.conflict-<版本>.bak`，
  再重建配置，并**明确说明冲突原因**（日志 + 前端「设置 → 配置文件」分区）。
  重建时仍会**抢救路径类稳定字段**（当前曲库路径 / NCM 工具 / 导入源），无需重新指定曲库。
- **构建/打包不删配置**：`dist/` 下的 `kqk-app.toml`、`.guest/`、`library.db`、`app.log`
  在每次构建时**一律保留**，在 `dist/` 内反复测试无需重新指定曲库。

详见 docs/03 §3.4。

## 功能状态

| 能力 | 状态 |
|------|------|
| 本地曲库扫描建索引（采样率/位深/码率/声道） | ✅ |
| GUI 设置本地曲库目录（保存配置并扫描） | ✅ |
| 曲库浏览 + 选中详情面板（封面/采样率等） | ✅ |
| 底部播放栏（播放/暂停/拖动进度） | ✅ |
| 曲库名称：初始化设置 + 顶栏展示 + 改名 + ✎ 切换曲库 | ✅（2026-11-04） |
| 播放列表（本地自建：新建/加曲/播放/移除/删除） | ✅（2026-11-03） |
| **歌词显示**：同名 `.lrc` + 音频内嵌歌词；按时间轴高亮、自动滚动、点击跳转 | ✅（2026-11-04） |
| **歌词同步导入**：导入歌曲时一并导入同目录同名 `.lrc`（含 ncm 转换路径） | ✅（2026-11-04） |
| **版本号与「关于」面板**（标题栏/顶栏/关于面板/`--version` 四处同源） | ✅（2026-11-04） |
| **配置双文件拆分**（app 级 / 曲库级，含旧配置自动迁移） | ✅（2026-11-04） |
| **配置版本兼容与保留**：兼容时配置内容原样保留，仅更新 `[meta]` 版本；冲突才备份重建并说明原因 | ✅（2026-11-04） |
| **构建不删 dist 配置**：`kqk-app.toml` 等运行时产物在每次构建时保留 | ✅（2026-11-04） |
| **导入进度面板**：逐首明细（复制/移动/转换/暂存/跳过），实时进度条 + 动作日志 + 结果统计 | ✅（2026-11-04） |
| **NCM 工具静默执行**：转换全程不弹终端窗口（`CREATE_NO_WINDOW`），失败时带出工具 stderr | ✅（2026-11-04） |
| **未设置网易云曲库时自动指定**：点「从本地网易云音乐曲库更新」→ 弹目录选择 → 指定后立即更新 | ✅（2026-11-04） |
| GUI 外观自定义：主题模式（浅色 / **跟随系统（默认）** / 暗色）+ 主题色预设/自定义 + 换肤（选背景图自动提取主题色，玻璃拟态）；Windows 原生标题栏随主题色同步染色 | ✅（2026-11-04 实现，2026-11-05 增补跟随系统与标题栏同步；偏好存 localStorage） |
| 设置面板左侧标签导航 + 右侧分栏（外观 / 曲库 / 导入 / 列表 / 关于），切换标签右侧同步更新 | ✅（2026-11-04） |
| 「导入文件夹」弹窗宽版左对齐布局：图标头部、统一输入控件、卡片式单选（选中态高亮） | ✅（2026-11-04） |
| 全站弹框 / 浮层布局与视觉一致性：统一「固定头部 + 可滚动内容 + 固定底栏」，输入/按钮/单选/复选控件尺寸与对齐一致，任意窗口尺寸不越界、不裁切、不重叠 | ✅（2026-11-05，重点重做「发现散落的 .ncm 文件」弹框；4 种窗口尺寸共 256 条几何断言通过） |
| 外部曲库导入（复制/剪切，按 `<艺人>/<专辑>/<曲名>` 归位） | ✅ |
| 从本地网易云音乐曲库目录增量合并（`import-netease`） | ✅（本地操作，不联网/不登录） || 网易云服务端扫码登录 / 按歌单下载 / 配额同步 | ❌ 已于 2026-10-29 彻底移除（规避批量下载封号风险） |
| 封面回退到网易云专辑图（非内嵌时） | ⚠️ 当前仅内嵌封面；外链回退待接入 |
| 音质升级「一次性清单确认」UI | ⚠️ 后端策略待细化，UI 暂未区分 |

## 导入目录里含 .ncm、但未配置 NCM 工具时的行为

**结论：不会丢文件，也不会报错中断——ncm 被暂存起来，等你指定工具后自动回补。**

| 环节 | 行为 |
|---|---|
| **导入时** | 普通音频正常入库；`.ncm` 文件**不入库**，复制/移动到 `<曲库>/download/pendingNcm/` 暂存，导入结果里 `pending` 计数 +1 |
| **源文件** | `copy` 模式保留源目录原件；`move` 模式原件移入 pendingNcm（不删除） |
| **数据库** | ncm **不产生任何曲目记录**，播放列表/搜索中都看不到 |
| **界面提示** | 导入结果显示「待转换 N」；`/api/status` 的 `pending_ncm` 给出积压数量，设置面板 NCM 区显示「未配置（无 NCM 工具时，.ncm 文件会进入 download/pendingNcm 暂存）」 |
| **指定工具后** | `POST /api/ncm-tool` 保存工具路径后**立即自动回补**：`process_pending_ncm` 逐个转换并入库，接口返回 `backlog{imported, skipped, pending, failed}` 统计 |
| **回补失败** | 文件**保留在 pendingNcm**（不丢），下次重新指定工具或重启后再试；失败原因写入 stderr 与日志 |

**唯一判据是 `tool_ready()`**（工具路径指向真实存在的文件）：没有 `enabled` / `auto_convert` 之类的额外开关。**只要指定了工具，所有 .ncm 一律自动转换**，无需再开任何开关。

另有一条兜底路径：若 .ncm 因转换失败被挪到了 `download/` 根目录（不在 `download/ncm`、`download/pendingNcm`、`download/ing` 之内），扫描时会被识别为「散落 .ncm」，通过 `mt-cli ncm-organize` 或 GUI 扫描后的弹窗可将其移入 download 体系按下载逻辑重新处理。

> 关键实现：`app/core/src/import.rs` 的 `import_ncm()`（三分支：有工具转换入库 / 无工具暂存 pendingNcm / 已在 pendingNcm 内则跳过）、`process_pending_ncm()`（积压回补）、`app/core/src/config.rs` 的 `NcmConfig::tool_ready()`（唯一开关）。

## 导入进度与 NCM 静默执行（2026-11-04）

### 逐首级导入进度

导入不再只显示一句「正在更新」，右下角浮出进度面板，**逐首说明在做什么**：

| 元素 | 内容 |
|---|---|
| 进度条 | `已处理 / 总数`，按**源文件**计数；总数未知时显示不确定态动画 |
| 当前动作 | 正在处理的文件名 + 动作（复制 / 移动 / 转换 / 暂存 / 跳过） |
| 动作日志 | 逐条记录，例如 `[复制] 夜色.mp3 → 已入库`、`[转换] 加密歌.ncm → 已入库`、`[暂存] 加密歌.ncm → 待转换` |
| 结果统计 | 入库 / 跳过 / 待转换 / 失败 四项计数 |
| 失败原因 | 最后一次失败写入面板与日志（含 NCM 工具的 stderr） |

**计数口径（刻意分离，避免误读）**：进度条与结果计数按**源文件**统计；动作日志按**真实动作**统计。1 个 ncm 源文件会产生「转换」+「入库」两条日志，但只结算为 1 个源文件。

接口：`GET /api/import-progress` → `{running, source, total, processed, current_file, current_action, imported, skipped, pending, failed, recent[], latest_seq, last_error}`。日志列表带单调递增的 `seq` 游标，前端按 `seq` 增量拉取，不重不漏。

CLI 侧同样有逐首明细（`mt-cli import` / `import-netease`）。

### NCM 转换不再闪终端窗口

**结论：不需要「后台常驻控制台窗口」这类折衷方案 —— 转换全程根本不创建任何窗口。**

- **根因**：GUI 程序是 PE GUI 子系统，ncmdump.exe 是 PE Console 子系统。Windows 在「GUI 进程创建 Console 进程」时**自动分配并显示一个控制台窗口**，所以每转换一首歌就闪一次。
- **上游工具无解**：ncmdump（taurusxin/ncmdump）只有 `-h/-v/-d/-r/-o/-m` 六个参数，**没有任何静默 / 抑制输出选项**，`[Done]` 输出是无条件 `std::cout`。因此只能在**调用侧**解决。
- **本项目做法**：`mt_core::silent_command()` 创建子进程时设置 `CREATE_NO_WINDOW = 0x08000000`（Windows 专有创建标志），进程正常运行但**不分配也不显示控制台窗口**，自然也不会抢焦点。`library.rs` 的 ncm 转换与 `config.rs` 的 `attrib +h` 均已改用。
- **验证**：以「父进程 = GUI 子系统、子进程 = Console 子系统」的组合做对照实验，在子进程内调用 `GetConsoleWindow()`：普通调用返回非空（存在窗口、会闪），`silent_command` 返回空（无窗口、不闪）。
- **附带收益**：改用 `Command::output()` 捕获 stdout/stderr，工具失败时能把**工具自己的错误信息**带进错误消息（此前只有退出码）。

> 关键实现：`app/core/src/lib.rs` 的 `silent_command()`、`app/core/src/library.rs` 的 `convert_ncm()`。

## 合规与法律风险（持续关注）

- 系统**不内置也不实现**任何解密/转码；仅提供可配置接口调用**用户自有**的 NCM 转换工具。
- 对外发布 / 构建 release 时**必须排除 `tools/ncm/bin/`**（不随 app 分发工具与产物）。
- 网易云非官方接口涉及**账号封禁风险**（非版权）。当前已通过**彻底移除服务端联网能力**规避。
- 仅个人非商业自用；**绝不重新分发**解密(转码)后的音频、**绝不公开分享** NCM 工具。

## 已知限制 / 后续

- 部分端点参数与动态配额字段需真实账号试做核实（Phase 2）。
- 升级策略 `ask` 的「清单确认」UI、ID3 校正、按专辑虚拟视图等留待 Phase 2。
- 应用图标已定稿：`app/gui/icons/icon.{svg,png,ico}` 为 v3（横版字标放大 1.20x），v1 / v2 备份留存于同目录。更换图标后须 `touch app/gui/tauri.conf.json && cargo build -p mt-gui` 重新嵌入 exe。
- GUI 已用**无头 Edge 截图**完成亮色 / 暗色 / 换肤三态与全部弹框的像素级核对，并在 1280×900 / 900×600 / 760×620 / 640×520 四种窗口尺寸下以几何断言验证控件不越界、动作栏固定可见（见 `docs/07` P10）；仍未经**真机 WebView2** 确认的是：歌词浮层、rfd 原生对话框、`backdrop-filter` 毛玻璃观感。
- `tools/package_gui.bat` 必须保持 **CRLF + 纯 ASCII**（详见该文件头部「编写规范」）：LF-only 或含非 ASCII 字符时，cmd.exe 会解析错乱把行拆成非法命令；`if (...)` 块内不得出现未加引号的括号。

## 版本控制与多会话协作

- 本地 git 仓库已初始化（分支 `main`，提交信息遵循 Conventional Commits）；`.gitignore` / `.gitattributes` 规定忽略范围与「禁用行尾转换」，二进制、运行时数据与 NCM 工具均不入库。
- **同一项目目录下有多个会话并行**：`CODEBUDDY.md` 是每轮由 IDE 自动注入所有会话的**共享事实入口**（当前技术栈、命令、版本控制现状、防回归约束、协作规则）；`.workbuddy/memory/MEMORY.md` 为长期约定，`YYYY-MM-DD.md` 为当日工作日志（新增内容只追加；历史日志可按用户要求压缩蒸馏，2026-11-06 已执行，见提交 `2e8e72a`）。
- 任何会话改变项目事实（版本 / 配置结构 / 接口 / 构建命令 / git 与合规规则），须同步 `CODEBUDDY.md` + `MEMORY.md` + 当日日志 + 相关 `docs/`（涉及版本再加 `VERSION.md`）；提交时只 `git add` 自己改动的文件，**禁止 `git add -A`** 带走其它会话的中间态。
