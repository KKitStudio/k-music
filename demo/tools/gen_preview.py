# -*- coding: utf-8 -*-
"""生成前端无头验证用的 _preview.html：真实 index.html + 注入 mock fetch + 断言脚本。

断言分四组：
  A. 布局风格（横向 / 纵向）
  B. 列宽比例与拖拽（默认可见 7 列；创建时间默认隐藏）
  C. 跨页详情数据隔离（专辑 / 歌手页残留曲库页歌曲数据）
  D. 导入时间默认显示 / 创建时间默认隐藏且由设置面板开关 + 外观主题方案持久化
"""
import datetime
import io
import json
import os
import shutil
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import common as C

SRC = C.FRONTEND_SRC
OUT = os.path.join(C.RUN, 'preview')
os.makedirs(OUT, exist_ok=True)

for f in ('styles.css', 'app.js'):
    shutil.copyfile(os.path.join(SRC, f), os.path.join(OUT, f))

# 曲目：按 album / artist 分组，便于断言「专辑 / 歌手页各自的数据」。
# 第 1 首使用超长曲名 —— 它就是「全部曲库」页默认选中的曲目，
# 其它页面若出现该曲名即代表跨页数据污染。
# 时间戳用本地时区生成，前端按本地时区渲染，跨进程一致。
def _ms(y, mo, d, h, mi, s):
    return int(datetime.datetime(y, mo, d, h, mi, s).timestamp() * 1000)


THIS_YEAR = datetime.date.today().year
# 刻意覆盖三种情况：跨年 / 同年 / 空值（空值必须渲染成空单元格，不能出现「—」占位）。
T_OLD = _ms(2025, 3, 4, 9, 5, 6)
T_SAME = _ms(THIS_YEAR, 5, 6, 12, 34, 0)
T_SAME2 = _ms(THIS_YEAR, 1, 2, 8, 0, 0)

TRACKS = [
    {"local_id": 1,
     "name": "A Very Long Song Title That Should Be Fully Visible After Resizing The Column",
     "artist": "Alan Walker", "album": "Different World",
     "quality_tier": "Hi-Res", "duration_ms": 245000, "file_size": 45678901, "format": "FLAC",
     "sample_rate": 96000, "bit_depth": 24, "bitrate": 1411000, "channels": 2,
     "imported_at": T_OLD, "created_at": None},
    {"local_id": 2, "name": "第二首", "artist": "Alan Walker", "album": "Different World",
     "quality_tier": "Lossless", "duration_ms": 180000, "file_size": 12345678, "format": "MP3",
     "sample_rate": 44100, "bit_depth": 16, "bitrate": 320000, "channels": 2,
     "imported_at": T_SAME, "created_at": T_SAME2},
    {"local_id": 3, "name": "第三首", "artist": "Iselin Solheim", "album": "Ignite",
     "quality_tier": "Hi-Res", "duration_ms": 210000, "file_size": 22345678, "format": "FLAC",
     "sample_rate": 96000, "bit_depth": 24, "bitrate": 1411000, "channels": 2,
     "imported_at": None, "created_at": None},
]
ALBUMS = [{"name": "Different World", "track_count": 2}, {"name": "Ignite", "track_count": 1}]
ARTISTS = [{"name": "Alan Walker", "track_count": 2}, {"name": "Iselin Solheim", "track_count": 1}]
PLAYLISTS = [{"id": 1, "name": "我的列表", "track_count": 2}]

MOCK = r"""
<script>
/* ===== 注入：mock fetch（拦截所有 /api 请求） ===== */
const MOCK_TRACKS = __TRACKS__;
const MOCK_ALBUMS = __ALBUMS__;
const MOCK_ARTISTS = __ARTISTS__;
const MOCK_PLAYLISTS = __PLAYLISTS__;
const MOCK_STATUS = {
  library_configured: true,
  library_dir: 'D:/music/test',
  library_name: '测试曲库',
  netease_library_dir: '',
  file_scheme: 'artist_album',
  ncm_ready: false,
  ncm_tool: '',
  pending_ncm: 0,
  loose_ncm: [],
  // 后端默认：导入时间显示、创建时间隐藏（track_columns 只存可见列）。
  track_columns: ['name','artist','album','quality_tier','duration_ms','file_size','imported_at'],
  track_columns_saved: false,
  layout_styles: { library: 1, playlists: 2, albums: 2, artists: 2 },
  column_widths: { name: 24, artist: 16, album: 18, quality_tier: 6, duration_ms: 6, file_size: 8, imported_at: 11, created_at: 11 },
  appearance: {
    theme: 'dark', accent: '#7e57c2', skin_type: '', skin_preset: '', skin_accent: '', saved: true
  },
  auto_playlist_count: 20,
  app_version: '0.3.0',
  min_compatible_version: '0.3.0',
  config_version: '0.3.0',
  config_compat: 'compatible',
};
window.__POSTS = [];
// 无头环境里 <audio src="/api/audio/..."> 必然 404；禁用「失效自动跳曲」，否则队列断言会被连跳打乱。
window.__KQK_DISABLE_AUDIO_ERROR_SKIP__ = true;
window.fetch = function (url, opts) {
  opts = opts || {};
  const u = String(url);
  if (opts.method === 'POST') {
    let body = {};
    try { body = JSON.parse(opts.body || '{}'); } catch (e) {}
    window.__POSTS.push({ url: u, body: body });
    // /api/columns 要回显保存后的列，前端才能据它重建 state.columns / state.visible。
    let reply = { ok: true };
    if (u.indexOf('/api/columns') === 0) reply.columns = body.columns || [];
    if (u.indexOf('/api/ui-prefs') === 0 && body.auto_playlist_count !== undefined) {
      reply.auto_playlist_count = Number(body.auto_playlist_count);
    }
    return Promise.resolve({ ok: true, json: () => Promise.resolve(reply) });
  }
  const qname = (u.indexOf('name=') >= 0)
    ? decodeURIComponent(u.split('name=')[1].split('&')[0]) : '';
  let data = {};
  if (u.indexOf('/api/status') === 0) data = MOCK_STATUS;
  else if (u.indexOf('/api/tracks') === 0) data = { tracks: MOCK_TRACKS };
  else if (u.indexOf('/api/track/') === 0) {
    const id = Number(u.split('/').pop());
    data = { track: MOCK_TRACKS.find((t) => t.local_id === id) || MOCK_TRACKS[0] };
  }
  else if (u.indexOf('/api/albums/tracks') === 0)
    data = { tracks: MOCK_TRACKS.filter((t) => t.album === qname) };
  else if (u.indexOf('/api/albums') === 0) data = { albums: MOCK_ALBUMS };
  else if (u.indexOf('/api/artists/tracks') === 0)
    data = { tracks: MOCK_TRACKS.filter((t) => t.artist === qname) };
  else if (u.indexOf('/api/artists') === 0) data = { artists: MOCK_ARTISTS };
  else if (u.indexOf('/api/playlists/') === 0 && u.indexOf('/tracks') > 0)
    data = { tracks: MOCK_TRACKS.slice(0, 2) };
  else if (u.indexOf('/api/playlists') === 0) data = { playlists: MOCK_PLAYLISTS };
  else if (u.indexOf('/api/playstate') === 0) data = { ok: true, state: null };
  else if (u.indexOf('/api/cover') === 0) data = {};
  else if (u.indexOf('/api/lyrics') === 0) data = { timed: false, lines: [] };
  else data = {};
  return Promise.resolve({ ok: true, json: () => Promise.resolve(data) });
};
</script>
"""

ASSERT = r"""
<script>
(async () => {
  const R = [];
  const ok = (cond, msg) => R.push((cond ? 'PASS' : 'FAIL') + ' | ' + msg);
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const $ = (id) => document.getElementById(id);
  const txt = (id) => { const e = $(id); return e ? e.textContent : ''; };
  const has = (id, s) => txt(id).indexOf(s) >= 0;
  const isPlaceholder = (id) => !!($(id) && $(id).querySelector('.placeholder'));

  for (let i = 0; i < 80; i++) {
    if (document.querySelectorAll('#track-body tr').length) break;
    await wait(50);
  }
  await wait(200);

  // ---------- A. 布局风格 ----------
  ok(!$('detail-pane').hidden, '默认「全部曲库」= 横向：右侧详情栏可见');
  ok($('lib-detail').hidden, '默认「全部曲库」= 横向：内联详情条隐藏');

  document.getElementById('tab-artists').click();
  await wait(250);
  ok($('detail-pane').hidden, '「歌手」默认纵向：右侧详情栏隐藏');
  ok($('ar-detail').hidden, '「歌手」默认纵向(未选中)：内联详情条隐藏（无数据不占位）');

  document.getElementById('tab-playlists').click();
  await wait(200);
  ok($('detail-pane').hidden, '「播放列表」默认纵向：右侧详情栏隐藏');
  ok($('pl-detail').hidden, '「播放列表」默认纵向(未选中)：内联详情条隐藏（无数据不占位）');

  document.getElementById('tab-albums').click();
  await wait(200);
  ok($('al-detail').hidden, '「专辑」默认纵向(未选中)：内联详情条隐藏（无数据不占位）');

  // 设置面板：切「歌手」为横向
  document.getElementById('tab-artists').click();
  await wait(150);
  document.getElementById('btn-settings').click();
  await wait(200);
  const rows = document.querySelectorAll('#layout-style-list .layout-style-row');
  ok(rows.length === 4, '设置→列表：布局风格共 4 行（' + rows.length + '）');
  const artistRow = document.querySelector('#layout-style-list .layout-style-row:nth-child(4)');
  ok(!!artistRow && artistRow.querySelector('.layout-style-label').textContent === '歌手', '第 4 行为「歌手」');
  const segLabels = Array.from(document.querySelectorAll('#layout-style-list .seg-btn')).map(b => b.textContent.trim());
  ok(segLabels.length === 8 && segLabels.every(t => t === '横向' || t === '纵向'),
     '布局风格按钮文案为「横向 / 纵向」（实际：' + segLabels.join(',') + '）');
  ok(!segLabels.some(t => t.indexOf('布局') === 0), '按钮文案中已无「布局1 / 布局2」旧称');
  const hintTxt = document.querySelector('.settings-pane[data-pane="columns"] .modal-hint').textContent;
  ok(hintTxt.indexOf('横向') >= 0 && hintTxt.indexOf('纵向') >= 0,
     '设置说明文案含「横向 / 纵向」（实际：' + hintTxt.replace(/\s+/g, ' ').trim().slice(0, 40) + '…）');
  artistRow.querySelector('.seg-btn[data-style="1"]').click();
  await wait(300);
  ok($('detail-pane').hidden, '切「歌手」为横向 后：该页无内容 → 右侧详情栏隐藏');
  ok($('ar-detail').hidden, '切「歌手」为横向 后：内联详情条隐藏');
  const posts1 = window.__POSTS.filter((p) => p.url.indexOf('/api/ui-prefs') === 0);
  ok(posts1.length === 1 && posts1[0].body.layout_styles
      && posts1[0].body.layout_styles.artists === 1,
      '布局切换已 POST /api/ui-prefs（artists=1）');

  // 「全部曲库」切到纵向（注意：布局按页面生效，需切到该页才可见）
  const libRow = document.querySelector('#layout-style-list .layout-style-row:nth-child(1)');
  libRow.querySelector('.seg-btn[data-style="2"]').click();
  await wait(300);
  document.getElementById('tab-library').click();
  await wait(300);
  ok(!$('lib-detail').hidden, '「全部曲库」切纵向：内联详情条可见');
  ok($('detail-pane').hidden, '「全部曲库」切纵向：右侧详情栏隐藏');

  // 切回横向
  document.getElementById('btn-settings').click();
  await wait(200);
  document.querySelector('#layout-style-list .layout-style-row:nth-child(1) .seg-btn[data-style="1"]').click();
  await wait(300);
  document.getElementById('settings-close').click();
  await wait(150);
  ok(!$('detail-pane').hidden && $('lib-detail').hidden, '「全部曲库」切回横向：恢复右侧详情栏');

  // ---------- B. 列宽（默认可见 7 列；创建时间默认隐藏） ----------
  const head = $('track-head');
  const namePctBefore = state.columnWidths.name;
  const artistPctBefore = state.columnWidths.artist;
  const bizTh = Array.from(head.querySelectorAll('th[data-col-key]'));
  ok(bizTh.length === 7, '主列表表头 7 个可见业务列（' + bizTh.length + '）');
  ok(!bizTh.some((t) => t.dataset.colKey === 'created_at'),
     'B 默认不渲染「创建时间」列');
  const widths = bizTh.map((t) => t.getBoundingClientRect().width);
  ok(widths.every((w) => w > 0), '各列已按占比分配到像素宽度');
  const sum = widths.reduce((a, b) => a + b, 0);
  const tableW = $('track-table').getBoundingClientRect().width;
  const idxW = head.querySelector('th.idx-col').getBoundingClientRect().width;
  ok(Math.abs(sum + idxW - tableW) < 3, '业务列宽 + 固定列宽 ≈ 表格总宽（不溢出/不留空）');
  // 可见列之间重新归一化（隐藏的 created_at 份额分摊给其余 7 列）。
  // 窄窗口下浏览器会给窄列施加最小像素宽度，实测占比与配置允许 ±2 个百分点误差。
  const expect = normalizedColPcts(bizTh.map((t) => t.dataset.colKey)).map((v) => Math.round(v));
  const gotPct = widths.map((w) => Math.round((w / sum) * 100));
  ok(gotPct.every((v, i) => Math.abs(v - expect[i]) <= 2),
     '列宽比例符合配置（期望 ' + expect.join('/') + ' → 实测 ' + gotPct.join('/') + '）');

  const handles = head.querySelectorAll('th.col-resizable .col-resizer');
  ok(handles.length === 6, '可拖拽手柄 6 个（7 列的最后一列无右邻）（' + handles.length + '）');

  // 拖拽「曲名」列分隔线 +120px
  const nameTh = head.querySelector('th[data-col-key="name"]');
  const artistTh = head.querySelector('th[data-col-key="artist"]');
  const wBefore = nameTh.getBoundingClientRect().width;
  const aBefore = artistTh.getBoundingClientRect().width;
  const h = nameTh.querySelector('.col-resizer');
  const r = h.getBoundingClientRect();
  h.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, clientX: r.left + 3, clientY: r.top + 10 }));
  document.dispatchEvent(new MouseEvent('mousemove', { bubbles: true, clientX: r.left + 3 + 120, clientY: r.top + 10 }));
  await wait(60);
  const wDuring = nameTh.getBoundingClientRect().width;
  const aDuring = artistTh.getBoundingClientRect().width;
  const tableDuring = $('track-table').getBoundingClientRect().width;
  ok(wDuring > wBefore, '拖拽中「曲名」列变宽（' + Math.round(wBefore) + ' → ' + Math.round(wDuring) + '）');
  ok(aDuring < aBefore, '拖拽中「艺人」列等量变窄（' + Math.round(aBefore) + ' → ' + Math.round(aDuring) + '）');
  ok(Math.abs((wDuring + aDuring) - (wBefore + aBefore)) < 1.5,
     '拖拽中两列总宽不变（此消彼长）：' + Math.round(wBefore + aBefore) + ' → ' + Math.round(wDuring + aDuring));
  ok(aDuring >= 47, '「艺人」列不低于像素下限 48px：' + Math.round(aDuring));
  ok(Math.abs(tableDuring - tableW) < 1.5, '拖拽中表格总宽不变（不撑破、不横向滚动）');
  document.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, clientX: r.left + 3 + 120, clientY: r.top + 10 }));
  await wait(300);
  const posts2 = window.__POSTS.filter((p) => p.url.indexOf('/api/ui-prefs') === 0 && p.body.column_widths);
  ok(posts2.length >= 1, '拖拽结束已 POST /api/ui-prefs（column_widths）');
  if (posts2.length) {
    const cw = posts2[posts2.length - 1].body.column_widths;
    const tot = Object.values(cw).reduce((a, b) => a + b, 0);
    ok(Math.abs(tot - 100) < 0.5, '回写比例之和仍为 100（' + tot.toFixed(2) + '）');
    ok(cw.name > namePctBefore, '「曲名」比例已增大（' + namePctBefore + ' → ' + cw.name + '）');
    ok(cw.artist < artistPctBefore, '「艺人」比例已减小（' + artistPctBefore + ' → ' + cw.artist + '）');
    ok(Math.abs(cw.album - 18) < 0.1, '未被拖动的列占比保持不变（album=' + cw.album + '）');
    ok(Math.abs(cw.created_at - 11) < 0.1, '隐藏列的占比保持不变（created_at=' + cw.created_at + '）');
  }
  ok(!document.body.classList.contains('col-resizing'), '拖拽结束后 body.col-resizing 已清除');

  // 设置面板中的比例显示
  document.getElementById('btn-settings').click();
  await wait(200);
  const wRows = document.querySelectorAll('#col-width-list li');
  ok(wRows.length === 8, '设置→列表：列宽比例共 8 行（' + wRows.length + '）');
  const pctTxt = Array.from(document.querySelectorAll('#col-width-list .col-pct')).map((e) => e.textContent);
  ok(pctTxt.length === 8 && pctTxt[0].indexOf('%') > 0, '列宽比例显示为百分比：' + pctTxt.join(' '));
  document.getElementById('settings-close').click();
  await wait(150);

  // ---------- C. 跨页详情数据隔离（本轮 bug 修复） ----------
  // LIB_PICKED = 「全部曲库」页默认选中的曲目（超长曲名，辨识度最高）。
  // 任何其它页面出现该曲名 = 跨页数据污染（本次要修的 bug）。
  const LIB_PICKED = 'A Very Long Song Title That Should Be Fully Visible After Resizing The Column';

  // C1 曲库页：默认自动选中第一首 → 两个载体都应显示它
  document.getElementById('tab-library').click();
  await wait(400);
  ok(has('detail-fields', LIB_PICKED), 'C1 曲库页：右侧详情栏显示曲库页选中的曲目');
  ok(has('lib-detail-fields', LIB_PICKED), 'C1 曲库页：内联详情条显示曲库页选中的曲目');

  // C2 切到专辑页，未选中任何专辑 → 不得出现曲库页曲名
  document.getElementById('tab-albums').click();
  await wait(400);
  ok(!has('al-detail-fields', LIB_PICKED), 'C2 专辑页(无选中)：内联详情条不含曲库页曲名');
  ok(!has('detail-fields', LIB_PICKED), 'C2 专辑页(无选中)：右侧详情栏不含曲库页曲名');
  ok($('al-detail-fields').hidden, 'C2 专辑页(无选中)：字段区隐藏');
  ok($('al-detail-cover').hidden, 'C2 专辑页(无选中)：封面控件隐藏');
  ok($('al-detail').hidden, 'C2 专辑页(无选中)：内联详情条整体隐藏');
  ok(!isPlaceholder('al-detail-fields'), 'C2 专辑页(无选中)：无占位提示元素');
  ok($('al-detail-fields').textContent.trim() === '', 'C2 专辑页(无选中)：字段区无任何文本');
  ok($('al-detail-cover').textContent.indexOf('无封面') < 0, 'C2 专辑页(无选中)：无「无封面」字样');
  ok(!$('al-detail-fields').querySelector('b'), 'C2 专辑页(无选中)：无曲目粗体标题');
  ok($('al-content').textContent.indexOf('选择一个专辑') >= 0, 'C2 专辑页(无选中)：列表区仍为未选中提示');

  // C3 切到歌手页，未选中任何歌手 → 不得出现曲库页曲名
  document.getElementById('tab-artists').click();
  await wait(400);
  ok(!has('ar-detail-fields', LIB_PICKED), 'C3 歌手页(无选中)：内联详情条不含曲库页曲名');
  ok(!has('detail-fields', LIB_PICKED), 'C3 歌手页(无选中)：右侧详情栏不含曲库页曲名');
  ok($('ar-detail-fields').hidden, 'C3 歌手页(无选中)：字段区隐藏');
  ok($('ar-detail-cover').hidden, 'C3 歌手页(无选中)：封面控件隐藏');
  ok($('ar-detail').hidden, 'C3 歌手页(无选中)：内联详情条整体隐藏');
  ok(!isPlaceholder('ar-detail-fields'), 'C3 歌手页(无选中)：无占位提示元素');
  ok($('ar-detail-fields').textContent.trim() === '', 'C3 歌手页(无选中)：字段区无任何文本');
  ok($('ar-detail-cover').textContent.indexOf('无封面') < 0, 'C3 歌手页(无选中)：无「无封面」字样');
  ok(!$('ar-detail-fields').querySelector('b'), 'C3 歌手页(无选中)：无曲目粗体标题');

  // C4 选中专辑 → 展示专辑信息（名称 + 类型 + 曲目数），仍不含任何曲目名
  document.getElementById('tab-albums').click();
  await wait(300);
  const alItems = Array.from(document.querySelectorAll('#al-list li'));
  ok(alItems.length === 2, 'C4 专辑页：左侧列出 2 个专辑（' + alItems.length + '）');
  const ign = alItems.find((li) => li.dataset.name === 'Ignite');
  ok(!!ign, 'C4 专辑页：存在「Ignite」专辑');
  if (ign) {
    ign.click();
    await wait(400);
    ok(has('al-detail-fields', 'Ignite'), 'C4 选中专辑后：内联详情条显示专辑名');
    ok(has('al-detail-fields', '类型：专辑'), 'C4 选中专辑后：显示「类型：专辑」');
    ok(has('al-detail-fields', '曲目数：1'), 'C4 选中专辑后：显示「曲目数：1」');
    ok(!has('al-detail-fields', LIB_PICKED), 'C4 选中专辑后：不含曲库页曲名');
    ok(!has('detail-fields', LIB_PICKED), 'C4 选中专辑后：右侧详情栏不含曲库页曲名');
    const alCover = $('al-detail-cover').querySelector('img');
    const alDetail = (typeof viewDetail === 'function') ? viewDetail('albums') : null;
    ok(alDetail && alDetail.type === 'group' && alDetail.coverId === 3,
       'C4 选中专辑后：封面来源 = 该专辑首曲 local_id（' + (alDetail ? alDetail.coverId : 'null') + '）');
    ok($('al-detail-cover').textContent.indexOf('无封面') < 0,
       'C4 选中专辑后：封面区无「无封面」字样');
    ok(!alCover || $('al-detail-cover').hidden,
       'C4 选中专辑后：封面图不可用时封面控件隐藏');
  }

  // C5 在专辑页选中具体歌曲 → 显示该曲详情，且不是曲库页那首
  const alRows = document.querySelectorAll('#al-content tr[data-id]');
  ok(alRows.length === 1, 'C5 专辑「Ignite」下有 1 首歌（' + alRows.length + '）');
  if (alRows.length) {
    alRows[0].click();
    await wait(400);
    ok(has('al-detail-fields', '第三首'), 'C5 专辑页选中歌曲后：内联详情条显示该曲名');
    ok(!has('al-detail-fields', LIB_PICKED), 'C5 专辑页选中歌曲后：不含曲库页曲名');
    ok(!has('detail-fields', LIB_PICKED), 'C5 专辑页选中歌曲后：右侧详情栏不含曲库页曲名');
    ok(has('detail-fields', '第三首'), 'C5 专辑页选中歌曲后：右侧详情栏显示本页歌曲');
  }

  // C6 歌手页：选中歌手 → 展示歌手信息
  document.getElementById('tab-artists').click();
  await wait(300);
  const arItems = Array.from(document.querySelectorAll('#ar-list li'));
  ok(arItems.length === 2, 'C6 歌手页：左侧列出 2 个歌手（' + arItems.length + '）');
  const solo = arItems.find((li) => li.dataset.name === 'Iselin Solheim');
  ok(!!solo, 'C6 歌手页：存在「Iselin Solheim」歌手');
  if (solo) {
    solo.click();
    await wait(400);
    ok(has('ar-detail-fields', 'Iselin Solheim'), 'C6 选中歌手后：内联详情条显示歌手名');
    ok(has('ar-detail-fields', '类型：歌手'), 'C6 选中歌手后：显示「类型：歌手」');
    ok(has('ar-detail-fields', '曲目数：1'), 'C6 选中歌手后：显示「曲目数：1」');
    ok(!has('ar-detail-fields', LIB_PICKED), 'C6 选中歌手后：不含曲库页曲名');
    ok(!has('detail-fields', LIB_PICKED), 'C6 选中歌手后：右侧详情栏不含曲库页曲名');
    const arCover = $('ar-detail-cover').querySelector('img');
    const arDetail = (typeof viewDetail === 'function') ? viewDetail('artists') : null;
    ok(arDetail && arDetail.type === 'group' && arDetail.coverId === 3,
       'C6 选中歌手后：封面来源 = 该歌手首曲 local_id（' + (arDetail ? arDetail.coverId : 'null') + '）');
    ok($('ar-detail-cover').textContent.indexOf('无封面') < 0,
       'C6 选中歌手后：封面区无「无封面」字样');
    ok(!arCover || $('ar-detail-cover').hidden,
       'C6 选中歌手后：封面图不可用时封面控件隐藏');
  }

  // C7 歌手页选中具体歌曲 → 显示该曲详情
  const arRows = document.querySelectorAll('#ar-content tr[data-id]');
  ok(arRows.length === 1, 'C7 歌手「Iselin Solheim」下有 1 首歌（' + arRows.length + '）');
  if (arRows.length) {
    arRows[0].click();
    await wait(400);
    ok(has('ar-detail-fields', '第三首'), 'C7 歌手页选中歌曲后：内联详情条显示该曲名');
    ok(!has('ar-detail-fields', LIB_PICKED), 'C7 歌手页选中歌曲后：不含曲库页曲名');
    ok(!has('detail-fields', LIB_PICKED), 'C7 歌手页选中歌曲后：右侧详情栏不含曲库页曲名');
  }

  // C8 切回曲库页 → 恢复曲库页自己的选中曲目（各页选中态互不干扰）
  document.getElementById('tab-library').click();
  await wait(400);
  ok(has('detail-fields', LIB_PICKED), 'C8 切回曲库页：右侧详情栏恢复曲库页自己的选中曲目');
  ok(has('lib-detail-fields', LIB_PICKED), 'C8 切回曲库页：内联详情条恢复曲库页自己的选中曲目');

  // C9 播放列表页同样隔离
  document.getElementById('tab-playlists').click();
  await wait(400);
  const plItems = Array.from(document.querySelectorAll('#pl-list li'));
  ok(plItems.length === 1, 'C9 播放列表页：左侧列出 1 个列表（' + plItems.length + '）');
  if (plItems.length) {
    plItems[0].click();
    await wait(400);
    ok(has('pl-detail-fields', '我的列表'), 'C9 选中播放列表后：内联详情条显示列表名');
    ok(has('pl-detail-fields', '类型：播放列表'), 'C9 选中播放列表后：显示「类型：播放列表」');
    ok(!has('pl-detail-fields', LIB_PICKED), 'C9 选中播放列表后：不含曲库页曲名');
    ok(!has('detail-fields', LIB_PICKED), 'C9 选中播放列表后：右侧详情栏不含曲库页曲名');
  }

  // C10 切回曲库页再次确认未被污染
  document.getElementById('tab-library').click();
  await wait(400);
  ok(has('detail-fields', LIB_PICKED), 'C10 最终切回曲库页：详情仍为曲库页曲目');

  // ---------- D. 时间列 + 创建时间默认隐藏（由设置面板控制） ----------
  await wait(300);
  const dTh = Array.from($('track-head').querySelectorAll('th[data-col-key]'));
  const dLabels = dTh.map((t) => t.textContent.trim());
  ok(dLabels.indexOf('导入时间') >= 0, 'D1 主列表默认显示「导入时间」');
  ok(dLabels.indexOf('创建时间') < 0, 'D1 主列表默认不显示「创建时间」');
  const dKeys = dTh.map((t) => t.dataset.colKey);
  const iIdx = dKeys.indexOf('imported_at');
  ok(iIdx >= 0 && dKeys.indexOf('created_at') < 0,
     'D1 只有 imported_at 进入表头 / 列宽分配');
  const bodyRows = Array.from($('track-body').querySelectorAll('tr'));
  ok(bodyRows.length === 3, 'D2 主列表渲染 3 行（' + bodyRows.length + '）');
  const cellText = (row, idx) => (row.children[idx + 1] ? row.children[idx + 1].textContent.trim() : '<none>');
  const cellEl = (row, idx) => row.children[idx + 1];
  const imp0 = cellText(bodyRows[0], iIdx);
  ok(imp0 === '2025-03-04', 'D2 跨年导入时间显示 YYYY-MM-DD（' + imp0 + '）');
  const span0 = cellEl(bodyRows[0], iIdx).querySelector('.dt-cell');
  ok(!!span0 && span0.title === '2025-03-04 09:05:06',
     'D2 完整时间放在 title：' + (span0 ? span0.title : 'null'));
  const imp1 = cellText(bodyRows[1], iIdx);
  ok(/^\d{2}-\d{2} \d{2}:\d{2}$/.test(imp1), 'D2 同年导入时间显示 MM-DD HH:MM（' + imp1 + '）');
  ok(cellText(bodyRows[2], iIdx) === '', 'D3 导入时间为空 → 空单元格（无「—」「暂无」占位）');
  const bodyTxt = $('track-body').textContent;
  ok(bodyTxt.indexOf('暂无') < 0 && bodyTxt.indexOf('—') < 0, 'D3 列表无「—」「暂无」占位内容');
  ok(has('detail-fields', '导入时间') && has('detail-fields', '2025-03-04'),
     'D4 详情面板显示「导入时间」');
  ok(!has('detail-fields', '创建时间'), 'D4 创建时间为空 → 该行整行不渲染（不显示「—」）');

  // 子页面（专辑 / 播放列表）三级列表跟随同一套显示项设置
  document.getElementById('tab-albums').click();
  await wait(400);
  const alLi = Array.from(document.querySelectorAll('#al-list li'));
  if (alLi.length) { alLi[0].click(); await wait(400); }
  const alTh = Array.from(document.querySelectorAll('#al-content th')).map((t) => t.textContent.trim());
  ok(alTh.indexOf('导入时间') >= 0 && alTh.indexOf('创建时间') < 0,
     'D5 专辑页三级列表跟随显示项：有导入时间、无创建时间');
  document.getElementById('tab-playlists').click();
  await wait(400);
  const plTh2 = Array.from(document.querySelectorAll('#pl-content th')).map((t) => t.textContent.trim());
  ok(plTh2.indexOf('导入时间') >= 0 && plTh2.indexOf('创建时间') < 0,
     'D5 播放列表页三级列表跟随显示项：有导入时间、无创建时间');

  // ---------- D8. 设置面板开关「创建时间」 ----------
  document.getElementById('tab-library').click();
  await wait(250);
  document.getElementById('btn-settings').click();
  await wait(200);
  const cfgLi = document.querySelector('#col-config-list li[data-key="created_at"]');
  const impLi = document.querySelector('#col-config-list li[data-key="imported_at"]');
  ok(!!cfgLi, 'D8 设置→列表：显示项里存在「创建时间」条目');
  const cbCre = cfgLi && cfgLi.querySelector('input[data-act="vis"]');
  const cbImp = impLi && impLi.querySelector('input[data-act="vis"]');
  ok(!!cbCre && !cbCre.checked, 'D8「创建时间」默认未勾选');
  ok(!!cbImp && cbImp.checked, 'D8「导入时间」默认已勾选');
  if (cbCre) {
    cbCre.checked = true;
    cbCre.dispatchEvent(new Event('change', { bubbles: true }));
  }
  document.getElementById('set-apply-columns').click();
  await wait(400);
  const colPosts = window.__POSTS.filter((p) => p.url.indexOf('/api/columns') === 0);
  ok(colPosts.length >= 1, 'D8 应用显示设置已 POST /api/columns');
  ok(colPosts.length > 0 && colPosts[colPosts.length - 1].body.columns.indexOf('created_at') >= 0,
     'D8 保存的列里包含 created_at');
  const thAfter = Array.from($('track-head').querySelectorAll('th[data-col-key]')).map((t) => t.dataset.colKey);
  ok(thAfter.indexOf('created_at') >= 0, 'D8 勾选后主列表出现「创建时间」列');
  const cIdx2 = thAfter.indexOf('created_at');
  const rowsAfter = Array.from($('track-body').querySelectorAll('tr'));
  ok(cIdx2 >= 0 && rowsAfter.length === 3 && cellText(rowsAfter[0], cIdx2) === '',
     'D8 第 1 首创建时间为空 → 空单元格（不显示占位）');
  ok(cIdx2 >= 0 && /^\d{2}-\d{2} \d{2}:\d{2}$/.test(cellText(rowsAfter[1], cIdx2)),
     'D8 第 2 首创建时间显示 MM-DD HH:MM（' + (cIdx2 >= 0 ? cellText(rowsAfter[1], cIdx2) : '') + '）');
  // 再取消勾选 → 列表恢复隐藏
  const cfgLi2 = document.querySelector('#col-config-list li[data-key="created_at"]');
  const cbCre2 = cfgLi2 && cfgLi2.querySelector('input[data-act="vis"]');
  if (cbCre2) {
    cbCre2.checked = false;
    cbCre2.dispatchEvent(new Event('change', { bubbles: true }));
  }
  document.getElementById('set-apply-columns').click();
  await wait(400);
  const thBack = Array.from($('track-head').querySelectorAll('th[data-col-key]')).map((t) => t.dataset.colKey);
  ok(thBack.indexOf('created_at') < 0, 'D8 取消勾选后「创建时间」列重新隐藏');
  const lastColPost = window.__POSTS.filter((p) => p.url.indexOf('/api/columns') === 0).pop();
  ok(!!lastColPost && lastColPost.body.columns.indexOf('created_at') < 0,
     'D8 取消勾选的保存结果不含 created_at');
  document.getElementById('settings-close').click();
  await wait(150);

  // 外观：以配置文件为准还原 + 改动落盘（用户报告的 bug）
  ok(document.documentElement.getAttribute('data-theme') === 'dark',
     'D6 主题从 /api/status 配置还原：data-theme=dark');
  const primary = getComputedStyle(document.documentElement).getPropertyValue('--primary').trim();
  ok(primary.toLowerCase() === '#7e57c2', 'D6 主题色从配置还原：' + primary);
  const segDark = document.querySelector('#ap-theme-seg .seg-btn[data-theme="dark"]');
  ok(!!segDark && segDark.classList.contains('active'), 'D6 主题分段控件高亮 dark');
  const postMark = window.__POSTS.length;
  const segLight = document.querySelector('#ap-theme-seg .seg-btn[data-theme="light"]');
  if (segLight) segLight.click();
  await wait(450);
  const apPosts = window.__POSTS.slice(postMark).filter((p) => p.body && p.body.appearance);
  ok(apPosts.length >= 1, 'D7 改主题后防抖 POST /api/ui-prefs（appearance）');
  ok(apPosts.length > 0 && apPosts[apPosts.length - 1].body.appearance.theme === 'light',
     'D7 POST 的 appearance.theme=light');
  ok(document.documentElement.getAttribute('data-theme') === 'light',
     'D7 改主题后界面立即生效');

  // ---------- E. 统一播放上下文 / n / 全局播控 ----------
  document.getElementById('tab-library').click();
  await wait(250);
  ok(state.autoPlaylistCount === 20, 'E1 /api/status 读回默认 n=20（实际 ' + state.autoPlaylistCount + '）');
  ok(state.playback.queue.length === 0, 'E1 首次进入无播放队列');
  ok($('queue-title').textContent === '', 'E1 未播放时队列标题为空');

  // n=2、曲库共 3 首 → 队列最多 2 首；点击第 2 首，检验「不从队首播」。
  state.autoPlaylistCount = 2;
  const rowsLib = Array.from(document.querySelectorAll('#track-body tr[data-id]'));
  const clickedId = Number(rowsLib[1].dataset.id);
  rowsLib[1].dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
  await wait(250);
  const pb0 = state.playback;
  ok(pb0.queue.length === 2, 'E2 n=2 → 无名队列最多 2 首（实际 ' + pb0.queue.length + '）');
  ok(pb0.n === 2, 'E2 队列快照记下生成时的 n=2');
  ok(pb0.named === false, 'E2 曲库来源判定为无名队列');
  ok(pb0.queue.indexOf(clickedId) >= 0 && pb0.index === pb0.queue.indexOf(clickedId),
     'E2 点击曲在队列内且 index 指向它（不从队首播）');
  ok(pb0.title.indexOf('自动播放列表') >= 0, 'E2 队列标题标识无名列表');

  // 快照只存 id，不引用 state.tracks（搜索 / 扫描 / 导入不会改写队列）。
  const qBefore = pb0.queue.join(',');
  state.tracks = [];
  ok(state.playback.queue.join(',') === qBefore, 'E3 改写 state.tracks 不影响已生成队列');
  state.tracks = MOCK_TRACKS.slice();
  renderTracks();
  state.autoPlaylistCount = 3;
  ok(state.playback.n === 2, 'E4 播放中改 n 不改写已生成队列的 n 快照');

  // 播控：上 / 下一首、循环开关、随机洗一次固定顺序。
  // 先重播队首（E2 点的是队尾，顺序模式下「下一首」按规格应停在末尾，不能拿它测推进）。
  state.autoPlaylistCount = 2;
  const libRows2 = document.querySelectorAll('#track-body tr[data-id]');
  libRows2[0].dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
  await wait(220);
  const idx0 = state.playback.index;
  ok(idx0 === 0, 'E5 重播队首后 index=0（实际 ' + idx0 + '）');
  $('btn-next').click(); await wait(120);
  ok(state.playback.index === idx0 + 1, 'E5 下一首推进 index');
  $('btn-prev').click(); await wait(120);
  ok(state.playback.index === idx0, 'E5 上一首回退 index');
  $('btn-repeat').click(); await wait(80);
  ok(state.playback.repeat === true, 'E5 列表循环开关生效');
  $('btn-repeat').click(); await wait(80);
  ok(state.playback.repeat === false, 'E5 可关闭列表循环（顺序停止）');
  const orig = state.playback.originalQueue.join(',');
  $('btn-shuffle').click(); await wait(80);
  ok(state.playback.shuffle === true, 'E6 随机开关生效');
  ok(state.playback.originalQueue.join(',') === orig, 'E6 洗牌不改写原始顺序快照');
  ok(state.playback.queue.length === 2 && state.playback.queue.slice().sort().join(',')
     === orig.split(',').sort().join(','), 'E6 随机只洗当前队列，集合不变');
  $('btn-shuffle').click(); await wait(80);
  ok(state.playback.queue.join(',') === orig, 'E6 关闭随机恢复界面顺序');

  // 队列面板 / 插播 / 清空。
  $('btn-queue').click(); await wait(80);
  ok(!$('queue-drawer').hidden, 'E7 队列面板可打开');
  ok($('queue-list').querySelectorAll('li').length === state.playback.queue.length,
     'E7 队列面板行数与队列长度一致');
  const li0 = $('queue-list').querySelectorAll('li')[0];
  if (li0) { li0.click(); await wait(100); ok(state.playback.index === 0, 'E7 点击队列行可跳播'); }
  $('queue-close').click();
  const beforeLen = state.playback.queue.length;
  state.current = MOCK_TRACKS[2];
  $('btn-enqueue').click(); await wait(80);
  ok(state.playback.queue.length === beforeLen + 1, 'E8 「下一首播放」插入一首');
  ok(Number(state.playback.queue[state.playback.index + 1]) === Number(MOCK_TRACKS[2].local_id),
     'E8 插播位置紧跟当前曲');
  $('queue-clear').click(); await wait(80);
  ok(state.playback.queue.length === 0, 'E8 清空队列后状态归零');

  // 有名列表：队列 = 列表全量，n=1 也不截断。
  document.getElementById('tab-playlists').click(); await wait(350);
  const plLi = document.querySelector('#pl-list li[data-id]');
  if (plLi) plLi.click();
  await wait(350);
  const plRows = Array.from(document.querySelectorAll('#pl-content tr[data-id]'));
  state.autoPlaylistCount = 1;
  if (plRows.length) plRows[0].dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
  await wait(250);
  ok(state.playback.named === true, 'E9 播放列表页来源判定为有名队列');
  ok(state.playback.n === null, 'E9 有名队列不记 n（不受截断影响）');
  ok(state.playback.queue.length === plRows.length && plRows.length === 2,
     'E9 有名队列 = 列表全量 2 首（n=1 仍全量，实际 ' + state.playback.queue.length + '）');

  // 设置 → 播放：保存 n，且不改动正在播放的队列。
  document.getElementById('btn-settings').click(); await wait(150);
  const tabPlayback = document.querySelector('.settings-tab[data-tab="playback"]');
  ok(!!tabPlayback, 'E10 设置面板新增「播放」分区');
  if (tabPlayback) tabPlayback.click();
  await wait(100);
  $('set-auto-count').value = '7';
  const pMark = window.__POSTS.length;
  $('set-apply-playback').click(); await wait(220);
  const nPosts = window.__POSTS.slice(pMark).filter(
    (p) => p.url.indexOf('/api/ui-prefs') === 0 && p.body.auto_playlist_count !== undefined);
  ok(nPosts.length >= 1 && Number(nPosts[nPosts.length - 1].body.auto_playlist_count) === 7,
     'E10 设置面板保存 n=7（POST /api/ui-prefs）');
  ok(state.autoPlaylistCount === 7, 'E10 保存后 state 同步为 7');
  ok(state.playback.queue.length === 2, 'E10 改 n 不影响正在播放的队列（下次生成才生效）');
  document.getElementById('settings-close').click();

  const pre = document.createElement('pre');
  pre.id = 'ASSERT';
  pre.textContent = R.join('\n');
  document.body.appendChild(pre);
})();
</script>
"""


def main():
    with io.open(os.path.join(SRC, 'index.html'), encoding='utf-8') as f:
        html = f.read()

    mock = (MOCK.replace('__TRACKS__', json.dumps(TRACKS, ensure_ascii=False))
                .replace('__ALBUMS__', json.dumps(ALBUMS, ensure_ascii=False))
                .replace('__ARTISTS__', json.dumps(ARTISTS, ensure_ascii=False))
                .replace('__PLAYLISTS__', json.dumps(PLAYLISTS, ensure_ascii=False)))
    tag = '<script src="app.js"></script>'
    assert tag in html, '未找到 app.js 脚本标签'
    html = html.replace(tag, mock + '\n' + tag + '\n' + ASSERT)

    with io.open(os.path.join(OUT, 'index.html'), 'w', encoding='utf-8') as f:
        f.write(html)
    print('preview written to', OUT)


if __name__ == '__main__':
    main()
