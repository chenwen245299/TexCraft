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

The desktop app starts with `Session::empty`. Every launch without an explicit document or
template shows the start page for project selection, including login/reboot and update Restart.
The old welcome preference and any saved workspace never reopen the previous project. An empty
folder is not an editor; a window with neither documents nor a PDF returns to the start page.
Native startup ignores an old editor's full-screen/maximized state so no empty full-screen
Space hides the start page. Settings and the recent-documents list still load, and explicit launch paths
open the selected files or folders directly. Untitled documents are created by New/template
commands; edited unsaved documents must pass the save prompt before quitting. Quit prompts
keep saved tabs available for the final typeset, while closing the last tab clears the
project/PDF and opens the welcome window. The welcome window remains open when a file panel
is cancelled.

Recent lists saved project root files only. Opening or saving a project's bibliography,
chapter or other tab remembers the window's fixed root. Legacy file/folder entries resolve
to confirmed roots and deduplicate on load and shared-preference adoption; ambiguous assets,
rootless folders and missing files are discarded.

Each project owns an independent native window. Open Recent resolves a file to its owning
project; opening another root or folder queues `Effect::OpenProjectWindow` without changing
the current session. Included chapters and shared assets remain tabs of their project.
A project shows in one window only, across these processes: a window holds an exclusive
`flock` on `windows/<hash of its project_key>.lock` in the data folder while it shows that
project (`apps/texcraft/src/window_registry.rs`; the system releases it when the process ends,
crashes included). Opening a held project anywhere (Open Recent, the welcome window, a launch
path, a link) appends the path to that project's `.inbox` instead and activates the owner
(`texcraft_macos::activate_process`); a watcher thread wakes the owner, which opens the paths
and comes forward. A process started for a project another window holds forwards its paths and
exits before showing a window, which also settles two windows starting with one project.
`texcraft_engine::project_key` (root file, else folder) is what every window compares.
If no window can start for a path, an empty window opens it itself; otherwise an alert says
where the saved file is.

Preferences (`app.ron`) and the AI configuration (`ai.json`) are shared by all windows. Each
save is a three-way merge (`texcraft_engine::prefs::merge3`, `apps/texcraft/src/prefs.rs`):
what this window changed since it last read or wrote the file wins field by field, the rest
is the file's. Windows look at the files every 0.7 s and save their own changes within that,
so a preference changed in one window applies in all; recent documents merge as lists. A
window's layout (sidebar, tabs, split, zoom, page mode) stays its own. One window does the
automatic update checks (`windows/updates.lock`); the installer waits until every TexCraft
process running from the bundle has quit, and a second installer steps aside.
The desktop platform starts an independent app instance for the new window after flushing
preferences. Its engine, PDF worker, menus and AI conversation are separate; closing either
window leaves the other running. Controlled child windows use an automatically allocated
port and the original token file, reporting their port in the inherited log. The parent
reports their process IDs and launch paths/templates in `ui.projectWindows`. New/template
commands also request independent windows when a project is already open. Cross-project
Save As writes the destination before transferring that tab to a separate window; the UI
retires its old save prompt before a reused tab index can close another document.

The initial file's owning root determines its project folder, including when opening a chapter
in a subfolder. The owning root stays fixed when shared chapters are selected. AI tools use
the project folder; history is keyed by the owning root file, falling back to the folder for
rootless projects. Thus independent roots in one folder have separate conversations. Legacy
folder histories are eligible only when exactly one root can own them. A direct directory change
synchronously stops an old AI request and resets its context before any tools can use the
new directory.

Each Assistant question captures the active editor tab afresh: project-relative file and root
paths, unsaved state, caret and selection lines. That identity takes precedence over files
mentioned in earlier turns. Document inclusion controls the current buffer and selected text;
the file identity still accompanies a question when contents are excluded. The composer chip
shows the active path, and `ai.editor` exposes the same metadata to agents. Requests already
in flight retain the editor context captured when their question was sent.

Conversations stay in the data folder's `chats/` by default. Settings → AI → Conversations
offers an explicit local preference, `Config::sync_chats_icloud` (`ai.setICloudSync`); when
enabled and iCloud Drive is available they use `iCloud Drive/TexCraft/AI Assistant/` instead.
The pane reports the actual store and location, unavailability, copying and errors; it does
not claim to know iCloud's network upload status. Changing stores takes effect immediately
without stopping an answer or clearing its draft. A cancellable worker copies history and
images, retaining the source and choosing the newer saved conversation; other windows follow
the local configuration. Turning sync off never deletes cloud files. Old automatic-cloud
histories are copied back once without opting in; `chat_storage_version` stays 0 until an
import finishes (also on disabling), so an interrupted import resumes on restart. With sync
off, a completed import does not keep reading cloud history. Each project folder holds one `<id>.json` per conversation,
`attachments/` (images, named by SHA-256, written once) and `project.json`; `index.json` only
caches titles, and the list comes from the conversation files. Folders are keyed by the
project's path relative to iCloud Drive or home, and stored paths use `$icloud/…`/`$home/…`,
so another Mac maps them to its own. Nothing is deleted automatically: a project missing on
this Mac may be on another one or on a drive not connected.

Messages persist their source project, root and file. Inserting an answer requires the same
source file; legacy answers without provenance must be copied manually. Renames record
timed aliases that apply only to conversations saved before them; the conversation shown
follows a rename of its own project (messages, draft and sources) instead of starting afresh. AI actions and inline
suggestions validate their original targets, including the file identity when a tab index is
reused. Tool rounds capture their directory at request time. Git operations discard stale
results/queued work when their directory changes. Renames remap tabs, navigation, roots and
chat provenance together, retiring jobs whose paths moved. Closing the last tab clears editor,
AI and diagnostic state synchronously. A late compiler result may publish its own output, but
cannot replace a different project's PDF, issues, logs or status.

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

Full-screen chrome and toolbar layout share AppKit's will/did enter/exit state. Title-bar
views are positioned only when stably windowed. Transition completion invalidates the native
content view and requests fresh egui frames; controlled toggles during an animation queue the
last requested state. They never replace the session or reset editor/PDF state.

## Live preview pipeline

1. An edit bumps the document revision and starts a live pass at once (`live_delay_ms`, default
   0); edits during a pass queue exactly one more, so passes run back to back while typing. The
   PDF viewer re-renders the pages that were on screen as soon as the new PDF opens and records
   keystroke-to-pixels latency (`state.inspect` → `typeset.latencyMs`).
2. `LiveRequest` carries the text of every open buffer inside the project (relative paths), the
   folders holding project files, and the engine.
3. `live::prepare_mirror` writes the buffers into `<project>/.texcraft/live/root-<hash>/`
   (only when changed). Each root has its own manifest, formats and generated files, including
   when multiple roots share a folder. It
   deletes mirror copies of closed buffers, copies the root if it is not open and the `.bbl` of the
   last full typeset. Everything else is found through `TEXINPUTS=.:<project>:`.
4. pdfLaTeX: `ensure_format` precompiles the preamble with `mylatexformat` into
   `texcraft-<hash>.fmt`; the key covers the preamble, project-local packages and the canonical
   paths/size/modification time of both the format builder and reader binaries. Replacing a TeX
   binary invalidates its old formats without spawning a version probe on each edit. Failed
   builds are remembered for that key.
5. pdfLaTeX: after a pass with the full font map, a trimmed `pdftex.map` (base families + families
   embedded by the document) is written into the live folder, where pdfTeX looks first. Passes with
   it run with `MKTEXPK=0`; a missing font fails fast and the pass is rerun with the full map.
6. pdfLaTeX, fast environment: after a normal pass (run with `-recorder`) the system files it
   opened are linked into `.texcraft-fast/farm`, next to a generated `texmf.cnf` that searches
   only the live folder, the project and the farm and copies the engine's memory settings
   (`kpsewhich -expand-var`). Its `TEXMFDBS = .` avoids an empty-value kpathsea warning.
   Fast passes set `TEXMFCNF` to it and skip the ~110 ms of TeX Live
   configuration parsing; the format is stored uncompressed (another ~50 ms). A fast pass that
   reports a missing file or font a normal pass did not is rerun normally and the farm rebuilt —
   including pdfTeX's `(file cmti10.pfb): cannot open Type 1 font file` when the document starts
   using a font the farm lacks (before, every later pass failed on it and the preview froze).
7. One engine pass. The log is parsed against the project folder; mirror paths in issues and
   SyncTeX results are mapped back to the real files (`Mirror::to_project` / `to_mirror`).
8. If the pass has errors and a previous PDF exists, the PDF is kept (setting `live_keep_good`);
   otherwise the viewer reopens it, keeping the old page textures until the new ones arrive, so the
   view neither flickers nor jumps.
9. The preview controller (`crates/engine/src/live.rs`) owns edit scheduling, queues,
   catch-up/settling and timeout recovery. Each document records the revision a successful pass
   typeset (`typeset_revision`), and whenever nothing runs or
   waits, text no pass typeset gets a catch-up pass (unless that exact text just failed or was
   stopped). A pass is one engine run, so what it shows of the contents, references and outline
   comes from the previous pass's `.aux`/`.toc`/`.out`/…; a pass that changed those files
   (`Outcome::read_back_changed`) is followed by passes over the same text until they settle (at
   most 3). Without that, a heading corrected after a half-typed one failed on the half-typed
   one's `.toc` and the last good PDF stayed up for good.

10. The watchdog detects a live pass with no output for 20 s while newer edits wait, or
    60 s even for the last keystroke (at least 4× the previous pass's duration). It stops the
    process group and queues one clean recovery per source/configuration stamp; a second
    timeout stops instead of restarting forever. Preamble builds have a separate 20 s bound,
    track their process group and yield immediately to newer edits. Dropping a job cancels
    its processes. A job whose thread dies finishes with an error.

Rejected precompiled formats are discarded together with their file farm. That pass retries
once without the format, restoring the original Xe/Lua source variant when needed; later edits
can rebuild the cache. A live result needs exit status 0 and a PDF changed by that run. A stale
PDF/log left by a failed engine cannot count as a successful update.

Each retried live engine process resets the displayed output; completion replaces it with
the final outcome's console so the log, diagnostics and status describe the same attempt.
An unsuccessful non-cancelled outcome without parsed errors gets a process/cache failure
issue, including when TeX aborts before opening its log. A fatal log cannot count as success,
and the toolbar shows a green check only for an explicitly successful outcome.

Other failed live passes get one retry without precompiled formats after removing only that
root's generated auxiliary files, font maps and fast-file farm. Source snapshots and the last
PDF remain intact; real source files are never modified by recovery. This also repairs poisoned
`.aux`/`.toc`/`.out` state after reopening a project. A stable source error remains an error.

The PDF view retries failed loads/renders twice, replacing its render worker, with a 10 s
watchdog for silent stalls. Exhaustion asks the preview controller for one clean rebuild of
the current source stamp. Hidden panes suspend the display watchdog. Recovery and stale
preview notices make failures visible while retaining the last good texture.

Mode, pause, resume and queue consumption share the same preview-controller transitions.
Pause drops queued preview passes. Abort additionally suppresses catch-up until another edit
or an explicit resume/configuration change; choosing LIVE retries the current revision even
if that revision failed or stopped earlier. Attempt identity includes the effective engine,
shell-escape and preamble-cache settings, so changing a compiler setting can recover without
requiring a dummy text edit.

`crates/ui/tests/live_preview.rs` tests the full editor → TeX → displayed page contract using
real input events, PDF text and rendered generations. `scripts/check-live-preview.sh` requires
the TeX prerequisites instead of allowing skips; it gates editor-related work, pull-request
checks and release builds. It runs the engine suite and covers unsaved typing/paste/Smart Return,
Chinese engine switching, source-error and corrupted-cache recovery, 300 edits interleaved with
typesetting/mode/pause transitions, included files, independent editor sessions, six reopen
cycles with poisoned auxiliary files, and display failures that rebuild without further input.

Full typesets (⌘T, and on opening a project) save edited files and run latexmk on the real files
into `.texcraft/build/`, then atomically publish a successful PDF beside the root. Publication
writes only the PDF bytes into a fresh per-process staging file (synced before the rename), so
macOS hidden flags and FinderInfo from the build or an old staging file cannot make the final
PDF invisible in Finder, and two windows cannot rename each other's half-written file. Live passes
never touch that PDF. LIVE mode periodically refreshes edited projects: 5 minutes after the
first edit, at a 5 s typing pause (at once after 10 minutes).

Bibliography edits additionally request `typeset.refreshBibliography`: after an 800 ms pause,
a quiet full toolchain run updates BibTeX/Biber output in every mode. It reads a separate
mirror of current buffers, writes the normal build cache and refreshes the PDF without saving
source files. Later live passes reuse the updated `.bbl`. It is background work: it never
holds up a live pass, and starts only once nothing runs and no live pass is due or queued. In
LIVE mode an edit during a run stops it without trace and re-arms it for the next pause (the
live pass shows the edit at once); otherwise edits queue another snapshot. Foreground
requests take priority: ⌘T stops a running refresh the same way, and quitting stops it. Nothing
shows it running (no gears, no "Typesetting…"), so Abort Typesetting is off for it; stopped
anyway, it runs again at the next pause. Failed snapshots preserve the good `.bbl`, PDF
and SyncTeX files. Newly added or changed citation keys in the body request the same refresh,
so entries first cited after a `.bib` rebuild also appear correctly. Ordinary text edits and
citation line-number changes keep using fast live passes. This refresh does not animate the
toolbar, change diagnostics/status, open the issues list or move either view. `typeset.wait`
includes its debounce and queued work.

PDF zoom preserves the page and PDF point at the viewport centre for buttons, shortcuts and
percentage changes, or under the pointer for pinch/command-wheel gestures. New page rectangles
determine bounded offsets; old scrolling animations/momentum are retired. Explicit page and
SyncTeX navigation take precedence. Thumbnails do not reuse reading-mode zoom anchors.

PDF refreshes keep the reader's current page, horizontal/vertical offsets and zoom. Automatic
scrolling to the source caret after typesetting is opt-in (`View::sync_after_typeset`, false
by default); persisted-preferences version 3 disables the old enabled default on upgrade.
Explicit SyncTeX navigation and the separate scroll-lock option still work as before.

Every normal exit waits for a forced full rebuild (`latexmk -g`) of each open project plus
edited projects whose tabs were closed, in LIVE, AUTO and MANUAL modes. Open tabs count through
the root they are typeset with (`Session::root_of`), and only whole documents are rebuilt. A
project is "edited" while the PDF beside it lags an edit: a failed typeset keeps an edited
root due but never makes an unedited one (or an untitled scratch file) due. Only the exit's
own typesets (`quit_job`) decide it; Don't Quit stops the one running. A closed project's
exit rebuild never replaces this window's issues, log or status; its failure names it in the
sheet. Unsaved buffers go
through the save prompt first; edits received during a build return to that prompt. Both the
compiler and publishing the PDF beside its root must succeed. Failure preserves the previous
published PDF and keeps TexCraft open with Retry Typesetting / Don't Quit; a running build
offers Don't Quit. There is no skip-build exit action. `crates/macos/src/quit.rs` intercepts
AppKit's `applicationShouldTerminate:` (Dock / Apple-event Quit) before winit's unconditional
will-terminate cleanup, forwarding it to the same root-window close gate as the red button,
menu and shortcut commands. It keeps winit's delegate and ivars by adding a subclass without
extra instance storage.

## Testing

- Parsers are oracle-tested against real artefacts from the installed TeX Live (`crates/log/tests`,
  `crates/synctex/tests`), plus garbage/truncation robustness tests.
- `crates/engine/src/tests.rs` fuzzes every command with adversarial parameters and runs real
  typesets (full and live, including the font-map fallback) when TeX is installed.
- UI changes are verified by driving the app over the control channel and reading `ui.screenshot`.
