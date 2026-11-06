# MEMORY.md — K music（musicMT）

## 定位与约束
- PC 本地音乐库 + 桌面播放器；Rust workspace：`mt-core`(纯库)/`mt-cli`/`mt-gui`(Tauri 2 + 原生 HTML/JS Material 前端)，Electron 排除。
- 网易云服务端登录/下载 2026-10-29 彻底移除；备份源 = 用户官方 APP + 本地增量合并（规避封号）。
- 文件组织 `config.library.file_scheme`（默认 artist_album，另 album/artist/flat）。
- 版本唯一源 = 根 `Cargo.toml [workspace.package] version`（**0.5.0**），须同步 `VERSION.md` + `app/gui/tauri.conf.json`；min_compatible=0.2.0。

## NCM 规则（易回归）
- 不内置解密，只调外部 `tools/ncm/bin/ncmdump.exe`；唯一开关 `NcmConfig::tool_ready()`，无 enabled/auto_convert。
- 无工具时普通音频入库、`.ncm` 暂存 `<曲库>/download/pendingNcm/`（不删源）；指定工具后 `POST /api/ncm-tool` 回补（不是 /api/ncm/tool）。
- `mt_core::silent_command()` 置 CREATE_NO_WINDOW；合规：不分发解密音频/工具，打包排除 `tools/ncm/bin`。

## 架构
- 模块：config / db(rusqlite bundled) / audio_meta(lofty) / library / import / downloads(UI 隐藏) / serve(axum + 手写 Range 206) / log / lyrics / progress / app。
- 双配置：app 级 `<exe目录>/kqk-app.toml` + 曲库级 `<曲库>/.mt/config.toml`；`Config::merge/split`。`[meta]` 版本兼容→只就地更新 `[meta]`（注释/未知键/键序保留）；冲突→先备份 `*.conflict-<版本>.bak` 再重建并说明原因。
- **构建/打包绝不删 dist 运行时配置**（build.rs 只同步 `dist/frontend/`；package_gui.bat 只拷 exe+README，保留 kqk-app.toml/.guest/library.db/app.log）。
- GUI 前端不嵌入 exe：WebView 加载 axum 本地服务 → **`dist/frontend/` 即实际前端源**，build.rs 编译期同步 `app/gui/frontend`→`dist/frontend`。
- 应用元信息唯一源 `app/core/src/app.rs` → /api/about、/api/status、标题栏、CLI --version 同源。
- `local_id` 自增主键；增量扫描门控 `(file_size,file_mtime)`；spawn_boot_scan；前端相对路径调 /api 免 CORS。
- 时间字段 `download_records.imported_at`（首次入库 UNIX 毫秒，UPDATE 不覆盖）/ `created_at`（Windows birth time，取不到退化 mtime）；旧库 ALTER + 回填，`refresh_created_at()` 校正；`TrackView`/`PlaylistItemView` 与四个曲目 API 都带；空值渲染空单元格，禁止「—」「暂无」占位。

## 前端强约束（防回归）
- 外观：`--primary/--surface/--bg` + `[data-theme=dark]` + `body.skin-active`；三态 light/system/dark，汇聚 `applyAppearance()`→末尾 `syncNativeTitlebar()`。
- **外观权威源 = app 级 `kqk-app.toml [ui]`**（theme/accent/skin_type/skin_preset/skin_accent/appearance_saved），由 /api/status + GET/POST /api/ui-prefs 读写；`localStorage('kqk_appearance')` 只作首屏缓存与旧偏好迁移来源，**自定义皮肤图片 base64 绝不写 TOML**；appearance_saved 区分「旧配置待迁移」与「用户显式设成默认值」。
- **详情分视图隔离**：四页各有 `state.viewSelection`；唯一来源 `viewDetail(view)`；`renderViewDetail` 只写本页内联条。**禁止把同一份数据灌进所有页面**（旧 `fillInlineDetails()` 是 bug 源）。
- **DetailRegion**：未选中→整区隐藏；封面缺失/失败→隐藏封面；空名称/空字段→不渲染对应行。显隐唯一入口 `syncDetailVisibility()`；`fillTrackDetail()`/`fillGroupDetail()` 已删勿复活。CSS 需 `[hidden]{display:none}`（flex 元素原生 hidden 无效）。
- **布局 + 列宽**：4 个 layout_*(1 横向/2 纵向，默认 library=1 其余=2) + **8** 个 width_*(f64 占比和恒 100，默认 24/16/18/6/6/8/11/11)；`save_ui_in_place()` 只就地替换 `[ui]`（保注释/未知键）；累计取整 + `MIN_COL_PX=48`；TOML 须 `doc["ui"].clone().try_into()`；`sanitize()` 需二次下限校正（8 列等比缩放后小列会跌破 4%）。
- **导入/创建时间列**：`all_track_column_keys()` = 8 列，但 **`default_track_columns()` 只有 7 列——创建时间默认不显示**，只能由「设置→列表→歌曲列表显示项」勾选经 /api/columns 打开。`track_columns_saved` 区分「旧配置可迁移（补 imported_at、不补 created_at）」与「用户显式保存过、不得覆盖」。四张歌曲表统一 `visibleTrackCols()`/`trackHeadHtml()`/`trackCellsHtml()`，勿硬编码列；**/api/columns 必须 `save_ui_in_place()`**（`save_app()` 整份重写会抹掉用户注释）。
- **列宽基准与拖拽写回**：`tableAvailWidth()` 必须用 `table.parentElement` 容器宽度（用已撑宽的表格宽度会正反馈挤走末列）；`onUp` **只按像素增量改被拖动的两列占比**，不可按渲染宽度整体重算。
- **首屏外观注入**：端口动态 → localStorage 按 origin 隔离；`serve.rs::index_handler` 把 `[ui]` 内联注入 `<!--KQK_BOOT_APPEARANCE-->`（no-store，JSON 转义 `<>&`），首屏脚本预设 theme/primary **并回写 localStorage**，否则 `applyAppearance()` 会拿陈旧缓存覆盖首帧；旧前端无占位符则降级。新增外观字段须同步「注入 + 首屏脚本 + applyAppearance」三处。
- **首帧空窗底色方案已回退（2026-11-06，强约束）**：`visible(false)`→恢复几何→`show()` 之间 WebView2 不绘制，会露出未着色表面（系统深色下纯黑），**这是窗口位置持久化的固有代价，回退不会让它消失**。曾试「注入 `<style>` 兜底底色 + builder `.background_color()` + system 校正」三层，真机仍黑一下，已整体回退（对应单测/断言一并删除），**勿复活、勿再写成「已修复」**。该方案无持续系统消耗（+0.008 ms/次请求、无常驻线程/定时器），回退理由是无效而非消耗。查证：tao 窗口类 `hbrBackground` 为空刷，`WM_ERASEBKGND` 只填父窗口客户区，WebView2 子窗口覆盖后基本被裁掉；wry 的 builder 着色需 `ICoreWebView2ControllerOptions3`，运行时 `set_background_color` 走 `ICoreWebView2Controller2`（更通用），若以后再查这是第一个该试的点。`TODO.md` C-4 记「启动 Logo（原生 splash，优先级极低，仅启动过慢时才提高）」——**HTML/CSS 里的 Logo 遮不住空窗**（页面尚未绘制）。
- **启动早期单实例（强制、无开关）**：`app/gui/src/single_instance.rs` 在 `run()` 最开头用命名互斥体 `Local\KQK_SINGLE_INSTANCE_com.musicmt.app` 判定，**必须早于配置加载 / DB / axum / `spawn_boot_scan`**（官方 `tauri-plugin-single-instance` 在 `Builder::build()` 才拦，太晚，第二进程已写 DB）。第二进程 `AllowSetForegroundWindow(ASFW_ANY)` → 读 `<exe目录>/kqk-instance.port` → `GET /api/focus`（复用 `set_post_dialog_focus_sink` 置前）→ `exit(0)`。端口文件在服务可响应后原子发布、正常退出由 Guard Drop 删除；互斥体 `bInitialOwner=FALSE` 且从不 wait；创建失败保守放行且不碰端口文件。**不做设置开关**（锁在启动时建立，运行期开关无法同步）。只约束 `mt-gui.exe`，`mt-cli.exe serve` 与 demo 用例不受影响。
- **窗口位置持久化**：`app/gui/src/window_state.rs` 独占读写 `<exe目录>/kqk-window.state`（物理像素 key=value，临时文件 + rename）；**刻意不写 kqk-app.toml**（/api/ui-prefs 从内存副本整段就地写 [ui]，混写会互相覆盖）。builder 的 position/inner_size 只吃逻辑像素，须 build 后 `set_position(Position::Physical)`/`set_size(Size::Physical)`；最大化跳过几何；恢复前 `available_monitors()` 校验可见性。

## 测试与构建
- 测试统一在 `demo/`（勿放 .workbuddy/tmp）：common.py / run_all.py / tests / tools / fixtures/library（持久化，DB 存绝对路径→原地复用，勿复制）。服务进程纪律：失败即回收、按 ExecutablePath 精准清孤儿；`clean_run.py` 用 onerror 重试，不可谎报成功。
- `export PATH="/c/Users/kill3/.cargo/bin:$PATH"`；build `cargo build --release -p mt-gui -p mt-cli`；test `cargo test -p mt-core -p mt-gui`(74+9) / `python demo/run_all.py`(12 用例) / `python demo/tools/run_preview.py`(3 尺寸，含 E 组队列断言)。**改后端后必须先重编 release，demo 端到端才测到新 exe**。
- 发布：改版本号(3 处)→release build→拷 exe 到 dist/、README 用 `tools/dist_readme.md`；保留 dist 运行时配置。**改图标后必须重编 mt-gui**。

## 应用图标（v3 定稿）
- 正式 = **v3 横版字标放大 1.20x**（`app/gui/icons/icon.{svg,png,ico}`）；候选 `icons/v3-candidates/`(H1/H2 已采用/H3)，对比图 `docs/screenshots/icon-v3-horizontal.png`；v1 冻结 `icons/v1/`，v2 否决 `icons/v2-backup/`。
- 工具 `tools/icon_pipeline.py [svg] [png] [ico]`（勿删）：Edge 无头渲染到品红键色→软抠图→面积平均降采样→自写 PNG/ICO；render_svg 的 workdir **必须绝对路径**；字标/飘带几何须分层渲染测量。验证品红残留 0、四角 alpha 0、ICO 6 帧无重叠。
- `tauri.conf.json` bundle.icon 引用，build.rs 编译期嵌入；改图标须 `touch tauri.conf.json && cargo build -p mt-gui`。

## 播放列表（2026-11-06 定稿，**v0.5.0 已实施并验证**）
- 权威文档 = `docs/01` FR-16～FR-20；`docs/03` §2.2/§2.3 已按 `db.rs::init_schema()` 实际 schema 校正，§2.9 记导入导出落地。**批次 0/1a/1b/2 全部落地**：`test_playlists.py` 35 断言、`test_auto_playlist_count.py` 24 断言、`test_playlist_io.py` 34 断言、`run_preview.py` E 组三尺寸 ALL PASS、`cargo test -p mt-core -p mt-gui` 74+9、`demo/run_all.py` 12/12；版本已升 0.5.0。真机目视见 `docs/07` P18。
- 两类统一为「播放上下文」：有名列表持久化（主键 `(playlist_id,local_id)` 禁重复、**不迁移**）；无名列表 = 由触发播放的**来源视图**推导的运行时 `local_id[]` 快照。播放是**全局功能**，非播放列表页专属；仅选中不播放不切换队列，跨 Tab 的 viewSelection 残留不得影响队列。
- `auto_playlist_count`（app 级 [ui]，默认 20 / 上限 200 / 下限 1）**只约束无名列表生成**，不碰任何列表的 DB/UI 行数/导出内容；有名队列 = 全量。构造：src = state.tracks（受搜索过滤与 LIMIT 2000，按界面顺序），V = 播放瞬间**视口可见行**并冻结 `[start,end]`（播放中不再读 DOM）；queue = src.slice(V[0], V[0]+n)；尾不足从紧邻 V 之前连续段向前补；总数不足就不足，不跨过滤补；|V|>n 以当前曲为锚截 n 首。currentIndex = queue.indexOf(点击曲)，**不从队首播**；只快照 id，不引用 state.tracks。
- 格式方案 A：**DB 唯一权威**；内部 JSON 无损备份可回读 + M3U8 **单向投影不回读**（UTF-8 Extended M3U，非 HLS），故无双写不一致。一期导出**单个列表全量**，绝不导出播放队列（否则 n 污染文件）。导入匹配：相对路径→绝对路径→文件名+艺人→标签；去重 ① local_id 级（留首次位置）② 文件名+艺人+时长内容级，必须发生在写 DB 前，主键 INSERT OR IGNORE 兜底；未匹配跳过并报告。网易云只兼容**本地导出文件**（不联网不登录）。
- **P0 已修**：`prune_missing()` 现在同步删除 `playlist_items` 孤儿（原缺陷导致 `track_count` 不符 / 播放到失效 id 中断）；`playlists_handler` / `playlist_tracks_handler` 不再用 `unwrap_or_default()` 吞 DB 错误；播放列表新建/重命名/删除改统一 modal，不再用原生 `prompt` / `confirm`。
- **实施决策 2026-11-06 13:20 全部拍板（19 项，用户采纳推荐方案）**：顺序 + 列表循环（随机只洗当前队列）；换来源一律重建 +「下一首播放」插播；改 n 下次生成生效（快照记生成时 n）；队列跨重启保留并恢复位置，存 **`<曲库根>/.mt/playstate.json`**（普通 JSON，非 TOML 非关系表；换库即换文件；id 失效则丢弃队列）；有名列表改名/删除/移除当前曲不重建队列；失效 id 跳过并提示；专辑沿用曲名字母序。播控**全加**（上/下一首 + 顺序/随机/循环 + 队列面板入口 + 当前队列标题）；导入多副本取第一个并报告歧义、未匹配跳过不建占位、误合并逐条列出；导出同时产 `.json` + `.m3u8`（M3U8 写相对曲库根路径）；**n 落 app 级 `[ui]`**（属应用设置，不随曲库切换）；设置面板新增「播放」分区放 n。
- **实施结果（已落地）**：批次 0 = P0 孤儿清理 + DB 错误不吞 + 重命名/重排/名称约束 + `test_playlists.py`；批次 1a = n + 设置「播放」分区 + `test_auto_playlist_count.py`；批次 1b = 全局队列/播控/`playstate.rs` + E 组断言；批次 2 = `playlist_io.rs` + JSON/M3U8 导入导出 + `test_playlist_io.py`。**版本已升 0.5.0**（`Cargo.toml` + `VERSION.md` + `tauri.conf.json` 三处同步）。代码位置：队列/播控 `app/gui/frontend/app.js`（`freshPlayback/sourceTracks/visibleRange/buildQueue/startPlayback/stepQueue/enqueueNext/renderQueueUI`）、跨重启 `app/core/src/playstate.rs` + `/api/playstate`、导入导出 `app/core/src/playlist_io.rs` + `/api/playlists/:id/export` / `/api/playlist-import` / `/api/pick-playlist-file`。

## 待真机验证
- 歌词浮层、rfd 原生对话框、backdrop-filter 观感、`:has()`（已用 .is-checked 兜底）、Windows 标题栏染色（DWM 任意色仅 Win11 21H2+）、切换曲库目录后旧曲目仍在 DB、真实音频内嵌封面显示效果。
- **启动首帧主题**（连重启 3 次覆盖端口回退）见 docs/07 P15；**首帧空窗底色**三层兜底已回退，黑窗仍是已知问题（重点「系统深色 + 应用浅色」、system 切明暗后重启），见 P15 ⑦ + `TODO.md` C-4；**窗口位置恢复**（多显示器 / 拔外接屏 / 125%·150% DPI）见 P16；**单实例**（重复双击、连续快速双击、最小化后唤起、崩溃残留、`dist` 与 `target` 双副本）见 P17；**播放列表 v0.5.0**（播控手感、`audio.ended` 推进、队列跨重启恢复、rfd 原生对话框焦点、导入报告、导出文件）见 P18。自动化只验 HTML/DOM 与端口文件，真机目视不可替代。

## 协作约定
- 本会话持续报 bug / 改 bug。流程：复现定位 → 改前备份 `.workbuddy/backups/YYYY-MM-DD-<主题>/`（含 MANIFEST.md5）→ 最小改动 → 验证（cargo test -p mt-core、demo/run_all.py、node --check、前端源与 dist/frontend md5 对齐）→ 追加当日日志 + 同步 docs/ 与 VERSION.md。
- 本地 git 已初始化，分支 main，仓库级身份 kill33 <kill33@126.com>，Conventional Commits；.gitignore 排除 target/、dist/ 运行时配置、`tools/ncm/{bin,source}`、demo/.run、.workbuddy/{backups,tmp}；.gitattributes `* -text`。**每轮开工先 `git status --short`；提交只 `git add <自己改的文件>`，禁止 `git add -A`**（多会话并行，勿带走他人中间态）。
- **多会话共享**：`CODEBUDDY.md` 是 IDE 每轮注入所有会话的共享事实入口；改变项目事实（版本/配置结构/接口/构建命令/git 与合规规则）必须同步 CODEBUDDY.md + 本文件 + 当日日志 + 相关 docs/。
- 收尾清理临时文件与残留服务进程（demo/.run 用 clean_run.py，勿留孤儿 mt-cli）；不擅自删除疑似并行会话在用的文件。
- 冒烟测试务必先把 exe 复制到临时目录再跑（**勿直接用 `dist/` 下的 exe**），否则会改写 `dist/kqk-app.toml` 的 `last_library_dir`、污染用户运行时配置。

## 文档
docs/：00 总览 / 01 需求 / 02 架构 / 03 数据模型配置 / 04 技术选型 / 05 曲库目录 / 06 进展 / 07 真机验证方案；根 VERSION.md = 版本变更记录，与 app 严格同步。
每日开发日志（`.workbuddy/memory/YYYY-MM-DD.md`）于 2026-11-06 按**用户明确要求**压缩为极简摘要（提交 `2e8e72a`）；此后新增内容仍只追加，历史摘要可按需再蒸馏，**不得把「append-only」理解成禁止用户授权的压缩**。详细发布历史见 VERSION.md、设计与架构见 docs/、长期约束见本文件。
