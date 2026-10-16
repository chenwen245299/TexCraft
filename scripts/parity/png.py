"""Minimal PNG reader/writer (8-bit RGB/RGBA, non-interlaced) for measuring screenshots."""
import struct
import zlib


def read(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    pos, idat = 8, b""
    w = h = None
    while pos < len(data):
        n = struct.unpack(">I", data[pos:pos + 4])[0]
        kind = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, ctype, _, _, inter = struct.unpack(">IIBBBBB", body)
            assert depth == 8 and inter == 0, (depth, inter)
            bpp = {2: 3, 6: 4}[ctype]
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    raw = zlib.decompress(idat)
    stride = w * bpp
    rows = []
    prev = bytearray(stride)
    i = 0
    for _ in range(h):
        f = raw[i]
        line = bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        if f == 1:
            for x in range(bpp, stride):
                line[x] = (line[x] + line[x - bpp]) & 255
        elif f == 2:
            for x in range(stride):
                line[x] = (line[x] + prev[x]) & 255
        elif f == 3:
            for x in range(stride):
                a = line[x - bpp] if x >= bpp else 0
                line[x] = (line[x] + ((a + prev[x]) >> 1)) & 255
        elif f == 4:
            for x in range(stride):
                a = line[x - bpp] if x >= bpp else 0
                b = prev[x]
                c = prev[x - bpp] if x >= bpp else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[x] = (line[x] + pr) & 255
        rows.append(line)
        prev = line
    return Image(w, h, bpp, rows)


class Image:
    def __init__(self, w, h, bpp, rows):
        self.w, self.h, self.bpp, self.rows = w, h, bpp, rows

    def px(self, x, y):
        r = self.rows[y]
        o = x * self.bpp
        return tuple(r[o:o + 3])

    def hexpx(self, x, y):
        return "#%02X%02X%02X" % self.px(x, y)

    def crop(self, x0, y0, x1, y1, path, scale=1):
        out = b""
        for y in range(y0, y1):
            line = bytearray()
            for x in range(x0, x1):
                line += bytes(self.px(x, y)) * scale
            for _ in range(scale):
                out += b"\x00" + bytes(line)
        w, h = (x1 - x0) * scale, (y1 - y0) * scale

        def chunk(t, b):
            return struct.pack(">I", len(b)) + t + b + struct.pack(">I", zlib.crc32(t + b) & 0xFFFFFFFF)

        png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
        png += chunk(b"IDAT", zlib.compress(out, 6)) + chunk(b"IEND", b"")
        open(path, "wb").write(png)

    def runs_row(self, y, x0=0, x1=None, thresh=30):
        """Segments of a row where colour changes by more than thresh: [(x_start, x_end, hex)]."""
        x1 = x1 or self.w
        segs = []
        start = x0
        base = self.px(x0, y)
        for x in range(x0 + 1, x1):
            p = self.px(x, y)
            if sum(abs(a - b) for a, b in zip(p, base)) > thresh:
                segs.append((start, x - 1, "#%02X%02X%02X" % base))
                start, base = x, p
        segs.append((start, x1 - 1, "#%02X%02X%02X" % base))
        return segs

    def runs_col(self, x, y0=0, y1=None, thresh=30):
        y1 = y1 or self.h
        segs = []
        start = y0
        base = self.px(x, y0)
        for y in range(y0 + 1, y1):
            p = self.px(x, y)
            if sum(abs(a - b) for a, b in zip(p, base)) > thresh:
                segs.append((start, y - 1, "#%02X%02X%02X" % base))
                start, base = y, p
        segs.append((start, y1 - 1, "#%02X%02X%02X" % base))
        return segs
