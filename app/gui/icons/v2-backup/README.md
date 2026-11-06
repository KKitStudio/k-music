# v2 图标（历史备份 · 已被否决）

冻结日期：2026-11-06
来源：`app/gui/icons/` 在 2026-11-06 09:33 时的正式成品（B 方案变体），三文件 MD5 与备份完全一致。

## 设计
低对比圆角方块（`#EEEBE7` → `#DBD6D0`）+ 深灰褐 `#4C4741` Segoe Script Bold 手写体：
大 K 置**左上**(x=300,y=470,size=400)、`music` 在中下(x=530,y=700,size=235)、细旋律飘带横贯下方。

## 否决原因
用户 2026-11-06 反馈「感觉不如 v1 的横版」，且觉得 v1 字标偏小。
现正式图标已改为 **v3：v1 横版构图 + 字标放大 1.20x**（见 `v3-candidates/H2-medium.svg`）。

## 回滚方法
```bash
cp app/gui/icons/v2-backup/icon.svg app/gui/icons/icon.svg
python tools/icon_pipeline.py
cd app/gui && touch tauri.conf.json && cd .. && cargo build -p mt-gui
```

## 校验
- `tools/icon_pipeline.py` 可从源 SVG 完整复现 PNG/ICO（品红键色软抠图 + 面积平均降采样）。
- ICO 为 256/128/64/48/32/16 六帧，BITMAPINFOHEADER 严格 40 字节，目录项无重叠。
- 改图标后必须重编 `mt-gui`，`build.rs` 的 `tauri_build::build()` 才会重新嵌入 exe 资源。
