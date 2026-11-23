//! Independent native project windows, each with its own engine, preview and AI session.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long a window just started may take to claim its project; until then requests for that
/// project wait for it (in its inbox) instead of starting another window.
const STARTING: Duration = Duration::from_secs(15);

pub struct ProjectWindow {
    child: Child,
    pub path: Option<PathBuf>,
    pub template: Option<String>,
    /// The project it was started for (`texcraft_engine::project_key`).
    pub key: Option<PathBuf>,
    started: Instant,
}

pub fn command(executable: &Path, path: &Path, token_file: Option<&Path>) -> Command {
    let mut command = Command::new(executable);
    command.arg(path).arg("--no-welcome");
    // Each controlled window gets a fresh port, while the same owner-only token file lets
    // agents authenticate. The child reports its allocated port in the inherited app log.
    if let Some(token_file) = token_file {
        command
            .args(["--control", "0", "--control-token-file"])
            .arg(token_file);
    }
    command.stdin(Stdio::null());
    command
}

impl ProjectWindow {
    pub fn open(path: PathBuf, token_file: Option<&Path>) -> std::io::Result<Self> {
        let child = command(&std::env::current_exe()?, &path, token_file).spawn()?;
        Ok(Self {
            child,
            key: texcraft_engine::project_key(&path),
            path: Some(path),
            template: None,
            started: Instant::now(),
        })
    }

    pub fn new(template: String, token_file: Option<&Path>) -> std::io::Result<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        command.args(["--new-template", &template, "--no-welcome"]);
        if let Some(token_file) = token_file {
            command
                .args(["--control", "0", "--control-token-file"])
                .arg(token_file);
        }
        let child = command.stdin(Stdio::null()).spawn()?;
        Ok(Self {
            child,
            path: None,
            template: Some(template),
            key: None,
            started: Instant::now(),
        })
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Started moments ago and still running: it may not hold its project yet.
    pub fn starting(&mut self) -> bool {
        self.started.elapsed() < STARTING && self.is_running()
    }

    pub fn is_running(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(Some(_)))
    }
}
