# -*- coding: utf-8 -*-
"""清理 demo 的运行产物目录（demo/.run/）。

背景：每次运行用例会新建一个带时间戳的工作目录（含 exe 副本与前端副本，约 7MB），
累积起来会占用较多磁盘。本脚本负责清理。

用法
----
    python demo/tools/clean_run.py           # 清理全部运行产物
    python demo/tools/clean_run.py --keep 2  # 只保留最近 2 次（按时间戳排序）

注意：**不会**触碰 `demo/fixtures/`（持久化夹具）。

实现要点
--------
Windows 上被占用的 exe（如 mt-cli.exe 仍在运行）会导致 rmtree 抛 PermissionError。
早期版本用 `ignore_errors=True` 会**静默失败**却照样打印「已清理」，造成磁盘占用假象。
现在改为：onerror 回调强制清只读属性并重试；失败项如实报告「清理失败」。
"""
import os
import shutil
import stat
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
DEMO = os.path.dirname(HERE)
RUN = os.path.join(DEMO, '.run')

# 不清理这些（它们不是"运行产物"，而是可复用的准备产物）
KEEP_NAMES = {'preview'}


def _on_rm_error(func, path, exc_info):
    """rmtree 失败时的补救：清只读属性后重试一次。"""
    try:
        os.chmod(path, stat.S_IWRITE)
        func(path)
    except Exception:
        pass


def dir_size(path):
    size = 0
    for root, _, files in os.walk(path):
        for f in files:
            try:
                size += os.path.getsize(os.path.join(root, f))
            except OSError:
                pass
    return size


def remove_dir(path):
    """删除目录，返回 (是否成功, 释放字节数)。"""
    size = dir_size(path)
    shutil.rmtree(path, onerror=_on_rm_error)
    return (not os.path.exists(path)), size


def remove_file(path):
    try:
        size = os.path.getsize(path)
    except OSError:
        size = 0
    try:
        os.remove(path)
    except OSError:
        return False, 0
    return True, size


def main():
    keep = 0
    if '--keep' in sys.argv:
        i = sys.argv.index('--keep')
        if i + 1 < len(sys.argv):
            keep = int(sys.argv[i + 1])

    if not os.path.isdir(RUN):
        print('运行产物目录不存在，无需清理：%s' % RUN)
        return 0

    # 0) 先干掉运行目录下的孤儿 mt-cli.exe（否则 exe 被占用 → 目录删不掉）
    try:
        sys.path.insert(0, DEMO)
        import common as C
        killed = C.kill_stale_servers()
        if killed:
            print('已结束 %d 个运行目录下的残留服务进程。' % killed)
    except Exception as e:
        print('（残留进程清理跳过：%s）' % e)

    # 1) 先处理散落在 .run 根目录的旧文件（DOM dump / *.new 等历史手工产物）
    for name in sorted(os.listdir(RUN)):
        p = os.path.join(RUN, name)
        if not os.path.isfile(p):
            continue
        ok, size = remove_file(p)
        print('%s：%s（%.1f KB）' % ('已清理' if ok else '清理失败', name, size / 1024.0))

    # 2) 再处理目录
    items = []
    for name in os.listdir(RUN):
        path = os.path.join(RUN, name)
        if not os.path.isdir(path):
            continue
        if name in KEEP_NAMES:
            print('跳过（保留项）：%s' % name)
            continue
        items.append((os.path.getmtime(path), name, path))

    items.sort(reverse=True)
    if keep > 0 and len(items) > keep:
        victims = items[keep:]
        items = items[:keep]
    else:
        victims = items
        items = []

    freed = 0
    failed = []
    for _, name, path in victims:
        ok, size = remove_dir(path)
        if ok:
            freed += size
            print('已清理：%s（%.1f MB）' % (name, size / 1048576.0))
        else:
            failed.append(name)
            print('清理失败（文件被占用）：%s（%.1f MB）→ 关闭占用该 exe 的进程后重试'
                  % (name, size / 1048576.0))

    if items:
        print('保留最近 %d 个：%s' % (len(items), ', '.join(n for _, n, _ in items)))
    print('共释放 %.1f MB。夹具 demo/fixtures/ 未被触碰。' % (freed / 1048576.0))
    if failed:
        print('仍有 %d 个目录未清理：%s' % (len(failed), ', '.join(failed)))
    return 0


if __name__ == '__main__':
    sys.exit(main() or 0)
