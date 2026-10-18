//! Source Control for the session (the sidebar's Git tab): the repository of the project folder,
//! read and changed through `texcraft-git` on a background thread, one action at a time.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use texcraft_git::{ChangeKind, Job, Op, Snapshot};
use texcraft_i18n::{tr, trf};

use crate::{CommandError, Effect, Session, SidebarTab};

/// While the Git tab shows, the state is re-read this often.
const REFRESH_EVERY: Duration = Duration::from_secs(4);

/// Source Control state.
#[derive(Default)]
pub struct GitState {
    /// Whether the `git` command exists (`None` until first needed).
    pub available: Option<bool>,
    /// The repository (`None` when the project is not in one, or not read yet).
    pub repo: Option<Snapshot>,
    /// The folder was read and is not in a repository.
    pub not_repo: bool,
    /// The action running now (for the sidebar's progress line).
    pub busy: Option<&'static str>,
    /// The last error, until the next action.
    pub error: Option<String>,
    /// The commit message being written.
    pub message: String,
    /// A diff to show: (title, text).
    pub diff: Option<(String, String)>,
    /// Re-read when convenient (files were saved).
    pub stale: bool,
    job: Option<Job>,
    /// Actions waiting for the running one.
    queue: Vec<Op>,
    read_at: Option<Instant>,
    /// The folder the state belongs to.
    dir: Option<PathBuf>,
}

impl GitState {
    /// A git command is running (an automatic refresh included, which shows no progress).
    pub fn is_busy(&self) -> bool {
        self.job.is_some()
    }
}

impl Session {
    /// The folder Source Control works in: the project folder, else the root file's.
    pub fn git_dir(&self) -> Option<PathBuf> {
        self.project_dir.clone().or_else(|| {
            self.root_path()
                .filter(|p| p.is_file())
                .and_then(|p| p.parent().map(Path::to_path_buf))
        })
    }

    /// Starts (or queues) a Source Control action.
    pub fn git_run(&mut self, op: Op) -> Result<(), CommandError> {
        let dir = self
            .git_dir()
            .ok_or_else(|| CommandError::Failed(tr("open a project folder first").into()))?;
        if *self
            .git
            .available
            .get_or_insert_with(texcraft_git::available)
        {
            if self.git.job.is_some() {
                if !self.git.queue.contains(&op) {
                    self.git.queue.push(op);
                }
                return Ok(());
            }
            self.git.error = None;
            self.git.busy = Some(op.label());
            self.git.dir = Some(dir.clone());
            self.git.job = Some(texcraft_git::start(dir, op, self.waker.clone()));
            Ok(())
        } else {
            Err(CommandError::Failed(
                tr("Git is not installed (https://git-scm.com)").into(),
            ))
        }
    }

    /// The repository-relative path of `path` (absolute or already relative).
    fn git_rel(&self, path: &str) -> String {
        let p = Path::new(path);
        match &self.git.repo {
            Some(r) if p.is_absolute() => p
                .strip_prefix(&r.root)
                .ok()
                .map(Path::to_path_buf)
                .or_else(|| {
                    let real = p.canonicalize().ok()?;
                    let root = r.root.canonicalize().ok()?;
                    real.strip_prefix(&root).ok().map(Path::to_path_buf)
                })
                .map_or_else(
                    || path.to_string(),
                    |r| {
                        r.components()
                            .map(|c| c.as_os_str().to_string_lossy())
                            .collect::<Vec<_>>()
                            .join("/")
                    },
                ),
            _ => path.to_string(),
        }
    }

    /// Stages `path`.
    pub fn git_stage(&mut self, path: &str) -> Result<(), CommandError> {
        let rel = self.git_rel(path);
        self.git_run(Op::Stage(vec![rel]))
    }

    /// Unstages `path`.
    pub fn git_unstage(&mut self, path: &str) -> Result<(), CommandError> {
        let rel = self.git_rel(path);
        self.git_run(Op::Unstage(vec![rel]))
    }

    /// Throws away the working-tree changes to `path`: tracked files go back to the index's
    /// version, untracked files go to the Trash.
    pub fn git_discard(&mut self, path: &str) -> Result<(), CommandError> {
        let rel = self.git_rel(path);
        let repo = self
            .git
            .repo
            .as_ref()
            .ok_or_else(|| CommandError::Failed(tr("not a Git repository").into()))?;
        let untracked = repo
            .status
            .changes
            .iter()
            .any(|c| c.path == rel && c.kind == ChangeKind::Untracked);
        let abs = repo.root.join(&rel);
        if untracked {
            self.trash_file(&abs)?;
            self.git_run(Op::Refresh)
        } else {
            // Open documents of the file must not keep the discarded text.
            if let Some(d) = self.docs.iter().find(|d| {
                d.path
                    .as_ref()
                    .and_then(|p| p.canonicalize().ok())
                    .zip(abs.canonicalize().ok())
                    .is_some_and(|(a, b)| a == b)
                    && d.is_dirty()
            }) {
                return Err(CommandError::Failed(trf!(
                    "{name} has unsaved changes: save or revert it first",
                    name = d.display_name()
                )));
            }
            self.git_run(Op::Discard(vec![rel]))
        }
    }

    /// Commits with the message being written (or `message`); stages everything first when
    /// nothing is staged.
    pub fn git_commit(&mut self, message: Option<&str>) -> Result<(), CommandError> {
        let message = message.map_or_else(|| self.git.message.clone(), str::to_string);
        if message.trim().is_empty() {
            return Err(CommandError::Failed(
                tr("Enter a commit message first.").into(),
            ));
        }
        let repo = self
            .git
            .repo
            .as_ref()
            .ok_or_else(|| CommandError::Failed(tr("not a Git repository").into()))?;
        if repo.status.is_clean() {
            return Err(CommandError::Failed(
                tr("There are no changes to commit.").into(),
            ));
        }
        let all = repo.status.staged.is_empty();
        // Save what is being committed.
        for i in 0..self.docs.len() {
            if self
                .docs
                .get(i)
                .is_some_and(|d| d.path.is_some() && d.is_dirty())
            {
                self.save(i)?;
            }
        }
        self.git_run(Op::Commit { message, all })
    }

    /// Shows the diff of `path` (`staged`: the index's).
    pub fn git_diff(&mut self, path: &str, staged: bool) -> Result<(), CommandError> {
        let rel = self.git_rel(path);
        let untracked = !staged
            && self.git.repo.as_ref().is_some_and(|r| {
                r.status
                    .changes
                    .iter()
                    .any(|c| c.path == rel && c.kind == ChangeKind::Untracked)
            });
        self.git_run(Op::Diff {
            path: rel,
            staged,
            untracked,
        })
    }

    /// Polls the running action; re-reads the repository while the Git tab shows.
    pub(crate) fn tick_git(&mut self, now: Instant) -> Option<Duration> {
        if let Some(outcome) = self.git.job.as_ref().and_then(texcraft_git::Job::try_recv) {
            self.git.job = None;
            self.git.busy = None;
            self.git.read_at = Some(now);
            self.git.not_repo = outcome.snapshot.is_none();
            let committed = matches!(outcome.op, Op::Commit { .. });
            match outcome.result {
                Ok(Some(text)) => {
                    if let Op::Diff { path, staged, .. } = &outcome.op {
                        let title = if *staged {
                            trf!("{path} (Index)", path = path)
                        } else {
                            trf!("{path} (Working Tree)", path = path)
                        };
                        let text = if text.trim().is_empty() {
                            tr("No changes.").to_string()
                        } else {
                            text
                        };
                        self.git.diff = Some((title, text));
                        self.effect(Effect::GitDiff);
                    }
                }
                Ok(None) => {
                    if committed {
                        self.git.message.clear();
                        self.set_status(tr("Committed"));
                    }
                }
                Err(e) => self.git.error = Some(e.0),
            }
            if outcome.snapshot.is_some() || self.git.not_repo {
                self.git.repo = outcome.snapshot;
            }
            if !self.git.queue.is_empty() {
                let op = self.git.queue.remove(0);
                if let Err(e) = self.git_run(op) {
                    self.git.error = Some(e.to_string());
                }
            }
        }
        let shown = self.view.sidebar && self.view.sidebar_tab == SidebarTab::Git;
        if !shown || self.git.job.is_some() || self.git.available == Some(false) {
            return self.git.job.is_some().then_some(Duration::from_millis(100));
        }
        // A different folder (another project was opened): start over.
        if self.git.dir.is_some() && self.git.dir != self.git_dir() {
            let message = std::mem::take(&mut self.git.message);
            self.git = GitState {
                message,
                available: self.git.available,
                ..GitState::default()
            };
        }
        let due = self.git.stale
            || self
                .git
                .read_at
                .is_none_or(|t| now.saturating_duration_since(t) >= REFRESH_EVERY);
        if due {
            self.git.stale = false;
            if self.git_dir().is_some() && self.git_run(Op::Refresh).is_err() {
                return None;
            }
            // Automatic refreshes are silent: no progress line, buttons stay usable (an action
            // waits in the queue until the refresh is done).
            self.git.busy = None;
            return Some(Duration::from_millis(100));
        }
        self.git
            .read_at
            .map(|t| REFRESH_EVERY.saturating_sub(now.saturating_duration_since(t)))
    }
}
