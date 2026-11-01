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
//! (`Document::typeset_revision`, set when a pass over it finishes), and
//! * a project file whose text no finished pass typeset gets a pass (a *catch-up*), unless the
//!   last pass started from exactly this text and failed or was stopped (it would again);
//! * a pass that rewrote the files the next one reads back is followed by passes over the same
//!   text until they no longer change, at most [`SETTLE_PASSES`].

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use texcraft_project::TypesetMode;
use texcraft_typeset::Outcome;

use crate::{QuitTypeset, Session};

/// Passes over unchanged text after an edit, to settle the files passes read back (contents
/// that grow by a page move the page numbers once more; latexmk allows a few runs too).
pub const SETTLE_PASSES: u8 = 3;

impl Session {
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
        for (path, revision) in revisions {
            for d in &mut self.docs {
                if d.typeset_path()
                    .is_some_and(|p| texcraft_project::normalize(p) == path)
                {
                    d.typeset_revision = revision;
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
        let started = if self.inputs_behind(&inputs) {
            if self.typeset.last_attempt == Some(self.live_stamp(&root, &inputs)) {
                // That pass failed or was stopped: the next edit tries again.
                return;
            }
            self.typeset.catch_ups += 1;
            self.start_live(false)
        } else if self.typeset.unsettled.as_deref() == Some(root.as_path())
            && self.typeset.settle_passes < SETTLE_PASSES
        {
            self.start_live(true)
        } else {
            return;
        };
        if let Err(e) = started {
            self.set_status(e.to_string());
        }
    }
}
