//! Contract tests through the real editor, TeX worker and PDF view. Editor-only checks cannot
//! establish that unsaved keystrokes actually reach the page displayed beside the source.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Context, Event, Key, Modifiers, Pos2, RawInput, Rect, vec2};
use serde_json::{Value, json};
use texcraft_engine::Session;
use texcraft_project::Engine;
use texcraft_ui::Workbench;

fn source(body: &str) -> String {
    format!("\\documentclass{{article}}\n\\begin{{document}}\n{body}\n\\end{{document}}\n")
}

struct Preview {
    ctx: Context,
    bench: Workbench,
    session: Session,
    dir: PathBuf,
}

impl Preview {
    fn live_dir(&self) -> PathBuf {
        texcraft_typeset::live::recovery::cache_dir(&self.dir, Path::new("main.tex"))
    }

    fn new(name: &str) -> Option<Self> {
        let ctx = Context::default();
        let bench = Workbench::new(&ctx);
        let mut session = Session::empty(Arc::new({
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        }));
        if !session.toolchain.supports(Engine::PdfLatex) {
            assert!(
                std::env::var_os("TEXCRAFT_REQUIRE_TEX").is_none(),
                "TeX is required for this check"
            );
            eprintln!("skipping: no pdfLaTeX");
            return None;
        }
        session.update.enabled = false;
        session.view.spellcheck = false;
        session.view.auto_complete = false;
        let dir =
            std::env::temp_dir().join(format!("texcraft-preview-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.tex"), source("OriginalPreview")).unwrap();
        session.open_path(&dir.join("main.tex")).unwrap();
        Some(Self {
            ctx,
            bench,
            session,
            dir,
        })
    }

    fn frame(&mut self, events: Vec<Event>) {
        let mut out = self.ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 600.0))),
                events,
                ..Default::default()
            },
            |ui| self.bench.ui(ui, &mut self.session),
        );
        out.textures_delta.clear();
    }

    fn command(&mut self, id: &str, params: Value) {
        self.session.run(id, &params).unwrap();
        self.frame(Vec::new());
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
    }

    fn paste_source(&mut self, text: &str) {
        self.command("edit.gotoLine", json!({"line": 3}));
        self.key(Key::A, Modifiers::COMMAND);
        self.frame(vec![Event::Paste(text.into())]);
        assert_eq!(
            self.session.doc().unwrap().text,
            text,
            "the real editor received the paste"
        );
    }

    fn until(&mut self, ready: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.frame(Vec::new());
            if ready(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "preview timed out: {}",
                self.session.inspect()["typeset"]
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn idle(&mut self) {
        self.until(|p| !p.session.typeset.is_busy());
    }

    /// Recreate the editor/engine/render worker while retaining the generated on-disk project
    /// and caches, as a fresh process opening the same saved project would do.
    fn reopen(&mut self) {
        self.idle();
        let settings = self.session.settings.clone();
        let view = self.session.view.clone();
        let ctx = Context::default();
        let bench = Workbench::new(&ctx);
        let mut session = Session::empty(Arc::new({
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        }));
        session.settings = settings;
        session.view = view;
        session.update.enabled = false;
        session.open_path(&self.dir.join("main.tex")).unwrap();
        self.ctx = ctx;
        self.bench = bench;
        self.session = session;
    }

    fn shows(&mut self, text: &str) {
        self.idle();
        assert!(
            !self.session.live_behind(),
            "the current source was not typeset"
        );
        self.command(
            "pdf.selectText",
            json!({"page": 1, "x": 0, "y": 0, "toX": 1000, "toY": 1000}),
        );
        self.until(|p| {
            p.session.pdf.selected_text.contains(text)
                && p.bench.pdf.rendered_generation(0) == Some(p.session.pdf.generation)
        });
        assert_eq!(self.session.issue_counts().0, 0);
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.session.stop_typeset();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn real_editor_edits_reach_the_displayed_pdf_without_saving() {
    for lang in [texcraft_i18n::Lang::En, texcraft_i18n::Lang::ZhHans] {
        let _language = texcraft_i18n::scoped(lang);
        let Some(mut p) = Preview::new(lang.code()) else {
            return;
        };
        p.shows("OriginalPreview");
        let generation = p.session.pdf.generation;
        p.paste_source(&source("PastedPreview"));
        p.shows("PastedPreview");
        assert!(p.session.pdf.generation > generation);
        p.command("edit.gotoLine", json!({"line": 4}));
        p.key(Key::Enter, Modifiers::NONE);
        // Smart Return and ordinary typing use different edit paths.
        p.frame(vec![Event::Text("TypedPreview".into())]);
        p.shows("TypedPreview");
        assert!(p.session.doc().unwrap().is_dirty());
        assert!(
            std::fs::read_to_string(p.dir.join("main.tex"))
                .unwrap()
                .contains("OriginalPreview")
        );
        assert!(p.session.settings.live_typeset && !p.session.live_paused);
    }
}

#[test]
fn failed_preview_stays_behind_and_recovers_after_correction() {
    let Some(mut p) = Preview::new("recovery") else {
        return;
    };
    p.shows("OriginalPreview");
    let generation = p.session.pdf.generation;
    p.paste_source(&source("\\undefinedPreviewCommand"));
    p.idle();
    assert!(p.session.issue_counts().0 > 0);
    assert_eq!(
        p.session.pdf.generation, generation,
        "keep the last good preview"
    );
    assert!(
        p.session.live_behind(),
        "an old PDF must not be reported as current"
    );
    let runs = p.session.typeset.runs;
    for _ in 0..10 {
        p.frame(Vec::new());
    }
    assert_eq!(
        p.session.typeset.runs, runs,
        "do not retry the same stable error forever"
    );
    p.paste_source(&source("RecoveredPreview"));
    p.shows("RecoveredPreview");
}

#[test]
fn typing_cjk_switches_engines_and_updates_the_displayed_pdf() {
    let Some(mut p) = Preview::new("cjk") else {
        return;
    };
    if !p.session.toolchain.supports(Engine::XeLatex) || p.session.toolchain.cjk_fonts.is_none() {
        assert!(
            std::env::var_os("TEXCRAFT_REQUIRE_TEX").is_none(),
            "XeLaTeX/CJK fonts are required for this check"
        );
        eprintln!("skipping: no XeLaTeX/CJK fonts");
        return;
    }
    p.shows("OriginalPreview");
    p.paste_source(&source("中文 CjkPreview"));
    p.shows("CjkPreview");
    assert_eq!(p.session.effective_engine(), Engine::XeLatex);
    p.paste_source(&source("中文 UpdatedCjkPreview"));
    p.shows("UpdatedCjkPreview");
}

#[test]
fn a_broken_preamble_cache_cannot_freeze_later_edits() {
    for engine in [Engine::PdfLatex, Engine::XeLatex, Engine::LuaLatex] {
        let Some(mut p) = Preview::new(&format!("broken-cache-{}", engine.id())) else {
            return;
        };
        if !p.session.toolchain.supports(engine) {
            continue;
        }
        p.session.settings.auto_sense = false;
        p.session.settings.engine = engine;
        p.idle();
        let initial = p.session.typeset.last.as_ref().unwrap();
        assert!(initial.success, "{engine:?}: {}", initial.console);
        p.shows("OriginalPreview");
        let formats: Vec<_> = std::fs::read_dir(p.live_dir())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "fmt"))
            .collect();
        if formats.is_empty() {
            assert!(
                engine != Engine::PdfLatex || std::env::var_os("TEXCRAFT_REQUIRE_TEX").is_none(),
                "a cached preamble is required for this check"
            );
            eprintln!("skipping {engine:?}: no precompiled preamble available");
            continue;
        }
        for path in formats {
            std::fs::write(path, b"invalid cached format").unwrap();
        }
        p.paste_source(&source("RecoveredCachePreview"));
        p.idle();
        let last = p.session.typeset.last.as_ref().unwrap();
        assert_eq!(last.exit_code, Some(0), "{}", last.console);
        p.shows("RecoveredCachePreview");
        p.paste_source(&source("NextCachedPreview"));
        p.shows("NextCachedPreview");
        assert!(p.session.typeset.last.as_ref().unwrap().preamble_cached);
    }
}

/// Exercise the races the small happy-path examples miss: do not wait for a pass between
/// edits, interleave full typesets/pauses, and check each batch's final *displayed* text.
#[test]
fn repeated_edits_and_typeset_transitions_always_reach_the_latest_displayed_revision() {
    let Some(mut p) = Preview::new("stress") else {
        return;
    };
    p.session.settings.save_before_typeset = false;
    p.shows("OriginalPreview");
    for batch in 0..6 {
        for step in 0..50 {
            let n = batch * 50 + step;
            if n % 17 == 0 {
                p.paste_source(&source("\\textbf{"));
            }
            p.paste_source(&source(&format!("StressRevision{n}End")));
            if n % 23 == 0 && !p.session.typeset.is_full_running() {
                p.command("typeset.run", Value::Null);
            }
            if n % 19 == 0 {
                p.command("typeset.pauseLive", Value::Null);
                p.frame(Vec::new());
                p.command("typeset.pauseLive", Value::Null);
            }
            if n % 31 == 0 {
                p.command("typeset.mode.manual", Value::Null);
                p.command("typeset.mode.live", Value::Null);
            }
            if n % 29 == 0 {
                p.command("view.editorOnly", Value::Null);
                p.frame(Vec::new());
                p.command("view.split", Value::Null);
            }
        }
        p.shows(&format!("StressRevision{}End", batch * 50 + 49));
    }
    assert!(p.session.doc().unwrap().is_dirty());
    assert!(
        std::fs::read_to_string(p.dir.join("main.tex"))
            .unwrap()
            .contains("OriginalPreview")
    );
}

#[test]
fn unsaved_included_file_edits_survive_tab_and_preview_visibility_changes() {
    let Some(mut p) = Preview::new("subfile") else {
        return;
    };
    let chapter = p.dir.join("chapter.tex");
    std::fs::write(&chapter, "% !TEX root = main.tex\nOriginalChapter\n").unwrap();
    p.paste_source(&source("\\input{chapter}"));
    p.shows("OriginalChapter");
    p.command("file.openPath", json!({"path": chapter}));
    let chapter_index = p.session.active;
    p.command("view.editorOnly", Value::Null);
    p.paste_source("% !TEX root = main.tex\nUpdatedChapterPreview\n");
    p.idle();
    p.command("window.selectTab", json!({"index": 0}));
    p.command("view.split", Value::Null);
    p.shows("UpdatedChapterPreview");
    p.command("window.selectTab", json!({"index": chapter_index}));
    p.paste_source("% !TEX root = main.tex\nLatestChapterPreview\n");
    p.shows("LatestChapterPreview");
    assert!(
        std::fs::read_to_string(chapter)
            .unwrap()
            .contains("OriginalChapter")
    );
}

#[test]
fn independent_editor_sessions_keep_their_own_latest_displayed_preview() {
    let Some(mut a) = Preview::new("session-a") else {
        return;
    };
    let Some(mut b) = Preview::new("session-b") else {
        return;
    };
    a.shows("OriginalPreview");
    b.shows("OriginalPreview");
    for n in 0..30 {
        a.paste_source(&source(&format!("WindowARevision{n}End")));
        b.paste_source(&source(&format!("WindowBRevision{n}End")));
        a.frame(Vec::new());
        b.frame(Vec::new());
    }
    a.shows("WindowARevision29End");
    b.shows("WindowBRevision29End");
    assert!(!a.session.pdf.selected_text.contains("WindowB"));
    assert!(!b.session.pdf.selected_text.contains("WindowA"));
}

#[test]
fn truncated_auxiliary_state_recovers_automatically_even_after_reopening() {
    let Some(mut p) = Preview::new("reopen-truncated-aux") else {
        return;
    };
    p.shows("OriginalPreview");
    let document = |marker: &str| {
        format!(
            "\\documentclass{{article}}\n\\usepackage{{hyperref}}\n\\begin{{document}}\n\\tableofcontents\n\\section{{{marker}}}\nBody.\n\\end{{document}}\n"
        )
    };
    p.paste_source(&document("BeforeRestart"));
    p.shows("BeforeRestart");
    // Bad generated files can fail before TeX overwrites them. Keep the valid saved source
    // and the poisoned caches across six fresh editor/engine/worker lifetimes.
    for (n, ext) in ["aux", "toc", "out", "aux", "toc", "out"]
        .into_iter()
        .enumerate()
    {
        std::fs::write(
            p.live_dir().join(format!("main.{ext}")),
            format!("\\input{{main.{ext}}}\n"),
        )
        .unwrap();
        let marker = format!("PreviewAfterRestart{n}End");
        std::fs::write(p.dir.join("main.tex"), document(&marker)).unwrap();
        p.reopen();
        p.shows(&marker);
        assert!(
            p.session.typeset.recoveries > 0,
            "{ext}: no automatic recovery recorded"
        );
    }
}

#[test]
fn a_failed_display_reload_automatically_rebuilds_without_user_input_or_source_changes() {
    let Some(mut p) = Preview::new("display-self-repair") else {
        return;
    };
    p.shows("OriginalPreview");
    let source = p.session.doc().unwrap().text.clone();
    // Simulate a newly published PDF being only partly readable. The last good texture stays
    // on screen; local reloads must escalate to one clean rebuild without another edit.
    std::fs::write(
        p.session.pdf.path.as_ref().unwrap(),
        b"partial published PDF",
    )
    .unwrap();
    p.session.pdf.generation += 1;
    let failed_generation = p.session.pdf.generation;
    p.shows("OriginalPreview");
    assert!(p.session.pdf.generation > failed_generation);
    assert!(p.session.typeset.recoveries > 0);
    assert_eq!(p.session.doc().unwrap().text, source);
    assert!(!p.session.doc().unwrap().is_dirty());
    assert_eq!(
        std::fs::read_to_string(p.dir.join("main.tex")).unwrap(),
        source
    );
}
