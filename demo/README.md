# demo —— K曲库（musicMT）测试代码集中区

本目录集中存放**全部**测试与验证代码，便于统一运行与查看。

```
demo/
├── README.md              ← 本文件
├── common.py              ← 公共模块：路径常量、夹具接入、断言收集、无头 Edge 驱动
├── run_all.py             ← 一键运行所有用例并汇总
├── fixtures/              ← 持久化测试夹具（不在临时目录，清理不受影响）
│   ├── make_fixtures.py   ←   构建/校验夹具曲库（幂等，--force 重建）
│   ├── library/           ←   已初始化并已入库的测试曲库（3 首带真实标签的 WAV）
│   └── manifest.json      ←   夹具清单
├── tests/                 ← 测试用例
│   ├── test_detail_region.py   详情区公共组件（三页一致的隐藏契约 + 跨页详情隔离）
│   ├── test_ui_prefs.py        后端 /api/ui-prefs 读写 + [ui] 段保留式就地更新
│   ├── test_appearance_config.py  外观主题方案落盘 + 重启读回 + 非法值回退
│   ├── test_boot_appearance.py    入口 HTML 首屏注入外观 + 兜底底色（防主题闪烁 / 黑屏）
│   ├── test_time_columns.py    导入/创建时间字段 + 创建时间默认隐藏/设置面板开关（真实 E2E）
│   └── test_real_config.py     用用户真实 dist/kqk-app.toml 验证就地更新不破坏内容
├── tools/                 ← 辅助工具
│   ├── gen_preview.py         生成前端 mock 预览页（注入假 fetch）
│   ├── run_preview.py         无头 Edge 跑前端断言（3 个窗口尺寸）
│   ├── run_old_regression.py  回归对照：用「修复前」的备份跑同一套断言
│   ├── shoot.py / shoot_fix.py  截图（mock 页面）
│   ├── shoot_time_columns.py   截图（真实 serve）：导入时间 / 创建时间默认隐藏 / 设置开关 三态
│   ├── clean_run.py            清理运行产物（demo/.run/）
│   └── diag*.py                几何 / 列宽 / 拖拽诊断
└── .run/                  ← 运行产物（临时，可随时删除）
    ├── preview/             ← gen_preview.py 生成的 mock 预览页（复用，不删）
    ├── <name>-<时间戳>/      ← 每次运行的独立工作目录（exe + 前端副本）
    └── dom-<时间戳>/         ← run_preview.py 的 DOM dump
```

> 运行目录一律用 `common.run_dir()` 生成**带时间戳的独立目录**（`C.run_dir(name)`），
> 绝不复用同一路径——Windows 下 exe 被占用时无法删除旧目录，复用会互相干扰。
> `clean_run.py` 清理失败时会**如实报告**（通常是 exe 仍被占用），不会静默跳过。

## 快速开始

```bash
# 列出所有用例
python demo/run_all.py --list

# 跑全部用例
python demo/run_all.py

# 只跑某一个
python demo/run_all.py --only detail_region

# 清理运行产物（每次运行会留一份 exe + 前端副本，约 7MB）
python demo/tools/clean_run.py            # 全清（会先结束残留服务进程）
python demo/tools/clean_run.py --keep 2   # 只留最近 2 次

# 单独运行某个脚本
python demo/tests/test_detail_region.py
python demo/tools/run_preview.py
```

## 服务进程管理

- 每个用例启动的 `mt-cli.exe serve` 都由 `common.start_server()` 拉起，
  解析端口阶段若出错会**立即回收进程**，不会留下孤儿。
- 用例结束时通过 `common.stop_server()` 统一停止（`terminate` → `kill` 兜底）。
- 若历史遗留导致进程仍在运行（例如脚本中途崩溃），`clean_run.py` 会先调用
  `common.kill_stale_servers()` **按可执行文件路径限定在 `demo/.run/` 下**结束它们，
  不会误杀用户自己启动的 mt-cli。

## 夹具（重要）

**测试曲库是持久化的**，位于 `demo/fixtures/library/`，包含 3 首带真实 RIFF INFO 标签的 WAV：

| 艺人 | 专辑 | 曲名 |
|---|---|---|
| Alan Walker | Different World | Faded |
| Alan Walker | Different World | Yellow Cold Rose |
| Iselin Solheim | Ignite | Spectre |

- 曲库用 `mt-cli library` **真正初始化** + `mt-cli scan` **真正入库**，不是手工拼出来的假数据；
  因此能真实覆盖 lofty 标签解析、专辑/歌手分组、增量扫描等链路。
- 夹具**不在临时目录**里 → 清理 `demo/.run/` 或任何临时产物都不会影响它，不必每次重新生成。
- ⚠️ **必须原地复用，不可复制到别处**：DB 里 `file_path` 存的是绝对路径，
  复制后路径失效会被增量扫描判为孤儿并清理。所有测试脚本都直接指向 `C.FIXTURE_LIB`。

重建夹具：

```bash
python demo/fixtures/make_fixtures.py            # 幂等：缺失/损坏时自动构建
python demo/fixtures/make_fixtures.py --force    # 强制重建（项目移动过路径后需要）
python demo/fixtures/make_fixtures.py --status   # 只看状态
```

## 前置条件

| 用途 | 要求 |
|---|---|
| 后端端到端用例 | `cargo build --release`（或 debug）产出 `mt-cli.exe` |
| 前端无头断言 | 同上 + `dist/frontend` 存在（`tools/package_gui.bat` 打包） |
| 无头浏览器 | Windows 版 Edge（`C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe`） |

## 用例说明

### `tests/test_detail_region.py` —— 详情区公共组件

覆盖两轮需求：

- **A 跨页详情隔离**：曲库页选中的歌曲**不得**出现在专辑 / 歌手页的详情区
  （旧实现把同一份详情写进全部 4 个页面内联条，切页时只切 `hidden` 从不重算内容）。
- **B 三页一致的隐藏契约**（公共组件 `DetailRegion`）：
  - 数据未选中 → 详情区整体隐藏（不渲染「选中歌曲以查看详情」之类占位）；
  - 封面缺失 / 加载失败 → 封面控件隐藏（不渲染「无封面」字样）；
  - 名称为空 → 不渲染名称行；
  - 字段值为空 → 该字段行不渲染（不渲染「—」占位）；
  - 播放列表 / 专辑 / 歌手三页在同一状态下显隐结论**完全一致**。

流程：真实 `mt-cli serve`（真实 axum + 真实 SQLite）→ 无头 Edge 打开**真实前端**
（只额外注入断言脚本，`app.js` / `styles.css` 一字不改）→ 逐项断言。

### `tests/test_ui_prefs.py` —— 后端界面偏好

`GET/POST /api/ui-prefs` 读写；布局风格（1=横向 / 2=纵向）与八列宽度占比；
落盘到 `kqk-app.toml` 的 `[ui]` 段时**保留**注释 / 未知段 / 键序；重启后读回；非法值宽容。
另验证旧配置的显示列迁移：保留用户自定义列 + 自动补 `imported_at`，**不补** `created_at`。

### `tests/test_real_config.py` —— 真实配置文件就地更新

直接拿用户真实的 `dist/kqk-app.toml`（含多行 `track_columns` 数组与全部段落）做验证：
就地更新 `[ui]` 段后，多行数组被规范化为单行（并带上迁移后的 `imported_at`，`created_at` 仍隐藏）、
原有段落与路径值原样保留。注意 `track_columns_saved` 是独立键，断言时不能按 `track_columns` 子串计数。

> 该用例会把 `last_library_dir` 改写为夹具曲库，避免触碰用户真实曲库。

### `tools/run_preview.py` —— 前端无头断言

用 mock fetch 注入假数据，在 3 个窗口尺寸（1080×645 / 900×600 / 1280×800）下断言：
布局风格切换、8 列存储占比 / 默认可见 7 列的列宽比例与拖拽、跨页详情隔离、
**导入时间默认显示 + 创建时间默认隐藏**（含空值隐藏契约）、
「设置 → 列表」勾选 / 取消「创建时间」后四张表同步显隐且 POST `/api/columns`、
外观主题方案还原与落盘。

> 默认列规则（勿回归）：`default_track_columns()` 只有 7 列（含 `imported_at`、不含 `created_at`）；
> `all_track_column_keys()` 仍是 8 列，所以设置面板里能看到但未勾选「创建时间」。
> `track_columns_saved` 用来区分「旧配置可迁移」与「用户显式保存过，不得覆盖」。

### `tests/test_appearance_config.py` —— 外观主题方案持久化

验证「设置 → 外观」的方案确实写进 app 级 `kqk-app.toml` 的 `[ui]` 段：
落盘内容、保留式就地更新、`/api/status` 读回、**重启服务后仍读回**、非法值回退、
清除皮肤时旧值能被空串覆盖、自定义皮肤图片（大 data URL）不写进 TOML。

### `tests/test_boot_appearance.py` —— 首屏外观注入（防启动主题闪烁）

用**前端源码**作为服务目录（`from_dir=C.FRONTEND_SRC`），验证 `/` 与 `/index.html`：
`window.__KQK_BOOT_APPEARANCE__` 已注入且占位符被替换、`Cache-Control: no-store`、
注入值与 `/api/ui-prefs` 的 `appearance` 逐字段一致、新配置 `saved=false`、
POST 保存 dark 主题后重新 GET 立即反映、首帧兜底底色随 dark / light / system 变化、`/index.html` 同样注入、
**无占位符的旧版前端原样返回**（优雅降级，不会因 build.rs 未同步而报错）。

> ⚠️ 该用例依赖**新编译**的 `target/release/mt-cli.exe`（注入逻辑在后端）。
> 改动 `serve.rs` 后须先 `cargo build --release -p mt-cli`，否则会测到旧 exe。

### `tests/test_time_columns.py` —— 导入 / 创建时间（真实端到端）

零 mock：真实 `mt-cli serve` + 真实 SQLite（旧夹具库自动 `ALTER TABLE` 迁移）+ 真实 `dist/frontend`。
验证两个字段在 DB / 四个曲目 API 里都存在、`imported_at` 重扫不变、
**默认列表含「导入时间」不含「创建时间」**、子页面三级列表同步、
「设置 → 列表」勾选后 `/api/columns` 落盘（`track_columns_saved=true`、**用户手写注释不丢**）、
重启后仍显示、取消勾选可再次隐藏。

### `tools/shoot_time_columns.py` —— 时间列三态效果图

用**真实** `mt-cli serve` + 真实夹具曲库 + 真实 `dist/frontend` 截图，生成
`docs/screenshots/time-columns-{library,settings,enabled}.png`。

> ⚠️ 每个状态必须用**独立 app 目录**，并**先建好 `frontend/` 再启动服务**：
> `resolve_frontend_dir()` 只在启动时探测一次，目录当时不存在就会回退到项目源码前端，
> 注入的驱动脚本永远不会执行（三张图会得到同一张默认列表）。
> 另外 `edge_screenshot()` 传独立 `user_data_dir`，避免 Edge 磁盘缓存复用旧 HTML。

### `tools/run_old_regression.py` —— 回归对照

用**修复前**的备份 `app.js` 跑同一套断言，确认断言确实能捕捉到目标 bug
（失败项即为修复前的真实症状）。
