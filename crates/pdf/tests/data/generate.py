#!/usr/bin/env python3
"""Rebuild the text-selection fixture PDFs: python3 -I crates/pdf/tests/data/generate.py

`select-en.pdf` comes from pdflatex (Type 1 Computer Modern, ligature glyphs, math) and
`select-zh.pdf` from xelatex + ctex with the Fandol fonts (CID fonts with ToUnicode maps). Both are
built in a temporary directory so no local path ends up in the files.
"""

import os
import shutil
import subprocess
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURES = [("select-en", "pdflatex"), ("select-zh", "xelatex")]

for name, engine in FIXTURES:
    with tempfile.TemporaryDirectory() as build:
        shutil.copy(os.path.join(HERE, name + ".tex"), build)
        for _ in range(2):  # twice, for \ref
            subprocess.run(
                [engine, "-interaction=nonstopmode", "-halt-on-error", name + ".tex"],
                cwd=build,
                check=True,
                stdout=subprocess.DEVNULL,
            )
        shutil.copy(os.path.join(build, name + ".pdf"), HERE)
        print("wrote", name + ".pdf")
