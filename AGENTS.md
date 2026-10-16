# AGENTS.md: guide for AI agents and contributors

TexCraft is an open-source, native LaTeX editor written in **Rust only** — a clean-room
re-implementation of the look, feel and workflow of **Texifier** (macOS). The desktop app is
native egui/eframe on wgpu; no Tauri, Electron or webview shells, ever. The product name is
always written **TexCraft** in user-facing text; machine names stay lowercase (`texcraft-*`).
The goal is Texifier parity (same layout, toolbar, menus, shortcuts and behaviour) with every
feature drivable by agents. Read this file first, then `docs/`.

## 1. Orientation

| Read | Why |
|---|---|
| `docs/architecture.md` | Crate map, layers, the engine/UI seam, threads |
| `docs/texifier-reference.md` | What the target app looks like and does (public material only) |
| `docs/target-app-parity.md` | Where TexCraft stands against Texifier, and the ranked gap list |
| `docs/control-protocol.md` | How agents drive and screenshot the running app |
| `docs/development-guide.md` | The working method in detail (Chinese): measuring the target, parity scripts, pitfalls, checklist |

## 2. Workspace map

```text
crates/
  syntax log synctex   L0  pure leaf crates: LaTeX lexer/outline/completion data, TeX log
                           parser, SyncTeX parser. No workspace deps, no I/O beyond what the
                           API takes, compile for wasm32.
  pdf project          L1  hayro-based PDF rendering (render worker), project model
                           (root file, magic comments, file tree, settings)
  typeset              L2  runs the TeX engines (latexmk / tectonic) off the UI thread
  update               L2  update checks: latest.json, download + SHA-256, install after quit
  git                  L2  Source Control: runs the git command line off the UI thread, parses it
  ai                   L2  AI Assistant: providers, encrypted API keys, streaming chat over curl
  engine               L3  Session + command registry: every user action is a command
  ui                   L4  egui shell (thin: all actions go through the engine)
apps/
  texcraft             desktop app (eframe/wgpu), native menu bar, control server
  texcraft-cli         headless CLI (typeset, outline, issues, synctex, commands)
```

A crate may depend only on lower layers. Nothing below `ui` may use egui, eframe or winit.

## 3. Golden rules

1. **Never crash.** Non-test code never panics: no `unwrap()`, `expect()`, `panic!`,
   `unreachable!`, `todo!`, `unimplemented!`, no slice indexing with input-derived numbers (use
   `get()`), checked/saturating arithmetic on lengths and offsets, char-boundary-safe string
   slicing. Return the crate's error type. Enforced by workspace clippy lints (tests are exempt).
   A malformed log, a truncated SyncTeX file or a broken PDF produces an error, never a panic.
2. **No `unsafe`.** `unsafe_code = "forbid"` workspace-wide. The one exception is the isolated
   `crates/macos` (window-button placement, system spell checker): `unsafe_code = "deny"`, every
   block with a `SAFETY:` comment. Don't add `unsafe` anywhere else.
3. **Clean-room.** We study Texifier's public website, docs, App Store screenshots and observed
   behaviour only. Never copy its code, icons, images, templates, help text or other assets.
   Use permissive third-party assets only, and record each in `ATTRIBUTION.md`.
4. **Everything is a command.** New user-visible behaviour is a command in `crates/engine`
   (id, label, menu path, shortcut, `enabled`, `run`) plus tests. The menu bar, toolbar, CLI and
   control channel all dispatch commands by id.
5. **Tests are the gate.** Every change comes with tests. Parsers are tested against real
   artefacts produced by the installed TeX toolchain (oracle tests: the `synctex` CLI, real
   `.log` files from deliberately broken documents). Fixtures live in the crate's `tests/data/`.
6. **The UI is thin and data-driven.** Colours and metrics come from `ui::theme::Tokens`,
   never hard-coded in widgets.
7. **Verify UI changes visually.** Start an isolated instance (`scripts/parity/run-dev-app.sh`),
   drive it over the control channel and take `ui.screenshot` (separate windows have an `inline`
   mode: `ui.prefs`, `ui.welcome`). Compare side by side with the target (`scripts/parity/`) and
   look at the PNG. Measure, don't guess: sizes and font sizes come from the target's screenshots.
8. **Performance is a feature.** Typesetting and PDF rendering never block the UI thread.
   Keystrokes stay under one frame on a 5,000-line document.

## 4. Before you finish a task

```sh
cargo fmt --all
cargo test -p <crates you touched>
cargo clippy -p <crates you touched> --all-targets -- -D warnings
```

Then add a terse entry to `docs/devlog.md` (what landed, numbers, what is still open).

Never commit secrets or personal data. `scripts/install-hooks.sh` installs a gitleaks
pre-commit hook (`.gitleaks.toml`: keys, private files, home-folder paths, files over 5 MB) and
CI scans every push and audits dependencies (`deny.toml`). Fake keys in tests must not look
real; generated fixtures must not carry absolute local paths.

## 5. Parallel agents

- Use your own target dir (`CARGO_TARGET_DIR=target-<name>`) to avoid the Cargo build lock,
  and edit only the files you own. Delete it when you are done.
- Keep every `Cargo.toml` valid at all times: the `crates/*` glob means one broken manifest
  breaks everyone's build.
- If someone else's in-progress edit breaks the build, wait and retry; don't fix their files.
