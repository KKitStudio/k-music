# K曲库（musicMT）— 项目共享事实 / 多会话入口

> 本文件由 IDE 在**每轮、每个会话**自动注入上下文，是同一项目目录下多个会话的**共享信息入口**。
> 权威细节：`.workbuddy/memory/MEMORY.md`（长期约定与防回归约束）、`VERSION.md`（版本变更全表）、
> `docs/`（设计文档）、`.workbuddy/memory/YYYY-MM-DD.md`（当日工作日志，append-only）。
> **任何会话改变了下列事实，必须同步：本文件 + MEMORY.md + 当日日志 + 相关 docs/（涉及版本再加 VERSION.md）。**
> 最近更新：2026-11-06 10:24（初始化 git 仓库后补齐多会话共享信息）；10:32 增补「旧版覆盖声明」。
>
> ⚠️ **旧版覆盖声明**：若你的上下文里还残留本文件的早期版本（描述为「无 git 仓库」「网易云备份下载工具」
> 「TypeScript 严格模式」「musiclibrary.exe」「npm run dev」等），那些内容**已作废**；一律以当前这份为准，
> 且**不要据旧内容写回文档或提交**。已开着的会话如需同步，只需重新读一次本文件（只读），无需再发起文档更新。

## 1. 多会话协作规则（务必遵守）
1. **开工先看** `git status --short`。若存在其它会话的未提交改动，不要覆盖、不要顺手提交。
2. **提交只带自己的文件**：`git add <具体文件>`，**禁止 `git add -A`** 把其它会话的中间态带进提交。
3. **日志 append-only**：`.workbuddy/memory/YYYY-MM-DD.md` 只追加、不覆盖、不删改历史；`MEMORY.md` 就地编辑并保持精简。
4. **改动流程**（本会话用于持续报 bug / 改 bug）：复现定位根因 → 备份到 `.workbuddy/backups/YYYY-MM-DD-<主题>/`（含 MANIFEST.md5）→ 最小范围改动 → 验证 → 追加日志 + 同步文档 → 提交。已有 git 可用 `diff`/`revert`，**备份仍保留为第二道保险**。
5. **验证基线**：`cargo test -p mt-core`（当前 58 passed）、`python demo/run_all.py`（7 用例）、`node --check app/gui/frontend/app.js`、前端源与 `dist/frontend` 三文件 md5 对齐。
6. **提交信息**用 Conventional Commits（`feat:` / `fix:` / `docs:` / `chore:` …）。

## 2. 版本控制现状（2026-11-06 起）
- 本地 git 仓库已初始化：分支 `main`；全局与仓库级身份均 `kill33 <kill33@126.com>`；**未配置 remote、未 push**。
- `.gitignore` 排除：`target/`、`dist/`、`app/data/`、运行时配置/DB/日志（`kqk-app.toml`、`.guest/`、`.mt/`）、`tools/ncm/{bin,source}`、`demo/.run/`、可重建夹具、`.workbuddy/{backups,tmp}`；**`.workbuddy/memory/` 入库**（多会话共享）。
- `.gitattributes` 为 `* -text`，禁用行尾自动转换（`app/gui/frontend` 与 `dist/frontend` 需字节级一致；`tools/package_gui.bat` 必须保持 CRLF）；仓库级 `core.autocrlf=false`。
- 合规：NCM 工具源码与二进制**不入库、不随包分发**，仅保留 `tools/ncm/README.md` 溯源；项目不内置解密。公开代码前须复核 `TODO.md` A2。

## 3. 项目定位（已冻结）
- **K曲库**：PC 端本地曲库管理 + 桌面播放器。`mt-core` 纯 Rust 库 + 可替换 UI 壳；Electron 排除。
- 网易云**服务端登录/下载已于 2026-10-29 彻底移除**（规避封号）：备份路径 = 用户在官方 APP 维护 + 本地「从本地网易云音乐曲库更新」增量合并（copy、不联网、不登录）。
- NCM：不内置解密，仅通过可配置路径调用外部 `ncmdump`；唯一开关是工具路径是否存在。

## 4. 技术栈与目录（纠正旧描述）
- Rust workspace：`app/core`(mt-core) / `app/cli`(mt-cli) / `app/gui`(mt-gui, Tauri 2 + 系统 WebView2)；SQLite(rusqlite bundled)、axum 本地 HTTP 服务（手写 Range/206）、lofty 元数据。
- 前端为**原生 HTML/JS**（`app/gui/frontend/{index.html,app.js,styles.css}`），**无 npm / TypeScript / 前端构建**；GUI 经 WebView2 加载本地 axum 服务，前端不嵌入 exe。
- **`dist/frontend/` 是 GUI 实际前端源**；`app/gui/build.rs` 编译期从 `app/gui/frontend` 同步 → 改前端后必须核对两者 md5。
- 配置双文件：app 级 `<exe目录>/kqk-app.toml` + 曲库级 `<曲库>/.mt/config.toml`，运行期由 `Config::merge/split` 转换。
- `app/data/`、`dist/`、`.workbuddy/{backups,tmp}`、`demo/.run/` 均为本机运行/临时数据，不入库。
- 应用元信息唯一数据源 `app/core/src/app.rs`；版本号唯一数据源 = 根 `Cargo.toml [workspace.package] version`（当前 **0.4.0**）。

## 5. 常用命令
```bash
export PATH="/c/Users/kill3/.cargo/bin:$PATH"
cargo test -p mt-core
cargo build --release -p mt-gui -p mt-cli       # target/release/mt-{gui,cli}.exe → 拷入 dist/
python demo/fixtures/make_fixtures.py --status  # 夹具必须原地复用（DB 存绝对路径）
python demo/run_all.py                          # 7 个端到端用例
mt-cli.exe --version | library <目录> | scan | serve | import | import-netease | ncm-organize
```
> 发布用 `tools/package_gui.bat`；保留 `dist/kqk-app.toml`、`.guest/` 运行时配置。**改图标后必须重编 `mt-gui`**（图标编译期嵌入 exe）。

## 6. 关键防回归约束（详见 `.workbuddy/memory/MEMORY.md`）
- 详情渲染**分视图隔离**（`state.viewSelection` + `viewDetail(view)`）；禁止"把同一份数据灌进所有页面"的函数。
- 详情区公共组件 `DetailRegion`：未选中 → 整区隐藏；封面缺失/加载失败 → 隐藏；空名称/空字段 → 不渲染对应行（不显示「—」）；显隐唯一入口 `syncDetailVisibility()`。
- 配置写入必须**就地更新**（`save_ui_in_place()`），不可用 `save_app()` 整份重写（会抹掉用户注释与未知键）。
- `default_track_columns()` 只有 7 列（**创建时间默认不显示**，由「设置 → 列表」勾选开启）；列宽拖拽只按像素增量改被拖动的两列占比。
- 改版本号须同步 `Cargo.toml` + `VERSION.md` + `app/gui/tauri.conf.json` 三处。

## 7. 当前进度 / 待办
- v0.4.0 已发布：扫描与增量索引、导入、NCM 外挂、播放与歌词、专辑/歌手分组、播放列表、外观三态主题与换肤、原生标题栏染色、布局风格与列宽、导入/创建时间列、WebView2 启动预检。
- **待真机验证**：歌词浮层、rfd 原生对话框焦点、backdrop-filter 观感、Windows 原生标题栏染色、1280 宽 + 8 列时时间单元格省略号（见 `docs/07` P9/P10/P14）。
- 发布流水线与 GitHub 开源待办见 `TODO.md`。
