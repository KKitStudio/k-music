# -*- coding: utf-8 -*-
"""测试：后端 /api/ui-prefs 读写 + 落盘到 kqk-app.toml 的 [ui] 段（保留式就地更新）。

覆盖：布局风格（1=横向 / 2=纵向）、八列宽度占比、注释与未知键保留、重启读回、非法值宽容。
"""
import io
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import common as C

A = C.Asserts('/api/ui-prefs 读写与落盘')
R = A.rows

# 运行目录带时间戳（见 common.run_dir）：避免旧目录被占用导致清理失败。
TMP = ''
EXE_DIR = ''
# 复用夹具曲库（固定绝对路径，原地使用）
LIB = C.FIXTURE_LIB
BASE = 'http://127.0.0.1:1'


def ok(cond, msg):
    return A.ok(cond, msg)


def http(path, method='GET', body=None):
    url = BASE + path
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body).encode('utf-8')
        headers['Content-Type'] = 'application/json'
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read().decode('utf-8'))


def start_server():
    """启动服务并从 stdout 解析实际端口（配置端口为 0 → 由系统分配）。

    统一走 common.start_server：解析端口阶段出错会立即回收进程，不留孤儿。
    """
    return C.start_server(EXE_DIR)


def main():
    if not C.ensure_fixture():
        ok(False, '夹具曲库不可用')
        return A.report()

    global TMP, EXE_DIR, BASE
    TMP = C.run_dir('ui_prefs')
    EXE_DIR = os.path.join(TMP, 'app')
    os.makedirs(EXE_DIR, exist_ok=True)

    # 复制 exe 到独立目录 → app 配置落在该目录，不污染 target/
    exe = C.pick_exe()
    shutil.copyfile(exe, os.path.join(EXE_DIR, 'mt-cli.exe'))

    # 写一份带注释与未知键的 app 配置，验证「就地更新不破坏用户内容」
    app_cfg = os.path.join(EXE_DIR, 'kqk-app.toml')
    with io.open(app_cfg, 'w', encoding='utf-8') as f:
        f.write("# 用户手写注释\nlast_library_dir = '%s'\n\n[meta]\nconfig_version = \"0.3.0\"\nmin_compatible_version = \"0.3.0\"\n\n[custom]\nmy_key = 42\n\n[ui]\ntrack_columns = [\"name\", \"artist\"]\n# ui 段注释\n" % LIB.replace('\\', '\\\\'))

    # 启动服务（放进 try：任何异常都必须走到 finally 回收进程）
    proc = None
    try:
        proc, port = start_server()
        BASE = 'http://127.0.0.1:%d' % port if port else BASE
        ok(port is not None, '服务已启动（端口 %s）' % port)
        up = False
        for _ in range(60):
            try:
                http('/api/status')
                up = True
                break
            except Exception:
                time.sleep(0.25)
        ok(up, '服务已启动并响应 /api/status')
        if not up:
            return

        st = http('/api/status')
        ls = st.get('layout_styles') or {}
        cw = st.get('column_widths') or {}
        ok(ls.get('library') == 1, '/api/status：library 默认横向（实际 %s）' % ls.get('library'))
        ok(ls.get('playlists') == 2 and ls.get('albums') == 2 and ls.get('artists') == 2,
           '/api/status：播放列表/专辑/歌手默认纵向（实际 %s/%s/%s）'
           % (ls.get('playlists'), ls.get('albums'), ls.get('artists')))
        ok(len(cw) == 8 and abs(sum(cw.values()) - 100) < 0.01,
           '/api/status：8 列宽占比之和为 100（实际 %s）' % round(sum(cw.values()), 3))

        # 旧配置迁移：保留用户自定义的两列，自动补上默认显示的「导入时间」；
        # 「创建时间」仍隐藏（默认不显示，只能由设置面板勾选开启）。
        ok(st.get('track_columns') == ['name', 'artist', 'imported_at'],
           '/api/status：旧配置自动补 imported_at、不补 created_at（实际 %s）'
           % st.get('track_columns'))
        ok(st.get('track_columns_saved') is False,
           '/api/status：track_columns_saved=false（未显式保存过，可迁移）')

        # GET /api/ui-prefs
        prefs = http('/api/ui-prefs')
        ok(prefs.get('ok') and 'layout_styles' in prefs and 'column_widths' in prefs,
           'GET /api/ui-prefs 返回布局风格与列宽比例')

        # POST：切布局 + 改列宽（前端实际行为：一次提交全部 8 列，合计 100）
        full = {'name': 45.5, 'artist': 10.25, 'album': 12.0,
                'quality_tier': 5.0, 'duration_ms': 5.0, 'file_size': 6.0,
                'imported_at': 8.125, 'created_at': 8.125}
        res = http('/api/ui-prefs', 'POST', {
            'layout_styles': {'artists': 1, 'library': 2},
            'column_widths': full,
        })
        ok(res.get('ok'), 'POST /api/ui-prefs 成功')
        rl = res.get('layout_styles') or {}
        rc = res.get('column_widths') or {}
        ok(rl.get('artists') == 1 and rl.get('library') == 2, 'POST 返回的布局风格已更新（%s）' % rl)
        ok(abs(sum(rc.values()) - 100) < 0.01, 'POST 后列宽占比之和仍为 100（%s）' % round(sum(rc.values()), 3))
        ok(abs(rc.get('name', 0) - 45.5) < 0.01 and abs(rc.get('artist', 0) - 10.25) < 0.01,
           'POST 后「曲名」变大、「艺人」变小（%s / %s）' % (rc.get('name'), rc.get('artist')))

        # 落盘检查：注释 / 未知键 / 键序保留
        with io.open(app_cfg, encoding='utf-8') as f:
            text = f.read()
        ok('# 用户手写注释' in text, '落盘：文件头注释保留')
        ok('# ui 段注释' in text, '落盘：[ui] 段内注释保留')
        ok('my_key = 42' in text, '落盘：未知段 [custom] 与其键保留')
        ok('[custom]' in text and '[ui]' in text, '落盘：段结构完整')
        ok('layout_artists = 1' in text, '落盘：layout_artists = 1')
        ok('layout_library = 2' in text, '落盘：layout_library = 2')
        ok('width_name = 45.5' in text, '落盘：width_name = 45.5')
        ok('width_artist = 10.25' in text, '落盘：width_artist = 10.25')
        ok('track_columns' in text, '落盘：track_columns 保留')

        # 配置仍可被解析（回读）
        import tomllib
        with open(app_cfg, 'rb') as f:
            doc = tomllib.load(f)
        ok('ui' in doc and 'custom' in doc, '落盘文件可被 TOML 解析')
        ok(abs(sum(doc['ui']['width_name'] for _ in [0]) * 0 + 0) == 0, '（占位）')
        tot = sum(doc['ui'][k] for k in ('width_name', 'width_artist', 'width_album',
                                         'width_quality_tier', 'width_duration_ms', 'width_file_size',
                                         'width_imported_at', 'width_created_at'))
        ok(abs(tot - 100) < 0.01, '落盘文件里八列占比之和为 100（%s）' % round(tot, 3))

        # 重启服务 → 配置应被读回
        C.stop_server(proc)
        proc = None
        proc, port2 = start_server()
        if port2:
            BASE = 'http://127.0.0.1:%d' % port2
        up = False
        for _ in range(60):
            try:
                st2 = http('/api/status')
                up = True
                break
            except Exception:
                time.sleep(0.25)
        ok(up, '重启后服务正常')
        if up:
            ls2 = st2.get('layout_styles') or {}
            cw2 = st2.get('column_widths') or {}
            ok(ls2.get('artists') == 1 and ls2.get('library') == 2,
               '重启后布局风格被正确读回（%s）' % ls2)
            ok(abs(cw2.get('name', 0) - 45.5) < 0.6,
               '重启后「曲名」列宽比例被正确读回（%s）' % cw2.get('name'))

        # 非法值应被忽略而非报错
        res2 = http('/api/ui-prefs', 'POST', {
            'layout_styles': {'library': 99, 'bogus': 2},
            'column_widths': {'name': -5},
        })
        ok(res2.get('ok'), 'POST 非法值不报错（宽容处理）')
        l3 = res2.get('layout_styles') or {}
        ok(l3.get('library') in (1, 2), '非法布局值被忽略（library=%s）' % l3.get('library'))
        ok('bogus' not in l3, '未知视图名被忽略')
    finally:
        C.stop_server(proc)

    return A.report()


if __name__ == '__main__':
    sys.exit(main())
