# 项目：个人曲库管理工具（MusicLibrary）

## 项目定位
- 本地音乐库管理 + 网易云音乐备份下载工具
- 支持 CLI 与 GUI 双模式，GUI 基于 Web 前端 + 后端服务
- 支持 ncm 格式解密/转码

## 技术栈
- 后端核心：Rust（推测，target/release 结构）
- 前端：Web 技术（frontend 目录）
- 构建：Cargo + 前端构建工具
- 第三方：ncm 解密工具（tools/ncm，C/C++ 实现）

## 目录结构说明
- `app/core/src`：核心业务逻辑（音乐库、备份、下载）
- `app/cli/src`：命令行入口
- `app/gui/src`：GUI 后端服务
- `app/gui/frontend`：GUI 前端代码
- `app/gui/gen/schemas`：接口/数据模型定义
- `app/gui/icons`：应用图标资源
- `app/data`：应用数据（配置、数据库等）
- `dist/`：构建输出目录
- `target/release/`：Rust 发布构建产物
- `target/release/musiclibrary/`：运行数据根目录
  - `download/ncm/`：已下载 ncm 文件
  - `download/pendingNcm/`：待处理 ncm
  - `download/ing/`：下载中临时文件
- `tools/ncm/`：ncm 解密工具源码与二进制
- `docs/`：设计文档、分析报告

## 编码规范
- Rust：遵循官方风格，使用 `cargo fmt` 和 `cargo clippy`
- 前端：TypeScript 严格模式，组件化
- 提交信息：Conventional Commits
- 所有配置项通过 `app/data/` 下的配置文件管理，不硬编码

## 会话协作约定
| 会话 | 职责 | 产出位置 |
|------|------|----------|
| 开发 | 核心功能实现 | `app/core/src/`、`app/cli/src/`、`app/gui/src/` |
| UI | 界面美化与换肤 | `app/gui/frontend/`、`app/gui/icons/` |
| 裁剪 | 性能分析与瘦身 | `docs/optimization.md`、代码清理 |
| Release | 整合修改与核对 | `docs/release-notes.md` |

## 网易云备份规范
- Cookie / 登录凭证只存放在 `.env` 或 `app/data/` 下的本地配置中，不提交 Git
- 下载任务使用增量标记：pending → ing → ncm
- 接口请求需控制频率，避免触发风控
- ncm 解密调用 `tools/ncm/bin/` 下的工具，不直接内嵌算法

## 常用命令
- 构建：`cargo build --release`
- 运行：`target/release/musiclibrary.exe`（或对应二进制）
- 前端开发：`cd app/gui/frontend && npm run dev`
- 前端构建：`npm run build`

## 当前进度
- [x] 项目初始化
- [x] CLI 基础框架
- [x] ncm 解密工具集成
- [ ] GUI 界面完善
- [ ] 换肤功能
- [ ] 性能优化与瘦身