//! Renders the workbench's surfaces in Simplified Chinese (and English): every sidebar tab,
//! every Settings pane and the Typeset Configuration sheet, the welcome window, the update
//! window and the dialogs. Nothing may panic, the Chinese frames must show the translations,
//! and the English frames must show no Chinese.

use egui::{Context, FullOutput, Pos2, RawInput, Rect, Shape, vec2};
use texcraft_engine::{LanguageChoice, Session, SidebarTab, UpdateStatus};
use texcraft_i18n::{Lang, scoped, tr_in};
use texcraft_ui::Workbench;
use texcraft_ui::dialogs::Dialog;
use texcraft_ui::prefs::Pane;

/// Every piece of text a frame drew.
fn texts(out: &FullOutput) -> String {
    fn walk(shape: &Shape, into: &mut String) {
        match shape {
            Shape::Text(t) => {
                into.push_str(t.galley.text());
                into.push('\n');
            }
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, into)),
            _ => {}
        }
    }
    let mut s = String::new();
    for c in &out.shapes {
        walk(&c.shape, &mut s);
    }
    s
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c) || ('\u{3000}'..='\u{303f}').contains(&c)
}

struct Rig {
    ctx: Context,
    bench: Workbench,
    session: Session,
}

impl Rig {
    fn new(lang: LanguageChoice) -> Self {
        let ctx = Context::default();
        let bench = Workbench::new(&ctx);
        let mut session = Session::new(std::sync::Arc::new(|| {}));
        session.view.language = lang;
        // Never the network.
        session.update.url = "/nonexistent/texcraft-latest.json".into();
        Self {
            ctx,
            bench,
            session,
        }
    }

    /// Two frames (egui lays some things out on the second); the text of the last.
    fn frames(&mut self) -> String {
        let mut last = String::new();
        for _ in 0..2 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1388.0, 824.0))),
                ..Default::default()
            };
            let mut out = self
                .ctx
                .run_ui(input, |ui| self.bench.ui(ui, &mut self.session));
            out.textures_delta.clear();
            last = texts(&out);
        }
        last
    }

    /// Draws every surface; returns all the text drawn, one entry per surface.
    fn every_surface(&mut self) -> Vec<(String, String)> {
        let mut seen = Vec::new();
        seen.push(("main window".to_string(), self.frames()));
        for tab in SidebarTab::ALL {
            self.session.view.sidebar = true;
            self.session.view.sidebar_tab = tab;
            seen.push((format!("sidebar {tab:?}"), self.frames()));
        }
        self.session.view.sidebar_tab = SidebarTab::Contents;
        // Settings, drawn inside the main window (as `ui.prefs {inline: true}` does).
        self.bench.prefs.open = true;
        self.bench.prefs.inline = true;
        for pane in Pane::ALL {
            self.bench.prefs.pane = pane;
            seen.push((format!("Settings {pane:?}"), self.frames()));
        }
        for tab in 0..4 {
            self.bench.prefs.sheet_tab = tab;
            self.bench.dialog = Dialog::TypesetConfiguration;
            seen.push((format!("Typeset Configuration tab {tab}"), self.frames()));
        }
        self.bench.dialog = Dialog::None;
        self.bench.prefs.open = false;
        // The welcome window, inline, both tabs.
        self.bench.welcome.inline = true;
        self.bench.welcome.show_window(false);
        for tab in 0..3 {
            self.bench.welcome.left_tab = tab;
            seen.push((format!("welcome tab {tab}"), self.frames()));
        }
        self.bench.welcome.open = false;
        // The update window.
        for status in [
            UpdateStatus::Checking,
            UpdateStatus::UpToDate,
            UpdateStatus::Failed("network".into()),
        ] {
            self.session.update.status = status;
            self.bench.update_window = true;
            seen.push(("update window".to_string(), self.frames()));
        }
        self.bench.update_window = false;
        for dialog in [
            Dialog::About,
            Dialog::Shortcuts,
            Dialog::GotoLine(String::new()),
            Dialog::ConfirmClose(0),
            Dialog::Alert {
                title: "Alert".into(),
                message: "Message".into(),
            },
        ] {
            let name = format!("{dialog:?}");
            self.bench.dialog = dialog;
            seen.push((name, self.frames()));
        }
        seen
    }
}

#[test]
fn every_surface_renders_in_chinese() {
    let _zh = scoped(Lang::ZhHans);
    let mut rig = Rig::new(LanguageChoice::ChineseSimplified);
    assert_eq!(rig.session.lang(), Lang::ZhHans);
    let seen = rig.every_surface();
    // TEXCRAFT_SHOW_TEXT=1 … -- --nocapture: the lines drawn without any Chinese (proper names,
    // numbers, shortcuts, or a string still missing its tr()).
    if std::env::var_os("TEXCRAFT_SHOW_TEXT").is_some() {
        for (surface, text) in &seen {
            let latin: Vec<&str> = text
                .lines()
                .filter(|l| !l.chars().any(is_cjk) && l.chars().any(|c| c.is_ascii_lowercase()))
                .collect();
            println!("== {surface}: {latin:?}");
        }
    }
    for (surface, text) in &seen {
        assert!(
            text.chars().any(is_cjk),
            "{surface}: no Chinese drawn:\n{text}"
        );
    }
    let all: String = seen.iter().map(|(_, t)| t.as_str()).collect();
    // A few landmarks of each surface, in their Chinese form.
    for en in [
        "General",
        "Appearance",
        "Language",
        "Typeset Configuration…",
        "Recent Documents",
        "Keyboard Shortcuts",
    ] {
        let zh = tr_in(Lang::ZhHans, en);
        assert_ne!(zh, en, "{en} has no translation");
        assert!(all.contains(zh), "“{zh}” ({en}) was not drawn");
    }
    // Settings → General shows the chosen language by its own name.
    assert!(all.contains("简体中文"));
}

#[test]
fn english_shows_no_chinese() {
    let _en = scoped(Lang::En);
    let mut rig = Rig::new(LanguageChoice::English);
    for (surface, text) in rig.every_surface() {
        // The Language popup names 简体中文 in its own language: the only Chinese allowed.
        let text = text.replace("简体中文", "");
        assert!(
            !text.chars().any(is_cjk),
            "{surface}: Chinese in the English interface:\n{text}"
        );
    }
}
