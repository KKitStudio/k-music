#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""K music 多语言骨架离线校验（不依赖第三方包、不修改任何文件）。

用法：python app/gui/scripts/check_i18n.py
退出码：0 = 全部通过；1 = 存在 FAIL。

检查项：
  1. lang/zh-CN.js 字典可解析、key 为点分语义命名空间、无重复 key；
  2. index.html 的 data-i18n 与 core.js / app.js 的 t() 引用的 key 全部存在；
  3. 字典中不存在未被使用的条目；
  4. 示范模块 #ap-tray-section 内每行中文文案都带 data-i18n（分批迁移的质量闸门）。

渐进迁移说明：当前示范模块仍保留中文原文作首屏兜底，故第 4 项只检查
「中文行是否挂了 data-i18n」；待字典覆盖足够、移除兜底文本后再切换严格模式。
"""
from __future__ import print_function

import io
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
FRONTEND = os.path.normpath(os.path.join(HERE, '..', 'frontend'))
DICT_PATH = os.path.join(FRONTEND, 'lang', 'zh-CN.js')
HTML_PATH = os.path.join(FRONTEND, 'index.html')
JS_FILES = [os.path.join(FRONTEND, n) for n in ('core.js', 'app.js')]
PILOT_MARKER = 'id="ap-tray-section"'

ENTRY_RE = re.compile(r"^\s*'([A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+)'\s*:\s*'(.*?)'\s*,?\s*$")
KEY_SHAPE_RE = re.compile(r'^[a-z][a-z0-9_]*(?:\.[a-z0-9_]+)+$')
HAN_RE = re.compile(r'[\u4e00-\u9fff]')
DATA_I18N_RE = re.compile(r'data-i18n="([^"]+)"')
T_CALL_RE = re.compile(r"\bt\(\s*'([^']+)'")


def read(path):
    with io.open(path, encoding='utf-8') as f:
        return f.read()


def parse_dictionary(text):
    entries = []
    for line in text.splitlines():
        m = ENTRY_RE.match(line)
        if m:
            entries.append((m.group(1), m.group(2)))
    return entries


def main():
    failures = []
    checks = []

    # 1. 字典合法性
    if not os.path.isfile(DICT_PATH):
        print('FAIL 字典文件不存在：%s' % DICT_PATH)
        return 1
    entries = parse_dictionary(read(DICT_PATH))
    if not entries:
        failures.append('字典未解析出任何条目（检查 zh-CN.js 的 key/value 行格式）')
    keys = [k for k, _ in entries]
    dup = sorted({k for k in keys if keys.count(k) > 1})
    bad_shape = [k for k in keys if not KEY_SHAPE_RE.match(k)]
    if dup:
        failures.append('字典存在重复 key：%s' % ', '.join(dup))
    if bad_shape:
        failures.append('key 不符合点分语义命名空间：%s' % ', '.join(bad_shape))
    checks.append('字典条目 %d 条 / 重复 %d / 非法 key %d'
                  % (len(entries), len(dup), len(bad_shape)))

    # 2. 引用覆盖与未使用条目
    html = read(HTML_PATH)
    html_refs = DATA_I18N_RE.findall(html)
    js_refs = []
    for path in JS_FILES:
        if os.path.isfile(path):
            js_refs.extend(T_CALL_RE.findall(read(path)))
    used = set(html_refs + js_refs)
    dict_keys = set(keys)
    absent = sorted(used - dict_keys)
    unused = sorted(dict_keys - used)
    if absent:
        failures.append('代码引用了字典中不存在的 key：%s' % ', '.join(absent))
    if unused:
        failures.append('字典中未被使用的 key：%s' % ', '.join(unused))
    checks.append('引用 key %d 个（HTML %d / JS %d）/ 缺失 %d / 未使用 %d'
                  % (len(used), len(html_refs), len(js_refs), len(absent), len(unused)))

    # 3. 示范模块残留写死中文
    if PILOT_MARKER not in html:
        failures.append('示范模块标记未找到：%s' % PILOT_MARKER)
    else:
        block = html.split(PILOT_MARKER, 1)[1].split('</div><!-- /pane: appearance -->', 1)[0]
        chinese_lines = [l for l in block.splitlines() if HAN_RE.search(l)]
        leaked = []
        for line in chinese_lines:
            stripped = line.strip()
            if stripped.startswith('<!--') or stripped.startswith('//'):
                continue
            if 'data-i18n=' not in line:
                leaked.append(stripped[:60])
        if leaked:
            failures.append('示范模块残留写死中文（缺 data-i18n）：%d 行 -> %s'
                            % (len(leaked), ' | '.join(leaked[:3])))
        checks.append('示范模块中文行 %d / 漏挂 data-i18n %d'
                      % (len(chinese_lines), len(leaked)))

    for line in checks:
        print('INFO ' + line)
    if failures:
        for line in failures:
            print('FAIL ' + line)
        print('RESULT FAIL (%d)' % len(failures))
        return 1
    print('RESULT PASS')
    return 0


if __name__ == '__main__':
    sys.exit(main())
