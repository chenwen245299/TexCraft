use super::*;
use serde_json::json;
use std::time::{Duration, Instant};

fn session() -> Session {
    let mut s = Session::new(Arc::new(|| {}));
    s.settings.live_typeset = false;
    // Tests never reach the network: update checks read a local file that is not there.
    s.update.url = "/nonexistent/texcraft-latest.json".into();
    s
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("texcraft-engine-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn starts_with_an_untitled_article() {
    let s = session();
    assert_eq!(s.docs.len(), 1);
    assert_eq!(s.doc().unwrap().display_name(), "Untitled");
    assert!(s.doc().unwrap().text.contains("\\documentclass"));
    assert!(s.root_path().is_some());
}

#[test]
fn unknown_and_disabled_commands_are_errors() {
    let mut s = session();
    assert!(matches!(
        s.run("nope", &Value::Null),
        Err(CommandError::Unknown(_))
    ));
    assert!(matches!(
        s.run("typeset.stop", &Value::Null),
        Err(CommandError::Disabled(_))
    ));
    assert!(matches!(
        s.run("edit.gotoLine", &json!({"line": "x"})),
        Ok(())
    ));
    assert!(matches!(
        s.run("pdf.setZoom", &json!({})),
        Err(CommandError::Params(_))
    ));
}

use serde_json::Value;

#[test]
fn every_command_survives_adversarial_params() {
    // No command may panic, whatever the parameters (they may return errors).
    let params = [
        Value::Null,
        json!({}),
        json!({"index": 99999, "line": 0, "page": 0, "x": -1e30, "y": f64::MAX, "zoom": -3}),
        json!({"text": "中文 $1 $0 $SEL", "anchor": 999999, "cursor": 0, "path": "/nonexistent/x.tex", "id": "nope"}),
        json!([1, 2, 3]),
    ];
    for spec in registry() {
        if matches!(spec.id, "app.quit" | "file.closeWindow") {
            continue;
        }
        for p in &params {
            let mut s = session();
            let _ = s.run(spec.id, p);
            while s.take_effect().is_some() {}
        }
    }
}

#[test]
fn formatting_commands_edit_the_selection() {
    let mut s = session();
    s.run("edit.setText", &json!({"text": "hello world"}))
        .unwrap();
    s.run("edit.select", &json!({"anchor": 6, "cursor": 11}))
        .unwrap();
    s.run("snippet.bold", &Value::Null).unwrap();
    assert_eq!(s.doc().unwrap().text, "hello \\textbf{world}");
    s.run("edit.toggleComment", &Value::Null).unwrap();
    assert_eq!(s.doc().unwrap().text, "% hello \\textbf{world}");
    s.run("edit.toggleComment", &Value::Null).unwrap();
    s.run("edit.select", &json!({"anchor": 0})).unwrap();
    s.run("snippet.itemize", &Value::Null).unwrap();
    assert!(
        s.doc()
            .unwrap()
            .text
            .starts_with("\\begin{itemize}\n  \\item ")
    );
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, Effect::SelectionChanged { .. }))
    );
}

#[test]
fn view_commands_toggle_state() {
    let mut s = session();
    assert!(s.view.sidebar);
    s.run("view.toggleSidebar", &Value::Null).unwrap();
    assert!(!s.view.sidebar);
    assert!(!s.is_checked("view.toggleSidebar"));
    s.run("view.pdfOnly", &Value::Null).unwrap();
    assert_eq!(s.view.layout, Layout::PdfOnly);
    assert!(s.is_checked("view.pdfOnly"));
    s.run("view.results", &Value::Null).unwrap();
    assert!(s.view.sidebar && s.view.sidebar_tab == SidebarTab::Results);
    s.pdf_fit_scale = Some(1.3);
    s.run("pdf.zoomIn", &Value::Null).unwrap();
    assert_eq!(s.view.zoom, Zoom::Fixed(1.5));
}

#[test]
fn auto_sense_locks_the_engine_choice() {
    let mut s = session();
    let text = "\\documentclass{article}\n\\begin{document}\n这是一个测试\n\\end{document}\n";
    s.run("edit.setText", &json!({ "text": text })).unwrap();
    // Auto-Sense on: the engine commands are off and tick the engine in use.
    assert!(s.settings.auto_sense);
    assert!(!s.is_enabled("typeset.engine.pdflatex"));
    assert!(matches!(
        s.run("typeset.engine.pdflatex", &Value::Null),
        Err(CommandError::Disabled(_))
    ));
    assert_eq!(s.settings.engine, Engine::PdfLatex);
    assert!(s.is_checked("typeset.engine.xelatex"));
    assert!(!s.is_checked("typeset.engine.pdflatex"));
    // Auto-Sense off: the engine is chosen by hand.
    s.settings.auto_sense = false;
    assert!(s.is_checked("typeset.engine.pdflatex"));
    s.run("typeset.engine.lualatex", &Value::Null).unwrap();
    assert!(s.is_checked("typeset.engine.lualatex"));
    assert_eq!(s.effective_engine(), Engine::LuaLatex);
}

#[test]
fn opens_a_project_folder_and_finds_the_root() {
    let dir = temp_dir("open");
    std::fs::create_dir_all(dir.join("chapters")).unwrap();
    std::fs::write(
        dir.join("thesis.tex"),
        "\\documentclass{book}\n\\begin{document}\n\\include{chapters/one}\n\\end{document}\n",
    )
    .unwrap();
    std::fs::write(dir.join("chapters/one.tex"), "\\chapter{One}\nText.\n").unwrap();
    let mut s = session();
    s.run("file.openPath", &json!({"path": dir.to_string_lossy()}))
        .unwrap();
    assert_eq!(s.doc().unwrap().display_name(), "thesis.tex");
    assert_eq!(
        s.docs.len(),
        1,
        "the pristine untitled document is replaced"
    );
    let one = std::fs::canonicalize(dir.join("chapters/one.tex")).unwrap();
    s.jump(&one, 2).unwrap();
    assert_eq!(s.doc().unwrap().display_name(), "one.tex");
    assert_eq!(s.doc().unwrap().caret_line(), 2);
    assert_eq!(s.root_path().unwrap().file_name().unwrap(), "thesis.tex");
    assert!(s.is_enabled("nav.back"));
    s.run("nav.back", &Value::Null).unwrap();
    assert_eq!(s.doc().unwrap().display_name(), "thesis.tex");
    std::fs::remove_dir_all(&dir).ok();
}

/// Sessions side by side (parallel tests) never typeset untitled documents into one folder: a
/// per-session count gave both `texcraft-<pid>-untitled-1`, and their runs clashed.
#[test]
fn untitled_documents_of_two_sessions_have_their_own_folders() {
    let a = session();
    let b = session();
    let dir = |s: &Session| s.doc().and_then(|d| d.scratch.clone()).unwrap();
    assert_ne!(dir(&a), dir(&b));
}

#[test]
fn typesets_an_untitled_document_when_tex_is_installed() {
    let mut s = session();
    if !s.toolchain.supports(Engine::PdfLatex) {
        eprintln!("skipping: no TeX");
        return;
    }
    s.run("edit.setText", &json!({"text": "\\documentclass{article}\n\\begin{document}\nHi \\oops.\n\\end{document}\n"})).unwrap();
    s.run("typeset.run", &Value::Null).unwrap();
    assert!(s.typeset.is_running());
    let deadline = Instant::now() + Duration::from_secs(60);
    while s.typeset.is_running() && Instant::now() < deadline {
        s.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!s.typeset.is_running());
    let o = s.typeset.last.as_ref().unwrap();
    assert!(o.error.is_none(), "{:?}", o.error);
    assert!(
        s.pdf.path.as_ref().is_some_and(|p| p.is_file()),
        "{}",
        s.typeset.console
    );
    assert!(s.effects.iter().any(|e| *e == Effect::PdfChanged));
    assert!(s.typeset.console.contains("latexmk"));
    let inspect = s.inspect();
    assert_eq!(inspect["typeset"]["runs"], 1);
}

#[test]
fn chinese_text_switches_to_xelatex_and_live_previews() {
    let mut s = session();
    let text = "\\documentclass[11pt,a4paper]{article}\n\\usepackage[utf8]{inputenc}\n\\usepackage[T1]{fontenc}\n\\usepackage{hyperref}\n\\begin{document}\n\\section{Method}\n这是一个测试\n\\end{document}\n";
    s.run("edit.setText", &json!({ "text": text })).unwrap();
    let plan = s.typeset_plan();
    assert_eq!(plan.engine, Engine::XeLatex);
    assert!(plan.add_cjk);
    assert_eq!(s.inspect()["cjkSetup"], true);
    if !s.toolchain.supports(Engine::XeLatex) || s.toolchain.cjk_pretex().is_none() {
        eprintln!("skipping the live pass: no XeLaTeX or CJK fonts");
        return;
    }
    s.live_typeset().unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while s.typeset.is_running() && Instant::now() < deadline {
        s.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
    let o = s.typeset.last.as_ref().unwrap();
    assert!(o.live && o.success, "{}", o.console);
    assert_eq!(o.report.errors(), 0, "{:?}", o.report.issues);
    assert!(o.pdf.as_ref().is_some_and(|p| p.is_file()));
}

#[test]
fn live_typeset_is_scheduled_after_edits() {
    let mut s = session();
    s.settings.live_typeset = true;
    s.settings.live_delay_ms = 50;
    s.note_edit();
    assert!(s.typeset.live_due.is_some());
    s.run("typeset.pauseLive", &Value::Null).unwrap();
    assert!(s.typeset.live_due.is_none());
    s.note_edit();
    assert!(s.typeset.live_due.is_none(), "paused");
}

#[test]
fn typeset_modes_switch_by_command() {
    let mut s = session();
    s.run("typeset.mode.live", &Value::Null).unwrap();
    assert_eq!(s.inspect()["typeset"]["mode"], "live");
    assert!(s.is_checked("typeset.mode.live"));
    s.note_edit();
    assert!(s.typeset.live_due.is_some(), "LIVE follows typing");
    s.run("typeset.mode.manual", &Value::Null).unwrap();
    assert!(
        s.typeset.live_due.is_none(),
        "MANUAL drops the pending pass"
    );
    s.note_edit();
    assert!(
        s.typeset.live_due.is_none(),
        "MANUAL never typesets by itself"
    );
    // ⇧⌘L returns to the previous automatic mode.
    s.run("typeset.toggleLive", &Value::Null).unwrap();
    assert_eq!(s.settings.mode(), texcraft_project::TypesetMode::Live);
    s.run("typeset.mode.auto", &Value::Null).unwrap();
    assert_eq!(s.settings.typesetter, Typesetter::External);
    // A click on the toolbar label: AUTO → MANUAL → LIVE.
    s.run("typeset.cycleMode", &Value::Null).unwrap();
    assert_eq!(s.inspect()["typeset"]["mode"], "manual");
    s.run("typeset.cycleMode", &Value::Null).unwrap();
    assert_eq!(s.inspect()["typeset"]["mode"], "live");
    s.run("typeset.mode.auto", &Value::Null).unwrap();
    assert!(s.is_checked("typeset.mode.auto") && !s.is_checked("typeset.mode.live"));
}

#[test]
fn typeset_in_live_mode_writes_the_pdf_even_during_a_live_pass() {
    let mut s = session();
    if !s.toolchain.supports(Engine::PdfLatex) {
        eprintln!("skipping: no TeX");
        return;
    }
    let dir = temp_dir("live-full");
    let main = dir.join("main.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\nSaved.\n\\end{document}\n",
    )
    .unwrap();
    s.run("file.openPath", &json!({"path": main.to_string_lossy()}))
        .unwrap();
    s.run("typeset.mode.live", &Value::Null).unwrap();
    s.run(
        "edit.setText",
        &json!({"text": "\\documentclass{article}\n\\begin{document}\nTyped.\n\\end{document}\n"}),
    )
    .unwrap();
    s.live_typeset().unwrap();
    assert!(s.typeset.is_running() && !s.typeset.is_full_running());
    // ⌘T while the live pass runs: it gives way to a full typeset instead of aborting.
    s.run("typeset.run", &Value::Null).unwrap();
    assert!(s.typeset.queued);
    let deadline = Instant::now() + Duration::from_secs(120);
    while (s.typeset.is_running() || s.typeset.queued) && Instant::now() < deadline {
        s.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
    let o = s.typeset.last.as_ref().unwrap();
    assert!(!o.live && o.success, "{}", o.console);
    let pdf = texcraft_project::output_pdf(&main);
    assert!(pdf.is_file(), "{}", pdf.display());
    assert_eq!(
        s.pdf.path.as_ref().map(|p| p.canonicalize().unwrap()),
        Some(pdf.canonicalize().unwrap())
    );
    // The result also lands beside the source, where Finder shows it.
    let beside = texcraft_project::published_pdf(&main);
    assert_eq!(
        std::fs::read(&beside).unwrap(),
        std::fs::read(&pdf).unwrap()
    );
    assert_eq!(
        s.pdf_file().map(|p| p.canonicalize().unwrap()),
        Some(beside.canonicalize().unwrap())
    );
    assert!(
        std::fs::read_to_string(&main).unwrap().contains("Typed."),
        "saved first"
    );
    assert_ne!(
        s.status.as_ref().map(|m| m.0.as_str()),
        Some("Typesetting stopped")
    );
    // Pressing ⌘T during a full typeset still aborts it.
    s.run("typeset.run", &Value::Null).unwrap();
    assert!(s.typeset.is_full_running());
    s.run("typeset.run", &Value::Null).unwrap();
    while s.typeset.is_running() {
        s.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(s.typeset.last.as_ref().unwrap().cancelled || s.typeset.last.as_ref().unwrap().success);
}

#[test]
fn live_preview_typesets_unsaved_buffers_without_saving() {
    let mut s = session();
    if !s.toolchain.supports(Engine::PdfLatex) {
        eprintln!("skipping: no TeX");
        return;
    }
    let dir = temp_dir("live");
    std::fs::create_dir_all(dir.join("chapters")).unwrap();
    let main = "\\documentclass{article}\n\\usepackage{amsmath}\n\\begin{document}\n\\input{chapters/one}\n\\end{document}\n";
    std::fs::write(dir.join("main.tex"), main).unwrap();
    std::fs::write(dir.join("chapters/one.tex"), "Saved text.\n").unwrap();
    s.run(
        "file.openPath",
        &json!({"path": dir.join("chapters/one.tex").to_string_lossy()}),
    )
    .unwrap();
    s.refresh_index(true);
    // Edit without saving, then run a live pass twice (the second uses the cached preamble).
    s.run(
        "edit.setText",
        &json!({"text": "Unsaved live text with $x^2$.\n"}),
    )
    .unwrap();
    for pass in 0..2 {
        s.live_typeset().unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        while s.typeset.is_running() && Instant::now() < deadline {
            s.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(10));
        }
        let o = s.typeset.last.as_ref().unwrap();
        assert!(
            o.live && o.error.is_none(),
            "pass {pass}: {:?}\n{}",
            o.error,
            o.console
        );
        assert!(o.success, "pass {pass}: {}", o.console);
        if pass == 1 {
            assert!(
                o.preamble_cached,
                "second pass uses the precompiled preamble"
            );
            assert!(
                o.fast_env,
                "second pass runs in the fast environment: {}",
                o.console
            );
        }
    }
    // A font outside the reduced live font map: the pass falls back to the full map, and the
    // next pass picks the new family up.
    s.run(
        "edit.setText",
        &json!({"text": "Unsaved live text with $x^2$. {\\fontfamily{ptm}\\selectfont Times.}\n"}),
    )
    .unwrap();
    for pass in 0..2 {
        s.live_typeset().unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        while s.typeset.is_running() && Instant::now() < deadline {
            s.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(10));
        }
        let o = s.typeset.last.as_ref().unwrap();
        assert!(o.success, "font pass {pass}: {}", o.console);
        if pass == 1 {
            assert!(
                !o.console.contains("retrying with the full map"),
                "the reduced map now has Times: {}",
                o.console
            );
        }
    }
    s.run(
        "edit.setText",
        &json!({"text": "Unsaved live text with $x^2$.\n"}),
    )
    .unwrap();
    s.live_typeset().unwrap();
    while s.typeset.is_running() {
        s.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
    // The user's file is untouched; the PDF comes from the mirror.
    assert_eq!(
        std::fs::read_to_string(dir.join("chapters/one.tex")).unwrap(),
        "Saved text.\n"
    );
    let pdf = s.pdf.path.clone().unwrap();
    assert!(
        pdf.to_string_lossy().contains(".texcraft/live"),
        "{}",
        pdf.display()
    );
    // SyncTeX maps the mirror back to the real file.
    s.run("edit.select", &json!({"anchor": 3})).unwrap();
    s.forward_sync(true);
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, Effect::ShowInPdf { rects, .. } if !rects.is_empty())),
        "forward search through the mirror"
    );
    let rect = s
        .effects
        .iter()
        .find_map(|e| match e {
            Effect::ShowInPdf { rects, .. } => rects.first().copied(),
            _ => None,
        })
        .unwrap();
    s.active = 0;
    s.inverse_sync(rect.page, rect.x + 2.0, rect.y + rect.height / 2.0)
        .unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(
        doc.display_name(),
        "one.tex",
        "inverse search lands in the real file"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Writes a manifest (and, on macOS, a real app archive) for update tests.
fn update_fixture(name: &str, version: &str) -> PathBuf {
    let dir = temp_dir(name);
    let mut asset = json!({"url": "file:///nonexistent.zip", "sha256": "00", "size": 1});
    if cfg!(target_os = "macos") {
        let app = dir.join("src/TexCraft.app/Contents/MacOS");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("texcraft"), b"new build").unwrap();
        let zip = dir.join("TexCraft.zip");
        let ok = std::process::Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(dir.join("src/TexCraft.app"))
            .arg(&zip)
            .status()
            .unwrap();
        assert!(ok.success());
        asset = json!({
            "url": format!("file://{}", zip.display()),
            "sha256": texcraft_update::sha256_file(&zip).unwrap(),
            "size": std::fs::metadata(&zip).unwrap().len(),
        });
    }
    let manifest = json!({
        "version": version,
        "notes": "Better things.",
        "release_url": "https://example.com",
        "platforms": {"macos": asset.clone(), "windows-x86_64": asset},
    });
    let path = dir.join("latest.json");
    std::fs::write(&path, manifest.to_string()).unwrap();
    path
}

fn settle_update(s: &mut Session) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while s.update.is_busy() && Instant::now() < deadline {
        s.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn update_check_finds_a_newer_release_and_skips_it() {
    let mut s = session();
    s.update.url = update_fixture("update-new", "99.0.0").display().to_string();
    s.run("app.checkForUpdates", &Value::Null).unwrap();
    assert!(s.effects.contains(&Effect::UpdateWindow));
    settle_update(&mut s);
    match &s.update.status {
        UpdateStatus::Available(r) if texcraft_update::platform_key().is_some() => {
            assert_eq!(r.version, "99.0.0");
            assert_eq!(r.notes, "Better things.");
        }
        UpdateStatus::UpToDate if texcraft_update::platform_key().is_none() => {}
        other => panic!("unexpected {other:?}"),
    }
    assert!(s.view.last_update_check > 0);
    if texcraft_update::platform_key().is_none() {
        return;
    }
    s.run("app.skipUpdate", &Value::Null).unwrap();
    assert_eq!(s.view.skipped_update.as_deref(), Some("99.0.0"));
    // An automatic check stays quiet about the skipped version.
    s.check_for_updates(false);
    settle_update(&mut s);
    assert_eq!(s.update.status, UpdateStatus::UpToDate);
}

#[test]
fn update_check_reports_up_to_date_and_errors() {
    let mut s = session();
    s.update.url = update_fixture("update-old", "0.0.1").display().to_string();
    s.check_for_updates(true);
    settle_update(&mut s);
    assert_eq!(s.update.status, UpdateStatus::UpToDate);
    s.update.url = "/nonexistent/latest.json".into();
    s.check_for_updates(true);
    settle_update(&mut s);
    assert!(matches!(s.update.status, UpdateStatus::Failed(_)));
}

#[cfg(target_os = "macos")]
#[test]
fn automatic_check_downloads_and_stages_the_update() {
    let mut s = session();
    s.update.url = update_fixture("update-auto", "99.0.0")
        .display()
        .to_string();
    s.view.auto_download_updates = true;
    s.check_for_updates(false);
    // Check, then the download it starts by itself.
    settle_update(&mut s);
    settle_update(&mut s);
    match &s.update.status {
        UpdateStatus::Ready(staged) => {
            assert_eq!(staged.release.version, "99.0.0");
            assert_eq!(
                std::fs::read(staged.path.join("Contents/MacOS/texcraft")).unwrap(),
                b"new build"
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(s.effects.contains(&Effect::UpdateWindow));
    // A test binary is not inside an app bundle: installing explains why it cannot.
    assert!(s.run("app.installUpdate", &Value::Null).is_err());
}

#[test]
fn every_start_up_checks_and_shows_a_newer_release_at_once() {
    if texcraft_update::platform_key().is_none() {
        return;
    }
    let mut s = session();
    // Its own version: the download folder is per version, and the tests run in parallel.
    s.update.url = update_fixture("update-launch", "98.0.0")
        .display()
        .to_string();
    s.update.enabled = true;
    s.view.auto_check_updates = true;
    s.view.auto_download_updates = true;
    // An earlier run checked a minute ago: start-up checks all the same.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    s.view.last_update_check = now - 60;
    let later = Instant::now() + Duration::from_secs(4);
    s.tick(later);
    assert!(s.update.is_busy(), "the start-up check runs");
    let deadline = Instant::now() + Duration::from_secs(30);
    while s.update.status == UpdateStatus::Checking && Instant::now() < deadline {
        s.tick(later);
        std::thread::sleep(Duration::from_millis(10));
    }
    // Found: the window opens now, while the download runs (not only once it is ready).
    assert!(
        matches!(s.update.status, UpdateStatus::Downloading { .. }),
        "{:?}",
        s.update.status
    );
    assert!(s.effects.contains(&Effect::UpdateWindow));
    settle_update(&mut s);
    // Only the first tick of a run is a start-up check.
    s.update.status = UpdateStatus::UpToDate;
    s.tick(later);
    assert!(!s.update.is_busy());
}

#[test]
fn recent_documents_list_a_project_once_by_its_main_file() {
    let mut s = session();
    // Opened paths are canonical (/private/var, not /var).
    let dir = temp_dir("recent").canonicalize().unwrap();
    std::fs::create_dir_all(dir.join("sections")).unwrap();
    let main = dir.join("main.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\n\\input{sections/intro}\n\\end{document}\n",
    )
    .unwrap();
    std::fs::write(dir.join("sections/intro.tex"), "Intro.\n").unwrap();
    std::fs::write(dir.join("refs.bib"), "@book{k, title={T}}\n").unwrap();
    for f in ["sections/intro.tex", "refs.bib", "main.tex"] {
        s.run(
            "file.openPath",
            &json!({"path": dir.join(f).to_string_lossy()}),
        )
        .unwrap();
        assert_eq!(s.recent, vec![main.clone()], "after opening {f}");
    }
    // A saved list from before is tidied the same way.
    s.set_recent(vec![
        dir.join("refs.bib"),
        main.clone(),
        dir.join("sections/intro.tex"),
    ]);
    assert_eq!(s.recent, vec![main.clone()]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn files_context_menu_commands_rename_duplicate_trash_and_insert() {
    let mut s = session();
    let dir = temp_dir("files-menu");
    std::fs::create_dir_all(dir.join("figs")).unwrap();
    let main = dir.join("main.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\n\n\\end{document}\n",
    )
    .unwrap();
    std::fs::write(dir.join("figs/plot.png"), [0u8; 8]).unwrap();
    std::fs::write(dir.join("intro.tex"), "Intro.\n").unwrap();
    s.run("file.openPath", &json!({"path": main.to_string_lossy()}))
        .unwrap();
    s.set_project_dir(dir.clone());
    // Insert in the Editor: a path relative to the document.
    s.run("edit.select", &json!({"anchor": 41})).unwrap();
    s.run(
        "files.insert",
        &json!({"path": dir.join("figs/plot.png").to_string_lossy()}),
    )
    .unwrap();
    assert!(
        s.doc()
            .unwrap()
            .text
            .contains("\\includegraphics{figs/plot.png}"),
        "{}",
        s.doc().unwrap().text
    );
    // Rename an open file: the document follows.
    s.run(
        "file.openPath",
        &json!({"path": dir.join("intro.tex").to_string_lossy()}),
    )
    .unwrap();
    s.run(
        "files.rename",
        &json!({"path": dir.join("intro.tex").to_string_lossy(), "name": "introduction.tex"}),
    )
    .unwrap();
    assert!(dir.join("introduction.tex").is_file() && !dir.join("intro.tex").exists());
    assert_eq!(
        s.doc().unwrap().path.clone(),
        Some(dir.join("introduction.tex").canonicalize().unwrap())
    );
    assert!(
        s.run(
            "files.rename",
            &json!({"path": dir.join("introduction.tex").to_string_lossy(), "name": "main.tex"}),
        )
        .is_err(),
        "never overwrites"
    );
    assert!(
        s.run(
            "files.rename",
            &json!({"path": dir.join("introduction.tex").to_string_lossy(), "name": "a/b"}),
        )
        .is_err()
    );
    // Duplicate.
    s.run(
        "files.duplicate",
        &json!({"path": dir.join("introduction.tex").to_string_lossy()}),
    )
    .unwrap();
    assert!(dir.join("introduction copy.tex").is_file());
    assert!(
        s.file_tree
            .iter()
            .any(|n| n.name == "introduction copy.tex")
    );
    // Move to Trash (a fake that deletes): the open document closes.
    s.trash = |p| std::fs::remove_file(p).map_err(|e| e.to_string());
    let open_before = s.docs.len();
    s.run(
        "files.trash",
        &json!({"path": dir.join("introduction.tex").to_string_lossy()}),
    )
    .unwrap();
    assert!(!dir.join("introduction.tex").exists());
    assert_eq!(s.docs.len(), open_before - 1);
    // An open file with unsaved changes is not trashed.
    s.run("file.openPath", &json!({"path": main.to_string_lossy()}))
        .unwrap();
    assert!(s.doc().unwrap().is_dirty());
    assert!(
        s.run("files.trash", &json!({"path": main.to_string_lossy()}))
            .is_err()
    );
    assert!(main.is_file());
    // Copy Full Path goes to the clipboard.
    s.run("files.copyPath", &json!({"path": "/x/y.tex"}))
        .unwrap();
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, Effect::CopyText(t) if t == "/x/y.tex"))
    );
}

/// Runs ticks until the Source Control action (and any queued ones) finished.
fn settle_git(s: &mut Session) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while (s.git.busy.is_some() || s.git.is_busy()) && Instant::now() < deadline {
        s.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn automatic_git_refreshes_show_no_progress() {
    if !texcraft_git::available() {
        eprintln!("skipping: no git");
        return;
    }
    let mut s = session();
    let dir = temp_dir("git-quiet");
    let main = dir.join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n").unwrap();
    s.run("file.openPath", &json!({"path": main.to_string_lossy()}))
        .unwrap();
    s.set_project_dir(dir);
    s.run("view.git", &Value::Null).unwrap();
    // The tab shows and was never read: a refresh starts by itself, without a progress line
    // (the panel would jump by a row every few seconds otherwise).
    s.tick(Instant::now());
    assert!(s.git.is_busy(), "an automatic refresh runs");
    assert!(s.git.busy.is_none(), "silently");
    settle_git(&mut s);
    // One the user asks for shows its progress.
    s.run("git.refresh", &Value::Null).unwrap();
    assert!(s.git.busy.is_some());
    settle_git(&mut s);
}

#[test]
fn source_control_stages_commits_and_discards() {
    if !texcraft_git::available() {
        eprintln!("skipping: no git");
        return;
    }
    let mut s = session();
    let dir = temp_dir("git-tab");
    let main = dir.join("main.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\nA\n\\end{document}\n",
    )
    .unwrap();
    s.run("file.openPath", &json!({"path": main.to_string_lossy()}))
        .unwrap();
    s.set_project_dir(dir.clone());
    s.run("view.git", &Value::Null).unwrap();
    s.run("git.refresh", &Value::Null).unwrap();
    settle_git(&mut s);
    assert!(s.git.not_repo, "a plain folder");
    s.run("git.init", &Value::Null).unwrap();
    settle_git(&mut s);
    assert!(s.git.repo.is_some(), "{:?}", s.git.error);
    for (k, v) in [
        ("user.name", "TexCraft Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
    ] {
        std::process::Command::new("git")
            .args(["-C"])
            .arg(&dir)
            .args(["config", k, v])
            .status()
            .unwrap();
    }
    s.run("git.refresh", &Value::Null).unwrap();
    settle_git(&mut s);
    assert_eq!(s.inspect()["git"]["changes"], json!(["U main.tex"]));
    // Nothing staged: Commit stages everything (and saves open documents first).
    assert!(s.run("git.commit", &Value::Null).is_err(), "no message");
    s.git.message = "First".into();
    s.run(
        "edit.setText",
        &json!({"text": "\\documentclass{article}\n\\begin{document}\nB\n\\end{document}\n"}),
    )
    .unwrap();
    s.run("git.commit", &Value::Null).unwrap();
    settle_git(&mut s);
    assert_eq!(s.git.error, None);
    let repo = s.git.repo.as_ref().unwrap();
    assert!(repo.status.is_clean(), "{:?}", repo.status);
    assert_eq!(repo.log[0].subject, "First");
    assert!(s.git.message.is_empty());
    assert!(
        std::fs::read_to_string(&main).unwrap().contains("\nB\n"),
        "saved before committing"
    );
    // Change, stage by absolute path, unstage, diff, discard.
    std::fs::write(&main, "changed\n").unwrap();
    s.run("git.refresh", &Value::Null).unwrap();
    settle_git(&mut s);
    s.run("git.stage", &json!({"path": main.to_string_lossy()}))
        .unwrap();
    settle_git(&mut s);
    assert_eq!(s.inspect()["git"]["staged"], json!(["M main.tex"]));
    s.run("git.unstage", &json!({"path": "main.tex"})).unwrap();
    settle_git(&mut s);
    assert_eq!(s.inspect()["git"]["changes"], json!(["M main.tex"]));
    s.run("git.diff", &json!({"path": "main.tex"})).unwrap();
    settle_git(&mut s);
    assert!(s.git.diff.as_ref().unwrap().1.contains("+changed"));
    assert!(s.effects.iter().any(|e| matches!(e, Effect::GitDiff)));
    // The open document has no unsaved changes (it was saved): discard restores the file.
    s.run("git.discard", &json!({"path": "main.tex"})).unwrap();
    settle_git(&mut s);
    assert!(std::fs::read_to_string(&main).unwrap().contains("\nB\n"));
    // An untracked file goes to the (fake) Trash.
    s.trash = |p| std::fs::remove_file(p).map_err(|e| e.to_string());
    std::fs::write(dir.join("scratch.tex"), "x").unwrap();
    s.run("git.refresh", &Value::Null).unwrap();
    settle_git(&mut s);
    s.run("git.discard", &json!({"path": "scratch.tex"}))
        .unwrap();
    settle_git(&mut s);
    assert!(!dir.join("scratch.tex").exists());
}

/// A local OpenAI-compatible server that answers one request with `answer` (streamed) and
/// returns the request it saw.
fn fake_ai_server(answer: &'static str) -> (u16, std::thread::JoinHandle<String>) {
    fake_ai_stream(serde_json::json!({ "content": answer }))
}

/// [`fake_ai_server`] streaming one chunk whose `delta` is `delta`.
fn fake_ai_stream(delta: Value) -> (u16, std::thread::JoinHandle<String>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut req = Vec::new();
        let mut buf = [0u8; 65536];
        // Read until the body (Content-Length) is complete.
        loop {
            let n = s.read(&mut buf).unwrap();
            req.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&req).to_string();
            if let Some(h) = text.find("\r\n\r\n") {
                let len = text
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if req.len() >= h + 4 + len {
                    break;
                }
            }
            if n == 0 {
                break;
            }
        }
        let chunk = serde_json::json!({"choices": [{"delta": delta}]});
        let body = format!("data: {chunk}\n\ndata: [DONE]\n\n");
        let _ = write!(
            s,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        String::from_utf8_lossy(&req).to_string()
    });
    (port, handle)
}

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
    0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// Images go with questions to models with Vision, in the request, and stay in the conversation;
/// a model without Vision is refused them, with the reason.
#[test]
fn images_attach_to_vision_models_and_go_with_the_question() {
    let mut s = session();
    let (port, server) = fake_ai_server("A red dot.");
    let ollama = texcraft_ai::PRESETS
        .iter()
        .find(|p| p.id == "ollama")
        .unwrap();
    let i = s.ai.config.add(ollama);
    s.ai.config.providers[i].base_url = format!("http://127.0.0.1:{port}/v1");
    s.ai.config.providers[i].models = vec![
        "plain-text".into(),
        texcraft_ai::Model {
            id: "eyes".into(),
            name: "Eyes 1".into(),
            capabilities: vec![texcraft_ai::Capability::Vision],
            ..Default::default()
        },
    ];
    s.ai.config.normalize();
    let pid = s.ai.config.providers[i].id.clone();
    // Without Vision: refused, and why.
    s.run(
        "ai.selectModel",
        &json!({"provider": pid, "model": "plain-text"}),
    )
    .unwrap();
    assert!(!s.ai_reads_images());
    let e = s
        .ai_attach_image("dot.png", PNG.to_vec())
        .unwrap_err()
        .to_string();
    assert!(e.contains("plain-text") && e.contains("Vision"), "{e}");
    // With Vision: PNG yes, anything else no; ten at most.
    s.run("ai.selectModel", &json!({"provider": pid, "model": "eyes"}))
        .unwrap();
    assert_eq!(s.ai_model().map(texcraft_ai::Model::label), Some("Eyes 1"));
    assert!(s.ai_attach_image("notes.txt", b"hello".to_vec()).is_err());
    s.ai_attach_image("dot.png", PNG.to_vec()).unwrap();
    for _ in 1..crate::ai::MAX_IMAGES {
        s.ai_attach_image("dot.png", PNG.to_vec()).unwrap();
    }
    assert!(s.ai_attach_image("dot.png", PNG.to_vec()).is_err());
    s.ai.draft_images.truncate(1);
    // Switching to a model without Vision keeps them but will not send them.
    s.run(
        "ai.selectModel",
        &json!({"provider": pid, "model": "plain-text"}),
    )
    .unwrap();
    assert!(s.run("ai.send", &json!({"text": "What is it?"})).is_err());
    assert_eq!(s.ai.draft_images.len(), 1);
    s.run("ai.selectModel", &json!({"provider": pid, "model": "eyes"}))
        .unwrap();
    s.run("ai.send", &json!({"text": "What is it?"})).unwrap();
    assert!(s.ai.draft_images.is_empty());
    let deadline = Instant::now() + Duration::from_secs(20);
    while s.ai.is_busy() && Instant::now() < deadline {
        s.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
    let seen = server.join().unwrap();
    assert!(seen.contains("\"image_url\""), "{seen}");
    assert!(seen.contains("data:image/png;base64,iVBORw0KGgo"), "{seen}");
    assert_eq!(s.ai.messages[0].images.len(), 1);
    assert_eq!(s.inspect()["ai"]["messages"][0]["images"], 1);
}

/// A reasoning model that spends the whole budget thinking: no suggestion, and the popover says
/// why instead of nothing happening.
#[test]
fn a_suggestion_lost_to_reasoning_is_explained() {
    let mut s = session();
    let (port, server) = fake_ai_stream(json!({ "reasoning_content": "Let me think about..." }));
    let ollama = texcraft_ai::PRESETS
        .iter()
        .find(|p| p.id == "ollama")
        .unwrap();
    let i = s.ai.config.add(ollama);
    s.ai.config.providers[i].base_url = format!("http://127.0.0.1:{port}/v1");
    s.ai.config.providers[i].models = vec!["thinker".into()];
    s.ai.config.normalize();
    let text = "\\documentclass{article}\n\\begin{document}\nNanjing is\n\\end{document}\n";
    s.run("edit.setText", &json!({ "text": text })).unwrap();
    let at = text.find(" is").unwrap() + 3;
    s.run("edit.select", &json!({ "anchor": at })).unwrap();
    s.run("ai.inline.trigger", &Value::Null).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while s.inline.last_error.is_none() && Instant::now() < deadline {
        s.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
    server.join().unwrap();
    assert!(s.inline_suggestion().is_none());
    let e = s.inline.last_error.clone().unwrap_or_default();
    assert!(e.contains("thinker") && e.contains("reasoning"), "{e}");
}

#[test]
fn inline_suggestions_follow_typing_and_are_accepted_with_tab() {
    let mut s = session();
    let (port, server) = fake_ai_server("```latex\nquick brown fox}\n```");
    let ollama = texcraft_ai::PRESETS
        .iter()
        .find(|p| p.id == "ollama")
        .unwrap();
    let i = s.ai.config.add(ollama);
    s.ai.config.providers[i].base_url = format!("http://127.0.0.1:{port}/v1");
    s.ai.config.providers[i].models = vec!["local-model".into()];
    s.ai.config.normalize();
    s.ai.config.inline.delay_ms = 100;
    assert!(s.inline_available());
    let text = "\\documentclass{article}\n\\begin{document}\n\\textbf{The }\n\\end{document}\n";
    s.run("edit.setText", &json!({ "text": text })).unwrap();
    // The caret after "The ", inside the braces: the user just typed there.
    let at = text.find("The ").unwrap() + 4;
    s.run("edit.select", &json!({ "anchor": at })).unwrap();
    s.note_edit();
    assert!(s.inline_suggestion().is_none());
    // The pause, then the answer.
    let deadline = Instant::now() + Duration::from_secs(20);
    while s.inline_suggestion().is_none() && Instant::now() < deadline {
        s.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
    let seen = server.join().unwrap();
    assert!(
        seen.contains("The <cursor/>}"),
        "context around the caret: {seen}"
    );
    assert!(seen.contains("\"max_tokens\":144"), "small budget: {seen}");
    // Fences gone, the closing brace the text already has dropped.
    assert_eq!(s.inline_suggestion().unwrap().text, "quick brown fox");
    // Typing what it says keeps the rest.
    s.run("edit.insertText", &json!({ "text": "qu" })).unwrap();
    s.note_edit();
    assert_eq!(s.inline_suggestion().unwrap().text, "ick brown fox");
    // ⌘→: the next word; Tab: the rest.
    s.run("ai.inline.acceptWord", &Value::Null).unwrap();
    assert_eq!(s.inline_suggestion().unwrap().text, "brown fox");
    assert!(s.is_enabled("ai.inline.acceptSentence"));
    s.run("ai.inline.accept", &Value::Null).unwrap();
    assert!(s.inline_suggestion().is_none());
    // Tab took all of it: the next suggestion is asked for straight away (Tab, Tab, …).
    assert!(s.inline.due.is_some_and(|d| d <= Instant::now()));
    // Moving the caret asks for nothing.
    s.inline_after_caret_move();
    assert!(s.inline.due.is_none() && s.inline_suggestion().is_none());
    assert!(
        s.doc()
            .unwrap()
            .text
            .contains("\\textbf{The quick brown fox}")
    );
    // Moving the caret hides a suggestion; Esc drops it; off means no requests.
    s.run("ai.inline.toggle", &Value::Null).unwrap();
    assert!(!s.ai.config.inline.enabled);
    s.note_edit();
    assert!(!s.inspect()["ai"]["inline"]["waiting"].as_bool().unwrap());
}

#[test]
fn ai_assistant_sends_the_document_and_inserts_the_answer() {
    let mut s = session();
    assert!(
        s.run("ai.send", &json!({"text": "Hi"})).is_err(),
        "no provider yet"
    );
    let (port, server) =
        fake_ai_server("Use this:\n```latex\n\\begin{table}\n\\end{table}\n```\nDone.");
    // Ollama: no key needed.
    let ollama = texcraft_ai::PRESETS
        .iter()
        .find(|p| p.id == "ollama")
        .unwrap();
    let i = s.ai.config.add(ollama);
    s.ai.config.providers[i].base_url = format!("http://127.0.0.1:{port}/v1");
    s.ai.config.providers[i].models = vec!["local-model".into()];
    s.ai.config.normalize();
    s.run(
        "edit.setText",
        &json!({"text": "\\documentclass{article}\n\\begin{document}\nHello\n\\end{document}\n"}),
    )
    .unwrap();
    s.ai.draft = "Add a table".into();
    s.run("ai.send", &Value::Null).unwrap();
    assert!(s.ai.draft.is_empty());
    let deadline = Instant::now() + Duration::from_secs(20);
    while s.ai.is_busy() && Instant::now() < deadline {
        s.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
    let seen = server.join().unwrap();
    assert!(
        seen.contains("<document>"),
        "the document goes along: {seen}"
    );
    assert!(seen.contains("   3  Hello"), "with line numbers: {seen}");
    assert!(seen.contains("\"model\":\"local-model\""), "{seen}");
    let answer = s.ai.messages.last().unwrap();
    assert_eq!(answer.error, None);
    assert!(answer.text.starts_with("Use this:"));
    assert_eq!(
        s.ai_snippet(1).as_deref(),
        Some("\\begin{table}\n\\end{table}")
    );
    s.run("edit.select", &json!({"anchor": 47})).unwrap(); // the start of line 4
    s.run("ai.insert", &json!({"index": 1})).unwrap();
    assert!(
        s.doc()
            .unwrap()
            .text
            .contains("Hello\n\\begin{table}\n\\end{table}\\end{document}"),
        "{}",
        s.doc().unwrap().text
    );
    s.run("ai.copy", &json!({"index": 1})).unwrap();
    assert!(
        s.effects
            .iter()
            .any(|e| matches!(e, Effect::CopyText(t) if t.starts_with("Use this")))
    );
    // A failed request: the error shows and the question is not resent later.
    s.ai.config.providers[i].base_url = "http://127.0.0.1:9/v1".into();
    s.run("ai.send", &json!({"text": "Again"})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while s.ai.is_busy() && Instant::now() < deadline {
        s.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(s.ai.messages.last().unwrap().error.is_some());
    s.run("ai.clear", &Value::Null).unwrap();
    assert!(s.ai.messages.is_empty());
}
