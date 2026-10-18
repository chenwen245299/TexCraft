# Architecture

## Layers

```text
L0  syntax  log  synctex        pure functions over text/bytes; no workspace deps
L0  i18n                        interface languages: tr()/trf! over an English-keyed catalogue
L1  pdf  project                hayro rendering worker; engines, settings, root detection, templates
L2  typeset                     runs TeX: full typesets (latexmk/tectonic) and live passes
L2  update                      release manifest, download/verify/unpack, install after quit
L2  git                         Source Control: git status/stage/commit/push/pull/log/diff (CLI)
L2  ai                          AI Assistant: provider config, AES-256-GCM key store, Anthropic
                                Messages + OpenAI-compatible streaming through curl, SSE parsing
L3  engine                      Session, ProjectIndex, command registry, Effects
L4  ui                          egui workbench (toolbar, tabs, sidebar, editor, PDF, dialogs)
app texcraft, texcraft-cli      eframe window, muda menu bar, rfd panels, control channel; CLI
```

A crate depends only on lower layers. Nothing below `ui` knows about egui.

## The engine/UI seam

- `Session` owns the documents (text + selection in char indices), project folder, settings,
  view state, typesetting state, PDF + SyncTeX state and the `ProjectIndex`.
- Every action is a `CommandSpec` (`commands.rs`): id, label, menu + group, shortcut, `enabled`,
  optional `checked`, `run`. The native menu bar is generated from the registry; the egui shortcut
  handler (non-macOS) uses the same shortcuts; the CLI and control channel dispatch by id.
- Commands never touch the GUI. They queue `Effect`s (reveal a line, push the selection into the
  editor, flash rectangles in the PDF, open a panel…) that `Workbench::apply_effects` turns into UI
  state or platform `Request`s for the app (file panels, Finder, quit, native edit actions).

## Interface languages

English is the source language. User-visible strings are written in English and wrapped in
`texcraft_i18n::tr("…")`, or `trf!("Renamed to {path}", path = …)` when they carry values; the
English text is the key of the catalogue (`crates/i18n/src/zh_hans/<area>.rs`) and the fallback.
The language is process-wide: the app resolves the Language setting (`View::language`: System /
English / 简体中文, System following macOS's preferred languages read at launch into
`Session::system_languages`) with `Session::lang` and applies it with `set_language` every
frame; egui text changes on the next frame and the native menu bar is rebuilt (AppKit menus
cannot be retitled in place). Background threads (typesetting, updates) read the same global, so
their errors arrive in the current language. Tests use `texcraft_i18n::scoped`, a per-thread
override, so parallel tests never see each other's language. `tests/catalogue.rs` scans the
workspace's source for `tr`/`trf!` literals: every one needs a zh-Hans entry, entries keep their
placeholders, `trf!` arguments match them, keys have one translation and none is stale.

## Threads

| Thread | Work |
|---|---|
| UI | egui frames; `Session::tick` polls the typeset job and fires live passes |
| `texcraft-typeset` / `texcraft-live` | one at a time: spawns the engine, streams output, parses the log |
| `texcraft-pdf-render` | owns the hayro document; renders requested pages, drops stale requests |
| `texcraft-control` | accepts control connections; requests are answered on the UI thread |

Engines run in their own process group so ⌘. (abort) kills latexmk and its children.

## Live preview pipeline

1. An edit bumps the document revision and starts a live pass at once (`live_delay_ms`, default
   0); edits during a pass queue exactly one more, so passes run back to back while typing. The
   PDF viewer re-renders the pages that were on screen as soon as the new PDF opens and records
   keystroke-to-pixels latency (`state.inspect` → `typeset.latencyMs`).
2. `LiveRequest` carries the text of every open buffer inside the project (relative paths), the
   folders holding project files, and the engine.
3. `live::prepare_mirror` writes the buffers into `<project>/.texcraft/live/` (only when changed),
   deletes mirror copies of closed buffers, copies the root if it is not open and the `.bbl` of the
   last full typeset. Everything else is found through `TEXINPUTS=.:<project>:`.
4. pdfLaTeX: `ensure_format` precompiles the preamble with `mylatexformat` into
   `texcraft-<hash>.fmt` (hash of the preamble and project-local packages); failures are remembered.
5. pdfLaTeX: after a pass with the full font map, a trimmed `pdftex.map` (base families + families
   embedded by the document) is written into the live folder, where pdfTeX looks first. Passes with
   it run with `MKTEXPK=0`; a missing font fails fast and the pass is rerun with the full map.
6. pdfLaTeX, fast environment: after a normal pass (run with `-recorder`) the system files it
   opened are linked into `.texcraft-fast/farm`, next to a generated `texmf.cnf` that searches
   only the live folder, the project and the farm and copies the engine's memory settings
   (`kpsewhich -expand-var`). Fast passes set `TEXMFCNF` to it and skip the ~110 ms of TeX Live
   configuration parsing; the format is stored uncompressed (another ~50 ms). A fast pass that
   reports a missing file or font a normal pass did not is rerun normally and the farm rebuilt.
7. One engine pass. The log is parsed against the project folder; mirror paths in issues and
   SyncTeX results are mapped back to the real files (`Mirror::to_project` / `to_mirror`).
8. If the pass has errors and a previous PDF exists, the PDF is kept (setting `live_keep_good`);
   otherwise the viewer reopens it, keeping the old page textures until the new ones arrive, so the
   view neither flickers nor jumps.

Full typesets (⌘T, and on opening a project) save edited files and run latexmk on the real files
into `.texcraft/build/`.

## Testing

- Parsers are oracle-tested against real artefacts from the installed TeX Live (`crates/log/tests`,
  `crates/synctex/tests`), plus garbage/truncation robustness tests.
- `crates/engine/src/tests.rs` fuzzes every command with adversarial parameters and runs real
  typesets (full and live, including the font-map fallback) when TeX is installed.
- UI changes are verified by driving the app over the control channel and reading `ui.screenshot`.
