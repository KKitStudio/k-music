# -*- coding: utf-8 -*-
"""测试：导入时间 / 创建时间——真实 DB 迁移、真实 API、真实前端渲染 + 设置面板开关。

与 run_preview.py 的 mock 断言互补：这里零 mock，用的是夹具曲库里真实的 WAV、
真实的 SQLite（含**旧库自动迁移**）、真实的 mt-cli serve、真实的 dist/frontend。

覆盖：
  1. 旧夹具库自动补上 imported_at / created_at 两列并回填出可用值；
  2. /api/tracks 及专辑 / 歌手 / 播放列表的曲目接口都带这两个字段；
  3. 两个字段是合理的 UNIX 毫秒时间戳，且 imported_at 在重新扫描后**不被改写**；
  4. **导入时间默认显示、创建时间默认不显示**（列表 / 子页面一致）；
  5. 「设置 → 列表 → 歌曲列表显示项」勾选「创建时间」+「应用显示设置」后，
     /api/columns 落盘、列表出现该列、重启服务后仍然显示。
"""
import io
import os
import shutil
import sqlite3
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import common as C

A = C.Asserts('导入时间 / 创建时间 + 创建日期默认隐藏（真实端到端）')
DB = os.path.join(C.FIXTURE_LIB, '.mt', 'library.db')

ASSERT_JS = r"""
<script>
(async () => {
  const R = [];
  const ok = (c, m) => R.push((c ? 'PASS' : 'FAIL') + ' | ' + m);
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  for (let i = 0; i < 80; i++) {
    if (document.querySelectorAll('#track-body tr').length >= 3) break;
    await wait(50);
  }
  await wait(250);
  const headKeys = () => Array.from(document.querySelectorAll('#track-head th[data-col-key]'))
                              .map(t => t.dataset.colKey);
  const labels = Array.from(document.querySelectorAll('#track-head th')).map(t => t.textContent.trim());
  ok(labels.indexOf('导入时间') >= 0, '真实前端：默认表头含「导入时间」');
  ok(labels.indexOf('创建时间') < 0, '真实前端：默认表头不含「创建时间」');
  const keys = headKeys();
  ok(keys.indexOf('imported_at') >= 0, '真实前端：imported_at 参与列宽分配');
  ok(keys.indexOf('created_at') < 0, '真实前端：created_at 不在默认显示列');
  const ii = keys.indexOf('imported_at');
  const rows = Array.from(document.querySelectorAll('#track-body tr'));
  ok(rows.length >= 3, '真实前端：渲染出 3 行真实曲目（' + rows.length + '）');
  let allFilled = ii >= 0, bad = false;
  rows.forEach((r) => {
    const a = r.children[ii + 1];
    const ta = a ? a.textContent.trim() : '';
    if (!ta) allFilled = false;
    if (ta === '—' || ta.indexOf('暂无') >= 0) bad = true;
  });
  ok(allFilled, '真实前端：每行导入时间都有时间文本（夹具已回填）');
  ok(!bad, '真实前端：无「—」「暂无」占位内容');

  // 设置面板：创建时间默认未勾选；勾选并「应用显示设置」后才出现。
  document.getElementById('btn-settings').click();
  await wait(200);
  const creLi = document.querySelector('#col-config-list li[data-key="created_at"]');
  const impLi = document.querySelector('#col-config-list li[data-key="imported_at"]');
  ok(!!creLi, '设置→列表：存在「创建时间」显示项');
  const cb = creLi && creLi.querySelector('input[data-act="vis"]');
  const cbImp = impLi && impLi.querySelector('input[data-act="vis"]');
  ok(!!cb && !cb.checked, '创建时间默认未勾选');
  ok(!!cbImp && cbImp.checked, '导入时间默认已勾选');
  if (cb) { cb.checked = true; cb.dispatchEvent(new Event('change', { bubbles: true })); }
  document.getElementById('set-apply-columns').click();
  await wait(600);
  const keys2 = headKeys();
  ok(keys2.indexOf('created_at') >= 0, '勾选并应用后：主列表出现「创建时间」列');
  const ci = keys2.indexOf('created_at');
  const rows2 = Array.from(document.querySelectorAll('#track-body tr'));
  let creFilled = ci >= 0 && rows2.length >= 3, creBad = false;
  rows2.forEach((r) => {
    const b = r.children[ci + 1];
    const tb = b ? b.textContent.trim() : '';
    if (!tb) creFilled = false;
    if (tb === '—' || tb.indexOf('暂无') >= 0) creBad = true;
  });
  ok(creFilled, '启用后：每行创建时间都有真实时间文本');
  ok(!creBad, '启用后：无「—」「暂无」占位内容');
  const pre = document.createElement('pre');
  pre.id = 'ASSERT';
  pre.textContent = R.join('\n');
  document.body.appendChild(pre);
})();
</script>
"""

# 重启服务后再加载一次前端：验证「勾选创建时间」确实写进了 app 级配置文件。
ASSERT_JS_RESTART = r"""
<script>
(async () => {
  const R = [];
  const ok = (c, m) => R.push((c ? 'PASS' : 'FAIL') + ' | ' + m);
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  for (let i = 0; i < 80; i++) {
    if (document.querySelectorAll('#track-body tr').length >= 3) break;
    await wait(50);
  }
  await wait(250);
  const keys = Array.from(document.querySelectorAll('#track-head th[data-col-key]'))
                    .map(t => t.dataset.colKey);
  ok(keys.indexOf('imported_at') >= 0, '重启后：导入时间列仍显示');
  ok(keys.indexOf('created_at') >= 0, '重启后：创建时间列仍显示（配置已持久化）');
  const pre = document.createElement('pre');
  pre.id = 'ASSERT';
  pre.textContent = R.join('\n');
  document.body.appendChild(pre);
})();
</script>
"""


def main():
    if not C.ensure_fixture():
        A.ok(False, '夹具曲库不可用')
        return A.report()

    run_dir = C.run_dir('time_columns')
    app_dir = os.path.join(run_dir, 'app')
    front_dir = os.path.join(app_dir, 'frontend')
    os.makedirs(app_dir, exist_ok=True)
    exe = C.pick_exe()
    if not exe:
        A.ok(False, '未找到 mt-cli.exe（先 cargo build）')
        return A.report()
    shutil.copyfile(exe, os.path.join(app_dir, 'mt-cli.exe'))
    C.build_frontend_copy(front_dir, inject_js=ASSERT_JS)
    app_cfg = os.path.join(app_dir, 'kqk-app.toml')
    # 故意不写 [ui]：验证全新配置的默认值（导入时间显示 / 创建时间隐藏）。
    # 手写注释用来验证 /api/columns 保存时走「[ui] 段就地更新」，不会整份重写配置。
    with io.open(app_cfg, 'w', encoding='utf-8') as f:
        f.write("# 用户手写注释：保存显示项后应保留\n"
                "last_library_dir = '%s'\n" % C.FIXTURE_LIB.replace('\\', '\\\\'))

    proc = None
    try:
        # ---------- 1) 旧库自动迁移 + 默认显示项 ----------
        proc, port = C.start_server(app_dir)
        A.ok(port is not None, '真实服务已启动（端口 %s）' % port)
        if not port:
            return A.report()
        base = 'http://127.0.0.1:%d' % port
        status = C.http_get(base, '/api/status')
        cols = status.get('track_columns') or []
        A.ok('imported_at' in cols, '全新配置默认显示「导入时间」（%s）' % cols)
        A.ok('created_at' not in cols, '全新配置默认不显示「创建时间」（%s）' % cols)
        A.ok(status.get('track_columns_saved') is False,
             '全新配置 track_columns_saved=false（旧配置迁移标记）')

        tracks = C.wait_for_tracks(base, want=3)
        A.ok(len(tracks) >= 3, '后端已索引 3 首真实音频（%d）' % len(tracks))

        # 等启动扫描跑完（它会把迁移回填的近似值校正成真实文件创建时间）。
        time.sleep(1.0)
        conn = sqlite3.connect(DB)
        dbcols = [r[1] for r in conn.execute("PRAGMA table_info('download_records')")]
        A.ok('imported_at' in dbcols and 'created_at' in dbcols,
             '旧夹具库自动补上 imported_at / created_at 两列（%s）' % dbcols)
        rows = list(conn.execute(
            "SELECT local_id, imported_at, created_at FROM download_records "
            "WHERE status='downloaded' AND file_path IS NOT NULL ORDER BY local_id"
        ))
        conn.close()
        A.ok(len(rows) >= 3, 'DB 里有 3 条已入库记录（%d）' % len(rows))
        A.ok(all(r[1] is not None and r[1] > 0 for r in rows),
             '每条记录的导入时间都已回填（%s）' % [r[1] for r in rows])
        A.ok(all(r[2] is not None and r[2] > 0 for r in rows),
             '每条记录的创建时间都已回填（%s）' % [r[2] for r in rows])
        now_ms = int(time.time() * 1000)
        A.ok(all(0 < r[1] <= now_ms + 86400000 for r in rows),
             '导入时间是合理的 UNIX 毫秒时间戳（非秒 / 非未来时间）')
        A.ok(all(0 < r[2] <= now_ms + 86400000 for r in rows),
             '创建时间是合理的 UNIX 毫秒时间戳（非秒 / 非未来时间）')
        before_imported = dict((r[0], r[1]) for r in rows)

        # ---------- 2) 各 API 都带两个字段（与显示/隐藏无关） ----------
        A.ok(all('imported_at' in t and 'created_at' in t for t in tracks),
             '/api/tracks 返回 imported_at / created_at')
        for path, key in (('/api/albums/tracks?name=Ignite', 'tracks'),
                          ('/api/artists/tracks?name=Iselin%20Solheim', 'tracks')):
            got = C.http_get(base, path).get(key) or []
            A.ok(len(got) >= 1 and 'imported_at' in got[0] and 'created_at' in got[0],
                 '%s 的曲目也带两个时间字段' % path.split('?')[0])
        pl = C.http_get(base, '/api/playlists').get('playlists') or []
        if pl:
            pid = pl[0]['id']
            items = C.http_get(base, '/api/playlists/%d/tracks' % pid).get('tracks') or []
            A.ok(not items or ('imported_at' in items[0] and 'created_at' in items[0]),
                 '播放列表曲目也带两个时间字段')

        # ---------- 3) 真实前端：默认隐藏 + 设置面板开启 ----------
        out_file = os.path.join(run_dir, 'dom.txt')
        dom = C.edge_dump_dom(None, out_file, window='1080,645', budget=30000, url=base + '/')
        C.merge_assert_results(A, C.extract_assert_block(dom))

        # ---------- 4) 开启「创建时间」后确实落盘 ----------
        with io.open(app_cfg, 'r', encoding='utf-8') as f:
            cfg_text = f.read()
        A.ok('track_columns_saved = true' in cfg_text,
             '应用显示设置后 app 配置写入 track_columns_saved = true')
        A.ok('created_at' in cfg_text and 'track_columns' in cfg_text,
             '应用显示设置后 app 配置的 track_columns 含 created_at')
        A.ok('# 用户手写注释：保存显示项后应保留' in cfg_text,
             '/api/columns 保存后用户手写注释未被整份重写抹掉')
        api_cols = C.http_get(base, '/api/columns').get('columns') or []
        A.ok('created_at' in api_cols and 'imported_at' in api_cols,
             'GET /api/columns 读回两列（%s）' % api_cols)

        # ---------- 5) 重启 / 重新扫描：时间不变，显示项仍持久化 ----------
        C.stop_server(proc)
        proc = None
        proc, port2 = C.start_server(app_dir)
        base2 = 'http://127.0.0.1:%d' % port2
        C.wait_for_tracks(base2, want=3)
        time.sleep(1.0)
        st2 = C.http_get(base2, '/api/status')
        A.ok('created_at' in (st2.get('track_columns') or []),
             '重启后 /api/status 仍含 created_at（配置文件为权威源）')
        A.ok(st2.get('track_columns_saved') is True,
             '重启后 track_columns_saved 仍为 true（不再自动回填覆盖用户选择）')
        conn2 = sqlite3.connect(DB)
        after = dict(conn2.execute(
            "SELECT local_id, imported_at FROM download_records "
            "WHERE status='downloaded' AND file_path IS NOT NULL"
        ))
        conn2.close()
        A.ok(all(after.get(k) == v for k, v in before_imported.items()),
             '重新扫描后 imported_at 保持不变（首次入库时间不被改写）')

        # 换一份只带「重启后」断言的前端，再加载一次真实页面。
        C.build_frontend_copy(front_dir, inject_js=ASSERT_JS_RESTART)
        out_file2 = os.path.join(run_dir, 'dom_restart.txt')
        dom2 = C.edge_dump_dom(None, out_file2, window='1080,645', budget=30000, url=base2 + '/')
        C.merge_assert_results(A, C.extract_assert_block(dom2))
    finally:
        C.stop_server(proc)

    return A.report()


if __name__ == '__main__':
    sys.exit(main())
