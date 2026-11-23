//! Quiet full rebuilds after bibliography edits. A short pause coalesces typing; the complete
//! toolchain reads a snapshot, so unsaved `.bib` and citation edits reach BibTeX/Biber together.

use std::time::{Duration, Instant};

use texcraft_i18n::tr;
use texcraft_typeset::Request;

use crate::{CommandError, QuitTypeset, Session};

pub const BIBLIOGRAPHY_PAUSE: Duration = Duration::from_millis(800);

impl Session {
    /// Requests a bibliography refresh for the current project, in every typesetting mode.
    pub fn refresh_bibliography(&mut self) -> Result<(), CommandError> {
        let root = self
            .root_path()
            .ok_or_else(|| CommandError::Failed(tr("nothing to typeset").into()))?;
        if !root
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("tex") || ext.eq_ignore_ascii_case("ltx"))
        {
            return Ok(());
        }
        let now = Instant::now();
        self.typeset.bibliography_due = Some((
            texcraft_project::normalize(&root),
            now.checked_add(BIBLIOGRAPHY_PAUSE).unwrap_or(now),
        ));
        Ok(())
    }

    /// An edit arrived while a quiet refresh of this project runs: its snapshot is already out
    /// of date, and the full toolchain would hold up the live pass for seconds. Stop it and
    /// refresh again at the next pause.
    pub(crate) fn yield_bibliography(&mut self) {
        if self.typeset.queued || self.typeset.is_quitting() {
            return;
        }
        if self
            .root_path()
            .is_none_or(|current| Some(texcraft_project::normalize(&current)) != self.typeset.root)
        {
            return;
        }
        self.drop_bibliography_job(true);
    }

    /// Stops a running quiet refresh so that it leaves no trace (the toolchain restores the
    /// last good outputs), for work that matters more. `rearm`: refresh again at the next
    /// pause. Returns whether one was running.
    pub(crate) fn drop_bibliography_job(&mut self, rearm: bool) -> bool {
        if !self.typeset.bibliography_job
            || !self.typeset.is_full_running()
            || self.typeset.live_dropped
        {
            return false;
        }
        let Some(root) = self.typeset.root.clone() else {
            return false;
        };
        if let Some(job) = &self.typeset.job {
            job.cancel();
        }
        self.typeset.live_dropped = true;
        if rearm {
            let now = Instant::now();
            self.typeset.bibliography_due =
                Some((root, now.checked_add(BIBLIOGRAPHY_PAUSE).unwrap_or(now)));
        }
        true
    }

    pub(crate) fn tick_bibliography(&mut self, now: Instant) -> Option<Duration> {
        let (root, due) = self.typeset.bibliography_due.clone()?;
        // A delayed request belongs to its original project, even if tabs/windows changed.
        if self
            .root_path()
            .is_none_or(|current| texcraft_project::normalize(&current) != root)
        {
            self.typeset.bibliography_due = None;
            return None;
        }
        if self.typeset.quit != QuitTypeset::Idle || self.typeset.queued {
            return None;
        }
        if now < due {
            return Some(due.saturating_duration_since(now));
        }
        // Background work: the preview of the latest edit comes first. Wait for whatever runs
        // (it wakes the next tick when it finishes) and for the live pass that is due.
        if self.typeset.is_running() || self.typeset.live_due.is_some() || self.typeset.queued_live
        {
            return None;
        }
        self.typeset.bibliography_due = None;
        let Ok(buffers) = self.preview_request(&root) else {
            return None;
        };
        // Not `last_attempt`: that belongs to live passes, and a refresh that fails must not
        // stop the preview from catching up with these buffers.
        let inputs = self.live_buffers(&buffers.project_dir);
        self.typeset.job_revisions = inputs
            .iter()
            .filter_map(|(i, _)| {
                let doc = self.docs.get(*i)?;
                Some((
                    texcraft_project::normalize(doc.typeset_path()?),
                    doc.revision,
                ))
            })
            .collect();
        let req = Request {
            root: root.clone(),
            engine: buffers.engine,
            shell_escape: buffers.shell_escape,
            bib_tool: self.settings.bib_tool,
            pretex: buffers.pretex.clone(),
            force: true,
        };
        self.typeset.root = Some(root);
        self.typeset.live_job = false;
        self.typeset.background = true;
        self.typeset.bibliography_job = true;
        self.typeset.quit_job = false;
        // It publishes nothing when it fails: the root's staleness is untouched.
        self.typeset.job_was_stale = false;
        self.typeset.job = Some(texcraft_typeset::start_buffered(
            req,
            buffers,
            &self.toolchain,
            self.waker.clone(),
        ));
        None
    }
}
