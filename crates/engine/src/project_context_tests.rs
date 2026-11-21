use super::*;
use serde_json::{Value, json};

fn fixture(name: &str) -> (Session, PathBuf) {
    let base = std::env::temp_dir().join(format!(
        "texcraft-context-audit-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    for folder in ["a", "b"] {
        let dir = base.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\n\\end{document}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("chapter.tex"),
            "% !TEX root = main.tex\nChapter.\n",
        )
        .unwrap();
    }
    let base = base.canonicalize().unwrap();
    let mut session = Session::empty(Arc::new(|| {}));
    session.settings.live_typeset = false;
    session.update.enabled = false;
    session.open_path(&base.join("a/main.tex")).unwrap();
    (session, base)
}

fn message(session: &Session, role: texcraft_ai::Role, text: &str) -> ai::ChatMessage {
    ai::ChatMessage {
        source: Some(session.ai_source()),
        role,
        text: text.into(),
        blocks: Vec::new(),
        model: None,
        provider: None,
        pending: false,
        thinking: false,
        error: None,
        note: None,
        images: Vec::new(),
        tools: Vec::new(),
    }
}

fn model(session: &mut Session, port: u16) {
    let preset = texcraft_ai::PRESETS
        .iter()
        .find(|preset| preset.id == "ollama")
        .unwrap();
    let index = session.ai.config.add(preset);
    session.ai.config.providers[index].base_url = format!("http://127.0.0.1:{port}/v1");
    session.ai.config.providers[index].models = vec!["audit-model".into()];
    session.ai.config.inline.enabled = false;
    session.ai.config.normalize();
}

#[test]
fn new_documents_and_templates_do_not_append_a_new_project_to_an_existing_window() {
    let (mut session, base) = fixture("new-window");
    for (command, params) in [
        ("file.new", Value::Null),
        ("file.newFromTemplate", json!({"id": "blank"})),
    ] {
        session.run(command, &params).unwrap();
        assert_eq!(session.docs.len(), 1);
        assert_eq!(session.project_dir, Some(base.join("a")));
        assert!(matches!(
            session.effects.back(),
            Some(Effect::NewProjectWindow(_))
        ));
    }
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn a_typeset_untitled_document_never_exposes_its_temporary_folder_to_ai_tools() {
    let mut session = Session::empty(Arc::new(|| {}));
    session.settings.live_typeset = false;
    session.new_untitled("Unsaved draft".into());
    let scratch = session.doc().unwrap().scratch.clone().unwrap();
    std::fs::create_dir_all(scratch.parent().unwrap()).unwrap();
    std::fs::write(&scratch, "Unsaved draft").unwrap();
    assert!(session.git_dir().is_none());
    assert!(session.ai_project_root().is_none());
    assert_eq!(session.ai_editor_context()["file"], "Untitled");
    std::fs::remove_dir_all(scratch.parent().unwrap()).unwrap();
}

#[test]
fn independent_roots_in_one_folder_keep_their_typeset_target_and_history_separate() {
    let (mut first, base) = fixture("same-folder");
    let alternate = base.join("a/alternate.tex");
    std::fs::write(
        &alternate,
        "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\n\\end{document}",
    )
    .unwrap();
    first.set_chat_store(base.join("chats"));
    first.ai.messages = vec![
        message(&first, texcraft_ai::Role::User, "First root question"),
        message(&first, texcraft_ai::Role::Assistant, "First root answer"),
    ];
    first.ai_save_chat();
    let id = first.ai.chat_id.clone().unwrap();
    let mut second = Session::empty(Arc::new(|| {}));
    second.settings.live_typeset = false;
    second.open_path(&alternate).unwrap();
    second.set_chat_store(base.join("chats"));
    second.open_path(&base.join("a/chapter.tex")).unwrap();
    assert_eq!(second.root_path(), Some(alternate));
    assert!(second.ai.history.is_empty());
    assert!(second.ai_open_chat(&id).is_err());
    assert!(first.ai.history.iter().any(|chat| chat.id == id));
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn history_and_answer_sources_follow_a_project_directory_rename() {
    let (mut session, base) = fixture("history-rename");
    session.set_chat_store(base.join("chats"));
    session.ai.messages = vec![
        message(&session, texcraft_ai::Role::User, "Question A"),
        message(&session, texcraft_ai::Role::Assistant, "% Audited answer"),
    ];
    session.ai_save_chat();
    let id = session.ai.chat_id.clone().unwrap();
    session.rename_file(&base.join("a"), "renamed").unwrap();
    assert!(session.ai.history.iter().any(|chat| chat.id == id));
    session.ai_open_chat(&id).unwrap();
    let source = session.ai.messages[1].source.as_ref().unwrap();
    assert_eq!(source.project, Some(base.join("renamed")));
    assert_eq!(source.file, Some(base.join("renamed/main.tex")));
    session.ai_insert(1).unwrap();
    assert!(session.doc().unwrap().text.contains("Audited answer"));
    session.rename_file(&base.join("renamed"), "again").unwrap();
    assert_eq!(session.ai.chats.as_ref().unwrap().sweep(), 0);
    session.ai_open_chat(&id).unwrap();
    assert_eq!(
        session.ai.messages[1].source.as_ref().unwrap().file,
        Some(base.join("again/main.tex"))
    );
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn renaming_into_a_previously_used_path_does_not_inherit_that_old_projects_history() {
    let (mut session, base) = fixture("retained-history");
    let store = chats::ChatStore::new(base.join("chats"));
    store
        .save(
            Some(&base.join("renamed/main.tex")),
            "stale",
            &[message(
                &session,
                texcraft_ai::Role::User,
                "Previously removed project",
            )],
            &Default::default(),
        )
        .unwrap();
    session.set_chat_store(base.join("chats"));
    // set_chat_store sweeps removed projects; recreate a cached destination after the sweep.
    store
        .save(
            Some(&base.join("renamed/main.tex")),
            "stale",
            &[message(
                &session,
                texcraft_ai::Role::User,
                "Previously removed project",
            )],
            &Default::default(),
        )
        .unwrap();
    session.ai.messages.push(message(
        &session,
        texcraft_ai::Role::User,
        "Current project",
    ));
    session.ai_save_chat();
    let current = session.ai.chat_id.clone().unwrap();
    session.rename_file(&base.join("a"), "renamed").unwrap();
    assert!(session.ai.history.iter().any(|chat| chat.id == current));
    assert!(session.ai.history.iter().all(|chat| chat.id != "stale"));
    assert!(
        std::fs::read_dir(base.join("chats"))
            .unwrap()
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("_retained-"))
    );
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn old_directory_histories_are_not_assigned_to_an_ambiguous_project_root() {
    let (mut session, base) = fixture("legacy-history");
    let store = chats::ChatStore::new(base.join("chats"));
    let mut messages = vec![
        message(&session, texcraft_ai::Role::User, "Old question"),
        message(&session, texcraft_ai::Role::Assistant, "Old answer"),
    ];
    for message in &mut messages {
        message.source = None;
    }
    store
        .save(
            Some(&base.join("a")),
            "legacy",
            &messages,
            &Default::default(),
        )
        .unwrap();
    session.set_chat_store(base.join("chats"));
    session.ai_open_chat("legacy").unwrap();
    assert!(session.ai_insert(1).is_err());
    std::fs::write(base.join("a/alternate.tex"), "\\documentclass{article}").unwrap();
    let mut other = Session::empty(Arc::new(|| {}));
    other.settings.live_typeset = false;
    other.open_path(&base.join("a/alternate.tex")).unwrap();
    other.set_chat_store(base.join("chats"));
    assert!(other.ai.history.is_empty());
    assert!(other.ai_open_chat("legacy").is_err());
    assert!(
        store.load(Some(&base.join("a")), "legacy").is_some(),
        "ambiguous legacy data is retained"
    );
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn legacy_history_survives_repeated_project_directory_renames_and_cleanup() {
    let (mut session, base) = fixture("legacy-moves");
    let store = chats::ChatStore::new(base.join("chats"));
    store
        .save(
            Some(&base.join("a")),
            "legacy",
            &[message(
                &session,
                texcraft_ai::Role::User,
                "Saved before root-based history",
            )],
            &Default::default(),
        )
        .unwrap();
    session.set_chat_store(base.join("chats"));
    session.rename_file(&base.join("a"), "renamed").unwrap();
    session.rename_file(&base.join("renamed"), "again").unwrap();
    assert_eq!(store.sweep(), 0);
    assert!(session.ai.history.iter().any(|chat| chat.id == "legacy"));
    session.ai_open_chat("legacy").unwrap();
    assert_eq!(
        session.ai.messages[0].source.as_ref().unwrap().project,
        Some(base.join("again"))
    );
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn an_old_answer_cannot_be_inserted_into_a_different_same_named_tab() {
    let (mut session, base) = fixture("insert-source");
    session.ai.messages.push(message(
        &session,
        texcraft_ai::Role::Assistant,
        "% Bound to original",
    ));
    std::fs::create_dir_all(base.join("a/other")).unwrap();
    std::fs::write(
        base.join("a/other/main.tex"),
        "% !TEX root = ../main.tex\nOther file.",
    )
    .unwrap();
    session.open_path(&base.join("a/other/main.tex")).unwrap();
    let before = session.doc().unwrap().text.clone();
    assert!(session.ai_insert(0).is_err());
    assert_eq!(session.doc().unwrap().text, before);
    session.active = 0;
    session.ai_insert(0).unwrap();
    assert!(session.doc().unwrap().text.contains("Bound to original"));
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn inconsistent_active_file_and_project_are_rejected_before_any_model_request() {
    let (mut session, base) = fixture("invalid-scope");
    session.set_project_dir(base.join("b"));
    let error = session
        .ai_send("Read the current project")
        .unwrap_err()
        .to_string();
    assert!(error.contains("does not belong"), "{error}");
    assert!(session.ai.messages.is_empty() && !session.ai.is_busy());
    assert!(session.ai_run_action("polish").is_err());
    assert!(session.inline_trigger().is_err());
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn an_old_window_can_finish_its_tool_round_without_reading_the_new_windows_project() {
    let (mut session, base) = fixture("pending-window");
    std::fs::write(base.join("a/only_a.txt"), "A").unwrap();
    std::fs::write(base.join("b/only_b.txt"), "B").unwrap();
    let first = ai_tools::tests::sse(
        json!({"tool_calls": [{"index":0,"id":"list","type":"function","function":{"name":"list_files","arguments":"{}"}}]}),
        Some("tool_calls"),
    );
    let second = ai_tools::tests::sse(json!({"content":"Answer for A"}), Some("stop"));
    let (port, server) = ai_tools::tests::fake_model(vec![first, second]);
    model(&mut session, port);
    session.ai_send("List this project's files").unwrap();
    session
        .run("file.openRecent", &json!({"path":base.join("b/main.tex")}))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while session.ai.is_busy() && Instant::now() < deadline {
        session.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!session.ai.is_busy());
    let requests = server.join().unwrap();
    assert!(requests[1].contains("only_a.txt") && !requests[1].contains("only_b.txt"));
    assert_eq!(
        session
            .ai
            .messages
            .last()
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .project,
        Some(base.join("a"))
    );
    assert!(
        session
            .effects
            .contains(&Effect::OpenProjectWindow(base.join("b/main.tex")))
    );
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn changing_directory_discards_an_already_returning_old_tool_call_before_execution() {
    let (mut session, base) = fixture("late-tools");
    let body = ai_tools::tests::sse(
        json!({"tool_calls": [{"index":0,"id":"late","type":"function","function":{"name":"list_files","arguments":"{}"}}]}),
        Some("tool_calls"),
    );
    let (port, server) = ai_tools::tests::fake_model(vec![body]);
    model(&mut session, port);
    session.ai_send("Old A request").unwrap();
    server.join().unwrap();
    session.set_project_dir(base.join("b"));
    session.tick(Instant::now());
    assert!(!session.ai.is_busy() && session.ai.messages.is_empty());
    assert_eq!(session.ai_project_root(), Some(base.join("b")));
    std::fs::remove_dir_all(base).unwrap();
}

#[cfg(unix)]
#[test]
fn a_symlink_to_another_project_is_routed_to_that_projects_own_window() {
    let (mut session, base) = fixture("symlink");
    let link = base.join("a/other-project.tex");
    std::os::unix::fs::symlink(base.join("b/main.tex"), &link).unwrap();
    assert_eq!(session.open_path(&link).unwrap(), OpenResult::NewWindow);
    assert_eq!(session.docs.len(), 1);
    assert!(
        session
            .effects
            .contains(&Effect::OpenProjectWindow(base.join("b/main.tex")))
    );
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn closing_the_last_tab_clears_ai_context_before_another_command_can_run() {
    let (mut session, base) = fixture("last-tab");
    session.ai.draft = "Private notes about A".into();
    session.close(0);
    assert!(session.ai.draft.is_empty());
    assert_eq!(session.ai_project_root(), None);
    session.open_path(&base.join("b/main.tex")).unwrap();
    assert_eq!(session.ai_project_root(), Some(base.join("b")));
    assert!(session.ai.messages.is_empty());
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn a_late_old_typeset_cannot_replace_the_new_projects_issues_or_open_its_results_sidebar() {
    let (mut session, base) = fixture("late-typeset");
    session.typeset.console = "Old private compile log".into();
    session.typeset.last = Some(Outcome::default());
    session.close(0);
    assert!(session.typeset.console.is_empty() && session.typeset.last.is_none());
    session.open_path(&base.join("b/main.tex")).unwrap();
    session.typeset.root = Some(base.join("a/main.tex"));
    session.view.sidebar_tab = SidebarTab::Files;
    session.finish_typeset(Outcome {
        report: texcraft_log::parse_log("! Old project error.\nl.1 \\bad", &base.join("a")),
        ..Default::default()
    });
    assert!(session.typeset.issues.is_empty());
    assert!(session.typeset.last.is_none());
    assert_eq!(session.view.sidebar_tab, SidebarTab::Files);
    assert_eq!(session.ai_project_root(), Some(base.join("b")));
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn a_real_inflight_compile_publishes_only_its_original_pdf_after_the_project_is_closed() {
    let (mut session, base) = fixture("inflight-compile");
    if !session.toolchain.supports(Engine::PdfLatex) {
        std::fs::remove_dir_all(base).unwrap();
        return;
    }
    session.typeset().unwrap();
    session.close(0);
    session.open_path(&base.join("b/main.tex")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while session.typeset.is_running() && Instant::now() < deadline {
        session.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!session.typeset.is_running());
    assert!(texcraft_project::published_pdf(&base.join("a/main.tex")).is_file());
    assert!(session.typeset.issues.is_empty() && session.typeset.console.is_empty());
    assert!(session.typeset.last.is_none() && session.pdf.path.is_none());
    assert_eq!(session.root_path(), Some(base.join("b/main.tex")));
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn saving_a_root_into_another_project_never_rebinds_its_tab_to_the_old_directory() {
    let (mut session, base) = fixture("save-as");
    session.open_path(&base.join("a/chapter.tex")).unwrap();
    session.active = 0;
    session
        .run("edit.insertText", &json!({"text": "% Latest unsaved A\n"}))
        .unwrap();
    session.save_as(0, base.join("b/copied.tex")).unwrap();
    assert!(
        std::fs::read_to_string(base.join("b/copied.tex"))
            .unwrap()
            .contains("Latest unsaved A")
    );
    assert!(
        !std::fs::read_to_string(base.join("a/main.tex"))
            .unwrap()
            .contains("Latest unsaved A")
    );
    assert!(session.docs.iter().all(|doc| {
        doc.path
            .as_ref()
            .is_some_and(|path| path.starts_with(base.join("a")))
    }));
    assert_eq!(session.ai_project_root(), Some(base.join("a")));
    assert!(
        session
            .effects
            .contains(&Effect::OpenProjectWindow(base.join("b/copied.tex")))
    );
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn renaming_a_project_updates_its_directory_ai_root_paths_and_navigation_together() {
    let (mut session, base) = fixture("rename");
    session.open_path(&base.join("a/chapter.tex")).unwrap();
    session.typeset.root = Some(base.join("a/main.tex"));
    session.history.push((base.join("a/chapter.tex"), 2));
    session.rename_file(&base.join("a"), "renamed").unwrap();
    assert_eq!(session.project_dir, Some(base.join("renamed")));
    assert_eq!(session.ai_project_root(), Some(base.join("renamed")));
    assert_eq!(session.typeset.root, Some(base.join("renamed/main.tex")));
    assert!(
        session
            .recent
            .iter()
            .all(|path| path.starts_with(base.join("renamed")))
    );
    assert_eq!(session.history[0].0, base.join("renamed/chapter.tex"));
    std::fs::remove_dir_all(base).unwrap();
}
