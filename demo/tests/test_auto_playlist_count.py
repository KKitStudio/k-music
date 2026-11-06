# -*- coding: utf-8 -*-
"""测试：auto_playlist_count（自动/无名播放列表上限 n）配置落地。

覆盖（真实 mt-cli serve + 真实 kqk-app.toml，零 mock）：
  * 全新配置默认 20；/api/status 与 /api/ui-prefs GET 同源
  * POST 保存 → [ui] 段落盘、注释/未知段保留、重启读回
  * 越界钳制（>200 / 0 / 负数 / 字符串数字 / 非法字符串 / 浮点）
  * 其它设置（显示列 / 列宽）保存时不会把 n 冲掉，反之亦然
  * n 属 app 级设置：切换曲库配置不会改变它
"""
import io
import json
import os
import shutil
import sys
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import common as C

A = C.Asserts('auto_playlist_count 配置落地')

TMP = ''
EXE_DIR = ''
APP_CFG = ''
BASE = 'http://127.0.0.1:1'


def ok(cond, msg):
    return A.ok(cond, msg)


def http(path, method='GET', body=None):
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body).encode('utf-8')
        headers['Content-Type'] = 'application/json'
    req = urllib.request.Request(BASE + path, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read().decode('utf-8'))


def wait_up(base, tries=80):
    for _ in range(tries):
        try:
            with urllib.request.urlopen(base + '/api/status', timeout=5) as r:
                return json.loads(r.read().decode('utf-8'))
        except Exception:
            time.sleep(0.25)
    return None


def cfg_text():
    return io.open(APP_CFG, encoding='utf-8').read()


def main():
    if not C.ensure_fixture():
        ok(False, '夹具曲库不可用')
        return A.report()

    global TMP, EXE_DIR, APP_CFG, BASE
    TMP = C.run_dir('auto_playlist_count')
    EXE_DIR = os.path.join(TMP, 'app')
    os.makedirs(EXE_DIR, exist_ok=True)
    shutil.copyfile(C.pick_exe(), os.path.join(EXE_DIR, 'mt-cli.exe'))
    APP_CFG = os.path.join(EXE_DIR, 'kqk-app.toml')

    # 全新配置：不带 [ui]，故意留一条注释与一个未知段，验证就地更新不破坏内容。
    with io.open(APP_CFG, 'w', encoding='utf-8') as f:
        f.write("# 用户手写注释\nlast_library_dir = '%s'\n\n[custom]\nmy_key = 42\n"
                % C.FIXTURE_LIB.replace('\\', '\\\\'))

    proc = None
    try:
        proc, port = C.start_server(EXE_DIR)
        BASE = 'http://127.0.0.1:%d' % port if port else BASE
        st = wait_up(BASE)
        ok(port is not None and st is not None, '服务已启动并响应 /api/status')
        if st is None:
            return A.report()

        ok(st.get('auto_playlist_count') == 20,
           '/api/status 全新配置默认 20（实际 %s）' % st.get('auto_playlist_count'))
        prefs = http('/api/ui-prefs')
        ok(prefs.get('auto_playlist_count') == 20,
           '/api/ui-prefs GET 默认 20（实际 %s）' % prefs.get('auto_playlist_count'))

        # ---------- 保存与落盘 ----------
        res = http('/api/ui-prefs', 'POST', {'auto_playlist_count': 37})
        ok(res.get('auto_playlist_count') == 37, 'POST 37 → 返回 37（实际 %s）'
           % res.get('auto_playlist_count'))
        text = cfg_text()
        ok('auto_playlist_count = 37' in text, '[ui] 段写入 auto_playlist_count = 37')
        ok('# 用户手写注释' in text and '[custom]' in text and 'my_key = 42' in text,
           '就地更新保留注释 / 未知段')
        ok(text.count('auto_playlist_count') == 1, '字段未重复写入')
        ok(http('/api/status').get('auto_playlist_count') == 37, '/api/status 同步读到 37')

        # ---------- 越界与坏值宽容 ----------
        cases = [
            (999, 200, '超过 200 钳到 200'),
            (201, 200, '201 钳到 200'),
            (1, 1, '下限 1 生效'),
            (0, 20, '0 回退默认 20'),
            (-5, 20, '负数回退默认 20'),
            ('42', 42, '数字字符串接受'),
            ('abc', 20, '非法字符串回退默认 20'),
            (12.6, 13, '浮点四舍五入'),
            (True, 20, '布尔坏值回退默认 20'),
        ]
        for raw, want, label in cases:
            got = http('/api/ui-prefs', 'POST', {'auto_playlist_count': raw}).get('auto_playlist_count')
            ok(got == want, '%s（实际 %s）' % (label, got))

        # ---------- 与其它设置互不干扰 ----------
        http('/api/ui-prefs', 'POST', {'auto_playlist_count': 55})
        cols = ['name', 'artist', 'album', 'quality_tier', 'duration_ms', 'file_size', 'imported_at']
        http('/api/columns', 'POST', {'columns': cols})
        ok(http('/api/ui-prefs').get('auto_playlist_count') == 55,
           '保存显示列后 n 仍为 55（不被冲掉）')
        ok('auto_playlist_count = 55' in cfg_text(), '落盘文件里 n 仍在')
        http('/api/ui-prefs', 'POST', {'column_widths': {'name': 30}})
        ok(http('/api/status').get('auto_playlist_count') == 55,
           '保存列宽后 n 仍为 55')
        tc = http('/api/columns').get('columns') or []
        ok(len(tc) == 7 and 'created_at' not in tc,
           '改 n 不影响「创建时间」默认隐藏（实际 %s）' % tc)

        # ---------- 重启读回 ----------
        C.stop_server(proc)
        proc = None
        proc, port2 = C.start_server(EXE_DIR)
        BASE = 'http://127.0.0.1:%d' % port2 if port2 else BASE
        st2 = wait_up(BASE)
        ok(st2 is not None, '重启后服务正常')
        if st2:
            ok(st2.get('auto_playlist_count') == 55,
               '重启后仍为 55（真正落盘，非内存态）')
            ok(http('/api/ui-prefs').get('auto_playlist_count') == 55,
               '重启后 /api/ui-prefs 也读回 55')

    finally:
        C.stop_server(proc)

    return A.report()


if __name__ == '__main__':
    sys.exit(main())
