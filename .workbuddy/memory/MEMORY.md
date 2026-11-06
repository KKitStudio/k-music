# MEMORY.md — K曲库（musicMT）音乐管理系统

## 定位与约束
- PC 端本地音乐库管理器 + 桌面播放器。Rust 路线：`mt-core` 纯库 + 可替换 UI 壳；PC 壳 = Tauri 2 + Material Web 前端；Electron 排除。
- 网易云服务端登录/下载已于 2026-10-29 彻底移除；备份源 = 用户在官方 APP 维护 + 本地增量合并（规避封号）。
- 文件组织由 `config.library.file_scheme` 决定（默认 `artist_album`，另 `album`/`artist`/`flat`）。
- 版本号唯一数据源 = 根 `Cargo.toml` `[workspace.package] version`（现 **0.4.0**），改动须同步 `VERSION.md` + `app/gui/tauri.conf.json`。`VERSION.md` 为权威变更记录（含兼容版本、min_compatible=0.2.0）。

## NCM 规则（易回归，务必遵守）
- 不内置解密，仅通过可配置路径调外部工具 `tools/ncm/bin/ncmdump.exe`（taurusxin/ncmdump, MIT）。
- 唯一开关 = `NcmConfig::tool_ready()`（工具路径存在即生效）；无 `enabled`/`auto_convert`。
- 无工具 + 导入含 ncm：普通音频入库，`.ncm` 暂存 `<曲库>/download/pendingNcm/`（不删源）；指定工具后 `POST /api/ncm-tool` 自动回补积压（路由是 `/api/ncm-tool` 非 `/api/ncm/tool`）。
- 静默执行：`mt_core::silent_command()` 置 `CREATE_NO_WINDOW=0x08000000`（ncmdump 无静默参数，只能调用侧解决）。
- 合规：绝不重新分发解密音频 / 公开分享 NCM 工具；打包排除 `tools/ncm/bin`。

## 架构现状
- workspace：`app/core`(mt-core) / `app/cli`(mt-cli) / `app/gui`(mt-gui)。模块：config / db(rusqlite bundled) / audio_meta(lofty) / library / import / downloads(UI 隐藏) / serve(axum+手写 Range/206) / log(落 `<曲库>/.mt/app.log`) / app(元信息常量) / lyrics(lrc+内嵌) / progress(导入进度上报)。
- 配置双文件：app 级 `<exe目录>/kqk-app.toml` + 曲库级 `<曲库>/.mt/config.toml`；`Config::merge/split` 转换，旧配置自动迁移。
- 配置版本兼容：`[meta]` 含 `config_version`/`min_compatible_version`；兼容→只就地更新 `[meta]`（注释/未知键/键序保留）；冲突→先备份 `*.conflict-<版本>.bak` 再重建并说明原因。**构建/打包绝不删 dist 运行时配置**（`build.rs` 只同步 `dist/frontend/`；`package_gui.bat` 只拷 exe+README，保留 `kqk-app.toml`/`.guest`/`library.db`/`app.log`）。
- GUI 前端不嵌入 exe：WebView 加载 axum 本地服务；前端由磁盘 `frontend/` 提供 → **`dist/frontend/` 即 GUI 实际前端源**；`build.rs` 编译期自动同步 `app/gui/frontend`→`dist/frontend`。
- 应用元信息（名称/版本/作者/许可证）唯一数据源 = `app/core/src/app.rs`；注入 `APP_VERSION`；`/api/about`、`/api/status.app_version`、关于面板、标题栏、CLI `--version` 同源展示。
- 关键机制：`local_id` 自增主键；增量扫描门控 `(file_size, file_mtime)`；`spawn_boot_scan` 后台扫；前端相对路径调 /api 免 CORS。
- 时间字段：`download_records.imported_at`（首次入库 UNIX 毫秒，UPDATE 不覆盖）/ `created_at`（文件系统创建时间，Windows birth time，取不到退化 mtime）；旧库自动 `ALTER TABLE` + 回填，`refresh_created_at()` 在增量扫描命中时校正。`TrackView`/`PlaylistItemView` 与四个曲目 API 都带；空值渲染空单元格，禁止「—」「暂无」占位。

## 前端要点（务必遵守，防回归）
- 原生 HTML/JS + Material 风。外观：`--primary/--surface/--bg` + `[data-theme="dark"]` + `body.skin-active` 玻璃拟态；主题三态 `light/system/dark`（默认 system），所有外观变化汇聚 `applyAppearance()`→末尾 `syncNativeTitlebar()` 上报标题栏。
- **外观偏好权威源 = app 级 `kqk-app.toml [ui]`**（`theme/accent/skin_type/skin_preset/skin_accent/appearance_saved`），由 `/api/status` + `GET/POST /api/ui-prefs` 读写；`localStorage('kqk_appearance')` 只作首屏缓存与旧偏好迁移来源，**自定义皮肤图片本体（base64）仍只存 localStorage，绝不写 TOML**。`appearance_saved` 区分「旧配置待迁移」与「用户显式设成默认值」。
- **详情渲染「分视图隔离」（强约束）**：四页各有 `state.viewSelection`；展示唯一来源 `viewDetail(view)`；`renderViewDetail(view)` 只写该页内联条，`renderActiveDetail()` 写全局右侧栏。**绝不要写「把同一份数据灌进所有页面」的函数**（旧 `fillInlineDetails()` 即 bug 源）。
- **详情区公共组件 `DetailRegion`**：未选中→整区隐藏；封面缺失/失败→隐藏封面（不显示「无封面」）；空名称/空字段→不渲染对应行（不显示「—」）。显隐仲裁唯一入口 `syncDetailVisibility()`。旧的 `fillTrackDetail()`/`fillGroupDetail()` 已删，勿复活。
- **布局风格 + 列宽**：`UiConfig` 4 个 `layout_*`(1=横向/2=纵向，按页面独立) + **8** 个 `width_*`(f64 占比，和恒 100)；默认 library=1 其余=2，列宽 24/16/18/6/6/8/11/11。落盘 `save_ui_in_place()` 只就地替换 `[ui]` 段（保留注释/未知键）。列宽用累计取整 + `MIN_COL_PX=48`；TOML 须 `doc["ui"].clone().try_into()`。`sanitize()` 需二次下限校正（8 列等比缩放后小列会跌破 4%）。
- **导入/创建时间列（强约束，易回归）**：`all_track_column_keys()` = 8 列（含 `imported_at`/`created_at`），但 **`default_track_columns()` 只有 7 列——「创建时间」默认不显示，只能由「设置 → 列表 → 歌曲列表显示项」勾选后经 `/api/columns` 打开**。`track_columns_saved` 标志区分「旧配置可迁移（补 imported_at、不补 created_at）」与「用户显式保存过、不得覆盖」。四张歌曲表统一走 `visibleTrackCols()`/`trackHeadHtml()`/`trackCellsHtml()`，勿在子页面里硬编码列。`/api/columns` 必须走 `save_ui_in_place()`（不可用 `save_app()` 整份重写，会抹掉用户注释）。
- **列宽基准与拖拽写回**：`tableAvailWidth()` 必须用 `table.parentElement` 容器宽度（用已撑宽的表格宽度会形成正反馈，末尾列被挤出可视区）；拖拽 `onUp` **只按像素增量改被拖动的两列占比**，不可按渲染宽度整体重算（窄窗口浏览器最小列宽会把误差永久写进配置）。
- **测试代码统一在 `demo/`**（勿放 .workbuddy/tmp）：`common.py`/`run_all.py`/`tests/`/`tools/`/`fixtures/library`(持久化，DB 存绝对路径→必须原地复用，勿复制)。服务进程纪律：失败即回收、按 ExecutablePath 精准清理孤儿。`clean_run.py` 用 `onerror` 重试，不可谎报成功。

- **首屏外观注入（防启动主题闪烁，强约束）**：GUI 端口动态 → `localStorage` 按 origin 隔离、缓存不可靠；`serve.rs::index_handler` 在返回 `/` 与 `/index.html` 时把 `[ui]` 外观内联注入 `index.html` 的 `<!--KQK_BOOT_APPEARANCE-->`（`no-store`，JSON 转义 `<>&`），首屏脚本预设 `data-theme`/`--primary` **并回写 localStorage**，否则 body 末尾 `app.js::applyAppearance()` 会拿陈旧缓存把首帧覆盖回去。旧前端无占位符时原样返回降级。新增外观字段必须同步「注入 + 首屏脚本 + applyAppearance」三处。
- **窗口位置持久化（GUI 侧，强约束）**：`app/gui/src/window_state.rs` 独占读写 `<exe目录>/kqk-window.state`（物理像素 `key=value`，临时文件 + rename 原子替换）；**刻意不写 `kqk-app.toml`**，因 `/api/ui-prefs` 从服务线程内存副本整段就地写 `[ui]`，混写会互相覆盖。`WebviewWindowBuilder::position/inner_size` 只吃逻辑像素，必须 build 后用 `set_position(Position::Physical)`/`set_size(Size::Physical)` 恢复（`visible(false)`→恢复→`show()`）；最大化期间跳过几何；恢复前用 `available_monitors()` 校验可见性，外接屏拔掉时回退默认居中。

## 应用图标（v3 横版放大定稿 / v1、v2 备份）
- **当前正式图标 = v3**（2026-11-06 用户反馈「不如 v1 的横版，但 v1 字有些小」→ 回到横版并把字标等比放大 1.20x）：`app/gui/icons/icon.{svg,png,ico}`。构图完全沿用 v1：低对比圆角方块(`#EEEBE7`→`#DBD6D0`, rx=224)+深灰褐 `#4C4741` Segoe Script Bold 手写单行「K music」(K269/music192, dx=36)+下方细旋律飘带；四角透明。字标 bbox x[91,933] y[407,617]，占幅 82%、边距 L91/R90、包围盒中心精确 (512,512)；飘带 x[146,912] y[638,731]，「字底→飘带顶」间距 21px 与 v1 一致。品红残留 0、四角 alpha=0。
- **三档放大候选留存** `app/gui/icons/v3-candidates/`：H1-mild(1.12x, K251/music179, 占幅77%, 边距120)、**H2-medium(1.20x, 已采用)**、H3-large(1.28x, K287/music205, 占幅88%, 边距62)。对比图 `docs/screenshots/icon-v3-horizontal.png`（v1 基准 + 三档 + 64/48/32/16px）。**想要更保守/更饱满只需** 复制对应 SVG 为 `icon.svg` → `python tools/icon_pipeline.py` → `touch app/gui/tauri.conf.json && cargo build -p mt-gui`。曾试 1.45x 实测右边距 0（贴边裁切）已废弃；飘带水平范围固定 146~912 不随字放大（否则超画布），故 H3 飘带短于字标、略显托不住。
- **v1 冻结备份** `app/gui/icons/v1/`（三件套 + README）：K 居中单行(K224/music160, dx=30, 占幅69%)+飘带。**v2 否决备份** `app/gui/icons/v2-backup/`（三件套 + README，含回滚命令）：B 左上方案(K 左上 400 / music 中下 235)，用户看后觉得不如横版；源留存 `v2-candidates/B-left-final.svg`。历史否决：①蓝紫渐变+白双音符(锯齿/粉边/不对齐) ②靛蓝底+Arial 白粗字(对比过高、字体硬)。
- **生成工具 `tools/icon_pipeline.py`（可复用，勿删）**：`python tools/icon_pipeline.py [svg] [png] [ico]`。Edge 无头渲染到品红键色→软抠图(反混色还原真实色)→面积平均降采样→自写 PNG/ICO。`render_svg` 的 workdir **必须绝对路径**（相对路径生成无效 file:/// URL 且 --screenshot 被忽略）。**字标/飘带几何测量须分层渲染**（分别删 `<g fill="none">` / `<text>` 再测），否则飘带的半透明中灰会混进字标抗锯齿像素导致误判；居中用迭代收敛到包围盒中心 (512,512)。验证：品红残留=0、四角 alpha=0、ICO 6 帧 biSize=40 无重叠；用 v1 复现比对 99.7% 像素一致。
- `tauri.conf.json` `bundle.icon` 引用；`build.rs` 的 `tauri_build::build()` 编译期嵌入 exe（**改图标后须重编 mt-gui**）。本轮已 `touch tauri.conf.json && cargo build -p mt-gui`（7.9s 通过），用 **256 帧 BMP 字标中部行**(offset 142+127*1024+100*4) 做像素指纹确认 exe 含 v3、不含 v1/v2；注意取背景平坦区做指纹会三版同时命中、无法区分。

## 待真机验证
- 歌词浮层、rfd 原生对话框、backdrop-filter 观感、`:has()` 已用 `.is-checked` 兜底。
- Windows 原生标题栏染色需真机目视（DWM 任意色仅 Win11 21H2+，Win10 仅明暗跟随）。
- 切换曲库目录后旧曲目仍在 DB（仅文件缺失才孤儿清理）。
- **启动首帧主题**（2026-11-06）：设置深色 / 换主题色后重启，从第 1 帧起不得出现默认浅色；需连续重启 3 次覆盖端口回退场景。自动化只验 HTML/DOM，真机目视见 `docs/07` P15。
- **窗口位置恢复**（2026-11-06）：拖动 / 缩放 / 最大化后关闭再打开，应直接出现在上次位置；重点验证多显示器、拔外接屏、125%/150% DPI 缩放。见 `docs/07` P16。

## 构建与运行
- 工具链：rustup stable / x86_64-pc-windows-msvc + VS。`export PATH="/c/Users/kill3/.cargo/bin:$PATH"`
- build：`cargo build --release -p mt-gui -p mt-cli`；test：`cargo test -p mt-core -p mt-gui`（当前 60 + 5 passed）、`python demo/run_all.py`（当前 8 用例）。产物：`target/release/mt-{cli,gui}.exe` → `dist/`。**改后端后必须先重新 build release，demo 端到端用例才会测到新 exe**。
- 发布流程：改版本号(3 处)→`cargo build --release`→拷 exe 到 `dist/`、README 用 `tools/dist_readme.md`、前端由 build.rs 自动同步；保留 `dist/kqk-app.toml`/`.guest` 运行时配置。**改图标后必须重编 mt-gui**（已验证：旧/新 exe 的 RT_ICON 哈希不同）。
- 验证：CLI `library <目录> [方案] [名称]`；`/api/about`、`/api/status`(含 app_version)、`/api/lyrics/:id`、`/api/import-progress` 可冒烟。

## 日常开发协作约定（2026-11-06 起）
- 本会话用于持续报 bug / 改 bug。**修复流程**：复现并定位根因 → 改动前备份到 `.workbuddy/backups/YYYY-MM-DD-<主题>/`（含 MANIFEST.md5）→ 最小范围改动 → 验证（`cargo test -p mt-core`、`demo/run_all.py`、`node --check`、前端源与 `dist/frontend` md5 对齐）→ 追加当日日志 + 同步 `docs/` 与 `VERSION.md`。
- **版本控制（2026-11-06 起）**：本地 git 仓库已初始化，主分支 `main`，提交身份为**仓库级** `kill33 <kill33@126.com>`（全局身份仍为空）；提交信息用 Conventional Commits。`git status/diff/revert` 现已可用，但**改动前备份保留为第二道保险**。`.gitignore` 排除 `target/` `dist/` 运行时配置/DB/日志 `tools/ncm/{bin,source}` `demo/.run` 可重建夹具 `.workbuddy/{backups,tmp}`；`.gitattributes` 的 `* -text` 禁用行尾转换（保证前端 md5 与 `.bat` CRLF）。改前先对比 md5，改后核对 `dist/frontend` 同步。
- 防回归强约束即上文各节（详情分视图隔离 / DetailRegion / 配置就地更新 / NCM 规则 / 夹具原地复用），改动前先确认不违反。
- 收尾清理临时文件与残留服务进程（`demo/.run/` 用 `clean_run.py`，勿留孤儿 mt-cli）；不擅自删除疑似并行会话在用的文件。
- **多会话共享**：同一项目目录有多个会话并行。`CODEBUDDY.md` 是 IDE 每轮自动注入所有会话的**共享事实入口**；改变项目事实（版本 / 配置结构 / 接口 / 构建命令 / git 与合规规则）必须同步 `CODEBUDDY.md` + 本文件 + 当日日志 + 相关 `docs/`。每轮开工先 `git status --short`；提交时只 `git add <自己改的文件>`，**禁止 `git add -A`** 带走其它会话的中间态。

## 文档
docs/：00 总览 / 01 需求 / 02 架构 / 03 数据模型配置 / 04 技术选型 / 05 曲库目录设计 / 06 进展 / 07 真机验证方案。根 `VERSION.md` = 版本变更记录，与 app 严格同步。
