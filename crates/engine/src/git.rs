//! Source Control for the session (the sidebar's Git tab): the repository of the project folder,
//! read and changed through `texcraft-git` on a background thread, one action at a time.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use texcraft_ai::commit as ai_commit;
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
    /// A commit message being written by the AI model.
    drafting: Option<Drafting>,
    job: Option<Job>,
    /// Actions waiting for the running one.
    queue: Vec<Op>,
    read_at: Option<Instant>,
    /// The folder the state belongs to.
    dir: Option<PathBuf>,
}

/// Writing a commit message with the AI model: reading the changes, then the answer.
enum Drafting {
    Reading,
    Asking {
        job: texcraft_ai::Job,
        answer: String,
    },
}

impl GitState {
    /// A git command is running (an automatic refresh included, which shows no progress).
    pub fn is_busy(&self) -> bool {
        self.job.is_some()
    }

    /// The AI model is writing a commit message.
    pub fn is_drafting(&self) -> bool {
        self.drafting.is_some()
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

    /// Has the Assistant's model write a commit message for what the commit would record (the
    /// staged changes, else all of them) into the message box, replacing what is there.
    pub fn git_generate_message(&mut self) -> Result<(), CommandError> {
        if self.git.drafting.is_some() {
            return Ok(());
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
        let staged = !repo.status.staged.is_empty();
        // Say now when no model is set up, not after reading the changes.
        self.ai_target()?;
        self.git_run(Op::Changes { staged })?;
        self.git.error = None;
        self.git.drafting = Some(Drafting::Reading);
        Ok(())
    }

    /// The changes were read: ask the model.
    fn ask_for_message(&mut self, changes: &str) {
        self.git.drafting = None;
        if changes.trim().is_empty() {
            self.git.error = Some(tr("There are no changes to commit.").into());
            return;
        }
        match self.ai_target() {
            Ok((provider, model, api_key)) => {
                let req = ai_commit::request(&provider, &model, api_key, changes);
                self.git.drafting = Some(Drafting::Asking {
                    job: texcraft_ai::start(&req, Arc::clone(&self.waker)),
                    answer: String::new(),
                });
            }
            Err(e) => self.git.error = Some(e.to_string()),
        }
    }

    /// Collects the model's commit message. Returns when to look again.
    pub(crate) fn tick_git_message(&mut self) -> Option<Duration> {
        let Some(Drafting::Asking { job, .. }) = &self.git.drafting else {
            return None;
        };
        let events: Vec<texcraft_ai::Event> = std::iter::from_fn(|| job.try_recv()).collect();
        for e in events {
            let Some(Drafting::Asking { answer, .. }) = &mut self.git.drafting else {
                break;
            };
            match e {
                texcraft_ai::Event::Text(t) => answer.push_str(&t),
                texcraft_ai::Event::Thinking | texcraft_ai::Event::Fallback(_) => {}
                texcraft_ai::Event::Done { .. } => {
                    let message = ai_commit::clean(answer);
                    self.git.drafting = None;
                    if message.is_empty() {
                        self.git.error = Some(tr("The model sent an empty answer.").into());
                    } else {
                        self.git.message = message;
                    }
                }
                texcraft_ai::Event::Refused(_) => {
                    self.git.drafting = None;
                    self.git.error = Some(tr("The model declined this request.").into());
                }
                texcraft_ai::Event::Failed(msg) => {
                    self.git.drafting = None;
                    self.git.error = Some(msg);
                }
            }
        }
        self.git
            .drafting
            .is_some()
            .then_some(Duration::from_millis(100))
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
            let changes = matches!(outcome.op, Op::Changes { .. });
            match outcome.result {
                Ok(Some(text)) if changes => self.ask_for_message(&text),
                Err(e) if changes => {
                    self.git.drafting = None;
                    self.git.error = Some(e.0);
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// A local OpenAI-compatible server that streams `answer` to one request and returns the
    /// request it saw.
    fn fake_model(answer: &'static str) -> (u16, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 65536];
            loop {
                let n = s.read(&mut buf).unwrap();
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req).to_string();
                if let Some(h) = text.find("\r\n\r\n") {
                    let len = text
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if req.len() >= h + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let chunk = serde_json::json!({"choices": [{"delta": {"content": answer}}]});
            let body = format!("data: {chunk}\n\ndata: [DONE]\n\n");
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            String::from_utf8_lossy(&req).to_string()
        });
        (port, handle)
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    fn settle(s: &mut Session) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while (s.git.busy.is_some() || s.git.is_busy() || s.git.is_drafting())
            && Instant::now() < deadline
        {
            s.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The model writes the message from the changes (cleaned to plain sentences), replacing
    /// what was typed; without a model or without changes it says why.
    #[test]
    fn the_model_writes_the_commit_message() {
        if !texcraft_git::available() {
            eprintln!("skipping: no git");
            return;
        }
        let mut s = Session::new(Arc::new(|| {}));
        s.settings.live_typeset = false;
        s.update.url = "/nonexistent/texcraft-latest.json".into();
        let dir = std::env::temp_dir().join(format!(
            "texcraft-engine-git-message-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.name", "TexCraft Test"]);
        git(&dir, &["config", "user.email", "test@example.invalid"]);
        git(&dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("main.tex"), "Old text.\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "First"]);
        std::fs::write(dir.join("main.tex"), "New text.\n").unwrap();
        s.set_project_dir(dir.clone());
        s.run("view.git", &Value::Null).unwrap();
        s.run("git.refresh", &Value::Null).unwrap();
        settle(&mut s);

        let e = s.run("git.generateMessage", &Value::Null).unwrap_err();
        assert!(e.to_string().contains("Settings → AI"), "{e}");
        assert!(!s.git.is_drafting());

        let (port, server) = fake_model("```\nCommit message: \"Rewrite the introduction.\"\n```");
        let ollama = texcraft_ai::PRESETS
            .iter()
            .find(|p| p.id == "ollama")
            .unwrap();
        let i = s.ai.config.add(ollama);
        s.ai.config.providers[i].base_url = format!("http://127.0.0.1:{port}/v1");
        s.ai.config.providers[i].models = vec!["m".into()];
        s.ai.config.normalize();
        s.git.message = "typed before".into();
        s.run("git.generateMessage", &Value::Null).unwrap();
        assert!(s.git.is_drafting());
        settle(&mut s);
        assert_eq!(
            s.git.message, "Rewrite the introduction.",
            "{:?}",
            s.git.error
        );
        let req = server.join().unwrap();
        assert!(
            req.contains("+New text."),
            "the diff went to the model: {req}"
        );
        assert!(req.contains("main.tex"), "{req}");

        git(&dir, &["commit", "-q", "-a", "-m", "Second"]);
        s.run("git.refresh", &Value::Null).unwrap();
        settle(&mut s);
        let e = s.run("git.generateMessage", &Value::Null).unwrap_err();
        assert!(e.to_string().contains("no changes"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
