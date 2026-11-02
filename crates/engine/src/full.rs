//! Full typesets TexCraft starts by itself. Live passes typeset the unsaved
//! buffers into a preview, but the PDF beside the source (the one Finder, mail and other apps
//! see) only changes with a full typeset. So a project edited since its last full typeset is
//! typeset in full now and then ([`FULL_TYPESET_EVERY`] after the first such edit, at a pause in
//! typing). Before quitting, every open project gets a full rebuild in every mode.
//!
//! A full typeset reads the files on disk, so like ⌘T it saves the open documents first when
//! "save before typesetting" is on. Before quitting it runs after the unsaved-changes prompt:
//! the PDF then matches what was saved.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use texcraft_project::TypesetMode;

use crate::{Effect, Session, TypesetState};

/// A project edited this long ago gets a full typeset at the next pause in typing.
pub const FULL_TYPESET_EVERY: Duration = Duration::from_secs(5 * 60);
/// The pause in typing a background full typeset waits for (it holds up live passes while it
/// runs)…
pub const FULL_TYPESET_PAUSE: Duration = Duration::from_secs(5);
/// …unless the first edit it covers is this old.
pub const FULL_TYPESET_LATEST: Duration = Duration::from_secs(10 * 60);
/// A live pass that runs this long (or four times its usual time, if that is longer) while
/// newer edits wait is stopped, and the next pass typesets them.
pub const LIVE_STUCK: Duration = Duration::from_secs(20);

/// Typesetting before quitting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum QuitTypeset {
    /// Not quitting.
    #[default]
    Idle,
    /// Typesetting these roots one after another (the one running is already off the list),
    /// then quitting.
    Running(Vec<PathBuf>),
    /// Compilation or publishing failed: keep the window open for Retry / Don't Quit.
    Failed {
        /// Project whose PDF could not be updated.
        root: PathBuf,
        /// Failure detail in the interface language.
        message: String,
    },
    /// Every full rebuild and PDF publication succeeded: the window may close.
    Done,
}

impl TypesetState {
    /// TexCraft is typesetting before it quits.
    pub fn is_quitting(&self) -> bool {
        matches!(
            self.quit,
            QuitTypeset::Running(_) | QuitTypeset::Failed { .. }
        )
    }
}

/// `t + d`, or `t` if that is not representable.
fn after(t: Instant, d: Duration) -> Instant {
    t.checked_add(d).unwrap_or(t)
}

impl Session {
    /// Notes an edit of the active document: its project's PDF is no longer current.
    pub(crate) fn note_stale(&mut self) {
        let now = Instant::now();
        self.typeset.last_edit_at = Some(now);
        if self.typeset.quit == QuitTypeset::Done {
            // Edited after all (the window stayed open): typeset again before quitting.
            self.typeset.quit = QuitTypeset::Idle;
        }
        // An untitled document has no PDF beside it.
        if self.doc().is_none_or(|d| d.path.is_none()) {
            return;
        }
        if let Some(root) = self.root_path() {
            self.mark_stale(&root, now);
        }
    }

    /// Remembers that the PDF beside `root` is not current (since `at`, unless known earlier).
    pub(crate) fn mark_stale(&mut self, root: &Path, at: Instant) {
        let root = texcraft_project::normalize(root);
        if !self.typeset.stale.iter().any(|(r, _)| *r == root) {
            self.typeset.stale.push((root, at));
        }
    }

    /// Starts the full typesets that are due: before quitting, and in LIVE mode for projects
    /// edited a while ago. Returns how long until the next one is due, if one is waiting.
    pub(crate) fn tick_full(&mut self, now: Instant) -> Option<Duration> {
        if self.typeset.is_running() || self.typeset.queued {
            return None;
        }
        if matches!(self.typeset.quit, QuitTypeset::Failed { .. }) {
            return None;
        }
        if matches!(self.typeset.quit, QuitTypeset::Running(_)) {
            // A later queued root must not silently save edits received after the save prompt.
            if self.docs.iter().any(|d| d.is_dirty() && !d.is_pristine()) {
                self.typeset.quit = QuitTypeset::Idle;
                self.effect(Effect::Quit);
                return None;
            }
            // One root after another, then quit.
            let next = match &mut self.typeset.quit {
                QuitTypeset::Running(roots) if !roots.is_empty() => Some(roots.remove(0)),
                _ => None,
            };
            if let Some(next) = next {
                if let Err(e) = self.start_full(next.clone(), true) {
                    self.fail_quit(next, e.to_string());
                }
                return None;
            }
            let pending: Vec<_> = self
                .typeset
                .stale
                .iter()
                .map(|(r, _)| r.clone())
                .filter(|r| r.is_file())
                .collect();
            if !pending.is_empty() {
                self.typeset.quit = QuitTypeset::Running(pending);
                return Some(Duration::ZERO);
            }
            self.typeset.quit = QuitTypeset::Done;
            self.effect(Effect::Quit);
            return None;
        }
        if self.settings.mode() != TypesetMode::Live
            || self.live_paused
            || self.typeset.live_due.is_some()
            || self.typeset.queued_live
            || self.typeset.quit != QuitTypeset::Idle
        {
            return None;
        }
        let (root, since) = self
            .typeset
            .stale
            .iter()
            .min_by_key(|(_, at)| *at)
            .cloned()?;
        let mut due = after(since, FULL_TYPESET_EVERY);
        if now.saturating_duration_since(since) < FULL_TYPESET_LATEST
            && let Some(edit) = self.typeset.last_edit_at
        {
            due = due.max(after(edit, FULL_TYPESET_PAUSE));
        }
        if now < due {
            return Some(due.saturating_duration_since(now));
        }
        if !root.is_file() {
            // Renamed or deleted: nothing to typeset.
            self.typeset.stale.retain(|(r, _)| *r != root);
            return Some(Duration::ZERO);
        }
        if let Err(e) = self.start_full(root.clone(), true) {
            self.set_status(e.to_string());
            // Try again later, not on every frame.
            if let Some(entry) = self.typeset.stale.iter_mut().find(|(r, _)| *r == root) {
                entry.1 = now;
            }
        }
        None
    }

    /// After the save prompt, rebuild every open project and any edited project closed earlier.
    /// The window stays open until compilation AND publishing its PDF succeed, in every mode.
    pub fn quit_requested(&mut self) -> bool {
        match self.typeset.quit {
            QuitTypeset::Done => return false,
            QuitTypeset::Running(_) => return true,
            QuitTypeset::Failed { .. } => return true,
            QuitTypeset::Idle => {}
        }
        let mut roots: Vec<PathBuf> = self
            .typeset
            .stale
            .iter()
            .map(|(r, _)| r.clone())
            .filter(|r| r.is_file())
            .collect();
        for doc in &self.docs {
            let Some(path) = &doc.path else { continue };
            let root = texcraft_project::project_entry(path, &|p| self.text_of(p));
            if !root
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("tex") || e.eq_ignore_ascii_case("ltx"))
            {
                continue;
            }
            let root = texcraft_project::normalize(&root);
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        if roots.is_empty() {
            return false;
        }
        // The preview no longer matters: a live pass due or running gives way.
        self.typeset.live_due = None;
        self.typeset.queued = false;
        self.typeset.queued_live = false;
        if self.typeset.live_job
            && let Some(job) = &self.typeset.job
        {
            job.cancel();
            self.typeset.live_dropped = true;
        }
        self.typeset.quit = QuitTypeset::Running(roots);
        true
    }

    /// Keep the window open when either the full rebuild or publishing failed.
    pub(crate) fn fail_quit(&mut self, root: PathBuf, message: String) {
        self.mark_stale(&root, Instant::now());
        self.set_status(message.clone());
        self.typeset.quit = QuitTypeset::Failed { root, message };
        self.typeset.queued = false;
        self.typeset.queued_live = false;
        self.typeset.live_due = None;
    }

    /// A failed exit can be retried after the save prompt has checked the buffers again.
    pub fn retry_quit(&mut self) {
        if matches!(self.typeset.quit, QuitTypeset::Failed { .. }) {
            self.typeset.quit = QuitTypeset::Idle;
            self.effect(Effect::Quit);
        }
    }

    /// Does not quit after all. A full typeset that is running finishes; the projects not yet
    /// typeset stay due.
    pub fn cancel_quit(&mut self) {
        if self.typeset.is_quitting() {
            self.typeset.quit = QuitTypeset::Idle;
        }
    }
}
