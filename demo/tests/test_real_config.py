# -*- coding: utf-8 -*-
"""测试：用用户真实 dist/kqk-app.toml 验证 [ui] 段就地更新（含多行数组）。

重点：配置就地更新必须**保留**用户手写内容（注释 / 未知段 / 多行数组 / 既有路径值）。
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

A = C.Asserts('真实配置 [ui] 段就地更新')
R = A.rows

# 运行目录带时间戳（见 common.run_dir）：避免旧目录被占用导致清理失败。
TMP = ''
EXE_DIR = ''
# 复用夹具曲库（固定绝对路径，原地使用）；真实配置文件里的 last_library_dir 会被改写为它，
# 以免触碰用户真实曲库。
LIB = C.FIXTURE_LIB
REAL = os.path.join(C.ROOT, 'dist', 'kqk-app.toml')


def ok(c, m):
    return A.ok(c, m)


def http(path, method='GET', body=None, base=None):
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body).encode('utf-8')
        headers['Content-Type'] = 'application/json'
    req = urllib.request.Request(base + path, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read().decode('utf-8'))


def main():
    if not C.ensure_fixture():
        ok(False, '夹具曲库不可用')
        return A.report()

    global TMP, EXE_DIR
    TMP = C.run_dir('real_config')
    EXE_DIR = os.path.join(TMP, 'app')
    os.makedirs(EXE_DIR, exist_ok=True)
    exe = C.pick_exe()
    shutil.copyfile(exe, os.path.join(EXE_DIR, 'mt-cli.exe'))

    # 用用户真实配置（多行数组 + 全部段落）
    app_cfg = os.path.join(EXE_DIR, 'kqk-app.toml')
    shutil.copyfile(REAL, app_cfg)
    # 把 last_library_dir 指向夹具曲库 → 避免测试触碰用户真实曲库；
    # 其余段落（netease / ncm / 注释 / 路径值）保持原样，正好验证「保留式就地更新」。
    text0 = io.open(app_cfg, encoding='utf-8').read()
    lib_escaped = LIB.replace('\\', '\\\\')
    text0 = re.sub(r"last_library_dir = '[^']*'",
                    "last_library_dir = '%s'" % lib_escaped, text0, count=1)
    # 真实配置会被运行中的界面就地更新（多行数组规范化成单行），不能假设它永远是多行。
    # 这里统一把 track_columns 重置为多行形式，保证「多行数组就地更新」这一核心场景可稳定复现；
    # 其余内容仍 100% 取自真实配置文件。
    multi_cols = ('track_columns = [\n'
                  '    "name",\n    "artist",\n    "album",\n'
                  '    "quality_tier",\n    "duration_ms",\n    "file_size",\n]')
    if re.search(r'track_columns\s*=\s*\[', text0):
        text0 = re.sub(r'track_columns\s*=\s*\[[^\]]*\]', multi_cols, text0,
                       count=1, flags=re.S)
    else:
        text0 = text0.replace('[ui]', '[ui]\n' + multi_cols, 1)
    with io.open(app_cfg, 'w', encoding='utf-8') as f:
        f.write(text0)
    before = io.open(app_cfg, encoding='utf-8').read()
    ok('track_columns = [' in before and '\n    "name",' in before, '起始配置含多行 track_columns 数组')

    proc = None
    try:
        proc, port = C.start_server(EXE_DIR)
        base = 'http://127.0.0.1:%d' % port
        for _ in range(60):
            try:
                http('/api/status', base=base)
                break
            except Exception:
                time.sleep(0.25)
        st = http('/api/status', base=base)
        # 真实配置是旧版 6 列 → sanitize() 迁移：补默认显示的 imported_at，不补 created_at。
        ok(st.get('track_columns') == ['name', 'artist', 'album', 'quality_tier',
                                       'duration_ms', 'file_size', 'imported_at'],
           '多行数组被正确解析并迁移为 7 列（创建时间默认隐藏）：%s' % st.get('track_columns'))

        http('/api/ui-prefs', 'POST', {
            'layout_styles': {'library': 2, 'artists': 1},
            'column_widths': {'name': 38.5, 'artist': 14.5, 'album': 15.0,
                              'quality_tier': 5.0, 'duration_ms': 5.0, 'file_size': 6.0,
                              'imported_at': 8.0, 'created_at': 8.0},
        }, base=base)

        after = io.open(app_cfg, encoding='utf-8').read()
        # 注意不能数子串：新字段 track_columns_saved 也含 "track_columns"，只数键行。
        ok(len(re.findall(r'(?m)^track_columns\s*=', after)) == 1,
           'track_columns 键未重复写入（track_columns_saved 是另一个键）')
        # 多行数组被就地重写为单行规范形式，并带上迁移后的 imported_at；续行不残留。
        ok('track_columns = ["name", "artist", "album", "quality_tier", '
           '"duration_ms", "file_size", "imported_at"]' in after,
           '多行 track_columns 被规范化为单行且含迁移后的 7 列')
        col_line = re.search(r'(?m)^track_columns\s*=.*$', after)
        ok(bool(col_line) and 'created_at' not in col_line.group(0),
           '默认可见列里不含 created_at')
        ok('\n    "name",' not in after, '旧多行数组的续行未被残留')
        ok('layout_library = 2' in after and 'layout_artists = 1' in after, '布局风格已就地写入 [ui] 段')
        ok('width_name = 38.5' in after, '列宽比例已就地写入 [ui] 段')
        for key in ('[netease]', '[ncm]', '[scanner]', '[import]', '[server]', '[logging]', '[meta]'):
            ok(key in after, '原有段落 %s 保留' % key)
        ok("library_dir = 'E:\\CloudMusic'" in after, '原有 netease 路径值原样保留')
        ok("tool_path = 'D:\\temp\\testMusicLab\\.mt\\tools\\ncmdump.exe'" in after, '原有 ncm 工具路径原样保留')

        import tomllib
        doc = tomllib.load(open(app_cfg, 'rb'))
        ok(len(doc['ui']['track_columns']) == 7,
           '落盘文件仍可被 TOML 解析，track_columns 7 项（创建时间隐藏）')
        tot = sum(doc['ui'][k] for k in ('width_name', 'width_artist', 'width_album',
                                         'width_quality_tier', 'width_duration_ms', 'width_file_size',
                                         'width_imported_at', 'width_created_at'))
        ok(abs(tot - 100) < 0.01, '落盘八列占比之和 100（%s）' % round(tot, 3))
        ok(doc['ui']['layout_library'] == 2 and doc['ui']['layout_artists'] == 1, '落盘布局风格正确')
        ok(doc['netease']['library_dir'] == 'E:\\CloudMusic', '落盘后 netease 目录未变')
    finally:
        C.stop_server(proc)

    return A.report()


if __name__ == '__main__':
    sys.exit(main())
