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

/// What every window compares to show each project in one window only: the root file `path`
/// opens as, else (a file or folder without a root document) its folder. `None` if `path`
/// does not exist.
pub fn project_key(path: &Path) -> Option<PathBuf> {
    let path = std::fs::canonicalize(path).ok()?;
    let read = |p: &Path| std::fs::read_to_string(p).ok();
    let entry = if path.is_dir() {
        match project_main(&path) {
            Some(main) => texcraft_project::project_entry(&main, &read),
            None => return Some(path),
        }
    } else {
        texcraft_project::project_entry(&path, &read)
    };
    if read(&entry).is_some_and(|text| texcraft_syntax::is_root_document(&text)) {
        Some(std::fs::canonicalize(&entry).unwrap_or(entry))
    } else {
        entry.parent().map(Path::to_path_buf)
    }
}

pub(crate) fn recent_entry(path: &Path, read: &dyn Fn(&Path) -> Option<String>) -> PathBuf {
    if path.is_dir() {
        project_main(path).unwrap_or_else(|| path.to_path_buf())
    } else {
        texcraft_project::project_entry(path, read)
    }
}

/// Only a saved root document can represent a project in Recent. A loose file, an empty
/// folder or an ambiguous asset must not become an entry merely because it can be opened.
pub(crate) fn recent_root(path: &Path, read: &dyn Fn(&Path) -> Option<String>) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    let entry = recent_entry(&path, read);
    if texcraft_project::FileKind::of(&entry) != texcraft_project::FileKind::Tex
        || !read(&entry).is_some_and(|text| texcraft_syntax::is_root_document(&text))
    {
        return None;
    }
    entry.canonicalize().ok()
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
    /// The [`project_key`] of the project this window shows (`None`: none, or only untitled
    /// documents).
    pub fn window_key(&self) -> Option<PathBuf> {
        self.project_root
            .clone()
            .or_else(|| self.project_dir.clone())
            .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
    }

    /// A window without a project of its own still leaves `path` to the window that already
    /// shows its project (the platform answers [`Session::project_elsewhere`]).
    pub(crate) fn shown_elsewhere(&self, path: &Path) -> bool {
        self.project_root.is_none()
            && self.project_dir.is_none()
            && self
                .project_elsewhere
                .as_ref()
                .is_some_and(|elsewhere| project_key(path).is_some_and(|key| elsewhere(&key)))
    }

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
            if directory.as_deref() != Some(path) {
                return true;
            }
            // This window's own folder, whose main project can be another root in it (the
            // window shows `alternate.tex`, the folder opens `main.tex`): that one's window.
            return self.project_root.as_ref().is_some_and(|own| {
                let own = std::fs::canonicalize(own).unwrap_or_else(|_| own.clone());
                project_key(path).is_some_and(|key| key != own)
            });
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
            // `path` is canonical; the index's paths are as written (a symlinked folder of
            // shared macros leads elsewhere).
            let is_path = |p: &Path| p == path || std::fs::canonicalize(p).is_ok_and(|c| c == path);
            if project.files.iter().any(|f| is_path(&f.path))
                || project.assets.iter().any(|f| is_path(&f.path))
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

    /// A window and a request to open its project agree on what the project is: its root file
    /// from the root, a chapter, the folder or the PDF beside it; a folder without one is
    /// itself the project.
    #[test]
    fn every_way_into_a_project_has_the_windows_key() {
        let base = projects("keys");
        let main = base.join("a/main.tex");
        for path in [
            main.clone(),
            base.join("a/sections/chapter.tex"),
            base.join("a"),
        ] {
            assert_eq!(
                project_key(&path).as_deref(),
                Some(main.as_path()),
                "{path:?}"
            );
        }
        let mut window = session();
        window
            .open_path(&base.join("a/sections/chapter.tex"))
            .unwrap();
        assert_eq!(window.window_key().as_deref(), Some(main.as_path()));
        let loose = base.join("loose");
        std::fs::create_dir_all(&loose).unwrap();
        std::fs::write(loose.join("notes.txt"), "notes").unwrap();
        assert_eq!(project_key(&loose).as_deref(), Some(loose.as_path()));
        assert_eq!(project_key(&base.join("missing")), None);
        let _ = std::fs::remove_dir_all(base);
    }

    /// A window without a project (the welcome window) leaves a project another window shows
    /// to that window instead of opening it a second time; one with a project of its own keeps
    /// its own files.
    #[test]
    fn a_project_shown_in_another_window_is_not_opened_again() {
        let base = projects("elsewhere");
        let a = base.join("a/main.tex");
        let shown = a.clone();
        let mut empty = session();
        empty.project_elsewhere = Some(Arc::new(move |key: &Path| key == shown));
        let result = empty
            .open_path(&base.join("a/sections/chapter.tex"))
            .unwrap();
        assert_eq!(result, OpenResult::NewWindow);
        assert!(empty.docs.is_empty() && empty.project_root.is_none());
        assert_eq!(
            empty.effects.back(),
            Some(&Effect::OpenProjectWindow(
                base.join("a/sections/chapter.tex")
            ))
        );
        // Another project still opens here.
        assert!(empty.open_path(&base.join("b/main.tex")).unwrap().is_here());
        // The window that shows a project opens its files itself.
        let mut owner = session();
        owner.open_path(&a).unwrap();
        let mine = a.clone();
        owner.project_elsewhere = Some(Arc::new(move |key: &Path| key == mine));
        assert!(
            owner
                .open_path(&base.join("a/sections/chapter.tex"))
                .unwrap()
                .is_here()
        );
        let _ = std::fs::remove_dir_all(base);
    }

    /// Open Folder on this window's own folder, whose main project is another root than the
    /// one this window shows, opens that project in its own window (not as a second root here).
    #[test]
    fn the_windows_own_folder_with_another_main_root_opens_elsewhere() {
        let base = projects("own-folder");
        let a = base.join("a");
        std::fs::write(
            a.join("alternate.tex"),
            "\\documentclass{article}\n\\begin{document}\nAlt.\n\\end{document}\n",
        )
        .unwrap();
        let mut window = session();
        window.open_path(&a.join("alternate.tex")).unwrap();
        assert_eq!(window.open_path(&a).unwrap(), OpenResult::NewWindow);
        assert_eq!(window.docs.len(), 1);
        assert_eq!(
            window.effects.back(),
            Some(&Effect::OpenProjectWindow(a.clone()))
        );
        // The window that shows the folder's main project opens it as before.
        let mut main = session();
        main.open_path(&a.join("main.tex")).unwrap();
        assert!(main.open_path(&a).unwrap().is_here());
        let _ = std::fs::remove_dir_all(base);
    }

    /// A chapter reached through a symlinked folder that points outside the project is still
    /// one of its tabs (its real path differs from the path the document names).
    #[test]
    fn a_chapter_through_a_symlink_stays_a_tab_of_its_project() {
        let base = projects("symlink");
        let shared = base.join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("macros.tex"), "\\newcommand{\\x}{x}\n").unwrap();
        std::os::unix::fs::symlink(&shared, base.join("a/shared")).unwrap();
        std::fs::write(
            base.join("a/main.tex"),
            "\\documentclass{article}\n\\input{shared/macros}\n\\begin{document}\n\\end{document}\n",
        )
        .unwrap();
        let mut window = session();
        window.open_path(&base.join("a/main.tex")).unwrap();
        let result = window.open_path(&base.join("a/shared/macros.tex")).unwrap();
        assert!(result.is_here(), "{:?}", window.effects);
        assert_eq!(window.docs.len(), 2);
        let _ = std::fs::remove_dir_all(base);
    }

    /// A `.bib` with no root of its own uses the last root typeset only if it is that root's;
    /// once the window is empty, that root is forgotten.
    #[test]
    fn a_lone_bib_never_borrows_another_projects_root() {
        let base = projects("bib-root");
        let other = base.join("elsewhere");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("refs.bib"), "@book{a, title={A}}\n").unwrap();
        let mut window = session();
        window.typeset.root = Some(base.join("a/main.tex"));
        window.open_path(&other.join("refs.bib")).unwrap();
        assert_eq!(window.root_path(), None);
        std::fs::write(base.join("a/refs.bib"), "@book{b, title={B}}\n").unwrap();
        let mut own = session();
        own.typeset.root = Some(base.join("a/main.tex"));
        own.open_path(&base.join("a/refs.bib")).unwrap();
        own.project_root = None;
        assert_eq!(own.root_path(), Some(base.join("a/main.tex")));
        let _ = std::fs::remove_dir_all(base);
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
        assert!(s.recent.is_empty());
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
