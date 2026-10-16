"""compose.py OUT A.png:x0,y0,x1,y1 B.png:x0,y0,x1,y1 ... — crops placed side by side (top-aligned)."""
import sys, struct, zlib
sys.path.insert(0, __file__.rsplit('/', 1)[0])
import png
out = sys.argv[1]
parts = []
for arg in sys.argv[2:]:
    path, box = arg.rsplit(':', 1)
    x0, y0, x1, y1 = map(int, box.split(','))
    parts.append((png.read(path), x0, y0, x1, y1))
W = sum(p[3] - p[1] for p in parts) + 10 * (len(parts) - 1)
H = max(p[4] - p[2] for p in parts)
rows = []
for y in range(H):
    line = bytearray()
    for k, (im, x0, y0, x1, y1) in enumerate(parts):
        if k:
            line += b'\xff\x00\xff' * 10
        for x in range(x0, x1):
            yy = y0 + y
            line += bytes(im.px(x, yy)) if yy < y1 else b'\xff\xff\xff'
    rows.append(b'\x00' + bytes(line))
def chunk(t, b):
    return struct.pack('>I', len(b)) + t + b + struct.pack('>I', zlib.crc32(t + b) & 0xFFFFFFFF)
data = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', W, H, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(b''.join(rows), 6)) + chunk(b'IEND', b'')
open(out, 'wb').write(data)
