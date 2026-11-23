//! One window per project, across the independent window processes. A window holds an
//! exclusive lock on `<key hash>.lock` in TexCraft's data folder while it shows that project
//! (the key is [`texcraft_engine::project_key`]); the system lets go of it when the process
//! ends, however it ends, so a crashed window never keeps its project. A request to open the
//! project elsewhere goes to `<key hash>.inbox` instead: the owner opens those paths and comes
//! to the front.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Paths handed over longer ago than this are not opened any more (nobody took them in time).
const INBOX_FRESH: Duration = Duration::from_secs(60);

/// FNV-1a: the same file name from one build to the next.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn file(dir: &Path, key: &Path, extension: &str) -> PathBuf {
    dir.join(format!("{:016x}.{extension}", fnv(&key.to_string_lossy())))
}

/// This window's hold on its project.
pub struct Registry {
    dir: PathBuf,
    /// The key held, and the locked file that holds it.
    held: Option<(PathBuf, File)>,
    /// The key last asked for (held or not), so a project another window took is not asked for
    /// again every frame.
    wanted: Option<PathBuf>,
    /// Held by the one window that checks for updates by itself (and downloads them).
    updates: Option<File>,
}

impl Registry {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            held: None,
            wanted: None,
            updates: None,
        }
    }

    /// Whether this window is the one that checks for updates by itself: one check and one
    /// download for the app, however many windows are open. When it quits, another takes
    /// over (ask again now and then).
    pub fn hold_updates(&mut self) -> bool {
        if self.updates.is_some() {
            return true;
        }
        if std::fs::create_dir_all(&self.dir).is_err() {
            return true;
        }
        let Ok(lock) = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join("updates.lock"))
        else {
            return true;
        };
        match lock.try_lock() {
            Ok(()) => {
                self.updates = Some(lock);
                true
            }
            Err(TryLockError::WouldBlock) => false,
            Err(TryLockError::Error(_)) => true,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The project this window holds.
    pub fn held(&self) -> Option<&Path> {
        self.held.as_ref().map(|(key, _)| key.as_path())
    }

    /// Keeps this window's hold in step with the project it shows (`None`: none). Returns
    /// whether it now holds `key`; `false` means another window took it first.
    pub fn follow(&mut self, key: Option<&Path>) -> bool {
        if self.wanted.as_deref() == key {
            return self.held() == key;
        }
        self.wanted = key.map(Path::to_path_buf);
        self.hold(key)
    }

    /// Makes this window the one showing `key`, letting go of what it held. `false`: another
    /// window shows it. Without a working data folder every window is on its own, as before.
    pub fn hold(&mut self, key: Option<&Path>) -> bool {
        self.wanted = key.map(Path::to_path_buf);
        if self.held() == key {
            return true;
        }
        // Dropping the file releases its lock.
        self.held = None;
        let Some(key) = key else {
            return true;
        };
        if std::fs::create_dir_all(&self.dir).is_err() {
            return true;
        }
        let Ok(mut lock) = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(file(&self.dir, key, "lock"))
        else {
            return true;
        };
        // Another process checking who holds it locks it for an instant: try a few times.
        for attempt in 0..5 {
            match lock.try_lock() {
                Ok(()) => {
                    let _ = lock.set_len(0);
                    let _ = write!(lock, "{}\n{}\n", std::process::id(), key.display());
                    self.held = Some((key.to_path_buf(), lock));
                    return true;
                }
                Err(TryLockError::WouldBlock) if attempt < 4 => {
                    std::thread::sleep(Duration::from_millis(15));
                }
                Err(TryLockError::WouldBlock) => return false,
                Err(TryLockError::Error(_)) => return true,
            }
        }
        false
    }

    /// The other process that shows `key`, if one does.
    pub fn owner(dir: &Path, key: &Path) -> Option<u32> {
        let mut lock = File::open(file(dir, key, "lock")).ok()?;
        match lock.try_lock() {
            // Nobody holds it (the lock goes with the file).
            Ok(()) => None,
            Err(TryLockError::WouldBlock) => {
                let mut text = String::new();
                lock.read_to_string(&mut text).ok()?;
                let pid: u32 = text.lines().next()?.trim().parse().ok()?;
                (pid != std::process::id()).then_some(pid)
            }
            Err(TryLockError::Error(_)) => None,
        }
    }

    /// Hands `path` to the window that shows (or is about to show) `key`.
    pub fn forward(dir: &Path, key: &Path, path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut inbox = OpenOptions::new()
            .create(true)
            .append(true)
            .open(file(dir, key, "inbox"))?;
        // One short line per write: appends of this size do not interleave.
        writeln!(inbox, "{}", path.display())
    }

    /// The inbox of the project this window holds.
    pub fn inbox(&self) -> Option<PathBuf> {
        self.held().map(|key| file(&self.dir, key, "inbox"))
    }

    /// Paths other windows handed to this one since the last call.
    pub fn take_inbox(&self) -> Vec<PathBuf> {
        let Some(inbox) = self.inbox() else {
            return Vec::new();
        };
        if !inbox.is_file() {
            return Vec::new();
        }
        let taken = inbox.with_extension(format!("taking-{}", std::process::id()));
        if std::fs::rename(&inbox, &taken).is_err() {
            return Vec::new();
        }
        let fresh = std::fs::metadata(&taken)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_none_or(|age| age <= INBOX_FRESH);
        let text = std::fs::read_to_string(&taken).unwrap_or_default();
        let _ = std::fs::remove_file(&taken);
        if !fresh {
            return Vec::new();
        }
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(PathBuf::from)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("texcraft-windows-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// One holder per project; the hold ends with the window (or its project); paths handed
    /// over reach the holder once.
    #[test]
    fn one_window_holds_a_project_and_receives_its_requests() {
        let dir = dir("hold");
        let paper = Path::new("/projects/paper/main.tex");
        let thesis = Path::new("/projects/thesis/main.tex");
        let mut first = Registry::new(dir.clone());
        let mut second = Registry::new(dir.clone());
        assert!(first.follow(Some(paper)));
        assert!(
            !second.follow(Some(paper)),
            "a second window cannot take it"
        );
        assert_eq!(second.held(), None);
        // Another project is free.
        assert!(second.follow(Some(thesis)));
        // Within one process the holder's own key is nobody else's.
        assert_eq!(Registry::owner(&dir, paper), None);

        Registry::forward(&dir, paper, Path::new("/projects/paper/intro.tex")).unwrap();
        Registry::forward(&dir, paper, Path::new("/projects/paper/main.tex")).unwrap();
        assert_eq!(
            first.take_inbox(),
            vec![
                PathBuf::from("/projects/paper/intro.tex"),
                PathBuf::from("/projects/paper/main.tex")
            ]
        );
        assert!(first.take_inbox().is_empty(), "each request is taken once");
        assert!(
            second.take_inbox().is_empty(),
            "nothing for another project"
        );

        // The window closes its project: another one may show it now.
        assert!(first.follow(None));
        assert!(second.hold(Some(paper)));
        drop(second);
        // A window that ends (or crashes) lets go of everything.
        let mut third = Registry::new(dir.clone());
        assert!(third.hold(Some(paper)) && third.hold(Some(thesis)));
        // One window checks for updates; another takes over once it has gone.
        assert!(first.hold_updates() && first.hold_updates());
        assert!(!third.hold_updates());
        drop(first);
        assert!(third.hold_updates());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The lock is the system's: a TexCraft process that is gone never holds a project, and
    /// one that runs does (seen from another process).
    #[test]
    fn a_holder_in_another_process_is_found_and_a_dead_one_is_not() {
        let dir = dir("process");
        let key = Path::new("/projects/paper/main.tex");
        // A stale file from a crashed window (no lock held).
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            file(&dir, key, "lock"),
            "999999\n/projects/paper/main.tex\n",
        )
        .unwrap();
        assert_eq!(Registry::owner(&dir, key), None);
        // `flock` from a child process holds it while the child runs.
        let lock = file(&dir, key, "lock");
        std::fs::write(&lock, "4242\n/projects/paper/main.tex\n").unwrap();
        let Ok(mut child) = std::process::Command::new("/usr/bin/python3")
            .args([
                "-c",
                "import fcntl,sys,time;f=open(sys.argv[1]);fcntl.flock(f,fcntl.LOCK_EX);print('locked',flush=True);time.sleep(30)",
            ])
            .arg(&lock)
            .stdout(std::process::Stdio::piped())
            .spawn()
        else {
            eprintln!("skipping: no python3 to hold a lock from another process");
            return;
        };
        let mut line = String::new();
        if let Some(out) = child.stdout.as_mut() {
            use std::io::BufRead;
            let _ = std::io::BufReader::new(out).read_line(&mut line);
        }
        assert_eq!(line.trim(), "locked");
        assert_eq!(Registry::owner(&dir, key), Some(4242));
        let mut mine = Registry::new(dir.clone());
        assert!(!mine.hold(Some(key)));
        let _ = child.kill();
        let _ = child.wait();
        assert!(mine.hold(Some(key)), "free once that process is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
