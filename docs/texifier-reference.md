# Texifier for macOS — Clean-room UI/Behaviour Reference

Target: Texifier for macOS (formerly Texpad), Valletta Ventures (UK) Ltd.
Current version at time of research: **1.9.33** (released 16 Jan 2026; Mac App Store requires macOS 14.0+).
Researched: 2026-10-15, from public docs, release notes, App Store listing and official doc screenshots only.

Conventions in this file:
- Everything is paraphrased. Short UI labels (menu items, button titles) are reproduced as labels.
- **(unconfirmed)** = stated in older docs or inferred but not seen in a current (1.9.x) screenshot.
- **(guess)** = educated guess, flagged explicitly. **unknown** = public material is silent.
- Many official screenshots were taken before the 2022 rebrand, so the app menu and some strings say
  "Texpad"; the UI is otherwise the 1.9.x UI. Pixel measurements come from 2x Retina screenshots
  and are given in points (pt). Colours were sampled from those screenshots (approximate, ±2).

---

## 1. Window layout

One window = one **project** = one root `.tex` (or `.md`) file producing one PDF. Multiple root
documents need multiple windows (the FAQ explicitly says one root per window; use macOS split screen).

### Regions (left → right, top → bottom)

| Region | Position | Notes |
|---|---|---|
| Unified title bar + toolbar | top, full width | Compact unified style, ~38 pt tall. Traffic lights at far left. See §2. |
| Native tab bar | directly under toolbar, full width | Only visible when the window has >1 tab (standard macOS window tabs). Each tab title = file name shown in that tab. |
| Sidebar | left column | Default ~200 pt wide. Tabbed (6 icon tabs). See §3. |
| Editor pane | middle | Has its own 26 pt header bar ("jump bar": root file › current file, plus √x Symbols button at right). |
| PDF viewer pane | right | Has its own 26 pt header bar (PDF toolbar). See §5. |
| Console / issues | **not a separate bottom pane** | Errors, warnings and raw log live in the **sidebar** "Typeset Results" tab. Since 1.8.0 nothing pops over the PDF. |
| Status bar | **none** | No bottom status bar under editor or PDF. Word/file counts are in the sidebar header. |

### Measured proportions (official full-window screenshot, window 1388 × 824 pt)

| Item | Size | Share of width |
|---|---|---|
| Sidebar | 200 pt | 14.4 % |
| Divider (sidebar/editor, editor/PDF) | 1 pt, colour ≈ #B6B5B6 (light) / #5F5E5F (dark) | — |
| Editor | 593 pt | 42.7 % |
| PDF viewer | 593 pt | 42.7 % |
| Toolbar height | ~38 pt | — |
| Editor/PDF header bars and sidebar tab-icon row | 26 pt each, all aligned on one horizontal line (by design since 1.8.0), 1 pt bottom separator | — |

So: sidebar fixed-ish narrow column, editor and PDF split the remainder **50/50**.

### Showing / hiding panes

- **Sidebar**: toolbar sidebar button (left of the typeset button); also "Toggle Outline" **⌘1** (docs).
- **View mode** (3-segment toolbar control, also **⌘2 / ⌘3 / ⌘4**):
  1. Editor only, 2. Editor + Output (side by side, default), 3. Output (PDF) only.
- **Split orientation**: only side-by-side (editor left, PDF right) is shown or documented.
  A vertical/stacked split is **unknown** (no evidence it exists). PDF can additionally be sent
  to an external display (monitor button in PDF toolbar).
- Pane dividers are draggable (standard split view; exact min/max widths unknown).
- Toolbar can be collapsed / styled "Icons Only" vs with titles (General prefs → Toolbar Style).
- Window and tab state (open tabs, PDF positions) is cached and restored on reopen ("workspace cache";
  can be reset in General prefs).

### Tabs
- New tab **⇧⌘T** clones the current tab's state; tabs may only hold files of the same project.
- Each tab remembers its own editor location and PDF location.
- Close Tab **⌘W**, Close Window **⇧⌘W**. Previous/next tab: docs say **⇧⌘[ / ⇧⌘]** (see conflict note in §8).
- A "New Tab" toolbar button sits at the far right of the toolbar.

---

## 2. Toolbar (left → right, from official 1.9.x screenshots)

| # | Item | Icon / look | Behaviour |
|---|---|---|---|
| 0 | Traffic lights | standard | — |
| 1 | Sidebar toggle | rounded rect with left column (SF "sidebar.left"-like); shows pressed state when sidebar visible | Show/hide sidebar. |
| 2 | **Typeset** button | two interlocking cogs (large + small) | Starts a typeset. Cogs animate/spin while typesetting; clicking again while spinning aborts. Optional animation during auto/live typeset (pref). |
| 3 | **LIVE** indicator/switch | the word "LIVE" in green text (≈ #6CC24A) | Shown when the built-in TexpadTeX live typesetter is active. Release notes describe "flicking the switch" to leave Live mode for an external TeX install. Exact click behaviour (unconfirmed). |
| — | thin vertical separator | 1 pt grey | |
| 4 | **Typeset chain summary** | plain grey text, e.g. `TexpadTeX`, `TexpadTeX > Bibliography`, `pdfLaTeX` | Short description of the tool chain that will run. |
| — | separator | | |
| 5 | **Typeset configuration** | vertical sliders icon (three faders) | Opens the per-project Typeset Configuration **sheet** (§6). |
| — | separator | | |
| 6 | **Status / issues indicator** | green circled check-mark on success; red number (e.g. "2") when errors | Count is project-wide. Clicking shows the issues (in 1.9.x the Issues list is in the sidebar; old docs call it a dropdown). Warning-only colour (unconfirmed; guess yellow). |
| — | flexible space | | |
| 7 | **Share / Export** | square with up-arrow (share) | Popover titled "Export files": Reveal PDF in Finder…, Export PDF…, Email PDF…, Email project as Zip…, Export project as Zip…, Export project as folder…, Export .zip including intermediate files… |
| 8 | **View mode** segmented control (3 segments) | (a) empty rectangle = Editor only; (b) outlined rect + filled dark rect = Editor + Output (selected by default); (c) filled dark rect = Output only | Same as ⌘2/⌘3/⌘4. |
| 9 | **Back / Forward** | `<` and `>` chevrons (forward greyed when unavailable) | Navigation history of jumps (issue clicks, outline clicks, search results) for the current tab. Shortcut unknown. |
| 10 | **New Tab** | rectangle with "+" badge at top-right | New tab in the same project. |

Toolbar colours: light ≈ #EDEDEE background, icons ≈ #717171 grey; dark ≈ #232425.
Typeset-chain text is grey; LIVE green; error count red.

---

## 3. Sidebar

Top: a row of **6 icon tabs** (centered, ~26 pt tall). The selected tab is drawn as a white glyph
in a filled accent-colour (system accent, default blue ≈ #0070F4) rounded square; others are grey glyphs.
Bottom: a **Filter** search field (with filter icon) plus a gear/action button (content varies by tab).
Light background ≈ #E1E1E2 (vibrancy over desktop), text ≈ #444444, rows ~22 pt tall.

| # | Tab | Icon | Contents and behaviour |
|---|---|---|---|
| 1 | **Contents** | viewfinder/document-in-brackets | Header "metadata" block: **Files** N, **Words** N (word count of the typeset output). Then a segmented control **Structure \| Subfiles**. See below. |
| 2 | **TODOs** | checked checkbox | Header "N TODOs" (grey, centred). List rows: checkbox icon + truncated TODO text. Picks up comments starting with `todo`/`TODO`/`todo:` (e.g. `% TODO fix`) and `\todo{…}` from todonotes. Click = jump (guess, standard). |
| 3 | **Connect** | infinity sign ∞ | Texifier Connect (cloud collaboration) panel; only meaningful when the project is a Connect project. |
| 4 | **File Browser** | folder | Tree of the root file's folder (incl. subfolders, `.texpadtmp`). Each row has a right-aligned role badge: `ROOT`, `SUB` (part of project), `OUTPUT` (the PDF), `TEMP` (intermediate folder). Bottom: Filter field and `+` button (new file/folder). Dragging a file onto the editor inserts `\include{…}` or `\includegraphics{…}` at the drop point. |
| 5 | **Typeset Results** | warning triangle | Header: "Last typeset Today at HH:MM:SS (Duration X seconds)". Segmented **Issues \| Log**. Issues: summary line ("2 errors and 0 warnings" in red, or "0 errors and 0 warnings" in green), then list; on success the empty area shows a large grey "Typeset Successful". Log: raw monospace typesetter output. Opens automatically after a failing typeset (pref "Show Error Pane Post Typeset"). |
| 6 | **Search** | magnifier | Segmented **Text Search \| Tag Search** (see below). |

### Contents → Structure (outline)
- Collapsible tree with disclosure triangles. Top-level groups observed: **Table of Contents** (with count
  badge, e.g. 69), **Citations**, **Figures** (count), **Tables** (count), **Labels** (count, e.g. 126);
  bibliography appears in the outline too.
- Inside Table of Contents: "Begin Document" / "End Document" (hourglass-like icons), "Bibliography: …",
  and sectioning entries with small letter badges: **C** = chapter (filled dark rounded square),
  **S** = section (filled), outlined **S** = subsection; deeper levels optional.
- Figures use small orange outlined squares, Tables green outlined squares, Labels orange outlined
  squares; figure rows read like "Figure (liouville.tex: 804)".
- A **blue vertical bar** (≈ #4E9BF8, ~5 pt wide) at the right edge of a row marks the section that
  currently contains the editor cursor (live).
- Click a row → editor jumps there and (if SyncTeX available) PDF jumps to the matching spot.
- Drag a row onto the editor → inserts a reference to it (e.g. `\ref{label}`).
- Bottom Filter field filters rows by text. Gear menu "Choose the level of detail…":
  Show subsections (default ✓) / Show subsubsections / Show paragraphs.
- Section titles are shown truncated with "…"; maths in titles shown raw (e.g. `$\susy{1}$`).

### Contents → Subfiles (project files)
- Tree by inclusion: root file first, marked with a circle-dot icon and a grey outlined `ROOT` pill;
  children nested under the file that includes them (e.g. a `.sty` under the `.sty` that loads it,
  images under the chapter that includes them).
- File icons: document glyph labelled TEX / IMG etc.
- Listed: root, `\input`/`\include` subfiles, `\includegraphics` images, local `.sty`/`.cls`/`.bib`,
  and files forced via a `%force-texpad-dependency: path/file.ext` comment.

### Search → Text Search (global, ⇧⌘F)
- Search field with magnifier+chevron (options menu) and clear (ⓧ) button; **Search** button.
- Options menu: "Ignoring Case" (toggle), "Treat search term as a regex" (ICU regex), "Recent Searches"
  list, "Clear Recent Searches".
- Replace field (placeholder mentions `$1, $2` for regex groups), buttons **Replace All** and **Replace**.
- Status line "61 results in 11 files". Results grouped by file (disclosure + file icon + name + count
  badge); each hit row = "line. context" with the match highlighted (accent-tinted background).
- Up/Down move through results, Return jumps & focuses editor, Esc closes without clearing.
- Empty state: large grey "Text Search" + hint text.

### Search → Tag Search ("Jump to", ⇧⌘O)
- Field + "N results" + rows with a star-in-square icon, title, and "file.tex : line" subtitle.
- Matches section titles, file names, citation keys, label keys, and line numbers.
- Up/Down select, Return jumps and focuses the editor.

There is **no** dedicated Symbols or Snippets sidebar tab: symbols are a popover from the editor
header (√x), snippets are a menu-bar menu.

---

## 4. Editor

### Look
- Default font **Source Code Pro 12 pt** (Editor → Look & Feel); line height ≈ 15 pt (estimate).
- **Soft wrap with hanging indent** is the default: wrapped continuation lines are indented by one tab
  width; line numbers are shown only on the first visual line of each logical line.
- Line numbers on by default, right-aligned in a ~38 pt gutter, light ≈ #939393 / dark ≈ #676767;
  current line number drawn darker/bolder; current line gets a light grey band.
- Error lines: the gutter number is replaced by a **red tag/flag-shaped badge** with the line number in
  white (errors appear "in the margin" during live typeset).
- Editor header ("jump bar"): root-file icon + root name `›` file icon + current file name; right side
  √x (Symbols). For single-file docs only the file name is shown.

### Syntax colours (default theme, sampled)

| Category | Light theme | Dark theme |
|---|---|---|
| Background | #FFFFFF | #2E2E2E |
| Plain text, braces, arguments (`{figure}`, label keys, cite keys) | #000000 | #B4B4B4 |
| Control sequences `\begin`, `\section`, `\frac`, `\cite`, custom macros | #0E6FD0 (blue) | #00A0BE (cyan/teal) |
| Math (`$…$`, `\(…\)`, and the math delimiters) | #D06F0E (orange) | #D06F0E (orange) |
| Comments `% …` | #339666 (green) | #42B645 (green) |
| Line numbers | #939393 | #676767 |
| Spelling error underline (dotted) | #FF7268 | #E07D7D |
| Global-search hit highlight | accent-tinted (e.g. #FCD3AF with orange accent) | unknown |

Other categories: unknown (no evidence of separate colours for environments names, `\label` keys, or
brackets). Embedded languages are highlighted inside their environments (Lua in `luacode`, R/Sweave,
Python, Haskell, Lilypond, minted); dollar signs inside verbatim-like environments are not treated as
math. Bundled theme count: "five standard colour themes" (older docs); default is named
"Texpad Default" in screenshots (current name unknown). Other theme names unknown.
Line-number gutter follows the theme colours.

### Editing aids (Preferences → Editor → Editing Aids)
- Show line numbers ✓ · Auto-complete commands ✓ · Auto Indent ✓ (copy previous line's leading whitespace)
  · Automatch `\begin` with `\end` ✓ · Editor tab indent size (Tab / 1 / 2 / 4 spaces)
  · Line wrapping (default "Soft wrap with hanging indent"; 1.7.0 also added an 80-column hard-wrap
  mode; other options unknown) · Hanging indent size (default "Tab Width") · Snippets base hotkey
  (default Ctrl+Shift) · "Use math expression auto-completion" (macOS 15 system feature, toggle since 1.9.28).
- **Auto-itemise**: Return on an `\item` line inside itemize/enumerate inserts a new `\item`.
- **Return after `\begin{env}`** inserts the matching `\end{env}`.
- **Comment/uncomment** selection: ⌘/ (toggle). **Indent/unindent**: ⌘] / ⌘[ per aids page, ⇧⌘] / ⇧⌘[ per menu page (conflict, see §8).
- **Bracket matching**: a parenthesis-match indicator exists (mentioned in release notes); style unknown.
- **Auto-pairing** of `{}`, `[]`, `$$`: unknown (not documented).
- **Code folding**: unknown / no evidence it exists.
- **Spell check** as you type (macOS checker), LaTeX-aware (skips commands).
- **Word count**: shown in sidebar Contents header (counts typeset output); no status-bar count.

### Autocomplete
- Pops up automatically while typing a `\command`; Esc also force-opens it (Edit → Autocomplete) and
  dismisses it. ↑/↓ select, **Tab** accepts, → (right arrow) cancels and moves on (since 1.9.7).
- Popover: list of candidates (selected row = accent background, white text; command part in blue),
  below it an **info panel** describing the highlighted entry, and a **Jump to** button for labels.
- Completes: built-in command database (incl. mathtools etc.), user `\newcommand`, `\DeclareMathOperator`,
  `\newenvironment`; argument "autofill" for `\ref{` (labels, with info "Chapter/Section/Subsection"
  context), `\cite{` (keys from `.bib` and `\bibitem`, info shows BibTeX fields), `\begin{` (environments;
  inserting also adds `\end{…}`), `\usepackage{`, `\bibliographystyle{`, and more.
- Placeholders are drawn as **blue badge tokens**; Tab jumps between them.
- Custom list: `autocompleted_commands.txt` in Application Support (Texifier menu → Add/Edit
  Autocompleted Commands…); line syntax `[*]key [| description] [E.g. example] [@ files] [@@ files]`,
  `stage:value` for argument autofill, `=new:existing` for autofill aliases. Restart required.

### Info boxes
- Rest the cursor inside `\cite{}`, `\ref{}` or `\includegraphics{}` → a summary pops over the editor:
  BibTeX fields / label context (section titles) / **image preview**.
- Inline math preview: unknown (not documented; guess: absent).

### Snippets (menu-bar "Snippets" menu)
- Plain-text files in Application Support/Snippets; keys `ctrl key`, `group`, `insert at cursor`
  (with `<SELECTION>` placeholder that wraps the current selection). Hotkey = base (default ⌃⇧) + letter.
- Shipped defaults (grouped): see §7 Snippets menu.

### Symbols (√x button in editor header)
- Drop-down panel: 6-column grid, each cell = rendered glyph + command name (e.g. Π `\Pi`).
- Category headers stacked as clickable bars (click jumps to section): Greek letters, Binary Relations,
  Binary Operators, BIG Operators, Arrows, Delimiters, Large delimiters, Misc Maths, Text mode symbols,
  AMS Delimiters, AMS Greek and Hebrew, AMS Binops, AMS Binary Operators, AMS Binary Relations, AMS Arrows.
- Click inserts the command; hover shows full name tooltip.

### Find / replace
- In-file: standard macOS find bar slides in under the editor header (⌘F): search field with
  options chevron, match count, clear, `<` `>` buttons, **Done**, and a **Replace** checkbox that reveals
  the replace row. Matches use the system yellow find indicator.
- Project-wide: sidebar Text Search (⇧⌘F); selected text is pre-filled.
- Jump to: ⇧⌘O (Tag Search).

---

## 5. PDF preview

### PDF toolbar (26 pt header above the PDF, left → right)

| Item | Look | Behaviour |
|---|---|---|
| Thumbnails | 3×2 grid icon | Toggles a thumbnail/grid view of pages. |
| Zoom out | magnifier with − | Step zoom out. |
| Zoom % | e.g. "100%" | Click to type an exact zoom. |
| Zoom in | magnifier with + | Step zoom in. |
| separator | | |
| First / Previous page | `«` `‹` | |
| Page label | "Page 96 of 159", centred, grey | (truncates to "Pag…" when narrow) |
| Next / Last page | `›` `»` | |
| separator | | |
| **Scroll lock** (SyncTeX lock) | document icon with ⇆ arrows; filled accent when on | Editor and PDF scroll together, tops kept in sync. |
| Accent-filled ↗ arrow button | blue square with diagonal arrow | Purpose **unknown** (guess: auto-reveal/follow cursor in PDF after typeset, or the toolbar "pin"). |
| External display | monitor icon (or two overlapping rectangles in some shots) | Choose a screen to show the PDF on. |
| **Pause live typeset** | two vertical bars ‖ | Temporarily suspends live typesetting (added 1.8.8). |
| More ••• | three dots | Menu: "Select Page Mode" + "Image Grading Type in Dark Mode" (below). |

### Page modes (••• menu)
Single Page Continuous (default ✓) · Two Pages Continuous · Two Pages Continuous with Cover Page ·
Single Page Continuous, separated · Single Page · Two Pages.

### Zoom
- Steps via buttons or ⌘+ / ⌘= and ⌘−; typed percentage; a "zoom to fit" option exists (1.7.42; where
  exposed: unknown). Fit-width/fit-page as named modes: unknown.

### Navigation
- Toolbar buttons, arrow keys page through the PDF, clickable internal (hyperref) and web links.
- Light-mode pane background is white (page edge barely distinguishable in screenshots).

### SyncTeX
- **Forward (editor → PDF)**: ⌘-click in the editor; or click an outline row. The target is flashed with a
  light-blue box in the PDF. (iOS docs list ⌘D "sync PDF to editor position" as macOS-compatible: unconfirmed on Mac.)
- **Inverse (PDF → editor)**: ⌘-click in the PDF; or right-click → "Synchronise PDF". Cursor lands at the
  closest character. Pref "Single-click PDF sync with editor" makes a plain click do it.
- Live typesetter keeps SyncTeX data in memory; external engines write `.synctex.gz` next to the root.
- Scroll lock button gives continuous sync.

### Dark mode ("grading")
- In system dark mode the viewer re-tints the PDF: page background ≈ #2E2E2E, black text → ≈ #B4B4B4
  (it is a display-only colour mapping; exports/prints unchanged).
- Options: Grade Entire Viewer Output (default) · Grade Everything Apart From Images · No Grading.
  Global default in Preferences → Document Viewer; per-project override in the PDF ••• menu.

---

## 6. Typesetting

### Typesetters
- **TexpadTeX** (built-in, default, "LIVE"): own TeX engine (LaTeX + Plain TeX) bundled with the app,
  works offline, typesets on every keystroke; package bundles downloaded via an in-app bundle manager;
  embedded bibliography (BibTeX-like "TexpadTeX Bib") and makeindex-compatible indexer.
  Some packages unsupported (historically fontspec, hyperref); Texifier then auto-switches to an
  external/cloud typesetter.
- **External typesetter** (installed MacTeX/TeX Live; distribution picked in Preferences, with
  "Set Custom Distribution" and a health check). Chains: **pdfLaTeX**, **LaTeX dvi2pdf**, **LaTeX dvips
  ps2pdf**, **XeLaTeX**, **LuaLaTeX**, **pLaTeX dvipdfmx**; also Plain TeX, ConTeXt, Knitr/Sweave (R in
  LaTeX), Markdown (converted then typeset through LaTeX). **latexmk is not used/mentioned** — Texifier
  computes the run sequence itself and skips unnecessary passes.
- **Cloud typesetter** (TeX Live on Texifier servers, free) — for the sandboxed Mac App Store build
  (and iOS); results shown as if local.
- **Custom `.tpbuild` scripts** (bash): global in `~/Library/Application Support/Texifier/BuildScripts/`
  or `root.tpbuild` beside the root; env vars `TEXPAD_ROOTFILE`, `TEXPAD_ROOTFILE_NO_EXT`; must
  produce `<root>.pdf`.

### Typeset Configuration sheet (sliders button)
- **Typesetter** section: radio **Built-in TexpadTeX Typesetter** [LIVE badge] ("Realtime typesetting ·
  Lean · Simple") vs **External Typesetter** ("Traditional · Large Download"), with an **Auto-Typeset**
  checkbox for external ("start a typeset when typing pauses").
- **Typeset Configuration** section: **Auto-Sense** checkbox (on by default: parsers watch the source and
  adjust the chain, e.g. adding `\bibliography{}` adds a bibliography run). Turning it off = manual mode.
- Tabs: **Summary** (bullet list, e.g. "Typeset locally with pdfLaTeX", "Store intermediate files in
  .texpadtmp", "Compile bibliographies with TexpadTeX Bib") · **General** (engine etc.; details unknown)
  · **Bibliographies** ("Bibliography Engine" popup: None / BibTeX / Biber / BibTeX8 …) · **Indices**
  (checkbox runs: Index (makeidx), Index (glossaries), Glossaries, Nomenclature, Numbers, Notations,
  Symbols, Acronyms) · **R in LaTeX** · **Build Scripts**. **Close** button bottom-right.

### Live / auto typesetting
- Live mode (TexpadTeX) re-typesets continuously; errors appear in the editor margin immediately.
- External mode: optional Auto-Typeset when typing pauses; Toggle Auto-typeset ⇧⌘L; manual ⌘T.
- Pause button in PDF toolbar suspends live typesetting.
- After a typeset (pref "Post-typeset pane behaviour"): default "Ensure PDF visible and focus editor".
- Abort: ⌘. or click the spinning cogs. Max typeset operations per type: 30 (1.9.26).
- OS notification on completed typeset (pref, on by default).

### Errors / warnings
- Parsed from the log into the sidebar Issues list. Row: severity icon (red triangle = error,
  yellow triangle = warning, green = minor layout/bad-box alerts), bold "file.tex [page] (line N)",
  grey message ("Undefined control sequence. (\anError …)"), and a circled-arrow button at right that
  opens the matching spot in the raw Log. Click row → jump to line (pushed on back/forward history).
- Detail level pref: Errors Only (Recommended) / Errors & Warnings / Split Hairs.
- Context menu on issues allows copy to clipboard. Undefined refs/cites show as `??`/`[?]` in PDF.
- Remove intermediate files (`.texpadtmp` hidden folder): ⌘K.

### Bibliographies / indices
- Auto-sensed: BibTeX vs Biber (biblatex backend), multibib, glossaries, xindy, splitindex.
- Live typesetter updates the reference list when `.bib` or citations change.
- Global BIBINPUTS/BSTINPUTS settings exist for BibTeX (older prefs docs).

### Magic comments
- `% !TEX program = …`, `% !TEX root = …`, `TS-program`: **not documented anywhere public; unknown**
  whether supported. Root detection is automatic: the file containing `\begin{document}` (or a
  Plain TeX `\bye`) is the root; opening the root opens the whole project.
- Documented directive: `%force-texpad-dependency: relative/or/absolute/path.ext` (force a file into
  the project / upload a font to the cloud typesetter).
- Shell-escape: global policy pref ("Allowed" default for autosense) with per-project override.

---

## 7. Menu bar

Observed top-level order (1.9.x screenshot): **Texifier** (shown as "Texpad" pre-rebrand) · **File** ·
**Edit** · **Snippets** · **Typeset** · **Workspace** · **Window** · **Help**. There is no menu
literally called "View" or "Format/Insert".

### Texifier (app) menu

| Item | Shortcut | Notes |
|---|---|---|
| About Texifier | — | version, legal, credits |
| Texifier for iPad/iPod/iPhone | — | link to iOS app |
| Preferences… | ⌘, | (may read "Settings…" on newer macOS — unconfirmed) |
| Add/Edit Autocompleted Commands… | — | reveals `autocompleted_commands.txt` |
| Check for Updates… | — | (unconfirmed; direct-download build auto-updates) |
| Services / Hide Texifier ⌘H / Hide Others ⌥⌘H / Show All / Quit ⌘Q | std | standard macOS |

### File menu (from 1.9.x screenshot, in order)

| Item | Shortcut | Notes |
|---|---|---|
| New Document | ⌘N | |
| New Tab | ⇧⌘T | |
| Open… | ⌘O | |
| Recent Documents ▸ | — | |
| — | | |
| New from Template ▸ | — | templates from Application Support/Templates |
| Add/Edit Templates… | — | reveals templates folder |
| — | | |
| Save | ⌘S | |
| Duplicate | ⇧⌘S | single file |
| Duplicate entire project | — | |
| Move To… | — | |
| Rename… | — | |
| Revert To ▸ | — | |
| — | | |
| Share ▸ | — | macOS share submenu |
| — | | |
| Close Window | ⇧⌘W | |
| Close Tab | ⌘W | |
| — | | |
| Jump to | ⇧⌘O | opens Tag Search |
| — | | |
| Page Setup… | ⇧⌘P | |
| Print LaTeX source… | ⌥⌘P | |
| Print PDF… | ⌘P | |
| — | | |
| Reveal PDF in Finder… | ⇧⌘R | older docs: ⌘R |
| Email PDF… | ⇧⌘E | older docs: ⌘E |
| Email project as Zip… | — | |
| Export project as Zip… | ⌥⌘E | |
| Export project as folder… | — | |
| — | | |
| File Encoding ▸ | — | "UTF8 - Recommended for most LaTeX documents" ✓, then common (Central European ISO Latin 2 / Windows Latin 2, Cyrillic (Windows), Greek (Windows), Japanese EUC / ISO 2022-JP / Windows-DOS, Turkish, Western ISO Latin 1 / Mac OS Roman / Windows Latin 1), then a long full list |
| Reload with File Encoding ▸ | — | (unconfirmed in 1.9.x screenshot; in docs) |

### Edit menu

| Item | Shortcut | Notes |
|---|---|---|
| Undo / Redo | ⌘Z / ⇧⌘Z | standard |
| Cut / Copy / Paste / Select All | ⌘X / ⌘C / ⌘V / ⌘A | standard |
| Find ▸ (Find…, Find Next, Find Previous, Use Selection for Find, Jump to Selection) | ⌘F, ⌘G, ⇧⌘G, ⌘E, ⌘J | standard NSTextView items (unconfirmed; note ⌘J conflicts with "switch focus" in cheatsheet) |
| Comment Block / Uncomment Block | ⌘/ (toggle) | |
| Indent Block / Unindent Block | ⇧⌘] / ⇧⌘[ (docs) | aids page says ⌘] / ⌘[ — conflict |
| Autocomplete | Esc | force-open popup |
| Global Search | ⇧⌘F | focuses sidebar Text Search |
| Spelling and Grammar ▸, Substitutions ▸, Transformations ▸, Speech ▸ | std | (unconfirmed) |
| Start Dictation / Emoji & Symbols | std | standard (unconfirmed) |

### Snippets menu (shipped defaults, from screenshot)

| Item | Shortcut | Group |
|---|---|---|
| Add/Edit Snippets… | — | (top) |
| Preamble | ⌃⇧P | Other |
| Bold | ⌃⇧B | Inline |
| Italic | ⌃⇧I | Inline |
| Typewriter | ⌃⇧T | Inline |
| Underline | ⌃⇧U | Inline |
| Example | ⌃⇧G | Inline |
| Colour Red | ⌃⇧R | Colours |
| Floating Table | ⌃⇧L | Floating |
| Figure | ⌃⇧F | Floating |
| Displaymath | ⌃⇧Y | Environment |
| Itemize | ⌃⇧Z | Environment |
| Enumerate | ⌃⇧M | Environment |
| Quotation | ⌃⇧Q | Environment |
| Equation | ⌃⇧E | Environment |
| Verbatim | ⌃⇧V | Environment |
| Tabular | ⌃⇧A | Environment |
| Description | ⌃⇧D | Environment |
| Matrix | ⌃⇧X | Environment |

Group names are drawn as grey section headers with separators between groups.

### Typeset menu (from menu docs; may be older than 1.9.x)

| Item | Shortcut | Notes |
|---|---|---|
| Typeset | ⌘T | single smart typeset (since 1.7, replaced ⌘B/⌘I/⌘L) |
| Add/Edit Global .tpbuild Scripts | — | reveals BuildScripts folder |
| Toggle Auto-typeset | ⇧⌘L | per project |
| Remove Intermediate Files | ⌘K | deletes `.texpadtmp` |
| Abort Typesetting | ⌘. | |
| Typeset Configuration… / engine choices | — | (unconfirmed whether present as menu items) |

### Workspace menu (contents not publicly documented; items below are those the docs place under "Window"/view — placement unconfirmed)

| Item | Shortcut | Notes |
|---|---|---|
| Toggle Outline (sidebar) | ⌘1 | (unconfirmed placement) |
| Editor Only | ⌘2 | (unconfirmed placement) |
| Editor and Output | ⌘3 | (unconfirmed placement) |
| Output Only | ⌘4 | (unconfirmed placement) |
| Switch focus Editor ↔ Output | ⌘J | cheatsheet (unconfirmed placement) |
| Show PDF / Show Log (issues) | ⌘8 / ⌘9 | old docs: switch output pane PDF↔log; iOS: issues / log. (unconfirmed) |
| Zoom In / Zoom Out (PDF) | ⌘+ (⌘=) / ⌘− | (unconfirmed placement) |
| Back / Forward | unknown | (unconfirmed) |

### Window menu

| Item | Shortcut | Notes |
|---|---|---|
| Minimize / Zoom / Fill & Arrange | ⌘M / — | standard |
| Show Welcome Window (Organiser) | ⌃⌘T | docs |
| Show Previous Tab / Show Next Tab | ⌃⇧⇥ / ⌃⇥ | standard tabbing (unconfirmed) |
| Move Tab to New Window / Merge All Windows | — | standard (unconfirmed) |
| Bring All to Front, open windows list | — | standard |

### Help menu

| Item | Shortcut | Notes |
|---|---|---|
| Search field | ⇧⌘/ | standard |
| Texifier Help / documentation link | — | (unconfirmed; docs live at texifier.com/docs) |

---

## 8. Keyboard shortcuts (consolidated)

| Action | Shortcut | Source / status |
|---|---|---|
| Preferences | ⌘, | docs |
| New document / Open / Save | ⌘N / ⌘O / ⌘S | 1.9.x screenshot |
| New tab / Close tab / Close window | ⇧⌘T / ⌘W / ⇧⌘W | 1.9.x screenshot |
| Previous / next tab | ⇧⌘[ / ⇧⌘] | tabs docs (conflicts with indent per menu docs) |
| Duplicate | ⇧⌘S | screenshot |
| Jump to (Tag Search) | ⇧⌘O | screenshot + docs |
| Global search (Text Search) | ⇧⌘F | docs |
| Find in file | ⌘F | standard (unconfirmed) |
| Page Setup / Print source / Print PDF | ⇧⌘P / ⌥⌘P / ⌘P | screenshot |
| Reveal PDF in Finder | ⇧⌘R | screenshot (old docs ⌘R) |
| Email PDF | ⇧⌘E | screenshot (old docs ⌘E) |
| Export project as Zip | ⌥⌘E | screenshot |
| Typeset | ⌘T | docs |
| Toggle auto-typeset | ⇧⌘L | docs |
| Remove intermediate files | ⌘K | docs |
| Abort typeset | ⌘. | docs |
| Toggle comment | ⌘/ | docs |
| Indent / unindent | ⌘] / ⌘[ (aids page) or ⇧⌘] / ⇧⌘[ (menu page) | conflicting docs |
| Autocomplete (force) / dismiss | Esc | docs |
| Accept completion / next placeholder | Tab | docs |
| Cancel completion and move right | → | 1.9.7 notes |
| Forward sync editor→PDF | ⌘-click in editor | docs |
| Inverse sync PDF→editor | ⌘-click in PDF (or single click if pref) | docs |
| Sync PDF to editor position | ⌘D | iOS docs say "compatible with macOS" (unconfirmed on Mac) |
| Toggle sidebar/outline | ⌘1 | docs |
| Editor only / split / PDF only | ⌘2 / ⌘3 / ⌘4 | docs |
| Switch focus editor↔PDF | ⌘J | cheatsheet (unconfirmed in 1.9.x) |
| Show PDF / show log | ⌘8 / ⌘9 | docs (meaning in 1.9.x unconfirmed) |
| PDF zoom in / out | ⌘+ (also ⌘=, ⇧⌘=) / ⌘− | docs |
| Welcome/Organiser window | ⌃⌘T | docs |
| Snippets | ⌃⇧ + letter (base configurable) | screenshot |
| Search results navigation | ↑ / ↓, Return jumps, Esc closes | docs |

Retired (pre-1.7) and listed only on the stale cheatsheet: ⌘L (auto engine), ⌘B (+BibTeX), ⌘I
(+MakeIndex), ⇧⌘I (+BibTeX+MakeIndex). Do not replicate as distinct actions.

---

## 9. Other features

### Organiser / welcome window (⌃⌘T; shown at launch by default)
- Two panes. **Left**: app icon, version string, segmented **Documents | Account | Licence**.
  Documents tab: "+ Open a local file or project…", "Template [popup, default Blank LaTeX Document]",
  "+ Create a local file…", and the 3 most recent documents (in some versions).
  Account: sign in to texifier.com (Connect, licence activation). Licence: key entry (direct builds only).
- **Right**: segmented **Recent Documents | File Accessibility | Connect Projects**. Recent rows: file
  icon, name, "Last Modified on <date>". File Accessibility (App Store/sandboxed build): manage folders
  the app may open. Connect Projects: plan summary, list with Open / Open in texifier.com / delete.
  Bottom Filter field (+ sort popup "Newest First" for Connect).

### Templates
- File → New from Template ▸ and the Organiser template popup; user templates are plain files in
  `~/Library/Application Support/Texifier/Templates/`. Default "Blank LaTeX Document"; other names unknown.

### Project management
- Open root → whole project (subfiles, images, `.bib`, `.sty`) discovered automatically.
- Typing `\include{new.tex}` for a missing file offers to create it.
- File Browser `+` creates files/folders. Duplicate entire project, Move To…, Rename…, Revert To ▸ (versions).
- Emergency snapshots in `~/Library/Application Support/Texifier/Editor-Archive/`.
- Intermediate files hidden in `.texpadtmp` (pref); exportable via "Export .zip including intermediate files".

### Sync / storage
- Files are ordinary files anywhere on disk (iCloud Drive / Dropbox folders work as normal folders; a
  troubleshooting page exists for cloud-storage issues). No built-in iCloud/Dropbox/Git integration on
  macOS (Git: not mentioned anywhere → unknown/none).
- **Texifier Connect**: proprietary cloud projects with collaborators, live co-editing; free plan plus
  paid plans (prices not public in docs).

### Export
- Export PDF, Email PDF, Email/Export project as Zip, Export as folder, Zip with intermediates,
  Print source, Print PDF.

### Preferences window (toolbar-tab style, titles under icons)
1. **General** — Suppress localisation; Show Organiser window by default ✓; Power save policy
   ("Optimise power usage when on battery"); Toolbar Style ("Icons Only" …); Troubleshooting: Restore
   factory settings, Reset workspace cache, Backup settings to a file, Restore settings from file.
2. **Sidebar** — contents unknown (older docs: hide outline on open, suppress `.def` files — unconfirmed).
3. **Editor** — sub-tabs **Editing Aids** (see §4) and **Editor Look & Feel** (Theme popup "Texpad Default";
   current font "Source Code Pro (12 pt)"; font popup + size; "Choose a custom font…"; live preview box).
4. **Document Viewer** — Single-click PDF sync with editor; Image Grading in Dark Mode.
5. **Typesetting** — sub-tabs **General Preferences** (inner tabs Behaviours / Typeset Issues / New
   Projects), **TexpadTeX Typesetter**, **External Typesetter** (distribution list, custom distribution,
   health check — location inside this tab unconfirmed), **Shared Files**.
   Behaviours: Post-typeset pane behaviour, Animate controls for auto-typeset, Ordering of Indices
   (Word Ordering / letter), Shell Escape security policy for Autosense, Post OS notifications.
   Typeset Issues: Show Error Pane Post Typeset; Typeset Error Detail Level.
   New Projects: (unknown; guess default auto-typeset / default typesetter).

### Localisation
English, Catalan, German, Italian, Japanese, Portuguese, Russian, Simplified Chinese.

### Pricing (no free tier; trial instead)
- Direct (texifier.com): one-time licence, **US$39.99** single seat (€34.99 / £34.99); volume tiers
  2–10 $34.99, 11–100 $31.99, 101–1000 $29.99 per seat. 14-day **full-featured trial** (macOS/Windows).
  No separate student price (volume discounts instead).
- Mac App Store: **US$44.99** one-time, no in-app purchases, sandboxed (cloud typesetter available).
- iOS and Windows are separate purchases; Connect has free + paid subscription plans.

---

## 10. Sources

- https://www.texifier.com/mac · https://www.texifier.com/purchase · https://www.texifier.com/docs (index)
- https://texifier.com/docs/apps/getting-started/ quick-start, cheatsheet, obtaining/trial, obtaining/educational-discount
- https://texifier.com/docs/apps/workspace/ one-window-design, toolbar, tabs, menu-items, external-keyboard (iOS),
  multi-file-documents, non-standard-includes, sharing-projects, editor-document-backups
- https://texifier.com/docs/apps/workspace/sidebar and sub-pages: contents, contents/metadata,
  contents/structure, contents/project-files, todos, file-browser, typeset-results, typeset-results/issues-list,
  typeset-results/log, search/text-search, search/tag-search
- https://texifier.com/docs/apps/workspace/editor/ syntax-highlighting, autocomplete, aids, info-boxes,
  snippets, symbol-table, toolbar, math-expressions-autocompletion
- https://texifier.com/docs/apps/workspace/pdf-viewer · …/pdf-viewer/dark-mode · …/pdf-viewer/syncing-with-editor
- https://texifier.com/docs/apps/typesetting/configuration · …/typesetting/typesetters · …/typesetters/external
- https://texifier.com/docs/apps/typesetting/typesetters/external/managing-external-typesetters · …/custom-typesetting
- https://texifier.com/docs/apps/typesetting/issues · …/bibliographies/compiling
- https://texifier.com/docs/tutorials/tex/typesetters/external/cloud
- https://texifier.com/docs/apps/settings/preferences
- https://texifier.com/docs/apps/organiser/macos · https://texifier.com/docs/apps/managing-documents/macos
- https://texifier.com/docs/apps/misc/faq
- https://texifier.com/docs/apps/release-notes/macos and per-version pages 1.5.x–1.9.33
  (key ones: 1.7.0, 1.7.29, 1.7.42, 1.8.0, 1.8.8, 1.9.0, 1.9.7, 1.9.13, 1.9.28, 1.9.33)
- https://apps.apple.com/us/app/texifier-latex-editor/id458866234 (Mac App Store listing)
- Official doc screenshots under https://www.texifier.com/static/media/showcase/docs/apps/… (viewed, not
  downloaded): workspace/workspace.png, workspace/toolbar/toolbar_macos.png, workspace/tabs_macos.png,
  jd-thesis-sawtooth-dark-mode_macos.png, sidebar/* (contents, project files, todos, search, file browser,
  issues list, log, structure filter menu, labels, figures/tables), editor/* (autocomplete, find bar,
  symbols table), document-viewer/* (toolbar, scroll lock, grading menus), typesetting/config/*,
  settings/* (general, editor aids, editor look & feel, typesetting, error level), menus/* (snippets,
  encodings, share), printing/macos/file-menu-with-print-pdf-highlighted.png, organiser/macos/*.
