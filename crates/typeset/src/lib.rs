//! Typesetting: finds the TeX toolchain, runs latexmk or tectonic for a root file on a
//! background thread, streams the console output and reports the PDF, SyncTeX file and the
//! issues parsed from the log.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use texcraft_i18n::{tr, trf};
use texcraft_log::{Issue, LogReport, parse_blg, parse_log};
use texcraft_project::{BibTool, Engine, build_dir};

pub mod cjk;
pub mod live;
pub use cjk::CjkFonts;
pub use live::LiveRequest;

/// Where the TeX programs are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toolchain {
    /// `latexmk`
    pub latexmk: Option<PathBuf>,
    /// `pdflatex`
    pub pdflatex: Option<PathBuf>,
    /// `xelatex`
    pub xelatex: Option<PathBuf>,
    /// `lualatex`
    pub lualatex: Option<PathBuf>,
    /// `tectonic`
    pub tectonic: Option<PathBuf>,
    /// `synctex`
    pub synctex: Option<PathBuf>,
    /// `pdftex` (builds precompiled preambles)
    pub pdftex: Option<PathBuf>,
    /// Directories prepended to `PATH` for the engines (where the TeX binaries live).
    pub bin_dirs: Vec<PathBuf>,
    /// Fonts for CJK text in documents that do not set it up (see [`cjk`]).
    pub cjk_fonts: Option<CjkFonts>,
}

impl Toolchain {
    /// Looks for the programs on `PATH` and in the usual TeX distribution folders (a GUI app on
    /// macOS does not inherit the shell's `PATH`).
    pub fn detect() -> Self {
        let dirs = search_dirs();
        let find = |name: &str| -> Option<PathBuf> {
            dirs.iter().map(|d| d.join(exe(name))).find(|p| p.is_file())
        };
        let latexmk = find("latexmk");
        let pdflatex = find("pdflatex");
        let mut bin_dirs = Vec::new();
        for p in [&pdflatex, &latexmk].into_iter().flatten() {
            if let Some(d) = p.parent()
                && !bin_dirs.iter().any(|b| b == d)
            {
                bin_dirs.push(d.to_path_buf());
            }
        }
        let mut tc = Self {
            latexmk,
            pdflatex,
            xelatex: find("xelatex"),
            lualatex: find("lualatex"),
            tectonic: find("tectonic"),
            synctex: find("synctex"),
            pdftex: find("pdftex"),
            bin_dirs,
            cjk_fonts: None,
        };
        tc.cjk_fonts = cjk::detect(tc.kpsewhich().as_deref());
        tc
    }

    /// Code that sets up CJK text for XeLaTeX (see [`cjk::pretex`]), if fonts were found.
    pub fn cjk_pretex(&self) -> Option<String> {
        self.cjk_fonts.as_ref().map(cjk::pretex)
    }

    /// `kpsewhich`, next to the TeX binaries.
    pub fn kpsewhich(&self) -> Option<PathBuf> {
        self.bin_dirs
            .iter()
            .map(|d| d.join(exe("kpsewhich")))
            .find(|p| p.is_file())
    }

    /// Whether `engine` can run with this toolchain.
    pub fn supports(&self, engine: Engine) -> bool {
        match engine {
            Engine::Tectonic => self.tectonic.is_some(),
            Engine::PdfLatex => self.latexmk.is_some() && self.pdflatex.is_some(),
            Engine::XeLatex => self.latexmk.is_some() && self.xelatex.is_some(),
            Engine::LuaLatex => self.latexmk.is_some() && self.lualatex.is_some(),
        }
    }

    /// One-line description for About / diagnostics.
    pub fn describe(&self) -> String {
        let have = |p: &Option<PathBuf>, n: &str| {
            if p.is_some() {
                n.to_string()
            } else {
                trf!("no {program}", program = n)
            }
        };
        format!(
            "{}, {}, {}, {}, {}",
            have(&self.latexmk, "latexmk"),
            have(&self.pdflatex, "pdflatex"),
            have(&self.xelatex, "xelatex"),
            have(&self.lualatex, "lualatex"),
            have(&self.tectonic, "tectonic")
        )
    }
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let mut extra = vec![
        PathBuf::from("/Library/TeX/texbin"),
        PathBuf::from("/usr/texbin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        extra.push(PathBuf::from(&home).join(".cargo/bin"));
        extra.push(PathBuf::from(&home).join("bin"));
    }
    // /usr/local/texlive/<year>/bin/<platform>, newest year first.
    for base in ["/usr/local/texlive", "C:\\texlive"] {
        if let Ok(years) = std::fs::read_dir(base) {
            let mut years: Vec<PathBuf> = years.filter_map(Result::ok).map(|e| e.path()).collect();
            years.sort();
            for y in years.into_iter().rev() {
                if let Ok(plats) = std::fs::read_dir(y.join("bin")) {
                    extra.extend(plats.filter_map(Result::ok).map(|e| e.path()));
                }
            }
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        extra.push(PathBuf::from(local).join("Programs\\MiKTeX\\miktex\\bin\\x64"));
    }
    for d in extra {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    dirs
}

/// What to typeset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The root `.tex` file (absolute).
    pub root: PathBuf,
    /// Engine.
    pub engine: Engine,
    /// Allow `\write18`.
    pub shell_escape: bool,
    /// Bibliography policy.
    pub bib_tool: BibTool,
    /// Code run before the document (latexmk `-usepretex`), e.g. the CJK setup.
    pub pretex: Option<String>,
}

impl Request {
    /// Output directory for this request.
    pub fn out_dir(&self) -> PathBuf {
        build_dir(&self.root)
    }

    /// The directory the engine runs in (the root file's folder).
    pub fn work_dir(&self) -> PathBuf {
        self.root
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }

    fn stem(&self) -> String {
        self.root
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "texput".into())
    }
}

/// The program and arguments for `req`, or why it cannot run.
pub fn command_line(req: &Request, tc: &Toolchain) -> Result<(PathBuf, Vec<String>), String> {
    let file_name = req
        .root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| tr("the root file has no name").to_string())?;
    let out_dir = req.out_dir().to_string_lossy().into_owned();
    match req.engine {
        Engine::Tectonic => {
            let prog = tc.tectonic.clone().ok_or_else(|| {
                tr("Tectonic is not installed. Install it (e.g. `brew install tectonic`) or pick another engine.").to_string()
            })?;
            let mut args = vec![
                "-X".into(),
                "compile".into(),
                "--synctex".into(),
                "--keep-logs".into(),
                "--keep-intermediates".into(),
                "--outdir".into(),
                out_dir,
            ];
            if req.shell_escape {
                args.push("-Z".into());
                args.push("shell-escape".into());
            }
            args.push(file_name);
            Ok((prog, args))
        }
        engine => {
            let prog = tc.latexmk.clone().ok_or_else(|| {
                tr("No TeX distribution found (latexmk is missing). Install MacTeX (https://tug.org/mactex/) or TeX Live, or choose the Tectonic engine.").to_string()
            })?;
            if !tc.supports(engine) {
                return Err(trf!(
                    "{engine} is not installed in this TeX distribution.",
                    engine = engine.label()
                ));
            }
            let mode = match engine {
                Engine::XeLatex => "-xelatex",
                Engine::LuaLatex => "-lualatex",
                _ => "-pdf",
            };
            let mut args = vec![
                mode.to_string(),
                "-synctex=1".into(),
                "-interaction=nonstopmode".into(),
                "-file-line-error".into(),
                "-f".into(),
                "-silent".into(),
                format!("-outdir={out_dir}"),
            ];
            if req.shell_escape {
                args.push("-shell-escape".into());
            }
            if req.bib_tool == BibTool::None {
                args.push("-bibtex-".into());
            }
            if let Some(code) = &req.pretex {
                args.push(format!("-usepretex={code}"));
            }
            args.push(file_name);
            Ok((prog, args))
        }
    }
}

/// The result of one typesetting run.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// Exit status 0 and a fresh PDF.
    pub success: bool,
    /// The run was cancelled.
    pub cancelled: bool,
    /// Exit code, if the process ran.
    pub exit_code: Option<i32>,
    /// The PDF this run produced (or found up to date).
    pub pdf: Option<PathBuf>,
    /// The PDF was (re)written by this run.
    pub pdf_fresh: bool,
    /// The SyncTeX file next to it, if any.
    pub synctex: Option<PathBuf>,
    /// The engine log, if any.
    pub log_path: Option<PathBuf>,
    /// Issues from the log and the bibliography tool.
    pub report: LogReport,
    /// Wall time.
    pub duration: Duration,
    /// Everything the process printed.
    pub console: String,
    /// Set when the process could not start (missing TeX, bad path).
    pub error: Option<String>,
    /// A live-preview pass (unsaved buffers, one engine run).
    pub live: bool,
    /// For live passes: the mirror folder and the project folder it stands for.
    pub mirror: Option<Mirror>,
    /// The live pass used a precompiled preamble.
    pub preamble_cached: bool,
    /// The live pass ran in the fast environment (minimal texmf.cnf and file farm).
    pub fast_env: bool,
}

/// Where a live pass mirrored the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mirror {
    /// `<project>/.texcraft/live`
    pub live_dir: PathBuf,
    /// The project folder.
    pub project_dir: PathBuf,
}

impl Mirror {
    /// Maps a project file to its mirror copy (for forward search), if it lies in the project.
    pub fn to_mirror(&self, file: &Path) -> Option<PathBuf> {
        let f = texcraft_project::normalize(file);
        f.strip_prefix(&self.project_dir)
            .ok()
            .map(|rel| self.live_dir.join(rel))
    }

    /// Maps a mirror path back to the project file (for inverse search and issues).
    pub fn to_project(&self, file: &Path) -> PathBuf {
        let f = texcraft_project::normalize(file);
        let live_canon =
            std::fs::canonicalize(&self.live_dir).unwrap_or_else(|_| self.live_dir.clone());
        for base in [&self.live_dir, &live_canon] {
            if let Ok(rel) = f.strip_prefix(base) {
                return self.project_dir.join(rel);
            }
        }
        f
    }
}

impl Outcome {
    /// Issues worth listing (errors first, then warnings, then bad boxes).
    pub fn issues(&self) -> &[Issue] {
        &self.report.issues
    }
}

/// Events from a running job.
#[derive(Debug, Clone)]
pub enum JobEvent {
    /// A line of console output.
    Output(String),
    /// The run finished (or failed to start, or was cancelled).
    Finished(Box<Outcome>),
}

/// A typesetting run in progress.
pub struct Job {
    rx: Receiver<JobEvent>,
    cancel: Arc<AtomicBool>,
    pid: Arc<Mutex<Option<u32>>>,
    /// When it started.
    pub started: Instant,
    /// The root file being typeset.
    pub root: PathBuf,
    /// A live-preview pass.
    pub live: bool,
}

impl Job {
    /// Next event, if any.
    pub fn try_recv(&self) -> Option<JobEvent> {
        match self.rx.try_recv() {
            Ok(e) => Some(e),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }

    /// Stops the run (and the engine processes it started).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        let pid = *self.pid.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pid) = pid {
            kill_tree(pid);
        }
    }

    /// Blocks until the run finishes (CLI use). Output lines are passed to `on_output`.
    pub fn wait(self, mut on_output: impl FnMut(&str)) -> Outcome {
        loop {
            match self.rx.recv() {
                Ok(JobEvent::Output(line)) => on_output(&line),
                Ok(JobEvent::Finished(o)) => return *o,
                Err(_) => {
                    return Outcome {
                        error: Some(tr("the typesetting thread stopped unexpectedly").into()),
                        ..Outcome::default()
                    };
                }
            }
        }
    }
}

#[cfg(unix)]
fn kill_tree(pid: u32) {
    // The job runs in its own process group (pgid == pid); signal the whole group.
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(format!("-{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(windows)]
fn kill_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(any(unix, windows)))]
fn kill_tree(_pid: u32) {}

/// Starts typesetting `req` on a background thread. `wake` is called after each event.
pub fn start(req: Request, tc: &Toolchain, wake: Arc<dyn Fn() + Send + Sync>) -> Job {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let pid = Arc::new(Mutex::new(None));
    let job = Job {
        rx,
        cancel: cancel.clone(),
        pid: pid.clone(),
        started: Instant::now(),
        root: req.root.clone(),
        live: false,
    };
    let tc = tc.clone();
    let spawned = std::thread::Builder::new()
        .name("texcraft-typeset".into())
        .spawn({
            let tx = tx.clone();
            let wake = wake.clone();
            move || {
                let outcome = run(&req, &tc, &tx, &wake, &cancel, &pid);
                let _ = tx.send(JobEvent::Finished(Box::new(outcome)));
                wake();
            }
        });
    if let Err(e) = spawned {
        let _ = tx.send(JobEvent::Finished(Box::new(Outcome {
            error: Some(trf!("could not start a thread: {e}", e = e)),
            ..Outcome::default()
        })));
        wake();
    }
    job
}

fn run(
    req: &Request,
    tc: &Toolchain,
    tx: &Sender<JobEvent>,
    wake: &Arc<dyn Fn() + Send + Sync>,
    cancel: &AtomicBool,
    pid_slot: &Mutex<Option<u32>>,
) -> Outcome {
    let started = Instant::now();
    let start_time = SystemTime::now();
    let mut outcome = Outcome::default();
    let (prog, args) = match command_line(req, tc) {
        Ok(c) => c,
        Err(e) => {
            outcome.error = Some(e);
            return outcome;
        }
    };
    let out_dir = req.out_dir();
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        outcome.error = Some(trf!(
            "cannot create {path}: {e}",
            path = out_dir.display(),
            e = e
        ));
        return outcome;
    }
    if let Some(temp) = out_dir.parent() {
        texcraft_project::ignore_temp_folder(temp);
    }

    let env = engine_env(tc, None);
    run_process(
        &prog,
        &args,
        &req.work_dir(),
        &env,
        tx,
        wake,
        cancel,
        pid_slot,
        &mut outcome,
    );
    outcome.duration = started.elapsed();

    let stem = req.stem();
    let fresh = |p: &Path| -> bool {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t + Duration::from_secs(1) >= start_time)
    };
    let pdf = out_dir.join(format!("{stem}.pdf"));
    // latexmk exits 0 without rewriting an up-to-date PDF: that PDF is still the result.
    if pdf.is_file() && (fresh(&pdf) || (outcome.exit_code == Some(0) && !outcome.cancelled)) {
        outcome.pdf_fresh = fresh(&pdf);
        outcome.pdf = Some(pdf);
    }
    for name in [format!("{stem}.synctex.gz"), format!("{stem}.synctex")] {
        let p = out_dir.join(name);
        if p.is_file() {
            outcome.synctex = Some(p);
            break;
        }
    }
    let log = out_dir.join(format!("{stem}.log"));
    let work_dir = req.work_dir();
    if log.is_file() {
        if let Ok(bytes) = std::fs::read(&log) {
            outcome.report = parse_log(&String::from_utf8_lossy(&bytes), &work_dir);
        }
        outcome.log_path = Some(log);
    }
    let blg = out_dir.join(format!("{stem}.blg"));
    if blg.is_file()
        && fresh(&blg)
        && let Ok(bytes) = std::fs::read(&blg)
    {
        let mut bib = parse_blg(&String::from_utf8_lossy(&bytes), &work_dir);
        outcome.report.issues.append(&mut bib);
    }
    outcome.report.issues.sort_by_key(|i| i.severity);
    outcome.success = !outcome.cancelled
        && outcome.exit_code == Some(0)
        && outcome.pdf.is_some()
        && outcome.report.errors() == 0;
    outcome
}

/// Environment for engine runs: unwrapped log lines, the TeX binaries on `PATH`, and an optional
/// `TEXINPUTS`/`BIBINPUTS` search path.
fn engine_env(
    tc: &Toolchain,
    search: Option<std::ffi::OsString>,
) -> Vec<(String, std::ffi::OsString)> {
    let mut env: Vec<(String, std::ffi::OsString)> = vec![
        ("max_print_line".into(), "2000".into()),
        ("error_line".into(), "254".into()),
        ("half_error_line".into(), "238".into()),
    ];
    if !tc.bin_dirs.is_empty() {
        let mut paths = tc.bin_dirs.clone();
        if let Some(p) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&p));
        }
        if let Ok(joined) = std::env::join_paths(paths) {
            env.push(("PATH".into(), joined));
        }
    }
    if let Some(s) = search {
        env.push(("TEXINPUTS".into(), s.clone()));
        env.push(("BIBINPUTS".into(), s));
    }
    env
}

/// Runs a program, streaming its output as [`JobEvent::Output`] and into `outcome.console`.
#[allow(clippy::too_many_arguments)]
fn run_process(
    prog: &Path,
    args: &[String],
    cwd: &Path,
    env: &[(String, std::ffi::OsString)],
    tx: &Sender<JobEvent>,
    wake: &Arc<dyn Fn() + Send + Sync>,
    cancel: &AtomicBool,
    pid_slot: &Mutex<Option<u32>>,
    outcome: &mut Outcome,
) {
    let mut cmd = Command::new(prog);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        if v.is_empty() {
            cmd.env_remove(k);
        } else {
            cmd.env(k, v);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let header = format!("$ {} {}", prog.display(), args.join(" "));
    outcome.console.push_str(&header);
    outcome.console.push('\n');
    let _ = tx.send(JobEvent::Output(header));

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            outcome.error = Some(trf!(
                "could not run {program}: {e}",
                program = prog.display(),
                e = e
            ));
            return;
        }
    };
    *pid_slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(child.id());
    if cancel.load(Ordering::SeqCst) {
        kill_tree(child.id());
    }
    let (line_tx, line_rx) = channel::<String>();
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        readers.push(spawn_reader(out, line_tx.clone()));
    }
    if let Some(err) = child.stderr.take() {
        readers.push(spawn_reader(err, line_tx.clone()));
    }
    drop(line_tx);
    for line in line_rx {
        outcome.console.push_str(&line);
        outcome.console.push('\n');
        let _ = tx.send(JobEvent::Output(line));
        wake();
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child.wait();
    *pid_slot.lock().unwrap_or_else(PoisonError::into_inner) = None;
    outcome.exit_code = status
        .as_ref()
        .ok()
        .and_then(std::process::ExitStatus::code);
    outcome.cancelled = cancel.load(Ordering::SeqCst);
}

/// Starts a live-preview pass for `req` on a background thread.
pub fn start_live(req: LiveRequest, tc: &Toolchain, wake: Arc<dyn Fn() + Send + Sync>) -> Job {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let pid = Arc::new(Mutex::new(None));
    let job = Job {
        rx,
        cancel: cancel.clone(),
        pid: pid.clone(),
        started: Instant::now(),
        root: req.project_dir.join(&req.root_rel),
        live: true,
    };
    let tc = tc.clone();
    let spawned = std::thread::Builder::new()
        .name("texcraft-live".into())
        .spawn({
            let tx = tx.clone();
            let wake = wake.clone();
            move || {
                let outcome = run_live(&req, &tc, &tx, &wake, &cancel, &pid);
                let _ = tx.send(JobEvent::Finished(Box::new(outcome)));
                wake();
            }
        });
    if let Err(e) = spawned {
        let _ = tx.send(JobEvent::Finished(Box::new(Outcome {
            error: Some(trf!("could not start a thread: {e}", e = e)),
            live: true,
            ..Outcome::default()
        })));
        wake();
    }
    job
}

/// Writes xdvipdfmx's reduced map for the TFMs the last pass read (unless reduced maps were given
/// up for this document, or `only_new` and one exists).
fn xe_map(
    tc: &Toolchain,
    env: &[(String, std::ffi::OsString)],
    cwd: &Path,
    stem: &str,
    only_new: bool,
) {
    if cwd.join(live::XE_NOMAP).is_file() || (only_new && cwd.join(live::XE_CFG).is_file()) {
        return;
    }
    let Some(kpsewhich) = tc.kpsewhich() else {
        return;
    };
    let (Some(cfg), Some(map)) = (
        live::system_dvipdfmx_cfg(&kpsewhich, env),
        live::system_font_map(&kpsewhich, env),
    ) else {
        return;
    };
    let (Ok(cfg), Ok(map), Ok(fls)) = (
        std::fs::read_to_string(cfg),
        std::fs::read_to_string(map),
        std::fs::read_to_string(cwd.join(format!("{stem}.fls"))),
    ) else {
        return;
    };
    let _ = live::write_xe_map(cwd, &cfg, &map, &live::tfms_from_fls(&fls));
}

fn run_live(
    req: &LiveRequest,
    tc: &Toolchain,
    tx: &Sender<JobEvent>,
    wake: &Arc<dyn Fn() + Send + Sync>,
    cancel: &AtomicBool,
    pid_slot: &Mutex<Option<u32>>,
) -> Outcome {
    let started = Instant::now();
    let start_time = SystemTime::now();
    let live_dir = req.live_dir();
    let mut outcome = Outcome {
        live: true,
        mirror: Some(Mirror {
            live_dir: live_dir.clone(),
            project_dir: req.project_dir.clone(),
        }),
        ..Outcome::default()
    };
    // XeLaTeX keeps a precompiled preamble too, from a variant of the root (see
    // `live::xe_format_source`); with TexCraft's CJK setup the fonts are set on each pass.
    let cjk_fonts = req
        .pretex
        .as_deref()
        .filter(|p| tc.cjk_pretex().as_deref() == Some(*p))
        .and(tc.cjk_fonts.as_ref())
        .map(cjk::font_setup);
    // XeLaTeX and LuaLaTeX read a variant of the root (see `live::format_source`).
    let variant = req.cache_preamble
        && match req.engine {
            Engine::XeLatex => req.pretex.is_none() || cjk_fonts.is_some(),
            Engine::LuaLatex => req.pretex.is_none(),
            _ => false,
        };
    let (root_text, root_replaced) = match live::prepare_mirror(req, |text| {
        variant
            .then(|| live::format_source(text, req.engine, cjk_fonts.as_deref()))
            .flatten()
    }) {
        Ok(t) => t,
        Err(e) => {
            outcome.error = Some(trf!("cannot prepare the live preview: {e}", e = e));
            return outcome;
        }
    };
    let env = engine_env(tc, Some(live::search_path(&req.project_dir)));
    let format = live::ensure_format(
        req,
        &root_text,
        tc,
        &env,
        cjk_fonts.as_deref().unwrap_or(""),
    );
    if root_replaced && format.is_none() {
        // No format after all (the preamble loads system fonts, say): typeset the user's root.
        if let Err(e) = live::write_root(req, &root_text) {
            outcome.error = Some(trf!("cannot prepare the live preview: {e}", e = e));
            return outcome;
        }
    }
    outcome.preamble_cached = format.is_some();
    let cwd = live_dir.join(req.root_rel.parent().unwrap_or(Path::new("")));
    let stem = req.stem();
    let log_path = cwd.join(format!("{stem}.log"));
    let local_map = cwd.join(live::LOCAL_MAP);
    let uses_pdftex = req.engine == Engine::PdfLatex;
    let uses_xetex = req.engine == Engine::XeLatex;
    // pdfTeX and LuaTeX read `pdftex.map` (38,000 lines): a reduced copy in the folder they run
    // in is found first.
    let uses_map = uses_pdftex || req.engine == Engine::LuaLatex;
    // The fast environment needs a known file set, so only the precompiled-preamble path uses it.
    let mut fast = uses_pdftex
        && format
            .as_deref()
            .is_some_and(|f| live::fast_env_ready(&live_dir, f));
    let read_log = || {
        std::fs::read(&log_path)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    };
    // Up to three attempts: fast pass, then normal passes when the fast environment or the
    // reduced font map turned out to miss something.
    for _attempt in 0..3 {
        let reduced = uses_map && local_map.is_file();
        // XeLaTeX passes always record the TFMs they read (to build xdvipdfmx's reduced map).
        let xe_reduced = uses_xetex && cwd.join(live::XE_CFG).is_file();
        let recorder = (uses_pdftex && !fast) || uses_xetex;
        let (prog, args) = match live::live_command(req, tc, format.as_deref(), recorder) {
            Ok(c) => c,
            Err(e) => {
                outcome.error = Some(e);
                return outcome;
            }
        };
        let env_run = if fast {
            live::fast_env(&env, &live_dir)
        } else if xe_reduced {
            live::xe_reduced_env(&env, &cwd)
        } else {
            if reduced {
                live::without_pk_fonts(&env, &cwd)
            } else {
                env.clone()
            }
        };
        outcome.console.clear();
        run_process(
            &prog,
            &args,
            &cwd,
            &env_run,
            tx,
            wake,
            cancel,
            pid_slot,
            &mut outcome,
        );
        outcome.fast_env = fast;
        if outcome.cancelled || outcome.error.is_some() {
            break;
        }
        if uses_xetex {
            if xe_reduced {
                // A font missing from the reduced map: add its family (or give the map up)
                // and typeset again.
                if let Some(missing) = live::xe_missing_fonts(&outcome.console) {
                    if missing.is_empty() || live::add_xe_families(&cwd, &missing).is_err() {
                        let _ = std::fs::write(cwd.join(live::XE_NOMAP), "");
                    }
                    let _ = std::fs::remove_file(cwd.join(live::XE_CFG));
                    if !missing.is_empty() {
                        xe_map(tc, &env, &cwd, &stem, false);
                    }
                    continue;
                }
            } else if read_log().contains("Output written") {
                xe_map(tc, &env, &cwd, &stem, true);
            }
            break;
        }
        if !uses_map {
            break;
        }
        let log_text = read_log();
        let missing = live::missing_files(&log_text);
        if fast {
            let known = live::known_missing(&live_dir);
            if missing.iter().any(|m| !known.contains(m)) {
                fast = false;
                let _ = std::fs::remove_dir_all(live_dir.join(live::FAST_DIR));
                continue;
            }
            break;
        }
        if reduced && live::missing_font(&log_text) {
            let _ = std::fs::remove_file(&local_map);
            continue;
        }
        if log_text.contains("Output written on") {
            if !reduced
                && let Some(full) = tc.kpsewhich().and_then(|k| live::system_font_map(&k, &env))
                && let Ok(full_text) = std::fs::read_to_string(&full)
            {
                let reduced_text = live::reduced_map(&full_text, &live::embedded_fonts(&log_text));
                let _ = std::fs::write(&local_map, reduced_text);
            }
            // The next pdfTeX passes can use the fast environment.
            if uses_pdftex
                && let (Some(f), Some(vars)) = (
                    format.as_deref(),
                    tc.kpsewhich().and_then(|k| live::engine_vars(&k, &env)),
                )
                && let Ok(fls) = std::fs::read_to_string(cwd.join(format!("{stem}.fls")))
            {
                let _ = live::build_fast_env(&live_dir, &req.project_dir, &fls, &vars, f, &missing);
            }
        }
        break;
    }
    outcome.duration = started.elapsed();
    let out_dir = cwd;
    let fresh = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t + Duration::from_secs(1) >= start_time)
    };
    let pdf = out_dir.join(format!("{stem}.pdf"));
    if pdf.is_file() && fresh(&pdf) {
        outcome.pdf = Some(pdf);
        outcome.pdf_fresh = true;
    }
    for name in [format!("{stem}.synctex.gz"), format!("{stem}.synctex")] {
        let p = out_dir.join(name);
        if p.is_file() {
            outcome.synctex = Some(p);
            break;
        }
    }
    let log = out_dir.join(format!("{stem}.log"));
    if let Ok(bytes) = std::fs::read(&log) {
        // Paths in the log are relative to the mirror, which has the project's layout.
        let mut report = parse_log(&String::from_utf8_lossy(&bytes), &req.project_dir);
        if let Some(m) = &outcome.mirror {
            for i in &mut report.issues {
                if let Some(f) = i.file.take() {
                    i.file = Some(m.to_project(&f));
                }
            }
        }
        report.issues.sort_by_key(|i| i.severity);
        outcome.report = report;
        outcome.log_path = Some(log);
    }
    outcome.success = !outcome.cancelled && outcome.pdf.is_some() && outcome.report.errors() == 0;
    outcome
}

fn spawn_reader(
    stream: impl Read + Send + 'static,
    tx: Sender<String>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf)
                        .trim_end_matches(['\n', '\r'])
                        .to_string();
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tc() -> Toolchain {
        Toolchain {
            latexmk: Some("/tex/latexmk".into()),
            pdflatex: Some("/tex/pdflatex".into()),
            xelatex: Some("/tex/xelatex".into()),
            lualatex: None,
            tectonic: None,
            synctex: None,
            pdftex: None,
            bin_dirs: vec!["/tex".into()],
            cjk_fonts: None,
        }
    }

    fn req(engine: Engine) -> Request {
        Request {
            root: "/proj/main.tex".into(),
            engine,
            shell_escape: false,
            bib_tool: BibTool::Auto,
            pretex: None,
        }
    }

    #[test]
    fn latexmk_command_lines() {
        let (prog, args) = command_line(&req(Engine::XeLatex), &tc()).unwrap();
        assert_eq!(prog, PathBuf::from("/tex/latexmk"));
        assert_eq!(args.first().map(String::as_str), Some("-xelatex"));
        assert!(args.contains(&"-synctex=1".to_string()));
        assert!(args.contains(&"-outdir=/proj/.texcraft/build".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("main.tex"));
        let mut cjk = req(Engine::XeLatex);
        cjk.pretex = Some("\\relax".into());
        let (_, args) = command_line(&cjk, &tc()).unwrap();
        assert!(args.contains(&"-usepretex=\\relax".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("main.tex"));
    }

    #[test]
    fn missing_tools_are_reported() {
        assert!(
            command_line(&req(Engine::LuaLatex), &tc())
                .unwrap_err()
                .contains("LuaLaTeX")
        );
        assert!(
            command_line(&req(Engine::Tectonic), &tc())
                .unwrap_err()
                .contains("Tectonic")
        );
        assert!(
            command_line(&req(Engine::PdfLatex), &Toolchain::default())
                .unwrap_err()
                .contains("No TeX distribution")
        );
    }

    /// Runs the real toolchain when it is installed.
    #[test]
    fn typesets_a_real_document_when_tex_is_installed() {
        let tc = Toolchain::detect();
        if !tc.supports(Engine::PdfLatex) {
            eprintln!("skipping: no TeX installed");
            return;
        }
        let dir = std::env::temp_dir().join(format!("texcraft-typeset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.join("doc.tex");
        std::fs::write(&root, "\\documentclass{article}\n\\begin{document}\nHello \\undefinedcmd world.\n\\end{document}\n").unwrap();
        let job = start(
            Request {
                root: root.clone(),
                engine: Engine::PdfLatex,
                shell_escape: false,
                bib_tool: BibTool::Auto,
                pretex: None,
            },
            &tc,
            Arc::new(|| {}),
        );
        let mut lines = 0;
        let o = job.wait(|_| lines += 1);
        assert!(o.error.is_none(), "{:?}", o.error);
        assert!(lines > 0);
        assert!(o.pdf.is_some(), "console:\n{}", o.console);
        assert!(o.synctex.is_some());
        assert!(o.log_path.is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Chinese text in a plain article: XeLaTeX with the CJK setup typesets it (oracle test).
    #[test]
    fn xelatex_live_passes_use_a_precompiled_preamble_and_a_reduced_font_map() {
        let tc = Toolchain::detect();
        let Some(pretex) = tc.cjk_pretex() else {
            eprintln!("skipping: no CJK fonts");
            return;
        };
        if !tc.supports(Engine::XeLatex) {
            eprintln!("skipping: no XeLaTeX");
            return;
        }
        let dir = std::env::temp_dir().join(format!("texcraft-xelive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let text = |body: &str| {
            format!(
                "\\documentclass[11pt]{{article}}\n\\usepackage[T1]{{fontenc}}\n\\usepackage{{amsmath}}\n\\usepackage{{hyperref}}\n\\begin{{document}}\n\\section{{Intro}}\n{body}\n\\end{{document}}\n"
            )
        };
        let req = |body: &str, pretex: Option<String>| LiveRequest {
            project_dir: dir.clone(),
            root_rel: "main.tex".into(),
            buffers: vec![("main.tex".into(), text(body))],
            dirs: vec![],
            engine: Engine::XeLatex,
            shell_escape: false,
            cache_preamble: true,
            pretex,
        };
        let live = dir.join(".texcraft/live");
        let pass = |r: LiveRequest| start_live(r, &tc, Arc::new(|| {})).wait(|_| {});
        let mut times = Vec::new();
        for (i, body) in ["测试 one $a^2$", "测试 two $b^2$", "测试 three $c^2$"]
            .iter()
            .enumerate()
        {
            let o = pass(req(body, Some(pretex.clone())));
            assert!(o.pdf.is_some(), "pass {i}:\n{}", o.console);
            assert_eq!(o.report.errors(), 0, "pass {i}: {:?}", o.report.issues);
            assert!(o.preamble_cached, "pass {i} used the precompiled preamble");
            let log = std::fs::read_to_string(live.join("main.log")).unwrap();
            assert!(
                !log.contains("Missing character"),
                "pass {i}: CJK glyphs missing"
            );
            times.push(o.duration);
        }
        // The copy TeX read keeps the user's lines (only the first and the \begin{document}
        // lines carry TexCraft's code).
        let copy = std::fs::read_to_string(live.join("main.tex")).unwrap();
        assert_eq!(copy.lines().count(), text("x").lines().count());
        // T1 stayed T1 (the preamble's choice), and the reduced font map was used.
        let pdf = std::fs::read(live.join("main.pdf")).unwrap();
        assert!(
            !pdf.windows(10).any(|w| w == b"Identity-H"),
            "TU fonts instead of T1"
        );
        if tc.kpsewhich().is_some() {
            assert!(live.join(live::XE_CFG).is_file(), "no reduced map");
        }
        eprintln!("XeLaTeX live passes: {times:?}");
        // A preamble that loads system fonts cannot be dumped: plain passes, the user's root.
        let fontspec = LiveRequest {
            buffers: vec![(
                "main.tex".into(),
                "\\documentclass{article}\n\\usepackage{fontspec}\n\\setmainfont{lmroman10-regular.otf}\n\\begin{document}\nHello\n\\end{document}\n".into(),
            )],
            pretex: None,
            ..req("x", None)
        };
        let o = pass(fontspec.clone());
        assert!(o.pdf.is_some(), "{}", o.console);
        assert!(!o.preamble_cached);
        assert_eq!(
            std::fs::read_to_string(live.join("main.tex")).unwrap(),
            fontspec.buffers[0].1
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lualatex_live_passes_use_a_precompiled_preamble_unless_lua_state_is_involved() {
        let tc = Toolchain::detect();
        if !tc.supports(Engine::LuaLatex) {
            eprintln!("skipping: no LuaLaTeX");
            return;
        }
        let dir = std::env::temp_dir().join(format!("texcraft-lualive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let req = |text: String| LiveRequest {
            project_dir: dir.clone(),
            root_rel: "main.tex".into(),
            buffers: vec![("main.tex".into(), text)],
            dirs: vec![],
            engine: Engine::LuaLatex,
            shell_escape: false,
            cache_preamble: true,
            pretex: None,
        };
        let doc = |body: &str| {
            format!(
                "\\documentclass[11pt]{{article}}\n\\usepackage{{amsmath}}\n\\usepackage{{hyperref}}\n\\begin{{document}}\n{body} $x^2$\n\\end{{document}}\n"
            )
        };
        let pass = |r: LiveRequest| start_live(r, &tc, Arc::new(|| {})).wait(|_| {});
        let mut times = Vec::new();
        for body in ["one", "two", "three"] {
            let o = pass(req(doc(body)));
            assert!(o.pdf.is_some(), "{}", o.console);
            assert_eq!(o.report.errors(), 0, "{:?}", o.report.issues);
            assert!(o.preamble_cached);
            times.push(o.duration);
        }
        eprintln!("LuaLaTeX live passes: {times:?}");
        // fontspec loads fonts through Lua in the preamble: no format, the user's root.
        let text = "\\documentclass{article}\n\\usepackage{fontspec}\n\\setmainfont{lmroman10-regular.otf}\n\\begin{document}\nHello\n\\end{document}\n".to_string();
        let o = pass(req(text.clone()));
        assert!(o.pdf.is_some(), "{}", o.console);
        assert!(!o.preamble_cached);
        assert_eq!(
            std::fs::read_to_string(dir.join(".texcraft/live/main.tex")).unwrap(),
            text
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn typesets_chinese_with_the_cjk_setup_when_installed() {
        let tc = Toolchain::detect();
        let Some(pretex) = tc.cjk_pretex() else {
            eprintln!("skipping: no CJK fonts");
            return;
        };
        if !tc.supports(Engine::XeLatex) {
            eprintln!("skipping: no XeLaTeX");
            return;
        }
        let dir = std::env::temp_dir().join(format!("texcraft-cjk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.join("doc.tex");
        std::fs::write(&root, "\\documentclass{article}\n\\usepackage[utf8]{inputenc}\n\\usepackage[T1]{fontenc}\n\\begin{document}\n\\section{介绍}\n这是一个测试。\n\\end{document}\n").unwrap();
        let job = start(
            Request {
                root: root.clone(),
                engine: Engine::XeLatex,
                shell_escape: false,
                bib_tool: BibTool::Auto,
                pretex: Some(pretex),
            },
            &tc,
            Arc::new(|| {}),
        );
        let o = job.wait(|_| {});
        assert!(o.pdf.is_some(), "console:\n{}", o.console);
        assert_eq!(o.report.errors(), 0, "{:?}", o.report.issues);
        let log = std::fs::read_to_string(o.log_path.unwrap()).unwrap();
        assert!(!log.contains("Missing character"), "CJK glyphs missing");
        std::fs::remove_dir_all(&dir).ok();
    }
}
