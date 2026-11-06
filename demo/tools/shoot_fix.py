# -*- coding: utf-8 -*-
"""生成「详情区公共组件」效果图（无头 Edge 截图）。

场景：
  fix-albums-empty.png     专辑页未选中专辑   → 详情区**整体隐藏**（无占位、无「无封面」）
  fix-albums-selected.png  专辑页选中专辑     → 展示专辑名 / 类型 / 曲目数
  fix-artists-empty.png    歌手页未选中歌手   → 详情区整体隐藏
  fix-artists-selected.png 歌手页选中歌手     → 展示歌手名 / 类型 / 曲目数
  fix-playlists-empty.png  播放列表页未选中   → 详情区整体隐藏（三页行为一致）
"""
import io
import json
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import common as C
ROOT = C.ROOT
SRC = C.FRONTEND_SRC
OUT = os.path.join(ROOT, 'docs', 'screenshots')
WORK = os.path.join(C.RUN, 'shot2')
EDGE = r'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'

TRACKS = [
    {"local_id": 1, "name": "Yellow Cold Rose", "artist": "Alan Walker", "album": "Different World",
     "quality_tier": "Hi-Res", "duration_ms": 200000, "file_size": 45678901, "format": "FLAC",
     "sample_rate": 96000, "bit_depth": 24, "bitrate": 1411000, "channels": 2},
    {"local_id": 2, "name": "Faded", "artist": "Alan Walker", "album": "Different World",
     "quality_tier": "Lossless", "duration_ms": 212000, "file_size": 12345678, "format": "MP3",
     "sample_rate": 44100, "bit_depth": 16, "bitrate": 320000, "channels": 2},
    {"local_id": 3, "name": "Ignite", "artist": "Iselin Solheim", "album": "Ignite",
     "quality_tier": "Hi-Res", "duration_ms": 228000, "file_size": 33456789, "format": "FLAC",
     "sample_rate": 96000, "bit_depth": 24, "bitrate": 1411000, "channels": 2},
]
ALBUMS = [{"name": "Different World", "track_count": 2}, {"name": "Ignite", "track_count": 1}]
ARTISTS = [{"name": "Alan Walker", "track_count": 2}, {"name": "Iselin Solheim", "track_count": 1}]

MOCK = """
<script>
const MOCK_TRACKS = %(tracks)s;
const MOCK_ALBUMS = %(albums)s;
const MOCK_ARTISTS = %(artists)s;
window.fetch = function (url, opts) {
  opts = opts || {};
  if (opts.method === 'POST') return Promise.resolve({ok:true, json:()=>Promise.resolve({ok:true})});
  const u = String(url);
  const qname = (u.indexOf('name=') >= 0) ? decodeURIComponent(u.split('name=')[1].split('&')[0]) : '';
  let data = {};
  if (u.indexOf('/api/status') === 0) data = { library_configured:true, library_dir:'D:/music',
    library_name:'我的音乐库', track_columns:['name','artist','album','quality_tier','duration_ms','file_size'],
    layout_styles:{library:1,playlists:2,albums:2,artists:2},
    column_widths:{name:30,artist:20,album:22,quality_tier:8,duration_ms:8,file_size:12},
    app_version:'0.3.0', ncm_ready:false, pending_ncm:0, loose_ncm:[], file_scheme:'artist_album' };
  else if (u.indexOf('/api/tracks') === 0) data = { tracks: MOCK_TRACKS };
  else if (u.indexOf('/api/track/') === 0) {
    const id = Number(u.split('/').pop());
    data = { track: MOCK_TRACKS.find((t) => t.local_id === id) || MOCK_TRACKS[0] };
  }
  else if (u.indexOf('/api/albums/tracks') === 0) data = { tracks: MOCK_TRACKS.filter((t) => t.album === qname) };
  else if (u.indexOf('/api/albums') === 0) data = { albums: MOCK_ALBUMS };
  else if (u.indexOf('/api/artists/tracks') === 0) data = { tracks: MOCK_TRACKS.filter((t) => t.artist === qname) };
  else if (u.indexOf('/api/artists') === 0) data = { artists: MOCK_ARTISTS };
  else if (u.indexOf('/api/playlists') === 0) data = { playlists: [] };
  else data = {};
  return Promise.resolve({ ok:true, json: () => Promise.resolve(data) });
};
</script>
""" % {'tracks': json.dumps(TRACKS, ensure_ascii=False),
       'albums': json.dumps(ALBUMS, ensure_ascii=False),
       'artists': json.dumps(ARTISTS, ensure_ascii=False)}

DRIVER = """
<script>
(async () => {
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  const h = location.hash.slice(1);
  const cfg = h ? JSON.parse(decodeURIComponent(h)) : {};
  for (let i=0;i<80;i++){ if(document.querySelectorAll('#track-body tr').length) break; await wait(50); }
  await wait(400);
  if (cfg.tab) { document.getElementById('tab-' + cfg.tab).click(); await wait(500); }
  if (cfg.select) {
    const items = Array.from(document.querySelectorAll(cfg.list + ' li'));
    const it = items.find((li) => li.dataset.name === cfg.select);
    if (it) { it.click(); await wait(600); }
  }
  const mark = document.createElement('div');
  mark.id = 'SHOTREADY';
  mark.textContent = '1';
  document.body.appendChild(mark);
})();
</script>
"""


def build():
    os.makedirs(WORK, exist_ok=True)
    for f in ('styles.css', 'app.js'):
        shutil.copyfile(os.path.join(SRC, f), os.path.join(WORK, f))
    with io.open(os.path.join(SRC, 'index.html'), encoding='utf-8') as f:
        html = f.read()
    tag = '<script src="app.js"></script>'
    html = html.replace(tag, MOCK + '\n' + tag + '\n' + DRIVER)
    with io.open(os.path.join(WORK, 'index.html'), 'w', encoding='utf-8') as f:
        f.write(html)


def q(s):
    out = []
    for ch in s:
        if ch.isalnum() or ch in '-_.~':
            out.append(ch)
        else:
            out.extend('%%%02X' % b for b in ch.encode('utf-8'))
    return ''.join(out)


def shot(name, cfg, w=1080, h=645):
    build()
    path = os.path.join(OUT, name)
    url = 'file:///' + os.path.join(WORK, 'index.html').replace('\\', '/')
    url += '#' + q(json.dumps(cfg, ensure_ascii=False))
    cmd = [EDGE, '--headless=new', '--disable-gpu', '--no-sandbox', '--hide-scrollbars',
           '--allow-file-access-from-files', '--window-size=%d,%d' % (w, h),
           '--virtual-time-budget=12000', '--screenshot=' + path, url]
    subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=180)
    print(('OK   ' if os.path.exists(path) else 'FAIL ') + name,
          os.path.getsize(path) if os.path.exists(path) else '')


if __name__ == '__main__':
    os.makedirs(OUT, exist_ok=True)
    # 专辑页：未选中（先在曲库页选中第一首 → 切到专辑页，验证不残留）
    shot('fix-albums-empty.png', {'tab': 'albums'})
    # 专辑页：选中「Ignite」
    shot('fix-albums-selected.png', {'tab': 'albums', 'list': '#al-list', 'select': 'Ignite'})
    # 歌手页：未选中
    shot('fix-artists-empty.png', {'tab': 'artists'})
    # 歌手页：选中「Iselin Solheim」
    shot('fix-artists-selected.png', {'tab': 'artists', 'list': '#ar-list', 'select': 'Iselin Solheim'})
    # 播放列表页：未选中（三页行为一致的证据）
    shot('fix-playlists-empty.png', {'tab': 'playlists'})
