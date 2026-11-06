# K music（musicMT）— 项目共享事实 / 多会话入口

> 本文件由 IDE 在**每轮、每个会话**自动注入上下文，是同一项目目录下多个会话的**共享信息入口**。
> 权威细节：`.workbuddy/memory/MEMORY.md`（长期约定与防回归约束）、`VERSION.md`（版本变更全表）、
> `docs/`（设计文档）、`.workbuddy/memory/YYYY-MM-DD.md`（当日工作日志：新增只追加；历史日志可按用户要求压缩蒸馏）。
> **任何会话改变了下列事实，必须同步：本文件 + MEMORY.md + 当日日志 + 相关 docs/（涉及版本再加 VERSION.md）。**
> 最近更新：2026-11-06（**播放列表功能已完整实施并升版 v0.5.0**：批次 0/1a/1b/2 全部落地；n 落 app 级 `[ui]`、队列存 `<曲库根>/.mt/playstate.json`、播控全加、JSON + M3U8 导入导出；19 项决策均已兑现；**发布入口改名 `k_music.exe`（TODO A3 已实施）**；**四张歌曲表统一右键菜单（FR-21）+ 曲库回收站删除 + 「下一首播放」同步追加到无名列表**；**应用图标正式定为 v5 低饱和蓝版（15:22 用户拍板，已重编嵌入 exe）**；**新增 §0 版本号规则 `a.b.c`（16:19 用户定义，所有会话强制遵守）**；**按规则升版 v0.5.1**（FR-21 三处反馈修复记一次 `c`）**；**改动收尾自动提交（16:36 用户要求，见 §1 第 2 条）**）。
>
> ⚠️ **旧版覆盖声明**：若你的上下文里还残留本文件的早期版本（描述为「无 git 仓库」「网易云备份下载工具」
> 「TypeScript 严格模式」「musiclibrary.exe」「npm run dev」等），那些内容**已作废**；一律以当前这份为准，
> 且**不要据旧内容写回文档或提交**。已开着的会话如需同步，只需重新读一次本文件（只读），无需再发起文档更新。

## 0. 版本号规则（2026-11-06 用户定义，所有会话强制遵守）

格式固定 **`a.b.c`**（三段十进制整数）。**本规则由用户于 2026-11-06 16:19 定义，优先级高于任何会话此前的升版习惯。**

| 段 | 含义 | 递增规则 | 归零规则 |
|---|---|---|---|
| `a` | 大版本 | **只由用户指定**，当前 **0**；任何会话**不得自行 +1** | `a` 变动时 `b`、`c` 一并归零重新计数 |
| `b` | 小版本 | **每完成一组大需求** +1 | `a` 变动时归零重新计数 |
| `c` | 修订号 | **每改一次 bug 或小需求** +1 | `b` +1 时归零重新计数（新小版本从 `a.b.0` 起步） |

执行约定（所有会话照做）：
1. **生效时间**：2026-11-06 16:19 起。**v0.5.0 及之前的条目是历史版本，不追溯改写、不重打已发布版本号**；规则生效后首个版本 = **v0.5.1**（FR-21 三处反馈修复记一次 `c`）。
2. **一次收尾只改一次号**：复现定位 → 备份 → 最小改动 → 验证 → 收尾时统一改号；禁止一轮内反复改号，也禁止「改了功能却把版本号留在旧值上补记」。
3. **改号必须同步**：根 `Cargo.toml [workspace.package] version`（**唯一数据源**）+ `app/gui/tauri.conf.json` + `VERSION.md` 头部「当前版本」表 + `VERSION.md` 变更记录追加该版本完整明细；随后 `cargo build --release -p mt-gui -p mt-cli` 刷新产物。
4. **判定权**：`a` 由用户明确指定；`b` 的「一组大需求」在一组需求收尾时由用户确认（会话可提出版本建议，但**不得擅自升 `a` 或 `b`**）；`c` 按「**一次改动收尾并成功产出用于交付的 release 可执行文件**」+1 —— 一轮内修多个 bug 仍只 +1，调试期重复编译与纯文档改动（不产出 exe）不计数。
5. 本规则同时写入 `VERSION.md` 头部、`.workbuddy/memory/MEMORY.md` 与当日日志；改变规则本身同样要同步四处。

## 1. 多会话协作规则（务必遵守）
1. **开工先看** `git status --short`。若存在其它会话的未提交改动，不要覆盖；自动提交时**跳过来源不明的中间态文件**（不 add、不 commit），并在收尾汇报里点名说明。
2. **改动收尾自动提交（2026-11-06 16:36 用户明确要求「改动后自动提交，不要每次都问我」，所有会话遵守）**：验证通过且文档同步后**直接 `git commit`，不再逐次征询「要不要提交」**。保留的护栏：仍只 `git add <具体文件>`、**禁止 `git add -A`**；提交信息用 Conventional Commits；提交后 `git status --short` 复核，并在汇报里给出 commit hash 与文件数。仅当用户明确说「先别提交」时才暂缓。
3. **日志规则**：当日新增内容**只追加、不覆盖**；**历史日志允许按用户要求压缩蒸馏**（2026-11-06 用户明确要求并执行，见提交 `2e8e72a`）——细节先归入 `MEMORY.md` / `VERSION.md` / `docs/`，再把旧日志压成极简摘要，这不是违规，勿据此「纠正」或回滚。`MEMORY.md` 就地编辑并保持精简。
4. **改动流程**（本会话用于持续报 bug / 改 bug）：复现定位根因 → 备份到 `.workbuddy/backups/YYYY-MM-DD-<主题>/`（含 MANIFEST.md5）→ 最小范围改动 → 验证 → 追加日志 + 同步文档 → **自动提交**（见第 2 条，不再询问）。已有 git 可用 `diff`/`revert`，**备份仍保留为第二道保险**。
5. **验证基线**：`cargo test -p mt-core`（当前 **80 passed**）、`cargo test -p mt-gui`（当前 **9 passed**）、`python demo/run_all.py`（**13 用例**）、`python demo/tools/run_preview.py`（3 尺寸含 E 组播放队列 / F 组右键菜单 / G 组交互反馈断言）、`node --check app/gui/frontend/app.js`、前端源与 `dist/frontend` 三文件 md5 对齐。
6. **提交信息**用 Conventional Commits（`feat:` / `fix:` / `docs:` / `chore:` …）。

## 2. 版本控制现状（2026-11-06 起）
- 本地 git 仓库已初始化：分支 `main`；全局与仓库级身份均 `kill33 <kill33@126.com>`；**未配置 remote、未 push**。
- `.gitignore` 排除：`target/`、`dist/`、`app/data/`、运行时配置/DB/日志（`kqk-app.toml`、`kqk-window.state`、`kqk-instance.port`、`.guest/`、`.mt/`）、`tools/ncm/{bin,source}`、`demo/.run/`、可重建夹具、`.workbuddy/{backups,tmp}`；**`.workbuddy/memory/` 入库**（多会话共享）。
- `.gitattributes` 为 `* -text`，禁用行尾自动转换（`app/gui/frontend` 与 `dist/frontend` 需字节级一致；`tools/package_gui.bat` 必须保持 CRLF）；仓库级 `core.autocrlf=false`。
- 合规：NCM 工具源码与二进制**不入库、不随包分发**，仅保留 `tools/ncm/README.md` 溯源；项目不内置解密。公开代码前须复核 `TODO.md` A2。

## 3. 项目定位（已冻结）
- **K music**：PC 端本地曲库管理 + 桌面播放器。`mt-core` 纯 Rust 库 + 可替换 UI 壳；Electron 排除。
- 网易云**服务端登录/下载已于 2026-10-29 彻底移除**（规避封号）：备份路径 = 用户在官方 APP 维护 + 本地「从本地网易云音乐曲库更新」增量合并（copy、不联网、不登录）。
- NCM：不内置解密，仅通过可配置路径调用外部 `ncmdump`；唯一开关是工具路径是否存在。

## 4. 技术栈与目录（纠正旧描述）
- Rust workspace：`app/core`(mt-core) / `app/cli`(mt-cli) / `app/gui`(mt-gui, Tauri 2 + 系统 WebView2)；SQLite(rusqlite bundled)、axum 本地 HTTP 服务（手写 Range/206）、lofty 元数据。
- 前端为**原生 HTML/JS**（`app/gui/frontend/{index.html,app.js,styles.css}`），**无 npm / TypeScript / 前端构建**；GUI 经 WebView2 加载本地 axum 服务，前端不嵌入 exe。
- **`dist/frontend/` 是 GUI 实际前端源**；`app/gui/build.rs` 编译期从 `app/gui/frontend` 同步 → 改前端后必须核对两者 md5。
- **首屏外观注入**：GUI 服务端口动态（冲突时回退随机端口），`localStorage` 按 origin 隔离故缓存不可靠；`serve.rs::index_handler` 在返回 `/` 与 `/index.html` 时，把 `kqk-app.toml [ui]` 外观内联注入 `index.html` 的 `<!--KQK_BOOT_APPEARANCE-->` 占位符（`no-store`），首屏内联脚本据此预设明暗/强调色并回写 `localStorage`，`app.js` 的 `applyAppearance()` 仍是运行期唯一入口。无占位符的旧前端原样返回、优雅降级。**只注入 JSON，不再注入兜底底色 CSS**（2026-11-06 回退）。
- **启动早期单实例（强制、无开关）**：`app/gui/src/single_instance.rs` 在 `run()` 最开头（任何 DB / axum / `spawn_boot_scan` 之前）用命名互斥体判定；第二进程读 `<exe目录>/kqk-instance.port` → `GET /api/focus` → 已有实例复用 `set_post_dialog_focus_sink` 置前 → `exit(0)`。**不用官方插件**：它在 `Builder::build()` 才拦，太晚，第二进程已写 DB。只约束 GUI 二进制（发布入口 `k_music.exe`，构建产物 `mt-gui.exe`；互斥体名与文件名无关）；`mt-cli.exe serve` 与 demo 用例不受影响。详见 `MEMORY.md`。
- **启动黑窗为已知问题**：WebView2 首次绘制前可能露出一整块纯黑 / 纯白空窗（窗口位置持久化 `visible(false)` 所致），三层兜底已回退、不再声称已修复；`TODO.md` C-4 记极低优先级「启动 Logo（必须原生 splash）」待办。
- **窗口几何状态**：`<exe目录>/kqk-window.state`（物理像素，GUI 进程独占读写）由 `app/gui/src/window_state.rs` 维护；**刻意不写进 `kqk-app.toml`**，避免与 `/api/ui-prefs` 从服务线程内存副本整段就地写 `[ui]` 互相覆盖。恢复前校验坐标仍落在当前显示器上。
- 配置双文件：app 级 `<exe目录>/kqk-app.toml` + 曲库级 `<曲库>/.mt/config.toml`，运行期由 `Config::merge/split` 转换。
- `app/data/`、`dist/`、`.workbuddy/{backups,tmp}`、`demo/.run/` 均为本机运行/临时数据，不入库。
- 应用元信息唯一数据源 `app/core/src/app.rs`；版本号唯一数据源 = 根 `Cargo.toml [workspace.package] version`（当前 **0.5.1**，递增规则见 §0）。
- **发布入口命名（2026-11-06，TODO A3 已实施）**：GUI crate / 构建产物仍为 `mt-gui` / `target/release/mt-gui.exe`，`tools/package_gui.bat` 拷入 `dist/` 时改名 `k_music.exe` 并清理旧 `dist/mt-gui.exe`；`mt-cli.exe` 不变。运行时按 `current_exe()` 目录、命名互斥体、exe 同级 `frontend/` 工作，**与文件名无关**，故 `cargo build -p mt-gui`、`.vscode` 调试配置、`tauri.conf.json` 均不改；只有发布目录的双击入口与真机验证文档用新名。

## 5. 常用命令
```bash
export PATH="/c/Users/kill3/.cargo/bin:$PATH"
cargo test -p mt-core -p mt-gui
cargo build --release -p mt-gui -p mt-cli       # target/release/mt-{gui,cli}.exe → 拷入 dist/
python demo/fixtures/make_fixtures.py --status  # 夹具必须原地复用（DB 存绝对路径）
python demo/run_all.py                          # 13 个端到端用例
mt-cli.exe --version | library <目录> | scan | serve | import | import-netease | ncm-organize
```
> 发布用 `tools/package_gui.bat`（产出 `dist/k_music.exe` GUI 入口 + `dist/mt-cli.exe`）；保留 `dist/kqk-app.toml`、`.guest/` 运行时配置。**改图标后必须重编 `mt-gui`**（图标编译期嵌入 exe）。demo 端到端用例依赖新编译的 `target/release/mt-cli.exe`，改后端后须先 `cargo build --release -p mt-cli`。

## 6. 关键防回归约束（详见 `.workbuddy/memory/MEMORY.md`）
- 详情渲染**分视图隔离**（`state.viewSelection` + `viewDetail(view)`）；禁止"把同一份数据灌进所有页面"的函数。
- 详情区公共组件 `DetailRegion`：未选中 → 整区隐藏；封面缺失/加载失败 → 隐藏；空名称/空字段 → 不渲染对应行（不显示「—」）；显隐唯一入口 `syncDetailVisibility()`。
- 配置写入必须**就地更新**（`save_ui_in_place()`），不可用 `save_app()` 整份重写（会抹掉用户注释与未知键）。
- `default_track_columns()` 只有 7 列（**创建时间默认不显示**，由「设置 → 列表」勾选开启）；列宽拖拽只按像素增量改被拖动的两列占比。
- **主题首帧不得只依赖 `localStorage`**：端口动态 → origin 变 → 缓存失效。新增外观字段须同时维护 `serve.rs` 注入、`index.html` 首屏脚本与 `app.js::applyAppearance()` 三处，任缺其一都会重现启动闪烁；注入的 JSON 必须转义 `<` `>` `&`。
- **首帧空窗底色三层兜底已回退，勿复活**：`visible(false)` → 恢复几何 → `show()` 之间 WebView2 不绘制，黑窗是窗口位置持久化的固有代价；曾试「注入 head 早期 `<style>` + builder `background_color(Color)` + `system` 校正」真机无效，已删除（含单测 / 断言）。**禁止改成等 `DOMContentLoaded` 再 show**（窗口晚几十～几百 ms）；也别用 HTML/CSS 启动 Logo 去遮（页面尚未绘制）。首屏注入本身仅 +0.008 ms（中位数 0.303 vs 0.295 ms），不是回退理由。待办见 `TODO.md` C-4（原生 splash，极低优先级）。
- **单实例判定必须在 `run()` 最开头**：早于 `config::load_startup()` / `db::open()` / `serve::spawn_server()`（`build_router` 会无条件 `spawn_boot_scan` 写 DB）。**不可改用官方插件的 builder 注册方式**——它在 `Builder::build()` 才拦，第二进程已产生 DB 与扫描副作用。互斥体 `bInitialOwner=FALSE` 且从不 wait；端口文件只在服务确认可响应后发布；第二进程无论能否唤起窗口都必须 `exit(0)`，绝不双开。
- **窗口几何用物理像素**：`WebviewWindowBuilder::position()/inner_size()` 只接受逻辑像素，直接塞物理值会在高 DPI 下偏移；必须在窗口创建后用 `set_position(Position::Physical)` / `set_size(Size::Physical)` 恢复（`visible(false)` → 恢复 → `show()`）。最大化期间不可写入几何。
- 改版本号须同步 `Cargo.toml` + `VERSION.md` + `app/gui/tauri.conf.json` 三处，并在 `VERSION.md` 追加该版本明细；**递增规则见本文件 §0（强制）**。

## 7. 当前进度 / 待办
- v0.4.0 已发布：扫描与增量索引、导入、NCM 外挂、播放与歌词、专辑/歌手分组、播放列表基础 CRUD、外观三态主题与换肤、原生标题栏染色、布局风格与列宽、导入/创建时间列、WebView2 启动预检、首屏外观注入、窗口位置持久化、**强制启动早期单实例**。（首帧底色方案已回退。）
- **v0.5.0 已发布（2026-11-06）**：播放列表功能完整实施——P0 孤儿清理 / 名称约束 / 重命名 / ↑↓ 重排 / DB 错误不吞 / 统一 modal；`auto_playlist_count`（n，app 级 `[ui]`，设置「播放」分区）；全局播放上下文（无名队列可视段构造 + 有名列表全量）、上一首 / 下一首、随机 / 列表循环、队列面板、「下一首播放」插播、`<曲库根>/.mt/playstate.json` 跨重启恢复；JSON 无损备份 + M3U8 单向投影的导入导出与两级去重。完整明细见 `VERSION.md`。
- **v0.5.1 已发布（2026-11-06，首个按 §0 规则递增的修订版）**：FR-21 三处用户反馈修复（reveal `raw_arg` 定位、移除 ▶ 播放按钮、跨界面加入列表后刷新）+ 版本号规则落地；不含新大需求故只升 `c`。改号同步 `Cargo.toml` / `tauri.conf.json` / `VERSION.md` 头部表 + 新增变更条目 / `Cargo.lock`（cargo 自动更新 `mt-*`），并重编 release 刷新 `dist/`。验证 80+9 passed、13/13、三尺寸 E/F/G ALL PASS、`mt-cli --version` = `K music v0.5.1`。完整明细见 `VERSION.md` v0.5.1 条目。
- **发布入口改名（TODO A3，2026-11-06 已实施）**：`dist/mt-gui.exe` → `dist/k_music.exe`，由打包脚本自动完成；crate / 构建产物 / 调试配置不变，`mt-cli.exe` 不变。详见 `VERSION.md` v0.5.0 补记与 `TODO.md` A3。
- **歌曲右键菜单 / 曲库回收站（FR-21，2026-11-06 已实施，v0.5.0 内补记）**：曲库 / 播放列表 / 专辑 / 歌手四张表统一菜单（播放 / 下一首播放 / 添加到播放列表 / 从本列表移除 / 打开文件所在位置 / 复制信息 / 删除歌曲）。删除 = 文件 `rename` 到 `<曲库>/.mt/trash/`（**不做不可逆删除、不是系统回收站**）+ 单事务清 `download_records` 与 `playlist_items`，同名避让 `_1/_2`，文件被占用或路径越界一律拒绝且索引不动。新增 `POST /api/tracks/delete`、`POST /api/tracks/reveal`、`library::trash_track()`、`db::delete_track()`、`ensure_within_library()`。**强不变式：`library.rs::scan_dir()` 必须跳过 `.mt` / `trash`**（曲库根即扫描根），否则回收站文件会被重新入库使删除失效；`docs/03` §2.10、`docs/07` P19、`demo/tests/test_track_context_menu.py` 35 断言。
- **「下一首播放」必须同步追加到当前无名播放列表（2026-11-06 15:04 用户追加要求）**：`enqueueNext(id, view)` 把被点歌曲**同时写入 `queue` 与 `originalQueue`**（各自按自己的顺序定位，随机模式下不能复用洗牌后下标），补 `meta` 并 `savePlaystate()` 持久化，关闭随机 / 重启后仍在该列表里；**当前没有队列时先按来源视图用既有 n 规则构造无名列表再加入并开始播放**，不能只提示「请先播放」。有名列表插播只进运行时队列、不写 DB。无头断言见 `run_preview.py` E8 + F10。
- **应用图标 v5 正式定稿（2026-11-06 15:22 用户拍板，v0.5.0 版本号内发布，未另升版）**：正式 `app/gui/icons/icon.{svg,png,ico}` = 低饱和蓝圆角方块（`#D5E1F2→#B9CCE8`）+ 手写「K music」字标（基线 y=496，相对 v4 上移）+ 底部加粗加深微笑飘带（stroke 14 / opacity .78），字与飘带 `#324A6E`；保留 v3「字标 + 飘带」构图，仅换配色与权重。冻结备份 `icons/v1/`（横版字标）、`icons/v2-backup/`（否决）、`icons/v3/`（暖灰艺术字 v3）、`icons/v5/v5-blue.*`；候选 `icons/v3-candidates/`（C-blue-soft / v4-smiley）保留但不采用。已 `touch tauri.conf.json` 并重编 `mt-gui`，在 `target/release/mt-gui.exe` 与 `dist/k_music.exe` 二进制中检索 v5 `icon.ico` 6 帧全部命中、旧 v3 帧 0 命中。**改图标后必须重编 `mt-gui`（编译期嵌入）**；蓝底候选渲染须用一次性修正版抠图 `(R+B)/2 - G` 判品红占比（`tools/icon_pipeline.py` 本体会把蓝底洗成青绿，勿改本体）。详见 `VERSION.md` v0.5.0 补记。
- **FR-21 三处用户反馈修复（2026-11-06 15:41 / 15:51，按 §0 规则**升版 v0.5.1**）**：①「打开文件所在位置」定位错误的根因是
  `Command::arg("/select,<path>")` 在含空格路径（`Alan Walker` / `Different World`）下把 `/select,` 前缀与路径**整体加引号**，
  explorer.exe 解析失败落到默认位置；改 `raw_arg("/select,\"<path>\"")`（前缀在引号外），文件不存在降级打开父目录，
  响应回带 `path`，新增 2 条 Rust 单测。② 播放列表 / 专辑 / 歌手三张表的 ▶ 播放按钮按用户要求移除
  （播放由双击行与右键菜单「播放」承担）；**必须同步删 `styles.css` 的 `.pl-table th:nth-child(2)`**，
  否则移除该列后曲名列被压成 46px。③ 跨界面「添加到已有播放列表」后曲目不刷新：`runTrackContextAction`
  去掉 `view === 'playlists'` 限制，`switchTab('playlists')` 对 `state.currentPlaylist` 重新拉取曲目。
  无头断言新增 G 组（三尺寸 ALL PASS）；详见 `VERSION.md` **v0.5.1 变更条目**与 `docs/01` FR-21。
- **待真机验证 / 已知问题**：歌词浮层、rfd 原生对话框焦点、backdrop-filter 观感、Windows 原生标题栏染色、1280 宽 + 8 列时时间单元格省略号（见 `docs/07` P9/P10/P14）；启动首帧主题见 P15；**整块纯黑 / 纯白空窗为已知问题（三层兜底已回退）**，见 P15 ⑦ + `TODO.md` C-4；窗口位置恢复见 P16（多显示器与高 DPI）；**单实例见 P17**（重复双击 / 快速双击 / 最小化唤起 / 崩溃残留 / 双副本）；**播放列表 v0.5.0 见 P18**（播控手感、队列跨重启恢复、rfd 原生对话框焦点、导入报告、导出文件）；**歌曲右键菜单见 P19**（菜单定位与关闭时机、原生资源管理器焦点、删除确认框、回收站文件、扫描不复活）。
- **播放列表需求已定稿并于 v0.5.0 完整实施（2026-11-06）**：`docs/01` FR-16～FR-20（有名 / 无名两类列表、统一播放上下文、`auto_playlist_count` 默认 20 上限 200、可视段构造规则、全局播放能力、有名列表最小集、JSON 无损备份 + M3U8 单向投影的导入导出与两级去重）；`docs/03` §2.2/§2.3 已按实际 schema 校正，§2.8/§2.9 记播放状态与导入导出落地。用户拍板项均已兑现：不允许列表内重复曲目（主键不迁移）、格式方案 A、去重粒度 ①+②、导出范围一期仅单个列表全量。**勿再按旧记录认为「代码侧只有 v0.4.0 基础 CRUD」**；19 项实施决策与完整变更见下条及 `VERSION.md`。
- **播放列表实施决策 19 项已拍板（2026-11-06 13:20，全部采纳推荐）**：① 循环=顺序+列表循环（随机只洗当前队列）；② 换来源一律重建 +「下一首播放」插播；③ 改 n 下次生成生效；④ 队列跨重启保留并恢复位置 → **`<曲库根>/.mt/playstate.json`**（普通 JSON，非 TOML 非表；换库即换文件）；⑤ 有名列表改名/删除/移除当前曲不重建队列；⑥ 失效 id 跳过提示；⑦ 专辑沿用曲名字母序；⑧ 播控**全加**（上/下一首 + 顺序/随机/循环 + 队列面板入口 + 当前队列标题）；⑨ 导入多副本取第一个并报告歧义、未匹配跳过不建占位、误合并逐条列出；⑩ 导出同时产 `.json` + `.m3u8`（M3U8 写相对曲库根路径）；⑪ **n 落 app 级 `kqk-app.toml [ui]`**（属应用设置，不随曲库切换）；⑫ 设置面板新增「播放」分区；⑬ **批次 0 = P0 孤儿清理 + DB 错误不吞 + 新增 `demo/tests/test_playlists.py`**；⑭ **版本升 0.5.0**（三处同步），顺序 **批次 0 → 1（n + 全局队列）→ 2（导入导出）**。
- 发布流水线与 GitHub 开源待办见 `TODO.md`。
