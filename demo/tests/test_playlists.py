# -*- coding: utf-8 -*-
"""测试：播放列表真实端到端（批次 0 回归基线）。

覆盖（全部走真实 mt-cli serve + 真实 SQLite，零 mock）：
  1. 建单 → 名称约束（空 / 超长 / 同名大小写不敏感）→ 重命名
  2. 加曲（幂等去重）→ 列表计数与内容一致 → 上移 / 下移重排
  3. P0 孤儿清理：注入一条指向失效文件的曲目记录并加入列表，
     POST /api/scan 触发 prune_missing 后，playlist_items 孤儿必须一起清掉，
     左侧 track_count 与右侧曲目列表同时归零（旧缺陷：只删 download_records）
  4. 移除 / 删除 / 不存在的 id 返回真实错误
  5. 重启服务后播放列表与曲目持久化

为避免污染持久化夹具曲库，本用例在 demo/.run/ 下**另建独立临时曲库**
并通过 /api/library-directory 切换；音频素材从夹具复制（副本可随意改）。
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

A = C.Asserts('播放列表端到端（批次 0）')

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


def pl_list():
    return http('/api/playlists').get('playlists') or []


def pl_tracks(pid):
    return http('/api/playlists/%d/tracks' % pid).get('tracks') or []


def fixture_wavs():
    """从持久化夹具拿 3 个真实 WAV（复制副本，不改动夹具）。"""
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

    global TMP, EXE_DIR, LIB, BASE
    TMP = C.run_dir('playlists')
    EXE_DIR = os.path.join(TMP, 'app')
    LIB = os.path.join(TMP, 'library')
    os.makedirs(EXE_DIR, exist_ok=True)
    os.makedirs(LIB, exist_ok=True)
    shutil.copyfile(C.pick_exe(), os.path.join(EXE_DIR, 'mt-cli.exe'))

    # 启动时先指向健康的夹具曲库；随后通过 API 切换到临时库（避免启动进入初始化向导）。
    app_cfg = os.path.join(EXE_DIR, 'kqk-app.toml')
    fixture_cfg = "last_library_dir = '%s'\n" % C.FIXTURE_LIB.replace('\\', '\\\\')
    with io.open(app_cfg, 'w', encoding='utf-8') as f:
        f.write(fixture_cfg)

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

        # 初始化独立临时曲库（file_scheme=flat，曲目不移动层级）。
        r = http('/api/library-directory', 'POST', {
            'path': LIB, 'file_scheme': 'flat', 'library_name': '播放列表测试库',
        })
        ok(bool(r.get('ok')), '临时曲库初始化成功（%s）' % r.get('error', ''))

        for i, src in enumerate(wavs):
            shutil.copyfile(src, os.path.join(LIB, 'track%d.wav' % i))
        scan = http('/api/scan', 'POST')
        ok(bool(scan.get('ok')), '扫描临时曲库成功')
        tracks = http('/api/tracks').get('tracks') or []
        ok(len(tracks) == 3, '临时曲库入库 3 首（实际 %d）' % len(tracks))
        if len(tracks) != 3:
            return A.report()
        ids = [t['local_id'] for t in tracks]

        # ---------- 1) 名称约束 ----------
        for bad, label in [('', '空名称'), ('x' * 61, '超 60 字符')]:
            rr = http('/api/playlists', 'POST', {'name': bad})
            ok(not rr.get('ok'), '拒绝%s（%s）' % (label, rr.get('error')))
        first = http('/api/playlists', 'POST', {'name': '我的歌单'})
        ok(bool(first.get('ok')), '新建播放列表成功')
        pid = first.get('id')
        dup = http('/api/playlists', 'POST', {'name': '我的歌单'})
        ok(not dup.get('ok'), '拒绝同名播放列表')
        dup2 = http('/api/playlists', 'POST', {'name': 'WO DE GE DAN'})
        ok(bool(dup2.get('ok')), '不同名称可创建（对照组）')
        http('/api/playlists/%d' % dup2.get('id'), 'DELETE')

        # ---------- 2) 加曲 / 幂等 / 计数 / 重排 ----------
        for tid in ids:
            rr = http('/api/playlists/%d/add' % pid, 'POST', {'local_id': tid})
            ok(bool(rr.get('ok')), '加入曲目 %d' % tid)
        http('/api/playlists/%d/add' % pid, 'POST', {'local_id': ids[0]})  # 重复加曲
        ok(len(pl_tracks(pid)) == 3, '重复加曲被主键去重，列表仍为 3 首')
        pl = next((p for p in pl_list() if p['id'] == pid), None)
        ok(pl is not None and pl['track_count'] == 3, '左侧 track_count = 3（实际 %s）'
           % (pl and pl['track_count']))
        order = [t['local_id'] for t in pl_tracks(pid)]
        last = order[-1]
        rr = http('/api/playlists/%d/move' % pid, 'POST', {'local_id': last, 'delta': -1})
        ok(rr.get('moved') is True, '末首上移成功')
        order2 = [t['local_id'] for t in pl_tracks(pid)]
        ok(order2[-2] == last, '重排后顺序确实改变')
        rr = http('/api/playlists/%d/move' % pid, 'POST', {'local_id': order2[0], 'delta': -1})
        ok(rr.get('moved') is False, '首项上移返回 moved=false（不报错）')

        rr = http('/api/playlists/%d/rename' % pid, 'POST', {'name': '重命名后的歌单'})
        ok(bool(rr.get('ok')), '重命名成功')
        pl = next((p for p in pl_list() if p['id'] == pid), None)
        ok(pl is not None and pl['name'] == '重命名后的歌单', '重命名已落库')

        # ---------- 3) P0 孤儿清理 ----------
        db_path = os.path.join(LIB, '.mt', 'library.db')
        conn = sqlite3.connect(db_path)
        conn.execute(
            "INSERT INTO download_records(local_id,source_kind,status,file_path,name,file_size) "
            "VALUES(?,?,?,?,?,?)",
            (999001, 'local_scan', 'downloaded',
             os.path.join(LIB, 'definitely-missing-file.flac'), '失效曲目', 1000),
        )
        conn.commit()
        conn.close()
        rr = http('/api/playlists/%d/add' % pid, 'POST', {'local_id': 999001})
        ok(bool(rr.get('ok')), '把失效曲目加入播放列表')
        pl = next((p for p in pl_list() if p['id'] == pid), None)
        ok(pl is not None and pl['track_count'] == 4, '清理前 track_count = 4（实际 %s）'
           % (pl and pl['track_count']))
        # 旧缺陷复现点：JOIN 取不到失效曲目，但 COUNT(*) 仍计入 → 4 vs 3。
        ok(len(pl_tracks(pid)) == 4, '清理前 JOIN 能取到 4 首（失效记录仍在 DB）')

        http('/api/scan', 'POST')  # 触发 prune_missing
        pl = next((p for p in pl_list() if p['id'] == pid), None)
        joined = pl_tracks(pid)
        ok(pl is not None and pl['track_count'] == 3,
           '扫描后 playlist_items 孤儿被清理，track_count 回到 3（实际 %s）'
           % (pl and pl['track_count']))
        ok(len(joined) == 3, '扫描后曲目列表也是 3 首（计数与内容一致）')
        ok(all(t['local_id'] != 999001 for t in joined), '失效曲目不再出现在列表里')

        # ---------- 4) 移除 / 删除 / 错误不再吞 ----------
        rr = http('/api/playlists/%d/remove' % pid, 'POST', {'local_id': ids[0]})
        ok(bool(rr.get('ok')), '移除曲目成功')
        ok(len(pl_tracks(pid)) == 2, '移除后剩 2 首')
        rr = http('/api/playlists/%d/remove' % pid, 'POST', {'local_id': ids[0]})
        ok(not rr.get('ok'), '重复移除返回真实错误（不是静默 ok）')
        rr = http('/api/playlists/999999', 'DELETE')
        ok(not rr.get('ok'), '删除不存在的列表返回真实错误')
        rr = http('/api/playlists', 'GET')
        ok(rr.get('ok') is True, 'GET /api/playlists 显式返回 ok=true')

        # ---------- 5) 重启持久化 ----------
        # 让重启后的 app 配置直接指向临时曲库（API 切换不写 app 级 last_library_dir）。
        with io.open(app_cfg, 'w', encoding='utf-8') as f:
            f.write("last_library_dir = '%s'\n" % LIB.replace('\\', '\\\\'))
        C.stop_server(proc)
        proc = None
        proc, port2 = C.start_server(EXE_DIR)
        BASE = 'http://127.0.0.1:%d' % port2 if port2 else BASE
        ok(port2 is not None and wait_up(BASE) is not None, '重启后服务正常')
        pls2 = pl_list()
        pl2 = next((p for p in pls2 if p['id'] == pid), None)
        ok(pl2 is not None and pl2['name'] == '重命名后的歌单', '重启后播放列表仍在且名称正确')
        ok(pl2 is not None and pl2['track_count'] == 2, '重启后曲目数仍为 2')
        ok(len(pl_tracks(pid)) == 2, '重启后曲目列表仍为 2 首')

    finally:
        C.stop_server(proc)

    return A.report()


if __name__ == '__main__':
    sys.exit(main())
