//! Exercise the actual welcome and save-prompt controls, including native panel cancellation.

#![allow(clippy::unwrap_used, clippy::panic)] // Test rig and assertions only.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{
    Context, Event, FullOutput, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, vec2,
};
use serde_json::{Value, json};
use texcraft_engine::Session;
use texcraft_ui::{Request, Workbench, dialogs::Dialog};

struct Rig {
    ctx: Context,
    bench: Workbench,
    session: Session,
}

impl Rig {
    fn new() -> Self {
        let ctx = Context::default();
        let bench = Workbench::new(&ctx);
        let mut session = Session::empty(Arc::new(|| {}));
        session.settings.live_typeset = false;
        session.update.enabled = false;
        Self {
            ctx,
            bench,
            session,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> FullOutput {
        let mut output = self.ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1388.0, 824.0))),
                events,
                ..Default::default()
            },
            |ui| self.bench.ui(ui, &mut self.session),
        );
        output.textures_delta.clear();
        output
    }

    fn click(&mut self, label: &str) {
        fn position(shape: &Shape, label: &str) -> Option<Pos2> {
            match shape {
                Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() * 0.5),
                Shape::Vec(shapes) => shapes.iter().find_map(|s| position(s, label)),
                _ => None,
            }
        }
        self.frame(Vec::new());
        let frame = self.frame(Vec::new());
        let pos = frame
            .shapes
            .iter()
            .find_map(|s| position(&s.shape, label))
            .unwrap_or_else(|| panic!("missing control: {label}"));
        self.frame(vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]);
        self.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
    }

    fn saved_file(&mut self, name: &str) -> PathBuf {
        let file = std::env::temp_dir().join(format!(
            "texcraft-lifecycle-{name}-{}.tex",
            std::process::id()
        ));
        std::fs::write(&file, "saved text").unwrap();
        self.session.open_path(&file).unwrap();
        file
    }
}

#[test]
fn cancelling_an_open_panel_keeps_welcome_and_never_exposes_a_template() {
    let mut rig = Rig::new();
    rig.bench.welcome.inline = true;
    rig.bench.welcome.show_window(true);
    rig.click("Open a local file or project…");
    assert!(rig.bench.requests.contains(&Request::Open));
    // Cancel: no platform document_opened callback.
    rig.frame(Vec::new());
    assert!(rig.bench.welcome.open && rig.bench.welcome.launch);
    assert!(rig.session.docs.is_empty() && rig.session.pdf.path.is_none());
    rig.click("Open a project folder…");
    assert!(rig.bench.requests.contains(&Request::OpenFolder));
    assert!(rig.bench.welcome.open);
    rig.bench.document_opened(&rig.ctx);
    assert!(!rig.bench.welcome.open && !rig.bench.welcome.launch);
}

#[test]
fn closing_last_tab_returns_to_welcome_and_closing_project_welcome_preserves_the_project() {
    let mut rig = Rig::new();
    let file = rig.saved_file("last-tab");
    rig.bench.welcome.inline = true;
    rig.bench.welcome.show_window(false);
    assert!(!rig.bench.welcome.launch);
    // Close the ordinary welcome window: keep the document underneath it.
    rig.bench.welcome.open = false;
    rig.frame(Vec::new());
    assert_eq!(rig.session.docs.len(), 1);
    rig.session.run("file.closeTab", &Value::Null).unwrap();
    rig.frame(Vec::new());
    assert!(rig.session.docs.is_empty());
    assert!(rig.bench.welcome.open && rig.bench.welcome.launch);
    assert!(!rig.bench.requests.contains(&Request::Quit));
    std::fs::remove_file(file).unwrap();
}

#[test]
fn save_and_discard_during_quit_keep_saved_tabs_for_relaunch() {
    for (name, button, expected) in [
        ("save", "Save", "new text"),
        ("discard", "Don't Save", "saved text"),
    ] {
        let mut rig = Rig::new();
        let file = rig.saved_file(name);
        rig.session
            .run("edit.setText", &json!({"text": "new text"}))
            .unwrap();
        rig.bench.dialog = Dialog::ConfirmClose { doc: 0, quit: true };
        rig.click(button);
        assert_eq!(rig.session.docs.len(), 1);
        assert_eq!(rig.session.doc().unwrap().text, expected);
        assert!(!rig.session.doc().unwrap().is_dirty());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), expected);
        assert_eq!(rig.session.workspace().documents.len(), 1);
        assert!(rig.bench.requests.contains(&Request::Quit));
        std::fs::remove_file(file).unwrap();
    }
}

#[test]
fn untitled_save_as_waits_for_success_and_cancel_does_not_quit() {
    let mut rig = Rig::new();
    rig.session.run("file.new", &Value::Null).unwrap();
    rig.session
        .run("edit.setText", &json!({"text": "my unsaved work"}))
        .unwrap();
    rig.bench.dialog = Dialog::ConfirmClose { doc: 0, quit: true };
    rig.click("Save");
    assert!(rig.bench.requests.contains(&Request::SaveAs(0)));
    // Cancelling the native Save As panel leaves the confirmation and text in place.
    rig.frame(Vec::new());
    assert_eq!(
        rig.bench.dialog,
        Dialog::ConfirmClose { doc: 0, quit: true }
    );
    assert_eq!(rig.session.doc().unwrap().text, "my unsaved work");
    assert!(!rig.bench.requests.contains(&Request::Quit));
    let file = std::env::temp_dir().join(format!(
        "texcraft-lifecycle-save-as-{}.tex",
        std::process::id()
    ));
    rig.session.save_as(0, file.clone()).unwrap();
    rig.frame(Vec::new());
    assert_eq!(rig.bench.dialog, Dialog::None);
    assert_eq!(rig.session.workspace().documents.len(), 1);
    assert!(rig.bench.requests.contains(&Request::Quit));
    std::fs::remove_file(file).unwrap();
}

#[test]
fn discarding_a_deleted_file_can_still_quit_without_recreating_it() {
    let mut rig = Rig::new();
    let file = rig.saved_file("deleted");
    rig.session
        .run("edit.setText", &json!({"text": "unsaved text"}))
        .unwrap();
    std::fs::remove_file(&file).unwrap();
    rig.bench.welcome.inline = true;
    rig.bench.dialog = Dialog::ConfirmClose { doc: 0, quit: true };
    rig.click("Don't Save");
    assert!(!file.exists());
    assert!(rig.session.docs.is_empty());
    assert!(rig.bench.requests.contains(&Request::Quit));
}
