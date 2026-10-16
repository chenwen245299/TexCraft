#!/usr/bin/env python3
"""Builds assets/icon/texcraft.ico (16–256 px, PNG-compressed entries) from texcraft-1024.png.

    scripts/make-ico.py        (macOS: resizes with sips)
"""
import os
import struct
import subprocess
import sys
import tempfile

root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
src = os.path.join(root, "assets", "icon", "texcraft-1024.png")
out = os.path.join(root, "assets", "icon", "texcraft.ico")
sizes = [16, 24, 32, 48, 64, 128, 256]
images = []
with tempfile.TemporaryDirectory() as tmp:
    for s in sizes:
        p = os.path.join(tmp, f"{s}.png")
        subprocess.run(["sips", "-z", str(s), str(s), src, "--out", p], check=True, capture_output=True)
        images.append((s, open(p, "rb").read()))
header = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
entries = b""
data = b""
for s, blob in images:
    dim = 0 if s >= 256 else s
    entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset + len(data))
    data += blob
open(out, "wb").write(header + entries + data)
print(f"wrote {out} ({len(images)} sizes)")
