# -*- coding: utf-8 -*-
"""测试：详情区公共组件（播放列表 / 专辑 / 歌手 三页 + 全部曲库页）。

覆盖两轮需求：
  A. 跨页详情隔离：曲库页选中的歌曲**不得**出现在专辑 / 歌手页的详情区。
  B. 三页一致的隐藏契约（公共组件 DetailRegion）：
     · 数据未选中          → 详情区整体隐藏（不渲染任何占位内容）；
     · 封面缺失/加载失败    → 封面控件隐藏（不渲染「无封面」字样）；
     · 名称为空            → 不渲染名称行；
     · 字段值为空          → 该字段行不渲染（不渲染「—」占位）；
     · 三页行为完全一致。

流程：复用持久化夹具曲库（demo/fixtures/library）→ 真实 mt-cli serve
     → 无头 Edge 打开真实前端（仅额外注入断言脚本）→ 逐项断言。
"""
import io
import os
import shutil
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import common as C

A = C.Asserts('详情区公共组件 / 跨页详情隔离')

# 断言脚本注入到真实前端页面内执行。
ASSERT_JS = r"""
<script>
(async () => {
  const R = [];
  const ok = (c, m) => R.push((c ? 'PASS' : 'FAIL') + ' | ' + m);
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  const $ = (id) => document.getElementById(id);
  const txt = (id) => { const e = $(id); return e ? e.textContent : ''; };
  const has = (id, s) => txt(id).indexOf(s) >= 0;
  const visible = (id) => {
    const e = $(id);
    if (!e) return false;
    if (e.hidden) return false;
    let p = e.parentElement;            // 祖先 hidden 也要算不可见
    while (p) { if (p.hidden) return false; p = p.parentElement; }
    const cs = window.getComputedStyle(e);
    return cs.display !== 'none' && cs.visibility !== 'hidden';
  };
  const ph = (id) => !!($(id) && $(id).querySelector('.placeholder'));

  for (let i = 0; i < 160; i++) {
    if (document.querySelectorAll('#track-body tr').length) break;
    await wait(50);
  }
  await wait(700);

  // 曲库页实际选中的曲名（动态取，避免扫描顺序差异导致断言失真）
  const libSel = (state.viewSelection && state.viewSelection.library) || null;
  const LIB_PICKED = (libSel && libSel.track && libSel.track.name)
    ? libSel.track.name
    : ((state.current && state.current.name) ? state.current.name : '');

  // ===== A. 跨页详情隔离 =====
  ok(!!LIB_PICKED, 'A0 曲库页：已自动选中首曲（' + LIB_PICKED + '）');
  ok(has('detail-fields', LIB_PICKED), 'A1 曲库页：右侧详情栏显示曲库页选中的曲目');

  document.getElementById('tab-albums').click(); await wait(800);
  ok(!has('al-detail-fields', LIB_PICKED), 'A2 专辑页(无选中)：内联详情条不含曲库页曲名');
  ok(!has('detail-fields', LIB_PICKED), 'A2 专辑页(无选中)：右侧详情栏不含曲库页曲名');
  ok(!$('al-detail-fields').querySelector('b'), 'A2 专辑页(无选中)：无曲目粗体标题');

  document.getElementById('tab-artists').click(); await wait(800);
  ok(!has('ar-detail-fields', LIB_PICKED), 'A3 歌手页(无选中)：内联详情条不含曲库页曲名');
  ok(!has('detail-fields', LIB_PICKED), 'A3 歌手页(无选中)：右侧详情栏不含曲库页曲名');
  ok(!$('ar-detail-fields').querySelector('b'), 'A3 歌手页(无选中)：无曲目粗体标题');

  // ===== B. 三页一致的隐藏契约 =====
  const pages = [
    { tab: 'albums',    key: 'al', label: '专辑' },
    { tab: 'artists',   key: 'ar', label: '歌手' },
    { tab: 'playlists', key: 'pl', label: '播放列表' },
  ];
  for (const p of pages) {
    document.getElementById('tab-' + p.tab).click(); await wait(800);
    ok(visible(p.key + '-detail') === false,
       'B1 ' + p.label + '页(未选中)：内联详情条隐藏');
    ok(visible(p.key + '-detail-cover') === false,
       'B1 ' + p.label + '页(未选中)：封面控件隐藏');
    ok(visible(p.key + '-detail-fields') === false,
       'B1 ' + p.label + '页(未选中)：字段区隐藏');
    ok(txt(p.key + '-detail-fields').trim() === '',
       'B1 ' + p.label + '页(未选中)：字段区无任何文本（无占位内容）');
    ok(txt(p.key + '-detail-cover').trim() === '',
       'B1 ' + p.label + '页(未选中)：封面控件无「无封面」字样');
    ok(!ph(p.key + '-detail-fields'),
       'B1 ' + p.label + '页(未选中)：无 placeholder 占位元素');
    ok(txt(p.key + '-detail').indexOf('无封面') < 0,
       'B1 ' + p.label + '页(未选中)：详情区不含「无封面」');
  }

  // B-2 选中专辑 → 显示分组信息（名称 / 类型 / 曲目数）
  document.getElementById('tab-albums').click(); await wait(600);
  const alItems = Array.from(document.querySelectorAll('#al-list li'));
  const ign = alItems.find((li) => li.dataset.name === 'Ignite');
  ok(!!ign, 'B2 专辑页：存在「Ignite」专辑');
  if (ign) {
    ign.click(); await wait(900);
    ok(visible('al-detail') === true, 'B2 选中专辑后：内联详情条显示');
    ok(has('al-detail-fields', 'Ignite'), 'B2 选中专辑后：显示专辑名');
    ok(has('al-detail-fields', '类型：专辑'), 'B2 选中专辑后：显示「类型：专辑」');
    ok(has('al-detail-fields', '曲目数：1'), 'B2 选中专辑后：显示「曲目数：1」');
    ok(!has('al-detail-fields', LIB_PICKED), 'B2 选中专辑后：不含曲库页曲名');
    ok(txt('al-detail-fields').indexOf('无封面') < 0, 'B2 选中专辑后：不含「无封面」字样');
  }

  // B-3 专辑页选中歌曲 → 切换为曲目详情
  const alRows = document.querySelectorAll('#al-content tr[data-id]');
  ok(alRows.length === 1, 'B3 专辑「Ignite」下有 1 首歌（' + alRows.length + '）');
  if (alRows.length) {
    alRows[0].click(); await wait(900);
    ok(has('al-detail-fields', 'Spectre'), 'B3 专辑页选中歌曲后：显示该曲名');
    ok(!has('al-detail-fields', LIB_PICKED), 'B3 专辑页选中歌曲后：不含曲库页曲名');
    ok(txt('al-detail-fields').indexOf('—') < 0,
       'B3 专辑页选中歌曲后：空字段不渲染为「—」占位');
  }

  // B-4 歌手页：选中 → 分组信息
  document.getElementById('tab-artists').click(); await wait(700);
  const arItems = Array.from(document.querySelectorAll('#ar-list li'));
  const solo = arItems.find((li) => li.dataset.name === 'Iselin Solheim');
  ok(!!solo, 'B4 歌手页：存在「Iselin Solheim」歌手');
  if (solo) {
    solo.click(); await wait(900);
    ok(visible('ar-detail') === true, 'B4 选中歌手后：内联详情条显示');
    ok(has('ar-detail-fields', 'Iselin Solheim'), 'B4 选中歌手后：显示歌手名');
    ok(has('ar-detail-fields', '类型：歌手'), 'B4 选中歌手后：显示「类型：歌手」');
    ok(has('ar-detail-fields', '曲目数：1'), 'B4 选中歌手后：显示「曲目数：1」');
    ok(!has('ar-detail-fields', LIB_PICKED), 'B4 选中歌手后：不含曲库页曲名');
  }

  // B-5 播放列表页：新建空播放列表 → 名称有值、曲目数为 0，
  //     详情区应显示名称 + 类型 + 曲目数 0，而不是「暂无信息」。
  document.getElementById('tab-playlists').click(); await wait(700);
  const created = await (async () => {
    const r = await fetch('/api/playlists', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'DemoEmptyList' }),
    });
    return r.json();
  })();
  ok(created && created.ok, 'B5 播放列表页：新建播放列表成功');
  if (created && created.ok) {
    // 走前端的「刷新列表 + 选中」路径（等价于点「＋ 新建」后的自动选中）。
    await loadPlaylists();
    await selectPlaylist(created.id);
    await wait(800);
    ok(visible('pl-detail') === true, 'B5 选中播放列表后：内联详情条显示');
    ok(has('pl-detail-fields', 'DemoEmptyList'), 'B5 选中播放列表后：显示列表名');
    ok(has('pl-detail-fields', '类型：播放列表'), 'B5 选中播放列表后：显示「类型：播放列表」');
    ok(has('pl-detail-fields', '曲目数：0'), 'B5 选中播放列表后：显示「曲目数：0」');
    ok(txt('pl-detail-fields').indexOf('暂无') < 0, 'B5 选中播放列表后：不含「暂无」字样');
    ok(visible('pl-detail-cover') === false,
       'B5 选中空播放列表后：无封面来源 → 封面控件隐藏');
    await fetch('/api/playlists/' + created.id, { method: 'DELETE' });
  }

  // ===== B-6 三页行为一致性 =====
  // 「一致」指的是**规则**一致，不是各页选中内容相同：
  //   · 三页都无选中 → 详情区（含封面 / 字段区）一律隐藏；
  //   · 封面控件的显隐完全由「是否有封面来源」决定；
  //   · 详情条的显隐完全由「是否有可渲染内容」决定。
  const resetSel = () => {
    // 只清三页的选中态，保留曲库页（B7 需要它验证「切回曲库页恢复自己的选中」）。
    state.currentAlbum = null; state.currentArtist = null; state.currentPlaylist = null;
    state.viewSelection.playlists = null;
    state.viewSelection.albums = null;
    state.viewSelection.artists = null;
  };
  const snap = (key) => ({
    wrap: visible(key + '-detail'),
    cover: visible(key + '-detail-cover'),
    fields: visible(key + '-detail-fields'),
  });

  // B-6a 三页均无选中 → 显隐状态必须完全一致（且全为 false）
  const noneStates = {};
  for (const p of pages) {
    // 清空全部选中态后，按该页重算详情（app.js 是普通脚本，顶层 function 挂在 window 上）。
    resetSel();
    renderViewDetail(p.tab);
    renderActiveDetail();
    await wait(200);
    noneStates[p.key] = snap(p.key);
  }
  const alNone = JSON.stringify(noneStates.al);
  ok(noneStates.al.wrap === false && noneStates.al.cover === false && noneStates.al.fields === false,
     'B6a 专辑页(无选中)：详情区三控件全部隐藏（' + alNone + '）');
  ok(JSON.stringify(noneStates.ar) === alNone,
     'B6a 三页一致：歌手页与专辑页显隐状态相同（' + alNone + ' vs ' + JSON.stringify(noneStates.ar) + '）');
  ok(JSON.stringify(noneStates.pl) === alNone,
     'B6a 三页一致：播放列表页与专辑页显隐状态相同（' + alNone + ' vs ' + JSON.stringify(noneStates.pl) + '）');

  // B-6b 封面控件显隐 ≡ 封面图**是否成功加载**
  //   （夹具 WAV 无内嵌封面 → /api/cover 返回 204 → 图加载失败 → 控件必须隐藏，
  //     这正是本轮契约：宁可隐藏，也不显示「无封面」占位。）
  const coverRule = (key) => {
    const el = $(key + '-detail-cover');
    const img = el ? el.querySelector('img') : null;
    const loaded = !!(img && img.complete && img.naturalWidth > 0);
    return loaded === visible(key + '-detail-cover');
  };
  document.getElementById('tab-albums').click(); await wait(700);
  const ign2 = Array.from(document.querySelectorAll('#al-list li'))
    .find((li) => li.dataset.name === 'Ignite');
  if (ign2) { ign2.click(); await wait(900); }
  ok(coverRule('al'), 'B6b 专辑页：封面控件显隐与「封面图是否加载成功」一致');
  ok(visible('al-detail-cover') === false,
     'B6b 专辑页：音频无内嵌封面 → 封面控件隐藏（不显示「无封面」）');
  document.getElementById('tab-artists').click(); await wait(700);
  const solo2 = Array.from(document.querySelectorAll('#ar-list li'))
    .find((li) => li.dataset.name === 'Iselin Solheim');
  if (solo2) { solo2.click(); await wait(900); }
  ok(coverRule('ar'), 'B6b 歌手页：封面控件显隐与「封面图是否加载成功」一致');
  ok(visible('ar-detail-cover') === false,
     'B6b 歌手页：音频无内嵌封面 → 封面控件隐藏（不显示「无封面」）');
  document.getElementById('tab-playlists').click(); await wait(700);
  ok(coverRule('pl'), 'B6b 播放列表页：封面控件显隐与「封面图是否加载成功」一致');

  // B-7 切回曲库页 → 恢复曲库页自己的选中曲目
  document.getElementById('tab-library').click(); await wait(800);
  ok(has('detail-fields', LIB_PICKED), 'B7 切回曲库页：右侧详情栏恢复曲库页自己的选中曲目');

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

    run_dir = C.run_dir('detail_region')
    app_dir = os.path.join(run_dir, 'app')
    front_dir = os.path.join(app_dir, 'frontend')
    os.makedirs(app_dir, exist_ok=True)

    # 真实前端 + 真实 exe
    C.build_frontend_copy(front_dir, inject_js=ASSERT_JS)
    exe = C.pick_exe()
    if not exe:
        A.ok(False, '未找到 mt-cli.exe（先 cargo build）')
        return A.report()
    shutil.copyfile(exe, os.path.join(app_dir, 'mt-cli.exe'))

    # app 级配置指向夹具曲库（固定绝对路径，原地复用）
    with io.open(os.path.join(app_dir, 'kqk-app.toml'), 'w', encoding='utf-8') as f:
        f.write("last_library_dir = '%s'\n" % C.FIXTURE_LIB.replace('\\', '\\\\'))

    A.ok(os.path.isfile(os.path.join(C.FIXTURE_LIB, '.mt', 'library.db')),
         '复用夹具曲库（%s）' % C.FIXTURE_LIB)

    proc, port = C.start_server(app_dir)
    A.ok(port is not None, '真实服务已启动（端口 %s）' % port)
    if not port:
        C.stop_server(proc)
        return A.report()
    base = 'http://127.0.0.1:%d' % port
    try:
        tracks = C.wait_for_tracks(base, want=3)
        A.ok(len(tracks) >= 3, '后端已索引 3 首真实音频（实际 %d）' % len(tracks))
        albums = C.http_get(base, '/api/albums').get('albums') or []
        artists = C.http_get(base, '/api/artists').get('artists') or []
        A.eq(sorted(a['name'] for a in albums), ['Different World', 'Ignite'],
             '真实标签解析出 2 个专辑')
        A.eq(sorted(a['name'] for a in artists), ['Alan Walker', 'Iselin Solheim'],
             '真实标签解析出 2 个歌手')

        out_file = os.path.join(run_dir, 'dom.txt')
        dom = C.edge_dump_dom(None, out_file, window='1080,645', budget=30000, url=base + '/')
        C.merge_assert_results(A, C.extract_assert_block(dom))
    finally:
        C.stop_server(proc)
    return A.report()


if __name__ == '__main__':
    sys.exit(main() or 0)
