# Control protocol

Start the app with a control port and a token file (created with mode 0600 if missing):

```sh
texcraft examples/demo-paper --control 7878 --control-token-file /private/path/texcraft.token
```

Without `--control-token-file` a fresh token is printed to standard error. The server listens on
`127.0.0.1` only. Each request and reply is one JSON line. The first request on a connection must
authenticate:

```json
{"id": 0, "method": "auth", "params": {"token": "<64 hex characters>"}}
{"id": 1, "method": "command.run", "params": {"id": "typeset.run"}}
```

Replies: `{"id": 1, "ok": true, "result": …}` or `{"id": 1, "ok": false, "error": "…"}`.

## Methods

| Method | Params | Result |
|---|---|---|
| `commands.list` | — | every command: id, label, menu, shortcut, params doc, enabled, checked |
| `command.run` | `{id, params?}` | `"ok"` or the command's error |
| `state.inspect` | — | documents, project, root, engine, `cjkSetup`, settings, view, typeset counts, `update` (current version, status), PDF, UI (pages, rendered pages, dialog, settings/welcome windows, `modelPicker` (open, provider, fetching, models and per-group counts, shown, ticked, filter, error), native `windows`, `repaintCauses`, completion list, `updateWindow` (the update window is open); `ai.readsImages`, `ai.draftImages` (name, type, bytes), each message's `images`, providers' `models` as `{id, name?, capabilities?, context_length?}`; `ai.inline`: settings, `available`, `busy`, `waiting`, the current `suggestion`, `lastError`, `lastLatencyMs`; macOS window buttons: `trafficLights` frames, `fullScreen` (winit) and `appKitFullScreen`) |
| `editor.getText` | — | the active document's text |
| `ui.key` | `{text}` or `{key, command?, shift?, alt?, ctrl?}` (egui key names) | types into the focused widget |
| `ui.fullscreen` | `{on?}` | enters or leaves full screen (toggles without `on`) the way the green window button does (AppKit's `toggleFullScreen:` on macOS); returns `{was, on}` |
| `ui.pointer` | `{x, y, click?}` (points; `click` `"primary"` or `"secondary"`) | moves the pointer there (hover) and optionally clicks: menus, context menus, tooltips |
| `ui.drag` | `{from: [x, y], to: [x, y], steps?}` (points) | presses at `from`, moves to `to` over `steps` frames (default 8), releases: dividers, sliders, text selection |
| `ui.wait` | `{frames}` | returns after that many frames |
| `typeset.wait` | `{timeoutMs?}` | waits until no typeset is running or pending; returns the typeset summary |
| `ui.screenshot` | `{path}` | saves a PNG of the window; returns path and size |
| `ui.welcome` | `{open?, inline?, leftTab?, rightTab?, close?}` | opens/closes the welcome window; `inline` draws it inside the main window for `ui.screenshot`; `close: true` presses its close button (at launch that quits) |
| `ui.prefs` | `{open?, pane?, editorTab?, sheet?, sheetTab?, inline?, models?, modelGroup?}` | opens/closes the Settings window (panes `general`, `sidebar`, `editor`, `viewer`, `typesetting`, `ai`) and the Typeset Configuration sheet; `models` opens (fetching the list) or closes the model picker for the provider selected in the AI pane, `modelGroup` picks its tab (`text`, `vision`, `other`; anything else: all); `inline` draws the Settings contents and the picker inside the main window so `ui.screenshot` can capture them |
| `ui.resize` | `{width, height}` | resizes the window |

Useful command ids: `file.openPath {path}`, `edit.setText {text}`, `edit.select {anchor, cursor}`,
`edit.insertText {text}`, `edit.gotoLine {line}`, `typeset.run`, `typeset.syncPdf`,
`typeset.inverseSync {page, x, y}`, `typeset.revealIssue {index}`, `pdf.setZoom {zoom}`,
`pdf.gotoPage {page}`, `view.*`, `typeset.mode.live|auto|manual`, `files.* {path}` (insert,
reveal, copyPath, rename `{path, name}`, duplicate, trash, newFolder/newFile `{dir}`), `git.*`
(refresh, init, stage/unstage/discard `{path}`, stageAll, unstageAll, commit `{message?}`, pull,
push, fetch, diff `{path, staged?}`), `ai.*` (send `{text?}`, stop, clear, insert/copy `{index}`,
selectModel `{provider, model}`, inline.toggle, inline.trigger (⌥\\), inline.accept (Tab), inline.acceptSentence (⌘→), inline.acceptWord, attachImage `{path}` (PNG/JPEG/GIF/WebP, models with Vision), removeImage `{index}`, inline.dismiss, inline.settings, toggleDocument, settings). `state.inspect` reports `typeset.mode`,
`git` (branch, staged, changes, busy, error) and `ai` (providers with `hasKey` only — API keys are
never reported — model, messages). `commands.list` documents the rest.

## One-shot screenshots

`texcraft <project> --screenshot out.png [--screenshot-delay-ms 2500] [--size 1388x824]` opens the
project, waits for typesetting, saves a screenshot and quits.
