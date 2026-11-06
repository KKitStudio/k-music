# -*- coding: utf-8 -*-
"""测试：/api/focus 端点（单实例第二进程唤起已有窗口的入口）。

边界：命名互斥体、端口文件与真实窗口置前只发生在 GUI 二进制进程内（发布名 k_music.exe），
无头环境无法覆盖（真机验证见 docs/07 P17）。这里覆盖「已有实例可被调用」的那一半：
端点存在、返回 ok、CLI serve 未注册 focus sink 时优雅空转（不 500 / 不 panic）。

注意：运行前需先 `cargo build --release -p mt-cli`，否则测到的是旧 exe。
"""
import io
import json
import os
import shutil
import sys
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import common as C

A = C.Asserts('/api/focus 单实例唤起端点')
BASE = 'http://127.0.0.1:1'


def ok(cond, msg):
    return A.ok(cond, msg)


def http_raw(path, timeout=10):
    req = urllib.request.Request(BASE + path, method='GET')
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.status, r.read().decode('utf-8')


def main():
    if not C.ensure_fixture():
        ok(False, '夹具曲库不可用')
        return A.report()

    global BASE
    tmp = C.run_dir('single-instance')
    exe_dir = os.path.join(tmp, 'app')
    os.makedirs(exe_dir, exist_ok=True)
    shutil.copyfile(C.pick_exe(), os.path.join(exe_dir, 'mt-cli.exe'))
    # 与 GUI 同源的 app 级配置；端口沿用 common.start_server 的分配方式。
    cfg = os.path.join(exe_dir, 'kqk-app.toml')
    lib_escaped = C.FIXTURE_LIB.replace('\\', '\\\\')
    with io.open(cfg, 'w', encoding='utf-8') as f:
        f.write(
            "# 单实例端点冒烟\n"
            "last_library_dir = '%s'\n\n"
            "[meta]\nconfig_version = \"0.4.0\"\nmin_compatible_version = \"0.2.0\"\n" % lib_escaped
        )

    proc = None
    try:
        proc, port = C.start_server(exe_dir)
        BASE = 'http://127.0.0.1:%d' % port
        status, body = http_raw('/api/focus')
        ok(status == 200, 'GET /api/focus 返回 200（%r）' % status)
        data = json.loads(body)
        ok(data.get('ok') is True, '响应体 ok=true（CLI 未注册 sink 时优雅空转）')
        # 端点必须落在 JSON API 上，不能被 ServeDir 静态回退成前端 HTML
        ok('<!DOCTYPE' not in body and '<html' not in body,
           '未被静态文件 fallback 截获，返回的是 JSON 而非前端 HTML')
    finally:
        C.stop_server(proc)
    return A.report()


if __name__ == '__main__':
    sys.exit(main())
