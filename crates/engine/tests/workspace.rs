#![allow(clippy::unwrap_used)] // Test fixtures and assertions only.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};
use texcraft_engine::{Effect, Selection, Session, workspace::Workspace};

fn empty() -> Session {
    let mut s = Session::empty(Arc::new(|| {}));
    s.settings.live_typeset = false;
    s.update.enabled = false;
    s
}

fn directory(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("texcraft-workspace-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

#[test]
fn empty_startup_never_creates_or_typesets_a_template() {
    let s = Session::empty(Arc::new(|| {}));
    assert!(s.docs.is_empty());
    assert!(s.root_path().is_none());
    assert!(s.typeset.live_due.is_none());
    assert!(!s.is_enabled("typeset.run"));
    assert!(s.workspace().documents.is_empty());
}

#[test]
fn workspace_round_trip_restores_project_tabs_active_file_and_unicode_selection() {
    let dir = directory("round-trip");
    let root = dir.join("main.tex");
    let chapter = dir.join("chapter.tex");
    std::fs::write(&root, "\\documentclass{article}\n\\input{chapter}").unwrap();
    std::fs::write(&chapter, "% !TEX root = main.tex\n中文 chapter").unwrap();
    let mut s = empty();
    s.open_path(&root).unwrap();
    s.open_path(&chapter).unwrap();
    s.doc_mut().unwrap().selection = Selection {
        anchor: 25,
        cursor: 29,
    };
    s.new_untitled("scratch document".into());
    s.active = 1;
    let saved: Workspace =
        serde_json::from_str(&serde_json::to_string(&s.workspace()).unwrap()).unwrap();
    let mut restored = empty();
    assert!(restored.restore_workspace(&saved).is_empty());
    assert_eq!(restored.docs.len(), 2);
    assert_eq!(restored.project_dir.as_ref(), Some(&dir));
    assert_eq!(restored.docs[0].path.as_ref(), Some(&root));
    assert_eq!(restored.doc().unwrap().path.as_ref(), Some(&chapter));
    assert_eq!(
        restored.doc().unwrap().selection,
        Selection {
            anchor: 25,
            cursor: 29
        }
    );
    assert!(restored.docs.iter().all(|d| !d.is_dirty()));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_tabs_and_shortened_files_do_not_block_restoration() {
    let dir = directory("missing");
    let a = dir.join("a.tex");
    let b = dir.join("b.tex");
    std::fs::write(&a, "a").unwrap();
    std::fs::write(&b, "long document").unwrap();
    let mut s = empty();
    s.open_path(&a).unwrap();
    s.open_path(&b).unwrap();
    s.doc_mut().unwrap().selection = Selection::caret(99);
    let saved = s.workspace();
    std::fs::remove_file(&a).unwrap();
    std::fs::write(&b, "中文").unwrap();
    let mut restored = empty();
    assert_eq!(restored.restore_workspace(&saved).len(), 1);
    assert_eq!(restored.docs.len(), 1);
    assert_eq!(restored.doc().unwrap().selection, Selection::caret(2));
    std::fs::remove_file(&b).unwrap();
    let mut gone = empty();
    assert_eq!(gone.restore_workspace(&saved).len(), 2);
    assert!(gone.docs.is_empty());
    assert!(gone.project_dir.is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn close_preserves_the_active_tab_then_returns_to_welcome_without_a_phantom_document() {
    let mut s = empty();
    for text in ["first", "second", "third"] {
        s.new_untitled(text.into());
    }
    s.run("file.closeTab", &json!({"index": 0, "force": true}))
        .unwrap();
    assert_eq!(s.doc().unwrap().text, "third");
    s.close(99);
    assert_eq!(s.docs.len(), 2);
    s.close(0);
    s.project_dir = Some(PathBuf::from("project"));
    s.pdf.path = Some(PathBuf::from("old.pdf"));
    s.close(0);
    assert!(s.docs.is_empty());
    assert!(s.project_dir.is_none() && s.file_tree.is_empty() && s.pdf.path.is_none());
    assert!(s.workspace().documents.is_empty());
    assert!(s.effects.iter().any(|e| matches!(e, Effect::Welcome)));
    assert!(!s.is_enabled("file.closeTab"));
    s.run("file.new", &Value::Null).unwrap();
    assert_eq!(
        s.docs.len(),
        1,
        "only explicit New creates an untitled document"
    );
}

#[test]
fn empty_folder_and_standalone_pdf_survive_relaunch() {
    let dir = directory("folder");
    let mut s = empty();
    s.open_path(&dir).unwrap();
    let mut restored = empty();
    assert!(restored.restore_workspace(&s.workspace()).is_empty());
    assert_eq!(restored.project_dir, Some(dir.clone()));
    assert!(restored.docs.is_empty());
    let pdf = dir.join("paper.pdf");
    std::fs::write(&pdf, b"%PDF-1.5").unwrap();
    s.show_pdf(pdf.clone());
    let mut restored = empty();
    restored.restore_workspace(&s.workspace());
    assert_eq!(restored.pdf.path, Some(pdf));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn active_bibliography_restores_its_typesetting_root() {
    let dir = directory("bibliography");
    let root = dir.join("main.tex");
    let bib = dir.join("refs.bib");
    std::fs::write(&root, "\\documentclass{article}").unwrap();
    std::fs::write(&bib, "@book{example, title={Example}}").unwrap();
    let mut s = empty();
    s.open_path(&root).unwrap();
    s.typeset.root = Some(root.clone());
    s.open_path(&bib).unwrap();
    let mut restored = empty();
    assert!(restored.restore_workspace(&s.workspace()).is_empty());
    assert_eq!(restored.doc().unwrap().path.as_ref(), Some(&bib));
    assert_eq!(restored.root_path(), Some(root));
    assert!(restored.is_enabled("typeset.run"));
    std::fs::remove_dir_all(dir).unwrap();
}
