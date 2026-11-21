//! Independent native project windows, each with its own engine, preview and AI session.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub struct ProjectWindow {
    child: Child,
    pub path: Option<PathBuf>,
    pub template: Option<String>,
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
            path: Some(path),
            template: None,
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
        })
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn is_running(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(Some(_)))
    }
}
