//! Updates: reads the release manifest (`latest.json`, written by the release workflow in
//! `.github/workflows/release.yml`), compares versions, downloads and verifies the archive for
//! this platform, unpacks it, and installs it after the app quits.
//!
//! Network and archive work use the system's tools, like the rest of TexCraft uses the TeX
//! programs: `curl` (macOS, Windows 10+), `ditto` (macOS) and `tar` (Windows 10+). Everything
//! runs on a background thread ([`Job`]); the UI polls its events.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The project's GitHub page (the update window links to it).
pub const REPO_URL: &str = "https://github.com/chenwen245299/TexCraft";

/// Where released builds publish their manifest (the newest non-pre-release).
pub const MANIFEST_URL: &str =
    "https://github.com/chenwen245299/TexCraft/releases/latest/download/latest.json";

/// An update error, shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateError(pub String);

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UpdateError {}

fn err(msg: impl Into<String>) -> UpdateError {
    UpdateError(msg.into())
}

/// The release manifest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    /// Version without the `v`, e.g. `0.2.0`.
    pub version: String,
    /// Release notes (the tag message).
    #[serde(default)]
    pub notes: String,
    /// Web page of the release.
    #[serde(default)]
    pub release_url: String,
    /// When it was published (RFC 3339).
    #[serde(default)]
    pub pub_date: String,
    /// Archives by platform key (see [`platform_key`]).
    #[serde(default)]
    pub platforms: BTreeMap<String, Asset>,
}

/// One downloadable archive.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asset {
    /// Download URL.
    pub url: String,
    /// Hex SHA-256 of the archive.
    pub sha256: String,
    /// Size in bytes.
    pub size: u64,
}

/// A newer release for this platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Version, e.g. `0.2.0`.
    pub version: String,
    /// Release notes.
    pub notes: String,
    /// Web page of the release.
    pub release_url: String,
    /// The archive for this platform.
    pub asset: Asset,
}

/// A downloaded, verified and unpacked update, ready to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    /// The release.
    pub release: Release,
    /// The new `TexCraft.app` (macOS) or `TexCraft` folder (Windows).
    pub path: PathBuf,
}

/// The manifest key of this build's archive, or `None` where TexCraft publishes no builds.
pub fn platform_key() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("macos")
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some("windows-x86_64")
    } else {
        None
    }
}

/// A semantic version: `MAJOR.MINOR.PATCH[-PRERELEASE]` (a leading `v` is allowed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// Numbers.
    pub core: (u64, u64, u64),
    /// Pre-release suffix (sorts before the release).
    pub pre: Option<String>,
}

impl Version {
    /// Parses `text`, or `None` if it is not a version.
    pub fn parse(text: &str) -> Option<Self> {
        let t = text.trim().trim_start_matches('v');
        let (core, pre) = match t.split_once('-') {
            Some((c, p)) => (c, Some(p.to_string())),
            None => (t, None),
        };
        let mut it = core.split('.').map(|n| n.parse::<u64>().ok());
        let v = (it.next()??, it.next()??, it.next()??);
        if it.next().is_some() {
            return None;
        }
        Some(Self { core: v, pre })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::{Equal, Greater, Less};
        match self.core.cmp(&other.core) {
            Equal => match (&self.pre, &other.pre) {
                (None, None) => Equal,
                (None, Some(_)) => Greater,
                (Some(_), None) => Less,
                (Some(a), Some(b)) => a.cmp(b),
            },
            o => o,
        }
    }
}

/// The release in `manifest` if it is newer than `current` and has an archive for `platform`.
pub fn newer_release(manifest: &Manifest, current: &str, platform: &str) -> Option<Release> {
    let new = Version::parse(&manifest.version)?;
    let cur = Version::parse(current)?;
    if new <= cur {
        return None;
    }
    let asset = manifest.platforms.get(platform)?.clone();
    Some(Release {
        version: manifest.version.trim_start_matches('v').to_string(),
        notes: manifest.notes.clone(),
        release_url: manifest.release_url.clone(),
        asset,
    })
}

/// Parses a manifest.
pub fn parse_manifest(json: &str) -> Result<Manifest, UpdateError> {
    serde_json::from_str(json).map_err(|e| err(format!("The update information is invalid: {e}")))
}

/// `curl` (on `PATH` on macOS and Windows 10+).
fn curl() -> Command {
    let program = if cfg!(windows) {
        "curl.exe"
    } else if Path::new("/usr/bin/curl").exists() {
        "/usr/bin/curl"
    } else {
        "curl"
    };
    let mut c = Command::new(program);
    no_window(&mut c);
    c
}

/// Keeps helper processes from flashing a console window on Windows.
fn no_window(c: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = c;
}

/// Fetches and parses the manifest at `url` (`https://…`, `http://127.0.0.1…` or a local path
/// for tests).
pub fn fetch_manifest(url: &str) -> Result<Manifest, UpdateError> {
    if let Some(path) = local_path(url) {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| err(format!("Cannot read {}: {e}", path.display())))?;
        return parse_manifest(&text);
    }
    let out = curl()
        .args(["-fsSL", "--max-time", "30", "--retry", "1", url])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| err(format!("Cannot run curl to check for updates: {e}")))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if msg.contains("404") {
            return Err(err("No TexCraft release has been published yet."));
        }
        return Err(err(if msg.is_empty() {
            "Cannot reach the update server.".to_string()
        } else {
            format!("Cannot reach the update server: {msg}")
        }));
    }
    parse_manifest(&String::from_utf8_lossy(&out.stdout))
}

fn local_path(url: &str) -> Option<PathBuf> {
    if let Some(rest) = url.strip_prefix("file://") {
        return Some(PathBuf::from(rest));
    }
    (!url.contains("://")).then(|| PathBuf::from(url))
}

/// Hex SHA-256 of a file.
pub fn sha256_file(path: &Path) -> Result<String, UpdateError> {
    let mut f = std::fs::File::open(path)
        .map_err(|e| err(format!("Cannot open {}: {e}", path.display())))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| err(format!("Cannot read {}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        h.update(buf.get(..n).unwrap_or(&[]));
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// What a background update job reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The check finished: a newer release, or none.
    Checked(Option<Release>),
    /// Bytes downloaded so far, of `total`.
    Progress {
        /// Downloaded.
        done: u64,
        /// Expected size.
        total: u64,
    },
    /// Downloaded, verified and unpacked.
    Ready(Staged),
    /// Something went wrong.
    Failed(String),
}

/// A background check or download.
pub struct Job {
    rx: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// The next event, if any; `None` when nothing is pending.
    pub fn try_recv(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }

    /// Asks the job to stop (a download is killed).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

fn spawn_job(
    work: impl FnOnce(&Sender<Event>, &AtomicBool) + Send + 'static,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Job {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let spawned = std::thread::Builder::new()
        .name("texcraft-update".into())
        .spawn(move || {
            work(&tx, &flag);
            wake();
        });
    if spawned.is_err() {
        let (tx, rx2) = channel();
        let _ = tx.send(Event::Failed("Cannot start the update thread.".into()));
        return Job { rx: rx2, cancel };
    }
    Job { rx, cancel }
}

/// Checks `url` for a release newer than `current` for this platform.
pub fn start_check(url: String, current: String, wake: Arc<dyn Fn() + Send + Sync>) -> Job {
    spawn_job(
        move |tx, _| {
            let event = match (fetch_manifest(&url), platform_key()) {
                (Ok(m), Some(key)) => Event::Checked(newer_release(&m, &current, key)),
                (Ok(_), None) => Event::Checked(None),
                (Err(e), _) => Event::Failed(e.0),
            };
            let _ = tx.send(event);
        },
        wake,
    )
}

/// Downloads `release` into `dir`, verifies it and unpacks it.
pub fn start_download(release: Release, dir: PathBuf, wake: Arc<dyn Fn() + Send + Sync>) -> Job {
    let progress_wake = wake.clone();
    spawn_job(
        move |tx, cancel| {
            let event = match download(&release, &dir, cancel, &|done, total| {
                let _ = tx.send(Event::Progress { done, total });
                progress_wake();
            }) {
                Ok(staged) => Event::Ready(staged),
                Err(e) => Event::Failed(e.0),
            };
            let _ = tx.send(event);
        },
        wake,
    )
}

/// Downloads, verifies (size and SHA-256) and unpacks `release` into `dir`.
pub fn download(
    release: &Release,
    dir: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<Staged, UpdateError> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir)
        .map_err(|e| err(format!("Cannot create {}: {e}", dir.display())))?;
    let archive = dir.join("update.zip");
    let total = release.asset.size;
    if let Some(src) = local_path(&release.asset.url) {
        std::fs::copy(&src, &archive)
            .map_err(|e| err(format!("Cannot copy {}: {e}", src.display())))?;
        progress(total, total);
    } else {
        let mut child = curl()
            .args(["-fsSL", "--retry", "2", "--connect-timeout", "20", "-o"])
            .arg(&archive)
            .arg(&release.asset.url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| err(format!("Cannot run curl to download the update: {e}")))?;
        let status = loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(err("The download was cancelled."));
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    let done = std::fs::metadata(&archive).map_or(0, |m| m.len());
                    progress(done.min(total), total);
                    std::thread::sleep(Duration::from_millis(150));
                }
                Err(e) => return Err(err(format!("The download failed: {e}"))),
            }
        };
        if !status.success() {
            let mut msg = String::new();
            if let Some(mut s) = child.stderr.take() {
                let _ = s.read_to_string(&mut msg);
            }
            return Err(err(format!("The download failed: {}", msg.trim())));
        }
        progress(total, total);
    }
    let size = std::fs::metadata(&archive).map_or(0, |m| m.len());
    if size != total {
        return Err(err(format!(
            "The download is incomplete ({size} of {total} bytes)."
        )));
    }
    let sum = sha256_file(&archive)?;
    if !sum.eq_ignore_ascii_case(release.asset.sha256.trim()) {
        return Err(err("The download is damaged (checksum mismatch)."));
    }
    let path = unpack(&archive, dir)?;
    Ok(Staged {
        release: release.clone(),
        path,
    })
}

/// Unpacks `archive` into `dir` and returns the app inside it: `TexCraft.app` (macOS) or the
/// `TexCraft` folder holding `texcraft.exe` (Windows).
pub fn unpack(archive: &Path, dir: &Path) -> Result<PathBuf, UpdateError> {
    let out = dir.join("unpacked");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out)
        .map_err(|e| err(format!("Cannot create {}: {e}", out.display())))?;
    let mut c = if cfg!(target_os = "macos") {
        let mut c = Command::new("/usr/bin/ditto");
        c.args(["-x", "-k"]).arg(archive).arg(&out);
        c
    } else {
        let mut c = Command::new("tar");
        c.arg("-xf").arg(archive).arg("-C").arg(&out);
        c
    };
    no_window(&mut c);
    let status = c
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| err(format!("Cannot unpack the update: {e}")))?;
    if !status.success() {
        return Err(err("Cannot unpack the update."));
    }
    let app = if cfg!(target_os = "macos") {
        out.join("TexCraft.app")
    } else {
        out.join("TexCraft")
    };
    let ok = if cfg!(target_os = "macos") {
        app.join("Contents")
            .join("MacOS")
            .join("texcraft")
            .is_file()
    } else {
        app.join("texcraft.exe").is_file()
    };
    if ok {
        Ok(app)
    } else {
        Err(err("The update archive does not contain TexCraft."))
    }
}

/// Where this copy of TexCraft is installed: the `.app` bundle (macOS) or the folder holding
/// the executable (Windows). `None` for a development build (not inside an app bundle).
pub fn install_location(exe: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        exe.ancestors()
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .map(Path::to_path_buf)
    } else {
        exe.parent().map(Path::to_path_buf)
    }
}

/// The shell script that waits for process `$1` to exit, replaces `$2` with `$3` and, if `$4` is
/// 1, opens the new app (macOS).
pub const MACOS_INSTALLER: &str = r#"pid="$1"; old="$2"; new="$3"; relaunch="$4"
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
rm -rf "$old.previous"
if mv "$old" "$old.previous" && mv "$new" "$old"; then
  rm -rf "$old.previous"
  xattr -dr com.apple.quarantine "$old" 2>/dev/null
else
  [ -d "$old.previous" ] && [ ! -d "$old" ] && mv "$old.previous" "$old"
fi
[ "$relaunch" = 1 ] && open "$old"
exit 0
"#;

/// The batch file doing the same on Windows: wait for `%1`, copy `%3` over `%2`, start it if
/// `%4` is 1.
pub const WINDOWS_INSTALLER: &str = "@echo off\r\n\
:wait\r\n\
tasklist /FI \"PID eq %1\" /NH 2>NUL | find \" %1 \" >NUL\r\n\
if not errorlevel 1 (timeout /t 1 /nobreak >NUL & goto wait)\r\n\
robocopy \"%~3\" \"%~2\" /E /NFL /NDL /NJH /NJS /NP >NUL\r\n\
if \"%4\"==\"1\" start \"\" \"%~2\\texcraft.exe\"\r\n\
del \"%~f0\"\r\n";

/// Starts a helper that installs `staged` over the running copy once this process exits, and
/// relaunches it if `relaunch`. The caller quits afterwards (or installs on its next quit).
pub fn install(staged: &Staged, relaunch: bool) -> Result<(), UpdateError> {
    let exe =
        std::env::current_exe().map_err(|e| err(format!("Cannot find TexCraft itself: {e}")))?;
    let target = install_location(&exe).ok_or_else(|| {
        err("This copy of TexCraft is not an installed app (a development build); download the new version from the release page.")
    })?;
    let parent = target.parent().unwrap_or(&target);
    let probe = parent.join(".texcraft-write-test");
    if std::fs::write(&probe, b"").is_err() {
        return Err(err(format!(
            "TexCraft cannot replace itself in {} (no permission); download the new version from the release page.",
            parent.display()
        )));
    }
    let _ = std::fs::remove_file(&probe);
    let pid = std::process::id().to_string();
    let flag = if relaunch { "1" } else { "0" };
    let mut c = if cfg!(windows) {
        let script = std::env::temp_dir().join(format!("texcraft-update-{pid}.cmd"));
        std::fs::write(&script, WINDOWS_INSTALLER)
            .map_err(|e| err(format!("Cannot write the installer: {e}")))?;
        let mut c = Command::new("cmd");
        c.arg("/C")
            .arg(&script)
            .arg(&pid)
            .arg(&target)
            .arg(&staged.path)
            .arg(flag);
        c
    } else {
        let mut c = Command::new("/bin/sh");
        c.arg("-c")
            .arg(MACOS_INSTALLER)
            .arg("texcraft-update")
            .arg(&pid)
            .arg(&target)
            .arg(&staged.path)
            .arg(flag);
        c
    };
    no_window(&mut c);
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| err(format!("Cannot start the installer: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str) -> Manifest {
        parse_manifest(&format!(
            r#"{{"version":"{version}","notes":"Faster.","release_url":"https://example.com/r",
               "platforms":{{"macos":{{"url":"https://example.com/m.zip","sha256":"ab","size":3}},
                            "windows-x86_64":{{"url":"https://example.com/w.zip","sha256":"cd","size":4}}}}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn versions_compare() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("0.2.0") > v("0.1.9"));
        assert!(v("v1.0.0") > v("0.99.99"));
        assert!(v("0.2.0") > v("0.2.0-beta.1"));
        assert!(v("0.2.0-beta.2") > v("0.2.0-beta.1"));
        assert_eq!(v("0.1.0"), v("v0.1.0"));
        assert!(Version::parse("0.1").is_none());
        assert!(Version::parse("1.2.3.4").is_none());
        assert!(Version::parse("x.y.z").is_none());
    }

    #[test]
    fn picks_newer_release_for_the_platform() {
        let m = manifest("0.2.0");
        let r = newer_release(&m, "0.1.0", "macos").unwrap();
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.asset.url, "https://example.com/m.zip");
        assert_eq!(r.notes, "Faster.");
        assert!(newer_release(&m, "0.2.0", "macos").is_none());
        assert!(newer_release(&m, "0.3.0", "macos").is_none());
        assert!(newer_release(&m, "0.1.0", "linux").is_none());
        assert!(parse_manifest("{oops").is_err());
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("texcraft-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn local_manifest_and_checksums() {
        let d = temp("manifest");
        let m = d.join("latest.json");
        std::fs::write(&m, r#"{"version":"9.9.9","platforms":{}}"#).unwrap();
        assert_eq!(
            fetch_manifest(m.to_str().unwrap()).unwrap().version,
            "9.9.9"
        );
        assert_eq!(
            fetch_manifest(&format!("file://{}", m.display()))
                .unwrap()
                .version,
            "9.9.9"
        );
        let f = d.join("x");
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(
            sha256_file(&f).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_dir_all(&d).ok();
    }

    /// A real archive made with ditto, fetched from a file:// URL, verified and unpacked; then
    /// the installer swaps it in place once a given process exits.
    #[cfg(target_os = "macos")]
    #[test]
    fn downloads_unpacks_and_installs_a_bundle() {
        let d = temp("bundle");
        let app = d.join("src").join("TexCraft.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/MacOS/texcraft"), b"new").unwrap();
        let zip = d.join("TexCraft.zip");
        let ok = Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(&app)
            .arg(&zip)
            .status()
            .unwrap();
        assert!(ok.success());
        let release = Release {
            version: "9.9.9".into(),
            notes: String::new(),
            release_url: String::new(),
            asset: Asset {
                url: format!("file://{}", zip.display()),
                sha256: sha256_file(&zip).unwrap(),
                size: std::fs::metadata(&zip).unwrap().len(),
            },
        };
        let staged =
            download(&release, &d.join("dl"), &AtomicBool::new(false), &|_, _| {}).unwrap();
        assert!(staged.path.join("Contents/MacOS/texcraft").is_file());
        let mut bad = release.clone();
        bad.asset.sha256 = "00".into();
        assert!(download(&bad, &d.join("dl2"), &AtomicBool::new(false), &|_, _| {}).is_err());

        // The installer script: waits for the process, swaps the bundle, keeps no backup.
        let installed = d.join("Applications").join("TexCraft.app");
        std::fs::create_dir_all(installed.join("Contents/MacOS")).unwrap();
        std::fs::write(installed.join("Contents/MacOS/texcraft"), b"old").unwrap();
        // Stand-in for the app: reaped on another thread so it does not linger as a zombie
        // (the real app is reaped by launchd).
        let mut sleeper = Command::new("sleep").arg("0.3").spawn().unwrap();
        let pid = sleeper.id().to_string();
        let reaper = std::thread::spawn(move || sleeper.wait());
        let status = Command::new("/bin/sh")
            .arg("-c")
            .arg(MACOS_INSTALLER)
            .arg("texcraft-update")
            .arg(&pid)
            .arg(&installed)
            .arg(&staged.path)
            .arg("0")
            .status()
            .unwrap();
        let _ = reaper.join();
        assert!(status.success());
        assert_eq!(
            std::fs::read(installed.join("Contents/MacOS/texcraft")).unwrap(),
            b"new"
        );
        assert!(!d.join("Applications/TexCraft.app.previous").exists());
        assert_eq!(
            install_location(&installed.join("Contents/MacOS/texcraft")),
            Some(installed.clone())
        );
        std::fs::remove_dir_all(&d).ok();
    }
}
