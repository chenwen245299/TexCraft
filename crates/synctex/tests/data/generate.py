#!/usr/bin/env python3
"""Regenerate the SyncTeX fixtures and the `synctex` CLI oracle outputs.

Usage (from anywhere):

    python3 -I crates/synctex/tests/data/generate.py [--keep]
    python3 -I crates/synctex/tests/data/generate.py --scrub   # only re-anonymise the .gz files

For every fixture directory listed in FIXTURES this script

1. copies the directory into a fresh temporary build directory,
2. compiles the main file twice with `<engine> -synctex=1`,
3. copies `<name>.synctex.gz` back next to the sources,
4. runs `synctex view` for every line of every project source file and writes
   `forward.txt`, and
5. runs `synctex edit` on a jittered grid of points on every page plus the
   centre of every box that forward search returned, and writes `inverse.txt`.

The SyncTeX file records absolute paths of the temporary build directory, which
name the user's home or temporary folder: the copy kept here has them replaced by
`/tmp/texcraft-synctex/<fixture>` (`scrub`), so no personal path is committed. The
expectation files only store paths relative to it (files outside it, such as
class files, are stored with an `@` prefix followed by their file name), so the
integration tests are path-agnostic.

Requires a TeX distribution with the `synctex` CLI on PATH (MacTeX/TeX Live).
"""

import gzip
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))

# (fixture directory, main file without extension, engine)
FIXTURES = [
    ("single", "single", "pdflatex"),
    ("multi", "main", "pdflatex"),
    ("twocol", "twocol", "pdflatex"),
    ("xetex", "xedoc", "xelatex"),
    ("lualatex", "luadoc", "lualatex"),
    # Plain TeX with \mag=1200: exercises the Magnification record.
    ("mag", "mag", "pdftex"),
    # \pdfxform/\pdfrefxform: form refs ("f" records) without form definitions.
    ("xform", "xform", "pdflatex"),
]

# Stands for the temporary build directory in the committed SyncTeX files.
NEUTRAL_BUILD = "/tmp/texcraft-synctex"

GRID_DX = 30.0
GRID_DY = 22.0
GRID_W = 620.0
GRID_H = 850.0


def run(cmd, cwd):
    return subprocess.run(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          text=True, errors="replace").stdout


def rel_name(name, build):
    """Path of an input relative to the build directory, `./` removed."""
    prefix = build.rstrip("/") + "/"
    if name.startswith(prefix):
        rest = name[len(prefix):]
        parts = [p for p in rest.split("/") if p not in ("", ".")]
        return "/".join(parts)
    return "@" + os.path.basename(name)


def parse_view(out):
    records = []
    cur = {}
    for line in out.splitlines():
        if ":" not in line:
            continue
        key, _, value = line.partition(":")
        if key == "Page":
            cur = {"page": int(value)}
        elif key in ("x", "y", "h", "v", "W", "H"):
            cur[key] = float(value)
            if key == "H":
                records.append(cur)
    return records


def parse_edit(out):
    results = []
    cur = None
    for line in out.splitlines():
        key, _, value = line.partition(":")
        if key == "Input":
            cur = {"input": value}
        elif key == "Line" and cur is not None:
            cur["line"] = int(value)
        elif key == "Column" and cur is not None:
            cur["column"] = int(value)
            results.append(cur)
            cur = None
    return results


def scrub(text, build, fixture):
    """Replaces the build directory `build` in the SyncTeX text with a neutral path (the real one
    names the user's folders). The `!<n>` records count the bytes written since the previous
    one, so each is corrected by how much the paths before it grew or shrank."""
    neutral = NEUTRAL_BUILD + "/" + fixture
    out = []
    delta = 0
    for line in text.split("\n"):
        if build in line:
            new = line.replace(build, neutral)
            delta += len(new.encode()) - len(line.encode())
            line = new
        elif delta and re.fullmatch(r"!\d+", line):
            new = "!%d" % (int(line[1:]) + delta)
            delta = len(new) - len(line)
            line = new
        out.append(line)
    return "\n".join(out)


def write_gz(path, text):
    """Writes `text` gzip-compressed with no timestamp or file name, so reruns are identical."""
    with open(path, "wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as f:
            f.write(text.encode("utf-8", "surrogateescape"))


def scrub_existing():
    """Re-anonymises the committed SyncTeX files (the build directory is the one of `Input:1`)."""
    for (fixture, name, _) in FIXTURES:
        gz = os.path.join(HERE, fixture, name + ".synctex.gz")
        text = gzip.open(gz, "rt", encoding="utf-8", errors="surrogateescape").read()
        first = re.search(r"^Input:1:(.*)/\./", text, re.M)
        if not first or first.group(1) == NEUTRAL_BUILD + "/" + fixture:
            continue
        write_gz(gz, scrub(text, first.group(1), fixture))
        print("%s: scrubbed %s" % (fixture, first.group(1)))


def main():
    if "--scrub" in sys.argv:
        scrub_existing()
        return
    keep = "--keep" in sys.argv
    synctex = shutil.which("synctex") or "/Library/TeX/texbin/synctex"
    for (fixture, name, engine) in FIXTURES:
        src = os.path.join(HERE, fixture)
        build_root = tempfile.mkdtemp(prefix="texcraft-synctex-", dir=os.environ.get("TEXCRAFT_SYNCTEX_BUILD"))
        build = os.path.join(os.path.realpath(build_root), fixture)
        shutil.copytree(src, build, ignore=shutil.ignore_patterns(
            "*.synctex.gz", "forward.txt", "inverse.txt"))
        for _ in range(2):
            run([engine, "-synctex=1", "-interaction=nonstopmode", name + ".tex"], build)
        gz = os.path.join(build, name + ".synctex.gz")
        pdf = os.path.join(build, name + ".pdf")
        text = gzip.open(gz, "rt", errors="replace").read()
        raw = gzip.open(gz, "rt", encoding="utf-8", errors="surrogateescape").read()
        write_gz(os.path.join(src, name + ".synctex.gz"), scrub(raw, build, fixture))
        inputs = re.findall(r"^Input:\d+:(.*)$", text, re.M)
        pages = sorted({int(p) for p in re.findall(r"^\{(\d+)$", text, re.M)})
        project = []
        for inp in inputs:
            rel = rel_name(inp, build)
            if rel.startswith("@") or not rel.endswith(".tex") or inp in [p[0] for p in project]:
                continue
            path = os.path.join(build, rel)
            if os.path.exists(path):
                nlines = sum(1 for _ in open(path, errors="replace"))
                project.append((inp, rel, nlines))

        # Forward search oracle.
        centres = set()
        with open(os.path.join(src, "forward.txt"), "w") as f:
            # The CLI accepts `-i line:column:page_hint:input` in its help text but never parses
            # the hint, so every query is recorded with hint 0.
            f.write("# synctex view oracle: view <input> <line> <page_hint> <count>, then\n")
            f.write("# one record per line: <page> <x> <y> <h> <v> <W> <H>\n")
            for (inp, rel, nlines) in project:
                for line in range(1, nlines + 4):
                    spec = "%d:0:%s" % (line, inp)
                    recs = parse_view(run([synctex, "view", "-i", spec, "-o", pdf], build))
                    f.write("view %s %d 0 %d\n" % (rel, line, len(recs)))
                    for r in recs:
                        f.write("  %d %.4f %.4f %.4f %.4f %.4f %.4f\n" % (
                            r["page"], r["x"], r["y"], r["h"], r["v"], r["W"], r["H"]))
                        cx = r["h"] + r["W"] / 2.0
                        cy = r["v"] - r["H"] / 2.0
                        centres.add((r["page"], round(cx, 1), round(cy, 1)))

        # Inverse search oracle.
        rng = random.Random(fixture)
        points = []
        for page in pages:
            y = 0.0
            while y <= GRID_H:
                x = 0.0
                while x <= GRID_W:
                    jx = round(x + rng.uniform(0, GRID_DX), 1)
                    jy = round(y + rng.uniform(0, GRID_DY), 1)
                    points.append((page, jx, jy))
                    x += GRID_DX
                y += GRID_DY
        points.extend(sorted(centres))
        with open(os.path.join(src, "inverse.txt"), "w") as f:
            f.write("# synctex edit oracle: <page> <x> <y> -> <input> <line> <column> [<line2> <column2>]\n")
            f.write("# or '-> none'. Inputs are relative to the build dir; '@name' is outside it.\n")
            for (page, x, y) in points:
                res = parse_edit(run([synctex, "edit", "-o", "%d:%.1f:%.1f:%s" % (page, x, y, pdf)], build))
                if not res:
                    f.write("%d %.1f %.1f -> none\n" % (page, x, y))
                    continue
                first = res[0]
                line = "%d %.1f %.1f -> %s %d %d" % (page, x, y, rel_name(first["input"], build),
                                                     first["line"], first["column"])
                if len(res) > 1:
                    line += " %d %d" % (res[1]["line"], res[1]["column"])
                f.write(line + "\n")
        print("%s: %d pages, %d project inputs, %d inverse points" % (
            fixture, len(pages), len(project), len(points)))
        if keep:
            print("  build kept in", build)
        else:
            shutil.rmtree(build_root, ignore_errors=True)


if __name__ == "__main__":
    main()
