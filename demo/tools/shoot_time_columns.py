# -*- coding: utf-8 -*-
"""生成「导入时间默认显示 / 创建时间默认隐藏 / 设置面板开关」三张效果图。

与纯 mock 的 shoot.py 不同：这里用**真实** mt-cli serve + 真实 SQLite 夹具曲库 +
真实 dist/frontend，只在前端额外注入一段「把界面摆到某个状态」的驱动脚本。

输出（docs/screenshots/）：
  time-columns-library.png   默认曲库列表：有导入时间、无创建时间
  time-columns-settings.png  设置 → 列表：创建时间默认未勾选
  time-columns-enabled.png   勾选并应用后：创建时间列出现
"""
import io
import os
import shutil
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import common as C

OUT = os.path.join(C.ROOT, 'docs', 'screenshots')

DRIVER = r"""
<script>
(async () => {
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  const MODE = '__MODE__';
  for (let i = 0; i < 100; i++) {
    if (document.querySelectorAll('#track-body tr').length >= 3) break;
    await wait(50);
  }
  await wait(350);
  if (MODE === 'settings' || MODE === 'enabled') {
    document.getElementById('btn-settings').click();
    await wait(200);
    const pane = document.querySelector('.settings-tab[data-tab="columns"]');
    if (pane) pane.click();
    await wait(250);
    const list = document.getElementById('col-config-list');
    if (list) list.scrollIntoView({ block: 'center' });
    await wait(250);
  }
  if (MODE === 'enabled') {
    const li = document.querySelector('#col-config-list li[data-key="created_at"]');
    const cb = li && li.querySelector('input[data-act="vis"]');
    if (cb) { cb.checked = true; cb.dispatchEvent(new Event('change', { bubbles: true })); }
    document.getElementById('set-apply-columns').click();
    await wait(700);
    document.getElementById('settings-close').click();
    await wait(250);
  }
  const mark = document.createElement('div');
  mark.id = 'SHOTREADY';
  mark.style.cssText = 'position:fixed;left:-9999px;top:0';
  document.body.appendChild(mark);
})();
</script>
"""


def main():
    if not C.ensure_fixture():
        print('夹具曲库不可用')
        return 2
    exe = C.pick_exe()
    if not exe:
        print('未找到 mt-cli.exe（先 cargo build）')
        return 2
    os.makedirs(OUT, exist_ok=True)
    run_dir = C.run_dir('shoot_time_columns')

    shots = [
        ('library', 'time-columns-library.png'),
        ('settings', 'time-columns-settings.png'),
        ('enabled', 'time-columns-enabled.png'),
    ]
    rc = 0
    for mode, name in shots:
        # ⚠️ 每个状态都用独立的 app 目录，且**必须先建好 frontend 再启动服务**：
        # resolve_frontend_dir() 只在启动时探测一次，若那时目录还不存在就会回退到
        # 项目源码里的 app/gui/frontend，注入的驱动脚本便永远不会被执行。
        app_dir = os.path.join(run_dir, 'app-' + mode)
        front = os.path.join(app_dir, 'frontend')
        os.makedirs(app_dir, exist_ok=True)
        shutil.copyfile(exe, os.path.join(app_dir, 'mt-cli.exe'))
        C.build_frontend_copy(front, inject_js=DRIVER.replace('__MODE__', mode))
        app_cfg = os.path.join(app_dir, 'kqk-app.toml')
        # 全新配置：不加 [ui]，让后端给出「导入时间显示 / 创建时间隐藏」的默认。
        with io.open(app_cfg, 'w', encoding='utf-8') as f:
            f.write("last_library_dir = '%s'\n" % C.FIXTURE_LIB.replace('\\', '\\\\'))

        proc = None
        try:
            proc, port = C.start_server(app_dir)
            if not port:
                print('FAIL %s：服务启动失败' % name)
                rc = 1
                continue
            base = 'http://127.0.0.1:%d' % port
            C.wait_for_tracks(base, want=3)
            path = os.path.join(OUT, name)
            got = C.edge_screenshot(
                base + '/', path, window='1280,800', budget=9000,
                user_data_dir=os.path.join(run_dir, 'edge-' + mode))
            if got:
                print('OK   %-32s %d bytes' % (name, os.path.getsize(got)))
            else:
                print('FAIL %s' % name)
                rc = 1
        finally:
            C.stop_server(proc)
    return rc


if __name__ == '__main__':
    sys.exit(main())
