"""K music 应用图标生成管线（可复用，勿删）。

用途：把 `app/gui/icons/*.svg` 渲染为带透明通道的 PNG + 多尺寸 ICO。
之所以不依赖 PIL / ImageMagick / rsvg：本环境未安装这些工具，改用
Windows 自带 Edge 无头渲染 SVG，再由本脚本完成抠图与编码。

用法：
    python tools/icon_pipeline.py [源SVG] [输出PNG] [输出ICO]
    # 默认: app/gui/icons/icon.svg -> icon.png / icon.ico

管线：SVG --(Edge 无头渲染到品红键色背景)--> PNG --(软抠图/反混色)-->
      RGBA --(面积平均降采样)--> 512 PNG + 256/128/64/48/32/16 ICO

关键约束（历史翻车点）：
  1. Edge 截图可能是 color type 2 (RGB) 而非 6 (RGBA)，read_png 必须按 IHDR 的
     ctype 展开，不可硬编码 stride = w*4。
  2. 抠图必须用「反混色」还原真实色，硬阈值会留下品红粉边/锯齿。
  3. ICO 内嵌 BITMAPINFOHEADER 必须严格 40 字节，否则目录项会全部指向同一帧、
     Windows 也无法正确解码。
  4. 设计上不得回到高饱和蓝底 + 纯白粗黑体（v1 之前两版被用户否决）。
"""

import os
import struct
import subprocess
import sys
import zlib

EDGE = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
KEY = (255, 0, 255)  # 品红键色
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


# ---------------------------------------------------------------- PNG 读写

def read_png(path):
    """读取 PNG，返回 (w, h, RGBA bytearray)。支持 ctype 2/6/0/4 与 filter 0-4。"""
    d = open(path, "rb").read()
    pos, w, h, ctype, idat = 8, 0, 0, 0, b""
    while pos < len(d):
        ln = struct.unpack(">I", d[pos:pos + 4])[0]
        typ = d[pos + 4:pos + 8]
        data = d[pos + 8:pos + 8 + ln]
        pos += 12 + ln
        if typ == b"IHDR":
            w, h, _bit, ctype = struct.unpack(">IIBB", data[:10])
        elif typ == b"IDAT":
            idat += data
        elif typ == b"IEND":
            break
    raw = zlib.decompress(idat)
    ch = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[ctype]
    stride = w * ch
    out = bytearray(w * h * 4)
    prev = bytearray(stride)
    p = 0
    for y in range(h):
        f = raw[p]
        p += 1
        line = bytearray(raw[p:p + stride])
        p += stride
        if f == 1:
            for i in range(stride):
                if i >= ch:
                    line[i] = (line[i] + line[i - ch]) & 255
        elif f == 2:
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 255
        elif f == 3:
            for i in range(stride):
                a = line[i - ch] if i >= ch else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 255
        elif f == 4:
            for i in range(stride):
                a = line[i - ch] if i >= ch else 0
                b = prev[i]
                c = prev[i - ch] if i >= ch else 0
                pp = a + b - c
                pa, pb, pc = abs(pp - a), abs(pp - b), abs(pp - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 255
        for x in range(w):
            si, di = x * ch, x * 4
            if ctype == 6:
                out[y * w * 4 + di:y * w * 4 + di + 4] = line[si:si + 4]
            elif ctype == 2:
                out[y * w * 4 + di:y * w * 4 + di + 3] = line[si:si + 3]
                out[y * w * 4 + di + 3] = 255
            else:
                v = line[si]
                out[y * w * 4 + di] = out[y * w * 4 + di + 1] = out[y * w * 4 + di + 2] = v
                out[y * w * 4 + di + 3] = 255
        prev = line
    return w, h, out


def write_png(path, w, h, data):
    """把 RGBA bytearray 写成 PNG（filter 0，无依赖）。"""
    def chunk(t, b):
        return struct.pack(">I", len(b)) + t + b + struct.pack(">I", zlib.crc32(t + b) & 0xffffffff)

    raw = bytearray()
    for y in range(h):
        raw.append(0)
        raw.extend(data[y * w * 4:(y + 1) * w * 4])
    open(path, "wb").write(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


# ---------------------------------------------------------------- 抠图 / 缩放

def chroma_key(w, h, px, key=KEY):
    """软抠图：估计品红背景占比 b，反混色还原前景真实色，输出带 alpha 的 RGBA。

    前景近似中性色，观测色 C = a*F + b*K（b = 1-a，K 的 G 通道为 0），
    故 C.r-C.g 与 C.b-C.g 近似等于 255*b。据此估计 b，再解出 F。
    极小/极大的 b 直接吸附到 0/1，保证主体完全不透明、纯背景完全透明。
    """
    kr, kg, kb = key
    out = bytearray(w * h * 4)
    for i in range(0, w * h * 4, 4):
        cr, cg, cb = px[i], px[i + 1], px[i + 2]
        d = max(cr - cg, cb - cg, 0)
        b = (d - 2) / 253.0
        if b < 0.02:
            b = 0.0
        elif b > 0.98:
            b = 1.0
        a = 1.0 - b
        if a <= 0:
            out[i] = out[i + 1] = out[i + 2] = out[i + 3] = 0
            continue
        out[i] = max(0, min(255, int((cr - b * kr) / a + 0.5)))
        out[i + 1] = max(0, min(255, int((cg - b * kg) / a + 0.5)))
        out[i + 2] = max(0, min(255, int((cb - b * kb) / a + 0.5)))
        out[i + 3] = max(0, min(255, int(a * 255 + 0.5)))
    return out


def downscale(sw, sh, src, dw, dh):
    """面积平均降采样（alpha 加权，避免白/黑边）。"""
    out = bytearray(dw * dh * 4)
    fx, fy = sw / dw, sh / dh
    for dy in range(dh):
        for dx in range(dw):
            x0 = int(dx * fx)
            x1 = max(x0 + 1, int((dx + 1) * fx))
            y0 = int(dy * fy)
            y1 = max(y0 + 1, int((dy + 1) * fy))
            n = (x1 - x0) * (y1 - y0)
            ar = ag = ab = aa = 0
            for sy in range(y0, y1):
                base = sy * sw * 4
                for sx in range(x0, x1):
                    i = base + sx * 4
                    a = src[i + 3]
                    ar += src[i] * a
                    ag += src[i + 1] * a
                    ab += src[i + 2] * a
                    aa += a
            i = (dy * dw + dx) * 4
            if aa == 0:
                continue
            r, g, b, a = ar / aa, ag / aa, ab / aa, aa / n
            out[i] = max(0, min(255, int(r + 0.5)))
            out[i + 1] = max(0, min(255, int(g + 0.5)))
            out[i + 2] = max(0, min(255, int(b + 0.5)))
            out[i + 3] = max(0, min(255, int(a + 0.5)))
    return bytes(out)


# ---------------------------------------------------------------- ICO 编码

def _bmp32(w, h, data):
    """BITMAPINFOHEADER(40B) + bottom-up BGRA 像素 + AND mask。"""
    out = bytearray()
    out += struct.pack("<I", 40)          # biSize
    out += struct.pack("<i", w)           # biWidth
    out += struct.pack("<i", h * 2)       # biHeight（含 AND mask）
    out += struct.pack("<H", 1)           # biPlanes
    out += struct.pack("<H", 32)          # biBitCount
    out += struct.pack("<I", 0)           # biCompression = BI_RGB
    out += struct.pack("<I", 0)           # biSizeImage
    out += struct.pack("<i", 0) * 2       # XPels / YPels
    out += struct.pack("<I", 0) * 2       # biClrUsed / biClrImportant
    assert len(out) == 40
    stride = w * 4
    for y in range(h - 1, -1, -1):
        row = data[y * stride:(y + 1) * stride]
        for x in range(w):
            i = x * 4
            out += bytes((row[i + 2], row[i + 1], row[i], row[i + 3]))
    out += b"\x00" * (((w + 31) // 32) * 4) * h
    return bytes(out)


def write_ico(path, w, h, px, sizes=(256, 128, 64, 48, 32, 16)):
    frames = [(s, _bmp32(s, s, downscale(w, h, px, s, s))) for s in sizes]
    offset = 6 + 16 * len(frames)
    directory = bytearray()
    body = bytearray()
    for s, bmp in frames:
        w8 = 0 if s >= 256 else s
        directory += struct.pack("<BBBBHHII", w8, w8, 0, 0, 1, 32, len(bmp), offset)
        body += bmp
        offset += len(bmp)
    open(path, "wb").write(bytes(struct.pack("<HHH", 0, 1, len(frames)) + directory + body))


def verify_ico(path):
    """返回 (是否合法, 描述列表)。"""
    d = open(path, "rb").read()
    _r, _t, n = struct.unpack("<HHH", d[:6])
    ents, off = [], 6
    for _ in range(n):
        w8, _h8, _c, _r2, _pl, bc, sz, o = struct.unpack("<BBBBHHII", d[off:off + 16])
        ents.append((256 if w8 == 0 else w8, bc, sz, o))
        off += 16
    ok = True
    desc = []
    for i, (s, bc, sz, o) in enumerate(ents):
        end = o + sz
        nxt = ents[i + 1][3] if i + 1 < len(ents) else len(d)
        hdr = struct.unpack("<I", d[o:o + 4])[0] if o + 4 <= len(d) else 0
        good = end <= nxt and end <= len(d) and hdr == 40 and bc == 32
        ok = ok and good
        desc.append("%dx%d biSize=%d %s" % (s, s, hdr, "ok" if good else "BAD"))
    return ok, desc


# ---------------------------------------------------------------- 渲染

def render_svg(svg_text, size=1024, workdir=None):
    """把 SVG 文本渲染成 (w,h,RGBA)。workdir 用于存放临时 html/png。"""
    workdir = os.path.abspath(workdir or os.getcwd())
    os.makedirs(workdir, exist_ok=True)
    # 必须是绝对路径：相对路径会生成 file:///app/... 之类的无效 URL，
    # 且 --screenshot 相对路径会被 Edge 忽略/拒绝。
    html_path = os.path.join(workdir, "_icon_render.html").replace("\\", "/")
    raw_path = os.path.join(workdir, "_icon_raw.png").replace("\\", "/")
    html = (
        '<!doctype html><html><head><meta charset="utf-8"><style>'
        'html,body{margin:0;padding:0;background:#ff00ff;width:%dpx;height:%dpx;overflow:hidden}'
        'svg{display:block}</style></head><body>%s</body></html>'
        % (size, size, svg_text)
    )
    open(html_path, "w", encoding="utf-8").write(html)
    if os.path.exists(raw_path):
        os.remove(raw_path)  # 防止复用上一次的截图导致假成功
    cmd = [
        EDGE, "--headless=new", "--disable-gpu", "--no-sandbox", "--hide-scrollbars",
        "--force-device-scale-factor=1",
        "--window-size=%d,%d" % (size, size),
        "--screenshot=" + raw_path,
        "file:///" + html_path.lstrip("/"),
    ]
    subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    if not os.path.exists(raw_path):
        raise RuntimeError("Edge 渲染失败，未生成 " + raw_path)
    w, h, px = read_png(raw_path)
    return w, h, chroma_key(w, h, px)


def build(svg_path, out_png, out_ico, master=512, workdir=None):
    """从 SVG 生成正式 icon.png / icon.ico。"""
    svg = open(svg_path, "r", encoding="utf-8").read()
    w, h, rgba = render_svg(svg, 1024, workdir)
    small = downscale(w, h, rgba, master, master)
    write_png(out_png, master, master, small)
    write_ico(out_ico, master, master, small)
    ok, desc = verify_ico(out_ico)
    print("PNG %dx%d -> %s" % (master, master, out_png))
    print("ICO valid=%s | %s" % (ok, "; ".join(desc)))
    return small


# ---------------------------------------------------------------- 预览合成

def composite(rgba, w, h, bg_fn):
    """把 RGBA 合成到 bg_fn(x,y)->(r,g,b) 上。"""
    out = bytearray(w * h * 4)
    for y in range(h):
        for x in range(w):
            i = (y * w + x) * 4
            a = rgba[i + 3] / 255.0
            br, bg, bb = bg_fn(x, y)
            out[i] = int(rgba[i] * a + br * (1 - a))
            out[i + 1] = int(rgba[i + 1] * a + bg * (1 - a))
            out[i + 2] = int(rgba[i + 2] * a + bb * (1 - a))
            out[i + 3] = 255
    return out


def checker_bg(block=32, light=224, dark=190):
    return lambda x, y: ((light, light, light) if ((x // block + y // block) % 2 == 0)
                        else (dark, dark, dark))


def paste(dst, dw, dh, src, sw, sh, ox, oy, bg=(255, 255, 255)):
    """把 src 合成到 dst 的 (ox,oy) 位置。"""
    for y in range(sh):
        for x in range(sw):
            si = (y * sw + x) * 4
            di = ((oy + y) * dw + (ox + x)) * 4
            a = src[si + 3] / 255.0
            dst[di] = int(src[si] * a + bg[0] * (1 - a))
            dst[di + 1] = int(src[si + 1] * a + bg[1] * (1 - a))
            dst[di + 2] = int(src[si + 2] * a + bg[2] * (1 - a))
            dst[di + 3] = 255


if __name__ == "__main__":
    icons = os.path.join(ROOT, "app", "gui", "icons")
    svg = sys.argv[1] if len(sys.argv) > 1 else os.path.join(icons, "icon.svg")
    png = sys.argv[2] if len(sys.argv) > 2 else os.path.join(icons, "icon.png")
    ico = sys.argv[3] if len(sys.argv) > 3 else os.path.join(icons, "icon.ico")
    build(svg, png, ico, workdir=icons)
