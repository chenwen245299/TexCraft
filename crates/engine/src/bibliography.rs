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
        if self.typeset.is_running() {
            // A foreground build finishes normally. A live pass yields to the full refresh.
            if self.typeset.live_job && !self.typeset.live_dropped {
                if let Some(job) = &self.typeset.job {
                    job.cancel();
                }
                self.typeset.live_dropped = true;
            }
            return None;
        }
        self.typeset.bibliography_due = None;
        let Ok(buffers) = self.preview_request(&root) else {
            return None;
        };
        let inputs = self.live_buffers(&buffers.project_dir);
        self.typeset.last_attempt = Some(self.live_stamp(&root, &inputs));
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
        self.typeset.live_due = None;
        self.typeset.queued_live = false;
        self.typeset.live_job = false;
        self.typeset.background = true;
        self.typeset.bibliography_job = true;
        self.typeset.job = Some(texcraft_typeset::start_buffered(
            req,
            buffers,
            &self.toolchain,
            self.waker.clone(),
        ));
        None
    }
}
