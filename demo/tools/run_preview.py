# -*- coding: utf-8 -*-
"""跑无头 Edge，提取 <pre id="ASSERT"> 断言结果。"""
import io
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import common as C

EDGE = C.EDGE
HERE = os.path.dirname(os.path.abspath(__file__))
PREVIEW = os.path.join(C.RUN, 'preview', 'index.html')
SIZES = [(1080, 645), (900, 600), (1280, 800)]


def run(w, h, outdir):
    out = os.path.join(outdir, 'dom_%dx%d.txt' % (w, h))
    dom = C.edge_dump_dom(PREVIEW, out, window='%d,%d' % (w, h), budget=30000)
    text = C.extract_assert_block(dom)
    if text is None:
        return None, dom
    return text, dom


def main():
    allok = True
    outdir = C.run_dir('dom')
    print('DOM 输出目录：%s' % outdir)
    for (w, h) in SIZES:
        print('=' * 60)
        print('窗口尺寸 %dx%d' % (w, h))
        print('=' * 60)
        try:
            txt, dom = run(w, h, outdir)
        except Exception as e:
            print('RUN ERROR:', e)
            allok = False
            continue
        if txt is None:
            print('!! 未找到 ASSERT 块（脚本可能未执行完）')
            allok = False
            continue
        for line in txt.splitlines():
            print(line)
            if line.startswith('FAIL'):
                allok = False
        print()
    print('=' * 60)
    print('总体：', 'ALL PASS' if allok else 'HAS FAIL')
    return 0 if allok else 1


if __name__ == '__main__':
    sys.exit(main())
