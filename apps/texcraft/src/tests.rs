use super::*;

#[test]
fn startup_migrates_legacy_recents_but_honours_an_explicitly_closed_workspace() {
    let file = std::env::temp_dir().join(format!("texcraft-startup-{}.tex", std::process::id()));
    std::fs::write(&file, "saved project").unwrap();
    let legacy: Persisted = serde_json::from_value(json!({
        "view": null, "settings": null, "recent": [file], "split": null, "version": 2
    }))
    .unwrap();
    assert!(legacy.workspace.is_none());
    let mut session = Session::empty(Arc::new(|| {}));
    session.settings.live_typeset = false;
    session.set_recent(legacy.recent);
    open_startup(&mut session, &Options::default(), legacy.workspace.as_ref());
    assert_eq!(session.doc().unwrap().text, "saved project");
    let mut closed = Session::empty(Arc::new(|| {}));
    closed.set_recent(session.recent.clone());
    open_startup(&mut closed, &Options::default(), Some(&Default::default()));
    assert!(closed.docs.is_empty());
    let explicit = Options {
        paths: vec![file.clone()],
        ..Default::default()
    };
    open_startup(&mut closed, &explicit, Some(&Default::default()));
    assert_eq!(closed.docs.len(), 1);
    std::fs::remove_file(file).unwrap();
}

#[test]
fn explicit_launch_path_overrides_saved_workspace_even_when_it_cannot_open() {
    let mut session = Session::empty(Arc::new(|| {}));
    let saved = texcraft_engine::workspace::Workspace {
        project_dir: Some(std::env::temp_dir()),
        ..Default::default()
    };
    open_startup(
        &mut session,
        &Options {
            paths: vec![PathBuf::from("/nonexistent/texcraft-launch.tex")],
            ..Default::default()
        },
        Some(&saved),
    );
    assert!(session.docs.is_empty() && session.project_dir.is_none());
    assert!(session.status.is_some());
}
