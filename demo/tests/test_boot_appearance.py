# -*- coding: utf-8 -*-
"""测试：入口 HTML 首屏注入已保存外观，消除启动时的默认主题闪烁。

背景（用户报告的 bug）：GUI 本地服务端口是动态的（端口占用时回退随机端口），
而 localStorage 按 origin 隔离（含端口），端口一变缓存就失效；styles.css 的
:root 是浅色、app.js 在 body 末尾、/api/status 还是异步的，于是启动时先闪一帧
默认浅色，再切到用户设置的主题。

覆盖：
  1. GET / 与 /index.html 返回注入后的 HTML，且 Cache-Control: no-store；
  2. 注入值与 /api/ui-prefs 的 appearance 一致（权威源 = kqk-app.toml [ui]）；
  3. 新配置 saved=false（前端保留旧偏好迁移语义）；
  4. 保存 dark 主题后重新 GET / 立即反映新值（不被浏览器缓存挡住）；
  5. 首帧兜底底色随 light / dark / system 变化（防「先整体黑一下」）；
  6. 旧版前端（无占位符）原样返回，优雅降级不报错。

注意：运行前需先 `cargo build --release -p mt-cli`，否则测到的是旧 exe。
"""
import io
import json
import os
import re
import shutil
import sys
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import common as C

A = C.Asserts('首屏外观注入（防启动主题闪烁）')
BOOT_RE = re.compile(r'window\.__KQK_BOOT_APPEARANCE__=(\{.*?\});</script>', re.S)


def ok(cond, msg):
    return A.ok(cond, msg)


def http_json(base, path, method='GET', body=None):
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body).encode('utf-8')
        headers['Content-Type'] = 'application/json'
    req = urllib.request.Request(base + path, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read().decode('utf-8'))


def http_html(base, path='/'):
    req = urllib.request.Request(base + path, method='GET')
    with urllib.request.urlopen(req, timeout=10) as r:
        return r.headers.get('Cache-Control', ''), r.read().decode('utf-8')


def boot_of(html):
    m = BOOT_RE.search(html)
    return json.loads(m.group(1)) if m else None


def wait_up(base, tries=60):
    for _ in range(tries):
        try:
            return http_json(base, '/api/status')
        except Exception:
            time.sleep(0.25)
    return None


def main():
    if not C.ensure_fixture():
        ok(False, '夹具曲库不可用')
        return A.report()

    tmp = C.run_dir('boot-appearance')
    exe_dir = os.path.join(tmp, 'app')
    os.makedirs(exe_dir, exist_ok=True)
    shutil.copyfile(C.pick_exe(), os.path.join(exe_dir, 'mt-cli.exe'))
    # 用前端源码（而非 dist 快照）作为服务目录，确保测的是本轮改动。
    C.build_frontend_copy(os.path.join(exe_dir, 'frontend'), from_dir=C.FRONTEND_SRC)
    idx = os.path.join(exe_dir, 'frontend', 'index.html')
    with io.open(idx, encoding='utf-8') as f:
        original_html = f.read()

    cfg = os.path.join(exe_dir, 'kqk-app.toml')
    lib_escaped = C.FIXTURE_LIB.replace('\\', '\\\\')
    with io.open(cfg, 'w', encoding='utf-8') as f:
        f.write(
            "# 用户手写注释\n"
            "last_library_dir = '%s'\n\n"
            "[meta]\nconfig_version = \"0.4.0\"\nmin_compatible_version = \"0.2.0\"\n\n"
            "[ui]\n# ui 段注释\nmy_unknown_key = \"keep-me\"\n" % lib_escaped
        )

    proc = None
    try:
        proc, port = C.start_server(exe_dir)
        base = 'http://127.0.0.1:%d' % port
        ok(wait_up(base) is not None, '服务已启动并响应 /api/status')
        if wait_up(base) is None:
            return A.report()

        # ---------- 1) 入口 HTML 注入 + 禁缓存 ----------
        cc, html = http_html(base, '/')
        ok('no-store' in cc.lower(), 'GET / 返回 Cache-Control: no-store（%r）' % cc)
        boot = boot_of(html)
        ok(boot is not None, 'GET / 已注入 window.__KQK_BOOT_APPEARANCE__')
        ok('<!--KQK_BOOT_APPEARANCE-->' not in html, '占位符已被替换，不会留在页面里')
        if boot is None:
            return A.report()

        # ---------- 2) 与权威源一致 ----------
        api = (http_json(base, '/api/ui-prefs') or {}).get('appearance') or {}
        keys = ('theme', 'accent', 'skin_type', 'skin_preset', 'skin_accent', 'saved')
        ok(all(boot.get(k) == api.get(k) for k in keys),
           '注入值与 /api/ui-prefs 的 appearance 逐字段一致')
        ok(boot.get('saved') is False,
           '新配置 saved=false（前端仍按旧偏好迁移语义处理）')

        # ---------- 3) 改主题后立即反映 ----------
        want = {'theme': 'dark', 'accent': '#7e57c2',
                'skin_type': '', 'skin_preset': '', 'skin_accent': ''}
        res = http_json(base, '/api/ui-prefs', 'POST', {'appearance': want})
        ok(res.get('ok') is True, 'POST /api/ui-prefs 保存 dark 主题成功')
        cc2, html2 = http_html(base, '/')
        boot2 = boot_of(html2)
        ok(boot2 is not None and boot2.get('theme') == 'dark',
           '重新 GET / 立即拿到 theme=dark（首帧不会再是默认浅色）')
        ok(boot2 is not None and boot2.get('accent') == '#7e57c2',
           '重新 GET / 立即拿到新强调色')
        ok(boot2 is not None and boot2.get('saved') is True,
           'saved=true 表示首屏以配置为准')
        ok('no-store' in cc2.lower(), '改主题后仍为 no-store，不会命中旧 HTML 缓存')

        # ---------- 3b) 首帧兜底底色随主题变化（防「先整体黑一下」） ----------
        ok('<style>html{background:#121830}</style>' in html2,
           'dark 主题注入深色兜底底色，WebView 首次绘制前的空窗不再是纯黑')
        want = dict(want, theme='light')
        res = http_json(base, '/api/ui-prefs', 'POST', {'appearance': want})
        ok(res.get('ok') is True, 'POST /api/ui-prefs 保存 light 主题成功')
        _, html_light = http_html(base, '/')
        ok('<style>html{background:#f6f8fc}</style>' in html_light,
           'light 主题注入浅色兜底底色')
        want = dict(want, theme='system')
        res = http_json(base, '/api/ui-prefs', 'POST', {'appearance': want})
        ok(res.get('ok') is True, 'POST /api/ui-prefs 保存 system 主题成功')
        _, html_system = http_html(base, '/')
        ok('<style>html{background:#f6f8fc}@media (prefers-color-scheme:dark)' in html_system,
           'system 模式用媒体查询按系统偏好裁决首帧底色')

        # ---------- 4) /index.html 同样注入 ----------
        _, html3 = http_html(base, '/index.html')
        ok(boot_of(html3) is not None, '直接访问 /index.html 也注入外观')

        # ---------- 5) 旧版前端优雅降级 ----------
        with io.open(idx, 'w', encoding='utf-8') as f:
            f.write('<html><head></head><body>legacy frontend</body></html>')
        _, legacy = http_html(base, '/')
        ok('legacy frontend' in legacy and boot_of(legacy) is None,
           '旧版前端（无占位符）原样返回、不报错')
    finally:
        try:
            with io.open(idx, 'w', encoding='utf-8') as f:
                f.write(original_html)
        except Exception:
            pass
        C.stop_server(proc)
    return A.report()


if __name__ == '__main__':
    sys.exit(main())
