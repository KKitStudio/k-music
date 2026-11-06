# -*- coding: utf-8 -*-
"""测试：播放列表导入导出（FR-20 / 格式方案 A）真实端到端。

覆盖：
  * 导出单个列表全量，同时产 `.json` 无损备份 + `.m3u8` 单向投影（UTF-8 Extended M3U）
  * M3U8 写相对曲库根路径、文件名安全化（非法字符不进文件名）
  * **n 不影响导出**：把 auto_playlist_count 设为 1 后导出仍是列表全量
  * 导入 M3U8 / JSON 备份 / CSV（网易云常见只有曲名+艺人）都能匹配回本地 id
  * 两级去重与报告：重复行、未匹配跳过不建占位、多副本歧义
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

A = C.Asserts('播放列表导入导出（批次 2）')

TMP = ''
EXE_DIR = ''
LIB = ''
EXPORT_DIR = ''
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
    with urllib.request.urlopen(req, timeout=20) as r:
        return json.loads(r.read().decode('utf-8'))


def wait_up(base, tries=80):
    for _ in range(tries):
        try:
            with urllib.request.urlopen(base + '/api/status', timeout=5) as r:
                return json.loads(r.read().decode('utf-8'))
        except Exception:
            time.sleep(0.25)
    return None


def fixture_wavs():
    out = []
    for root, _dirs, files in os.walk(C.FIXTURE_LIB):
        if '.mt' in root.split(os.sep):
            continue
        for f in files:
            if f.lower().endswith('.wav'):
                out.append(os.path.join(root, f))
    return sorted(out)


def main():
    if not C.ensure_fixture():
        ok(False, '夹具曲库不可用')
        return A.report()

    global TMP, EXE_DIR, LIB, EXPORT_DIR, BASE
    TMP = C.run_dir('playlist_io')
    EXE_DIR = os.path.join(TMP, 'app')
    LIB = os.path.join(TMP, 'library')
    EXPORT_DIR = os.path.join(TMP, 'export')
    os.makedirs(EXE_DIR, exist_ok=True)
    os.makedirs(LIB, exist_ok=True)
    os.makedirs(EXPORT_DIR, exist_ok=True)
    shutil.copyfile(C.pick_exe(), os.path.join(EXE_DIR, 'mt-cli.exe'))
    app_cfg = os.path.join(EXE_DIR, 'kqk-app.toml')
    with io.open(app_cfg, 'w', encoding='utf-8') as f:
        f.write("last_library_dir = '%s'\n" % C.FIXTURE_LIB.replace('\\', '\\\\'))

    wavs = fixture_wavs()
    ok(len(wavs) == 3, '夹具提供 3 个 WAV 素材（实际 %d）' % len(wavs))
    if len(wavs) != 3:
        return A.report()

    proc = None
    try:
        proc, port = C.start_server(EXE_DIR)
        BASE = 'http://127.0.0.1:%d' % port if port else BASE
        ok(port is not None and wait_up(BASE) is not None, '服务已启动并响应 /api/status')
        if port is None:
            return A.report()
        r = http('/api/library-directory', 'POST', {
            'path': LIB, 'file_scheme': 'flat', 'library_name': '导入导出测试库',
        })
        ok(bool(r.get('ok')), '临时曲库初始化成功')
        for i, src in enumerate(wavs):
            shutil.copyfile(src, os.path.join(LIB, 'track%d.wav' % i))
        http('/api/scan', 'POST')
        tracks = http('/api/tracks').get('tracks') or []
        ok(len(tracks) == 3, '临时曲库入库 3 首（实际 %d）' % len(tracks))
        if len(tracks) != 3:
            return A.report()

        # ---------- 导出 ----------
        # 名称刻意含 Windows 非法字符，验证文件名安全化。
        made = http('/api/playlists', 'POST', {'name': '导出/测试:列表'})
        ok(bool(made.get('ok')), '新建待导出播放列表')
        pid = made.get('id')
        for t in tracks:
            http('/api/playlists/%s/add' % pid, 'POST', {'local_id': t['local_id']})

        # n=1 也绝不能把导出内容截断成 1 首。
        http('/api/ui-prefs', 'POST', {'auto_playlist_count': 1})
        exp = http('/api/playlists/%s/export' % pid, 'POST', {'dir': EXPORT_DIR})
        ok(bool(exp.get('ok')), '导出成功（%s）' % exp.get('error', ''))
        ok(exp.get('track_count') == 3, '导出的是列表全量 3 首，不受 n=1 影响（实际 %s）'
           % exp.get('track_count'))
        files = exp.get('files') or []
        ok(len(files) == 2, '一次产出 2 个文件（.json + .m3u8）')
        ok(any(f.endswith('.json') for f in files) and any(f.endswith('.m3u8') for f in files),
           '文件后缀分别为 .json 与 .m3u8')
        ok(all(os.path.isfile(f) for f in files), '两个文件都真实落盘')
        ok(all(':' not in os.path.basename(f) and '/' not in os.path.basename(f)
               for f in files), '列表名里的 / 与 : 未进入文件名（无路径穿越）')

        json_file = next(f for f in files if f.endswith('.json'))
        m3u_file = next(f for f in files if f.endswith('.m3u8'))
        backup = json.loads(io.open(json_file, encoding='utf-8').read())
        ok(backup.get('format') == 'kqk-playlist' and backup.get('version') == 1,
           'JSON 备份带格式标识与版本')
        ok(len(backup.get('tracks') or []) == 3, 'JSON 备份含 3 首（无损全量）')
        bt = (backup.get('tracks') or [{}])[0]
        ok(all(k in bt for k in ('position', 'local_id', 'name', 'file_path', 'imported_at')),
           'JSON 备份含 local_id / position / 时间等无损字段')
        ok(not os.path.isabs(bt.get('file_path', '').replace('\\', '/')),
           'JSON 里也写相对曲库根路径')

        m3u = io.open(m3u_file, encoding='utf-8').read()
        ok(m3u.startswith('#EXTM3U'), 'M3U8 以 #EXTM3U 开头（UTF-8 Extended M3U，非 HLS）')
        ok('#EXTINF:' in m3u and '#PLAYLIST:' in m3u, 'M3U8 含 #EXTINF 与列表名')
        paths = [l.strip() for l in m3u.splitlines() if l.strip() and not l.startswith('#')]
        ok(len(paths) == 3, 'M3U8 含 3 行曲目路径（实际 %d）' % len(paths))
        ok(all(not os.path.isabs(p) for p in paths), 'M3U8 路径均为相对路径')

        # ---------- 导入：M3U8 / JSON ----------
        imp1 = http('/api/playlist-import', 'POST', {'file': m3u_file})
        ok(bool(imp1.get('ok')), '导入 M3U8 成功（%s）' % imp1.get('error', ''))
        ok(imp1.get('matched') == 3, 'M3U8 相对路径匹配回 3 首（实际 %s）' % imp1.get('matched'))
        ok(not (imp1.get('report') or {}).get('unmatched'), 'M3U8 导入无未匹配条目')
        ok('导入' in (imp1.get('name') or '') and imp1.get('name') != '导出/测试:列表',
           '导入列表名来自 #PLAYLIST 且自动避开同名（实际 %s）' % imp1.get('name'))
        pl1 = http('/api/playlists/%s/tracks' % imp1.get('playlist_id')).get('tracks') or []
        ok(len(pl1) == 3, '导入后新建列表确有 3 首')

        imp2 = http('/api/playlist-import', 'POST', {'file': json_file})
        ok(bool(imp2.get('ok')) and imp2.get('matched') == 3,
           '导入内部 JSON 无损备份也匹配回 3 首（实际 %s）' % imp2.get('matched'))

        # ---------- 导入：重复 + 未匹配 + 报告 ----------
        messy = os.path.join(EXPORT_DIR, 'messy.m3u8')
        with io.open(messy, 'w', encoding='utf-8') as f:
            f.write('#EXTM3U\n#PLAYLIST:去重与未匹配测试\n')
            f.write('#EXTINF:1,%s - %s\n%s\n' % (tracks[0].get('artist') or '',
                                                tracks[0].get('name') or '', paths[0]))
            f.write('#EXTINF:1,%s - %s\n%s\n' % (tracks[0].get('artist') or '',
                                                tracks[0].get('name') or '', paths[0]))
            f.write('#EXTINF:1,不存在 - 没有这首歌\n不存在的路径/xxx.flac\n')
            f.write('#EXTINF:1,%s - %s\n%s\n' % (tracks[1].get('artist') or '',
                                                tracks[1].get('name') or '', paths[1]))
        imp3 = http('/api/playlist-import', 'POST', {'file': messy})
        ok(bool(imp3.get('ok')), '导入含重复/未匹配的 M3U8 成功')
        rep = imp3.get('report') or {}
        ok(imp3.get('matched') == 2, '去重后写入 2 首（实际 %s）' % imp3.get('matched'))
        ok(len(rep.get('duplicate_ids') or []) == 1, '报告列出 1 条 local_id 级重复')
        ok(len(rep.get('unmatched') or []) == 1, '报告列出 1 条未匹配（跳过而非建占位）')
        pl3 = http('/api/playlists/%s/tracks' % imp3.get('playlist_id')).get('tracks') or []
        ok(len(pl3) == 2, '未匹配条目确实没有生成占位行')

        # ---------- 导入：CSV（只有曲名 + 艺人，无路径）----------
        csv = os.path.join(EXPORT_DIR, 'netease_style.csv')
        with io.open(csv, 'w', encoding='utf-8') as f:
            f.write('name,artist,duration\n')
            for t in tracks[:2]:
                f.write('%s,%s,%s\n' % (t.get('name') or '', t.get('artist') or '',
                                       (t.get('duration_ms') or 0) // 1000))
        imp4 = http('/api/playlist-import', 'POST', {'file': csv})
        ok(bool(imp4.get('ok')), '导入网易云风格 CSV 成功（%s）' % imp4.get('error', ''))
        ok(imp4.get('matched') == 2, '无路径 CSV 按「曲名+艺人」标签匹配回 2 首（实际 %s）'
           % imp4.get('matched'))

        # ---------- 错误路径 ----------
        bad = http('/api/playlist-import', 'POST', {'file': os.path.join(EXPORT_DIR, 'no.txt')})
        ok(not bad.get('ok'), '不支持的扩展名返回错误')
        miss = http('/api/playlists/999999/export', 'POST', {'dir': EXPORT_DIR})
        ok(not miss.get('ok'), '导出不存在的列表返回真实错误')

    finally:
        C.stop_server(proc)

    return A.report()


if __name__ == '__main__':
    sys.exit(main())
