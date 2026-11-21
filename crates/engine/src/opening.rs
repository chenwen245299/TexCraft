//! One project per window. Other projects are platform requests, never extra editor tabs.

use std::path::{Path, PathBuf};

use crate::{Document, Session, index};

/// The folder's main file, using the same deterministic selection for Open and recents.
pub(crate) fn project_main(dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "tex"))
        .collect();
    candidates.sort();
    candidates
        .iter()
        .find(|path| path.file_name().is_some_and(|name| name == "main.tex"))
        .or_else(|| {
            candidates.iter().find(|path| {
                std::fs::read_to_string(path)
                    .is_ok_and(|text| texcraft_syntax::is_root_document(&text))
            })
        })
        .or_else(|| candidates.first())
        .cloned()
}

pub(crate) fn recent_entry(path: &Path, read: &dyn Fn(&Path) -> Option<String>) -> PathBuf {
    if path.is_dir() {
        project_main(path).unwrap_or_else(|| path.to_path_buf())
    } else {
        texcraft_project::project_entry(path, read)
    }
}

/// Where an open request was handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenResult {
    /// Opened in this window, with the text tab's index if there is one.
    Here(Option<usize>),
    /// The platform was asked to open an independent project window.
    NewWindow,
}

impl OpenResult {
    /// Whether this window opened the requested content.
    pub fn is_here(self) -> bool {
        matches!(self, Self::Here(_))
    }
}

impl Session {
    pub(crate) fn needs_project_window(&self, path: &Path) -> bool {
        let directory = self
            .project_dir
            .as_ref()
            .map(|dir| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone()));
        if (!self.docs.is_empty()
            && self.docs.iter().all(Document::is_pristine)
            && directory.as_ref().is_none_or(|dir| path.starts_with(dir)))
            || (self.docs.is_empty() && self.project_dir.is_none() && self.pdf.path.is_none())
        {
            return false;
        }
        if path.is_dir() {
            return directory.as_deref() != Some(path);
        }
        if self.docs.iter().any(|d| d.path.as_deref() == Some(path)) {
            return false;
        }
        let read = |p: &Path| self.text_of(p);
        let entry = texcraft_project::project_entry(path, &read);
        let root = self.project_root.clone().or_else(|| {
            self.doc()
                .and_then(|d| d.path.as_ref())
                .map(|p| texcraft_project::project_entry(p, &read))
                .or_else(|| self.root_path())
        });
        if root.as_deref() == Some(entry.as_path()) {
            return false;
        }
        // Included chapters and shared assets can live outside the project folder. They
        // remain tabs of their owning project, including subfiles with a document class.
        if let Some(root) = &root {
            let project = index::build(root, &read);
            if project.files.iter().any(|f| f.path == path)
                || project.assets.iter().any(|f| f.path == path)
            {
                return false;
            }
        }
        // Independent roots in the same folder are independent projects too.
        if root.is_some()
            && read(&entry).is_some_and(|text| texcraft_syntax::is_root_document(&text))
        {
            return true;
        }
        !directory.as_ref().is_some_and(|dir| path.starts_with(dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use serde_json::json;
    use texcraft_ai::ToolCall;

    use crate::{Effect, OpenResult};

    fn session() -> Session {
        let mut session = Session::empty(Arc::new(|| {}));
        session.settings.live_typeset = false;
        session.update.enabled = false;
        session
    }

    fn projects(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("texcraft-windows-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for name in ["a", "b"] {
            let dir = base.join(name);
            std::fs::create_dir_all(dir.join("sections")).unwrap();
            std::fs::write(dir.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/chapter}\n\\end{document}\n").unwrap();
            std::fs::write(
                dir.join("sections/chapter.tex"),
                "% !TEX root = ../main.tex\nChapter.\n",
            )
            .unwrap();
            std::fs::write(dir.join(format!("{name}_only.txt")), name).unwrap();
        }
        base.canonicalize().unwrap()
    }

    #[test]
    fn recent_project_opens_an_independent_window_without_touching_unsaved_work_or_ai() {
        let base = projects("recent");
        let mut original = session();
        original.open_path(&base.join("a/main.tex")).unwrap();
        original
            .open_path(&base.join("a/sections/chapter.tex"))
            .unwrap();
        original
            .run("edit.insertText", &json!({"text": "Unsaved A"}))
            .unwrap();
        original.ai.draft = "Question about A".into();
        original.show_pdf(base.join("a/main.pdf"));
        original.effects.clear();
        let old_text = original.doc().unwrap().text.clone();

        original
            .run(
                "file.openRecent",
                &json!({"path": base.join("b/sections/chapter.tex")}),
            )
            .unwrap();
        assert_eq!(original.docs.len(), 2);
        assert_eq!(original.doc().unwrap().text, old_text);
        assert!(original.doc().unwrap().is_dirty());
        assert_eq!(original.project_dir, Some(base.join("a")));
        assert_eq!(original.pdf.path, Some(base.join("a/main.pdf")));
        assert_eq!(original.ai.draft, "Question about A");
        assert_eq!(original.ai_project_root(), Some(base.join("a")));
        assert_eq!(
            original.effects,
            [Effect::OpenProjectWindow(base.join("b/main.tex"))]
        );

        // The platform launches a fresh session for the requested entry, never the old tabs.
        let mut other = session();
        other.open_path(&base.join("b/main.tex")).unwrap();
        assert_eq!(other.docs.len(), 1);
        assert_eq!(other.project_dir, Some(base.join("b")));
        assert_eq!(other.ai_project_root(), Some(base.join("b")));
        assert!(other.ai.draft.is_empty() && other.ai.messages.is_empty());
        let (listing, used) = other.run_ai_tool(
            &base.join("b"),
            &ToolCall {
                id: "list".into(),
                name: "list_files".into(),
                arguments: "{}".into(),
            },
        );
        assert!(!used.failed && listing.contains("b_only.txt"), "{listing}");
        assert!(!listing.contains("a_only.txt"), "{listing}");
        let (_, used) = other.run_ai_tool(
            &base.join("b"),
            &ToolCall {
                id: "read".into(),
                name: "read_file".into(),
                arguments: json!({"path": base.join("a/a_only.txt")}).to_string(),
            },
        );
        assert!(
            used.failed,
            "the new window cannot read the old project's files"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn chapters_shared_inputs_and_bibliographies_remain_tabs_of_their_project() {
        let base = projects("members");
        std::fs::write(base.join("shared.tex"), "Shared chapter.").unwrap();
        let main = base.join("a/main.tex");
        let text = std::fs::read_to_string(&main).unwrap() + "\n\\input{../shared}\n";
        std::fs::write(&main, text).unwrap();
        std::fs::write(base.join("a/refs.bib"), "@book{key,title={Book}}").unwrap();
        let mut s = session();
        s.open_path(&main).unwrap();
        for path in [
            "a/sections/chapter.tex",
            "a/refs.bib",
            "shared.tex",
            "a/main.tex",
        ] {
            assert!(s.open_path(&base.join(path)).unwrap().is_here());
        }
        assert_eq!(s.docs.len(), 4);
        assert_eq!(s.project_dir, Some(base.join("a")));
        assert!(
            !s.effects
                .iter()
                .any(|e| matches!(e, Effect::OpenProjectWindow(_)))
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn opening_a_chapter_from_welcome_uses_its_owning_project_for_files_and_ai() {
        let base = projects("chapter");
        let mut s = session();
        assert!(
            s.open_path(&base.join("b/sections/chapter.tex"))
                .unwrap()
                .is_here()
        );
        assert_eq!(s.project_dir, Some(base.join("b")));
        assert_eq!(s.ai_project_root(), Some(base.join("b")));
        assert_eq!(s.root_path(), Some(base.join("b/main.tex")));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn independent_roots_and_folders_never_become_tabs_of_another_project() {
        let base = projects("roots");
        let alternate = base.join("a/alternate.tex");
        std::fs::write(&alternate, "\\documentclass{article}").unwrap();
        let mut s = session();
        s.open_path(&base.join("a/main.tex")).unwrap();
        for path in [&alternate, &base.join("b")] {
            assert_eq!(s.open_path(path).unwrap(), OpenResult::NewWindow);
            assert_eq!(s.docs.len(), 1);
            assert_eq!(s.project_dir, Some(base.join("a")));
        }
        assert_eq!(s.recent.first(), Some(&base.join("b/main.tex")));
        let empty = base.join("a/empty-project");
        std::fs::create_dir_all(&empty).unwrap();
        s.set_recent(vec![empty.clone()]);
        assert_eq!(s.recent.as_slice(), std::slice::from_ref(&empty));
        s.run("file.openRecent", &json!({"path": empty})).unwrap();
        assert_eq!(s.effects.back(), Some(&Effect::OpenProjectWindow(empty)));
        let before = s.effects.len();
        assert!(s.open_path(&base.join("missing.tex")).is_err());
        assert_eq!(s.effects.len(), before);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn changing_the_project_directory_updates_ai_context_immediately_without_a_chat_store() {
        let base = projects("ai-directory");
        let mut s = session();
        s.set_project_dir(base.join("a"));
        s.ai.draft = "Old project question".into();
        s.set_project_dir(base.join("b"));
        assert_eq!(s.ai_project_root(), Some(base.join("b")));
        assert!(s.ai.draft.is_empty());
        assert_eq!(s.inspect()["ai"]["project"], json!(base.join("b")));
        std::fs::remove_dir_all(base).unwrap();
    }
}
