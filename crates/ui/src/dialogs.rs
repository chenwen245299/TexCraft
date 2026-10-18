//! Sheets and windows: Go to Line, About, Keyboard Shortcuts,
//! alerts and the unsaved-changes prompt. Settings and Typeset Configuration are in
//! [`crate::prefs`].

use egui::{Id, Key, RichText, vec2};
use texcraft_engine::{Session, registry};
use texcraft_i18n::{tr, trf};

use crate::menu::{self, ButtonKind};
use crate::theme::{Tokens, semibold};

/// Which dialog is open (one at a time).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Dialog {
    /// None.
    #[default]
    None,
    /// Typeset configuration sheet.
    TypesetConfiguration,
    /// Go to line.
    GotoLine(String),
    /// About TexCraft.
    About,
    /// Keyboard shortcuts.
    Shortcuts,
    /// A message.
    Alert {
        /// Title.
        title: String,
        /// Text.
        message: String,
    },
    /// Close a document with unsaved changes.
    ConfirmClose(usize),
}

/// What the app must do after the welcome window (things that need the platform).
#[derive(Debug, Clone, PartialEq)]
pub enum DialogAction {
    /// Show the open panel.
    Open,
    /// Show the open-folder panel.
    OpenFolder,
    /// Quit (the start-up welcome window was closed).
    Quit,
}

/// A dialog's title (semibold 14 pt) and optional message (12 pt, secondary colour).
fn heading(ui: &mut egui::Ui, t: &Tokens, title: &str, message: Option<&str>) {
    ui.label(
        RichText::new(title)
            .font(semibold(14.0))
            .color(t.form_label),
    );
    if let Some(m) = message {
        ui.add_space(4.0);
        ui.label(RichText::new(m).size(12.0).color(t.form_desc));
    }
}

/// Draws the open dialog, if any. All of them are sheets in the app's style
/// ([`menu::sheet_frame`]) with capsule buttons, the default one at the right.
pub fn show(ctx: &egui::Context, t: &Tokens, session: &mut Session, dialog: &mut Dialog) {
    let mut close = false;
    let modal = |id: &str| {
        egui::Modal::new(Id::new(id))
            .frame(menu::sheet_frame(t, 20))
            .backdrop_color(menu::backdrop())
    };
    let enter = ctx.input(|i| i.key_pressed(Key::Enter));
    match dialog {
        // The Typeset Configuration sheet is drawn by `prefs::typeset_sheet`.
        Dialog::None | Dialog::TypesetConfiguration => return,
        Dialog::GotoLine(text) => {
            let mut go = None;
            let m = modal("goto-line").show(ctx, |ui| {
                ui.set_width(280.0);
                heading(ui, t, tr("Go to Line"), None);
                ui.add_space(10.0);
                let r = ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(tr("Line number"))
                        .desired_width(f32::INFINITY)
                        .margin(vec2(8.0, 5.0)),
                );
                r.request_focus();
                ui.add_space(14.0);
                match menu::dialog_buttons(
                    ui,
                    t,
                    &[
                        (tr("Cancel"), ButtonKind::Normal),
                        (tr("Go"), ButtonKind::Default),
                    ],
                ) {
                    Some(0) => close = true,
                    Some(_) => {
                        go = text.trim().parse::<u32>().ok();
                        close = true;
                    }
                    None if enter => {
                        go = text.trim().parse::<u32>().ok();
                        close = true;
                    }
                    None => {}
                }
            });
            if m.should_close() {
                close = true;
            }
            if let Some(line) = go {
                let _ = session.run("edit.gotoLine", &serde_json::json!({ "line": line }));
            }
        }
        Dialog::About => {
            let m = modal("about").show(ctx, |ui| {
                ui.set_width(340.0);
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), egui::Sense::hover());
                    crate::welcome::icon(ui, t, r.center(), 72.0);
                    ui.add_space(8.0);
                    ui.label(RichText::new("TexCraft").font(semibold(20.0)).color(t.form_label));
                    ui.label(
                        RichText::new(trf!("Version {version}", version = env!("CARGO_PKG_VERSION")))
                            .size(12.0)
                            .color(t.form_desc),
                    );
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(tr("A native LaTeX editor written in pure Rust."))
                            .size(12.0)
                            .color(t.form_label),
                    );
                    ui.add_space(4.0);
                    for line in [
                        tr("Clean-room: modelled on public descriptions of Texifier; no code or assets copied.").to_string(),
                        session.toolchain.describe(),
                        "MIT OR Apache-2.0".to_string(),
                    ] {
                        ui.label(RichText::new(line).size(11.0).color(t.form_note));
                    }
                });
                ui.add_space(14.0);
                if menu::dialog_buttons(ui, t, &[(tr("OK"), ButtonKind::Default)]).is_some()
                    || enter
                {
                    close = true;
                }
            });
            if m.should_close() {
                close = true;
            }
        }
        Dialog::Shortcuts => {
            let m = modal("shortcuts").show(ctx, |ui| {
                ui.set_width(560.0);
                heading(ui, t, tr("Keyboard Shortcuts"), None);
                ui.add_space(10.0);
                egui::ScrollArea::vertical().max_height(460.0).show(ui, |ui| {
                    let mut last_menu = None;
                    egui::Grid::new("shortcuts-grid")
                        .num_columns(2)
                        .spacing(vec2(24.0, 5.0))
                        .show(ui, |ui| {
                            for c in registry() {
                                let Some(sc) = c.shortcut else { continue };
                                if last_menu != Some(c.menu) {
                                    if last_menu.is_some() {
                                        ui.end_row();
                                    }
                                    ui.label(
                                        RichText::new(c.menu.title())
                                            .font(semibold(12.0))
                                            .color(t.form_label),
                                    );
                                    ui.label("");
                                    ui.end_row();
                                    last_menu = Some(c.menu);
                                }
                                ui.label(RichText::new(c.label()).size(12.0).color(t.form_label));
                                ui.label(RichText::new(sc.display()).size(12.0).color(t.form_desc));
                                ui.end_row();
                            }
                        });
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(tr("Editor: Esc opens autocomplete · Tab accepts · ⌘-click shows the position in the PDF · ⌘-click in the PDF jumps to the source"))
                            .size(11.0)
                            .color(t.form_note),
                    );
                });
                ui.add_space(14.0);
                if menu::dialog_buttons(ui, t, &[(tr("Close"), ButtonKind::Default)]).is_some()
                    || enter
                {
                    close = true;
                }
            });
            if m.should_close() {
                close = true;
            }
        }
        Dialog::Alert { title, message } => {
            let m = modal("alert").show(ctx, |ui| {
                ui.set_width(380.0);
                heading(ui, t, title.as_str(), Some(message.as_str()));
                ui.add_space(16.0);
                if menu::dialog_buttons(ui, t, &[(tr("OK"), ButtonKind::Default)]).is_some()
                    || enter
                {
                    close = true;
                }
            });
            if m.should_close() {
                close = true;
            }
        }
        Dialog::ConfirmClose(idx) => {
            let idx = *idx;
            let name = session
                .docs
                .get(idx)
                .map(texcraft_engine::Document::display_name)
                .unwrap_or_default();
            let m = modal("confirm-close").show(ctx, |ui| {
                ui.set_width(400.0);
                heading(
                    ui,
                    t,
                    &trf!(
                        "Do you want to save the changes made to “{name}”?",
                        name = name
                    ),
                    Some(tr("Your changes will be lost if you don't save them.")),
                );
                ui.add_space(16.0);
                // "Don't Save" sits apart at the left, as on macOS.
                let row = ui.max_rect().left();
                let top = ui.cursor().top();
                let mut left = ui.new_child(egui::UiBuilder::new().max_rect(
                    egui::Rect::from_min_size(egui::pos2(row, top), vec2(110.0, 28.0)),
                ));
                if menu::dialog_button(&mut left, t, tr("Don't Save"), ButtonKind::Normal, 100.0) {
                    session.close(idx);
                    close = true;
                }
                match menu::dialog_buttons(
                    ui,
                    t,
                    &[
                        (tr("Cancel"), ButtonKind::Normal),
                        (tr("Save"), ButtonKind::Default),
                    ],
                ) {
                    Some(0) => close = true,
                    Some(_) => {
                        if session.save(idx).is_ok()
                            && session.docs.get(idx).is_some_and(|d| !d.is_dirty())
                        {
                            session.close(idx);
                        }
                        close = true;
                    }
                    None => {}
                }
            });
            if m.should_close() {
                close = true;
            }
        }
    }
    if close {
        *dialog = Dialog::None;
    }
}
