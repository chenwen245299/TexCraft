//! LIVE mode's promise: once typing stops, the PDF preview has typeset the current text of every
//! open file of the project, and the files one pass leaves for the next (`.aux`, `.toc`, `.out`,
//! …) have settled.
//!
//! Edits ask for passes as they happen ([`Session::note_edit`] and the commands that edit), but
//! that alone is not enough: a pass can be stopped, fail to start or give way to a full typeset;
//! a command can change the text without asking (Revert); and a live pass is a single engine
//! run, so what it shows of the contents, references and outline comes from the previous pass's
//! files. When a half-typed edit broke those files, the first pass over the corrected text fails
//! too, the last good PDF stays up, and with typing over nothing would ever typeset again.
//!
//! So the engine checks the promise itself whenever nothing runs or waits
//! ([`Session::keep_live_current`]): every document records the revision the preview has typeset
//! ([`crate::Document::typeset_revision`], set only after a successful pass), and
//! * a project file whose text no finished pass typeset gets a pass (a *catch-up*), unless the
//!   last pass started from exactly this text and failed or was stopped (it would again);
//! * a pass that rewrote the files the next one reads back is followed by passes over the same
//!   text until they no longer change, at most [`SETTLE_PASSES`].

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use texcraft_i18n::tr;
use texcraft_project::{TypesetMode, Typesetter};
use texcraft_typeset::LiveRequest;
use texcraft_typeset::Outcome;

use crate::{CommandError, QuitTypeset, Session, full};

/// Passes over unchanged text after an edit, to settle the files passes read back (contents
/// that grow by a page move the page numbers once more; latexmk allows a few runs too).
pub const SETTLE_PASSES: u8 = 3;

impl Session {
    /// Mode changes and explicit resumes are one transition: discard old queued work and let
    /// the current source retry even if its previous attempt was stopped or failed.
    pub(crate) fn set_typeset_mode(&mut self, mode: TypesetMode) {
        self.settings.set_mode(mode);
        self.live_paused = false;
        self.typeset.preview_stopped = false;
        self.cancel_pending_preview();
        if mode != TypesetMode::Manual {
            self.resume_preview();
        }
    }

    pub(crate) fn set_live_paused(&mut self, paused: bool) {
        self.live_paused = paused;
        if paused {
            self.cancel_pending_preview();
        } else {
            self.resume_preview();
        }
    }

    pub(crate) fn cancel_pending_preview(&mut self) {
        self.typeset.live_due = None;
        self.typeset.queued_live = false;
        self.typeset.recovery_pending = false;
    }

    fn resume_preview(&mut self) {
        self.typeset.preview_stopped = false;
        self.typeset.last_attempt = None;
        self.typeset.recovery_stamp = None;
        if self.root_path().is_some() && (self.live_behind() || self.pdf.path.is_none()) {
            self.schedule_live();
        }
    }

    /// A compiler setting can fix the current source without changing its text.
    pub(crate) fn preview_configuration_changed(&mut self) {
        self.typeset.preview_stopped = false;
        self.typeset.last_attempt = None;
        self.typeset.recovery_stamp = None;
        if self.root_path().is_some() {
            self.schedule_live();
        }
    }

    /// Queue consumption obeys the current mode, including settings adopted from another
    /// window while a job was running. A stale queue cannot bypass pause/manual mode.
    pub(crate) fn run_queued_preview(&mut self) {
        if std::mem::take(&mut self.typeset.queued_live)
            && self.settings.mode() == TypesetMode::Live
            && !self.live_paused
            && !self.typeset.preview_stopped
            && self.typeset.quit == QuitTypeset::Idle
            && let Err(e) = self.live_typeset()
        {
            self.set_status(e.to_string());
        }
    }

    pub(crate) fn schedule_live(&mut self) {
        self.typeset.preview_stopped = false;
        if !self.settings.live_typeset || self.live_paused || self.typeset.quit != QuitTypeset::Idle
        {
            return;
        }
        // Remember when the first edit not yet in the PDF happened (for the latency readout).
        self.typeset
            .pending_edit_at
            .get_or_insert_with(Instant::now);
        if self.settings.typesetter == Typesetter::External {
            // Full typesets start when typing pauses.
            let pause = u64::from(self.settings.live_delay_ms).max(800);
            self.typeset.live_due = Some(Instant::now() + Duration::from_millis(pause));
        } else if self.typeset.is_running() {
            // A quiet bibliography refresh gives way; otherwise exactly one more pass right
            // after the current one: back-to-back while typing.
            self.yield_bibliography();
            if let Some(job) = self
                .typeset
                .job
                .as_ref()
                .filter(|job| job.live && job.is_precompiling())
            {
                job.cancel();
                self.typeset.live_dropped = true;
            }
            self.typeset.queued_live = true;
        } else {
            self.typeset.live_due = Some(
                Instant::now() + Duration::from_millis(u64::from(self.settings.live_delay_ms)),
            );
        }
    }

    /// Starts a live-preview pass over the unsaved buffers (never saves anything).
    pub fn live_typeset(&mut self) -> Result<(), CommandError> {
        self.start_live(false)
    }

    /// Starts a live pass (queues one if a run is in progress). `settle`: a pass over text the
    /// last pass already typeset, because that pass rewrote files the next one reads back.
    pub(crate) fn start_live(&mut self, settle: bool) -> Result<(), CommandError> {
        self.typeset.preview_stopped = false;
        // An explicit start consumes the scheduled request too; otherwise the next tick can
        // enqueue a duplicate pass even though nobody typed again.
        self.typeset.live_due = None;
        if self.typeset.is_running() {
            self.typeset.queued_live = true;
            return Ok(());
        }
        let recovery = std::mem::take(&mut self.typeset.recovery_pending);
        let root = self
            .root_path()
            .ok_or_else(|| CommandError::Failed(tr("nothing to typeset").into()))?;
        let root = texcraft_project::normalize(&root);
        let project_dir = root.parent().map(Path::to_path_buf).unwrap_or_default();
        let inputs = self.live_buffers(&project_dir);
        self.typeset.last_attempt = Some(self.live_stamp(&root, &inputs));
        let req = self.preview_request(&root)?;
        self.typeset.console.clear();
        self.typeset.root = Some(root);
        self.typeset.job_revisions = inputs
            .iter()
            .filter_map(|(i, _)| {
                let d = self.docs.get(*i)?;
                Some((texcraft_project::normalize(d.typeset_path()?), d.revision))
            })
            .collect();
        self.typeset.settle_passes = if settle {
            self.typeset.settle_passes.saturating_add(1)
        } else {
            0
        };
        self.typeset.job_edit_at = self.typeset.pending_edit_at.take();
        self.typeset.live_job = true;
        self.typeset.background = false;
        self.typeset.bibliography_job = false;
        self.typeset.quit_job = false;
        self.typeset.job_was_stale = false;
        self.typeset.last_output_at = Some(Instant::now());
        let start = if recovery {
            texcraft_typeset::start_live_recovery
        } else {
            texcraft_typeset::start_live
        };
        self.typeset.job = Some(start(req, &self.toolchain, self.waker.clone()));
        Ok(())
    }

    /// Detect silent hangs even without another queued edit. A source stamp gets one clean
    /// recovery; an exhausted retry stops so a stable failure cannot restart forever.
    fn watch_live(&mut self, now: Instant) {
        let usual = self
            .typeset
            .last_live
            .map_or(Duration::ZERO, |d| d.saturating_mul(4));
        let stuck = self.typeset.live_job
            && !self.typeset.live_dropped
            && self.typeset.job.as_ref().is_some_and(|job| {
                let limit = if self.typeset.queued_live {
                    full::LIVE_STUCK
                } else {
                    full::LIVE_STUCK.saturating_mul(3)
                }
                .max(usual);
                now.saturating_duration_since(job.started) >= limit
                    && self
                        .typeset
                        .last_output_at
                        .is_none_or(|at| now.saturating_duration_since(at) >= limit)
            });
        if !stuck {
            return;
        }
        if let Some(job) = &self.typeset.job {
            job.cancel();
        }
        // Even the last keystroke can hang: do not require another queued edit to detect it.
        // A second timeout of the same input stops instead of restarting forever.
        let stamp = self.typeset.last_attempt;
        let retry = stamp.is_some() && self.typeset.recovery_stamp != stamp;
        self.typeset.recovery_stamp = stamp;
        self.typeset.recovery_pending = retry;
        self.typeset.queued_live = retry;
        self.typeset.live_dropped = retry;
        self.set_status(if retry {
            tr("A live preview pass took too long and was restarted")
        } else {
            tr("Typesetting stopped")
        });
    }

    /// Called when the view cannot load/display a compiled generation after local retries.
    /// The same source gets one automatic clean rebuild; stable failures do not spin.
    pub fn recover_preview(&mut self) -> bool {
        if self.settings.mode() != TypesetMode::Live
            || self.live_paused
            || self.typeset.preview_stopped
            || self.typeset.is_running()
            || self.typeset.quit != QuitTypeset::Idle
        {
            return false;
        }
        let Some((root, inputs)) = self.live_inputs() else {
            return false;
        };
        let stamp = self.live_stamp(&root, &inputs);
        if self.typeset.recovery_stamp == Some(stamp) {
            return false;
        }
        self.typeset.recovery_stamp = Some(stamp);
        self.typeset.recovery_pending = true;
        self.typeset.live_due = Some(Instant::now());
        true
    }

    /// Advances only preview scheduling/recovery; called by the job event pump each frame.
    pub(crate) fn tick_live(&mut self, now: Instant) -> Option<Duration> {
        self.watch_live(now);
        // Live typesetting. A pending bibliography refresh never holds it up: it runs once the
        // preview has caught up (below).
        let mut live_wait = None;
        if let Some(due) = self.typeset.live_due {
            if !self.settings.live_typeset || self.live_paused {
                self.typeset.live_due = None;
            } else if now >= due {
                self.typeset.live_due = None;
                let external = self.settings.typesetter == Typesetter::External;
                if self.typeset.is_running() {
                    if external {
                        self.typeset.queued = true;
                    } else {
                        self.typeset.queued_live = true;
                    }
                } else {
                    let started = if external {
                        self.typeset()
                    } else {
                        self.live_typeset()
                    };
                    if let Err(e) = started {
                        self.set_status(e.to_string());
                    }
                }
            } else {
                live_wait = Some(due.saturating_duration_since(now));
            }
        }
        // LIVE mode: the preview catches up with edits no finished pass typeset, and settles.
        self.keep_live_current();
        live_wait
    }

    /// Snapshot used by a live pass or a full bibliography refresh. Both read current buffers
    /// without saving them, and resolve other files from the owning project.
    pub(crate) fn preview_request(&self, root: &Path) -> Result<LiveRequest, CommandError> {
        let project_dir = root.parent().map(Path::to_path_buf).unwrap_or_default();
        let root_rel = root
            .file_name()
            .map(PathBuf::from)
            .ok_or_else(|| CommandError::Failed(tr("the root file has no name").into()))?;
        let buffers: Vec<_> = self
            .live_buffers(&project_dir)
            .iter()
            .filter_map(|(i, rel)| Some((rel.clone(), self.docs.get(*i)?.text.clone())))
            .collect();
        if !buffers.iter().any(|(p, _)| *p == root_rel) && !root.is_file() {
            return Err(CommandError::Failed(
                tr("the root file is not saved yet").into(),
            ));
        }
        let plan = self.typeset_plan();
        let mut dirs = Vec::new();
        for file in &self.index.files {
            if let Some(rel) = file
                .path
                .parent()
                .and_then(|d| d.strip_prefix(&project_dir).ok())
                && !rel.as_os_str().is_empty()
                && !dirs.iter().any(|d| d == rel)
            {
                dirs.push(rel.to_path_buf());
            }
        }
        Ok(LiveRequest {
            project_dir,
            root_rel,
            buffers,
            dirs,
            engine: plan.engine,
            shell_escape: self.settings.shell_escape,
            cache_preamble: self.settings.cache_preamble,
            pretex: self.pretex_for(&plan),
        })
    }
    /// The open documents a live pass of a root in `folder` typesets from their buffers:
    /// (index, path relative to `folder`).
    pub(crate) fn live_buffers(&self, folder: &Path) -> Vec<(usize, PathBuf)> {
        self.docs
            .iter()
            .enumerate()
            .filter_map(|(i, d)| {
                let p = texcraft_project::normalize(d.typeset_path()?);
                let rel = p.strip_prefix(folder).ok()?.to_path_buf();
                Some((i, rel))
            })
            .collect()
    }

    /// Identifies what a live pass of `root` over `inputs` starts from.
    pub(crate) fn live_stamp(&self, root: &Path, inputs: &[(usize, PathBuf)]) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        root.hash(&mut h);
        self.effective_engine().hash(&mut h);
        self.settings.shell_escape.hash(&mut h);
        self.settings.cache_preamble.hash(&mut h);
        for (i, rel) in inputs {
            rel.hash(&mut h);
            self.docs.get(*i).map(|d| d.revision).hash(&mut h);
        }
        h.finish()
    }

    /// The active root, its folder and the documents a live pass of it reads.
    fn live_inputs(&self) -> Option<(PathBuf, Vec<(usize, PathBuf)>)> {
        let root = texcraft_project::normalize(&self.root_path()?);
        let inputs = self.live_buffers(root.parent()?);
        Some((root, inputs))
    }

    /// Whether `inputs` hold text no finished pass has typeset.
    fn inputs_behind(&self, inputs: &[(usize, PathBuf)]) -> bool {
        inputs.iter().any(|(i, _)| {
            self.docs
                .get(*i)
                .is_some_and(|d| d.revision != d.typeset_revision)
        })
    }

    /// The preview has not typeset the current text of every open file of the active project.
    pub fn live_behind(&self) -> bool {
        self.docs.iter().any(|d| d.revision != d.typeset_revision)
            && self
                .live_inputs()
                .is_some_and(|(_, inputs)| self.inputs_behind(&inputs))
    }

    /// Records what a finished run typeset: the revisions it covered, and whether a live pass
    /// left files the next pass reads back unsettled.
    pub(crate) fn note_typeset(&mut self, outcome: &Outcome) {
        if outcome.recovery_attempted {
            self.typeset.recovery_stamp = self.typeset.last_attempt;
            if outcome.success {
                self.typeset.recoveries = self.typeset.recoveries.saturating_add(1);
            }
        }
        let revisions = std::mem::take(&mut self.typeset.job_revisions);
        let root = self
            .typeset
            .root
            .as_deref()
            .map(texcraft_project::normalize);
        if outcome.cancelled || outcome.error.is_some() {
            if outcome.live {
                self.typeset.unsettled = None;
            }
            return;
        }
        // Finishing an attempt is not proof the preview contains that source. In particular,
        // TeX errors keep the last good PDF: its revision must stay behind until recovery.
        if outcome.success {
            for (path, revision) in revisions {
                for d in &mut self.docs {
                    if d.typeset_path()
                        .is_some_and(|p| texcraft_project::normalize(p) == path)
                    {
                        d.typeset_revision = revision;
                    }
                }
            }
        }
        if outcome.live {
            self.typeset.unsettled = root.filter(|_| outcome.read_back_changed);
        } else if self.typeset.unsettled == root {
            // latexmk runs until those files settle.
            self.typeset.unsettled = None;
        }
    }

    /// Keeps LIVE mode's promise (see the module documentation): when nothing runs or waits,
    /// starts the pass the preview still needs, if any.
    pub(crate) fn keep_live_current(&mut self) {
        if self.settings.mode() != TypesetMode::Live
            || self.live_paused
            || self.typeset.preview_stopped
            || self.typeset.is_running()
            || self.typeset.queued
            || self.typeset.queued_live
            || self.typeset.live_due.is_some()
            || self.typeset.quit != QuitTypeset::Idle
        {
            return;
        }
        // Every frame while idle: the common case costs one pass over the documents.
        if self.typeset.unsettled.is_none()
            && self.docs.iter().all(|d| d.revision == d.typeset_revision)
        {
            return;
        }
        let Some((root, inputs)) = self.live_inputs() else {
            return;
        };
        let started = if self.inputs_behind(&inputs)
            && self.typeset.last_attempt != Some(self.live_stamp(&root, &inputs))
        {
            self.typeset.catch_ups += 1;
            self.start_live(false)
        } else if self.typeset.unsettled.as_deref() == Some(root.as_path())
            && self.typeset.settle_passes < SETTLE_PASSES
        {
            self.start_live(true)
        } else {
            // A stable error/stopped pass waits for an edit; changed auxiliary files above
            // can still settle after a failed pass without falsely marking the PDF current.
            return;
        };
        if let Err(e) = started {
            self.set_status(e.to_string());
        }
    }
}
