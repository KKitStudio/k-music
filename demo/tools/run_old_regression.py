# -*- coding: utf-8 -*-
"""回归对照：用「修复前」的 app.js（备份）跑同一套断言，验证 C 组确实会失败。"""
import html as htmllib
import io
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.dirname(HERE))
import common as C
ROOT = C.ROOT
SRC = C.FRONTEND_SRC
BACKUP = os.path.join(C.BACKUP_DIR, '2026-11-05-fix-stale-detail')
WORK = C.run_dir('preview_old')
EDGE = C.EDGE

os.makedirs(WORK, exist_ok=True)
# 旧 app.js（修复前）+ 当前 styles.css
shutil.copyfile(os.path.join(BACKUP, 'app.js'), os.path.join(WORK, 'app.js'))
shutil.copyfile(os.path.join(SRC, 'styles.css'), os.path.join(WORK, 'styles.css'))
# 旧 index.html（修复前的占位文案）
shutil.copyfile(os.path.join(BACKUP, 'index.html'), os.path.join(WORK, 'index.html'))

# 复用 gen_preview.py 生成的 mock+assert 片段
import gen_preview
with io.open(os.path.join(SRC, 'index.html'), encoding='utf-8') as f:
    html = f.read()
mock = (gen_preview.MOCK.replace('__TRACKS__', __import__('json').dumps(gen_preview.TRACKS, ensure_ascii=False))
        .replace('__ALBUMS__', __import__('json').dumps(gen_preview.ALBUMS, ensure_ascii=False))
        .replace('__ARTISTS__', __import__('json').dumps(gen_preview.ARTISTS, ensure_ascii=False))
        .replace('__PLAYLISTS__', __import__('json').dumps(gen_preview.PLAYLISTS, ensure_ascii=False)))
tag = '<script src="app.js"></script>'
assert tag in html
html = html.replace(tag, mock + '\n' + tag + '\n' + gen_preview.ASSERT)
with io.open(os.path.join(WORK, 'index.html'), 'w', encoding='utf-8') as f:
    f.write(html)

out = os.path.join(WORK, 'dom_old.txt')
url = 'file:///' + os.path.join(WORK, 'index.html').replace(os.sep, '/')
cmd = [EDGE, '--headless=new', '--disable-gpu', '--no-sandbox',
       '--allow-file-access-from-files', '--window-size=1080,645',
       '--virtual-time-budget=30000', '--dump-dom', url]
with io.open(out, 'w', encoding='utf-8') as f:
    subprocess.run(cmd, stdout=f, stderr=subprocess.PIPE, timeout=180)
with io.open(out, encoding='utf-8') as f:
    dom = f.read()
m = re.search(r'<pre id="ASSERT">(.*?)</pre>', dom, re.S)
if not m:
    print('!! 未找到 ASSERT 块')
    raise SystemExit(2)
txt = htmllib.unescape(m.group(1))
lines = [l for l in txt.splitlines() if l.strip()]
c_all = [l for l in lines if l[6:].strip().startswith('C')]
c_fail = [l for l in c_all if l.startswith('FAIL')]
print('旧代码（修复前）下 C 组断言失败数：%d / %d' % (len(c_fail), len(c_all)))
for l in c_fail:
    print('  ' + l)
