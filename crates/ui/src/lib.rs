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
pub mod flash;
pub mod git_panel;
pub mod icons;
pub mod menu;
pub mod model_picker;
pub mod pdf_select;
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

/// How long the sidebar takes to slide open or shut.
const SIDEBAR_SLIDE_SECONDS: f32 = 0.22;

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
    /// It shows on the Settings window (Check for Updates… was clicked there).
    update_on_settings: bool,
    /// The open documents (count, active, its path) as of the last frame.
    docs_seen: Option<(usize, usize, Option<PathBuf>)>,
    /// A dialog was up in the last frame.
    had_dialog: bool,
    /// Requests for the app.
    pub requests: Vec<Request>,
    /// Leave room for the macOS window buttons in the toolbar.
    pub traffic_lights: bool,
    /// Handle keyboard shortcuts in egui (false when a native menu bar delivers them).
    pub handle_shortcuts: bool,
    /// Editor share of the editor+PDF width.
    pub split: f32,
    tokens: Tokens,
    /// The egui theme whose style holds the tokens' visuals.
    visuals_for: Option<egui::Theme>,
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
            update_on_settings: false,
            docs_seen: None,
            had_dialog: false,
            requests: Vec::new(),
            traffic_lights: cfg!(target_os = "macos"),
            handle_shortcuts: true,
            split: 0.5,
            tokens,
            visuals_for: None,
            template: 1,
        }
    }

    /// Hands edit events from the native Edit menu to the window with keyboard focus. The menu
    /// bar takes ⌘V and co. before any window sees the keys, and the app's input hook reaches only
    /// the main window, so the Settings and welcome windows get theirs here, on their next frame.
    /// Returns the events for the main window: all of them when neither has focus.
    pub fn route_edit_events(
        &mut self,
        ctx: &Context,
        events: Vec<egui::Event>,
    ) -> Vec<egui::Event> {
        if events.is_empty() {
            return events;
        }
        let focused =
            |id: egui::ViewportId| ctx.input_for(id, |i| i.viewport().focused == Some(true));
        if !self.prefs.inline
            && self.prefs.model_picker().is_open()
            && focused(model_picker::picker_viewport())
        {
            self.prefs.model_picker().edit_events.extend(events);
            Vec::new()
        } else if self.prefs.open && !self.prefs.inline && focused(prefs::settings_viewport()) {
            self.prefs.edit_events.extend(events);
            Vec::new()
        } else if self.welcome.open && !self.welcome.inline && focused(welcome::welcome_viewport())
        {
            self.welcome.edit_events.extend(events);
            Vec::new()
        } else {
            events
        }
    }

    /// Closes the Settings or PDF window if it has keyboard focus (⌘W from the menu bar).
    /// Returns whether one was closed.
    pub fn close_focused_window(&mut self, ctx: &Context) -> bool {
        let focused =
            |id: egui::ViewportId| ctx.input_for(id, |i| i.viewport().focused == Some(true));
        if self.prefs.model_picker().is_open() && focused(model_picker::picker_viewport()) {
            self.prefs.model_picker().close();
            return true;
        }
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
            // Asked of AppKit on macOS: a window with a chosen appearance hears of no system
            // change, so egui's system theme can be stale right after a Light or Dark choice.
            ThemeChoice::System => texcraft_macos::system_dark()
                .unwrap_or_else(|| ctx.system_theme().is_some_and(|t| t == egui::Theme::Dark)),
        };
        let base = if dark {
            Tokens::dark()
        } else {
            Tokens::light()
        };
        let wanted = base.with_editor_theme(session.view.editor_theme);
        // egui keeps each native window's appearance (and with it the welcome window's
        // translucent material, the window buttons) in step with its theme preference, so a
        // Light choice under a dark system gets light windows too.
        let preference = match session.view.theme {
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
            ThemeChoice::System => egui::ThemePreference::System,
        };
        let switched = ctx.options(|o| o.theme_preference) != preference;
        if switched {
            ctx.set_theme(preference);
        }
        // Visuals go to egui's style for its current theme: set them again when that changes
        // (the system theme arrives after start-up, or the preference moves).
        let theme = ctx.theme();
        if wanted != self.tokens || self.visuals_for != Some(theme) {
            self.tokens = wanted;
            ctx.set_visuals(self.tokens.visuals());
            self.visuals_for = Some(theme);
        }
    }

    /// Applies effects the engine queued.
    pub fn apply_effects(&mut self, ctx: &Context, session: &mut Session) {
        while let Some(e) = session.take_effect() {
            match e {
                Effect::RevealLine { doc, line } => {
                    self.editor.reveal = Some((doc, line));
                    self.editor.focus = true;
                    self.pdf.active = false;
                }
                Effect::FlashSource { doc, start, end } => {
                    self.editor.flash = Some(editor::SourceFlash {
                        doc,
                        start,
                        end,
                        at: Instant::now(),
                    });
                    // ⌘-click in the separate PDF window: the editor comes forward to type in.
                    if self.pdf.window {
                        ctx.send_viewport_cmd_to(
                            egui::ViewportId::ROOT,
                            egui::ViewportCommand::Focus,
                        );
                    }
                }
                Effect::SelectPdfText(sel) => self.pdf.select(sel),
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
                Effect::Native(NativeEdit::Copy) if self.pdf_copies(ctx, session) => {
                    if let Err(e) = session.run("pdf.copySelection", &serde_json::Value::Null) {
                        session.set_status(e.to_string());
                    }
                }
                Effect::Native(n) => self.requests.push(Request::Native(n)),
                Effect::Quit => self.requests.push(Request::Quit),
                Effect::UpdateWindow => {
                    // It shows on the window in front: Settings when asked for there (General →
                    // Check for Updates…), the main window when that is. Each news (checked,
                    // downloaded) moves it there.
                    let focused = |id| ctx.input_for(id, |i| i.viewport().focused == Some(true));
                    if self.prefs.open && !self.prefs.inline && focused(prefs::settings_viewport())
                    {
                        self.update_on_settings = true;
                    } else if !self.update_window || focused(egui::ViewportId::ROOT) {
                        self.update_on_settings = false;
                    }
                    self.update_window = true;
                }
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

    /// ⌘C (the Edit menu's Copy) copies the PDF's selected text: the user last clicked in the
    /// PDF and its window has keyboard focus.
    fn pdf_copies(&self, ctx: &Context, session: &Session) -> bool {
        let viewport = if self.pdf.window {
            egui::ViewportId::from_hash_of("texcraft-pdf-window")
        } else {
            egui::ViewportId::ROOT
        };
        self.pdf.owns_copy(session)
            && ctx.input_for(viewport, |i| i.viewport().focused != Some(false))
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
        self.apply_effects(ctx, session);
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
        // Sidebar: slides open and shut, the editor following (egui's collapsible panel, at
        // a pace the eye can follow rather than its quick default). Dragging the divider past
        // the narrowest width closes it, as in Finder.
        let mut open = session.view.sidebar;
        let quick = ctx.global_style().animation_time;
        ctx.global_style_mut(|s| s.animation_time = SIDEBAR_SLIDE_SECONDS);
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(SIDEBAR_WIDTH)
            .size_range(170.0..=380.0)
            .frame(egui::Frame::NONE.fill(t.sidebar_bg))
            .show_separator_line(true)
            .drag_to_open(false)
            .show_collapsible(root, &mut open, |ui| {
                let rect = ui.max_rect();
                // egui keeps the width the contents occupy: claim all of it, or the panel
                // falls back to its minimum and every drag of the divider is undone.
                ui.expand_to_include_rect(rect);
                sidebar::show(ui, &t, session, &mut self.sidebar, rect);
            });
        ctx.global_style_mut(|s| s.animation_time = quick);
        session.view.sidebar = open;
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
            self.apply_effects(ctx, session);
        }

        // Dialogs. The Typeset Configuration sheet and the Settings window have their own module.
        if self.dialog == Dialog::TypesetConfiguration {
            if prefs::typeset_sheet(ctx, &t, session, &mut self.prefs) {
                self.dialog = Dialog::None;
            }
        } else {
            dialogs::show(ctx, &t, session, &mut self.dialog);
        }
        if !self.update_window || !self.prefs.open || self.prefs.inline {
            self.update_on_settings = false;
        }
        let update_on_welcome = self.update_on_welcome();
        if !update_on_welcome && !self.update_on_settings {
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
            self.update_on_settings.then_some(&mut self.update_window),
        ) && self.dialog == Dialog::None
        {
            self.dialog = Dialog::TypesetConfiguration;
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
        self.apply_effects(ctx, session);
        self.bring_main_forward(ctx, session);
        let _ = vec2(0.0, 0.0);
    }

    /// Whether the update window shows on the welcome window: at launch the main window waits
    /// hidden behind it (the update window opens at start-up when there is a newer release),
    /// unless it was asked for in Settings.
    fn update_on_welcome(&self) -> bool {
        self.welcome.open && self.welcome.launch && !self.welcome.inline && !self.update_on_settings
    }

    /// Brings the main window forward when it gets something to show: a dialog (About, an alert,
    /// a save prompt) or a newly opened or created document. Menu commands reach it while the
    /// Settings, welcome or PDF window is in front, and at launch it waits hidden behind the
    /// welcome window. Returns whether it did. It takes focus only from the app's own windows.
    fn bring_main_forward(&mut self, ctx: &Context, session: &Session) -> bool {
        let docs = (
            session.docs.len(),
            session.active,
            session.doc().and_then(|d| d.path.clone()),
        );
        let changed = self.docs_seen.as_ref().is_some_and(|seen| *seen != docs);
        self.docs_seen = Some(docs);
        let has_dialog = self.dialog != Dialog::None;
        let dialog = has_dialog && !self.had_dialog;
        self.had_dialog = has_dialog;
        if !(changed || dialog) {
            return false;
        }
        let root = egui::ViewportId::ROOT;
        if self.main_hidden {
            if changed {
                // The document takes the start-up window's place, as picking it there does.
                self.welcome.open = false;
            } else {
                // The dialog shows on the main window; the welcome window stays up as an
                // ordinary window (closing it no longer quits).
                ctx.send_viewport_cmd_to(root, egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd_to(root, egui::ViewportCommand::Focus);
                self.main_hidden = false;
                self.welcome.launch = false;
            }
            return true;
        }
        let focused = |id| ctx.input_for(id, |i| i.viewport().focused == Some(true));
        let secondary = (self.prefs.open
            && !self.prefs.inline
            && focused(prefs::settings_viewport()))
            || (self.welcome.open && !self.welcome.inline && focused(welcome::welcome_viewport()))
            || (self.prefs.model_picker().is_open() && focused(model_picker::picker_viewport()))
            || (self.pdf.window && focused(egui::ViewportId::from_hash_of("texcraft-pdf-window")));
        if secondary {
            ctx.send_viewport_cmd_to(root, egui::ViewportCommand::Focus);
        }
        secondary
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

#[cfg(test)]
mod tests {
    use super::*;

    /// ⌘C (the Edit menu's Copy) copies the PDF's selection after a click in the PDF, else the
    /// text widget's; inverse search flashes the source and gives ⌘C back to the editor.
    #[test]
    fn copy_follows_the_last_click_and_inverse_search_flashes() {
        let ctx = Context::default();
        let mut bench = Workbench::new(&ctx);
        let mut session = Session::new(std::sync::Arc::new(|| {}));
        session.update.url = "/nonexistent/texcraft-latest.json".into();
        session.pdf.path = Some("/nonexistent/x.pdf".into());
        let copy = |bench: &mut Workbench, session: &mut Session| {
            bench.requests.clear();
            session.run("edit.copy", &serde_json::Value::Null).unwrap();
            bench.apply_effects(&ctx, session);
            bench.requests.clone()
        };
        session.pdf.selected_text = "选中的 text".into();
        // The last click was elsewhere: the text widget copies.
        assert_eq!(
            copy(&mut bench, &mut session),
            [Request::Native(NativeEdit::Copy)]
        );
        // `pdf.selectText` (or a click and drag in the PDF): the PDF's selection.
        session
            .run(
                "pdf.selectText",
                &serde_json::json!({"page": 1, "x": 10, "y": 10, "toX": 90, "toY": 20}),
            )
            .unwrap();
        bench.apply_effects(&ctx, &mut session);
        assert!(bench.pdf.active);
        assert_eq!(
            copy(&mut bench, &mut session),
            [Request::CopyText("选中的 text".into())]
        );
        // Inverse search: the editor flashes the source and takes the keyboard back.
        session
            .effects
            .push_back(Effect::RevealLine { doc: 0, line: 3 });
        session.effects.push_back(Effect::FlashSource {
            doc: 0,
            start: 20,
            end: 40,
        });
        bench.apply_effects(&ctx, &mut session);
        let flash = bench.editor.flash.unwrap();
        assert_eq!((flash.doc, flash.start, flash.end), (0, 20, 40));
        assert!(bench.editor.focus && !bench.pdf.active);
        assert_eq!(
            copy(&mut bench, &mut session),
            [Request::Native(NativeEdit::Copy)]
        );
        // Clearing the selection.
        session
            .run("pdf.clearSelection", &serde_json::Value::Null)
            .unwrap();
        bench.apply_effects(&ctx, &mut session);
        assert_eq!(bench.pdf.selected_range(), None);
    }

    fn focus(ctx: &Context, id: egui::ViewportId) {
        ctx.input_mut_for(id, |i| {
            i.raw.viewport_id = id;
            i.raw.viewports.insert(
                id,
                egui::ViewportInfo {
                    focused: Some(true),
                    ..Default::default()
                },
            );
        });
    }

    /// Edit events from the menu bar go to the focused Settings or welcome window, else to the
    /// main window.
    #[test]
    fn menu_edit_events_go_to_the_focused_window() {
        let ctx = Context::default();
        let mut bench = Workbench::new(&ctx);
        let paste = || vec![egui::Event::Paste("text".into())];
        // Nothing else open: the main window.
        assert_eq!(bench.route_edit_events(&ctx, paste()).len(), 1);
        // Settings open but not focused: still the main window.
        bench.prefs.open = true;
        assert_eq!(bench.route_edit_events(&ctx, paste()).len(), 1);
        assert!(bench.prefs.edit_events.is_empty());
        // Settings focused: its own.
        focus(&ctx, prefs::settings_viewport());
        assert!(bench.route_edit_events(&ctx, paste()).is_empty());
        assert_eq!(bench.prefs.edit_events.len(), 1);
        // Drawn inside the main window (control channel): the main window.
        bench.prefs.edit_events.clear();
        bench.prefs.inline = true;
        assert_eq!(bench.route_edit_events(&ctx, paste()).len(), 1);
        // The welcome window's filter field.
        bench.prefs.open = false;
        bench.welcome.open = true;
        focus(&ctx, welcome::welcome_viewport());
        assert!(bench.route_edit_events(&ctx, paste()).is_empty());
        assert_eq!(bench.welcome.edit_events.len(), 1);
    }

    /// Check for Updates… clicked in the Settings window shows the update window there, in
    /// front; asked for anywhere else, it shows on the main window.
    #[test]
    fn update_window_shows_where_it_was_asked_for() {
        let ctx = Context::default();
        let mut bench = Workbench::new(&ctx);
        let mut session = Session::new(std::sync::Arc::new(|| {}));
        // Never the network: a local file that is not there.
        session.update.url = "/nonexistent/texcraft-latest.json".into();
        let check = |bench: &mut Workbench, session: &mut Session| {
            bench.update_window = false;
            assert!(
                session
                    .run("app.checkForUpdates", &serde_json::Value::Null)
                    .is_ok()
            );
            bench.apply_effects(&ctx, session);
            assert!(bench.update_window);
            bench.update_on_settings
        };
        assert!(!check(&mut bench, &mut session), "main window");
        bench.prefs.open = true;
        assert!(!check(&mut bench, &mut session), "Settings behind");
        focus(&ctx, prefs::settings_viewport());
        assert!(check(&mut bench, &mut session), "Settings in front");
        bench.prefs.inline = true;
        assert!(!check(&mut bench, &mut session), "Settings drawn inline");
        // Already open on the main window (a background check), then asked for in Settings:
        // it moves there.
        bench.prefs.inline = false;
        bench.update_window = true;
        bench.update_on_settings = false;
        assert!(
            session
                .run("app.checkForUpdates", &serde_json::Value::Null)
                .is_ok()
        );
        bench.apply_effects(&ctx, &mut session);
        assert!(bench.update_on_settings, "moves to Settings");
        // With the launch welcome window up too, it shows on Settings only.
        bench.welcome.show_window(true);
        assert!(!bench.update_on_welcome(), "drawn once");
        bench.update_on_settings = false;
        assert!(bench.update_on_welcome());
    }

    /// Sets whether a window has keyboard focus.
    fn set_focus(ctx: &Context, id: egui::ViewportId, on: bool) {
        ctx.input_mut_for(id, |i| {
            i.raw.viewport_id = id;
            i.raw.viewports.insert(
                id,
                egui::ViewportInfo {
                    focused: Some(on),
                    ..Default::default()
                },
            );
        });
    }

    /// The main window comes forward for a dialog or a new document raised while another of the
    /// app's windows is in front, and at launch while it waits hidden behind the welcome window.
    #[test]
    fn main_window_comes_forward_for_documents_and_dialogs() {
        let ctx = Context::default();
        let mut bench = Workbench::new(&ctx);
        let mut session = Session::new(std::sync::Arc::new(|| {}));
        // Launch: the welcome window up, the main window hidden behind it.
        bench.welcome.show_window(true);
        bench.main_hidden = true;
        assert!(!bench.bring_main_forward(&ctx, &session), "first look");
        // Window → Welcome Window while it is up: still the start-up window.
        bench.welcome.show_window(false);
        assert!(bench.welcome.launch);
        // TexCraft → About: the main window shows for it; the welcome window stays, ordinary.
        bench.dialog = Dialog::About;
        assert!(bench.bring_main_forward(&ctx, &session));
        assert!(!bench.main_hidden && bench.welcome.open && !bench.welcome.launch);
        bench.dialog = Dialog::None;
        assert!(!bench.bring_main_forward(&ctx, &session));
        // Hidden again at launch, then ⌘N: the new document takes the welcome window's place.
        bench.welcome.show_window(true);
        bench.main_hidden = true;
        assert!(session.run("file.new", &serde_json::Value::Null).is_ok());
        assert!(bench.bring_main_forward(&ctx, &session));
        assert!(!bench.welcome.open);
        // Main window shown, Settings in front: a dialog brings the main window forward.
        bench.main_hidden = false;
        bench.prefs.open = true;
        set_focus(&ctx, prefs::settings_viewport(), true);
        bench.dialog = Dialog::Shortcuts;
        assert!(bench.bring_main_forward(&ctx, &session));
        bench.dialog = Dialog::None;
        assert!(!bench.bring_main_forward(&ctx, &session));
        // The main window itself in front (or another app): nothing to take focus from.
        set_focus(&ctx, prefs::settings_viewport(), false);
        set_focus(&ctx, egui::ViewportId::ROOT, true);
        bench.dialog = Dialog::About;
        assert!(!bench.bring_main_forward(&ctx, &session));
    }

    /// The system theme arrives after start-up (egui starts in its fallback theme): the style
    /// egui then uses gets the tokens' visuals too, not egui's stock ones.
    #[test]
    fn visuals_follow_egui_into_the_system_theme() {
        let ctx = Context::default();
        let mut bench = Workbench::new(&ctx);
        let session = Session::new(std::sync::Arc::new(|| {}));
        bench.update_theme(&ctx, &session);
        let input = egui::RawInput {
            system_theme: Some(egui::Theme::Light),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            bench.update_theme(ui.ctx(), &session);
            assert_eq!(ui.ctx().theme(), egui::Theme::Light);
            assert!(ui.ctx().global_style().visuals == bench.tokens().visuals());
        });
        out.textures_delta.clear();
    }

    /// The Appearance choice reaches egui's theme preference (which sets the native windows'
    /// appearance), and the visuals land on the style egui then uses.
    #[test]
    fn appearance_choice_sets_the_window_theme() {
        let ctx = Context::default();
        let mut bench = Workbench::new(&ctx);
        let mut session = Session::new(std::sync::Arc::new(|| {}));
        for (choice, preference, dark) in [
            (ThemeChoice::Light, egui::ThemePreference::Light, false),
            (ThemeChoice::Dark, egui::ThemePreference::Dark, true),
            (ThemeChoice::Light, egui::ThemePreference::Light, false),
        ] {
            session.view.theme = choice;
            bench.update_theme(&ctx, &session);
            assert_eq!(ctx.options(|o| o.theme_preference), preference);
            assert_eq!(bench.tokens().dark, dark);
            assert_eq!(ctx.global_style().visuals.dark_mode, dark);
        }
        session.view.theme = ThemeChoice::System;
        bench.update_theme(&ctx, &session);
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::System
        );
        assert_eq!(
            ctx.global_style().visuals.dark_mode,
            bench.tokens().dark,
            "visuals follow the tokens"
        );
    }
}
