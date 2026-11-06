# -*- coding: utf-8 -*-
"""demo 测试公共模块：路径常量、夹具接入、断言收集、无头浏览器驱动。

所有测试脚本统一从这里取路径，避免各脚本各写一份（且容易写错层级）。

目录约定
--------
    demo/
      common.py          ← 本模块（公共路径 / 工具）
      fixtures/          ← 持久化测试夹具（曲库 + 素材），不在临时目录，清理不受影响
        make_fixtures.py
        library/         ← 已初始化并入库的测试曲库（固定绝对路径，原地复用）
      tests/             ← 测试用例（pytest 风格的可执行脚本）
      tools/             ← 辅助工具（截图、诊断、回归对照）
      .run/              ← 运行产物（临时，可随时删除）
"""
import html as htmllib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import time

# ---------- 路径 ----------
DEMO = os.path.dirname(os.path.abspath(__file__))            # demo/
ROOT = os.path.abspath(os.path.join(DEMO, '..'))              # 项目根
FIXTURES = os.path.join(DEMO, 'fixtures')
FIXTURE_LIB = os.path.join(FIXTURES, 'library')              # 夹具曲库（固定绝对路径）
RUN = os.path.join(DEMO, '.run')                             # 运行产物目录
FRONTEND_SRC = os.path.join(ROOT, 'app', 'gui', 'frontend')  # 前端源码
DIST_FRONTEND = os.path.join(ROOT, 'dist', 'frontend')        # 打包后的前端
RELEASE_EXE = os.path.join(ROOT, 'target', 'release', 'mt-cli.exe')
DEBUG_EXE = os.path.join(ROOT, 'target', 'debug', 'mt-cli.exe')
BACKUP_DIR = os.path.join(ROOT, '.workbuddy', 'backups')

EDGE = r'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'


def ensure_fixture():
    """确保夹具曲库可用；缺失/损坏则现场构建（幂等）。返回是否可用。"""
    sys.path.insert(0, FIXTURES)
    import make_fixtures
    if make_fixtures.is_healthy()[0]:
        return True
    print('夹具缺失或损坏，正在构建…')
    return make_fixtures.build() == 0


def run_dir(name):
    """为一次运行分配独立的工作目录（带时间戳）。

    刻意**不使用** `shutil.rmtree` 清理旧目录：
      · Windows 上 exe 可能仍被占用，rmtree 会抛 PermissionError；
      · 某些环境会拦截批量删除。
    每次用新目录即可，旧的留在 demo/.run/ 下由人工或后续清理脚本处理。
    """
    stamp = time.strftime('%Y%m%d-%H%M%S')
    d = os.path.join(RUN, '%s-%s' % (name, stamp))
    os.makedirs(d, exist_ok=True)
    return d


def pick_exe():
    """优先 release，其次 debug。"""
    for p in (RELEASE_EXE, DEBUG_EXE):
        if os.path.isfile(p):
            return p
    return None


# ---------- 断言收集 ----------
class Asserts(object):
    """收集断言结果，最后统一汇报。"""

    def __init__(self, title):
        self.title = title
        self.rows = []

    def ok(self, cond, msg):
        line = ('PASS' if cond else 'FAIL') + ' | ' + msg
        self.rows.append(line)
        print(line)
        return bool(cond)

    def eq(self, got, want, msg):
        return self.ok(got == want, '%s（实际 %r，期望 %r）' % (msg, got, want))

    @property
    def failed(self):
        return [r for r in self.rows if r.startswith('FAIL')]

    def report(self):
        print('=' * 64)
        print('【%s】总计 %d 项断言，失败 %d 项' % (self.title, len(self.rows), len(self.failed)))
        for r in self.failed:
            print('  ' + r)
        return 1 if self.failed else 0


# ---------- 无头 Edge ----------
def edge_dump_dom(html_path, out_file, window='1080,645', budget=30000, url=None):
    """用无头 Edge 打开页面并把最终 DOM dump 到文件。返回 DOM 文本。"""
    cmd = [EDGE, '--headless=new', '--disable-gpu', '--no-sandbox',
           '--allow-file-access-from-files', '--window-size=' + window,
           '--virtual-time-budget=%d' % budget, '--dump-dom',
           url or ('file:///' + os.path.abspath(html_path).replace('\\', '/'))]
    with io.open(out_file, 'w', encoding='utf-8') as f:
        subprocess.run(cmd, stdout=f, stderr=subprocess.PIPE, timeout=300)
    with io.open(out_file, encoding='utf-8') as f:
        return f.read()


def edge_screenshot(url, out_file, window='1280,800', budget=9000, user_data_dir=None):
    """用无头 Edge 给指定 URL 截图（真实 serve 或 file:// 均可），返回图片路径。

    user_data_dir 传独立目录可避免多次截图之间复用磁盘缓存（同一 URL 会拿到旧 HTML）。
    """
    cmd = [EDGE, '--headless=new', '--disable-gpu', '--no-sandbox',
           '--hide-scrollbars', '--allow-file-access-from-files',
           '--window-size=' + window, '--virtual-time-budget=%d' % budget,
           '--screenshot=' + os.path.abspath(out_file),
           url or '']
    if user_data_dir:
        cmd.insert(1, '--user-data-dir=' + os.path.abspath(user_data_dir))
    subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=300)
    return out_file if os.path.isfile(out_file) else None


def extract_assert_block(dom, block_id='ASSERT'):
    """从 dump 出的 DOM 中取出 <pre id="..."> 里的断言文本。"""
    m = re.search(r'<pre id="%s">(.*?)</pre>' % re.escape(block_id), dom, re.S)
    if not m:
        return None
    return htmllib.unescape(m.group(1))


def merge_assert_results(a, text):
    """把页面内断言文本并入 Asserts 收集器。"""
    if text is None:
        a.ok(False, '未取到断言结果（页面脚本未执行完）')
        return
    for line in text.splitlines():
        if line.strip():
            a.ok(line.startswith('PASS'), line)


# ---------- 真实后端 ----------
def start_server(exe_dir, cwd=None, timeout=30):
    """启动 mt-cli serve，返回 (proc, port)。

    ⚠️ 端口解析阶段一旦抛错（例如脚本里漏了 import re），Popen 出来的进程会成为
    孤儿并一直占用 exe 文件（导致后续无法清理运行目录）。这里做失败即回收。
    """
    proc = subprocess.Popen([os.path.join(exe_dir, 'mt-cli.exe'), 'serve'],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            cwd=cwd)
    try:
        port = None
        deadline = time.time() + timeout
        while time.time() < deadline:
            line = proc.stdout.readline()
            if not line:
                break
            m = re.search(r'http://127\.0\.0\.1:(\d+)/', line.decode('utf-8', 'ignore'))
            if m:
                port = int(m.group(1))
                break
        return proc, port
    except Exception:
        stop_server(proc)
        raise


def stop_server(proc):
    """停止服务进程（幂等）。terminate 失败时退回 kill。"""
    if not proc:
        return
    try:
        proc.terminate()
        proc.wait(timeout=10)
    except Exception:
        try:
            proc.kill()
            proc.wait(timeout=10)
        except Exception:
            pass


def kill_stale_servers():
    """清理「运行目录下」遗留的 mt-cli.exe 孤儿进程。

    只按 **可执行文件路径位于 demo/.run/ 下** 来判断，不会误杀用户自己启动的服务
    或其他位置的 mt-cli。返回被清理的进程数。
    """
    import subprocess as sp
    run_abs = os.path.abspath(RUN).lower()
    ps = ('Get-CimInstance Win32_Process -Filter "Name=\'mt-cli.exe\'" | '
          'ForEach-Object { "$($_.ProcessId)|$($_.ExecutablePath)" }')
    try:
        out = sp.run(['powershell', '-NoProfile', '-Command', ps],
                     stdout=sp.PIPE, stderr=sp.DEVNULL, timeout=60).stdout.decode(
                         'utf-8', 'ignore')
    except Exception:
        return 0
    killed = 0
    for line in out.splitlines():
        line = line.strip()
        if '|' not in line:
            continue
        pid, path = line.split('|', 1)
        pid = pid.strip()
        path = path.strip()
        if not pid.isdigit() or not path:
            continue
        if os.path.abspath(path).lower().startswith(run_abs):
            try:
                sp.run(['taskkill', '/F', '/PID', pid], stdout=sp.DEVNULL,
                       stderr=sp.DEVNULL, timeout=20)
                killed += 1
            except Exception:
                pass
    return killed


def http_get(base, path, timeout=10):
    import urllib.request
    with urllib.request.urlopen(base + path, timeout=timeout) as r:
        return json.loads(r.read().decode('utf-8'))


def wait_for_tracks(base, want=3, tries=120, interval=0.25):
    """等待后端索引到指定数量的曲目。"""
    tracks = []
    for _ in range(tries):
        try:
            tracks = http_get(base, '/api/tracks').get('tracks') or []
            if len(tracks) >= want:
                break
        except Exception:
            pass
        time.sleep(interval)
    return tracks


# ---------- 前端注入 ----------
def build_frontend_copy(dest_dir, inject_js='', from_dir=None):
    """复制前端到目标目录，可选注入额外脚本（app.js / styles.css 一字不改）。"""
    src = from_dir or DIST_FRONTEND
    if not os.path.isdir(src):
        raise RuntimeError('前端目录不存在：%s（先执行 cargo build + tools/package_gui.bat）' % src)
    if os.path.isdir(dest_dir):
        # 不整目录删除（可能被占用 / 被环境拦截），逐文件覆盖即可。
        for name in os.listdir(dest_dir):
            s_ = os.path.join(src, name)
            d_ = os.path.join(dest_dir, name)
            if os.path.isfile(s_):
                shutil.copyfile(s_, d_)
    else:
        shutil.copytree(src, dest_dir)
    if inject_js:
        idx = os.path.join(dest_dir, 'index.html')
        with io.open(idx, encoding='utf-8') as f:
            html = f.read()
        tag = '<script src="app.js"></script>'
        if tag not in html:
            raise RuntimeError('index.html 中未找到脚本标签：%s' % tag)
        html = html.replace(tag, tag + '\n' + inject_js, 1)
        with io.open(idx, 'w', encoding='utf-8') as f:
            f.write(html)
    return dest_dir
