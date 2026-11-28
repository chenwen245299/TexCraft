use super::*;

#[test]
fn existing_preferences_keep_the_pdf_reading_position_after_upgrading() {
    for version in 0..3 {
        let mut view = View {
            sync_after_typeset: true,
            zoom: texcraft_engine::Zoom::Fixed(1.75),
            page_mode: texcraft_engine::PageMode::TwoContinuous,
            ..Default::default()
        };
        let mut settings = texcraft_project::Settings::default();
        migrate_preferences(version, &mut view, &mut settings);
        assert!(
            !view.sync_after_typeset,
            "legacy version {version} must stop forced jumps"
        );
        assert_eq!(view.zoom, texcraft_engine::Zoom::Fixed(1.75));
        assert_eq!(view.page_mode, texcraft_engine::PageMode::TwoContinuous);
    }
    for opted_in in [false, true] {
        let mut view = View {
            sync_after_typeset: opted_in,
            ..Default::default()
        };
        migrate_preferences(
            PERSISTED_VERSION,
            &mut view,
            &mut texcraft_project::Settings::default(),
        );
        assert_eq!(
            view.sync_after_typeset, opted_in,
            "later explicit choices persist"
        );
    }
}

#[test]
fn a_new_template_window_starts_locally_without_requesting_another_window() {
    let mut session = Session::empty(Arc::new(|| {}));
    session.settings.live_typeset = false;
    open_startup(
        &mut session,
        &Options {
            template: Some("blank".into()),
            ..Default::default()
        },
    );
    assert_eq!(session.docs.len(), 1);
    assert!(session.doc().unwrap().path.is_none());
    assert!(session.project_dir.is_none());
    assert!(
        !session
            .effects
            .iter()
            .any(|effect| matches!(effect, texcraft_engine::Effect::NewProjectWindow(_)))
    );
}

#[test]
fn independent_project_windows_get_only_the_selected_project_and_a_separate_control_port() {
    let exe = std::path::Path::new("texcraft");
    let path = std::path::Path::new("project with spaces/main.tex");
    let token = std::path::Path::new("control.token");
    let command = project_windows::command(exe, path, Some(token));
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(command.get_program(), exe.as_os_str());
    assert_eq!(
        args,
        [
            path.as_os_str(),
            std::ffi::OsStr::new("--no-welcome"),
            std::ffi::OsStr::new("--control"),
            std::ffi::OsStr::new("0"),
            std::ffi::OsStr::new("--control-token-file"),
            token.as_os_str()
        ]
    );
    let command = project_windows::command(exe, path, None);
    assert_eq!(command.get_args().count(), 2);
}

#[test]
fn startup_waits_for_a_project_choice_with_legacy_or_saved_workspaces() {
    let dir = std::env::temp_dir().join(format!("texcraft-startup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    let file = dir.join("main.tex");
    let pdf = dir.join("paper.pdf");
    std::fs::write(&file, "\\documentclass{article}\n").unwrap();
    std::fs::write(&pdf, b"%PDF-1.5").unwrap();
    for workspace in [
        Value::Null,
        json!({}),
        json!({
            "project_dir": dir,
            "documents": [{ "path": file }],
            "active": file,
            "root": file
        }),
        json!({ "project_dir": dir }),
        json!({ "pdf": pdf }),
    ] {
        let mut data = json!({
            "view": null, "settings": null, "recent": [file], "split": null,
            "version": 2, "workspace": workspace
        });
        if workspace.is_null() {
            data.as_object_mut().unwrap().remove("workspace");
        }
        let saved: Persisted = serde_json::from_value(data).unwrap();
        let mut session = Session::empty(Arc::new(|| {}));
        session.set_recent(saved.recent);
        open_startup(&mut session, &Options::default());
        assert!(session.docs.is_empty());
        assert!(session.project_dir.is_none() && session.pdf.path.is_none());
        assert!(session.root_path().is_none() && session.typeset.live_due.is_none());
        assert_eq!(session.recent, vec![file.clone()]);
        assert!(session.view.show_welcome);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn explicit_launch_paths_still_open_the_selected_files() {
    let dir = std::env::temp_dir().join(format!("texcraft-launch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let files = [dir.join("main.tex"), dir.join("chapter.tex")];
    for file in &files {
        std::fs::write(file, "selected project").unwrap();
    }
    let mut session = Session::empty(Arc::new(|| {}));
    session.settings.live_typeset = false;
    open_startup(
        &mut session,
        &Options {
            paths: files.to_vec(),
            ..Default::default()
        },
    );
    assert_eq!(session.docs.len(), 2);
    assert_eq!(session.doc().unwrap().text, "selected project");
    assert!(session.status.is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_explicit_launch_path_reports_an_error_without_opening_a_recent_project() {
    let mut session = Session::empty(Arc::new(|| {}));
    session.set_recent(vec![std::env::temp_dir()]);
    open_startup(
        &mut session,
        &Options {
            paths: vec![PathBuf::from("/nonexistent/texcraft-launch.tex")],
            ..Default::default()
        },
    );
    assert!(session.docs.is_empty() && session.project_dir.is_none());
    assert!(session.status.is_some());
}

/// Every launch without an explicit document starts at project selection, even with a legacy
/// disabled welcome preference and a valid recent project. A rootless folder is no editor.
#[test]
fn empty_launches_always_offer_the_start_page_instead_of_reopening_or_showing_a_blank_editor() {
    let dir = std::env::temp_dir().join(format!("texcraft-reopen-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.tex");
    std::fs::write(
        &file,
        "\\documentclass{article}\n\\begin{document}\nHi\n\\end{document}\n",
    )
    .unwrap();
    for old_preference in [true, false] {
        let mut session = Session::empty(Arc::new(|| {}));
        session.settings.live_typeset = false;
        session.view.show_welcome = old_preference;
        session.recent = vec![dir.join("gone.tex"), file.clone()];
        open_startup(&mut session, &Options::default());
        assert!(session.docs.is_empty());
        assert!(needs_start_page(&session));
        session.project_dir = Some(dir.clone());
        assert!(
            needs_start_page(&session),
            "an empty folder must not bypass project selection"
        );
    }
    let mut explicit = Session::empty(Arc::new(|| {}));
    explicit.settings.live_typeset = false;
    open_startup(
        &mut explicit,
        &Options {
            paths: vec![file],
            ..Default::default()
        },
    );
    assert!(!needs_start_page(&explicit));
    std::fs::remove_dir_all(dir).unwrap();
}

/// When no window could start for a file (saved into another project, say), an empty window
/// shows it itself; one with a project of its own leaves it (the user is told where it is).
#[test]
fn a_window_that_could_not_start_falls_back_to_an_empty_one() {
    let dir = std::env::temp_dir().join(format!("texcraft-fallback-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    for p in ["a/main.tex", "b/main.tex"] {
        std::fs::write(
            dir.join(p),
            "\\documentclass{article}\n\\begin{document}\nHi\n\\end{document}\n",
        )
        .unwrap();
    }
    let mut empty = Session::empty(Arc::new(|| {}));
    empty.settings.live_typeset = false;
    assert!(open_in_empty_window(&mut empty, &dir.join("b/main.tex")));
    assert_eq!(empty.docs.len(), 1);
    let mut busy = Session::empty(Arc::new(|| {}));
    busy.settings.live_typeset = false;
    busy.open_path(&dir.join("a/main.tex")).unwrap();
    assert!(!open_in_empty_window(&mut busy, &dir.join("b/main.tex")));
    assert_eq!(busy.docs.len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}
