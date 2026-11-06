# -*- coding: utf-8 -*-
"""测试：歌曲右键菜单的后端能力（FR-21，真实 mt-cli serve + 真实 SQLite，零 mock）。

覆盖：
  1. 删除歌曲 = 文件移入 <曲库>/.mt/trash/ + 索引与所有播放列表引用清除
  2. 重新扫描不会把回收站里的文件重新入库（scan_dir 必须跳过 .mt）
  3. 回收站同名冲突自动避让（track0.wav → track0_1.wav）
  4. 曲库外路径拒绝删除 / 拒绝定位，且不产生任何副作用
  5. 不存在的 id / 非法 local_id 返回真实错误

为避免污染持久化夹具，本用例在 demo/.run/ 下另建独立临时曲库，音频素材从夹具复制。
"""
import io
import json
import os
import shutil
import sqlite3
import sys
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import common as C

A = C.Asserts('歌曲右键菜单 · 删除/回收站/越界（FR-21）')

TMP = ''
EXE_DIR = ''
LIB = ''
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


def tracks():
    return http('/api/tracks').get('tracks') or []


def pl_tracks(pid):
    return http('/api/playlists/%d/tracks' % pid).get('tracks') or []


def fixture_wavs():
    out = []
    for root, _dirs, files in os.walk(C.FIXTURE_LIB):
        if '.mt' in root.split(os.sep):
            continue
        for f in files:
            if f.lower().endswith('.wav'):
                out.append(os.path.join(root, f))
    return sorted(out)


def db_exec(sql, args=()):
    path = os.path.join(LIB, '.mt', 'library.db')
    conn = sqlite3.connect(path)
    try:
        cur = conn.execute(sql, args)
        conn.commit()
        return cur.fetchall()
    finally:
        conn.close()


def main():
    if not C.ensure_fixture():
        ok(False, '夹具曲库不可用')
        return A.report()

    global TMP, EXE_DIR, LIB, BASE
    TMP = C.run_dir('track_context_menu')
    EXE_DIR = os.path.join(TMP, 'app')
    LIB = os.path.join(TMP, 'library')
    os.makedirs(EXE_DIR, exist_ok=True)
    os.makedirs(LIB, exist_ok=True)
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
            'path': LIB, 'file_scheme': 'flat', 'library_name': '右键菜单测试库',
        })
        ok(bool(r.get('ok')), '临时曲库初始化成功（%s）' % r.get('error', ''))
        for i, src in enumerate(wavs):
            shutil.copyfile(src, os.path.join(LIB, 'track%d.wav' % i))
        ok(bool(http('/api/scan', 'POST').get('ok')), '扫描临时曲库成功')
        before = tracks()
        ok(len(before) == 3, '临时曲库入库 3 首（实际 %d）' % len(before))
        if len(before) != 3:
            return A.report()
        ids = [t['local_id'] for t in before]
        first_path = next(t['file_path'] for t in before if t['local_id'] == ids[0])

        # ---------- 1) 建两个列表，让被删曲目同时出现在两个列表里 ----------
        p1 = http('/api/playlists', 'POST', {'name': '列表一'})
        p2 = http('/api/playlists', 'POST', {'name': '列表二'})
        ok(bool(p1.get('ok')) and bool(p2.get('ok')), '新建两个播放列表')
        pid1, pid2 = p1.get('id'), p2.get('id')
        for tid in ids:
            http('/api/playlists/%d/add' % pid1, 'POST', {'local_id': tid})
        http('/api/playlists/%d/add' % pid2, 'POST', {'local_id': ids[0]})
        ok(len(pl_tracks(pid1)) == 3 and len(pl_tracks(pid2)) == 1, '删除前两个列表各含 3 / 1 首')

        # ---------- 2) 删除歌曲：文件进回收站 + 索引与列表引用清除 ----------
        r = http('/api/tracks/delete', 'POST', {'local_id': ids[0]})
        ok(bool(r.get('ok')), '删除歌曲成功（%s）' % r.get('error', ''))
        trash_dir = os.path.join(LIB, '.mt', 'trash')
        trashed = r.get('trashed') or ''
        ok(os.path.isdir(trash_dir), '曲库回收站 .mt/trash/ 已创建')
        ok(bool(trashed) and os.path.isfile(trashed), '文件已移入回收站且存在：%s' % trashed)
        ok(bool(trashed) and os.path.normcase(os.path.dirname(trashed)) ==
           os.path.normcase(trash_dir), '回收站路径落在 <曲库>/.mt/trash/ 下')
        ok(not os.path.exists(first_path), '原位置文件已不存在')
        ok(r.get('playlists_affected') == 2, '报告清掉 2 个列表引用（实际 %s）'
           % r.get('playlists_affected'))
        after = tracks()
        ok(len(after) == 2 and all(t['local_id'] != ids[0] for t in after),
           '曲库剩 2 首且不含被删曲目（实际 %d）' % len(after))
        ok(len(pl_tracks(pid1)) == 2 and len(pl_tracks(pid2)) == 0,
           '两个列表分别剩 2 / 0 首（引用被级联清除）')
        refs = db_exec('SELECT COUNT(*) FROM playlist_items WHERE local_id=?', (ids[0],))
        ok(refs[0][0] == 0, 'DB 里没有指向被删曲目的 playlist_items 孤儿')
        rows = db_exec('SELECT COUNT(*) FROM download_records WHERE local_id=?', (ids[0],))
        ok(rows[0][0] == 0, 'DB 里没有该曲目的 download_records 记录')

        # ---------- 3) 关键回归：重扫不得把回收站里的文件重新入库 ----------
        ok(bool(http('/api/scan', 'POST').get('ok')), '删除后重新扫描成功')
        rescan = tracks()
        ok(len(rescan) == 2, '重扫后仍是 2 首（.mt 未被扫描，实际 %d）' % len(rescan))
        ok(all('.mt' not in t['file_path'].replace('\\', '/').split('/')
               for t in rescan), '重扫后没有任何曲目路径指向 .mt/ 回收站')
        ok(os.path.isfile(trashed), '回收站文件在重扫后仍然存在')

        # ---------- 4) 同名冲突自动避让 ----------
        shutil.copyfile(wavs[0], os.path.join(LIB, 'track0.wav'))
        ok(bool(http('/api/scan', 'POST').get('ok')), '放回同名文件后重新扫描成功')
        fresh = [t for t in tracks()
                 if t['local_id'] != ids[0] and
                 os.path.normcase(os.path.basename(t['file_path'])) == 'track0.wav']
        ok(len(fresh) == 1, '同名文件作为新曲目入库（实际 %d 条）' % len(fresh))
        if fresh:
            r2 = http('/api/tracks/delete', 'POST', {'local_id': fresh[0]['local_id']})
            ok(bool(r2.get('ok')), '第二次删除同名文件成功（%s）' % r2.get('error', ''))
            second = r2.get('trashed') or ''
            ok(os.path.normcase(os.path.basename(second)) == 'track0_1.wav',
               '同名自动避让为 track0_1.wav（实际 %s）' % os.path.basename(second))
            ok(os.path.isfile(trashed) and os.path.isfile(second), '两份文件都保留在回收站')

        # ---------- 5) 不存在的 id / 非法参数 ----------
        ok(not http('/api/tracks/delete', 'POST', {'local_id': 999999}).get('ok'),
           '删除不存在的 id 返回真实错误')
        ok(not http('/api/tracks/delete', 'POST', {'local_id': 0}).get('ok'),
           'local_id=0 被拒绝')
        ok(not http('/api/tracks/reveal', 'POST', {'local_id': 999999}).get('ok'),
           '定位不存在的 id 返回真实错误')
        ok(not http('/api/tracks/reveal', 'POST', {'local_id': 0}).get('ok'),
           '定位 local_id=0 被拒绝')

        # ---------- 6) 路径越界：不得碰曲库外文件、不得删索引 ----------
        outside = os.path.join(TMP, 'outside.wav')
        shutil.copyfile(wavs[0], outside)
        db_exec('UPDATE download_records SET file_path=? WHERE local_id=?', (outside, ids[1]))
        r3 = http('/api/tracks/delete', 'POST', {'local_id': ids[1]})
        ok(not r3.get('ok'), '曲库外路径的删除被拒绝')
        ok('不在当前曲库目录内' in (r3.get('error') or ''), '错误信息说明越界原因（%s）'
           % r3.get('error', ''))
        ok(os.path.isfile(outside), '曲库外文件未被移动')
        still = db_exec('SELECT COUNT(*) FROM download_records WHERE local_id=?', (ids[1],))
        ok(still[0][0] == 1, '越界删除失败后索引保持不变')
        r4 = http('/api/tracks/reveal', 'POST', {'local_id': ids[1]})
        ok(not r4.get('ok') and '不在当前曲库目录内' in (r4.get('error') or ''),
           '曲库外路径的定位也被拒绝（不会打开资源管理器）')

    finally:
        C.stop_server(proc)

    return A.report()


if __name__ == '__main__':
    sys.exit(main())
