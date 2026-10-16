//! Git for the Source Control sidebar. TexCraft uses the `git` command line the way it uses the
//! TeX programs: [`status`] parses `git status --porcelain=v2`, [`log`] a fixed `--pretty`
//! format, and every operation runs on a background thread ([`start`]) that answers with the
//! result and a fresh [`Snapshot`] of the repository.
//!
//! Commands never wait for a password: terminal prompts are off (`GIT_TERMINAL_PROMPT=0`, SSH in
//! batch mode), so a push that needs credentials fails with git's message instead of hanging.
//! Credential helpers (the macOS keychain, Git Credential Manager) and SSH agents still work.

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

/// A git error, shown to the user (git's own message when it has one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError(pub String);

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GitError {}

fn err(msg: impl Into<String>) -> GitError {
    GitError(msg.into())
}

/// What happened to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    /// Changed.
    Modified,
    /// New in the index.
    Added,
    /// Removed.
    Deleted,
    /// Renamed (see [`Change::orig`]).
    Renamed,
    /// Copied.
    Copied,
    /// File type changed (e.g. file ↔ symlink).
    TypeChanged,
    /// Not tracked yet.
    Untracked,
    /// Unmerged (a merge conflict).
    Conflicted,
}

impl ChangeKind {
    /// The letter the Source Control list shows (VS Code's).
    pub fn letter(self) -> char {
        match self {
            Self::Modified => 'M',
            Self::Added => 'A',
            Self::Deleted => 'D',
            Self::Renamed => 'R',
            Self::Copied => 'C',
            Self::TypeChanged => 'T',
            Self::Untracked => 'U',
            Self::Conflicted => '!',
        }
    }

    /// A short description for tool tips.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Modified => "Modified",
            Self::Added => "Index Added",
            Self::Deleted => "Deleted",
            Self::Renamed => "Renamed",
            Self::Copied => "Copied",
            Self::TypeChanged => "Type Changed",
            Self::Untracked => "Untracked",
            Self::Conflicted => "Conflict",
        }
    }

    fn from_code(c: u8) -> Option<Self> {
        match c {
            b'M' => Some(Self::Modified),
            b'A' => Some(Self::Added),
            b'D' => Some(Self::Deleted),
            b'R' => Some(Self::Renamed),
            b'C' => Some(Self::Copied),
            b'T' => Some(Self::TypeChanged),
            _ => None,
        }
    }
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Path relative to the repository root, `/`-separated.
    pub path: String,
    /// The old path of a rename or copy.
    pub orig: Option<String>,
    /// What happened.
    pub kind: ChangeKind,
}

/// The working tree and index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// Current branch (`None` when HEAD is detached).
    pub branch: Option<String>,
    /// Short commit id of HEAD (`None` before the first commit).
    pub head: Option<String>,
    /// The upstream branch, e.g. `origin/main`.
    pub upstream: Option<String>,
    /// Commits not on the upstream yet.
    pub ahead: u32,
    /// Upstream commits not here yet.
    pub behind: u32,
    /// Changes in the index (what the next commit records).
    pub staged: Vec<Change>,
    /// Changes in the working tree, untracked files and conflicts.
    pub changes: Vec<Change>,
}

impl Status {
    /// Nothing to commit.
    pub fn is_clean(&self) -> bool {
        self.staged.is_empty() && self.changes.is_empty()
    }
}

/// One commit of the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Abbreviated id.
    pub hash: String,
    /// First line of the message.
    pub subject: String,
    /// Author name.
    pub author: String,
    /// Author time (Unix seconds).
    pub time: i64,
}

/// Parses `git status --porcelain=v2 --branch -z`.
pub fn parse_status(out: &[u8]) -> Status {
    let mut st = Status::default();
    let mut fields = out.split(|b| *b == 0).peekable();
    while let Some(entry) = fields.next() {
        let line = String::from_utf8_lossy(entry);
        let Some(kind) = line.chars().next() else {
            continue;
        };
        match kind {
            '#' => {
                let mut parts = line.splitn(3, ' ');
                let _hash = parts.next();
                let key = parts.next().unwrap_or("");
                let value = parts.next().unwrap_or("");
                match key {
                    "branch.oid" if value != "(initial)" => {
                        st.head = Some(value.chars().take(7).collect());
                    }
                    "branch.head" if value != "(detached)" => st.branch = Some(value.into()),
                    "branch.upstream" => st.upstream = Some(value.into()),
                    "branch.ab" => {
                        for n in value.split(' ') {
                            if let Some(a) = n.strip_prefix('+') {
                                st.ahead = a.parse().unwrap_or(0);
                            } else if let Some(b) = n.strip_prefix('-') {
                                st.behind = b.parse().unwrap_or(0);
                            }
                        }
                    }
                    _ => {}
                }
            }
            '1' | '2' => {
                // 1 XY sub mH mI mW hH hI path
                // 2 XY sub mH mI mW hH hI Xscore path NUL origPath
                let n = if kind == '1' { 9 } else { 10 };
                let parts: Vec<&str> = line.splitn(n, ' ').collect();
                let (Some(xy), Some(path)) = (parts.get(1), parts.get(n - 1)) else {
                    continue;
                };
                let orig = if kind == '2' {
                    fields
                        .next()
                        .map(|o| String::from_utf8_lossy(o).into_owned())
                } else {
                    None
                };
                let xy = xy.as_bytes();
                if let Some(k) = xy.first().copied().and_then(ChangeKind::from_code) {
                    st.staged.push(Change {
                        path: (*path).into(),
                        orig: orig.clone(),
                        kind: k,
                    });
                }
                if let Some(k) = xy.get(1).copied().and_then(ChangeKind::from_code) {
                    st.changes.push(Change {
                        path: (*path).into(),
                        orig,
                        kind: k,
                    });
                }
            }
            'u' => {
                // u XY sub m1 m2 m3 mW h1 h2 h3 path
                if let Some(path) = line.splitn(11, ' ').nth(10) {
                    st.changes.push(Change {
                        path: path.into(),
                        orig: None,
                        kind: ChangeKind::Conflicted,
                    });
                }
            }
            '?' => {
                if let Some(path) = line.get(2..) {
                    st.changes.push(Change {
                        path: path.into(),
                        orig: None,
                        kind: ChangeKind::Untracked,
                    });
                }
            }
            _ => {}
        }
    }
    st
}

/// The `--pretty` format [`parse_log`] reads: fields split by 0x1f, records by 0x1e.
const LOG_FORMAT: &str = "--pretty=format:%h%x1f%s%x1f%an%x1f%at%x1e";

/// Parses `git log` output in [`LOG_FORMAT`].
pub fn parse_log(out: &str) -> Vec<Commit> {
    out.split('\u{1e}')
        .filter_map(|rec| {
            let rec = rec.trim_start_matches(['\n', '\r']);
            let mut f = rec.split('\u{1f}');
            let hash = f.next()?.trim();
            if hash.is_empty() {
                return None;
            }
            Some(Commit {
                hash: hash.into(),
                subject: f.next().unwrap_or("").into(),
                author: f.next().unwrap_or("").into(),
                time: f.next().and_then(|t| t.trim().parse().ok()).unwrap_or(0),
            })
        })
        .collect()
}

/// A `git` command in `dir` that never prompts.
fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C")
        .arg(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null());
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        c.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console window flashing up for each call.
        c.creation_flags(0x0800_0000);
    }
    c
}

/// Runs git in `dir`, waiting at most `timeout`. Returns standard output, or git's message.
/// `ok_codes` lists exit codes that count as success (`diff --no-index` exits 1).
fn run_in(
    dir: &Path,
    args: &[&str],
    timeout: Duration,
    ok_codes: &[i32],
) -> Result<Vec<u8>, GitError> {
    let mut child = git(dir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| err(format!("cannot run git: {e}")))?;
    // Read both pipes on threads so a chatty command cannot fill one and block.
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_t = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut v);
        }
        v
    });
    let err_t = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut v);
        }
        v
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(err(format!(
                    "git {} took longer than {} s and was stopped",
                    args.first().unwrap_or(&""),
                    timeout.as_secs()
                )));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(e) => return Err(err(format!("git failed: {e}"))),
        }
    };
    let out = out_t.join().unwrap_or_default();
    let errs = err_t.join().unwrap_or_default();
    if status.success() || status.code().is_some_and(|c| ok_codes.contains(&c)) {
        Ok(out)
    } else {
        let msg = String::from_utf8_lossy(&errs).trim().to_string();
        let msg = if msg.is_empty() {
            String::from_utf8_lossy(&out).trim().to_string()
        } else {
            msg
        };
        Err(err(if msg.is_empty() {
            format!("git {} failed ({status})", args.first().unwrap_or(&""))
        } else {
            msg
        }))
    }
}

/// Local commands.
const LOCAL: Duration = Duration::from_secs(30);
/// Commands that talk to a remote.
const NETWORK: Duration = Duration::from_secs(120);

fn run(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    run_in(dir, args, LOCAL, &[]).map(|o| String::from_utf8_lossy(&o).into_owned())
}

/// Whether the `git` command exists.
pub fn available() -> bool {
    git(Path::new("."))
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The repository containing `dir`, if any.
pub fn discover(dir: &Path) -> Option<PathBuf> {
    run(dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| !p.as_os_str().is_empty())
}

/// The status of the repository at `root`.
pub fn status(root: &Path) -> Result<Status, GitError> {
    let out = run_in(
        root,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ],
        LOCAL,
        &[],
    )?;
    Ok(parse_status(&out))
}

/// The last `n` commits (none before the first commit).
pub fn log(root: &Path, n: usize) -> Vec<Commit> {
    run(root, &["log", &format!("-n{n}"), LOG_FORMAT])
        .map(|s| parse_log(&s))
        .unwrap_or_default()
}

/// The repository's state, as the sidebar shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Repository root.
    pub root: PathBuf,
    /// Index and working tree.
    pub status: Status,
    /// Recent commits, newest first.
    pub log: Vec<Commit>,
}

/// Reads status and history of `root`.
pub fn snapshot(root: &Path) -> Result<Snapshot, GitError> {
    Ok(Snapshot {
        root: root.to_path_buf(),
        status: status(root)?,
        log: log(root, 40),
    })
}

/// A Source Control action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Just re-read the state.
    Refresh,
    /// `git init` in the folder (the root of the job).
    Init,
    /// Stage these paths (relative to the root).
    Stage(Vec<String>),
    /// Unstage these paths.
    Unstage(Vec<String>),
    /// Stage everything, untracked files included.
    StageAll,
    /// Unstage everything.
    UnstageAll,
    /// Throw away working-tree changes to these tracked paths.
    Discard(Vec<String>),
    /// Commit the index; `all` stages everything first.
    Commit {
        /// The message.
        message: String,
        /// Stage everything first (nothing was staged).
        all: bool,
    },
    /// `git pull`.
    Pull,
    /// `git push` (setting the upstream on the first push).
    Push,
    /// `git fetch`.
    Fetch,
    /// The diff of one file: `staged` for the index, else the working tree.
    Diff {
        /// Path relative to the root.
        path: String,
        /// The staged diff.
        staged: bool,
        /// The file is untracked (diff against nothing).
        untracked: bool,
    },
}

impl Op {
    /// What the sidebar shows while it runs.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Refresh => "Refreshing",
            Self::Init => "Initializing",
            Self::Stage(_) | Self::StageAll => "Staging",
            Self::Unstage(_) | Self::UnstageAll => "Unstaging",
            Self::Discard(_) => "Discarding",
            Self::Commit { .. } => "Committing",
            Self::Pull => "Pulling",
            Self::Push => "Pushing",
            Self::Fetch => "Fetching",
            Self::Diff { .. } => "Reading changes",
        }
    }
}

/// Performs `op` in `root` (no snapshot). Returns git's message or the diff, if any.
pub fn perform(root: &Path, op: &Op) -> Result<Option<String>, GitError> {
    let paths = |cmd: &[&str], ps: &[String]| -> Result<Option<String>, GitError> {
        let mut args: Vec<&str> = cmd.to_vec();
        args.push("--");
        args.extend(ps.iter().map(String::as_str));
        run(root, &args).map(|_| None)
    };
    match op {
        Op::Refresh => Ok(None),
        Op::Init => run(root, &["init"]).map(|_| None),
        Op::Stage(ps) => paths(&["add", "-A"], ps),
        Op::Unstage(ps) => {
            // Before the first commit there is no HEAD to reset to.
            if run(root, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
                paths(&["reset", "-q", "HEAD"], ps)
            } else {
                paths(&["rm", "--cached", "-r", "-q"], ps)
            }
        }
        Op::StageAll => run(root, &["add", "-A"]).map(|_| None),
        Op::UnstageAll => {
            if run(root, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
                run(root, &["reset", "-q"]).map(|_| None)
            } else {
                run(root, &["rm", "--cached", "-r", "-q", "."]).map(|_| None)
            }
        }
        Op::Discard(ps) => paths(&["checkout", "-q"], ps),
        Op::Commit { message, all } => {
            if message.trim().is_empty() {
                return Err(err("Enter a commit message first."));
            }
            if *all {
                run(root, &["add", "-A"])?;
            }
            run(root, &["commit", "-q", "-m", message]).map(|_| None)
        }
        Op::Pull => run_in(root, &["pull", "--no-edit"], NETWORK, &[]).map(|_| None),
        Op::Push => {
            let st = status(root)?;
            if st.upstream.is_some() {
                run_in(root, &["push"], NETWORK, &[]).map(|_| None)
            } else {
                let remotes = run(root, &["remote"])?;
                let Some(remote) = remotes.lines().map(str::trim).find(|r| !r.is_empty()) else {
                    return Err(err(
                        "This repository has no remote to push to. Add one with `git remote add origin <url>`.",
                    ));
                };
                run_in(root, &["push", "-u", remote, "HEAD"], NETWORK, &[]).map(|_| None)
            }
        }
        Op::Fetch => run_in(root, &["fetch", "--prune"], NETWORK, &[]).map(|_| None),
        Op::Diff {
            path,
            staged,
            untracked,
        } => {
            let out = if *untracked {
                let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
                run_in(
                    root,
                    &["diff", "--no-color", "--no-index", "--", null, path],
                    LOCAL,
                    &[1],
                )?
            } else if *staged {
                run_in(
                    root,
                    &["diff", "--no-color", "--cached", "--", path],
                    LOCAL,
                    &[],
                )?
            } else {
                run_in(root, &["diff", "--no-color", "--", path], LOCAL, &[])?
            };
            Ok(Some(String::from_utf8_lossy(&out).into_owned()))
        }
    }
}

/// The answer of a [`Job`].
#[derive(Debug, Clone)]
pub struct Outcome {
    /// The action.
    pub op: Op,
    /// Its result: git's output for diffs.
    pub result: Result<Option<String>, GitError>,
    /// The repository afterwards (`None` when it is not a repository).
    pub snapshot: Option<Snapshot>,
}

/// A running Source Control action.
pub struct Job {
    rx: Receiver<Outcome>,
}

impl Job {
    /// The outcome, once it is there.
    pub fn try_recv(&self) -> Option<Outcome> {
        self.rx.try_recv().ok()
    }
}

/// Runs `op` in the repository at (or containing) `dir` on a background thread, then reads a
/// fresh snapshot; calls `waker` when done.
pub fn start(dir: PathBuf, op: Op, waker: Arc<dyn Fn() + Send + Sync>) -> Job {
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new()
        .name("texcraft-git".into())
        .spawn(move || {
            let root = if op == Op::Init {
                Some(dir.clone())
            } else {
                discover(&dir)
            };
            let (result, snapshot) = match root {
                None => (Ok(None), None),
                Some(root) => {
                    let result = perform(&root, &op);
                    // After `init` the folder is the root.
                    let root = discover(&root).unwrap_or(root);
                    (result, snapshot(&root).ok())
                }
            };
            let _ = tx.send(Outcome {
                op,
                result,
                snapshot,
            });
            waker();
        });
    if let Err(e) = spawned {
        let (tx, rx2) = channel();
        let _ = tx.send(Outcome {
            op: Op::Refresh,
            result: Err(err(format!("cannot start git: {e}"))),
            snapshot: None,
        });
        return Job { rx: rx2 };
    }
    Job { rx }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2() {
        let out = b"# branch.oid 1234567890abcdef\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 M. N... 100644 100644 100644 aaa bbb staged.tex\0\
1 .M N... 100644 100644 100644 aaa bbb sections/edit.tex\0\
1 MM N... 100644 100644 100644 aaa bbb both.tex\0\
2 R. N... 100644 100644 100644 aaa bbb R100 new name.tex\0old name.tex\0\
u UU N... 100644 100644 100644 100644 a b c conflict.tex\0\
? figs/new plot.png\0";
        let st = parse_status(out);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.head.as_deref(), Some("1234567"));
        assert_eq!(st.upstream.as_deref(), Some("origin/main"));
        assert_eq!((st.ahead, st.behind), (2, 1));
        let staged: Vec<_> = st
            .staged
            .iter()
            .map(|c| (c.path.as_str(), c.kind))
            .collect();
        assert_eq!(
            staged,
            vec![
                ("staged.tex", ChangeKind::Modified),
                ("both.tex", ChangeKind::Modified),
                ("new name.tex", ChangeKind::Renamed),
            ]
        );
        assert_eq!(st.staged[2].orig.as_deref(), Some("old name.tex"));
        let changes: Vec<_> = st
            .changes
            .iter()
            .map(|c| (c.path.as_str(), c.kind))
            .collect();
        assert_eq!(
            changes,
            vec![
                ("sections/edit.tex", ChangeKind::Modified),
                ("both.tex", ChangeKind::Modified),
                ("conflict.tex", ChangeKind::Conflicted),
                ("figs/new plot.png", ChangeKind::Untracked),
            ]
        );
        // Before the first commit, detached heads and garbage.
        let st = parse_status(b"# branch.oid (initial)\0# branch.head (detached)\0\0garbage\0");
        assert_eq!((st.head.as_deref(), st.branch.as_deref()), (None, None));
        assert!(st.is_clean());
    }

    #[test]
    fn parses_log() {
        let out = "abc1234\x1fFirst line\x1fAda\x1f1700000000\x1e\ndef5678\x1fSecond\x1fBob\x1f1700000100\x1e";
        let log = parse_log(out);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].subject, "First line");
        assert_eq!(log[1].time, 1_700_000_100);
        assert!(parse_log("").is_empty());
    }

    /// Oracle test against the installed git: init, stage, commit, change, unstage, discard.
    #[test]
    fn works_with_real_git() {
        if !available() {
            eprintln!("skipping: no git");
            return;
        }
        let dir = std::env::temp_dir().join(format!("texcraft-git-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(discover(&dir).is_none() || discover(&dir) != Some(dir.clone()));
        perform(&dir, &Op::Init).unwrap();
        // A committer identity for the test repository only.
        run(&dir, &["config", "user.name", "TexCraft Test"]).unwrap();
        run(&dir, &["config", "user.email", "test@example.invalid"]).unwrap();
        run(&dir, &["config", "commit.gpgsign", "false"]).unwrap();
        let root = discover(&dir).unwrap();
        std::fs::write(dir.join("main.tex"), "Hello\n").unwrap();
        let st = status(&root).unwrap();
        assert_eq!(st.changes[0].kind, ChangeKind::Untracked);
        // Unstage before the first commit uses `rm --cached`.
        perform(&root, &Op::Stage(vec!["main.tex".into()])).unwrap();
        assert_eq!(status(&root).unwrap().staged[0].kind, ChangeKind::Added);
        perform(&root, &Op::Unstage(vec!["main.tex".into()])).unwrap();
        assert!(status(&root).unwrap().staged.is_empty());
        assert!(
            perform(
                &root,
                &Op::Commit {
                    message: " ".into(),
                    all: true
                }
            )
            .is_err(),
            "empty message"
        );
        perform(
            &root,
            &Op::Commit {
                message: "First".into(),
                all: true,
            },
        )
        .unwrap();
        let snap = snapshot(&root).unwrap();
        assert!(snap.status.is_clean());
        assert_eq!(snap.log[0].subject, "First");
        std::fs::write(dir.join("main.tex"), "Hello, world\n").unwrap();
        let diff = perform(
            &root,
            &Op::Diff {
                path: "main.tex".into(),
                staged: false,
                untracked: false,
            },
        )
        .unwrap()
        .unwrap();
        assert!(diff.contains("+Hello, world"), "{diff}");
        perform(&root, &Op::Discard(vec!["main.tex".into()])).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("main.tex")).unwrap(),
            "Hello\n"
        );
        std::fs::write(dir.join("new.tex"), "New\n").unwrap();
        let diff = perform(
            &root,
            &Op::Diff {
                path: "new.tex".into(),
                staged: false,
                untracked: true,
            },
        )
        .unwrap()
        .unwrap();
        assert!(diff.contains("+New"), "{diff}");
        // No remote: push explains instead of failing obscurely.
        let e = perform(&root, &Op::Push).unwrap_err();
        assert!(e.0.contains("no remote"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
