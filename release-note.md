## 更新亮点
- 远程更新检测测试通过

## 说明
- v0.12.5 未单独发版，随本版一并发布；上一公开版本为 v0.12.4。

## 验证基线
- cargo test -p mt-core -p mt-gui 103 passed
- run_all.py --all 16 用例 / 0 失败；check_i18n.py PASS
- 三尺寸前端断言 1680 PASS / 0 FAIL（每尺寸 560）
- 前端源与 dist/frontend 产物 md5 全等
