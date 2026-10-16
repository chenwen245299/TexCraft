# SyncTeX fixtures and oracle data

Each directory holds a small TeX project, the `.synctex.gz` its engine wrote, and the answers
of the real `synctex` CLI (TeX Live 2022, SyncTeX 1.21) for that file:

| Fixture    | Engine     | What it covers                                                     |
|------------|------------|--------------------------------------------------------------------|
| `single`   | pdflatex   | paragraphs, inline/display math, lists, floats, tables, TikZ, footnote, parboxes (3 pages) |
| `multi`    | pdflatex   | `\input{sections/intro}` etc. in a subdirectory, nested input      |
| `twocol`   | pdflatex   | two columns, spanning float, 3 pages                               |
| `xetex`    | xelatex    | XeTeX via `.xdv`, Unicode text, 2 pages                            |
| `lualatex` | lualatex   | LuaTeX records (including its odd kern tags), A4                   |
| `mag`      | pdftex     | plain TeX with `\mag=1200` (`Magnification:1200`)                  |
| `xform`    | pdflatex   | `\pdfxform`/`\pdfrefxform`: form refs (`f`) without form definitions |

* `forward.txt`: `synctex view -i <line>:0:<input> -o <pdf>` for every line of every project
  file (plus three past the end): `view <input> <line> <page_hint> <count>` followed by one
  `<page> <x> <y> <h> <v> <W> <H>` line per record, in the order the CLI prints them.
* `inverse.txt`: `synctex edit -o <page>:<x>:<y>:<pdf>` on a jittered 30×22 pt grid covering
  each page plus the centre of every box forward search returned:
  `<page> <x> <y> -> <input> <line> <column> [<line2> <column2>]`, or `-> none`.

Inputs are stored relative to the (temporary) build directory; files outside it are written
`@<file name>`. The `.synctex.gz` files themselves record absolute temporary paths, so the
tests (`tests/oracle.rs`) match files by trailing path components and are path-agnostic.

## Regenerating

```sh
python3 -I crates/synctex/tests/data/generate.py          # needs pdflatex, xelatex, lualatex, synctex
TEXCRAFT_SYNCTEX_BUILD=/some/tmp python3 -I .../generate.py # choose where the temporary builds go
```

The script copies each fixture to a temporary directory, compiles it twice with
`-synctex=1`, copies the `.synctex.gz` back here and rewrites `forward.txt` and
`inverse.txt`. The PDFs are not kept.

## Known CLI quirks the data reflects

* `synctex view` documents `-i line:column:page_hint:input` but never parses the page hint
  (it becomes part of the input name), so all forward data uses hint 0. Page-hint ordering is
  covered by a synthetic test (`tests/format.rs`).
* When `synctex edit` returns two candidates it prints the *first* candidate's file name for
  both. LuaTeX writes kern records with tags that name no input (e.g. `k2268,…`); the CLI
  still reports them as second candidates, `SyncTex::inverse_candidates` drops them.
