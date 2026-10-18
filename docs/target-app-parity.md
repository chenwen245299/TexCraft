# Parity with the target app

Target: Texifier for macOS 1.9.33, as described in [texifier-reference.md](texifier-reference.md)
(public material only). Last reviewed 2026-10-15 at TexCraft 0.1.0.

Legend: ✅ matches · 🟡 partial · ❌ missing.

## Layout and chrome

| Area | Status | Notes |
|---|---|---|
| Unified toolbar, item order, capsule groups (macOS 26 look) | ✅ | 52 pt, window buttons centred (`texcraft-macos`), AUTO/LIVE, chain, sliders, status, export, view modes, back/forward, new; capsules are opaque, not glass. Full screen: the left capsule slides to the edge; leaving full screen it is back at once, and the buttons stay hidden during AppKit's animation. Gap: in full screen AppKit drops its title bar (with the buttons) as a separate strip, not inside our toolbar as in the target |
| Sidebar (200 pt), icon tabs, 26.5 pt aligned headers, nested outline, Files/Words rows | ✅ | 7 tabs: Source Control (Git) takes the place of the proprietary Connect, plus an AI Assistant tab; divider drag fixed |
| Typesetting modes | ✅ | toolbar label cycles LIVE → AUTO → MANUAL on click (`typeset.cycleMode`); ⌘T always runs a full typeset that writes the PDF (a running live pass gives way) |
| Files browser | ✅ | project folder row, chevrons, TEMP/ROOT/SUB/OUTPUT badges, the target's context menus (insert, new folder/file, reveal, copy path, rename in place, duplicate, Move to Trash) |
| Menus, pop-up lists, sheets | ✅ | one style (`crates/ui/src/menu.rs`): grey panel, 10 pt corners, 25 pt rows, accent highlight, check marks; dialogs are sheets with capsule buttons |
| Editor selection | ✅ | neutral tint, full-width block for whole lines, syntax colours kept |
| Source Control (not in the target) | ✅ | VS Code-like: branch, pull/push/fetch, message + Commit (⌘↩), staged/changes with stage/unstage/discard, diff sheet, recent commits |
| AI inline suggestions (not in the target) | ✅ | grey suggestion at the caret after a typing pause (Tab / ⌘→ word / Esc / ⌥\ now), ✦ button in the editor header with a popover for pause, length, context before/after and model |
| AI Assistant (not in the target) | ✅ | providers in Settings → AI (SoCLaaS at NUS, Anthropic, OpenAI, Gemini, OpenRouter, DeepSeek, Kimi, Qwen Token Plan, MiMo, Zhipu GLM, MiniMax, StepFun, MoleAPI, Ollama, custom: Argus's list), sidebar tab before Search, Settings pane after Editor, keys encrypted at rest, streaming answers with Insert/Copy, provider/model logos (LobeHub, Apache-2.0) |
| Interface type | ✅ | system font in its Text optical size (`opsz` 17) and semibold weight, sizes measured on the target |
| Scroll bars | ✅ | always-visible 11 pt thumbs, as the target shows them with a mouse attached |
| Settings window | ✅ | separate window, pane toolbar (General, Sidebar, Editor with Editing Aids / Look & Feel, Document Viewer, Typesetting), description–control rows; only TexCraft's own options (no snippets hotkey, AI completion, cloud) |
| Editor / PDF 50:50 split, ⌘2/⌘3/⌘4 | ✅ | draggable divider |
| Native window tabs | 🟡 | in-window tab strip when more than one file is open |
| No console / status bar | ✅ | log and issues live in the sidebar |
| Welcome / organiser window | ✅ | separate 780×480 window on the system's translucent material, the only window at launch; Documents/About/Licence and Recent Documents/Templates tabs, filter, "Last Modified" dates; no Account or Connect panes (no cloud service) |

## Typesetting

| Area | Status | Notes |
|---|---|---|
| Live typesetting on edit | 🟡 | external engine on mirrored buffers: ~0.23 s keystroke → PDF on the TikZ demo (pdfLaTeX with preamble, font-map and fast-environment caches); XeLaTeX ~0.35 s and LuaLaTeX ~0.45 s a pass (precompiled preamble, reduced font maps); the target's in-process engine is incremental |
| ⌘T, ⇧⌘L, ⌘K, ⌘. | ✅ | |
| Engines | 🟡 | pdfLaTeX, XeLaTeX, LuaLaTeX, Tectonic; no dvips/dvipdfmx/pLaTeX/ConTeXt/knitr/Markdown chains |
| Bibliography auto-sense | ✅ | via latexmk on full typesets; live passes reuse the last `.bbl` |
| Typeset Configuration sheet | ✅ | target layout: Live vs External typesetter columns with Auto-Typeset, Auto-Sense, Summary/General/Bibliographies/Indices tabs; no R in LaTeX or Build Scripts tabs |
| Auto-Sense engine | ✅ | fontspec/ctex/unicode-math or CJK text → XeLaTeX, luacode/luatexja → LuaLaTeX; magic comments win |
| CJK text without setup | ✅ | Auto-Sense loads xeCJK after `\documentclass` (`latexmk -usepretex`, or ahead of `\input` in live passes) with system fonts (Songti SC + Hiragino Sans GB on macOS; Fandol fallback); the file is not edited. XeLaTeX live passes take ~0.95 s vs ~0.23 s for pdfLaTeX |
| Errors in margin, Issues list, Log | ✅ | |
| Magic comments | ✅ | `% !TEX program/root/TS-program`, `% !BIB program` |

## Editor

| Area | Status | Notes |
|---|---|---|
| Syntax colours (sampled palette, light/dark) | ✅ | |
| Line numbers, current line, error flags | ✅ | |
| Soft wrap with hanging indent | ✅ | continuation rows shifted by two characters |
| Autocomplete: commands, environments, `\ref`, `\cite`, files | ✅ | Tab/Return accept, Esc toggles; no placeholder tokens |
| Auto-itemise, auto `\end`, auto-indent, comment toggle, indent | ✅ | |
| Find & replace bar, global search, Jump to | ✅ | |
| Snippets menu (⌃⇧ + letter) | ✅ | fixed set, not user-editable yet |
| √x symbol table | 🟡 | 10 categories |
| Spell check | ✅ | prose only (text and comments), system word list + macOS checker, dotted underline, suggestions / Learn / Ignore |
| Editor themes | 🟡 | Classic (cream, olive comments) and Plain (white); dark editor in dark mode |
| Info boxes (cite/ref/image previews on hover) | ❌ | |

## PDF viewer

| Area | Status | Notes |
|---|---|---|
| Zoom −/%/+, page navigation, "Page N of M" | ✅ | |
| Page modes | 🟡 | 3 of 6 |
| SyncTeX ⌘-click both ways, flash | ✅ | |
| Scroll lock | 🟡 | PDF follows the editor, not the reverse |
| Dark-mode grading | 🟡 | whole page only |
| Zoom percentage relative to fit width, edge-to-edge pages | ✅ | |
| Thumbnails | 🟡 | grid view, click opens the page |
| Header bar (thumbnails, zoom, first/previous, page, next/last, scroll lock, open externally, separate window, pause, more) | ✅ | positions measured to within 1 pt |
| Separate PDF window (external display) | ✅ | |
| Text selection and copy | ✅ | drag, double/triple click, ⌘C; English and CJK (also fonts without `ToUnicode`) |
| Inverse search lands on the clicked word | ✅ | its sentence (else the line) flashes in the editor |
| Links, PDF search | ❌ | |

## Updates and distribution

| Area | Status | Notes |
|---|---|---|
| Automatic update check, download and install | ✅ | GitHub releases + `latest.json`; the target uses its own updater, TexCraft's window follows the author's Argus app |
| Release builds | ✅ | tag-triggered GitHub Actions: macOS universal (ad-hoc signed, not notarised), Windows x64 |

## Files and export

| Area | Status | Notes |
|---|---|---|
| Open file/folder, recent, templates, save, save as, revert | ✅ | recent documents list each project once, by its main file (a chapter, `.bib` or figure opened stands for its root); a full typeset writes `main.pdf` beside `main.tex` (intermediates stay in `.texcraft`) |
| Reveal PDF, Export PDF | ✅ | |
| Print, email, zip export, encodings, duplicate/rename/move | ❌ | |

## Ranked gaps (work list)

1. PDF search and clickable links.
2. Texifier Connect tab (proprietary cloud collaboration: out of scope; the tab is Source Control).
3. Hover info boxes and inline image previews.
4. User-editable snippets and templates folders; autocomplete placeholders with Tab jumps.
5. Faster live passes for XeLaTeX/LuaLaTeX (preamble caching does not apply).
6. Printing and zip export.
7. Windows/Linux testing; wasm build of the L0–L1 crates.
