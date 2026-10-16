//! TexCraft's egui workbench: the whole window (toolbar, tabs, sidebar, editor, PDF viewer,
//! dialogs) drawn from a [`Session`]. Thin by design: every action goes through engine commands.
//!
//! Platform work (file panels, Finder, quitting, native edit actions) is handed to the app as
//! [`Request`]s so this crate stays free of OS dependencies.

use std::path::PathBuf;
use std::time::Instant;

use egui::{Context, Rect, Sense, Stroke, pos2, vec2};
use texcraft_engine::{Effect, Layout, NativeEdit, Session, ThemeChoice, registry};

pub mod ai_inline;
pub mod ai_logos;
pub mod ai_panel;
mod cjk_font;
mod clock;
pub mod dialogs;
pub mod editor;
pub mod git_panel;
pub mod icons;
pub mod menu;
pub mod pdf_view;
pub mod prefs;
pub mod sidebar;
pub mod symbols;
pub mod theme;
pub mod toolbar;
pub mod update;
pub mod welcome;
pub mod widgets;

use dialogs::{Dialog, DialogAction};
use theme::{HEADER_HEIGHT, SIDEBAR_WIDTH, TOOLBAR_HEIGHT, Tokens};

/// Something only the app (platform layer) can do.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Show the open panel (files).
    Open,
    /// Show the open panel (folders).
    OpenFolder,
    /// Show the save panel for document `doc`.
    SaveAs(usize),
    /// Show the save panel for the PDF.
    ExportPdf,
    /// Reveal a path in Finder / Explorer.
    Reveal(PathBuf),
    /// Open a file with its default application.
    OpenExternally(PathBuf),
    /// Perform a native text-edit action in the focused text widget.
    Native(NativeEdit),
    /// Put text on the clipboard.
    CopyText(String),
    /// Quit.
    Quit,
}

/// The window's UI state.
pub struct Workbench {
    /// Editor pane state.
    pub editor: editor::EditorState,
    /// PDF pane state.
    pub pdf: pdf_view::PdfView,
    /// Sidebar state.
    pub sidebar: sidebar::SidebarState,
    /// The open dialog.
    pub dialog: Dialog,
    /// The Settings window and the Typeset Configuration sheet.
    pub prefs: prefs::Prefs,
    /// The welcome window.
    pub welcome: welcome::Welcome,
    /// The main window is hidden while the launch welcome window is up.
    main_hidden: bool,
    /// The update window is open.
    pub update_window: bool,
    /// Requests for the app.
    pub requests: Vec<Request>,
    /// Leave room for the macOS window buttons in the toolbar.
    pub traffic_lights: bool,
    /// Handle keyboard shortcuts in egui (false when a native menu bar delivers them).
    pub handle_shortcuts: bool,
    /// Editor share of the editor+PDF width.
    pub split: f32,
    tokens: Tokens,
    template: usize,
}

impl Workbench {
    /// Sets up fonts and style on `ctx`.
    pub fn new(ctx: &Context) -> Self {
        theme::install_fonts(ctx);
        theme::install_style(ctx);
        // SVG logos (AI providers and models).
        egui_extras::install_image_loaders(ctx);
        let tokens = Tokens::light();
        ctx.set_visuals(tokens.visuals());
        Self {
            editor: editor::EditorState::default(),
            pdf: pdf_view::PdfView::default(),
            sidebar: sidebar::SidebarState::default(),
            dialog: Dialog::None,
            prefs: prefs::Prefs::default(),
            welcome: welcome::Welcome::default(),
            main_hidden: false,
            update_window: false,
            requests: Vec::new(),
            traffic_lights: cfg!(target_os = "macos"),
            handle_shortcuts: true,
            split: 0.5,
            tokens,
            template: 1,
        }
    }

    /// Closes the Settings or PDF window if it has keyboard focus (⌘W from the menu bar).
    /// Returns whether one was closed.
    pub fn close_focused_window(&mut self, ctx: &Context) -> bool {
        let focused =
            |id: egui::ViewportId| ctx.input_for(id, |i| i.viewport().focused == Some(true));
        if self.prefs.open && focused(prefs::settings_viewport()) {
            self.prefs.open = false;
            return true;
        }
        if self.pdf.window && focused(egui::ViewportId::from_hash_of("texcraft-pdf-window")) {
            self.pdf.window = false;
            return true;
        }
        if self.welcome.open && !self.welcome.inline && focused(welcome::welcome_viewport()) {
            self.welcome.open = false;
            if self.welcome.launch {
                // Like its close button: the start-up window was the only window.
                self.main_hidden = false;
                self.welcome.launch = false;
                self.requests.push(Request::Quit);
            }
            return true;
        }
        false
    }

    /// The current design tokens.
    pub fn tokens(&self) -> &Tokens {
        &self.tokens
    }

    fn update_theme(&mut self, ctx: &Context, session: &Session) {
        let dark = match session.view.theme {
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
            ThemeChoice::System => ctx.system_theme().is_some_and(|t| t == egui::Theme::Dark),
        };
        let base = if dark {
            Tokens::dark()
        } else {
            Tokens::light()
        };
        let wanted = base.with_editor_theme(session.view.editor_theme);
        if wanted != self.tokens {
            self.tokens = wanted;
            ctx.set_visuals(self.tokens.visuals());
        }
    }

    /// Applies effects the engine queued.
    pub fn apply_effects(&mut self, session: &mut Session) {
        while let Some(e) = session.take_effect() {
            match e {
                Effect::RevealLine { doc, line } => {
                    self.editor.reveal = Some((doc, line));
                    self.editor.focus = true;
                }
                Effect::SelectionChanged { doc } => self.editor.push_selection = Some(doc),
                Effect::ShowInPdf { rects, flash } => {
                    if session.view.layout == Layout::EditorOnly && flash {
                        session.view.layout = Layout::Split;
                    }
                    self.pdf.show_rects(rects, flash);
                }
                Effect::PdfChanged => {}
                Effect::OpenDialog => self.requests.push(Request::Open),
                Effect::OpenFolderDialog => self.requests.push(Request::OpenFolder),
                Effect::SaveAsDialog { doc } => self.requests.push(Request::SaveAs(doc)),
                Effect::ExportPdfDialog => self.requests.push(Request::ExportPdf),
                Effect::Reveal(p) => self.requests.push(Request::Reveal(p)),
                Effect::OpenExternally(p) => self.requests.push(Request::OpenExternally(p)),
                Effect::Native(n) => self.requests.push(Request::Native(n)),
                Effect::Quit => self.requests.push(Request::Quit),
                Effect::UpdateWindow => self.update_window = true,
                Effect::InlineSettings => self.editor.inline_open = true,
                Effect::CopyText(text) => self.requests.push(Request::CopyText(text)),
                Effect::BeginRename(path) => self.sidebar.begin_rename(path),
                Effect::GitDiff => self.sidebar.git.diff_open = true,
                Effect::TemplateGallery | Effect::Welcome => self.welcome.show_window(false),
                Effect::GotoLineDialog => self.dialog = Dialog::GotoLine(String::new()),
                Effect::Find { replace } => {
                    self.editor.find.open = true;
                    self.editor.find.focus = true;
                    if replace {
                        self.editor.find.replace_open = true;
                    }
                    if let Some(d) = session.doc() {
                        let a = texcraft_engine::edit::byte_of(&d.text, d.selection.start());
                        let b = texcraft_engine::edit::byte_of(&d.text, d.selection.end());
                        if let Some(sel) = d
                            .text
                            .get(a..b)
                            .filter(|s| !s.is_empty() && !s.contains('\n'))
                        {
                            self.editor.find.query = sel.to_string();
                        }
                    }
                }
                Effect::FindNext { backwards } => {
                    editor::find_next(session, &mut self.editor, backwards)
                }
                Effect::ConfirmClose { doc } => self.dialog = Dialog::ConfirmClose(doc),
                Effect::Alert { title, message } => self.dialog = Dialog::Alert { title, message },
                Effect::About => self.dialog = Dialog::About,
                Effect::Shortcuts => self.dialog = Dialog::Shortcuts,
                Effect::Settings => {
                    self.prefs.open = true;
                    self.prefs.focus = true;
                }
                Effect::AiSettings => {
                    self.prefs.open = true;
                    self.prefs.focus = true;
                    self.prefs.pane = prefs::Pane::Ai;
                }
                Effect::TypesetConfiguration => self.dialog = Dialog::TypesetConfiguration,
                Effect::GotoPage(p) => self.pdf.goto_page(p),
                Effect::FocusSidebarSearch => self.sidebar.focus_search = true,
                Effect::Autocomplete => self.editor.focus = true,
            }
        }
    }

    /// Runs keyboard shortcuts from the command registry (exact modifier match).
    fn shortcuts(&mut self, ctx: &Context, session: &mut Session) {
        if !self.handle_shortcuts {
            return;
        }
        let mut fired: Vec<&'static str> = Vec::new();
        ctx.input_mut(|i| {
            i.events.retain(|ev| {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = ev
                else {
                    return true;
                };
                for c in registry() {
                    let Some(sc) = c.shortcut else { continue };
                    // Native edit actions stay with the text widget.
                    if c.id.starts_with("edit.")
                        && matches!(
                            c.id,
                            "edit.undo"
                                | "edit.redo"
                                | "edit.cut"
                                | "edit.copy"
                                | "edit.paste"
                                | "edit.selectAll"
                        )
                    {
                        continue;
                    }
                    if key_matches(sc.key, *key)
                        && modifiers.command == sc.cmd
                        && modifiers.shift == sc.shift
                        && modifiers.alt == sc.alt
                        && (if cfg!(target_os = "macos") {
                            modifiers.ctrl == sc.ctrl
                        } else {
                            modifiers.ctrl == (sc.ctrl || sc.cmd)
                        })
                    {
                        fired.push(c.id);
                        return false;
                    }
                }
                true
            });
        });
        for id in fired {
            if let Err(e) = session.run(id, &serde_json::Value::Null) {
                session.set_status(e.to_string());
            }
        }
    }

    /// Draws one frame.
    pub fn ui(&mut self, root: &mut egui::Ui, session: &mut Session) {
        let ctx = &root.ctx().clone();
        self.update_theme(ctx, session);
        self.shortcuts(ctx, session);
        if let Some(wait) = session.tick(Instant::now()) {
            ctx.request_repaint_after(wait);
        }
        self.apply_effects(session);
        self.pdf.sync_document(ctx, session);
        session.refresh_index(false);
        let t = self.tokens.clone();

        // Toolbar.
        egui::Panel::top("toolbar")
            .exact_size(TOOLBAR_HEIGHT)
            .frame(egui::Frame::NONE)
            .show_separator_line(false)
            .show(root, |ui| {
                let rect = ui.max_rect();
                let inset = toolbar::left_inset(ui.ctx(), self.traffic_lights);
                toolbar::show(ui, &t, session, rect, inset);
            });
        // Tabs.
        if session.docs.len() > 1 {
            egui::Panel::top("tabs")
                .exact_size(HEADER_HEIGHT)
                .frame(egui::Frame::NONE)
                .show_separator_line(false)
                .show(root, |ui| {
                    let rect = ui.max_rect();
                    toolbar::tabs(ui, &t, session, rect);
                });
        }
        // Sidebar.
        if session.view.sidebar {
            egui::Panel::left("sidebar")
                .resizable(true)
                .default_size(SIDEBAR_WIDTH)
                .size_range(170.0..=380.0)
                .frame(egui::Frame::NONE.fill(t.sidebar_bg))
                .show_separator_line(true)
                .show(root, |ui| {
                    let rect = ui.max_rect();
                    // egui keeps the width the contents occupy: claim all of it, or the panel
                    // falls back to its minimum and every drag of the divider is undone.
                    ui.expand_to_include_rect(rect);
                    sidebar::show(ui, &t, session, &mut self.sidebar, rect);
                });
        }
        // The PDF in its own window (e.g. on an external display): the main window shows only
        // the editor meanwhile.
        if self.pdf.window {
            let t2 = t.clone();
            let pdf = &mut self.pdf;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("texcraft-pdf-window"),
                egui::ViewportBuilder::default()
                    .with_title("TexCraft — PDF")
                    .with_inner_size([620.0, 820.0])
                    .with_min_inner_size([320.0, 300.0]),
                |ui, _class| {
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        pdf.window = false;
                    }
                    let rect = ui.max_rect();
                    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    pdf_view::show(&mut c, &t2, session, pdf);
                },
            );
        }
        let layout = if self.pdf.window {
            Layout::EditorOnly
        } else {
            session.view.layout
        };

        // Editor and PDF.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(t.editor_bg))
            .show(root, |ui| {
                let rect = ui.max_rect();
                match layout {
                    Layout::EditorOnly => {
                        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                        editor::show(&mut c, &t, session, &mut self.editor);
                    }
                    Layout::PdfOnly => {
                        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                        pdf_view::show(&mut c, &t, session, &mut self.pdf);
                    }
                    Layout::Split => {
                        let split_x = (rect.left() + rect.width() * self.split).round();
                        let left = Rect::from_min_max(rect.min, pos2(split_x, rect.bottom()));
                        let right = Rect::from_min_max(pos2(split_x + 1.0, rect.top()), rect.max);
                        {
                            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(left));
                            c.set_clip_rect(left);
                            editor::show(&mut c, &t, session, &mut self.editor);
                        }
                        {
                            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(right));
                            c.set_clip_rect(right);
                            pdf_view::show(&mut c, &t, session, &mut self.pdf);
                        }
                        // Draggable divider.
                        let handle = Rect::from_min_max(
                            pos2(split_x - 3.0, rect.top()),
                            pos2(split_x + 4.0, rect.bottom()),
                        );
                        let resp =
                            ui.interact(handle, ui.id().with("split-divider"), Sense::drag());
                        if resp.hovered() || resp.dragged() {
                            ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        }
                        if resp.dragged()
                            && let Some(p) = resp.interact_pointer_pos()
                        {
                            self.split = ((p.x - rect.left()) / rect.width()).clamp(0.2, 0.8);
                        }
                        ui.painter().line_segment(
                            [
                                pos2(split_x + 0.5, rect.top()),
                                pos2(split_x + 0.5, rect.bottom()),
                            ],
                            Stroke::new(1.0, t.divider),
                        );
                    }
                }
            });

        // Start a live pass in the same frame as the keystroke that asked for it.
        if let Some(wait) = session.tick(Instant::now()) {
            ctx.request_repaint_after(wait);
        }

        // Scroll lock: the PDF follows the caret.
        if session.view.scroll_lock && self.editor.caret_moved {
            session.forward_sync(false);
            self.apply_effects(session);
        }

        // Dialogs. The Typeset Configuration sheet and the Settings window have their own module.
        if self.dialog == Dialog::TypesetConfiguration {
            if prefs::typeset_sheet(ctx, &t, session, &mut self.prefs) {
                self.dialog = Dialog::None;
            }
        } else {
            dialogs::show(ctx, &t, session, &mut self.dialog);
        }
        // At launch the main window waits hidden behind the welcome window: the update window
        // (which opens at start-up when there is a newer release) shows on the welcome window.
        let update_on_welcome = self.welcome.open && self.welcome.launch && !self.welcome.inline;
        if !update_on_welcome {
            update::show(ctx, &t, session, &mut self.update_window);
        }
        // The welcome window; at launch the main window waits hidden behind it.
        if self.welcome.open && self.welcome.launch && !self.welcome.inline && !self.main_hidden {
            ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::Visible(false),
            );
            self.main_hidden = true;
        }
        if let Some(action) = welcome::show(
            ctx,
            &t,
            session,
            &mut self.welcome,
            &mut self.template,
            update_on_welcome.then_some(&mut self.update_window),
        ) {
            self.requests.push(match action {
                DialogAction::Open => Request::Open,
                DialogAction::OpenFolder => Request::OpenFolder,
                DialogAction::Quit => {
                    // The hidden main window is not shown on the way out.
                    self.main_hidden = false;
                    self.welcome.launch = false;
                    Request::Quit
                }
            });
        }
        if !self.welcome.open && self.main_hidden {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            self.main_hidden = false;
            self.welcome.launch = false;
        }
        if prefs::settings_window(
            ctx,
            &t,
            session,
            &mut self.prefs,
            &mut self.sidebar.outline_depth,
        ) && self.dialog == Dialog::None
        {
            self.dialog = Dialog::TypesetConfiguration;
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
        self.apply_effects(session);
        let _ = vec2(0.0, 0.0);
    }
}

/// Whether a registry key name matches an egui key.
pub fn key_matches(name: &str, key: egui::Key) -> bool {
    use egui::Key as K;
    match name {
        "/" => key == K::Slash,
        "\\" => key == K::Backslash,
        "[" => key == K::OpenBracket,
        "]" => key == K::CloseBracket,
        "=" => key == K::Equals || key == K::Plus,
        "-" => key == K::Minus,
        "." => key == K::Period,
        "," => key == K::Comma,
        ";" => key == K::Semicolon,
        "Tab" => key == K::Tab,
        "Left" => key == K::ArrowLeft,
        "Right" => key == K::ArrowRight,
        "Up" => key == K::ArrowUp,
        "Down" => key == K::ArrowDown,
        "Enter" => key == K::Enter,
        "Escape" => key == K::Escape,
        "Backspace" => key == K::Backspace,
        other => K::from_name(other).is_some_and(|k| k == key),
    }
}
