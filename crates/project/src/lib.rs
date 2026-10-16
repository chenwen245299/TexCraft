//! The TexCraft project model: which TeX engine to run, settings, the root (main) file of a
//! project, the project's file tree and the new-document templates.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use texcraft_syntax::{is_root_document, magic_comments, outline::OutlineKind};

pub mod templates;

/// A TeX engine TexCraft can typeset with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Engine {
    /// pdfLaTeX through latexmk.
    #[default]
    PdfLatex,
    /// XeLaTeX through latexmk (Unicode, system fonts, CJK).
    XeLatex,
    /// LuaLaTeX through latexmk.
    LuaLatex,
    /// Tectonic (self-contained, fetches packages on demand).
    Tectonic,
}

impl Engine {
    /// Every engine, in menu order.
    pub const ALL: [Engine; 4] = [
        Self::PdfLatex,
        Self::XeLatex,
        Self::LuaLatex,
        Self::Tectonic,
    ];

    /// User-facing name.
    pub fn label(self) -> &'static str {
        match self {
            Self::PdfLatex => "pdfLaTeX",
            Self::XeLatex => "XeLaTeX",
            Self::LuaLatex => "LuaLaTeX",
            Self::Tectonic => "Tectonic",
        }
    }

    /// Stable id used in commands and settings files.
    pub fn id(self) -> &'static str {
        match self {
            Self::PdfLatex => "pdflatex",
            Self::XeLatex => "xelatex",
            Self::LuaLatex => "lualatex",
            Self::Tectonic => "tectonic",
        }
    }

    /// Parses an id or a `% !TEX program` value (`xelatex`, `XeLaTeX`, `latexmk -xelatex`, ...).
    pub fn from_program(name: &str) -> Option<Self> {
        let n = name.trim().to_ascii_lowercase();
        if n.contains("tectonic") {
            Some(Self::Tectonic)
        } else if n.contains("xelatex") || n.contains("xetex") {
            Some(Self::XeLatex)
        } else if n.contains("lualatex") || n.contains("luatex") {
            Some(Self::LuaLatex)
        } else if n.contains("pdflatex") || n == "latex" || n.contains("pdftex") || n == "latexmk" {
            Some(Self::PdfLatex)
        } else {
            None
        }
    }
}

/// Which bibliography processor latexmk should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BibTool {
    /// Let latexmk decide (BibTeX or biber from the `.aux`/`.bcf`).
    #[default]
    Auto,
    /// Never run a bibliography tool.
    None,
}

/// Which typesetter runs while you type (the Typeset Configuration sheet's two columns).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Typesetter {
    /// Live preview: one fast engine pass over the unsaved text after every change.
    #[default]
    Live,
    /// External: full latexmk/tectonic runs (with bibliography and reruns); with Auto-Typeset
    /// they start when typing pauses.
    External,
}

/// When typesetting happens (the toolbar's LIVE / AUTO / MANUAL label). A typeset started by
/// hand (⌘T) is always a full one that writes the PDF, whatever the mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypesetMode {
    /// A live-preview pass over the unsaved text after every change.
    Live,
    /// A full typeset when typing pauses.
    Auto,
    /// Only when asked (⌘T).
    Manual,
}

impl TypesetMode {
    /// All modes, in menu order.
    pub const ALL: [TypesetMode; 3] = [Self::Live, Self::Auto, Self::Manual];

    /// The mode a click on the toolbar label switches to: LIVE → AUTO → MANUAL → LIVE.
    pub fn next(self) -> Self {
        match self {
            Self::Live => Self::Auto,
            Self::Auto => Self::Manual,
            Self::Manual => Self::Live,
        }
    }

    /// The toolbar label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Live => "LIVE",
            Self::Auto => "AUTO",
            Self::Manual => "MANUAL",
        }
    }

    /// Stable lowercase name (command ids, the control channel).
    pub fn name(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Auto => "auto",
            Self::Manual => "manual",
        }
    }
}

/// Per-project (and default) typesetting settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Live preview or external typesetter.
    pub typesetter: Typesetter,
    /// Pick the engine from the document (fontspec/ctex → XeLaTeX, …) when it has no
    /// `% !TEX program` comment.
    pub auto_sense: bool,
    /// Engine when the root file has no `% !TEX program` comment (and Auto-Sense is off or
    /// finds nothing).
    pub engine: Engine,
    /// Typeset automatically shortly after typing stops.
    pub live_typeset: bool,
    /// Pause before a live typeset, in milliseconds.
    pub live_delay_ms: u32,
    /// Allow `\write18` (minted, gnuplot, ...). Off by default for safety.
    pub shell_escape: bool,
    /// Bibliography tool policy.
    pub bib_tool: BibTool,
    /// Save edited files before a full typeset (live previews never save).
    pub save_before_typeset: bool,
    /// Live preview keeps showing the last good PDF while the document has errors.
    pub live_keep_good: bool,
    /// Precompile the preamble for live previews (pdfLaTeX).
    pub cache_preamble: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            typesetter: Typesetter::Live,
            auto_sense: true,
            engine: Engine::PdfLatex,
            live_typeset: true,
            live_delay_ms: 0,
            shell_escape: false,
            bib_tool: BibTool::Auto,
            save_before_typeset: true,
            live_keep_good: true,
            cache_preamble: true,
        }
    }
}

impl Settings {
    /// The typesetting mode these settings amount to.
    pub fn mode(&self) -> TypesetMode {
        match (self.live_typeset, self.typesetter) {
            (false, _) => TypesetMode::Manual,
            (true, Typesetter::Live) => TypesetMode::Live,
            (true, Typesetter::External) => TypesetMode::Auto,
        }
    }

    /// Switches to `mode`. Manual keeps the typesetter, so turning automatic typesetting back
    /// on (⇧⌘L) returns to the previous mode.
    pub fn set_mode(&mut self, mode: TypesetMode) {
        match mode {
            TypesetMode::Live => {
                self.typesetter = Typesetter::Live;
                self.live_typeset = true;
            }
            TypesetMode::Auto => {
                self.typesetter = Typesetter::External;
                self.live_typeset = true;
            }
            TypesetMode::Manual => self.live_typeset = false,
        }
    }
}

/// The engine to use for `root_text`: its `% !TEX program` comment, else `default`.
pub fn engine_for(root_text: &str, default: Engine) -> Engine {
    magic_comments(root_text)
        .program
        .as_deref()
        .and_then(Engine::from_program)
        .unwrap_or(default)
}

/// The preamble of `text` (everything before `\begin{document}`; packages can only be loaded
/// there) without `%` comments (escaped `\%` is rare enough in preambles to ignore).
fn code_of(text: &str) -> String {
    let preamble = text
        .find("\\begin{document}")
        .and_then(|i| text.get(..i))
        .unwrap_or(text);
    preamble
        .lines()
        .map(|l| l.split('%').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether `code` loads `name` as a package or class (`{name}` or inside a `{a,name,b}` list).
fn loads(code: &str, name: &str) -> bool {
    code.contains(&format!("{{{name}}}"))
        || code.contains(&format!(",{name}}}"))
        || code.contains(&format!("{{{name},"))
        || code.contains(&format!(",{name},"))
}

/// A Chinese, Japanese or Korean character (ideographs, kana, hangul, CJK punctuation and
/// full-width forms).
pub fn is_cjk(c: char) -> bool {
    matches!(
        u32::from(c),
        0x3000..=0x303F
            | 0x3040..=0x30FF
            | 0x3100..=0x312F
            | 0x3130..=0x318F
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF
            | 0x20000..=0x2FA1F
    )
}

/// Whether `text` has CJK characters outside comments.
pub fn has_cjk(text: &str) -> bool {
    text.lines()
        .any(|l| l.split('%').next().unwrap_or("").chars().any(is_cjk))
}

/// Whether the document sets up CJK typesetting itself (ctex, xeCJK, CJK, luatexja, …), so
/// TexCraft must not add its own.
pub fn has_cjk_setup(root_text: &str) -> bool {
    let code = code_of(root_text);
    [
        "ctex",
        "ctexart",
        "ctexrep",
        "ctexbook",
        "ctexbeamer",
        "xeCJK",
        "CJK",
        "CJKutf8",
        "luatexja",
        "luatexja-fontspec",
        "ltjarticle",
        "ltjsarticle",
        "ltjbook",
        "zhspacing",
        "kotex",
        "bxjsarticle",
        "jarticle",
        "ujarticle",
    ]
    .iter()
    .any(|p| loads(&code, p))
}

/// The engine a document needs, judged from the packages it loads and the text it contains
/// (Auto-Sense): Unicode-font and CJK packages, or CJK text without its own setup, need
/// XeLaTeX; Lua packages LuaLaTeX. `None` when any engine will do.
pub fn sense_engine(root_text: &str) -> Option<Engine> {
    let code = code_of(root_text);
    let has = |name: &str| loads(&code, name);
    if [
        "luacode",
        "luatexja",
        "luatexja-fontspec",
        "luaotfload",
        "lualatex-math",
    ]
    .iter()
    .any(|p| has(p))
    {
        return Some(Engine::LuaLatex);
    }
    if [
        "fontspec",
        "xeCJK",
        "ctex",
        "ctexart",
        "ctexrep",
        "ctexbook",
        "ctexbeamer",
        "polyglossia",
        "unicode-math",
        "xltxtra",
        "xunicode",
    ]
    .iter()
    .any(|p| has(p))
    {
        return Some(Engine::XeLatex);
    }
    // CJK text: pdfLaTeX cannot typeset it unless the CJK package is set up for it.
    if has_cjk(root_text) && !has("CJK") && !has("CJKutf8") {
        return Some(Engine::XeLatex);
    }
    None
}

/// Where the engine of a [`Plan`] comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineSource {
    /// A `% !TEX program` comment.
    MagicComment,
    /// Auto-Sense read the document.
    AutoSense,
    /// The engine chosen in the settings.
    Setting,
}

/// How a document will be typeset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// The engine.
    pub engine: Engine,
    /// Why this engine.
    pub source: EngineSource,
    /// The document has CJK text but no CJK setup: TexCraft loads xeCJK with system fonts
    /// (Auto-Sense, XeLaTeX only).
    pub add_cjk: bool,
}

/// The typeset plan for `root_text` under `settings`. `cjk_elsewhere`: another file of the
/// project has CJK text.
pub fn plan(root_text: &str, cjk_elsewhere: bool, settings: &Settings) -> Plan {
    let (engine, source) = if let Some(e) = magic_comments(root_text)
        .program
        .as_deref()
        .and_then(Engine::from_program)
    {
        (e, EngineSource::MagicComment)
    } else if let Some(e) = settings
        .auto_sense
        .then(|| sense_engine(root_text))
        .flatten()
        .or_else(|| {
            (settings.auto_sense && cjk_elsewhere && !has_cjk_setup(root_text))
                .then_some(Engine::XeLatex)
        })
    {
        (e, EngineSource::AutoSense)
    } else {
        (settings.engine, EngineSource::Setting)
    };
    let cjk = cjk_elsewhere || has_cjk(root_text);
    Plan {
        engine,
        source,
        add_cjk: settings.auto_sense
            && engine == Engine::XeLatex
            && cjk
            && !has_cjk_setup(root_text),
    }
}

/// The engine for `root_text` under `settings`: magic comment, then Auto-Sense, then the
/// chosen engine.
pub fn effective_engine(root_text: &str, settings: &Settings) -> Engine {
    plan(root_text, false, settings).engine
}

/// Removes `.` and resolves `..` components lexically (no file-system access).
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Finds the root (main) file for `file`, whose text is `text`.
///
/// 1. A `% !TEX root = ...` comment wins (relative to `file`'s directory).
/// 2. A file with `\documentclass` is its own root.
/// 3. Otherwise a `.tex` file in the same directory or its parent that has `\documentclass` and
///    includes this file (`\input`, `\include`, `\subfile`).
/// 4. Otherwise the file itself.
///
/// `read` loads another file's text (so this is testable without a file system).
pub fn find_root(file: &Path, text: &str, read: &dyn Fn(&Path) -> Option<String>) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new(""));
    if let Some(root) = magic_comments(text).root {
        return normalize(&dir.join(root));
    }
    if is_root_document(text) {
        return file.to_path_buf();
    }
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let candidates_dirs = [Some(dir), dir.parent()];
    for cand_dir in candidates_dirs.into_iter().flatten() {
        let Ok(entries) = std::fs::read_dir(if cand_dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            cand_dir
        }) else {
            continue;
        };
        let mut names: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "tex") && p != file)
            .collect();
        names.sort();
        for cand in names {
            let Some(ctext) = read(&cand) else { continue };
            if !is_root_document(&ctext) {
                continue;
            }
            let includes_us = texcraft_syntax::outline(&ctext).iter().any(|i| {
                i.kind == OutlineKind::Input && {
                    let target = normalize(&cand_dir.join(&i.title));
                    let target_no_ext = target.with_extension("");
                    target == normalize(file)
                        || target_no_ext == normalize(&file.with_extension(""))
                        || Path::new(&i.title).file_stem().and_then(|s| s.to_str()) == Some(stem)
                }
            });
            if includes_us {
                return cand;
            }
        }
    }
    file.to_path_buf()
}

/// The document that stands for `file` in the recent-documents list: its project's entry point,
/// so a project is listed once, by its main file. A chapter, a `.bib` or a figure opened from
/// the file list stands for its root ([`find_root`]); a file nothing includes stands for the
/// root document of its folder or the folder above (`main.tex` among several, else the only
/// one); a file with none of these, or a root itself, stands for itself.
pub fn project_entry(file: &Path, read: &dyn Fn(&Path) -> Option<String>) -> PathBuf {
    let is_tex = file
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("tex"));
    if is_tex {
        let text = read(file).unwrap_or_default();
        let root = find_root(file, &text, read);
        if root != file || is_root_document(&text) {
            return root;
        }
    }
    let dir = file.parent().unwrap_or(Path::new(""));
    for cand_dir in [Some(dir), dir.parent()].into_iter().flatten() {
        let Ok(entries) = std::fs::read_dir(cand_dir) else {
            continue;
        };
        let mut roots: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "tex") && p != file)
            .filter(|p| read(p).is_some_and(|t| is_root_document(&t)))
            .collect();
        roots.sort();
        if let Some(main) = roots
            .iter()
            .find(|p| p.file_stem().is_some_and(|s| s == "main"))
        {
            return main.clone();
        }
        if let [only] = roots.as_slice() {
            return only.clone();
        }
        if !roots.is_empty() {
            break;
        }
    }
    file.to_path_buf()
}

/// Where TexCraft writes the auxiliary and output files for `root`: `<dir>/.texcraft/build`.
pub fn build_dir(root: &Path) -> PathBuf {
    root.parent()
        .unwrap_or(Path::new("."))
        .join(TEMP_FOLDER)
        .join("build")
}

/// The PDF beside the root (`main.tex` → `main.pdf`): every full typeset puts its result there,
/// as Texifier does, while the build folder keeps the working copy the viewer and SyncTeX use.
pub fn published_pdf(root: &Path) -> PathBuf {
    root.with_extension("pdf")
}

/// The PDF that typesetting `root` produces.
pub fn output_pdf(root: &Path) -> PathBuf {
    let stem = root.file_stem().unwrap_or_default();
    build_dir(root).join(stem).with_extension("pdf")
}

/// What a file is, for icons and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// A directory.
    Folder,
    /// `.tex`, `.ltx`, `.sty`, `.cls`, `.dtx`
    Tex,
    /// `.bib`
    Bib,
    /// Images TeX can include.
    Image,
    /// `.pdf`
    Pdf,
    /// Plain text-like files (`.txt`, `.md`, `.csv`, `.dat`, ...).
    Text,
    /// Anything else.
    Other,
}

impl FileKind {
    /// Classifies a path by extension.
    pub fn of(path: &Path) -> Self {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        match ext.as_str() {
            "tex" | "ltx" | "sty" | "cls" | "dtx" | "ins" | "latex" => Self::Tex,
            "bib" => Self::Bib,
            "png" | "jpg" | "jpeg" | "eps" | "svg" | "gif" | "tif" | "tiff" => Self::Image,
            "pdf" => Self::Pdf,
            "txt" | "md" | "csv" | "dat" | "tsv" | "json" | "bst" | "cfg" | "def" | "tikz"
            | "lua" | "py" => Self::Text,
            _ => Self::Other,
        }
    }

    /// Whether TexCraft opens it in the text editor.
    pub fn is_editable(self) -> bool {
        matches!(self, Self::Tex | Self::Bib | Self::Text)
    }
}

/// Auxiliary files TeX and friends write; hidden from the file list.
pub fn is_aux_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    const AUX: &[&str] = &[
        ".aux",
        ".log",
        ".synctex.gz",
        ".synctex",
        ".fdb_latexmk",
        ".fls",
        ".out",
        ".toc",
        ".lof",
        ".lot",
        ".bbl",
        ".blg",
        ".bcf",
        ".run.xml",
        ".nav",
        ".snm",
        ".vrb",
        ".idx",
        ".ind",
        ".ilg",
        ".xdv",
        ".dvi",
        ".glo",
        ".gls",
        ".glg",
        ".ist",
        ".acn",
        ".acr",
        ".alg",
        ".thm",
        ".loa",
        ".lol",
        ".synctex(busy)",
        ".auxlock",
    ];
    AUX.iter().any(|ext| name.ends_with(ext))
}

/// A node of the project file tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileNode {
    /// File name.
    pub name: String,
    /// Full path.
    pub path: PathBuf,
    /// Kind.
    pub kind: FileKind,
    /// Children (folders only), folders first then files, alphabetical.
    pub children: Vec<FileNode>,
}

/// Lists `dir` recursively (up to `max_depth` levels and 5,000 entries), hiding dot-files and
/// auxiliary files. TexCraft's own folder (`.texcraft`, the target's `.texpadtmp`) comes first
/// when it exists, listed in full: the Files sidebar marks it `TEMP`.
pub fn file_tree(dir: &Path, max_depth: usize) -> Vec<FileNode> {
    let mut budget = 5000usize;
    let mut nodes = list(dir, max_depth, &mut budget, false);
    let temp = dir.join(TEMP_FOLDER);
    if temp.is_dir() {
        let children = list(&temp, max_depth.saturating_sub(1), &mut budget, true);
        nodes.insert(
            0,
            FileNode {
                name: TEMP_FOLDER.to_string(),
                path: temp,
                kind: FileKind::Folder,
                children,
            },
        );
    }
    nodes
}

/// TexCraft's folder for intermediate files, inside the project folder.
pub const TEMP_FOLDER: &str = ".texcraft";

/// Keeps `temp` (a [`TEMP_FOLDER`]) out of version control: writes a `.gitignore` that ignores
/// everything in it, unless one is there. Errors are ignored (it is a courtesy).
pub fn ignore_temp_folder(temp: &Path) {
    let file = temp.join(".gitignore");
    if temp.is_dir() && !file.exists() {
        let _ = std::fs::write(
            &file,
            "# TexCraft's intermediate files (build, live preview): not for version control.\n*\n",
        );
    }
}

/// Lists `dir`; `everything` keeps dot-files and auxiliary files.
fn list(dir: &Path, depth: usize, budget: &mut usize, everything: bool) -> Vec<FileNode> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut nodes: Vec<FileNode> = Vec::new();
    for e in entries.filter_map(Result::ok) {
        if *budget == 0 {
            break;
        }
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if !everything && (name.starts_with('.') || is_aux_file(&path)) {
            continue;
        }
        *budget -= 1;
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        let (kind, children) = if is_dir {
            if matches!(name.as_str(), "node_modules" | "target" | "_minted") {
                continue;
            }
            let children = if depth > 0 {
                list(&path, depth - 1, budget, everything)
            } else {
                Vec::new()
            };
            (FileKind::Folder, children)
        } else {
            (FileKind::of(&path), Vec::new())
        };
        nodes.push(FileNode {
            name,
            path,
            kind,
            children,
        });
    }
    nodes.sort_by(|a, b| {
        (a.kind != FileKind::Folder)
            .cmp(&(b.kind != FileKind::Folder))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engines_from_magic() {
        assert_eq!(
            engine_for("% !TEX program = XeLaTeX\n", Engine::PdfLatex),
            Engine::XeLatex
        );
        assert_eq!(
            engine_for("% !TEX TS-program = lualatex", Engine::PdfLatex),
            Engine::LuaLatex
        );
        assert_eq!(
            engine_for("\\documentclass{x}", Engine::Tectonic),
            Engine::Tectonic
        );
        assert_eq!(
            Engine::from_program("latexmk -xelatex"),
            Some(Engine::XeLatex)
        );
        assert_eq!(Engine::from_program("context"), None);
    }

    #[test]
    fn typeset_modes_round_trip_and_manual_remembers_the_typesetter() {
        let mut s = Settings::default();
        assert_eq!(s.mode(), TypesetMode::Live);
        for m in TypesetMode::ALL {
            s.set_mode(m);
            assert_eq!(s.mode(), m);
        }
        s.set_mode(TypesetMode::Auto);
        s.set_mode(TypesetMode::Manual);
        s.live_typeset = true; // ⇧⌘L turns automatic typesetting back on
        assert_eq!(s.mode(), TypesetMode::Auto);
        assert_eq!(TypesetMode::Manual.label(), "MANUAL");
        assert_eq!(TypesetMode::Live.next().next().next(), TypesetMode::Live);
    }

    #[test]
    fn cjk_text_plans_xelatex_with_cjk_setup() {
        let s = Settings::default();
        let doc = "\\documentclass{article}\n\\usepackage[utf8]{inputenc}\n\\begin{document}\n这是一个测试\n\\end{document}";
        assert!(has_cjk(doc));
        assert!(!has_cjk("% 注释里的中文\n\\documentclass{article}"));
        let p = plan(doc, false, &s);
        assert_eq!(
            (p.engine, p.source, p.add_cjk),
            (Engine::XeLatex, EngineSource::AutoSense, true)
        );
        // The document sets CJK up itself: nothing added.
        let ctex = "\\documentclass{ctexart}\n\\begin{document}\n中文\n\\end{document}";
        assert_eq!(plan(ctex, false, &s).engine, Engine::XeLatex);
        assert!(!plan(ctex, false, &s).add_cjk);
        // CJKutf8 works with pdfLaTeX.
        let cjk = "\\usepackage{CJKutf8}\n\\begin{CJK}{UTF8}{gbsn}中文\\end{CJK}";
        assert_eq!(plan(cjk, false, &s).engine, Engine::PdfLatex);
        // CJK only in an \input file.
        let root = "\\documentclass{article}\n\\begin{document}\\input{ch1}\\end{document}";
        let p = plan(root, true, &s);
        assert_eq!((p.engine, p.add_cjk), (Engine::XeLatex, true));
        // Auto-Sense off: the chosen engine, nothing added.
        let manual = Settings {
            auto_sense: false,
            ..Settings::default()
        };
        let p = plan(doc, false, &manual);
        assert_eq!(
            (p.engine, p.source, p.add_cjk),
            (Engine::PdfLatex, EngineSource::Setting, false)
        );
        // Magic comment wins; CJK setup still added for XeLaTeX.
        let p = plan(&format!("% !TEX program = xelatex\n{doc}"), false, &s);
        assert_eq!(
            (p.engine, p.source, p.add_cjk),
            (Engine::XeLatex, EngineSource::MagicComment, true)
        );
        assert!(is_cjk('あ') && is_cjk('한') && is_cjk('，') && !is_cjk('é'));
    }

    #[test]
    fn auto_sense_engines() {
        assert_eq!(
            sense_engine("\\documentclass[UTF8]{ctexart}"),
            Some(Engine::XeLatex)
        );
        assert_eq!(
            sense_engine("\\usepackage{amsmath,fontspec}"),
            Some(Engine::XeLatex)
        );
        assert_eq!(
            sense_engine("\\usepackage{luacode}"),
            Some(Engine::LuaLatex)
        );
        assert_eq!(
            sense_engine("% \\usepackage{fontspec}\n\\documentclass{article}"),
            None
        );
        let mut s = Settings::default();
        assert_eq!(
            effective_engine("\\usepackage{fontspec}", &s),
            Engine::XeLatex
        );
        s.auto_sense = false;
        assert_eq!(
            effective_engine("\\usepackage{fontspec}", &s),
            Engine::PdfLatex
        );
        assert_eq!(
            effective_engine("% !TEX program = lualatex\n\\usepackage{fontspec}", &s),
            Engine::LuaLatex
        );
    }

    #[test]
    fn normalizes_paths() {
        assert_eq!(
            normalize(Path::new("/a/./b/../c.tex")),
            PathBuf::from("/a/c.tex")
        );
        assert_eq!(normalize(Path::new("../x")), PathBuf::from("../x"));
    }

    #[test]
    fn root_from_magic_and_documentclass() {
        let none = |_: &Path| None;
        assert_eq!(
            find_root(
                Path::new("/p/ch/one.tex"),
                "% !TEX root = ../main.tex\n\\section{A}",
                &none
            ),
            PathBuf::from("/p/main.tex")
        );
        assert_eq!(
            find_root(Path::new("/p/main.tex"), "\\documentclass{article}", &none),
            PathBuf::from("/p/main.tex")
        );
    }

    #[test]
    fn a_project_is_remembered_by_its_main_file() {
        let dir = std::env::temp_dir().join(format!("texcraft-entry-{}", std::process::id()));
        let ch = dir.join("sections");
        std::fs::create_dir_all(&ch).unwrap();
        let main = dir.join("main.tex");
        std::fs::write(
            &main,
            "\\documentclass{article}\\begin{document}\\input{sections/intro}\\end{document}",
        )
        .unwrap();
        std::fs::write(ch.join("intro.tex"), "\\section{Intro}").unwrap();
        std::fs::write(ch.join("unused.tex"), "\\section{Unused}").unwrap();
        std::fs::write(dir.join("refs.bib"), "@book{k, title={T}}").unwrap();
        std::fs::write(ch.join("plot.png"), "").unwrap();
        let read = |p: &Path| std::fs::read_to_string(p).ok();
        for f in [
            main.clone(),
            ch.join("intro.tex"),
            ch.join("unused.tex"),
            dir.join("refs.bib"),
            ch.join("plot.png"),
        ] {
            assert_eq!(project_entry(&f, &read), main, "{}", f.display());
        }
        // With several roots and no main.tex a stray file stands for itself; each root for
        // itself.
        std::fs::remove_file(&main).unwrap();
        std::fs::write(dir.join("a.tex"), "\\documentclass{article}").unwrap();
        std::fs::write(dir.join("b.tex"), "\\documentclass{article}").unwrap();
        assert_eq!(
            project_entry(&dir.join("refs.bib"), &read),
            dir.join("refs.bib")
        );
        assert_eq!(project_entry(&dir.join("b.tex"), &read), dir.join("b.tex"));
        // A lone file outside any project stands for itself.
        let lone = std::env::temp_dir().join(format!("texcraft-lone-{}", std::process::id()));
        let notes = lone.join("a/notes.bib");
        std::fs::create_dir_all(lone.join("a")).unwrap();
        std::fs::write(&notes, "").unwrap();
        assert_eq!(project_entry(&notes, &read), notes);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&lone).ok();
    }

    #[test]
    fn root_by_inclusion_on_disk() {
        let dir = std::env::temp_dir().join(format!("texcraft-root-{}", std::process::id()));
        let ch = dir.join("chapters");
        std::fs::create_dir_all(&ch).unwrap();
        std::fs::write(
            dir.join("main.tex"),
            "\\documentclass{book}\\begin{document}\\include{chapters/intro}\\end{document}",
        )
        .unwrap();
        std::fs::write(dir.join("other.tex"), "\\documentclass{article}").unwrap();
        let intro = ch.join("intro.tex");
        std::fs::write(&intro, "\\chapter{Intro}").unwrap();
        let read = |p: &Path| std::fs::read_to_string(p).ok();
        assert_eq!(
            find_root(&intro, "\\chapter{Intro}", &read),
            dir.join("main.tex")
        );
        let tree = file_tree(&dir, 4);
        let names: Vec<&str> = tree.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["chapters", "main.tex", "other.tex"]);
        assert_eq!(tree[0].children[0].name, "intro.tex");
        // The intermediate-files folder comes first, with everything in it.
        std::fs::create_dir_all(dir.join(".texcraft/build")).unwrap();
        std::fs::write(dir.join(".texcraft/build/main.aux"), "").unwrap();
        let tree = file_tree(&dir, 4);
        assert_eq!(tree[0].name, ".texcraft");
        assert_eq!(tree[0].children[0].children[0].name, "main.aux");
        assert_eq!(tree[1].name, "chapters");
        ignore_temp_folder(&dir.join(".texcraft"));
        let ignore = std::fs::read_to_string(dir.join(".texcraft/.gitignore")).unwrap();
        assert!(ignore.ends_with("*\n"), "{ignore}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn aux_files_and_kinds() {
        assert!(is_aux_file(Path::new("a.synctex.gz")));
        assert!(is_aux_file(Path::new("thesis.fdb_latexmk")));
        assert!(!is_aux_file(Path::new("main.tex")));
        assert_eq!(FileKind::of(Path::new("x.BIB")), FileKind::Bib);
        assert_eq!(FileKind::of(Path::new("fig.png")), FileKind::Image);
        assert_eq!(
            output_pdf(Path::new("/p/main.tex")),
            PathBuf::from("/p/.texcraft/build/main.pdf")
        );
    }
}
