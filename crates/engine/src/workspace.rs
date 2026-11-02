//! The saved workspace contains real files, never the scratch paths of untitled documents.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{CommandError, Effect, Selection, Session};

/// Documents and project to reopen after quitting. An explicitly empty workspace means the
/// user closed their documents; it must not fall back to the recent-documents list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    /// Project folder, including folders without a TeX root.
    pub project_dir: Option<PathBuf>,
    /// Tabs in display order.
    pub documents: Vec<WorkspaceDocument>,
    /// Active file (a path remains meaningful when another tab has since disappeared).
    pub active: Option<PathBuf>,
    /// Last root, needed when a bibliography is the active tab.
    pub root: Option<PathBuf>,
    /// A PDF opened on its own, rather than a typeset result.
    pub pdf: Option<PathBuf>,
}

/// A saved tab and its editor selection, measured in characters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDocument {
    /// Source file on disk.
    pub path: PathBuf,
    /// Cursor and selection, clamped to the current file when reopening it.
    #[serde(default)]
    pub selection: Selection,
}

impl Session {
    /// Captures what should reopen. Unsaved text is handled by the save prompt before quit.
    pub fn workspace(&self) -> Workspace {
        Workspace {
            project_dir: self.project_dir.clone(),
            documents: self
                .docs
                .iter()
                .filter_map(|d| {
                    Some(WorkspaceDocument {
                        path: d.path.clone()?,
                        selection: d.selection,
                    })
                })
                .collect(),
            active: self.doc().and_then(|d| d.path.clone()),
            root: self
                .doc()
                .filter(|d| d.path.is_some())
                .and_then(|_| self.root_path()),
            pdf: self.pdf.path.clone().filter(|_| self.docs.is_empty()),
        }
    }

    /// Restores into an empty session. A missing tab does not prevent other tabs reopening.
    pub fn restore_workspace(&mut self, workspace: &Workspace) -> Vec<CommandError> {
        let mut errors = Vec::new();
        self.typeset.root = workspace.root.clone().filter(|p| p.is_file());
        if let Some(dir) = workspace.project_dir.as_ref().filter(|d| d.is_dir()) {
            self.set_project_dir(dir.clone());
        }
        for tab in &workspace.documents {
            match self.open_path(&tab.path) {
                Ok(Some(i)) => {
                    if let Some(doc) = self.docs.get_mut(i) {
                        let len = doc.text.chars().count();
                        doc.selection = Selection {
                            anchor: tab.selection.anchor.min(len),
                            cursor: tab.selection.cursor.min(len),
                        };
                    }
                }
                Ok(None) => {}
                Err(e) => errors.push(e),
            }
        }
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.path.is_some() && d.path == workspace.active)
        {
            self.active = i;
        }
        if self.docs.is_empty() && !workspace.documents.is_empty() {
            // All source files disappeared: return to the welcome window.
            self.project_dir = None;
            self.file_tree.clear();
        }
        if let Some(pdf) = workspace.pdf.as_ref().filter(|p| p.is_file()) {
            self.show_pdf(pdf.clone());
        }
        if let Some(doc) = self.doc() {
            self.effect(Effect::RevealLine {
                doc: self.active,
                line: doc.caret_line(),
            });
            self.effect(Effect::SelectionChanged { doc: self.active });
        }
        errors
    }
}
