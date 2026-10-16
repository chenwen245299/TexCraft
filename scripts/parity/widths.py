#!/usr/bin/env python3
"""Ink width of text inside boxes of a screenshot.

    widths.py IMG.png [--scale PX_PER_PT] name:x0,x1,y0,y1 ...

The box's top-left pixel is taken as the background; pixels that differ from it by more than 90
(sum of RGB differences) are ink. Prints the ink extent in pixels and its width in points
(default scale 2 px/pt, a Retina screenshot). Compare with scripts/parity text widths printed by
`cargo test -p texcraft-ui --test text_widths -- --ignored --nocapture` to find font sizes.
"""
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])
import png  # noqa: E402

args = sys.argv[1:]
scale = 2.0
if "--scale" in args:
    i = args.index("--scale")
    scale = float(args[i + 1])
    del args[i : i + 2]
im = png.read(args[0])
for arg in args[1:]:
    name, box = arg.split(":")
    x0, x1, y0, y1 = map(int, box.split(","))
    bg = im.px(x0, y0)
    xs = [
        x
        for y in range(y0, y1)
        for x in range(x0, x1)
        if sum(abs(a - b) for a, b in zip(im.px(x, y), bg)) > 90
    ]
    if xs:
        print(f"{name:16s} {min(xs)}..{max(xs)}  {(max(xs) - min(xs)) / scale:.1f} pt")
    else:
        print(f"{name:16s} none")
