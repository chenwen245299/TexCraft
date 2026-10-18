//! Software updates for the session: checks the release manifest in the background (at every
//! start-up and every few hours when allowed), downloads a newer release automatically or on
//! request, and installs it on quit or right away (see `texcraft-update`). A newer release found
//! at start-up opens the update window straight away (downloading meanwhile); one found by a
//! later check opens it once downloaded.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use texcraft_i18n::tr;
pub use texcraft_update::REPO_URL;
use texcraft_update::{Event, Job, Release, Staged};

use crate::{CommandError, Effect, Session};

/// This build's version.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// How often automatic checks run.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// The first automatic check waits this long after start-up.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(3);

/// Where the update process stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    /// Nothing happening.
    #[default]
    Idle,
    /// Reading the manifest.
    Checking,
    /// The last check found nothing newer.
    UpToDate,
    /// A newer release is available.
    Available(Release),
    /// Downloading a release.
    Downloading {
        /// The release.
        release: Release,
        /// Bytes so far.
        done: u64,
        /// Expected size.
        total: u64,
    },
    /// Downloaded and verified: installs on quit, or now on request.
    Ready(Staged),
    /// The last check or download failed.
    Failed(String),
}

/// Update state of a session.
pub struct UpdateState {
    /// Current status.
    pub status: UpdateStatus,
    job: Option<Job>,
    /// Network checks are allowed (the app turns this on; tests and the CLI stay offline).
    pub enabled: bool,
    /// The manifest URL.
    pub url: String,
    /// The user asked for the current check (report "up to date" and errors too).
    pub interactive: bool,
    /// The current check is the one at start-up (a newer release opens the window at once).
    at_launch: bool,
    /// The start-up check has been started.
    launch_checked: bool,
    started: Instant,
}

impl Default for UpdateState {
    fn default() -> Self {
        Self {
            status: UpdateStatus::Idle,
            job: None,
            enabled: false,
            url: texcraft_update::MANIFEST_URL.to_string(),
            interactive: false,
            at_launch: false,
            launch_checked: false,
            started: Instant::now(),
        }
    }
}

impl UpdateState {
    /// A check or download is running.
    pub fn is_busy(&self) -> bool {
        self.job.is_some()
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Session {
    /// Looks for a newer release. `interactive`: the user asked (the update window opens and
    /// reports the result whatever it is).
    pub fn check_for_updates(&mut self, interactive: bool) {
        if interactive {
            self.update.interactive = true;
            self.effect(Effect::UpdateWindow);
        }
        if self.update.job.is_some() || matches!(self.update.status, UpdateStatus::Ready(_)) {
            return;
        }
        self.update.status = UpdateStatus::Checking;
        self.update.job = Some(texcraft_update::start_check(
            self.update.url.clone(),
            CURRENT_VERSION.to_string(),
            self.waker.clone(),
        ));
    }

    /// Downloads the available release.
    pub fn download_update(&mut self) -> Result<(), CommandError> {
        let UpdateStatus::Available(release) = self.update.status.clone() else {
            return Err(CommandError::Failed(tr("no update is available").into()));
        };
        let dir = std::env::temp_dir().join(format!("texcraft-update-{}", release.version));
        self.update.status = UpdateStatus::Downloading {
            total: release.asset.size,
            release: release.clone(),
            done: 0,
        };
        self.update.job = Some(texcraft_update::start_download(
            release,
            dir,
            self.waker.clone(),
        ));
        Ok(())
    }

    /// Installs the downloaded release: `relaunch` quits now and starts the new version;
    /// otherwise it is installed when TexCraft quits.
    pub fn install_update(&mut self, relaunch: bool) -> Result<(), CommandError> {
        let UpdateStatus::Ready(staged) = &self.update.status else {
            return Err(CommandError::Failed(
                tr("no update is ready to install").into(),
            ));
        };
        if relaunch {
            texcraft_update::install(staged, true).map_err(|e| CommandError::Failed(e.0))?;
            // The installer waits for this process; it is installed even if quitting is
            // postponed by a save prompt.
            self.update.status = UpdateStatus::Idle;
            self.effect(Effect::Quit);
        }
        Ok(())
    }

    /// Called when the app quits: installs a downloaded update (without relaunching). Skipping the
    /// update clears it, so this only installs what the user did not turn down.
    pub fn install_update_on_quit(&mut self) {
        if let UpdateStatus::Ready(staged) = &self.update.status
            && let Err(e) = texcraft_update::install(staged, false)
        {
            self.set_status(e.0);
        }
    }

    /// Hides the available release until a newer one appears.
    pub fn skip_update(&mut self) {
        let version = match &self.update.status {
            UpdateStatus::Available(r) => Some(r.version.clone()),
            UpdateStatus::Ready(s) => Some(s.release.version.clone()),
            UpdateStatus::Downloading { release, .. } => Some(release.version.clone()),
            _ => None,
        };
        if let Some(v) = version {
            self.view.skipped_update = Some(v);
        }
        self.cancel_update();
        self.update.status = UpdateStatus::Idle;
    }

    /// Stops a running check or download.
    pub fn cancel_update(&mut self) {
        if let Some(job) = self.update.job.take() {
            job.cancel();
        }
        self.update.interactive = false;
        if let UpdateStatus::Downloading { release, .. } = &self.update.status {
            self.update.status = UpdateStatus::Available(release.clone());
        } else if self.update.status == UpdateStatus::Checking {
            self.update.status = UpdateStatus::Idle;
        }
    }

    /// Polls the update job and starts automatic checks. Returns when to look again.
    pub(crate) fn tick_updates(&mut self, now: Instant) -> Option<Duration> {
        let events: Vec<Event> = self
            .update
            .job
            .as_ref()
            .map(|job| std::iter::from_fn(|| job.try_recv()).collect())
            .unwrap_or_default();
        for event in events {
            self.on_update_event(event);
        }
        if !self.update.enabled || !self.view.auto_check_updates || self.update.job.is_some() {
            return None;
        }
        if matches!(
            self.update.status,
            UpdateStatus::Ready(_) | UpdateStatus::Available(_)
        ) {
            return None;
        }
        let since_start = now.saturating_duration_since(self.update.started);
        if since_start < FIRST_CHECK_AFTER {
            return Some(FIRST_CHECK_AFTER - since_start);
        }
        // Every start-up checks, however recently the last check ran.
        if !self.update.launch_checked {
            self.update.launch_checked = true;
            self.update.at_launch = true;
            self.check_for_updates(false);
            return None;
        }
        let since_check = now_secs().saturating_sub(self.view.last_update_check);
        if since_check >= CHECK_EVERY.as_secs() {
            self.check_for_updates(false);
            None
        } else {
            Some(Duration::from_secs(CHECK_EVERY.as_secs() - since_check))
        }
    }

    fn on_update_event(&mut self, event: Event) {
        match event {
            Event::Checked(found) => {
                self.update.job = None;
                self.view.last_update_check = now_secs();
                let interactive = std::mem::take(&mut self.update.interactive);
                let at_launch = std::mem::take(&mut self.update.at_launch);
                match found {
                    Some(r)
                        if interactive
                            || self.view.skipped_update.as_deref() != Some(r.version.as_str()) =>
                    {
                        self.update.status = UpdateStatus::Available(r);
                        if self.view.auto_download_updates && !interactive {
                            // In the background; a later check's window opens once it is
                            // ready to install, the start-up one's right away (with progress).
                            if let Err(e) = self.download_update() {
                                self.set_status(e.to_string());
                            }
                            if at_launch {
                                self.effect(Effect::UpdateWindow);
                            }
                        } else {
                            self.update.interactive = interactive;
                            self.effect(Effect::UpdateWindow);
                        }
                    }
                    _ => {
                        self.update.status = UpdateStatus::UpToDate;
                    }
                }
            }
            Event::Progress { done, total } => {
                if let UpdateStatus::Downloading {
                    done: d, total: t, ..
                } = &mut self.update.status
                {
                    *d = done;
                    *t = total;
                }
            }
            Event::Ready(staged) => {
                self.update.job = None;
                self.update.status = UpdateStatus::Ready(staged);
                self.effect(Effect::UpdateWindow);
            }
            Event::Failed(msg) => {
                // Quiet unless the user asked: the update window (or Settings → General) shows
                // it; a background check that cannot reach the server is not news.
                self.update.job = None;
                self.update.status = UpdateStatus::Failed(msg);
            }
        }
    }
}
