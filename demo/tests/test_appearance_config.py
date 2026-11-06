# -*- coding: utf-8 -*-
"""测试：外观 / 主题方案确实落盘到 app 级 kqk-app.toml 的 [ui] 段，且能重启读回。

背景（用户报告的 bug）：修复前外观偏好只写 localStorage('kqk_appearance')，
配置文件里完全没有对应字段；清掉 WebView 数据 / 换设备后主题就丢。

覆盖：
  1. 新配置 appearance.saved=false（供前端做旧偏好一次性迁移）；
  2. POST /api/ui-prefs 的 appearance 段落写入 [ui] 段，且保留注释 / 未知键；
  3. GET /api/status 与 /api/ui-prefs 返回一致；
  4. **重启服务后仍能读回**（真正持久化，而非只在内存里）；
  5. 非法值宽容回退；清除皮肤时旧值能被覆盖（空串而非字段消失）；
  6. 自定义皮肤图片本体（超大 data URL）绝不写进 TOML。
"""
import io
import json
import os
import shutil
import sys
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import common as C

A = C.Asserts('外观主题方案持久化')


def ok(cond, msg):
    return A.ok(cond, msg)


def http(base, path, method='GET', body=None):
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body).encode('utf-8')
        headers['Content-Type'] = 'application/json'
    req = urllib.request.Request(base + path, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read().decode('utf-8'))


def wait_up(base, tries=60):
    for _ in range(tries):
        try:
            return http(base, '/api/status')
        except Exception:
            time.sleep(0.25)
    return None


def main():
    if not C.ensure_fixture():
        ok(False, '夹具曲库不可用')
        return A.report()

    tmp = C.run_dir('appearance')
    exe_dir = os.path.join(tmp, 'app')
    os.makedirs(exe_dir, exist_ok=True)
    shutil.copyfile(C.pick_exe(), os.path.join(exe_dir, 'mt-cli.exe'))

    # 手写配置：带注释、未知段、未知键，验证「保留式就地更新」不被破坏。
    cfg = os.path.join(exe_dir, 'kqk-app.toml')
    lib_escaped = C.FIXTURE_LIB.replace('\\', '\\\\')
    with io.open(cfg, 'w', encoding='utf-8') as f:
        f.write(
            "# 用户手写注释\n"
            "last_library_dir = '%s'\n\n"
            "[meta]\nconfig_version = \"0.4.0\"\nmin_compatible_version = \"0.2.0\"\n\n"
            "[custom]\nmy_key = 42\n\n"
            "[ui]\n# ui 段注释\nmy_unknown_key = \"keep-me\"\n" % lib_escaped
        )

    proc = None
    try:
        # ---------- 1) 初始状态 ----------
        proc, port = C.start_server(exe_dir)
        base = 'http://127.0.0.1:%d' % port
        st0 = wait_up(base)
        ok(st0 is not None, '服务已启动并响应 /api/status')
        if st0 is None:
            return A.report()
        ap0 = st0.get('appearance') or {}
        ok(ap0.get('saved') is False, '新配置 appearance.saved=false（触发旧 localStorage 偏好迁移）')
        ok(ap0.get('theme') == 'system', '新配置默认主题 = system（%s）' % ap0.get('theme'))
        ok(ap0.get('skin_type') == '', '新配置默认无皮肤（%r）' % ap0.get('skin_type'))

        # ---------- 2) 保存一套完整主题方案 ----------
        want = {
            'theme': 'dark',
            'accent': '#7e57c2',
            'skin_type': 'preset',
            'skin_preset': 'purple',
            'skin_accent': '#9575cd',
        }
        res = http(base, '/api/ui-prefs', 'POST', {'appearance': want})
        ok(res.get('ok') is True, 'POST /api/ui-prefs 保存外观成功')
        ap1 = res.get('appearance') or {}
        ok(ap1.get('theme') == 'dark' and ap1.get('skin_preset') == 'purple',
           '写回响应包含新方案（%s）' % ap1)
        ok(ap1.get('saved') is True, '写回响应 appearance.saved=true')

        # ---------- 3) 配置文件里确实出现 ----------
        text = io.open(cfg, encoding='utf-8').read()
        ok('theme = "dark"' in text, '配置文件出现 theme = "dark"')
        ok('accent = "#7e57c2"' in text, '配置文件出现 accent = "#7e57c2"')
        ok('skin_type = "preset"' in text, '配置文件出现 skin_type = "preset"')
        ok('skin_preset = "purple"' in text, '配置文件出现 skin_preset = "purple"')
        ok('skin_accent = "#9575cd"' in text, '配置文件出现 skin_accent = "#9575cd"')
        ok('appearance_saved = true' in text, '配置文件出现 appearance_saved = true')
        ok('# 用户手写注释' in text and 'my_key = 42' in text
           and 'my_unknown_key = "keep-me"' in text and '# ui 段注释' in text,
           '就地更新保留注释 / 未知段 / 未知键')
        ok(len(text) < 8192, '配置文件保持轻量，未被大图片 data URL 撑大（%d 字节）' % len(text))
        ok('data:image' not in text and 'base64' not in text,
           '自定义皮肤图片本体未写入 TOML')

        import tomllib
        doc = tomllib.load(open(cfg, 'rb'))
        ok(doc['ui']['theme'] == 'dark', '落盘文件可被 TOML 解析且 theme 正确')
        ok(doc['ui']['appearance_saved'] is True, '落盘文件 appearance_saved 为 true')

        # ---------- 4) 接口读回 + 重启读回 ----------
        st1 = http(base, '/api/status')
        ok((st1.get('appearance') or {}).get('theme') == 'dark',
           '/api/status 立即读回 theme=dark')
        get1 = http(base, '/api/ui-prefs')
        ok((get1.get('appearance') or {}).get('skin_preset') == 'purple',
           '/api/ui-prefs GET 读回皮肤预设')

        C.stop_server(proc)
        proc = None
        proc, port2 = C.start_server(exe_dir)
        base2 = 'http://127.0.0.1:%d' % port2
        st2 = wait_up(base2)
        ok(st2 is not None, '重启后服务正常')
        if st2 is None:
            return A.report()
        ap2 = st2.get('appearance') or {}
        ok(ap2.get('theme') == 'dark', '重启后仍为 dark（真正落盘，非内存态）')
        ok(ap2.get('accent') == '#7e57c2', '重启后主题色仍为 #7e57c2')
        ok(ap2.get('skin_preset') == 'purple', '重启后预设皮肤仍为 purple')
        ok(ap2.get('saved') is True, '重启后 appearance.saved 仍为 true')

        # ---------- 5) 非法值宽容回退 ----------
        res3 = http(base2, '/api/ui-prefs', 'POST', {
            'appearance': {'theme': 'neon', 'accent': 'not-a-color', 'skin_type': 'bogus'},
        })
        ap3 = res3.get('appearance') or {}
        ok(ap3.get('theme') == 'system', '非法 theme 回退 system（%s）' % ap3.get('theme'))
        ok(ap3.get('accent') == '', '非法颜色被清空（%r）' % ap3.get('accent'))
        ok(ap3.get('skin_type') == '', '非法皮肤类型被清空（%r）' % ap3.get('skin_type'))

        # ---------- 6) 清除皮肤时旧值能被覆盖（关键：空串必须真的写进文件）----------
        res4 = http(base2, '/api/ui-prefs', 'POST', {
            'appearance': {
                'theme': 'light', 'accent': '#00897b',
                'skin_type': '', 'skin_preset': '', 'skin_accent': '',
            },
        })
        ap4 = res4.get('appearance') or {}
        ok(ap4.get('skin_type') == '' and ap4.get('skin_preset') == ''
           and ap4.get('skin_accent') == '', '清除皮肤后所有皮肤字段为空')
        text4 = io.open(cfg, encoding='utf-8').read()
        ok('skin_preset = ""' in text4 and 'skin_accent = ""' in text4,
           '清除动作确实覆盖了配置文件里的旧值（不是字段消失）')
        ok('theme = "light"' in text4 and 'accent = "#00897b"' in text4,
           '清除皮肤不影响主题模式与主题色')

    finally:
        C.stop_server(proc)

    return A.report()


if __name__ == '__main__':
    sys.exit(main())
