# MEMORY.md — K music（musicMT）

> 防回归不变式索引。细节权威源：`CODEBUDDY.md`（共享事实）、`docs/01·03·07`、`VERSION.md`、当日日志。实施流水不写这里。

## 定位 / 版本 / 发布
- Rust workspace `mt-core`(纯库)/`mt-cli`/`mt-gui`(Tauri 2 + 原生 HTML/CSS/JS)；Electron 排除。网易云服务端登录/下载已移除，备份源 = 官方 APP + 本地增量合并。`config.library.file_scheme` 默认 artist_album。
- 版本唯一源 = 根 `Cargo.toml [workspace.package] version`（**0.5.0**），同步 `VERSION.md` + `app/gui/tauri.conf.json`；min_compatible=0.2.0。
- **发布入口**：crate/产物仍 `mt-gui.exe`；`tools/package_gui.bat` 拷成 `dist/k_music.exe`、删旧 `dist/mt-gui.exe`、`mt-cli.exe` 原样拷入；脚本 CRLF+纯 ASCII、两个 `copy` 带 `if errorlevel 1`（防 exe 占用静默失败）。运行时只依赖 `current_exe()` 目录 / 命名互斥体 / exe 同级 `frontend/`，**无文件名硬引用**；`.vscode` 与历史文档保留旧名。
- **构建/打包绝不删 dist 运行时配置**（kqk-app.toml/.guest/library.db/app.log）。

## 后端
- 模块：config / db(rusqlite bundled) / audio_meta(lofty) / library / import / downloads(UI 隐藏) / serve(axum + 手写 Range 206) / log / lyrics / playlist_io / playstate / progress / app。
- 双配置：app 级 `<exe目录>/kqk-app.toml` + 曲库级 `<曲库>/.mt/config.toml`；`Config::merge/split`。`[meta]` 兼容→只就地更新（保注释/未知键/键序）；冲突→备份 `*.conflict-<版本>.bak` 再重建。**所有写 `[ui]` 的入口必须 `save_ui_in_place()`**（`save_app()` 会抹掉用户注释）。
- 前端不嵌入 exe：WebView 加载 axum → **`dist/frontend/` 即实际前端源**（build.rs 编译期同步）。元信息唯一源 `app/core/src/app.rs` → /api/about、/api/status、标题栏、CLI --version 同源；相对路径调 /api 免 CORS。
- `local_id` 自增主键；增量扫描门控 `(file_size,file_mtime)`；`spawn_boot_scan`。`imported_at`（首次入库 UNIX ms，UPDATE 不覆盖）/ `created_at`（Windows birth time，退化 mtime）；旧库 ALTER+回填、`refresh_created_at()`；四个曲目 API 都带；空值渲染空单元格，**禁止「—」「暂无」占位**。
- NCM：不内置解密，只调外部 `tools/ncm/bin/ncmdump.exe`；唯一开关 `NcmConfig::tool_ready()`（无 enabled/auto_convert）。无工具时 `.ncm` 暂存 `<曲库>/download/pendingNcm/`（不删源），指定工具后 `POST /api/ncm-tool` 回补。`silent_command()` 置 CREATE_NO_WINDOW；不分发解密音频/工具，打包排除 `tools/ncm/bin`。
- **启动早期单实例（强制、无开关）**：`app/gui/src/single_instance.rs` 在 `run()` 最开头用互斥体 `Local\KQK_SINGLE_INSTANCE_com.musicmt.app` 判定，**必须早于配置/DB/axum/扫描**；第二进程 `AllowSetForegroundWindow` → 读 `<exe目录>/kqk-instance.port` → `GET /api/focus` → `exit(0)`；端口文件原子发布、Guard Drop 删除；`bInitialOwner=FALSE` 从不 wait。只约束 GUI 二进制。
- **窗口位置持久化**：`app/gui/src/window_state.rs` 独占 `<exe目录>/kqk-window.state`（物理像素、临时文件+rename）；**刻意不写 kqk-app.toml**（/api/ui-prefs 就地写 [ui]，混写互相覆盖）。builder 只吃逻辑像素，须 build 后 `set_position/set_size(Physical)`；最大化跳过；恢复前 `available_monitors()` 校验。

## 前端强约束
- 外观：`--primary/--surface/--bg` + `[data-theme=dark]` + `body.skin-active`；三态汇聚 `applyAppearance()`→`syncNativeTitlebar()`。**权威源 = app 级 `[ui]`**（/api/status + GET/POST /api/ui-prefs）；`localStorage('kqk_appearance')` 只是首屏缓存/旧偏好迁移，**皮肤图片 base64 绝不写 TOML**；appearance_saved 区分「待迁移」与「显式设默认值」。
- **详情分视图隔离**：四页各有 `state.viewSelection`；唯一来源 `viewDetail(view)`；`renderViewDetail` 只写本页内联条；**禁止把同一份数据灌进所有页面**（旧 `fillInlineDetails()` 是 bug 源）。**DetailRegion**：未选中→整区隐藏；无封面→隐藏封面；空字段→不渲染；唯一入口 `syncDetailVisibility()`；`fillTrackDetail()`/`fillGroupDetail()` 已删勿复活；CSS 需 `[hidden]{display:none}`。
- **布局 + 列宽**：4 个 layout_*(1 横向/2 纵向，默认 library=1 其余=2) + **8** 个 width_*(占比和恒 100，默认 24/16/18/6/6/8/11/11)；累计取整 + `MIN_COL_PX=48`；TOML 须 `doc["ui"].clone().try_into()`；`sanitize()` 需二次下限校正。
- **时间列**：`all_track_column_keys()` 8 列，**`default_track_columns()` 只有 7 列——创建时间默认不显示**，只由「设置→列表→显示项」经 /api/columns 打开。`track_columns_saved` 区分旧配置可迁移（补 imported_at、不补 created_at）与用户显式保存过。四张表统一 `visibleTrackCols()`/`trackHeadHtml()`/`trackCellsHtml()`。
- **列宽基准与拖拽**：`tableAvailWidth()` 用 `table.parentElement` 容器宽度（用撑宽的表格宽度会正反馈挤走末列）；`onUp` **只按像素增量改被拖动的两列占比**。
- **首屏外观注入**：端口动态 → localStorage 按 origin 隔离；`index_handler` 把 `[ui]` 注入 `<!--KQK_BOOT_APPEARANCE-->`（no-store，转义 `<>&`），首屏脚本预设 theme/primary **并回写 localStorage**；旧前端无占位符降级。新增外观字段须同步「注入 + 首屏脚本 + applyAppearance」三处。
- **首帧空窗底色方案已回退（勿复活、勿写成已修复）**：`visible(false)`→恢复几何→`show()` 之间 WebView2 不绘制，系统深色下露黑，是窗口位置持久化的固有代价；三层兜底真机无效已整体回退。若再查先试 wry `ICoreWebView2Controller2::set_background_color`；`TODO.md` C-4 记原生 splash，**HTML/CSS Logo 遮不住空窗**。
- 统一 modal 替代原生 `prompt`/`confirm`；rfd 对话框后须 `restore_focus_after_dialog()`。

## 播放列表（v0.5.0，详见 docs/01 FR-16～FR-20）
- 两类统一「播放上下文」：有名列表持久化（主键 `(playlist_id,local_id)` 禁重复、**不迁移**，队列=全量）；无名列表 = 由触发播放的**来源视图**推导的运行时 `local_id[]` 快照。播放是**全局功能**；仅选中不播放不切换队列，生成后不再读 DOM。
- `auto_playlist_count`（app 级 [ui]，默认 20 / 1–200）**只约束无名列表生成**，不碰 DB/UI 行数/导出。src = 当前视图已加载集合（受搜索与 LIMIT 2000），V = 播放瞬间**视口可见行**并冻结；queue = src.slice(V[0], V[0]+n)，尾不足从紧邻 V 之前连续段前补，总数不足就不足，|V|>n 以当前曲为锚截 n 首；`currentIndex = queue.indexOf(点击曲)`，**不从队首播**。
- 格式方案 A：**DB 唯一权威**；JSON 无损备份可回读 + M3U8 **单向投影不回读**（UTF-8 Extended M3U，非 HLS）。一期导出**单个列表全量**，绝不导出队列。导入匹配：相对路径→绝对路径→文件名+艺人→标签；去重 ① local_id ② 文件名+艺人+时长内容级，**必须发生在写 DB 前**；未匹配跳过并报告。网易云只兼容本地导出文件。队列跨重启存 **`<曲库根>/.mt/playstate.json`**（普通 JSON，换库即换文件，id 失效丢弃）。
- P0 已修：`prune_missing()` 同步删 `playlist_items` 孤儿；handler 不再 `unwrap_or_default()` 吞错。代码：`app/gui/frontend/app.js`（`freshPlayback/sourceTracks/visibleRange/buildQueue/startPlayback/stepQueue/enqueueNext/renderQueueUI`）、`app/core/src/playstate.rs` + `/api/playstate`、`app/core/src/playlist_io.rs` + 三个导入导出 API。
- **「下一首播放」同步追加到无名列表（用户追加要求，勿回归）**：`enqueueNext(id, view)` 必须把被点曲同时写进 `queue` 与 `originalQueue`，**各自按自己的顺序定位**（随机模式不可复用洗牌后 `at`），补 `meta` 并 `savePlaystate()`；**无队列时先按来源视图用既有 n 规则 `buildQueue` 构造无名列表再加入并播放**，不能只提示「请先播放」。有名列表插播只进运行时队列、不写 DB。断言 `run_preview.py` E8 + F10。

## 右键菜单 / 曲库回收站（FR-21，v0.5.0 内补记）
- 四张歌曲表统一菜单：播放、下一首播放、添加到播放列表（含新建并加入）、从本列表移除（仅列表页）、打开文件所在位置、复制信息、删除歌曲（危险项 + 统一 modal 二次确认）。右键即选中行；点击外/Esc/滚动/缩放/切 Tab 关闭；视口边缘自动翻转。
- **删除语义**：文件 `rename` 到 `<曲库>/.mt/trash/`（可手动取回，**不做不可逆删除、不是系统回收站**）+ 单事务删 `download_records` 与 `playlist_items` 全部引用；同名避让 `_1/_2`；移动失败索引不动、索引删失败文件移回。只删 DB 无效（扫描会重新入库）。
- **强不变式**：`library.rs::scan_dir()` 必须跳过 `.mt` / `trash`（曲库根即扫描根），否则回收站文件被重新入库、删除失效。`ensure_within_library()` 词法规范化 + `starts_with(曲库根)`，删除与 reveal 共用，越界拒绝。后端 `library::trash_track()` / `db::delete_track()` / `POST /api/tracks/delete` / `POST /api/tracks/reveal`；前端 `openTrackContextMenu` / `dropFromPlaybackQueue` / `refreshAfterTrackDelete`。验证见 `docs/07` P19。

## 测试 / 构建 / 协作
- 测试统一在 `demo/`（勿放 .workbuddy/tmp）：`fixtures/library` 持久化（DB 存绝对路径→原地复用，勿复制）；服务进程失败即回收、按 ExecutablePath 精准清孤儿；`clean_run.py` onerror 重试、不可谎报成功。
- `export PATH="/c/Users/kill3/.cargo/bin:$PATH"`；build `cargo build --release -p mt-gui -p mt-cli`；test `cargo test -p mt-core -p mt-gui`(78+9) / `python demo/run_all.py`(13) / `python demo/tools/run_preview.py`(3 尺寸，E 队列 + F 菜单)。**改后端先重编 release，改前端把三文件拷进 `dist/frontend/`**（两者 md5 须一致）。发布改版本号(3 处)→release build→`tools/package_gui.bat`；**改图标后必须重编 mt-gui**（`touch tauri.conf.json`）。冒烟测试先把 exe 复制到临时目录再跑，**勿直接用 `dist/` 下的 exe**（会改写 `dist/kqk-app.toml`）。
- 应用图标：当前正式版 = **v5**（`app/gui/icons/icon.{svg,png,ico}`）= 低饱和蓝圆角方块 + 手写「K music」字标（基线 y=496，上移）+ 底部加粗加深微笑飘带（stroke 14 / opacity .78），配色 `#D5E1F2→#B9CCE8` 底 / `#324A6E` 字与飘带。冻结备份：`v1/`（横版字标）、`v2-backup/`（否决）、`v3/`（暖灰艺术字 v3）、`v5/`（v5-blue.*）。工具 `tools/icon_pipeline.py` 勿删，render_svg 的 workdir **必须绝对路径**；其 `chroma_key` 按中性色假设会把纯蓝底洗成青绿，渲染蓝底图标须用一次性修正版抠图 `(R+B)/2 - G` 判品红占比（不要改 `tools/icon_pipeline.py` 本体）。验证：品红残留 0 / 四角 alpha 0 / ICO 6 帧无重叠；exe 内嵌校验 = 在 `target/release/mt-gui.exe` 与 `dist/k_music.exe` 二进制中检索 v5 `icon.ico` 每帧原始字节全部命中、且不含旧 v3 帧。改图标流程：覆盖 `icon.svg` → `python tools/icon_pipeline.py`（或一次性修正脚本生成候选）→ `touch app/gui/tauri.conf.json && cargo build --release -p mt-gui`（图标编译期嵌入），如需刷新发布入口再拷 `target/release/mt-gui.exe`→`dist/k_music.exe`。
- 流程：复现定位 → 改前备份 `.workbuddy/backups/YYYY-MM-DD-<主题>/`（含 MANIFEST.md5）→ 最小改动 → 验证 → 追加当日日志 + 同步 docs/ 与 VERSION.md。本地 git 分支 main、Conventional Commits；.gitignore 排除 target/、dist 运行时配置、`tools/ncm/{bin,source}`、demo/.run、.workbuddy/{backups,tmp}。**每轮开工先 `git status --short`；提交只 `git add <自己改的文件>`，禁止 `git add -A`；未明确要求不自动提交**。
- **多会话共享**：`CODEBUDDY.md` 是 IDE 每轮注入的共享事实入口；改变项目事实必须同步 CODEBUDDY.md + 本文件 + 当日日志 + 相关 docs/。收尾清理临时文件与残留服务进程；不擅自删除疑似并行会话在用的文件。每日日志按**用户明确要求**压缩为极简摘要（提交 `2e8e72a`），此后新增仍只追加。
- docs/：00 总览 / 01 需求 / 02 架构 / 03 数据模型配置 / 04 技术选型 / 05 曲库目录 / 06 进展 / 07 真机验证方案；根 VERSION.md = 变更全表。**待真机目视**：歌词浮层、rfd 对话框、标题栏染色、首帧主题 P15、窗口位置 P16、单实例 P17、播放列表 P18、右键菜单 P19；**首帧空窗黑窗是已知问题**。自动化只验 HTML/DOM 与端口文件，真机不可替代。
