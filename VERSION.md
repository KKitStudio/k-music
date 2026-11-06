# K music · 版本记录（VERSION）

> 本文件与应用程序版本号**严格同步**。
>
> - **版本号唯一数据源**：根 `Cargo.toml` 的 `[workspace.package] version`，编译期注入 `mt_core::app::APP_VERSION`；
>   `app/gui/tauri.conf.json` 的 `version` 与之保持一致。
> - **显示位置**（同源，永远一致）：原生窗口标题栏、浏览器/WebView 标签标题、
>   设置面板「关于」分区、`mt-cli --version`。
> - **升级版本号时必须同步更新本文件**，并在本文件「变更记录」中补充该版本的完整改动明细。
>
> | 项目 | 值 |
> |------|-----|
> | 当前版本 | **v0.5.1** |
> | 版本号数据源 | `Cargo.toml` → `[workspace.package] version` |
> | 配置文件最低兼容版本 | `0.2.0`（`mt_core::app::MIN_COMPATIBLE_VERSION`，见 docs/03 §3.4） |
> | 许可证 / 作者 | MIT / kill33 <kill33@126.com> |
>
> **入口命名（2026-11-06 起）**：发布目录 GUI 入口为 `k_music.exe`；crate 与构建产物仍为 `mt-gui` / `mt-gui.exe`，
> 由 `tools/package_gui.bat` 在拷贝时改名。本文件 v0.4.0 及更早条目中的 `mt-gui.exe` 是当时的实际文件名，属历史记录，不改写。

---

## 版本号递增规则（2026-11-06 16:19 用户定义，所有会话强制遵守）

格式固定 **`a.b.c`**：

| 段 | 含义 | 递增 | 归零 |
|---|---|---|---|
| `a` | 大版本 | **只由用户指定**，当前 `0`；任何会话不得自行 +1 | `a` 变动时 `b`、`c` 一并归零 |
| `b` | 小版本 | **每完成一组大需求** +1 | `a` 变动时归零重新计数 |
| `c` | 修订号 | **每改一次 bug 或小需求** +1 | `b` +1 时归零（从 `a.b.0` 起步） |

- **生效与历史**：自 2026-11-06 16:19 起生效；**v0.5.0 及之前为历史版本，不追溯改写**。规则生效后首个版本 = **v0.5.1**（FR-21 三处反馈修复记一次 `c`）。
- **一次收尾只改一次号**，且**禁止「改了功能 / 修了 bug 却把版本号留在旧值上补记」**。
- **改号必同步 4 处**：`Cargo.toml [workspace.package] version`（唯一数据源）、`app/gui/tauri.conf.json`、本文件头部「当前版本」表、本文件「变更记录」追加该版本完整明细；随后重编 release 刷新产物。
- **判定权**：`a` 用户指定；`b` 的「一组大需求」由用户确认（会话可提建议，不得擅自升 `a` / `b`）；`c` 按「**一次改动收尾并成功产出用于交付的 release 可执行文件**」计一次（一轮内多个 bug 只 +1，调试期重复编译与纯文档改动不计数）。
- 共享事实入口：`CODEBUDDY.md` §0；长期约定：`.workbuddy/memory/MEMORY.md`。

---

## 变更记录

### v0.5.1 — 2026-11-06

> **首个按 `CODEBUDDY.md` §0 版本号规则递增的修订版本**：FR-21 三处用户反馈修复记一次 `c` +1，
> **0.5.0 → 0.5.1**（`b` 不变）。用户 16:19 定义 `a.b.c` 规则后拍板「立即记为 v0.5.1」。
> 本版本**只含修复与规则落地，不含新大需求**，故不升 `b`。

#### 修复（FR-21 用户反馈三处）

1. **「打开文件所在位置」定位错误**：`serve.rs::track_reveal_handler` 原 `cmd.arg("/select,<path>")` 在含空格路径
   （`Alan Walker\Different World\Faded.wav`）下把 `/select,` 前缀与路径整体加引号，explorer.exe 解析失败落到默认位置。
   改为 `#[cfg(windows)] reveal_explorer_arg()` + `CommandExt::raw_arg("/select,\"<path>\"")`（前缀在引号外、引号只包路径）；
   文件不存在降级打开父目录；成功响应回带 `path`。新增 2 条 Rust 单测。
2. **移除列表内 ▶ 播放按钮**：播放列表 / 专辑 / 歌手三张表的第二列按钮、表头空 `<th>` 与事件绑定全部删除；
   同步删 `styles.css` 的 `.pl-table th:nth-child(2)` 46px 规则（否则曲名列被压窄）。播放由双击行与右键菜单「播放」承担。
3. **跨界面加入已有播放列表后不刷新**：`runTrackContextAction` 去掉 `view === 'playlists'` 限制，
   `switchTab('playlists')` 对 `state.currentPlaylist` 重新 `loadPlaylistTracks()`（`plReq` 并发守卫）。

#### 规则落地

- `CODEBUDDY.md` 新增 **§0 版本号规则**（每轮 IDE 注入，所有会话强制）：`a` 只由用户指定、`b` 一组大需求 +1、`c` 一次 bug / 小需求 +1；
  `a` 变动时 `b` / `c` 一并归零，`b` +1 时 `c` 归零。同步 `VERSION.md` 头部规则块、`MEMORY.md`、当日日志。
- 判定权：`a` 用户指定、`b` 一组大需求由用户确认（会话不得擅自升 `a` / `b`）；`c` 按「一次改动收尾并成功产出用于交付的
  release 可执行文件」计一次，调试期重复编译与纯文档改动不计数。
- 历史：**v0.5.0 及更早条目不追溯改写**；运行时配置里的 `config_version`（生成该配置时的构建版本）保持原值，不随应用版本改动。

#### 验证

- `cargo test -p mt-core -p mt-gui`：**80 + 9 = 89 passed, 0 failed**。
- `python demo/run_all.py`：**13/13**；重新生成预览页后 `run_preview.py` 三尺寸 **E/F/G 组 ALL PASS**（G 组 10 条 ×3）。
- `node --check` / `py_compile` 通过；前端源与 `dist/frontend` 三文件 md5 一致；`mt-cli --version` = `K music v0.5.1`。
- 改号同步：`Cargo.toml`（唯一源）/ `tauri.conf.json` / `VERSION.md` 头部表 + 本条目 / `Cargo.lock`（cargo 自动更新 `mt-*` 三处）；
  release 重编后刷新 `dist/k_music.exe` 与 `dist/mt-cli.exe`，**运行时配置未动**。

#### 不变式（防回归）

- 曲库根即扫描根：`.mt`（含 `trash/`）必须被 `scan_dir` 跳过；删除曲目须「文件进回收站 + 清索引」同时成功，任一步失败回滚，路径越界一律拒绝。
- 「下一首播放」插播项同时进入 `queue` 与 `originalQueue` 并持久化；无队列时先构造无名列表再加入。
- reveal 必须用 `raw_arg`；四张表无 ▶ 按钮且 `.pl-table th:nth-child(2)` 不得复活；跨界面加入列表必须刷新。
- **改了 bug / 小需求必须升 `c`，禁止留在旧版本号上「补记」**。

### v0.5.0 — 2026-11-06

> 播放列表功能完整实施：`docs/01` FR-16～FR-20 与 2026-11-06 13:20 逐条拍板的 19 项决策全部落地。
> 按 minor 升级 **0.4.0 → 0.5.0**，三处同步：根 `Cargo.toml` / `VERSION.md` / `app/gui/tauri.conf.json`。
> 同期产品名定为 **K music**（`mt_core::app::APP_NAME`），不影响版本号规则。

#### 新增 / 变更

- **批次 0 · 播放列表基础链路加固（先修 P0）**
  - `db.rs::prune_missing()` 在删除失效 `download_records` 后，同步删除 `playlist_items` 中
    `NOT EXISTS download_records.local_id` 的孤儿条目；修复左侧 `track_count` 与右侧曲目 JOIN 不一致、
    以及新模型下「播放到失效 id 中断」。
  - 新增 `MAX_PLAYLIST_NAME_CHARS`（60）与 `normalize_playlist_name()`（空名 / 超长 / 同名拒绝）、
    `rename_playlist()`、`move_playlist_item()`（上移 / 下移，交换相邻 `position`）。
  - `playlists_handler` / `playlist_tracks_handler` 不再用 `unwrap_or_default()` 把 DB 异常伪装成「暂无播放列表」；
    GET 显式返回 `ok: true`，失败返回真实 `error`。
  - 播放列表新建 / 重命名 / 删除改走前端统一 modal，移除 `prompt` / `confirm`，规避历史「原生对话框抢焦点」问题。
  - 新增 `demo/tests/test_playlists.py`（35 断言：名称约束 / CRUD / 幂等加曲 / 重排 / 孤儿清理 / 错误不吞 / 重启持久化）。

- **批次 1a · 自动播放列表容量上限 n**
  - `UiConfig::auto_playlist_count` 落 **app 级 `kqk-app.toml [ui]`**（默认 20 / 上限 200 / 下限 1），
    宽容反序列化（数字 / 数字字符串 / 坏值回退默认）+ `clamp_auto_playlist_count()` + `sanitize()` 钳制，
    `UI_FIELD_ORDER` 追加该键；新增 3 条 Rust 单测。
  - `/api/status` 与 `GET/POST /api/ui-prefs` 贯通；设置面板新增「播放」分区（数字输入 + 「应用」按钮）。
  - **作用域不变式**：n 只用于生成无名播放队列，不读不改任何列表的 DB 内容 / UI 行数 / 导出内容；
    有名列表队列恒为全量；播放中修改 n 只影响下一次生成（队列快照记「生成时的 n」）。
  - 新增 `demo/tests/test_auto_playlist_count.py`（24 断言：默认值 / 落盘 / 重启读回 / 越界钳制 /
    `[ui]` 注释与未知段保留 / 其它设置互不干扰 / 创建时间默认隐藏不受影响）。

- **批次 1b · 全局播放上下文与播控**
  - 前端 `app.js` 把两类列表统一为**播放上下文**：`freshPlayback()` / `sourceTracks(view)` /
    `visibleRange(view,total)` / `buildQueue(view,clickedId,named)` / `startPlayback()` / `stepQueue()` /
    `enqueueNext()` / `renderQueueUI()`；队列只快照 `local_id[]` 与显示摘要，**不引用 `state.tracks` 对象**。
  - 无名队列构造按 Q1.A+① / Q2.A / Q3.A：以播放瞬间**视口可见段**首项为起点冻结 `[start,end]`，
    向后取 n 首，尾不足从紧邻 V 之前的连续段向前补，总数不足就不足，`|V|>n` 时以当前曲为锚截连续 n 首；
    `currentIndex = queue.indexOf(点击曲)`，**从点击曲开始播，不从队首播**。
  - 播控全加：上一首 / 下一首、随机（只洗当前队列一次并固定顺序）、列表循环、队列面板（≤200 行）、
    当前队列标题、「下一首播放」插播、清空队列；`audio.ended` 改为推进队列；失效 id 跳过并提示。
  - 新增 `app/core/src/playstate.rs` 与 `GET/POST /api/playstate`：队列跨重启保留并恢复位置，
    落 **`<曲库根>/.mt/playstate.json`**（普通 JSON，非 TOML / 非关系表；换库即换文件）；
    写入前逐个校验 `local_id` 仍在 `download_records`，任一失效即丢弃整份队列并删除文件；
    临时文件 + rename 原子落盘；`clear: true` 删除状态文件。
  - `index.html` / `styles.css` 增加播控按钮、队列抽屉、统一 modal、导入报告与「播放」设置分区。
  - `demo/tools/gen_preview.py` 新增 E 组无头断言（n / 无名与有名队列 / 快照语义 / 播控 / 洗牌 /
    队列面板 / 插播 / 清空 / 设置面板保存 n），三窗口尺寸全部通过。

- **批次 2 · 播放列表导入 / 导出（格式方案 A）**
  - 新增 `app/core/src/playlist_io.rs`：**DB 是运行时唯一权威**，文件只在导入 / 导出时存在，无双写不一致。
    `.json` 为**无损备份、可回读**（`local_id` / `file_path` / `position` / `added_at` + 元数据快照）；
    `.m3u8` 为 **UTF-8 Extended M3U 单向投影、不回读**（`#EXTM3U` / `#EXTINF` + 相对曲库根路径，**非 HLS**）。
  - 解析支持 `.m3u` / `.m3u8` / `.json` / `.csv`；匹配优先级 相对路径 → 绝对路径 → 文件名+艺人 → 标签；
    **两级去重**（① `local_id` 级保留首次位置；② 文件名+艺人+时长内容级）全部发生在写 DB 之前，
    主键 `INSERT OR IGNORE` 兜底；多副本取第一个并报告歧义、未匹配跳过不建占位、误合并逐条列出。
  - 新增 REST：`POST /api/playlists/:id/rename`、`/api/playlists/:id/move`、`/api/playlists/:id/export`、
    `POST /api/playlist-import`、`POST /api/pick-playlist-file`（rfd 原生选择 + 焦点恢复）。
    导出**单个列表全量**，绝不导出当前播放队列（避免 n 的截断结果污染文件），一次同时产出 `.json` + `.m3u8`。
  - 新增 `demo/tests/test_playlist_io.py`（34 断言：双文件导出 / 全量不受 n=1 影响 / 文件名安全化 /
    JSON 无损字段 / M3U8 相对路径 / 四种格式导入 / 两级去重 / 未匹配跳过 / 错误语义）。

- **发布入口重命名为 `k_music.exe`（TODO A3）**
  - 采用「打包时改名、crate 名不动」：`tools/package_gui.bat` 把 `target/release/mt-gui.exe` 拷成 `dist/k_music.exe`，
    `mt-cli.exe` 原样拷入，并清理旧版残留的 `dist/mt-gui.exe`；脚本仍保持 CRLF + 纯 ASCII。
  - 运行时无文件名硬引用（GUI 用 `current_exe()` 取目录、单实例用命名互斥体、前端目录按 exe 目录级联解析），改名安全；
    `target/release/mt-gui.exe`、`cargo build -p mt-gui`、`tauri.conf.json`、`productName`、`.vscode` 调试配置均不变。
  - 同步发布说明与真机验证口径：`README.md`、`tools/dist_readme.md`、`dist/README.md`、`docs/07` §0 与 P1/P15/P16/P17、
    `CODEBUDDY.md`、`MEMORY.md`、`demo/README.md`、`demo/tests/test_single_instance.py`；`docs/02` 仅含模块名 `mt-gui::native_titlebar`，无需改。
  - 改名当时 `dist/k_music.exe` 与 `target/release/mt-gui.exe` md5 一致；`dist/` 运行时配置（`kqk-app.toml` / `.guest/` /
    `kqk-instance.port` / `kqk-window.state`）全部保留，旧 `dist/mt-gui.exe` 已不存在。
  - 验证时发现「GUI 正在运行时 exe 被占用、`copy` 静默失败」：已给 `tools/package_gui.bat` 的两个 `copy` 加
    `if errorlevel 1` 检查并提示「关闭 K music 后重试」，避免打包看似成功却仍是旧 exe。

- **歌曲列表右键菜单（FR-21，2026-11-06 补记，在 v0.5.0 版本号内发布，未另升版）**
  - 用户要求「列表里的歌曲增加右键菜单，含删除歌曲和添加到播放列表，其他功能看着做」；
    **四张歌曲表（曲库 / 播放列表 / 专辑 / 歌手）统一菜单**，右键即选中该行，菜单只存 `local_id` 快照。
  - 菜单项：播放、下一首播放、添加到播放列表（列出全部已有列表 +「新建播放列表并加入」）、
    「从本播放列表移除」（仅播放列表页）、打开文件所在位置、复制歌曲信息、删除歌曲（危险项 + 统一 modal 二次确认）。
  - **删除语义（关键决策）**：文件 `rename` 到 `<曲库>/.mt/trash/`（可手动取回，**不做不可逆删除、不是系统回收站**）
    + 单事务清 `download_records` 与 `playlist_items` 全部引用；同名避让 `_1` / `_2`；
    文件被占用导致移动失败则报错且**索引保持不变**，索引删除失败则把文件移回原位。
    只删 DB 是无效的（文件还在磁盘，下次扫描会重新入库），故两者必须同时发生。
  - **配套不变式**：`library.rs::scan_dir()` 新增跳过 `.mt` / `trash`（曲库根即扫描根），
    否则回收站文件会被扫描重新入库使删除失效；`ensure_within_library()` 词法规范化后校验路径落在曲库根内，
    删除与「打开文件所在位置」共用，越界一律拒绝。
  - 后端新增 `library::trash_track()`、`db::delete_track()`、`POST /api/tracks/delete`、
    `POST /api/tracks/reveal`（4 条 Rust 单测）；前端新增菜单模块与 `dropFromPlaybackQueue()` /
    `refreshAfterTrackDelete()`，删除正在播放的曲目自动顺延并刷新视图与分组计数。
  - **「下一首播放」同步追加到当前无名播放列表（用户 15:04 追加要求）**：`enqueueNext(id, view)` 把被点曲
    同时写入 `queue` 与 `originalQueue`（各自按自己的顺序定位，修掉随机模式下复用洗牌后下标导致关闭随机错位）、
    补 `meta` 并 `savePlaystate()` 落盘；**无队列时先按来源视图用既有 n 规则构造无名列表再加入并播放**，
    不再只提示「请先播放一首歌」。有名列表插播只进运行时队列、不写 DB。新增 F10 无头断言。
  - 验证：`cargo test -p mt-core -p mt-gui` **78 + 9 = 87 passed**；
    `demo/tests/test_track_context_menu.py` **35 断言**；`run_preview.py` F 组三尺寸 ALL PASS；
    `demo/run_all.py` **13/13**。真机目视见 `docs/07` P19。

- **应用图标正式定为 v5 低饱和蓝版（2026-11-06 15:22 用户拍板，v0.5.0 版本号内发布，未另升版）**
  - 正式文件 `app/gui/icons/icon.{svg,png,ico}`：低饱和蓝圆角方块渐变 `#D5E1F2→#B9CCE8` +
    手写「K music」字标（基线 y=496，相对 v4 上移）+ 底部加粗加深微笑飘带（stroke 14 / opacity .78），
    字与飘带统一 `#324A6E`；保留 v3 的「字标 + 飘带」构图，仅换配色与字/飘带权重。
  - 冻结备份：`icons/v1/`（横版字标）、`icons/v2-backup/`（否决）、`icons/v3/`（暖灰艺术字 v3）、
    `icons/v5/v5-blue.{svg,png,ico}`；候选 `icons/v3-candidates/`（C-blue-soft / v4-smiley）保留但不采用。
  - 已 `touch app/gui/tauri.conf.json` 后 `cargo build --release -p mt-gui -p mt-cli`（图标编译期嵌入），
    并刷新 `dist/k_music.exe` / `dist/mt-cli.exe`、删除旧 `dist/mt-gui.exe`；运行时配置未动。
  - **验证**：在 `target/release/mt-gui.exe` 与 `dist/k_music.exe` 二进制中检索 v5 `icon.ico` 的 6 帧原始字节
    全部命中、旧 v3 帧 0 命中 → 嵌入正确；真机目视（任务栏 / 资源管理器 / 标题栏）仍待确认。
  - 注意：`tools/icon_pipeline.py` 的 `chroma_key` 按中性色假设会把纯蓝底洗成青绿；渲染蓝底候选须用
    一次性修正版抠图 `(R+B)/2 - G` 判品红占比，**不要改管线本体**。

- **FR-21 三处用户反馈修复（2026-11-06 15:41 / 15:51，v0.5.0 版本号内补记，未另升版）**
  - ①「打开文件所在位置」定位错误：根因是 `Command::arg("/select,<path>")` 在含空格路径
    （曲库内 `Alan Walker` / `Different World` 等目录）下会把 `/select,` 前缀与路径**整体加引号**，
    explorer.exe 不按标准 argv 解析，遂落到默认位置。改为 `raw_arg("/select,\"<path>\"")`
    —— 前缀在引号外、引号只包路径；文件已不在磁盘时降级为打开其父目录；
    响应回带 `path`（实际定位的绝对路径）便于排查。新增 2 条 Rust 单测覆盖参数构造。
  - ② 歌曲列表内 ▶ 播放按钮移除（用户要求）：播放列表页、专辑页、歌手页表格第二列的
    `<td class="pl-play">`、表头空 `<th>` 与对应事件绑定全部删除；播放改由**双击行**与
    **右键菜单「播放」**承担。同步删除 `styles.css` 的 `.pl-table th:nth-child(2)` 46px 规则，
    否则移除该列后曲名列会被误压成 46px。
  - ③ 跨界面「添加到已有播放列表」后曲目不刷新：`runTrackContextAction` 的 add/new 分支去掉
    `view === 'playlists'` 限制（只要目标列表正是当前打开列表就刷新）；`switchTab('playlists')`
    对 `state.currentPlaylist` 重新 `loadPlaylistTracks()`，兜住任何视图发生的加入 / 移除 / 删除。
  - 无头断言新增 **G 组**（三尺寸 ALL PASS）：四表无 ▶ 按钮、表头列数正确、
    从曲库页加入后立即重拉曲目、切回播放列表页再次重拉。

#### 验证

- `cargo test -p mt-core -p mt-gui`：**发布时 74 + 9 = 83 passed；FR-21 三处修复后 80 + 9 = 89 passed, 0 failed**
  （新增 2 条 `serve.rs::reveal_explorer_arg()` 参数构造单测）。
- `python demo/run_all.py`：**发布时 12/12；FR-21 补记后 13/13**（新增 `test_track_context_menu.py`）；
  `gen_preview.py` + `run_preview.py` 在 1080×645 / 900×600 / 1280×800 三窗口尺寸 **ALL PASS**
  （含 E 组播放队列 / F 组右键菜单 / F10 插播断言）。
- release 构建后 `dist/mt-{gui,cli}.exe` 同步；`app/gui/frontend` 与 `dist/frontend` 三文件 md5 一致。
- 真机目视项见 `docs/07` P18（播控手感、队列跨重启恢复、原生文件对话框焦点、导入报告、导出文件）。

#### 不变式（防回归）

- n 是**应用设置**，落 app 级 `[ui]`，不随曲库切换；`playstate.json` 是**曲库级**运行时状态，两者不得混写。
- 有名播放列表队列不受 n 约束；导出文件不含播放队列；M3U8 不回读、不被当作第二份数据源。
- 列表内不允许同一曲目重复，沿用主键 `(playlist_id, local_id)`，**不做破坏性迁移**。
- 播放来源判定唯一依据 = 触发播放的那个视图；仅选中不播放不切换队列；队列生成后不再读 DOM。
- **曲库根即扫描根**：`.mt`（含 `trash/`）必须被 `scan_dir` 跳过；删除曲目必须「文件进回收站 + 清索引」同时成功，
  任一步失败都要回滚，且路径越界一律拒绝。
- **「下一首播放」的插播项必须同时进入 `queue` 与 `originalQueue` 并持久化**；无队列时先构造无名列表再加入，
  两个快照各自按自己的顺序定位（随机模式不得复用洗牌后下标）。

### v0.4.0 — 2026-11-05

#### 变更

- **应用图标重做（`app/gui/icons/icon.svg` / `icon.png` / `icon.ico` 重新生成，15:5x）**
  - 设计：低对比柔和圆角方块（石墨暖灰渐变 `#EEEBE7` → `#DBD6D0`）+ 深灰褐 `#4C4741` 飘逸手写美术字「K music」
    （Segoe Script 连笔体，K 大 music 小、`dx=30` 拉开间距、居中）+ 一条细旋律飘带收笔，呼应音乐主题。
  - 四角透明；编译期由 `tauri_build` 嵌入 `mt-gui.exe`，任务栏 / 资源管理器 / 窗口标题栏均显示新图标。
  - 验证要点：构建后 exe 内 `RT_GROUP_ICON` 资源为新图；最终视觉效果需在真机双击 `mt-gui.exe` 目视确认。
- **后端逻辑更新**：`app/core/src/config.rs`、`serve.rs`、`db.rs`（11:00–13:46 改动）。
- **前端界面更新**：`app/gui/frontend/app.js`、`index.html`、`styles.css`（15:07–15:10 改动）。

#### 2026-11-06 补充（在 v0.4.0 版本号内发布，未单独升版；v0.5.0 才升 minor）

- **外观主题方案持久化（修复「主题方案设置后没保存到配置文件」）**
  - `UiConfig` 新增 `theme` / `accent` / `skin_type` / `skin_preset` / `skin_accent` / `appearance_saved`，
    落在 app 级 `kqk-app.toml` 的 `[ui]` 段；`/api/status`、`GET/POST /api/ui-prefs` 均带 `appearance`。
  - 前端每次外观改动防抖 POST `/api/ui-prefs`；启动时以配置为权威源还原，或把旧 `localStorage` 偏好一次性迁移进配置。
  - 自定义皮肤图片本体（base64 data URL，可能数百 KB～数 MB）仍只存 `localStorage`，**不写入 TOML**。
- **导入时间 / 创建时间字段贯通**
  - `download_records` 新增 `imported_at`（首次入库时刻，UNIX 毫秒；更新元数据不覆盖）与
    `created_at`（音频文件系统创建时间；Windows birth time，取不到时退化为修改时间）。
  - 旧库自动 `ALTER TABLE` + 回填；增量扫描命中时 `refresh_created_at()` 把近似值校正为真实创建时间。
  - `TrackView` / `PlaylistItemView` 与全部曲目 API（曲库 / 专辑 / 歌手 / 播放列表）都返回两个字段；
    前端列表、详情面板、子页面三级列表均可显示；空值渲染**空单元格**，不出现「—」「暂无」占位。
  - 列宽配置由 6 列扩为 8 列（`width_imported_at` / `width_created_at`），总和恒 100；
    `sanitize()` 增加二次下限校正，避免缩放后小列跌破 4%。
- **创建时间默认隐藏，由「设置 → 列表」控制（用户反馈修正）**
  - `default_track_columns()` = 7 列（含 `imported_at`、**不含** `created_at`）；`all_track_column_keys()` 仍是 8 列，
    设置面板里「创建时间」条目始终存在、默认未勾选，勾选并点「应用显示设置」后才显示。
  - 新增 `track_columns_saved` 标志，区分「旧配置从未显式保存过（自动迁移：补导入时间、不补创建时间）」与
    「用户主动隐藏所有时间列」；`/api/columns` 保存后置 true，重启不再覆盖用户选择。
  - `/api/columns` 改为走 `save_ui_in_place()` 的 `[ui]` 段就地更新（原先 `save_app()` 会整份重写配置、
    抹掉用户手写注释 / 未知键），与布局 / 列宽保存策略统一。
  - 四张歌曲表（曲库 / 播放列表 / 专辑 / 歌手）统一走 `visibleTrackCols()`，显示项设置对四表同时生效。
  - 修复列宽拖拽写回：只按像素增量改被拖动的两列占比，不再用受浏览器最小列宽影响的渲染宽度整体重算
    （窄窗口下一次「拖不动」的拖拽曾污染全部列比例）。
  - 修复 `tableAvailWidth()`：以**容器**宽度而非已被表头撑宽的表格宽度为基准，避免末尾列被挤出可视区。
- **修复「启动时先闪一下默认主题」（2026-11-06）**
  - 根因：`styles.css` 的 `:root` 为浅色默认，外观由 body 末尾 `app.js` 与异步 `/api/status` 应用；且 GUI 服务端口动态（占用时回退随机端口），`localStorage` 按 origin 隔离，端口一变缓存失效，只能等接口返回才切回 TOML 里保存的深色。
  - `app/core/src/serve.rs`：新增 `/` 与 `/index.html` 的 `index_handler`，读磁盘 `index.html` 并把 `kqk-app.toml [ui]` 外观内联注入 `<!--KQK_BOOT_APPEARANCE-->` 占位符（`Cache-Control: no-store`；JSON 转义 `<` `>` `&` 防脚本提前闭合）；旧前端无占位符时原样返回，优雅降级。
  - `app/gui/frontend/index.html`：head 内（CSS 之前）新增首屏内联脚本，按权威源预设 `data-theme` / `data-theme-mode` / `--primary` / `--primary-dark`，并把权威外观回写 `localStorage`（保留自定义皮肤图片本体），使 `app.js::applyAppearance()` 不会用陈旧缓存覆盖首帧。`saved=false` 时沿用本地旧偏好，保持旧配置迁移语义；`app.js` 零改动。
  - 新增 Rust 单测 2 条（注入转义）、端到端用例 `demo/tests/test_boot_appearance.py`（注入 / no-store / 与 `/api/ui-prefs` 一致 / 改主题即时生效 / 旧前端降级）。
- **修复「窗口位置不记忆」（2026-11-06）**
  - 根因：`WebviewWindowBuilder` 只硬编码 `inner_size(1100,720)`，未读取也未保存位置 / 尺寸 / 最大化态；Tauri 2 不自动持久化。
  - 新增 `app/gui/src/window_state.rs`：独占读写 `<exe目录>/kqk-window.state`（物理像素、原子替换）。**不写进 `kqk-app.toml`**——`/api/ui-prefs` 从服务线程内存副本整段就地写 `[ui]`，混写会互相覆盖。
  - 监听 `Moved` / `Resized` / `CloseRequested` / `Destroyed`；最大化期间跳过几何；从未移动过窗口且无历史状态时不写盘（保持系统默认居中）。恢复前用 `available_monitors()` 校验窗口仍与当前显示器有交集，外接屏拔掉 / 改分辨率时回退默认居中。
  - **高 DPI 处理**：Tauri 2.12.5 的 builder `position/inner_size` 只接受逻辑像素，故在窗口创建后以 `visible(false)` → `set_position(Position::Physical)` / `set_size(Size::Physical)` → `show()` 恢复物理像素坐标，避免 125%/150% 缩放下位置偏移，也不会让用户看到位置跳变。
  - 新增 Rust 单测 4 条（状态解析 / 越界回退 / 负坐标 / 空显示器列表）。真机目视验证见 `docs/07` P16。
- **「启动首帧先整体黑一下」：方案经真机验证无效，已整体回退（2026-11-06）**
  - 根因：上一轮为避免位置跳变，窗口以 `visible(false)` 创建、恢复几何后再 `show()`；隐藏窗口里的 WebView2 不绘制，从窗口显示到首次绘制之间露出**未着色表面**，系统深色模式下是纯黑（用户配置恰为显式浅色主题 + 系统深色，对比最明显）。首屏外观注入只消掉了「浅色主题闪」，这块空窗仍在，反而更突出。
  - `serve.rs::index_handler` 在占位符处同时注入 `<style>html{background:...}</style>`：`dark` → `#121830`、`light` → `#f6f8fc`、`system` → 浅色 + `@media (prefers-color-scheme:dark)` 深色。位置早于 `styles.css` 的 `<link>`，后者以同等优先级后到覆盖，最终渐变 / 皮肤外观完全不变；CSS 只由白名单分支生成，不拼接用户可编辑的 TOML 字符串。
  - `app/gui/src/lib.rs`：builder 增加 `.background_color(Color)`（同时设 HWND 背景与 WebView2 `DefaultBackgroundColor`），颜色取 `cfg.ui.theme`；`system` 在窗口创建后读 `win.theme()` 精确校正。**必须在 `show()` 之前调用**——两者都是事件循环消息且 FIFO，先改色再显示；显式 light/dark 不受系统主题影响。
  - **刻意不采用**「等 `DOMContentLoaded` 再 show 窗口」：那会把窗口出现推迟几十～几百毫秒。本次是纯属性设置，无 I/O、无等待，`show()` 时机不变。
  - 启动耗时实测（keep-alive 300 次；同一份 `index.html`，有注入 vs 去掉占位符原样返回）：中位数 **0.303 ms vs 0.295 ms（+0.008 ms）**，均值 +0.020 ms；56KB 静态 `styles.css` 反而更慢（0.403 ms），说明首页耗时由文件读取 / 传输主导，注入可忽略。
  - 新增 Rust 单测 4 条（兜底底色 CSS 1 + 颜色裁决 3）、端到端断言 4 条（dark / light / system 三种首帧底色）。真机目视见 `docs/07` P15 ⑦。
  - **回退结论（用户真机反馈「还是会黑一下」）**：该三层兜底未解决问题，已按用户决定精确回退到 `3ccc7d9` 的三个代码文件——删除注入的 `<style>` 兜底底色与 `boot_background_css()`、builder `.background_color()`、`system` 主题校正，以及对应 Rust 单测 1+3 条与端到端断言 4 条；**保留**首屏外观注入（主题闪烁确已修好）与窗口位置持久化。
  - 回退依据：① 自动化只能验 HTML/DOM 与属性设置，无法证明 WebView2 实际首帧，此前将其写成「已修复」属过度结论；② 方案**未增加系统消耗**（+0.008 ms/次请求，无常驻线程 / 定时器 / 额外进程），所以「消耗大」不是回退理由，「无效 + 增加维护面」才是；③ 黑窗来自窗口位置持久化的 `visible(false)`，回退不会让它消失，故登记为已知问题。
  - 待办写入 `TODO.md` C-4：**启动 Logo，优先级极低，仅在日后出现启动过慢时才提高优先级以增加用户体验**。技术约束：HTML/CSS 里的 Logo 遮不住这段空窗（页面尚未绘制），必须原生层 splash（纯 GDI 小窗口为佳）；延后 `show()` 能遮住但会推迟窗口出现。
- **强制单实例：同一时刻只运行一个 GUI（2026-11-06）**
  - 用户反馈「每次双击 exe 都会启动一个新界面」。核查发现不止是视觉重复：每个进程都会 `db::open` 同一个 `library.db`，`build_router` 无条件 `spawn_boot_scan`（全库扫描 + 孤儿清理，写 DB）；两份 `cfg` 内存副本经 `/api/ui-prefs` 整段就地写 `[ui]` 互相覆盖；`kqk-window.state` 后退出者覆盖；日志交错、各占端口。
  - **不用 `tauri-plugin-single-instance`**：该插件在 `Builder::build()` 才判定，而本项目在 Builder 之前就已开 DB / 起 axum / 跑全库扫描，拦得太晚——第二个窗口虽不出现，DB 副作用已发生。
  - 新增 `app/gui/src/single_instance.rs`：`run()` 最开头（WebView2 预检之后、配置加载之前）用命名互斥体 `Local\KQK_SINGLE_INSTANCE_com.musicmt.app` 判定；第二个进程 `AllowSetForegroundWindow(ASFW_ANY)` → 读 `<exe目录>/kqk-instance.port` → 本机 `GET /api/focus` → 已有实例复用 `set_post_dialog_focus_sink` 把主窗口置前 → `exit(0)`，**此时尚未加载配置 / 开 DB / 起服务 / 触发扫描**。
  - 端口文件在服务确认可响应后以「临时文件 + rename」发布，正常退出由 `InstanceGuard::drop` 删除；崩溃残留由下次连接探测判定为陈旧，不影响正确性。互斥体 `bInitialOwner=FALSE` 且从不 wait，第一个进程崩溃后内核自动销毁对象，不会永久挡住后续启动；创建失败时保守放行且不碰端口文件（不能因锁故障导致起不来，也不能删掉别人的端口）。
  - `app/core/src/serve.rs` 新增 `GET /api/focus`（复用 `restore_focus_after_dialog()`；CLI `serve` 未注册 sink 时优雅空转）；`.gitignore` 增 `kqk-instance.port` / `.tmp`。
  - **无设置开关**（用户决定）：锁在进程启动时建立，运行期开关无法同步到已启动进程，只会引入状态不一致。只约束 `mt-gui.exe`；`mt-cli.exe serve` 不获取该锁，demo 用例不受影响。
  - 新增 Rust 单测 4 条（端口解析 / 请求文本 / 端口文件往返与清理 / 命名互斥体二次创建）、端到端用例 `demo/tests/test_single_instance.py`。真机目视见 `docs/07` P17。

#### 说明

- 本次叠加「图标 + 后端逻辑 + 前端界面」三类改动，按 minor 升级 **0.3.0 → 0.4.0**；
  上述 2026-11-06 补充为同一版本内的功能完善与 bug 修复，不另升版本号。

---

### v0.3.0 — 2026-11-04

#### 新增

- **歌词功能**
  - 新增 `app/core/src/lyrics.rs`：歌词来源优先级 = **同名 `.lrc` 边车文件** → **音频内嵌歌词**。
  - 内嵌解析：ID3v2 `USLT` 帧（自研解析：synchsafe 尺寸 / 扩展头 / 反同步 / 逐帧扫描 / 内容描述符分隔，
    编码字节 0=Latin1、1=UTF-16 BOM、2=UTF-16BE、3=UTF-8）+ Vorbis Comment `LYRICS`（FLAC / OGG）。
    > 原因：lofty 通用 `Tag` 转换会丢弃 ID3v2 `USLT`，且 `ItemKey` 无 Lyrics 变体，故 MP3 必须自研解析。
  - 文本编码自动识别：UTF-8 / UTF-16 BOM / UTF-8 / **GBK-GB18030**（新增依赖 `encoding_rs`）。
  - 导入联动：`import_audio_file` 在移动源音频前捕获边车文件，归位后同步落一份同名 `.lrc`；
    `import_ncm` 暂存 `.ncm` 时一并搬运。**自定义文件夹导入与「从本地网易云音乐曲库更新」两条路径均覆盖。**
  - 新增接口 `GET /api/lyrics/:id` → `{ok, source("lrc"|"embedded"), timed, text}`。
  - 前端：播放器新增「词」按钮 + 歌词浮层；`parseLrc()` 支持 `[mm:ss]` / `[mm:ss.xx]` / `[mm:ss.xxx]`、
    一行多时间戳、元信息行不入歌词轴、按时间排序；`syncLyrics()` 高亮当前行 + 居中滚动；点击行跳转。
- **应用元信息与「关于」面板**
  - 新增 `app/core/src/app.rs` 作为应用名称 / 版本 / 作者 / 联系方式 / 许可证的**唯一数据源**。
  - 新增接口 `GET /api/about`；`/api/status` 增加 `app_version` 字段。
  - 前端设置面板新增「关于」分区（名称 / 版本号 / 作者 / 邮箱 / 许可证 / 技术栈 / 简介）。
- **曲库名称**
  - `LibraryConfig` 新增 `name`（空白回退曲库目录名，限长 120 字符）；初始化向导 / CLI 可指定。
  - 新增接口 `POST /api/library-name`；顶栏展示曲库名称 + ✎ 切换曲库按钮。
- **配置双文件拆分**
  - 配置按「属于谁」拆成两份：**app 级** → `<exe 目录>/kqk-app.toml`；**曲库级** → `<曲库根>/.mt/config.toml`。
  - **唯一特例**：当前曲库路径 `last_library_dir` 放在 app 目录（应用得先知道当前曲库才能找到其配置）。
  - 运行期 `Config` 结构刻意保持不变，差异全部收敛在 `merge()` / `split()` / `save_app()` / `save_library()`，
    业务调用点零改动。
  - 旧 `musicmt-launcher.toml` + 旧曲库 `config.toml`（app/曲库混放）**自动迁移**。
- **配置文件版本兼容与保留策略**
  - 两份配置新增 `[meta]` 段：`config_version`（生成时的构建版本号）+ `min_compatible_version`（可正常运行的最低版本号）。
  - **版本兼容 → 既有配置及其内容完整保留**：只就地更新 `[meta]` 段，注释、手工添加的未知字段、键序全部原样保留；
    版本信息已是最新时**连文件都不触碰**（连时间戳都不动）。
  - **逻辑冲突（结构/字段不兼容）→ 才允许替换**：先备份原文件为 `*.conflict-<版本>.bak`，再重建，
    并**明确说明冲突原因**（stderr 日志 + 前端「设置 → 配置文件」分区 + `/api/status` 的 `config_conflict_reason`）。
  - 重建时仍**抢救路径类稳定字段**（当前曲库路径 / NCM 工具 / 导入源），无需重新指定曲库。
  - **构建/打包不再删除运行时配置**：`app/gui/build.rs` 只同步 `dist/frontend/`；
    `tools/package_gui.bat` 只拷贝两份 exe 与 README——`dist/` 下的 `kqk-app.toml`、`.guest/`、
    `library.db`、`app.log` 一律保留。
  - 修复：旧实现中 app 配置**解析失败会静默回退默认**（用户配置被清空）→ 改为备份 + 重建 + 说明原因。

- **导入进度上报（逐首明细）**
  - 新增 `app/core/src/progress.rs`：`ProgressTracker`（`Arc<Mutex<ImportProgress>>`，可克隆共享状态）
    + `ImportAction{Copy, Move, Convert, Stage, Skip}` / `EntryStatus{Imported, Skipped, Pending, Failed}`。
  - 新增接口 `GET /api/import-progress` → `{running, source, total, processed, current_file, current_action,
    imported, skipped, pending, failed, recent[], latest_seq, last_error}`。
  - 接入点：`import_folder_with_progress` / `import_netease_library_with_progress` /
    `import_ncm_with_progress` / `process_pending_ncm_with_progress`（原无进度版本保留为包装，零调用点改动）。
  - **计数口径刻意分离**：进度条与结果计数按**源文件**计，动作日志按**真实动作**计。
    1 个 .ncm 源文件产生「转换」+「入库」两条日志，但只结算为 1 个源文件。
  - **增量游标**：`ProgressEntry.seq` 单调递增。因日志「新的在前」且有长度上限，按下标取增量会漏/重，
    故统一用 `entries_after(after_seq)` 按 seq 消费（CLI 与 GUI 共用此语义）。
  - 前端新增右下角**进度面板**：进度条 + 当前动作 + 逐条动作日志 + 四项结果统计 + 失败原因。
    日志按 `e.seq > logSeq` 取增量，DOM 保留最近 60 条。
  - CLI 侧同样输出逐首明细（`mt-cli import` / `import-netease`）。
- **NCM 工具静默执行（解决转换时反复闪终端窗口）**
  - 新增 `mt_core::silent_command()`：Windows 下创建子进程时置 `CREATE_NO_WINDOW = 0x08000000`，
    进程正常运行但**不分配也不显示控制台窗口**，因此既不闪窗也不抢焦点。
  - `library.rs` 的 ncm 转换、`config.rs` 的 `attrib +h` 均改用该函数。
  - 根因：GUI 父进程（PE GUI 子系统）创建 Console 子进程（ncmdump.exe，PE Subsystem=3）时，
    Windows 会自动分配并显示控制台窗口。
  - **上游工具无静默开关**：ncmdump（taurusxin/ncmdump）仅有 `-h/-v/-d/-r/-o/-m`，
    `src/main.cpp` 的 `[Done]` 输出为无条件 `std::cout` —— 故只能在**调用侧**解决，
    无需「后台常驻控制台窗口」这类折衷方案。
  - 附带收益：改用 `Command::output()` 捕获 stdout/stderr，工具失败时把工具自身报错带入错误消息。
  - 验证：以「父进程 = GUI 子系统、子进程 = Console 子系统」对照实验，子进程内 `GetConsoleWindow()`，
    普通调用返回非空（有窗口，会闪）、`silent_command` 返回空（无窗口，不闪）。
- **导航新增「专辑」与「歌手」分组浏览入口**
  - 左侧导航栏在「全部曲库」「播放列表」之后新增 **专辑** / **歌手** 两个 Tab（并列的一级入口）。
  - 排版与交互**完全复用**播放列表的 `.playlists-pane` 双栏结构（`pl-sidebar` 分组列表 + `pl-main` 内容区），
    零新增 CSS——玻璃拟态 / 明暗主题（`body.skin-active` 选择器）自动继承。
  - **三级层级**（与播放列表一致）：一级 Tab → 二级分组列表（专辑名/歌手名 + 曲目数）
    → 点击某分组后三级展示该专辑/歌手的歌曲表。
  - 后端新增 4 条 REST：
    - `GET /api/albums` → `{albums:[{name,track_count}]}`（按 `album` 字段分组，NULL/空折叠为「未知专辑」）；
    - `GET /api/albums/tracks?name=` → `{tracks,count}`（成员查询与分组使用同一 `COALESCE(NULLIF(...))` 表达式，
      保证分组可点开；空名返回 `{"ok":false,"error":"专辑名称不能为空"}`）；
    - `GET /api/artists` / `GET /api/artists/tracks?name=` 同理（NULL/空折叠为「未知艺人」）。
  - 数据层 `app/core/src/db.rs` 新增 `GroupView` + `list_albums` / `list_artists` / `get_tracks_by_album` /
    `get_tracks_by_artist`；前端 `renderBrowseTracks()` 复用 `selectTrack` / `playTrack`，歌曲可直接选中/播放/看详情。
  - 注：歌手按**整字段**分组（不做 `A / B` 多艺人拆分），与「展示对应的歌手列表」字面一致、实现最简。

#### 变更

- **顶栏去重与图标重做**
  - 移除顶栏品牌名旁的版本徽标（`v0.2.0`）——版本号已由窗口标题栏显示，顶栏不再重复。
  - 「切换曲库」铅笔按钮改为**透明背景 + 内联 SVG 线稿图标**。
    > 原因：原按钮只带 `icon-btn` 类，而样式表仅有 `.md-btn.icon-btn` 规则，导致它落入浏览器默认按钮样式（白底黑字）。
  - 顶栏曲库名称补充可点击样式（鼠标指针 + 悬停下划线），与铅笔按钮构成同一入口。
- **「从本地网易云音乐曲库更新」自动指定**
  - 未设置本地网易云曲库目录时，点击该按钮**直接弹出目录选择框**；选好目录后自动保存并**立即执行更新**，
    无需再去设置面板手动指定。
  - 设置面板的「更新」按钮同步支持：输入框已填路径则先保存再更新。
  - 抽出公共函数 `saveNeteaseDir()`，保存 / 浏览 / 更新三处共用同一逻辑。
- **作者名修正**：`kill3` → **`kill33`**（邮箱 `kill33@126.com` 不变）。
- **打包脚本 `tools/package_gui.bat` 修复**：原先为 LF 行尾 + 含中文 + 含反引号，cmd.exe 解析错乱，
  把含中文的行拆成非法命令并报错（如「xxx 不是内部或外部命令」），实际无法正常打包。
  - 根因三则：① LF-only 行尾；② 文件含非 ASCII —— 本机控制台代码页为 936(GBK)，
    cmd.exe 按 GBK 解码 UTF-8 字节会**丢失字节对齐**（GBK 版在 CP936 下可解析，但换到 CP65001 又会坏，
    纯 ASCII 才是唯一到处都安全的编码）；③ `if errorlevel 1 (...)` 块内出现**未加引号的括号 `(cargo)`**，
    提前闭合了代码块。
  - 处置：脚本改为 **CRLF + 纯 ASCII**，块内括号加引号，文件头部写明「编写规范」踩坑记录。
- 配置容错：所有配置子结构改为 `#[serde(default)]` + 带默认值的 `Default` 实现，
  **残缺配置不再整份作废**，能解析出的字段全部保留。
- **歌手页布局：详情（封面 + 文字）内联到歌曲列表上方**
  - 原布局：`.layout` 为 flex 行 —— `sidebar 144px + 歌手列表 230px + 歌曲列表(flex:1) + 右侧详情栏 300px`。
    在 1080px 窗口下歌曲列表只剩 **314px**，「艺人」列可用宽 47px（实际需 98px）→ 文字被省略号截断（显示成 "Alan Wal..."）；
    在 900px 最小窗口下更只剩 134px / 2px。
  - 改法：歌手页隐藏独立右侧详情栏（`#detail-pane[hidden]`），把「封面 + 文字」改为 `.pl-main` 内、歌曲列表**上方**的横向条带
    （`.ar-detail`：80px 封面 + 字段横向换行排列）。详情内容与右侧详情栏**同源共用**（`fillTrackDetail()` 同时写入两处），
    **视觉风格与功能不变**，仅调整排列顺序与空间分配。
  - 效果：歌曲列表宽度 **+312px**（300px 详情栏 + 12px 间距让出）。1080px 窗口下 314px → **626px**，
    「艺人」列 47px → **127px**，文字完整显示。
  - 窄窗兜底：`#ar-content .pl-table { min-width: 600px }`，并给 `#artists-pane` 与 `.pl-main` 补 `min-width: 0`
    （两处缺一不可，否则内容固有宽度会顶穿父级、造成页面级横向滚动）——
    900px 最小窗口下表格在列表容器**内部**横向滚动，文字不被截断，页面不横向滚动。
  - 备份：`.workbuddy/backups/2026-11-05-detail-inline-layout/`（index.html / styles.css / app.js + MANIFEST.md5）。
  - 验证：无头 Edge 几何断言 **12/12 PASS**（1080×645 / 900×600 / 1280×800 三尺寸）；另断言「全部曲库 / 播放列表 / 专辑」
    三个 Tab 的右侧详情栏显示**无回归**。
  - **待确认**：按要求本轮**仅改歌手页**；专辑 / 播放列表 / 全部曲库是否同样改为内联布局，待确认后再统一应用。
- **「设置 → 列表」新增「布局风格」选项（横向 / 纵向），四个页面各自独立**
  - 需求落地：原布局 = **横向**（曲目详情在窗口右侧独立详情栏），新布局 = **纵向**（详情内联到歌曲列表上方，
    把 300px 宽度让给列表）；**主界面（全部曲库）默认横向，播放列表 / 专辑 / 歌手默认纵向**。
  - 配置：`UiConfig` 新增 `layout_library` / `layout_playlists` / `layout_albums` / `layout_artists` 四个 `u8` 字段
    （全部 `#[serde(default)]`，旧配置直接回退默认值）；新增常量 `LAYOUT_STYLE_HORIZONTAL=1` / `LAYOUT_STYLE_VERTICAL=2` / `VIEW_KEYS`。
  - 接口：新增 `GET/POST /api/ui-prefs`（部分更新，返回 `layout_styles` + `column_widths`）；`/api/status` 同步暴露这两个字段。
  - 落盘：走 `config::save_ui_in_place()` —— **只就地替换 `[ui]` 段内的已知键**，段内注释、未知键、既有键序原样保留
    （与既有 `[meta]` 就地更新策略一致）；缺失的键追加到段尾，多行数组的续行不会被误当作键。
  - 前端：`switchTab()` 改为按当前页面的布局风格决定「右侧详情栏」还是「列表上方内联详情条」；
    新增通用类 `.inline-detail` / `.inline-cover` / `.inline-fields`（`.ar-detail*` 保留为兼容别名），
    四个页面各有一份内联详情条；详情内容与右侧详情栏**同源共用**（`renderViewDetail()` / `renderActiveDetail()`
    由同一个 `viewDetail(view)` 派生，**按页面隔离**，互不串数据）。
- **歌曲列表列宽可拖拽调整，比例持久化到配置文件**
  - 需求落地：拖动歌曲列表**表头**的列分隔线即可调整列宽，解决曲名 / 艺人 / 专辑名称显示不全；调整后的**比例**写入配置文件。
  - 存储形态：存**百分比占比**（6 个业务列 `name`/`artist`/`album`/`quality_tier`/`duration_ms`/`file_size`，之和恒为 100），
    而非像素值 —— 像素值在窗口尺寸变化后即失效，占比可随容器自适应。
  - 配置：`UiConfig` 新增 6 个 `width_*` 字段（`#[serde(default)]`）；新增 `MIN_COLUMN_WIDTH_PCT = 4.0`；
    `UiConfig::sanitize()` 统一归一化（单列 ≥ 4%、总和归一到 100），运行期 `Config::merge()` 也会调用。
  - 渲染：`table-layout: fixed` 下按占比把像素宽写到表头 `th`；**用「累计取整」分配**（而非逐列取整），
    保证各列宽度之和**恰好**等于可用总宽 —— 逐列舍入会累积误差使总宽超出几个像素，浏览器随即重新分配并撑宽表格。
  - 拖拽：**按像素**计算（与直觉一致：分隔线移动了多少像素），该列变宽、右邻列等量变窄，总宽不变；
    列宽像素下限 `MIN_COL_PX = 48`（单元格左右内边距合计 24px + 留文字宽度）——
    border-box 下 padding 不可压缩，窄于此值浏览器会强制撑宽该列导致表格溢出。
  - 写回：松手时按实际渲染宽度归一化，再**分摊回这批列在存储空间里的份额**（未显示的列份额不变），
    故六列之和恒为 100；前端 `normalizeStoredWidths()` 与后端 `sanitize()` 规则一致，内存与文件不产生偏差。
  - 前端：三个表格（全部曲库 / 播放列表 / 专辑·歌手）统一支持；设置面板「列表」分区新增「列宽比例」列表（占比条 + 百分比）
    与「恢复默认列宽」按钮；窗口缩放 / 布局切换后按占比重新分配。
  - 备份：`.workbuddy/backups/2026-11-05-layout-style-colwidth/`（index.html / styles.css / app.js / config.rs / serve.rs + MANIFEST.md5）。

#### 修复

- **切换到「专辑」/「歌手」列表后，详情区仍显示「全部曲库」页所选歌曲的数据**
  - 现象：在「全部曲库」选中一首歌 → 切到「专辑」或「歌手」页（尚未选中任何分组 / 歌曲），
    列表上方的详情条与右侧详情栏仍显示**曲库页那首歌**的封面与曲目信息。
  - 根因（四条叠加）：
    1. `fillInlineDetails()` 把**同一份曲目详情写进全部 4 个页面的内联详情条**（`lib-detail` / `pl-detail` /
       `al-detail` / `ar-detail`）—— 专辑 / 歌手页在曲库页选中歌曲的那一刻就已被污染；
    2. `applyLayoutStyle()` 切页时**只切换 `hidden`、从不重算内容**，旧数据原样暴露；
    3. `state.current` 是**全局单例**选中态，四个页面共用，且 `switchTab()` 不重置任何状态；
    4. `selectTrack()` 无条件写全局右侧详情栏，且跨表清除高亮；`selectAlbum()` / `selectArtist()`
       **从不写详情面板**，选中分组后既无「专辑 / 歌手信息」，旧的曲级选中也不清理。
  - 修复：**详情状态改为分视图隔离**。
    - 新增 `state.viewSelection = { library, playlists, albums, artists }`，各页面的曲级选中互不干扰
      （`{ track, trx }` 或 `null`）；`state.current` 保留为「全局最近选中曲目」，仅供歌词面板等动作上下文使用。
    - 新增 `viewDetail(view)`：统一计算某页当前应展示的内容 —— 曲级选中 → 曲目详情；
      分组选中（专辑 / 歌手 / 播放列表）→ 分组信息（名称 + 类型 + 曲目数 + 该组首曲封面）；都没有 → 空。
    - `renderViewDetail(view)` 只写**该页自己的**内联详情条；`renderActiveDetail()` 写全局右侧详情栏
      （四个页面共用同一 DOM，必须按当前页重算）。
    - `switchTab()` 增加「按新页重算详情 + 同步表格高亮 + 重算该页内联条」三步；
      `selectAlbum()` / `selectArtist()` / `selectPlaylist()` 切换分组时**清空该页曲级选中**并重算详情。
    - 新增 `fillGroupDetail()`：分组详情渲染（专辑名 / 歌手名 / 列表名 + 类型 + 曲目数 + 代表封面），
      封面取该分组首曲的 `/api/cover/:id`（专辑封面即专辑内嵌封面）。
    - `selectTrack()` 改为**按点击时所在页面**写入选中态，并加**并发令牌**（`detailSeq`）：
      连续快速点击只认最后一次响应，避免旧响应覆盖新选中。
    - 专辑 / 歌手 / 播放列表的曲目列表加载各加**请求序号守卫**（`albumReq` / `artistReq` / `plReq`），
      快速切换分组时旧响应不再覆盖新列表。
    - 「＋ 加入选中歌曲」改为读取**「全部曲库」页**的选中曲（与提示语一致，不再受其它页面残留影响）。
    - 各页占位文案按页面定制（如「选择左侧专辑查看专辑信息，或点击歌曲查看详情」）。
  - **后端零改动**：`GroupView` 已有 `name` / `track_count`，`/api/albums` `/api/artists` 无需扩展；
    封面来源复用已有的分组曲目列表首曲，无需新增接口。
  - 验证（三层）：
    1. **无头 Edge 断言**（mock API）**81 条/尺寸 × 3 尺寸 = 243 条全通过**（含用户指定的完整链路：
       曲库选中歌曲 → 切专辑页(无选中) → 切歌手页(无选中) → 选中专辑 → 选中歌曲 → 切回曲库页）。
    2. **真实端到端**（真实 `mt-cli serve` + 真实 SQLite + 真实带标签 WAV + 真实 `dist/frontend` 前端，
       无任何 mock）**36/36 通过**；并用**修复前的**前端跑同一套真实 E2E → **36 条中失败 17 条**，
       失败项正是用户报告的现象（专辑 / 歌手页未选中时显示曲库页曲名、选中专辑后无专辑信息、切回曲库页无法恢复）。
    3. 后端零改动：`cargo test -p mt-core` **54 passed**；界面偏好 E2E 30/30、真实配置 E2E 20/20 全通过。
  - 备份：`.workbuddy/backups/2026-11-05-fix-stale-detail/`（index.html / styles.css / app.js / serve.rs / db.rs + MANIFEST.md5）。

- **播放列表 / 专辑 / 歌手 三页交互不一致，且未选中时渲染「无封面」「暂无信息」等占位内容**
  - 现象：三页详情区在数据未选中时仍显示占位条与「无封面」文字；空字段显示为「—」；
    三页各写一套渲染代码，行为容易漂移。
  - 根因：详情渲染散落在 `fillTrackDetail()` / `fillGroupDetail()` 两处，逻辑重复且都以「写占位文案」兜底；
    封面控件与字段区是 `display:flex`，`hidden` 属性对其无效；分组列表渲染在三处重复实现。
  - 修复：**抽出可复用的公共组件**（`app/gui/frontend/app.js`）：
    - **详情区公共组件 `DetailRegion`** —— 统一契约：
      1) 数据未选中 → **整个详情区隐藏**，不渲染任何占位内容；
      2) 封面缺失 **或封面图加载失败** → **封面控件隐藏**，不渲染「无封面」字样；
      3) 名称为空 → 不渲染名称行；
      4) 字段值为空 → **该字段行不渲染**，不渲染「—」占位。
      对外 API：`detailTargets()` / `detailValue()` / `detailRowsHtml()` / `renderDetailCover()` /
      `renderDetailRegion()` / `hideDetailRegion()` / `detailModelOf()`。
    - **显隐仲裁唯一入口 `syncDetailVisibility()`** —— 横向（1）显示全局右侧详情栏、纵向（2）显示当前页内联条；
      无论哪种布局，**该页没有可展示内容时对应详情区整体隐藏**（右侧栏也不留空条）。
      `applyLayoutStyle()` 改为委托给它，布局切换与内容渲染不再各管一套。
    - **分组列表公共渲染 `renderGroupList()`** —— 播放列表 / 专辑 / 歌手三页共用
      （`keyField` 参数化唯一键：播放列表用数字 id，专辑 / 歌手用名称）；
      名称为空不渲染名称控件、曲目数为空不渲染数量控件；空态统一为「暂无X」。
    - `index.html`：5 处封面控件与 5 处字段区改为**初始隐藏且无占位文本**（不再写死「无封面」）。
    - `styles.css`：新增 `[hidden]` 生效规则（`.inline-cover` / `.inline-fields` / `.cover` / `.detail-fields`
      均为 `display:flex`，须显式 `display:none` 才真正消失）。
  - **测试代码统一迁移到 `demo/` 目录**（需求：便于集中运行与查看）：
    - `demo/common.py`：公共模块 —— 路径常量、夹具接入、断言收集器、无头 Edge 驱动、真实后端启停；
    - `demo/run_all.py`：一键运行全部用例并汇总（`--list` / `--only <name>`）；
    - `demo/tests/`：`test_detail_region.py`（详情区公共组件 + 跨页隔离）、`test_ui_prefs.py`、
      `test_real_config.py`；
    - `demo/tools/`：`gen_preview.py`（mock 预览）、`run_preview.py`（前端无头断言）、
      `run_old_regression.py`（回归对照）、`shoot.py` / `shoot_fix.py`（截图）、`diag*.py`（诊断）；
    - `demo/fixtures/`：**持久化测试夹具**（3 首带真实 RIFF INFO 标签的 WAV + 已初始化并已入库的曲库）。
      用 `mt-cli library` 真正初始化 + `mt-cli scan` 真正入库，**不再每次重新生成**。
      ⚠️ DB 存绝对路径 → 曲库必须**原地复用**（复制会导致路径失效并被增量扫描判为孤儿清理）。
  - 验证：
    1. **真实端到端**（真实 `mt-cli serve` + 真实 SQLite + 真实夹具音频 + 真实 `dist/frontend` 前端，零 mock）
       **67/67 通过**；
    2. **前端无头断言**（mock API，3 窗口尺寸）**93 条/尺寸 × 3 = 279 条全通过**；
    3. **回归对照**：同一套真实 E2E 跑**修复前**的前端 → **67 条中失败 28 条**，
       失败项正是「未选中仍显示占位 / 无封面」的症状；
    4. `cargo test -p mt-core` **54 passed**；`demo/run_all.py` **5/5 用例全通过**；
    5. release 构建 + `tools/package_gui.bat` 打包成功，`dist/frontend` 三文件 md5 与源码一致，
       `dist/kqk-app.toml` 未被覆盖。
  - 备份：`.workbuddy/backups/2026-11-05-demo-tests-detail-region/`（app.js / index.html / styles.css + MANIFEST.md5）。

- **导入云音乐曲库（及任意原生文件夹/文件对话框）后主窗口无响应、点击歌曲无法播放**
  - 根因：前端「从本地网易云曲库更新」在未设置曲库目录时先弹原生文件夹选择框（`rfd::FileDialog`，Windows 原生模态对话框）；该对话框关闭后 WebView **常未能自动重获焦点**，主窗口表现为「无响应 / 点击不生效 / 交互紊乱」，进而点歌曲列表无法触发播放。右下角的进度面板（`#progress-panel`，`z-index:60`、`pointer-events` 正常）只是恰好在对话框之后出现，被误认为「抢焦点」的元凶——它自身不会吞掉主窗口的原生焦点。
  - 修复：沿用既有「外观同步」钩子模式，在 `serve.rs` 新增 `set_post_dialog_focus_sink(fn())` + `restore_focus_after_dialog()`；三个原生对话框处理器（`pick_folder` / `pick_library_directory` / `pick_ncm_tool`）在 `rfd` 返回后调用之；`mt-gui` 的 `native_titlebar` 模块新增 `refocus()`，用 Win32 `SetForegroundWindow` + `SetFocus` 把主窗口带回前台并夺回键盘焦点，并在 `setup` 注册。`CLI serve` 未注册则静默空转。
  - 验证：`cargo build --release -p mt-core -p mt-gui` EXIT=0；`cargo test -p mt-core` 46 passed。`SetForegroundWindow` 在对话框关闭的同源时序内调用（用户刚交互过，进程持前台权限），应能可靠夺回焦点；原生窗口行为需真机目视确认（与标题栏染色同属 Windows 原生相关，见 `docs/07` P9）。

#### 性能 / 体积

- 新增依赖 `encoding_rs`（GBK/GB18030 歌词解码），二进制体积 +约 199 KB。

#### 测试

- `cargo test -p mt-core` 单测由 22 个增至 **54 个**（新增：版本比较 2 个、兼容判定 5 个、
  `[meta]` 就地更新 2 个、顶层键就地写入 1 个、残缺配置容错 1 个、`[meta]` 往返 1 个、
  冲突抢救 1 个、**导入进度 7 个**、**专辑/歌手分组查询 3 个**、**界面偏好 8 个**（默认布局/默认列宽、
  非法布局值忽略、越界归一化、旧配置缺字段仍可解析、`[ui]` 段就地更新保留注释与未知键、
  多行数组不被写坏、落盘往返）；原 22 个全部保持通过）。
- **修复单测偶发失败（此前约 40% 概率失败）**：
  - 现象：`downloads::tests::enqueue_and_take_next` / `pause_one_queued_becomes_paused`
    随机报 `assertion left == right failed: left: 1, right: 2`。
  - 根因：`DownloadManager::new` 把队列持久化到 `config_path.parent()/download_queue.json`；
    测试传入**相对路径** `"nonexistent_config.toml"`，其 `parent()` 为空 → 队列文件落到**当前工作目录**，
    所有用例**共享同一个 `download_queue.json`**。Rust 单测默认并行执行，一个用例入队的 `song_id`
    被另一个用例读到 → 去重逻辑误判为重复，`enqueue` 返回 1 而非 2。
  - 修复：新增测试辅助结构 `TestMgr`，为每个用例在 `std::env::temp_dir()` 下分配**唯一目录**
    （用例名 + 进程号），队列文件随之隔离；`Drop` 自动清理。
  - 附带收益：仓库里不再残留测试产生的 `app/core/download_queue.json` 垃圾文件。
  - 验证：连续 **20/20** 次 `cargo test -p mt-core` 全部 43 passed；临时目录零残留。

---

### v0.2.0 — 2026-11-04

#### 新增

- 左侧 **Tab 选择栏**（全部曲库 / 播放列表）与歌曲列表**序号列**。
- **歌曲列表显示列自定义**：可勾选可见列并调整顺序（持久化到 app 级配置）。
- 刷新按钮旁增加**列表歌曲统计**。
- **外观系统**（纯前端实现，偏好存 `localStorage`）：明暗主题切换、主题色预设/自定义、
  换肤（选本地背景图自动提取主色 + 玻璃拟态面板）。
- 散落 `.ncm` 处理弹窗（扫描发现曲库内有散落 `.ncm` 时询问是否移入 `download/` 处理）。
- 应用改名：**musicMT → K music**（产品名 / 窗口标题 / 前端文案；bundle `identifier` 保留以延续已装实例）。

#### 变更

- **产物瘦身**：`Cargo.toml` 增加 `[profile.release]`（`opt-level="z"` / `lto="thin"` / `codegen-units=1` /
  `panic="abort"` / `strip=true`）；删除 8 个死依赖（`ureq` / `aes` / `cbc` / `num-bigint` / `hex` /
  `base64` / `rand` / `percent-encoding`，随 2026-10-29 移除的网易云服务端功能一起失去引用），
  `tokio` 由 `full` 改为显式最小 feature 集。
  > 实测：mt-gui.exe 13.9 MB → 5.7 MB，mt-cli.exe 6.1 MB → 3.2 MB；依赖树 527 → 约 450 crate。
- `app/gui/build.rs` 在 `cargo build` 编译期**自动**把前端同步到 `dist/frontend`，
  不再需要手动执行 `package_gui.bat` 同步前端。

---

### v0.1.0 — 初始版本

- 曲库扫描建索引（采样率 / 位深 / 码率 / 声道，增量门控 `(size, mtime)` + 孤儿清理）。
- 外部曲库导入（复制 / 剪切，按 `<艺人>/<专辑>/<曲名>` 归位，四种文件结构方案）。
- NCM 外部转换工具接口（只调用用户自有工具，项目**不内置也不实现**任何解密/转码）。
- 本地 HTTP 播放服务（axum + 手写 Range/206 流式响应 + 封面接口）。
- SQLite 索引（rusqlite bundled）、日志落盘、CLI（`scan` / `import` / `serve` / `import-netease` / `ncm-organize`）。
- Tauri 2 GUI 壳（Material 风原生 HTML/JS 前端，由 axum 从磁盘提供，不嵌入二进制）。
