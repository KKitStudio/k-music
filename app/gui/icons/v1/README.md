# 应用图标 v1（冻结备份）

冻结日期：2026-11-06
来源：`app/gui/icons/` 当时的正式成品，三文件 MD5 与备份完全一致。

> 状态：同日用户已选定 v2（B 方案变体：K 在左上 + 下方飘带）并替换正式图标，**本目录现为历史备份，不再代表当前正式图标**。

## 文件内容
| 文件 | 说明 |
|---|---|
| `icon.svg` | 唯一设计源（透明底、1024×1024 viewBox） |
| `icon.png` | 512×512 RGBA，四角透明 |
| `icon.ico` | 256/128/64/48/32/16 六帧，32bpp，40 字节 BITMAPINFOHEADER |

## 设计规格
- 背景：低对比中性暖灰渐变 `#EEEBE7 → #DBD6D0`，圆角 `rx=224`，四角透明。
- 字标：Segoe Script Bold（Windows 自带 `segoescb.ttf`）连笔手写美术字，`K` 大（224）、`music` 小（160），`dx=30` 拉开字距，基线 `y=592`，色 `#4C4741`。
- 点缀：细旋律飘带（stroke-width 7 / opacity 0.45），从左下划过并末端上扬收笔。
- 关键约束：**不得使用高饱和蓝底 + 纯白粗黑体**（前两版被否决原因）；必须保留低对比、柔和、平滑边缘。

## 生成方式
使用仓库根目录 `tools/icon_pipeline.py`（可复用）：SVG → Edge 无头渲染到品红键色背景 → 软抠图（反混色还原真实色，避免硬阈值粉边）→ 面积平均降采样 → 自写 PNG/ICO 编码。
`tauri.conf.json` 的 `bundle.icon` 引用 `icons/icon.ico` 与 `icons/icon.png`，由 `tauri_build::build()` 编译期嵌入 exe；**修改图标后必须重编 `mt-gui`**（可 `touch app/gui/tauri.conf.json` 强制重跑 build script）。

## 恢复方法
将本目录三文件复制回 `app/gui/icons/` 覆盖，然后 `cargo build -p mt-gui`。
