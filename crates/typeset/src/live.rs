//! Live preview typesetting: typesets the editor's **unsaved** buffers, fast.
//!
//! * The open buffers are mirrored into `<project>/.texcraft/live/` (same relative paths, same
//!   folder structure); every other project file is found through `TEXINPUTS`, so nothing is
//!   saved and the user's files are never touched.
//! * One engine pass, no latexmk: references come from the previous pass's `.aux`, the
//!   bibliography from the last full typeset's `.bbl`.
//! * For pdfLaTeX and XeLaTeX the preamble is precompiled into a format (`mylatexformat`) and
//!   reused while it does not change, so each update only typesets the document body. XeTeX
//!   cannot store system fonts in a format, so its dump reads the preamble with TFM fonts and the
//!   CJK fonts are set at the start of each pass ([`xe_format_source`]).
//! * XeLaTeX's PDF step (xdvipdfmx) spends ~0.15 s reading the full font maps on every pass: a
//!   local `dvipdfmx.cfg` points it at a map with just the families the document uses.
//! * pdfTeX spends ~0.2 s parsing the full 38,000-line `pdftex.map` on every run. After a pass
//!   with the full map, a `pdftex.map` with just the families the document uses (plus the common
//!   maths families) is written into the live folder, where pdfTeX finds it first. Bitmap-font
//!   generation is disabled for those passes; if a font is missing the pass fails fast and is
//!   rerun with the full map, which then refreshes the reduced one.
//! * pdfTeX also spends ~110 ms reading TeX Live's `texmf.cnf` (variable expansion, `ls-R`) and
//!   ~50 ms inflating a gzipped format before typesetting anything. The format is stored
//!   uncompressed, and after a normal pass (run with `-recorder`) the system files it opened are
//!   linked into `.texcraft-fast/farm` next to a minimal `texmf.cnf` that searches only the live
//!   folder, the project and that farm. Fast passes use it (`TEXMFCNF`); if one cannot find
//!   something a normal pass can, the pass is rerun normally and the farm rebuilt.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use texcraft_project::Engine;

/// What a live typeset needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRequest {
    /// The folder of the root file: relative paths are relative to it.
    pub project_dir: PathBuf,
    /// The root file's path relative to `project_dir`.
    pub root_rel: PathBuf,
    /// Current text of every open buffer inside the project: (relative path, text).
    pub buffers: Vec<(PathBuf, String)>,
    /// Folders (relative to the project) that hold project files: recreated in the mirror so
    /// `\include` can write its `.aux` files there.
    pub dirs: Vec<PathBuf>,
    /// Engine.
    pub engine: Engine,
    /// Allow `\write18`.
    pub shell_escape: bool,
    /// Use a precompiled preamble when possible (pdfLaTeX only).
    pub cache_preamble: bool,
    /// Code run before the document, e.g. the CJK setup (not with a precompiled preamble).
    pub pretex: Option<String>,
}

impl LiveRequest {
    /// `<project>/.texcraft/live`.
    pub fn live_dir(&self) -> PathBuf {
        self.project_dir.join(".texcraft").join("live")
    }

    /// `<project>/.texcraft/build` (where full typesets write the `.bbl`).
    pub fn build_dir(&self) -> PathBuf {
        self.project_dir.join(".texcraft").join("build")
    }

    /// File stem of the root.
    pub fn stem(&self) -> String {
        self.root_rel
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "texput".into())
    }
}

const MANIFEST: &str = ".texcraft-mirror";

/// Writes the buffers into the live folder, removes mirror copies of buffers that were closed,
/// recreates the project's folder structure and copies the root (if it is not open) and the last
/// `.bbl`. Returns the root's text.
pub fn prepare_mirror(
    req: &LiveRequest,
    root_override: impl FnOnce(&str) -> Option<String>,
) -> std::io::Result<(String, bool)> {
    let live = req.live_dir();
    std::fs::create_dir_all(&live)?;
    if let Some(temp) = live.parent() {
        texcraft_project::ignore_temp_folder(temp);
    }
    for d in req.dirs.iter().filter(|d| is_safe_rel(d)) {
        let _ = std::fs::create_dir_all(live.join(d));
    }

    let mut files: Vec<(PathBuf, String)> = req.buffers.clone();
    if !files.iter().any(|(p, _)| *p == req.root_rel) {
        let text = std::fs::read_to_string(req.project_dir.join(&req.root_rel))?;
        files.push((req.root_rel.clone(), text));
    }
    let root_text = files
        .iter()
        .find(|(p, _)| *p == req.root_rel)
        .map(|(_, t)| t.clone())
        .unwrap_or_default();
    // The root as TeX reads it: the user's text, or a variant for a precompiled preamble.
    let overridden = root_override(&root_text);
    let replaced = overridden.is_some();
    if let Some(text) = overridden
        && let Some(entry) = files.iter_mut().find(|(p, _)| *p == req.root_rel)
    {
        entry.1 = text;
    }
    // Remove copies of files that are no longer mirrored, so TEXINPUTS finds the real ones.
    let manifest = live.join(MANIFEST);
    if let Ok(old) = std::fs::read_to_string(&manifest) {
        for rel in old.lines().filter(|l| !l.trim().is_empty()) {
            let rel = PathBuf::from(rel);
            if !files.iter().any(|(p, _)| *p == rel) && is_safe_rel(&rel) {
                let _ = std::fs::remove_file(live.join(&rel));
            }
        }
    }
    let mut list = String::new();
    for (rel, text) in &files {
        if !is_safe_rel(rel) {
            continue;
        }
        let dest = live.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Rewrite only when changed (keeps mtimes stable).
        if std::fs::read_to_string(&dest).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&dest, text)?;
        }
        list.push_str(&rel.to_string_lossy());
        list.push('\n');
    }
    std::fs::write(&manifest, list)?;

    // Bibliography from the last full typeset.
    let stem = req.stem();
    let bbl = req.build_dir().join(format!("{stem}.bbl"));
    let live_bbl = live.join(format!("{stem}.bbl"));
    if let Ok(content) = std::fs::read(&bbl)
        && std::fs::read(&live_bbl).ok().as_deref() != Some(content.as_slice())
    {
        let _ = std::fs::write(&live_bbl, content);
    }
    Ok((root_text, replaced))
}

/// Writes `text` as the root's copy in the live folder (when it differs).
pub fn write_root(req: &LiveRequest, text: &str) -> std::io::Result<()> {
    if !is_safe_rel(&req.root_rel) {
        return Ok(());
    }
    let dest = req.live_dir().join(&req.root_rel);
    if std::fs::read_to_string(&dest).ok().as_deref() != Some(text) {
        std::fs::write(&dest, text)?;
    }
    Ok(())
}

/// The root as XeTeX reads it with a precompiled preamble. XeTeX cannot store system (OpenType)
/// fonts in a format, and LaTeX selects one as soon as the class sets the type size. So, on the
/// root's first line (line numbers stay put), the preamble is read with TFM fonts (OT1) and,
/// when TexCraft adds CJK support (`cjk_fonts`: the font commands), xeCJK is loaded after the
/// class with font selection paused; on the `\begin{document}` line, `\endofdump` ends the
/// dump and each pass restores XeLaTeX's TU encoding (unless the preamble chose another, such as
/// T1) and sets the CJK fonts. `None`: no `\begin{document}` in the root, or it manages formats
/// itself.
pub fn xe_format_source(root_text: &str, cjk_fonts: Option<&str>) -> Option<String> {
    format_source(root_text, Engine::XeLatex, cjk_fonts)
}

/// Fails the format run (not the passes) when the preamble left Lua state behind that a format
/// cannot keep: a callback a package registered (the kernel's `ltx.*`, luaotfload's and
/// `tracingstacklevels` are fine), or a font loaded through Lua (luaotfload).
const LUA_FORMAT_GUARD: &str = "\\ifx\\endofdump\\relax\\else\\directlua{local bad = false for name in pairs(callback.list()) do for _, v in ipairs(luatexbase.callback_descriptions(name) or {}) do if not (string.find(v, 'luaotfload', 1, true) == 1 or string.find(v, 'ltx.', 1, true) == 1 or v == 'tracingstacklevels') then bad = true end end end for _, f in font.each() do if f.encodingbytes == 2 then bad = true end end if bad then tex.error('TexCraft: no format for this preamble') end}\\fi";

/// [`xe_format_source`] for XeLaTeX and LuaLaTeX. LuaTeX cannot keep Lua state (callbacks,
/// luaotfload's fonts) in a format either: the dump also checks the preamble left none.
pub fn format_source(root_text: &str, engine: Engine, cjk_fonts: Option<&str>) -> Option<String> {
    if !matches!(engine, Engine::XeLatex | Engine::LuaLatex)
        || root_text.starts_with("%&")
        || root_text.contains("endofdump")
    {
        return None;
    }
    let at = preamble(root_text)?.len();
    let mut head =
        String::from("\\def\\texcraftdumpenc{OT1}\\def\\encodingdefault{OT1}\\fontencoding{OT1}");
    let mut run = String::from(if engine == Engine::LuaLatex {
        LUA_FORMAT_GUARD
    } else {
        ""
    });
    run.push_str(
        "\\csname endofdump\\endcsname\\ifx\\encodingdefault\\texcraftdumpenc\\def\\encodingdefault{TU}\\fi",
    );
    if let Some(fonts) = cjk_fonts.filter(|_| engine == Engine::XeLatex) {
        head.push_str(
            "\\ifdefined\\AddToHook\\AddToHook{class/after}{\\ifdefined\\xeCJKsetup\\else\\def\\texcraftcjk{}\\let\\texcraftselectfont\\selectfont\\let\\selectfont\\relax\\RequirePackage{xeCJK}\\let\\selectfont\\texcraftselectfont\\def\\encodingdefault{OT1}\\fontencoding{OT1}\\fi}\\fi",
        );
        run.push_str(&format!("\\ifdefined\\texcraftcjk {fonts}\\fi"));
    }
    Some(format!(
        "{head}{}{run}{}",
        root_text.get(..at)?,
        root_text.get(at..)?
    ))
}

/// A relative path without `..` or absolute components.
fn is_safe_rel(p: &Path) -> bool {
    p.is_relative()
        && p.components().all(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

/// The preamble of `root_text` (everything before `\begin{document}`), if there is one.
pub fn preamble(root_text: &str) -> Option<&str> {
    let mut offset = 0usize;
    for line in root_text.split_inclusive('\n') {
        let code = line.split('%').next().unwrap_or("");
        if let Some(i) = code.find("\\begin{document}") {
            return root_text.get(..offset + i);
        }
        offset += line.len();
    }
    None
}

/// A hash identifying the preamble and the project-local files it loads.
pub fn preamble_key(req: &LiveRequest, preamble: &str, extra: &str) -> u64 {
    let mut h = DefaultHasher::new();
    preamble.hash(&mut h);
    extra.hash(&mut h);
    req.engine.id().hash(&mut h);
    // Local packages, classes and \input-ed preamble files change the format too.
    for (offset, name) in texcraft_syntax::outline::commands(preamble) {
        if !matches!(
            name,
            "usepackage" | "RequirePackage" | "documentclass" | "input" | "include" | "LoadClass"
        ) {
            continue;
        }
        let at = offset + 1 + name.len();
        let Some((arg, _)) = texcraft_syntax::outline::skip_optional(preamble, at)
            .and_then(|p| texcraft_syntax::outline::braced_argument(preamble, p))
        else {
            continue;
        };
        for item in arg.split(',') {
            let item = item.trim();
            for cand in [
                item.to_string(),
                format!("{item}.sty"),
                format!("{item}.cls"),
                format!("{item}.tex"),
            ] {
                let rel = PathBuf::from(&cand);
                let text = req
                    .buffers
                    .iter()
                    .find(|(p, _)| *p == rel)
                    .map(|(_, t)| t.clone())
                    .or_else(|| std::fs::read_to_string(req.project_dir.join(&rel)).ok());
                if let Some(t) = text {
                    cand.hash(&mut h);
                    t.hash(&mut h);
                }
            }
        }
    }
    h.finish()
}

/// Makes sure a precompiled preamble format exists for `req` and returns its name (for
/// `-fmt=`), or `None` when it cannot be used (other engines, no preamble, or the preamble does
/// not dump, e.g. it loads system fonts with fontspec). Builds it if needed from the root's copy
/// in the live folder (for XeLaTeX the [`xe_format_source`] variant, whose code is `extra`);
/// failures are remembered so they are not retried.
pub fn ensure_format(
    req: &LiveRequest,
    root_text: &str,
    tc: &crate::Toolchain,
    env: &[(String, std::ffi::OsString)],
    extra: &str,
) -> Option<String> {
    let (prog, base) = match req.engine {
        Engine::PdfLatex => (tc.pdftex.clone()?, "&pdflatex"),
        Engine::XeLatex => (tc.xelatex.clone()?, "&xelatex"),
        Engine::LuaLatex => (tc.lualatex.clone()?, "&lualatex"),
        Engine::Tectonic => return None,
    };
    if !req.cache_preamble {
        return None;
    }
    let pre = preamble(root_text)?;
    // Documents that set the format themselves or use \endofdump know better.
    if root_text.starts_with("%&") {
        return None;
    }
    let key = preamble_key(req, pre, extra);
    let name = format!("texcraft-{key:016x}");
    let live = req.live_dir();
    let fmt = live.join(format!("{name}.fmt"));
    let failed = live.join(format!("{name}.nofmt"));
    if fmt.is_file() {
        return Some(name);
    }
    if failed.is_file() {
        return None;
    }
    // Old formats are 4–20 MB each: keep only the current one.
    if let Ok(rd) = std::fs::read_dir(&live) {
        for e in rd.filter_map(Result::ok) {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("texcraft-")
                && (n.ends_with(".fmt") || n.ends_with(".nofmt") || n.ends_with(".log"))
            {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let mut cmd = Command::new(prog);
    cmd.current_dir(&live)
        .arg("-ini")
        .arg("-interaction=nonstopmode")
        .arg(format!("-jobname={name}"))
        .arg(base)
        .arg("mylatexformat.ltx")
        .arg(&req.root_rel)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if req.shell_escape {
        cmd.arg("-shell-escape");
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let ran = cmd.status().is_ok_and(|s| s.success());
    // XeTeX writes a format even after "Can't \dump a format with native fonts": unusable;
    // LuaLaTeX formats fail the Lua-state check in [`format_source`].
    let dumped_fonts = std::fs::read(live.join(format!("{name}.log"))).is_ok_and(|log| {
        let log = String::from_utf8_lossy(&log);
        log.contains("Can't \\dump") || log.contains("TexCraft: no format")
    });
    let ok = ran && fmt.is_file() && !dumped_fonts;
    if ok {
        // The engines read uncompressed formats directly: saves 50–80 ms of inflating per pass.
        let _ = inflate_in_place(&fmt);
        let _ = std::fs::remove_dir_all(live.join(FAST_DIR));
        Some(name)
    } else {
        let _ = std::fs::remove_file(&fmt);
        let _ = std::fs::write(&failed, "");
        None
    }
}

/// Replaces a gzip file with its inflated content.
fn inflate_in_place(path: &Path) -> std::io::Result<()> {
    use std::io::Read;
    let bytes = std::fs::read(path)?;
    if bytes.get(..2) != Some(&[0x1f, 0x8b]) {
        return Ok(());
    }
    let mut out = Vec::with_capacity(bytes.len().saturating_mul(3));
    flate2::read::GzDecoder::new(bytes.as_slice()).read_to_end(&mut out)?;
    let tmp = path.with_extension("fmt.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(tmp, path)
}

/// Folder (in the live folder) holding the fast environment.
pub const FAST_DIR: &str = ".texcraft-fast";

/// Engine variables copied into the minimal `texmf.cnf`: the format must see the memory sizes it
/// was built with, and the security settings must stay the user's.
const ENGINE_VARS: &[&str] = &[
    "main_memory",
    "extra_mem_top",
    "extra_mem_bot",
    "font_mem_size",
    "font_max",
    "hash_extra",
    "pool_size",
    "string_vacancies",
    "max_strings",
    "pool_free",
    "buf_size",
    "nest_size",
    "max_in_open",
    "param_size",
    "save_size",
    "stack_size",
    "dvi_buf_size",
    "error_line",
    "half_error_line",
    "max_print_line",
    "trie_size",
    "hyph_size",
    "expand_depth",
    "openout_any",
    "openin_any",
    "shell_escape",
    "shell_escape_commands",
];

/// The engine variables' values for pdfLaTeX, read once with `kpsewhich -expand-var`.
pub fn engine_vars(
    kpsewhich: &Path,
    env: &[(String, std::ffi::OsString)],
) -> Option<Vec<(String, String)>> {
    static VARS: std::sync::OnceLock<Option<Vec<(String, String)>>> = std::sync::OnceLock::new();
    VARS.get_or_init(|| {
        let expr: Vec<String> = ENGINE_VARS.iter().map(|v| format!("${v}")).collect();
        let mut cmd = Command::new(kpsewhich);
        cmd.args(["-engine=pdftex", "-progname=pdflatex"])
            .arg(format!("-expand-var={}", expr.join("|")))
            .current_dir(std::env::temp_dir())
            .stdin(std::process::Stdio::null());
        for (k, v) in env.iter().filter(|(k, _)| k == "PATH") {
            cmd.env(k, v);
        }
        let out = cmd.output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let values: Vec<&str> = text.split('|').collect();
        if values.len() != ENGINE_VARS.len() {
            return None;
        }
        Some(
            ENGINE_VARS
                .iter()
                .zip(values)
                .filter(|(_, v)| !v.is_empty() && !v.starts_with('$'))
                .map(|(k, v)| ((*k).to_string(), v.to_string()))
                .collect(),
        )
    })
    .clone()
}

/// Whether a path can be written into a `texmf.cnf` value unchanged.
fn cnf_safe(p: &Path) -> bool {
    let s = p.to_string_lossy();
    let separators: &[char] = if cfg!(windows) { &[';'] } else { &[';', ':'] };
    !s.contains(['$', '{', '}', '%', ',', '!', '\n']) && !s.contains(separators)
}

/// Names TeX reported missing in a log (files, packages, fonts, or `<environment>` when the
/// configuration itself did not fit).
pub fn missing_files(log: &str) -> Vec<String> {
    let quoted = |l: &str, after: &str| -> Option<String> {
        let i = l.find(after)? + after.len();
        let rest = l.get(i..)?;
        let end = rest.find('\'')?;
        rest.get(..end).map(str::to_string)
    };
    let mut out: Vec<String> = Vec::new();
    for l in log.lines() {
        let name = if l.contains("' not found") {
            quoted(l, "File `")
        } else if l.contains("I can't find file") {
            quoted(l, "find file `")
        } else if l.contains("Metric (TFM) file") {
            l.split_whitespace()
                .find_map(|w| w.split('=').nth(1).map(str::to_string))
        } else if l.contains("Font ") && l.contains(" not found") {
            l.split("Font ")
                .nth(1)
                .and_then(|r| r.split_whitespace().next())
                .map(str::to_string)
        } else if l.contains("Fatal format file error") || l.contains("kpathsea: Running mktex") {
            Some("<environment>".to_string())
        } else {
            None
        };
        if let Some(n) = name.filter(|n| !n.is_empty())
            && !out.contains(&n)
        {
            out.push(n);
        }
    }
    out
}

/// Whether the fast environment exists for `format`.
pub fn fast_env_ready(live_dir: &Path, format: &str) -> bool {
    std::fs::read_to_string(live_dir.join(FAST_DIR).join("format"))
        .is_ok_and(|f| f.trim() == format)
}

/// Names a normal pass could not find either (the document's own mistakes): a fast pass that
/// misses only these is fine.
pub fn known_missing(live_dir: &Path) -> Vec<String> {
    std::fs::read_to_string(live_dir.join(FAST_DIR).join("missing"))
        .map(|t| {
            t.lines()
                .map(str::to_string)
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Builds the fast environment from a normal pass's recorder file (`.fls`). Returns false when
/// it cannot (unsafe paths, two different files with one name).
pub fn build_fast_env(
    live_dir: &Path,
    project_dir: &Path,
    fls: &str,
    vars: &[(String, String)],
    format: &str,
    missing: &[String],
) -> std::io::Result<bool> {
    let fast = live_dir.join(FAST_DIR);
    let farm = fast.join("farm");
    let _ = std::fs::remove_dir_all(&fast);
    if !cnf_safe(live_dir) || !cnf_safe(project_dir) {
        return Ok(false);
    }
    std::fs::create_dir_all(&farm)?;
    let live_canon = std::fs::canonicalize(live_dir).unwrap_or_else(|_| live_dir.to_path_buf());
    let project_canon =
        std::fs::canonicalize(project_dir).unwrap_or_else(|_| project_dir.to_path_buf());
    let mut seen: std::collections::HashMap<std::ffi::OsString, PathBuf> =
        std::collections::HashMap::new();
    for line in fls.lines() {
        let Some(path) = line.strip_prefix("INPUT ") else {
            continue;
        };
        let path = Path::new(path);
        if !path.is_absolute()
            || path.starts_with(live_dir)
            || path.starts_with(&live_canon)
            || path.starts_with(project_dir)
            || path.starts_with(&project_canon)
            || path.extension().is_some_and(|e| e == "fmt" || e == "cnf")
            || !path.is_file()
        {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        match seen.get(name) {
            Some(prev) if prev != path => {
                let _ = std::fs::remove_dir_all(&fast);
                return Ok(false);
            }
            Some(_) => continue,
            None => {}
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(path, farm.join(name))?;
        #[cfg(not(unix))]
        std::fs::copy(path, farm.join(name)).map(|_| ())?;
        seen.insert(name.to_os_string(), path.to_path_buf());
    }
    let (l, p, f) = (live_dir.display(), project_dir.display(), farm.display());
    let mut cnf = String::from("% Generated by TexCraft for fast live previews.\nTEXMFDBS =\n");
    cnf.push_str(&format!("TEXINPUTS = .;{l};{p};{f}\n"));
    for v in [
        "TFMFONTS",
        "T1FONTS",
        "ENCFONTS",
        "TEXFONTMAPS",
        "VFFONTS",
        "TEXPSHEADERS",
        "OPENTYPEFONTS",
        "TTFONTS",
        "AFMFONTS",
        "MISCFONTS",
        "TEXCONFIG",
        "PKFONTS",
        "BIBINPUTS",
        "BSTINPUTS",
    ] {
        cnf.push_str(&format!("{v} = .;{l};{f}\n"));
    }
    cnf.push_str(&format!(
        "TEXFORMATS = .;{l}\nTEXMFVAR = {l}\nTEXMFCONFIG = {l}\n"
    ));
    for (k, v) in vars {
        cnf.push_str(&format!("{k} = {v}\n"));
    }
    std::fs::write(fast.join("texmf.cnf"), cnf)?;
    std::fs::write(fast.join("missing"), missing.join("\n"))?;
    std::fs::write(fast.join("format"), format)?;
    Ok(true)
}

/// Environment for a fast pass: our `texmf.cnf`, no inherited search paths, no font generation.
/// Empty values mean "remove from the environment".
pub fn fast_env(
    base: &[(String, std::ffi::OsString)],
    live_dir: &Path,
) -> Vec<(String, std::ffi::OsString)> {
    let mut env: Vec<(String, std::ffi::OsString)> = base
        .iter()
        .filter(|(k, _)| k != "TEXINPUTS" && k != "BIBINPUTS")
        .cloned()
        .collect();
    env.push(("TEXMFCNF".into(), live_dir.join(FAST_DIR).into_os_string()));
    for k in [
        "TEXINPUTS",
        "BIBINPUTS",
        "TEXMFHOME",
        "TEXMFLOCAL",
        "TEXFONTMAPS",
        "TFMFONTS",
        "T1FONTS",
        "ENCFONTS",
        "VFFONTS",
    ] {
        env.push((k.into(), std::ffi::OsString::new()));
    }
    for k in ["MKTEXPK", "MKTEXTFM", "MKTEXTEX", "MKTEXFMT"] {
        env.push((k.into(), "0".into()));
    }
    env
}

/// The engine program and arguments for a live pass (run in the live folder). `recorder` adds
/// `-recorder` (pdfTeX), used by normal passes to learn which files the fast environment needs.
pub fn live_command(
    req: &LiveRequest,
    tc: &crate::Toolchain,
    format: Option<&str>,
    recorder: bool,
) -> Result<(PathBuf, Vec<String>), String> {
    let root = req.root_rel.to_string_lossy().into_owned();
    let mut args = Vec::new();
    let prog = match req.engine {
        Engine::Tectonic => {
            let prog = tc.tectonic.clone().ok_or("Tectonic is not installed.")?;
            args.extend(
                [
                    "-X",
                    "compile",
                    "--synctex",
                    "--keep-logs",
                    "--keep-intermediates",
                    "--outdir",
                    ".",
                ]
                .map(String::from),
            );
            if req.shell_escape {
                args.extend(["-Z".to_string(), "shell-escape".to_string()]);
            }
            args.push(root);
            return Ok((prog, args));
        }
        Engine::PdfLatex => tc.pdflatex.clone(),
        Engine::XeLatex => tc.xelatex.clone(),
        Engine::LuaLatex => tc.lualatex.clone(),
    };
    let prog = prog.ok_or_else(|| {
        format!(
            "{} is not installed. Install MacTeX or TeX Live.",
            req.engine.label()
        )
    })?;
    if let Some(f) = format {
        args.push(format!("-fmt={f}"));
    }
    if recorder {
        args.push("-recorder".into());
    }
    args.extend(["-synctex=1", "-interaction=nonstopmode", "-file-line-error"].map(String::from));
    if req.shell_escape {
        args.push("-shell-escape".into());
    }
    match &req.pretex {
        // The job name still comes from the root (the first file opened).
        Some(code) if format.is_none() => args.push(format!("{code}\\input{{{root}}}")),
        _ => args.push(root),
    }
    Ok((prog, args))
}

/// `TEXINPUTS`-style search path: the live folder first, then the project, then the defaults.
pub fn search_path(project_dir: &Path) -> std::ffi::OsString {
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut s = std::ffi::OsString::from(format!(".{sep}"));
    s.push(project_dir.as_os_str());
    s.push(sep);
    s
}

/// Font families every reduced map keeps (Computer Modern, AMS, Euler, LaTeX symbols, Latin
/// Modern, rsfs, stmaryrd, wasy, Zapf Dingbats, Symbol).
const BASE_FONT_FAMILIES: &[&str] = &[
    "cm", "msa", "msb", "eu", "lasy", "line", "lcircle", "rsfs", "stmary", "wasy", "pzd", "psy",
    "lm",
];

/// The name of the generated map in the live folder (pdfTeX looks in `.` first).
pub const LOCAL_MAP: &str = "pdftex.map";

/// The system's `pdftex.map`, found once with `kpsewhich` (run outside the live folder so it
/// cannot find our own copy).
pub fn system_font_map(kpsewhich: &Path, env: &[(String, std::ffi::OsString)]) -> Option<PathBuf> {
    static MAP: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    MAP.get_or_init(|| {
        let mut cmd = Command::new(kpsewhich);
        cmd.arg("pdftex.map")
            .current_dir(std::env::temp_dir())
            .stdin(std::process::Stdio::null());
        for (k, v) in env.iter().filter(|(k, _)| k == "PATH") {
            cmd.env(k, v);
        }
        let out = cmd.output().ok()?;
        let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        p.is_file().then_some(p)
    })
    .clone()
}

/// Base names (no extension) of the Type 1 font files pdfTeX embedded, from the end of a log.
pub fn embedded_fonts(log: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in log.split('<').skip(1) {
        let Some(end) = part.find('>') else { continue };
        let file = part.get(..end).unwrap_or("");
        let lower = file.to_ascii_lowercase();
        if !(lower.ends_with(".pfb") || lower.ends_with(".pfa")) {
            continue;
        }
        let base = Path::new(file)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !base.is_empty() && !out.contains(&base) {
            out.push(base);
        }
    }
    out
}

/// A log shows a font that could not be found (with bitmap generation disabled).
pub fn missing_font(log: &str) -> bool {
    log.lines()
        .any(|l| (l.contains("Font ") && l.contains(" not found")) || l.contains("mktexpk"))
}

/// The map lines worth keeping: the base families plus every family (by the first two letters
/// of the TFM name) of a font file the document embedded.
pub fn reduced_map(full_map: &str, used_fonts: &[String]) -> String {
    let font_file = |line: &str| -> Vec<String> {
        line.split_whitespace()
            .filter(|t| t.starts_with('<'))
            .map(|t| t.trim_start_matches(['<', '[']))
            .filter_map(|t| {
                Path::new(t)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .collect()
    };
    let mut families: Vec<String> = BASE_FONT_FAMILIES
        .iter()
        .map(|f| (*f).to_string())
        .collect();
    for line in full_map.lines() {
        if line.starts_with(['%', '#']) || line.trim().is_empty() {
            continue;
        }
        if font_file(line).iter().any(|f| used_fonts.contains(f))
            && let Some(tfm) = line.split_whitespace().next()
        {
            let fam: String = tfm.chars().take(2).collect();
            if !families.contains(&fam) {
                families.push(fam);
            }
        }
    }
    map_for_families(full_map, &families)
}

/// The lines of `full_map` for the font `families` (TFM name prefixes).
fn map_for_families(full_map: &str, families: &[String]) -> String {
    let mut out = String::from(
        "% Generated by TexCraft for live preview: the families this document uses.\n",
    );
    for line in full_map.lines() {
        if line.starts_with(['%', '#']) || line.trim().is_empty() {
            continue;
        }
        let Some(tfm) = line.split_whitespace().next() else {
            continue;
        };
        if families.iter().any(|f| tfm.starts_with(f.as_str())) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The family of a TFM name for reduced maps: its first two characters (`ec-lmr10` → `ec`,
/// `ptmr8t` → `pt`), which also covers the raw fonts of most virtual fonts.
fn family_of(tfm: &str) -> String {
    tfm.chars().take(2).collect()
}

/// xdvipdfmx's configuration in the folder XeTeX runs in: names [`XE_MAP`] instead of the full
/// `pdftex.map`. Found first because passes put `.` ahead of `DVIPDFMXINPUTS`.
pub const XE_CFG: &str = "dvipdfmx.cfg";
/// The reduced map for xdvipdfmx.
pub const XE_MAP: &str = "texcraft-live.map";
/// Families added after xdvipdfmx missed a font (virtual fonts whose raw font is named apart).
pub const XE_FAMILIES: &str = "texcraft-live.families";
/// Written when reduced maps cannot serve this document: passes use the full maps.
pub const XE_NOMAP: &str = "texcraft-live.nomap";

/// The system's `dvipdfmx.cfg`, found once with `kpsewhich`.
pub fn system_dvipdfmx_cfg(
    kpsewhich: &Path,
    env: &[(String, std::ffi::OsString)],
) -> Option<PathBuf> {
    static CFG: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    CFG.get_or_init(|| {
        let mut cmd = Command::new(kpsewhich);
        cmd.args([
            "-progname=dvipdfmx",
            "-format=other text files",
            "dvipdfmx.cfg",
        ])
        .current_dir(std::env::temp_dir())
        .stdin(std::process::Stdio::null());
        for (k, v) in env.iter().filter(|(k, _)| k == "PATH") {
            cmd.env(k, v);
        }
        let out = cmd.output().ok()?;
        let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        p.is_file().then_some(p)
    })
    .clone()
}

/// TFM names a pass read, from its recorder (`.fls`) file.
pub fn tfms_from_fls(fls: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in fls.lines() {
        let Some(path) = line.strip_prefix("INPUT ") else {
            continue;
        };
        let p = Path::new(path.trim());
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("tfm"))
            && let Some(stem) = p.file_stem()
        {
            let stem = stem.to_string_lossy().into_owned();
            if !out.contains(&stem) {
                out.push(stem);
            }
        }
    }
    out
}

/// Writes the reduced map for `tfms` (plus the families in [`XE_FAMILIES`]) and the local
/// configuration naming it into `dir`.
pub fn write_xe_map(
    dir: &Path,
    system_cfg: &str,
    full_map: &str,
    tfms: &[String],
) -> std::io::Result<()> {
    let mut families: Vec<String> = BASE_FONT_FAMILIES
        .iter()
        .map(|f| (*f).to_string())
        .collect();
    let extra = std::fs::read_to_string(dir.join(XE_FAMILIES)).unwrap_or_default();
    for f in tfms
        .iter()
        .map(|t| family_of(t))
        .chain(extra.lines().map(str::to_string))
    {
        if !f.is_empty() && !families.contains(&f) {
            families.push(f);
        }
    }
    std::fs::write(dir.join(XE_MAP), map_for_families(full_map, &families))?;
    let mut cfg = String::from("% Generated by TexCraft for live preview (reduced font map).\n");
    for line in system_cfg.lines() {
        let mut words = line.split_whitespace();
        if words.next() == Some("f") && words.next() == Some("pdftex.map") {
            cfg.push_str(&format!("f {XE_MAP}\n"));
        } else {
            cfg.push_str(line);
            cfg.push('\n');
        }
    }
    std::fs::write(dir.join(XE_CFG), cfg)
}

/// After a pass with the reduced map: the fonts xdvipdfmx could not find (`Some(names)`, maybe
/// empty when it gave none), or `None` when the PDF was written.
pub fn xe_missing_fonts(console: &str) -> Option<Vec<String>> {
    if !(console.contains("Cannot proceed without") || console.contains("No output PDF file")) {
        return None;
    }
    let mut names = Vec::new();
    for part in console.split("Font file name \"").skip(1) {
        if let Some(name) = part.split('"').next()
            && !name.is_empty()
            && !names.iter().any(|n: &String| n == name)
        {
            names.push(name.to_string());
        }
    }
    Some(names)
}

/// Remembers the families of `fonts` for the next reduced map.
pub fn add_xe_families(dir: &Path, fonts: &[String]) -> std::io::Result<()> {
    let path = dir.join(XE_FAMILIES);
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    for f in fonts.iter().map(|n| family_of(n)) {
        if !text.lines().any(|l| l == f) {
            text.push_str(&f);
            text.push('\n');
        }
    }
    std::fs::write(path, text)
}

/// `env` with bitmap (PK) fonts out of reach, for passes with a reduced font map: a font the map
/// misses then fails visibly ("Font … not found") instead of turning silently into a blurry PK
/// font that an earlier run left in the cache.
pub fn without_pk_fonts(
    env: &[(String, std::ffi::OsString)],
    dir: &Path,
) -> Vec<(String, std::ffi::OsString)> {
    let mut out: Vec<(String, std::ffi::OsString)> = env
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "PKFONTS" | "TEXPKS" | "MKTEXPK"))
        .cloned()
        .collect();
    let none = dir.join(".texcraft-no-pk");
    out.push(("PKFONTS".into(), none.clone().into_os_string()));
    out.push(("TEXPKS".into(), none.into_os_string()));
    out.push(("MKTEXPK".into(), "0".into()));
    out
}

/// Environment for an XeLaTeX pass with the reduced map: the local configuration first, and no
/// bitmap fonts, so a font missing from the map stops xdvipdfmx instead of turning into a
/// blurry PK font silently.
pub fn xe_reduced_env(
    env: &[(String, std::ffi::OsString)],
    dir: &Path,
) -> Vec<(String, std::ffi::OsString)> {
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut out: Vec<(String, std::ffi::OsString)> = without_pk_fonts(env, dir)
        .into_iter()
        .filter(|(k, _)| k != "DVIPDFMXINPUTS")
        .collect();
    let mut inputs = std::ffi::OsString::from(format!(".{sep}"));
    if let Some(v) = std::env::var_os("DVIPDFMXINPUTS") {
        inputs.push(v);
    }
    out.push(("DVIPDFMXINPUTS".into(), inputs));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(dir: &Path) -> LiveRequest {
        LiveRequest {
            project_dir: dir.to_path_buf(),
            root_rel: "main.tex".into(),
            buffers: vec![(
                "main.tex".into(),
                "\\documentclass{article}\n\\begin{document}\nHi\n\\end{document}\n".into(),
            )],
            dirs: vec!["chapters".into()],
            engine: Engine::PdfLatex,
            shell_escape: false,
            cache_preamble: true,
            pretex: None,
        }
    }

    #[test]
    fn pretex_goes_before_the_root_on_the_command_line() {
        let dir = Path::new("/proj");
        let tc = crate::Toolchain {
            xelatex: Some("/tex/xelatex".into()),
            ..crate::Toolchain::default()
        };
        let mut r = req(dir);
        r.engine = Engine::XeLatex;
        r.pretex = Some("\\relax".into());
        let (_, args) = live_command(&r, &tc, None, false).unwrap();
        assert_eq!(
            args.last().map(String::as_str),
            Some("\\relax\\input{main.tex}")
        );
    }

    #[test]
    fn reduced_maps_keep_used_families() {
        let map = "% comment\ncmr10 CMR10 <cmr10.pfb\necrm1095 SFRM1095 \" T1Encoding ReEncodeFont \" <cm-super-t1.enc <sfrm1095.pfb\necbx1200 SFBX1200 \" T1Encoding ReEncodeFont \" <cm-super-t1.enc <sfbx1200.pfb\nptmr8r Times-Roman \"TeXBase1Encoding ReEncodeFont\" <8r.enc <utmr8a.pfb\n";
        let log = "Output written\n{/tex/pdftex.map}</tex/fonts/cmr10.pfb></tex/fonts/cm-super/sfrm1095.pfb>\n";
        let used = embedded_fonts(log);
        assert_eq!(used, vec!["cmr10", "sfrm1095"]);
        let r = reduced_map(map, &used);
        assert!(r.contains("cmr10 CMR10"));
        assert!(
            r.contains("ecrm1095") && r.contains("ecbx1200"),
            "the whole ec family comes along"
        );
        assert!(!r.contains("ptmr8r"));
        assert!(missing_font(
            "!pdfTeX error: pdflatex (file eufm10): Font eufm10 at 600 not found"
        ));
        assert!(!missing_font("Output written on x.pdf"));
    }

    #[test]
    fn missing_names_from_logs() {
        let log = "! LaTeX Error: File `nosuch.sty' not found.\n! I can't find file `chap9'.\n! Font \\x=wasy10 at 10pt not loadable: Metric (TFM) file not found.\n!pdfTeX error: pdflatex (file eufm10): Font eufm10 at 600 not found\nOutput written";
        assert_eq!(
            missing_files(log),
            vec!["nosuch.sty", "chap9", "wasy10", "eufm10"]
        );
        assert!(missing_files("No file main.toc.\nOutput written on x.pdf").is_empty());
    }

    #[test]
    fn fast_env_farm_and_cnf() {
        let dir = std::env::temp_dir().join(format!("texcraft-fast-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let live = dir.join("proj/.texcraft/live");
        std::fs::create_dir_all(&live).unwrap();
        let sys = dir.join("sys");
        std::fs::create_dir_all(sys.join("a")).unwrap();
        std::fs::write(sys.join("a/pkg.sty"), "x").unwrap();
        let fls = format!(
            "PWD {}\nINPUT {}\nINPUT ./main.tex\nINPUT {}\n",
            live.display(),
            sys.join("a/pkg.sty").display(),
            live.join("main.aux").display()
        );
        let vars = [("main_memory".to_string(), "5000000".to_string())];
        assert!(build_fast_env(&live, &dir.join("proj"), &fls, &vars, "texcraft-x", &[]).unwrap());
        assert!(live.join(FAST_DIR).join("farm/pkg.sty").exists());
        let cnf = std::fs::read_to_string(live.join(FAST_DIR).join("texmf.cnf")).unwrap();
        assert!(cnf.contains("TEXMFDBS =") && cnf.contains("main_memory = 5000000"));
        assert!(fast_env_ready(&live, "texcraft-x") && !fast_env_ready(&live, "texcraft-y"));
        // Two different files with one name: no farm.
        std::fs::create_dir_all(sys.join("b")).unwrap();
        std::fs::write(sys.join("b/pkg.sty"), "y").unwrap();
        let clash = format!(
            "INPUT {}\nINPUT {}\n",
            sys.join("a/pkg.sty").display(),
            sys.join("b/pkg.sty").display()
        );
        assert!(!build_fast_env(&live, &dir.join("proj"), &clash, &[], "texcraft-x", &[]).unwrap());
        assert!(!fast_env_ready(&live, "texcraft-x"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preamble_split() {
        assert_eq!(
            preamble(
                "\\documentclass{a}\n% \\begin{document}\n\\usepackage{x}\n\\begin{document}\nX"
            ),
            Some("\\documentclass{a}\n% \\begin{document}\n\\usepackage{x}\n")
        );
        assert_eq!(preamble("\\section{x}"), None);
    }

    #[test]
    fn mirror_writes_buffers_and_cleans_up() {
        let dir = std::env::temp_dir().join(format!("texcraft-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("chapters")).unwrap();
        std::fs::write(dir.join("main.tex"), "on disk").unwrap();
        let mut r = req(&dir);
        r.buffers
            .push(("chapters/one.tex".into(), "unsaved chapter".into()));
        let (root, _) = prepare_mirror(&r, |_| None).unwrap();
        assert!(root.contains("\\documentclass"));
        let live = r.live_dir();
        assert_eq!(
            std::fs::read_to_string(live.join("chapters/one.tex")).unwrap(),
            "unsaved chapter"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("main.tex")).unwrap(),
            "on disk",
            "the user's file is untouched"
        );
        // Closing the chapter removes its mirror copy.
        r.buffers.pop();
        prepare_mirror(&r, |_| None).unwrap();
        assert!(!live.join("chapters/one.tex").exists());
        assert!(
            live.join("chapters").is_dir(),
            "folder structure is kept for \\include .aux files"
        );
        // Unsafe paths are never written.
        r.buffers.push(("../escape.tex".into(), "x".into()));
        prepare_mirror(&r, |_| None).unwrap();
        assert!(!dir.join(".texcraft/escape.tex").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preamble_key_tracks_local_packages() {
        let dir = std::env::temp_dir().join(format!("texcraft-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mystyle.sty"), "\\newcommand\\a{1}").unwrap();
        let r = req(&dir);
        let pre = "\\documentclass{article}\\usepackage{mystyle}";
        let k1 = preamble_key(&r, pre, "");
        assert_eq!(k1, preamble_key(&r, pre, ""));
        std::fs::write(dir.join("mystyle.sty"), "\\newcommand\\a{2}").unwrap();
        assert_ne!(k1, preamble_key(&r, pre, ""));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn format_sources_keep_the_lines_and_mark_the_dump() {
        let root = "% !TEX program = xelatex\n\\documentclass{article}\n\\usepackage{amsmath}\n\\begin{document}\nHi\n\\end{document}\n";
        let fonts = "\\setCJKmainfont{Songti SC}";
        let xe = format_source(root, Engine::XeLatex, Some(fonts)).unwrap();
        assert_eq!(xe.lines().count(), root.lines().count());
        let first = xe.lines().next().unwrap();
        assert!(first.starts_with("\\def\\texcraftdumpenc{OT1}"));
        assert!(first.contains("\\RequirePackage{xeCJK}"));
        assert!(first.ends_with("% !TEX program = xelatex"));
        let begin = xe.lines().nth(3).unwrap();
        assert!(begin.starts_with("\\csname endofdump\\endcsname"));
        assert!(begin.contains(fonts) && begin.ends_with("\\begin{document}"));
        // LuaLaTeX: no CJK code, a guard against Lua state before the dump.
        let lua = format_source(root, Engine::LuaLatex, Some(fonts)).unwrap();
        assert!(!lua.contains("xeCJK") && !lua.contains(fonts));
        assert!(lua.contains("TexCraft: no format") && lua.contains("ltx."));
        // Not for pdfLaTeX; not without \begin{document} or with formats of its own.
        assert_eq!(format_source(root, Engine::PdfLatex, None), None);
        assert_eq!(format_source("\\input{x}\n", Engine::XeLatex, None), None);
        assert_eq!(
            format_source(&format!("%&myfmt\n{root}"), Engine::XeLatex, None),
            None
        );
    }

    #[test]
    fn xdvipdfmx_reduced_map_and_its_failures() {
        let fls = "PWD /x\nINPUT /tex/fonts/tfm/ec-lmr10.tfm\nINPUT ./main.aux\nINPUT /tex/fonts/tfm/cmmi10.tfm\nINPUT /tex/fonts/tfm/ec-lmr10.tfm\nOUTPUT main.xdv\n";
        assert_eq!(tfms_from_fls(fls), vec!["ec-lmr10", "cmmi10"]);
        let dir = std::env::temp_dir().join(format!("texcraft-xemap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let full = "% map\nec-lmr10 LMRoman10-Regular \"enc\" <lm-ec.enc <lmr10.pfb\ncmmi10 CMMI10 <cmmi10.pfb\nptmr8r Times-Roman <utmr8a.pfb\nrtxr rtxr <rtxr.pfb\n";
        let cfg = "%comment\np a4\nf  pdftex.map\nf kanjix.map\n";
        write_xe_map(&dir, cfg, full, &tfms_from_fls(fls)).unwrap();
        let map = std::fs::read_to_string(dir.join(XE_MAP)).unwrap();
        assert!(map.contains("ec-lmr10") && map.contains("cmmi10") && !map.contains("ptmr8r"));
        let local = std::fs::read_to_string(dir.join(XE_CFG)).unwrap();
        assert!(local.contains(&format!("f {XE_MAP}")) && !local.contains("pdftex.map"));
        assert!(local.contains("f kanjix.map") && local.contains("p a4"));
        // xdvipdfmx missed the raw font of a virtual font: its family joins the next map.
        let console = "xdvipdfmx:warning: >> Font file name \"rtxr\" was assumed but failed to locate that font.\nxdvipdfmx:fatal: Cannot proceed without .vf or \"physical\" font for PDF output...\nNo output PDF file written.\n";
        let missing = xe_missing_fonts(console).unwrap();
        assert_eq!(missing, vec!["rtxr"]);
        add_xe_families(&dir, &missing).unwrap();
        write_xe_map(&dir, cfg, full, &tfms_from_fls(fls)).unwrap();
        assert!(
            std::fs::read_to_string(dir.join(XE_MAP))
                .unwrap()
                .contains("rtxr")
        );
        assert_eq!(xe_missing_fonts("Output written on main.pdf"), None);
        // Reduced-map passes cannot fall back to cached bitmap fonts.
        let env = without_pk_fonts(&[("MKTEXPK".into(), "1".into())], &dir);
        assert!(env.iter().any(|(k, v)| k == "MKTEXPK" && v == "0"));
        assert!(env.iter().any(|(k, _)| k == "PKFONTS"));
        let xe = xe_reduced_env(&[], &dir);
        assert!(
            xe.iter()
                .any(|(k, v)| k == "DVIPDFMXINPUTS" && v.to_string_lossy().starts_with('.'))
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
