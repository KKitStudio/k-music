# -*- coding: utf-8 -*-
"""持久化测试夹具：构建/校验一份可复用的测试曲库（含 3 首带真实标签的 WAV）。

为什么需要它
------------
早先的真实端到端验证每次运行都会：rmtree 整个临时目录 → 重新合成 WAV →
重新初始化曲库 → 重新扫描入库。清理临时目录后这些产物一并消失，下一次又要重来。

这里把「素材 + 已建库并已入库的曲库」固化到 `demo/fixtures/` 下：
  * WAV 素材合成一次即可（含真实 RIFF INFO 标签，能被 lofty 解析出 title/artist/album）；
  * 曲库用 mt-cli 真正初始化 + 真正 scan 入库，DB 里的记录指向 fixtures 内的绝对路径；
  * 后续验证脚本**原地复用**该曲库（不可复制——DB 存绝对路径，复制会导致路径失效、
    被增量扫描判为孤儿并清理）。

用法
----
    python make_fixtures.py            # 幂等：缺失/损坏时构建，健康则直接复用
    python make_fixtures.py --force    # 强制重建（曲库整体删除重来）
    python make_fixtures.py --status   # 只打印当前状态，不做任何改动

注意
----
* fixtures/ 不在临时目录里 → 清理临时产物不会影响它。
* 若整个项目被移动到别的路径，DB 里的绝对路径会失效 → 执行 `--force` 重建即可。
"""
import json
import os
import shutil
import struct
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))          # demo/fixtures
DEMO = os.path.abspath(os.path.join(HERE, '..'))            # demo
ROOT = os.path.abspath(os.path.join(DEMO, '..'))            # 项目根

LIB = os.path.join(HERE, 'library')          # 测试曲库（固定绝对路径，原地复用）
BIN = os.path.join(HERE, 'bin')              # 夹具专用 exe + app 配置
EXE = os.path.join(BIN, 'mt-cli.exe')
APP_CFG = os.path.join(BIN, 'kqk-app.toml')
MANIFEST = os.path.join(HERE, 'manifest.json')

# 曲名刻意不与专辑名/歌手名重名，避免断言出现歧义。
TRACK_SPEC = [
    ('Alan Walker', 'Different World', 'Faded'),
    ('Alan Walker', 'Different World', 'Yellow Cold Rose'),
    ('Iselin Solheim', 'Ignite', 'Spectre'),
]
EXPECT_ALBUMS = ['Different World', 'Ignite']
EXPECT_ARTISTS = ['Alan Walker', 'Iselin Solheim']


def make_wav(path, title, artist, album, seconds=1, rate=8000):
    """合成带 RIFF INFO 标签（INAM/IART/IPRD）的 PCM WAV。"""
    n = rate * seconds
    data = b'\x00\x00' * n
    fmt = struct.pack('<HHIIHH', 1, 1, rate, rate * 2, 2, 16)

    def sub(key, val):
        b = val.encode('utf-8') + b'\x00'
        if len(b) % 2:
            b += b'\x00'
        return key.encode('ascii') + struct.pack('<I', len(b)) + b

    info_body = b'INFO' + sub('INAM', title) + sub('IART', artist) + sub('IPRD', album)
    info = b'LIST' + struct.pack('<I', len(info_body)) + info_body
    body = (b'WAVE'
            + b'fmt ' + struct.pack('<I', len(fmt)) + fmt
            + info
            + b'data' + struct.pack('<I', len(data)) + data)
    with open(path, 'wb') as f:
        f.write(b'RIFF' + struct.pack('<I', len(body)) + body)


def pick_exe():
    """优先 release，其次 debug。"""
    for rel in (('target', 'release', 'mt-cli.exe'), ('target', 'debug', 'mt-cli.exe')):
        p = os.path.join(ROOT, *rel)
        if os.path.isfile(p):
            return p
    return None


def wav_paths():
    out = []
    for artist, album, title in TRACK_SPEC:
        out.append(os.path.join(LIB, artist, album, title + '.wav'))
    return out


def is_healthy():
    """曲库是否可用：配置 + DB + 3 个 WAV 素材 + 清单一致。"""
    cfg = os.path.join(LIB, '.mt', 'config.toml')
    db = os.path.join(LIB, '.mt', 'library.db')
    if not (os.path.isfile(cfg) and os.path.isfile(db)):
        return False, '缺少 .mt/config.toml 或 .mt/library.db'
    for p in wav_paths():
        if not os.path.isfile(p):
            return False, '缺少音频素材：%s' % os.path.basename(p)
    if os.path.isfile(MANIFEST):
        with open(MANIFEST, encoding='utf-8') as f:
            m = json.load(f)
        if m.get('tracks') != len(TRACK_SPEC):
            return False, 'manifest 曲目数与预期不符'
    return True, 'OK'


def status():
    ok, why = is_healthy()
    print('夹具曲库：%s' % LIB)
    print('状态：%s（%s）' % ('健康' if ok else '不可用', why))
    for artist, album, title in TRACK_SPEC:
        p = os.path.join(LIB, artist, album, title + '.wav')
        print('  %-14s / %-16s / %-18s %s'
              % (artist, album, title, 'OK' if os.path.isfile(p) else '缺失'))
    print('提示：复用时必须原地使用（DB 存绝对路径），不要复制到别处。')
    return 0 if ok else 1


def build():
    src_exe = pick_exe()
    if not src_exe:
        print('!! 未找到 mt-cli.exe（先 cargo build）')
        return 2

    # 1) 清理旧曲库
    if os.path.isdir(LIB):
        shutil.rmtree(LIB, ignore_errors=True)
    os.makedirs(LIB, exist_ok=True)

    # 2) 合成 WAV 素材
    for artist, album, title in TRACK_SPEC:
        d = os.path.join(LIB, artist, album)
        os.makedirs(d, exist_ok=True)
        make_wav(os.path.join(d, title + '.wav'), title, artist, album)
    print('已合成 %d 个 WAV 素材' % len(TRACK_SPEC))

    # 3) 准备夹具专用 exe + app 配置（app 配置只用于让 scan 知道扫哪个库）
    os.makedirs(BIN, exist_ok=True)
    shutil.copyfile(src_exe, EXE)
    with open(APP_CFG, 'w', encoding='utf-8') as f:
        f.write("last_library_dir = '%s'\n" % LIB.replace('\\', '\\\\'))

    # 4) 真正初始化曲库
    p = subprocess.run([EXE, 'library', LIB, 'artist_album', 'K music测试夹具'],
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)
    out = p.stdout.decode('utf-8', 'ignore')
    if p.returncode != 0 or '.mt/config.toml' not in out:
        print('!! 曲库初始化失败：\n' + out)
        return 3

    # 5) 真正扫描入库
    p = subprocess.run([EXE, 'scan'], stdout=subprocess.PIPE,
                       stderr=subprocess.STDOUT, timeout=180)
    out = p.stdout.decode('utf-8', 'ignore')
    if p.returncode != 0:
        print('!! 扫描失败：\n' + out)
        return 4
    print('扫描：' + ' / '.join(l.strip() for l in out.splitlines() if l.strip()))

    # 6) 写清单
    with open(MANIFEST, 'w', encoding='utf-8') as f:
        json.dump({
            'library_dir': LIB,
            'tracks': len(TRACK_SPEC),
            'albums': EXPECT_ALBUMS,
            'artists': EXPECT_ARTISTS,
            'spec': [{'artist': a, 'album': b, 'title': t} for a, b, t in TRACK_SPEC],
        }, f, ensure_ascii=False, indent=2)

    ok, why = is_healthy()
    print('构建完成：%s（%s）' % ('健康' if ok else '不健康', why))
    return 0 if ok else 5


def main():
    if '--status' in sys.argv:
        return status()
    if '--force' in sys.argv:
        return build()
    ok, why = is_healthy()
    if ok:
        print('夹具已存在且健康，直接复用（%s）' % LIB)
        print('如需重建：python make_fixtures.py --force')
        return 0
    print('夹具不可用（%s），开始构建…' % why)
    return build()


if __name__ == '__main__':
    sys.exit(main() or 0)
