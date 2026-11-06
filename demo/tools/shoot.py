# -*- coding: utf-8 -*-
"""生成布局风格 / 列宽功能的效果图（无头 Edge 截图）。"""
import io
import json
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import common as C
ROOT = C.ROOT
SRC = C.FRONTEND_SRC
OUT = os.path.join(ROOT, 'docs', 'screenshots')
WORK = os.path.join(C.RUN, 'shot')
EDGE = r'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'

TRACKS = [
    {"local_id": 1, "name": "Yellow Cold Rose", "artist": "Alan Walker", "album": "Different World",
     "quality_tier": "Hi-Res", "duration_ms": 200000, "file_size": 45678901, "format": "FLAC",
     "sample_rate": 96000, "bit_depth": 24, "bitrate": 1411000, "channels": 2},
    {"local_id": 2, "name": "Faded", "artist": "Alan Walker feat. Iselin Solheim", "album": "Different World",
     "quality_tier": "Lossless", "duration_ms": 212000, "file_size": 12345678, "format": "MP3",
     "sample_rate": 44100, "bit_depth": 16, "bitrate": 320000, "channels": 2},
    {"local_id": 3, "name": "Ignite", "artist": "Alan Walker", "album": "Ignite",
     "quality_tier": "Hi-Res", "duration_ms": 228000, "file_size": 33456789, "format": "FLAC",
     "sample_rate": 96000, "bit_depth": 24, "bitrate": 1411000, "channels": 2},
    {"local_id": 4, "name": "Spectre", "artist": "Alan Walker feat. Mia Wright", "album": "Different World",
     "quality_tier": "Lossless", "duration_ms": 195000, "file_size": 23456789, "format": "MP3",
     "sample_rate": 44100, "bit_depth": 16, "bitrate": 320000, "channels": 2},
]

MOCK = """
<script>
const MOCK_TRACKS = %s;
const MOCK_LAYOUTS = %s;
const MOCK_WIDTHS = %s;
window.fetch = function (url, opts) {
  opts = opts || {};
  if (opts.method === 'POST') return Promise.resolve({ok:true, json:()=>Promise.resolve({ok:true})});
  let data = {};
  const u = String(url);
  if (u.indexOf('/api/status') === 0) data = { library_configured:true, library_dir:'D:/music',
    library_name:'我的音乐库', track_columns:['name','artist','album','quality_tier','duration_ms','file_size'],
    layout_styles: MOCK_LAYOUTS, column_widths: MOCK_WIDTHS,
    app_version:'0.3.0', ncm_ready:false, pending_ncm:0, loose_ncm:[], file_scheme:'artist_album' };
  else if (u.indexOf('/api/tracks') === 0) data = { tracks: MOCK_TRACKS };
  else if (u.indexOf('/api/track/') === 0) data = { track: MOCK_TRACKS[0] };
  else if (u.indexOf('/api/albums/tracks') === 0) data = { tracks: MOCK_TRACKS };
  else if (u.indexOf('/api/albums') === 0) data = { albums:[{name:'Different World',track_count:3},{name:'Ignite',track_count:1}] };
  else if (u.indexOf('/api/artists/tracks') === 0) data = { tracks: MOCK_TRACKS };
  else if (u.indexOf('/api/artists') === 0) data = { artists:[{name:'Alan Walker',track_count:4}] };
  else if (u.indexOf('/api/playlists') === 0) data = { playlists:[] };
  else data = {};
  return Promise.resolve({ok:true, json:()=>Promise.resolve(data)});
};
</script>
""" % (json.dumps(TRACKS, ensure_ascii=False), '{}', '{}')

DRIVER = """
<script>
(async () => {
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  const h = location.hash.slice(1);
  const cfg = h ? JSON.parse(decodeURIComponent(h)) : {};
  for (let i=0;i<80;i++){ if(document.querySelectorAll('#track-body tr').length) break; await wait(50); }
  await wait(400);
  if (cfg.tab) { document.getElementById('tab-' + cfg.tab).click(); await wait(500); }
  if (cfg.settings) {
    document.getElementById('btn-settings').click();
    await wait(200);
    const pane = document.querySelector('.settings-tab[data-tab="columns"]');
    if (pane && pane.click) pane.click();
    await wait(300);
    if (cfg.scrollTo) {
      const el = document.getElementById(cfg.scrollTo);
      if (el) el.scrollIntoView({ block: 'center' });
      await wait(300);
    }
  }
  const mark = document.createElement('div');
  mark.id = 'SHOTREADY';
  mark.textContent = '1';
  document.body.appendChild(mark);
})();
</script>
"""


def build(layouts, widths):
    os.makedirs(WORK, exist_ok=True)
    for f in ('styles.css', 'app.js'):
        shutil.copyfile(os.path.join(SRC, f), os.path.join(WORK, f))
    with io.open(os.path.join(SRC, 'index.html'), encoding='utf-8') as f:
        html = f.read()
    mock = MOCK.replace("'{}'", json.dumps(layouts)).replace("'{}'", json.dumps(widths))
    # 精确替换两处占位
    mock = MOCK
    mock = mock.replace('%s' % json.dumps(TRACKS, ensure_ascii=False), json.dumps(TRACKS, ensure_ascii=False))
    mock = mock.replace('const MOCK_LAYOUTS = {};', 'const MOCK_LAYOUTS = %s;' % json.dumps(layouts))
    mock = mock.replace('const MOCK_WIDTHS = {};', 'const MOCK_WIDTHS = %s;' % json.dumps(widths))
    tag = '<script src="app.js"></script>'
    html = html.replace(tag, mock + '\n' + tag + '\n' + DRIVER)
    with io.open(os.path.join(WORK, 'index.html'), 'w', encoding='utf-8') as f:
        f.write(html)


def shot(name, layouts, widths, w=1080, h=645, cfg=None):
    build(layouts, widths)
    path = os.path.join(OUT, name)
    url = 'file:///' + os.path.join(WORK, 'index.html').replace('\\', '/')
    if cfg:
        url += '#' + urllib_quote(json.dumps(cfg))
    cmd = [EDGE, '--headless=new', '--disable-gpu', '--no-sandbox', '--hide-scrollbars',
           '--allow-file-access-from-files', '--window-size=%d,%d' % (w, h),
           '--virtual-time-budget=9000', '--screenshot=' + path, url]
    subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
    print(('OK   ' if os.path.exists(path) else 'FAIL ') + name, os.path.getsize(path) if os.path.exists(path) else '')


def urllib_quote(s):
    out = []
    for ch in s:
        if ch.isalnum() or ch in '-_.~':
            out.append(ch)
        else:
            out.extend('%%%02X' % b for b in ch.encode('utf-8'))
    return ''.join(out)


DEF_W = {'name': 30, 'artist': 20, 'album': 22, 'quality_tier': 8, 'duration_ms': 8, 'file_size': 12}

if __name__ == '__main__':
    os.makedirs(OUT, exist_ok=True)
    # 1) 主界面 横向（默认）：右侧独立详情栏
    shot('layout1-library-1080x645.png',
         {'library': 1, 'playlists': 2, 'albums': 2, 'artists': 2}, DEF_W, cfg={'tab': 'library'})
    # 2) 主界面 纵向：内联详情条
    shot('layout2-library-1080x645.png',
         {'library': 2, 'playlists': 2, 'albums': 2, 'artists': 2}, DEF_W, cfg={'tab': 'library'})
    # 3) 歌手页 纵向（默认）
    shot('layout2-artists-1080x645.png',
         {'library': 1, 'playlists': 2, 'albums': 2, 'artists': 2}, DEF_W, cfg={'tab': 'artists'})
    # 4) 设置 → 列表 分区（布局风格 + 列宽比例）
    shot('settings-list-columns-1080x645.png',
         {'library': 1, 'playlists': 2, 'albums': 2, 'artists': 2}, DEF_W, cfg={'settings': True})
    # 5) 设置 → 列表：滚动到「歌曲列表列宽比例」
    shot('settings-col-widths-1080x645.png',
         {'library': 1, 'playlists': 2, 'albums': 2, 'artists': 2}, DEF_W,
         cfg={'settings': True, 'scrollTo': 'col-width-list'})
