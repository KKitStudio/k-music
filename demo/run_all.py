# -*- coding: utf-8 -*-
"""demo 一键运行：按顺序跑完所有测试并汇总结果。

用法
----
    python demo/run_all.py              # 跑全部（默认跳过需要先构建的用例）
    python demo/run_all.py --all        # 跑全部，含前端无头断言（需先 gen_preview）
    python demo/run_all.py --only detail_region
    python demo/run_all.py --list       # 列出所有用例

说明
----
* 用例全部复用 `demo/fixtures/library` 里的**持久化夹具曲库**，
  不会每次重新生成音频、重建曲库（DB 存绝对路径 → 必须原地复用）。
* 运行产物写在 `demo/.run/`（临时，可随时删）。
"""
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import common as C

# 用例：名称 → (脚本路径, 说明, 是否需要前端构建产物)
CASES = [
    ('ui_prefs', os.path.join('tests', 'test_ui_prefs.py'),
     '后端 /api/ui-prefs 读写 + [ui] 段保留式就地更新', False),
    ('real_config', os.path.join('tests', 'test_real_config.py'),
     '用用户真实 dist/kqk-app.toml 验证就地更新不破坏内容', False),
    ('appearance_config', os.path.join('tests', 'test_appearance_config.py'),
     '外观主题方案落盘 + 重启读回 + 非法值回退', False),
    ('boot_appearance', os.path.join('tests', 'test_boot_appearance.py'),
     '入口 HTML 首屏注入外观（消除启动主题闪烁）', False),
    ('single_instance', os.path.join('tests', 'test_single_instance.py'),
     '/api/focus 单实例唤起端点（CLI 未注册 sink 时优雅空转）', False),
    ('detail_region', os.path.join('tests', 'test_detail_region.py'),
     '详情区公共组件：三页一致的隐藏契约 + 跨页详情隔离', False),
    ('time_columns', os.path.join('tests', 'test_time_columns.py'),
     '导入/创建时间：真实 DB 迁移 + API + 前端渲染', False),
    ('frontend_ui', os.path.join('tools', 'gen_preview.py'),
     '（准备）生成前端 mock 预览页', False),
    ('frontend_assert', os.path.join('tools', 'run_preview.py'),
     '前端无头断言：布局风格 / 列宽 / 跨页详情隔离（3 窗口尺寸）', True),
]


def list_cases():
    for name, rel, desc, _ in CASES:
        print('  %-16s %-46s %s' % (name, rel, desc))
    return 0


def run_case(name, rel, desc, needs_preview):
    path = os.path.join(HERE, rel)
    print()
    print('#' * 68)
    print('# 用例：%s —— %s' % (name, desc))
    print('#' * 68)
    if needs_preview and not os.path.isfile(os.path.join(C.RUN, 'preview', 'index.html')):
        print('SKIP：缺少 demo/.run/preview/index.html（先跑 gen_preview）')
        return 0, 'SKIP'
    t0 = time.time()
    p = subprocess.run([sys.executable, path], cwd=HERE)
    dt = time.time() - t0
    print('--- %s 用时 %.1fs，退出码 %d ---' % (name, dt, p.returncode))
    return p.returncode, 'OK' if p.returncode == 0 else 'FAIL'


def main():
    args = sys.argv[1:]
    if '--list' in args:
        return list_cases()

    only = None
    for i, a in enumerate(args):
        if a == '--only' and i + 1 < len(args):
            only = args[i + 1]

    # 夹具准备（幂等）
    print('== 准备测试夹具 ==')
    if not C.ensure_fixture():
        print('夹具构建失败')
        return 2

    results = []
    for name, rel, desc, needs_preview in CASES:
        if only and only != name:
            continue
        rc, status = run_case(name, rel, desc, needs_preview)
        results.append((name, status, rc))

    print()
    print('=' * 68)
    print('汇总')
    print('=' * 68)
    failed = 0
    for name, status, rc in results:
        print('  %-16s %s' % (name, status))
        if status == 'FAIL':
            failed += 1
    print('总计 %d 个用例，失败 %d 个' % (len(results), failed))
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main() or 0)
