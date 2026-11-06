# -*- coding: utf-8 -*-
"""诊断：列宽分配的几何一致性。"""
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
SRC = C.FRONTEND_SRC
OUT = os.path.join(C.RUN, 'diag')
os.makedirs(OUT, exist_ok=True)
for f in ('styles.css', 'app.js'):
    shutil.copyfile(os.path.join(SRC, f), os.path.join(OUT, f))

TRACKS = [
    {"local_id": 1, "name": "Long Song Title Here", "artist": "Long Artist Name",
     "album": "Long Album Name", "quality_tier": "Hi-Res", "duration_ms": 245000,
     "file_size": 45678901, "format": "FLAC", "sample_rate": 96000, "bit_depth": 24,
     "bitrate": 1411000, "channels": 2},
]

MOCK = """
<script>
const MOCK_TRACKS = %s;
window.__POSTS = [];
window.fetch = function (url, opts) {
  opts = opts || {};
  if (opts.method === 'POST') { window.__POSTS.push({url:String(url), body: JSON.parse(opts.body||'{}')}); return Promise.resolve({ok:true, json:()=>Promise.resolve({ok:true})}); }
  let data = {};
  const u = String(url);
  if (u.indexOf('/api/status') === 0) data = { library_configured:true, library_dir:'D:/m', library_name:'t',
    track_columns:['name','artist','album','quality_tier','duration_ms','file_size'],
    layout_styles:{library:1,playlists:2,albums:2,artists:2},
    column_widths:{name:30,artist:20,album:22,quality_tier:8,duration_ms:8,file_size:12},
    app_version:'0.3.0', ncm_ready:false, pending_ncm:0, loose_ncm:[] };
  else if (u.indexOf('/api/tracks') === 0) data = { tracks: MOCK_TRACKS };
  else if (u.indexOf('/api/track/') === 0) data = { track: MOCK_TRACKS[0] };
  else data = {};
  return Promise.resolve({ok:true, json:()=>Promise.resolve(data)});
};
</script>
""" % json.dumps(TRACKS, ensure_ascii=False)

DIAG = """
<script>
(async () => {
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  for (let i=0;i<80;i++){ if(document.querySelectorAll('#track-body tr').length) break; await wait(50); }
  await wait(500);
  const t = document.getElementById('track-table');
  const ths = Array.from(t.querySelectorAll('thead th'));
  const rows = ths.map(th => ({
    key: th.dataset.colKey || ('FIXED:' + th.className),
    styleW: th.style.width || '(none)',
    rectW: Math.round(th.getBoundingClientRect().width * 100) / 100,
  }));
  const sumBiz = ths.filter(th=>th.dataset.colKey).reduce((a,th)=>a+th.getBoundingClientRect().width,0);
  const sumFixed = ths.filter(th=>!th.dataset.colKey).reduce((a,th)=>a+th.getBoundingClientRect().width,0);
  const info = {
    tableRect: Math.round(t.getBoundingClientRect().width*100)/100,
    tableOffsetWidth: t.offsetWidth,
    sumBiz: Math.round(sumBiz*100)/100,
    sumFixed: Math.round(sumFixed*100)/100,
    sumAll: Math.round((sumBiz+sumFixed)*100)/100,
    containerW: document.getElementById('library-scroll').clientWidth,
    rows: rows,
  };
  const pre = document.createElement('pre');
  pre.id = 'DIAG';
  pre.textContent = JSON.stringify(info, null, 1);
  document.body.appendChild(pre);
})();
</script>
"""

with io.open(os.path.join(SRC, 'index.html'), encoding='utf-8') as f:
    html = f.read()
tag = '<script src="app.js"></script>'
html = html.replace(tag, MOCK + '\n' + tag + '\n' + DIAG)
with io.open(os.path.join(OUT, 'index.html'), 'w', encoding='utf-8') as f:
    f.write(html)

EDGE = r'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'
cmd = [EDGE, '--headless=new', '--disable-gpu', '--no-sandbox', '--allow-file-access-from-files',
       '--window-size=1080,645', '--virtual-time-budget=10000', '--dump-dom',
       'file:///' + os.path.join(OUT, 'index.html').replace('\\', '/')]
p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
dom = p.stdout.decode('utf-8', 'ignore')
m = re.search(r'<pre id="DIAG">(.*?)</pre>', dom, re.S)
print(m.group(1) if m else 'NO DIAG BLOCK')
