//! The command registry. Every menu item, toolbar button and shortcut is one of these; the
//! UI, CLI and control channel dispatch them by id through [`Session::run`].
//!
//! Menus and shortcuts follow Texifier's (see `docs/texifier-reference.md` §7–8).

use std::path::{Path, PathBuf};

use serde_json::Value;
use texcraft_project::{Engine, TypesetMode};

use crate::{
    CommandError, ContentsMode, Effect, Grading, Layout, NativeEdit, PageMode, ResultsMode,
    SearchMode, Session, SidebarTab, ThemeChoice, Zoom, edit,
};

/// A top-level menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Menu {
    /// The application menu (TexCraft).
    App,
    /// File.
    File,
    /// Edit.
    Edit,
    /// Snippets.
    Snippets,
    /// Typeset.
    Typeset,
    /// Workspace (panes, zoom, navigation).
    Workspace,
    /// Window.
    Window,
    /// Help.
    Help,
    /// Not in a menu (toolbar, control channel or parameterised commands).
    None,
}

impl Menu {
    /// Menu bar order.
    pub const BAR: [Menu; 8] = [
        Self::App,
        Self::File,
        Self::Edit,
        Self::Snippets,
        Self::Typeset,
        Self::Workspace,
        Self::Window,
        Self::Help,
    ];

    /// Title in the menu bar.
    pub fn title(self) -> &'static str {
        match self {
            Self::App => "TexCraft",
            Self::File => "File",
            Self::Edit => "Edit",
            Self::Snippets => "Snippets",
            Self::Typeset => "Typeset",
            Self::Workspace => "Workspace",
            Self::Window => "Window",
            Self::Help => "Help",
            Self::None => "",
        }
    }
}

/// A keyboard shortcut. `key` is a letter `A`–`Z`, a digit, one of `/ \\ [ ] = - . , ;`, or a
/// named key (`Tab`, `Left`, `Right`, `Up`, `Down`, `Enter`, `Escape`, `Backspace`).
/// `cmd` is ⌘ on macOS and Ctrl elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    /// The key.
    pub key: &'static str,
    /// ⌘ (Ctrl on other platforms).
    pub cmd: bool,
    /// ⇧
    pub shift: bool,
    /// ⌥
    pub alt: bool,
    /// ⌃ (the real Control key on macOS).
    pub ctrl: bool,
}

impl Shortcut {
    const fn cmd(key: &'static str) -> Self {
        Self {
            key,
            cmd: true,
            shift: false,
            alt: false,
            ctrl: false,
        }
    }
    const fn cmd_shift(key: &'static str) -> Self {
        Self {
            key,
            cmd: true,
            shift: true,
            alt: false,
            ctrl: false,
        }
    }
    const fn cmd_alt(key: &'static str) -> Self {
        Self {
            key,
            cmd: true,
            shift: false,
            alt: true,
            ctrl: false,
        }
    }
    const fn ctrl_cmd(key: &'static str) -> Self {
        Self {
            key,
            cmd: true,
            shift: false,
            alt: false,
            ctrl: true,
        }
    }
    const fn ctrl_shift(key: &'static str) -> Self {
        Self {
            key,
            cmd: false,
            shift: true,
            alt: false,
            ctrl: true,
        }
    }
    const fn alt(key: &'static str) -> Self {
        Self {
            key,
            cmd: false,
            shift: false,
            alt: true,
            ctrl: false,
        }
    }
    const fn ctrl(key: &'static str) -> Self {
        Self {
            key,
            cmd: false,
            shift: false,
            alt: false,
            ctrl: true,
        }
    }

    /// Human-readable form with macOS symbols, e.g. `⇧⌘L`.
    pub fn display(self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push('⌃');
        }
        if self.alt {
            s.push('⌥');
        }
        if self.shift {
            s.push('⇧');
        }
        if self.cmd {
            s.push('⌘');
        }
        s.push_str(match self.key {
            "Tab" => "⇥",
            "Left" => "←",
            "Right" => "→",
            "Up" => "↑",
            "Down" => "↓",
            "Enter" => "↩",
            "Escape" => "⎋",
            "Backspace" => "⌫",
            k => k,
        });
        s
    }
}

type RunFn = fn(&mut Session, &Value) -> Result<(), CommandError>;
type StateFn = fn(&Session) -> bool;

/// A command.
#[derive(Clone, Copy)]
pub struct CommandSpec {
    /// Stable id (`typeset.run`).
    pub id: &'static str,
    /// Menu / tooltip label.
    pub label: &'static str,
    /// Which menu shows it.
    pub menu: Menu,
    /// Separator group within the menu (a separator is drawn when it changes).
    pub group: u8,
    /// Default shortcut.
    pub shortcut: Option<Shortcut>,
    /// Whether it can run now.
    pub enabled: StateFn,
    /// Check-mark state, for toggles and radio items.
    pub checked: Option<StateFn>,
    /// Runs it.
    pub run: RunFn,
    /// Parameter documentation (empty when it takes none).
    pub params: &'static str,
}

impl std::fmt::Debug for CommandSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandSpec")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

fn always(_: &Session) -> bool {
    true
}
fn has_doc(s: &Session) -> bool {
    s.doc().is_some()
}
/// Native text editing (⌘Z, ⌘X, ⌘C, ⌘V, ⌘A) also serves the text fields of the Settings and
/// welcome windows, which may be open with no document.
fn native_edit(_: &Session) -> bool {
    true
}
fn has_inline_suggestion(s: &Session) -> bool {
    s.inline_suggestion().is_some()
}

fn has_pdf(s: &Session) -> bool {
    s.pdf.path.is_some()
}
fn is_running(s: &Session) -> bool {
    s.typeset.is_running()
}
fn can_typeset(s: &Session) -> bool {
    s.root_path().is_some()
}
/// The engine is chosen by hand only with Auto-Sense off; otherwise Auto-Sense picks it.
fn engine_choosable(s: &Session) -> bool {
    !s.settings.auto_sense
}
/// The engine menu ticks the engine in use while Auto-Sense picks it, the chosen one otherwise.
fn engine_checked(s: &Session, e: Engine) -> bool {
    if s.settings.auto_sense {
        s.effective_engine() == e
    } else {
        s.settings.engine == e
    }
}
fn has_back(s: &Session) -> bool {
    s.history_pos > 0
}
fn has_forward(s: &Session) -> bool {
    s.history_pos + 1 < s.history.len()
}
fn many_docs(s: &Session) -> bool {
    s.docs.len() > 1
}

fn param_str<'a>(p: &'a Value, key: &str) -> Result<&'a str, CommandError> {
    p.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::Params(format!("missing string `{key}`")))
}

fn param_u32(p: &Value, key: &str) -> Result<u32, CommandError> {
    p.get(key)
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| CommandError::Params(format!("missing number `{key}`")))
}

fn param_f32(p: &Value, key: &str) -> Result<f32, CommandError> {
    p.get(key)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
        .ok_or_else(|| CommandError::Params(format!("missing number `{key}`")))
}

macro_rules! cmd {
    ($id:literal, $label:literal, $menu:ident, $group:literal, $sc:expr, $enabled:expr, $run:expr) => {
        CommandSpec {
            id: $id,
            label: $label,
            menu: Menu::$menu,
            group: $group,
            shortcut: $sc,
            enabled: $enabled,
            checked: None,
            run: $run,
            params: "",
        }
    };
    ($id:literal, $label:literal, $menu:ident, $group:literal, $sc:expr, $enabled:expr, $run:expr, checked = $checked:expr) => {
        CommandSpec {
            id: $id,
            label: $label,
            menu: Menu::$menu,
            group: $group,
            shortcut: $sc,
            enabled: $enabled,
            checked: Some($checked),
            run: $run,
            params: "",
        }
    };
    ($id:literal, $label:literal, $menu:ident, $group:literal, $sc:expr, $enabled:expr, $run:expr, params = $params:literal) => {
        CommandSpec {
            id: $id,
            label: $label,
            menu: Menu::$menu,
            group: $group,
            shortcut: $sc,
            enabled: $enabled,
            checked: None,
            run: $run,
            params: $params,
        }
    };
}

fn snippet(s: &mut Session, text: &str) -> Result<(), CommandError> {
    s.edit_active(|t, sel| edit::insert_snippet(t, sel, text))
}

fn set_engine(s: &mut Session, e: Engine) -> Result<(), CommandError> {
    s.settings.engine = e;
    s.set_status(format!("Typesetter: {}", e.label()));
    Ok(())
}

fn set_mode(s: &mut Session, mode: TypesetMode) -> Result<(), CommandError> {
    s.settings.set_mode(mode);
    s.live_paused = false;
    if mode == TypesetMode::Manual {
        s.typeset.live_due = None;
        s.typeset.queued_live = false;
    }
    Ok(())
}

fn zoom_step(s: &mut Session, factor: f32) {
    let current = match s.view.zoom {
        Zoom::Fixed(z) => z,
        _ => s.pdf_fit_scale.unwrap_or(1.0),
    };
    let steps = [
        0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0,
        6.4,
    ];
    let next = if factor > 1.0 {
        steps
            .iter()
            .copied()
            .find(|z| *z > current + 0.001)
            .unwrap_or(6.4)
    } else {
        steps
            .iter()
            .rev()
            .copied()
            .find(|z| *z < current - 0.001)
            .unwrap_or(0.25)
    };
    s.view.zoom = Zoom::Fixed(next);
}

/// Every command, in menu order.
pub fn registry() -> &'static [CommandSpec] {
    REGISTRY
}

static REGISTRY: &[CommandSpec] = &[
    // ---------------------------------------------------------------- TexCraft (app) menu
    cmd!(
        "app.about",
        "About TexCraft",
        App,
        0,
        None,
        always,
        |s, _| {
            s.effect(Effect::About);
            Ok(())
        }
    ),
    cmd!(
        "app.checkForUpdates",
        "Check for Updates…",
        App,
        0,
        None,
        always,
        |s, _| {
            s.check_for_updates(true);
            Ok(())
        }
    ),
    cmd!(
        "app.downloadUpdate",
        "Download Update",
        None,
        0,
        None,
        |s| matches!(s.update.status, crate::UpdateStatus::Available(_)),
        |s, _| s.download_update()
    ),
    cmd!(
        "app.installUpdate",
        "Install Update and Relaunch",
        None,
        0,
        None,
        |s| matches!(s.update.status, crate::UpdateStatus::Ready(_)),
        |s, _| s.install_update(true)
    ),
    cmd!(
        "app.skipUpdate",
        "Skip This Version",
        None,
        0,
        None,
        |s| matches!(
            s.update.status,
            crate::UpdateStatus::Available(_)
                | crate::UpdateStatus::Ready(_)
                | crate::UpdateStatus::Downloading { .. }
        ),
        |s, _| {
            s.skip_update();
            Ok(())
        }
    ),
    cmd!(
        "app.cancelUpdate",
        "Cancel Update",
        None,
        0,
        None,
        |s| s.update.is_busy(),
        |s, _| {
            s.cancel_update();
            Ok(())
        }
    ),
    cmd!(
        "app.settings",
        "Settings…",
        App,
        1,
        Some(Shortcut::cmd(",")),
        always,
        |s, _| {
            s.effect(Effect::Settings);
            Ok(())
        }
    ),
    cmd!(
        "app.quit",
        "Quit TexCraft",
        App,
        9,
        Some(Shortcut::cmd("Q")),
        always,
        |s, _| {
            s.effect(Effect::Quit);
            Ok(())
        }
    ),
    // ---------------------------------------------------------------- File
    cmd!(
        "file.new",
        "New Document",
        File,
        0,
        Some(Shortcut::cmd("N")),
        always,
        |s, _| {
            let blank = texcraft_project::templates::template("blank").map_or("", |t| t.source);
            s.new_untitled(blank.to_string());
            Ok(())
        }
    ),
    cmd!(
        "file.open",
        "Open…",
        File,
        0,
        Some(Shortcut::cmd("O")),
        always,
        |s, _| {
            s.effect(Effect::OpenDialog);
            Ok(())
        }
    ),
    cmd!(
        "file.openFolder",
        "Open Project Folder…",
        File,
        0,
        None,
        always,
        |s, _| {
            s.effect(Effect::OpenFolderDialog);
            Ok(())
        }
    ),
    cmd!(
        "file.newFromTemplate",
        "New from Template…",
        File,
        1,
        None,
        always,
        |s, p| {
            match p.get("id").and_then(Value::as_str) {
                Some(id) => {
                    let t = texcraft_project::templates::template(id)
                        .ok_or_else(|| CommandError::Params(format!("no template `{id}`")))?;
                    s.new_untitled(t.source.to_string());
                }
                None => s.effect(Effect::TemplateGallery),
            }
            Ok(())
        },
        params = "{id?: template id} — without id, shows the template gallery"
    ),
    cmd!(
        "file.save",
        "Save",
        File,
        2,
        Some(Shortcut::cmd("S")),
        has_doc,
        |s, _| {
            let i = s.active;
            s.save(i)?;
            s.refresh_files();
            Ok(())
        }
    ),
    cmd!(
        "file.saveAs",
        "Save As…",
        File,
        2,
        Some(Shortcut::cmd_shift("S")),
        has_doc,
        |s, p| {
            let i = s.active;
            match p.get("path").and_then(Value::as_str) {
                Some(path) => s.save_as(i, PathBuf::from(path)),
                None => {
                    s.effect(Effect::SaveAsDialog { doc: i });
                    Ok(())
                }
            }
        },
        params = "{path?: string}"
    ),
    cmd!(
        "file.saveAll",
        "Save All",
        File,
        2,
        Some(Shortcut::cmd_alt("S")),
        has_doc,
        |s, _| {
            for i in 0..s.docs.len() {
                if s.docs
                    .get(i)
                    .is_some_and(|d| d.path.is_some() && d.is_dirty())
                {
                    s.save(i)?;
                }
            }
            Ok(())
        }
    ),
    cmd!(
        "file.revert",
        "Revert to Saved",
        File,
        2,
        None,
        |s| s.doc().is_some_and(|d| d.path.is_some() && d.is_dirty()),
        |s, _| {
            let i = s.active;
            let path = s
                .doc()
                .and_then(|d| d.path.clone())
                .ok_or_else(|| CommandError::Failed("not saved yet".into()))?;
            let text =
                std::fs::read_to_string(&path).map_err(|e| CommandError::Failed(e.to_string()))?;
            if let Some(d) = s.docs.get_mut(i) {
                d.text = text;
                d.revision += 1;
                d.saved_revision = d.revision;
                d.selection = crate::Selection::default();
            }
            s.effect(Effect::SelectionChanged { doc: i });
            Ok(())
        }
    ),
    cmd!(
        "file.closeTab",
        "Close Tab",
        File,
        3,
        Some(Shortcut::cmd("W")),
        has_doc,
        |s, p| {
            let i = p
                .get("index")
                .and_then(Value::as_u64)
                .map_or(s.active, |v| v as usize);
            let dirty = s
                .docs
                .get(i)
                .is_some_and(|d| d.is_dirty() && !d.is_pristine());
            if dirty && !p.get("force").and_then(Value::as_bool).unwrap_or(false) {
                s.effect(Effect::ConfirmClose { doc: i });
            } else {
                s.close(i);
            }
            Ok(())
        },
        params = "{index?: number, force?: bool}"
    ),
    cmd!(
        "file.closeWindow",
        "Close Window",
        File,
        3,
        Some(Shortcut::cmd_shift("W")),
        always,
        |s, _| {
            s.effect(Effect::Quit);
            Ok(())
        }
    ),
    cmd!(
        "file.jumpTo",
        "Jump to…",
        File,
        4,
        Some(Shortcut::cmd_shift("O")),
        always,
        |s, _| {
            s.view.sidebar = true;
            s.view.sidebar_tab = SidebarTab::Search;
            s.view.search_mode = SearchMode::Tag;
            s.effect(Effect::FocusSidebarSearch);
            Ok(())
        }
    ),
    cmd!(
        "file.revealPdf",
        "Reveal PDF in Finder",
        File,
        5,
        Some(Shortcut::cmd_shift("R")),
        has_pdf,
        |s, _| {
            if let Some(p) = s.pdf_file() {
                s.effect(Effect::Reveal(p));
            }
            Ok(())
        }
    ),
    cmd!(
        "file.exportPdf",
        "Export PDF…",
        File,
        5,
        Some(Shortcut::cmd_shift("E")),
        has_pdf,
        |s, p| {
            match p.get("path").and_then(Value::as_str) {
                Some(dest) => {
                    let src = s
                        .pdf
                        .path
                        .clone()
                        .ok_or_else(|| CommandError::Failed("no PDF yet".into()))?;
                    std::fs::copy(&src, dest)
                        .map_err(|e| CommandError::Failed(format!("export failed: {e}")))?;
                    s.set_status(format!("Exported {dest}"));
                }
                None => s.effect(Effect::ExportPdfDialog),
            }
            Ok(())
        },
        params = "{path?: string}"
    ),
    cmd!(
        "file.openPath",
        "Open Path",
        None,
        0,
        None,
        always,
        |s, p| {
            let path = param_str(p, "path")?;
            s.open_path(std::path::Path::new(path)).map(|_| ())
        },
        params = "{path: string} — a file or a project folder"
    ),
    // ------------------------------------------------- Files sidebar context menu
    cmd!(
        "files.insert",
        "Insert in the Editor",
        None,
        0,
        None,
        has_doc,
        |s, p| s.insert_file_reference(Path::new(param_str(p, "path")?)),
        params = "{path: string} — inserts \\input / \\includegraphics / \\bibliography for it"
    ),
    cmd!(
        "files.reveal",
        "Reveal in Finder",
        None,
        0,
        None,
        always,
        |s, p| {
            s.effect(Effect::Reveal(PathBuf::from(param_str(p, "path")?)));
            Ok(())
        },
        params = "{path: string}"
    ),
    cmd!(
        "files.copyPath",
        "Copy Full Path",
        None,
        0,
        None,
        always,
        |s, p| {
            s.effect(Effect::CopyText(param_str(p, "path")?.to_string()));
            Ok(())
        },
        params = "{path: string}"
    ),
    cmd!(
        "files.rename",
        "Rename",
        None,
        0,
        None,
        always,
        |s, p| {
            let to = s.rename_file(Path::new(param_str(p, "path")?), param_str(p, "name")?)?;
            s.set_status(format!("Renamed to {}", to.display()));
            Ok(())
        },
        params = "{path: string, name: string} — a new name in the same folder"
    ),
    cmd!(
        "files.duplicate",
        "Duplicate",
        None,
        0,
        None,
        always,
        |s, p| {
            let to = s.duplicate_file(Path::new(param_str(p, "path")?))?;
            s.set_status(format!("Created {}", to.display()));
            Ok(())
        },
        params = "{path: string}"
    ),
    cmd!(
        "files.newFolder",
        "Create a new folder",
        None,
        0,
        None,
        always,
        |s, p| s.create_folder(Path::new(param_str(p, "dir")?)).map(|_| ()),
        params = "{dir: string} — creates “untitled folder” there and starts renaming it"
    ),
    cmd!(
        "files.newFile",
        "Create a new file",
        None,
        0,
        None,
        always,
        |s, p| s.create_file(Path::new(param_str(p, "dir")?)).map(|_| ()),
        params = "{dir: string} — creates an empty “untitled.tex” there and starts renaming it"
    ),
    cmd!(
        "files.trash",
        "Move to Trash",
        None,
        0,
        None,
        always,
        |s, p| s.trash_file(Path::new(param_str(p, "path")?)),
        params = "{path: string} — a file or folder; open documents in it close"
    ),
    // ------------------------------------------------- AI Assistant (sidebar tab)
    cmd!(
        "ai.send",
        "Send",
        None,
        0,
        None,
        always,
        |s, p| {
            let text = p
                .get("text")
                .and_then(Value::as_str)
                .map_or_else(|| s.ai.draft.clone(), str::to_string);
            s.ai_send(&text)?;
            s.ai.draft.clear();
            Ok(())
        },
        params = "{text?: string} — default: the question being written"
    ),
    cmd!(
        "ai.stop",
        "Stop Answering",
        None,
        0,
        None,
        |s| s.ai.is_busy(),
        |s, _| {
            s.ai_stop();
            Ok(())
        }
    ),
    cmd!(
        "ai.attachImage",
        "Attach Image…",
        None,
        0,
        None,
        always,
        |s, p| {
            let path = std::path::PathBuf::from(param_str(p, "path")?);
            s.ai_attach_file(&path)
        },
        params = "{path: string} — a PNG, JPEG, GIF or WebP file, for a model with Vision"
    ),
    cmd!(
        "ai.removeImage",
        "Remove Image",
        None,
        0,
        None,
        always,
        |s, p| {
            let i = param_u32(p, "index")? as usize;
            if i < s.ai.draft_images.len() {
                s.ai.draft_images.remove(i);
                Ok(())
            } else {
                Err(CommandError::Params(format!("no image {i}")))
            }
        },
        params = "{index: number} — an image attached to the question being written"
    ),
    cmd!(
        "ai.clear",
        "New Conversation",
        None,
        0,
        None,
        always,
        |s, _| {
            s.ai_stop();
            s.ai.messages.clear();
            Ok(())
        }
    ),
    cmd!(
        "ai.insert",
        "Insert in the Editor",
        None,
        0,
        None,
        has_doc,
        |s, p| s.ai_insert(param_u32(p, "index")? as usize),
        params = "{index: number} — an answer's LaTeX (its first latex code block) at the caret"
    ),
    cmd!(
        "ai.copy",
        "Copy Answer",
        None,
        0,
        None,
        always,
        |s, p| {
            let i = param_u32(p, "index")? as usize;
            let text =
                s.ai.messages
                    .get(i)
                    .map(|m| m.text.clone())
                    .ok_or_else(|| CommandError::Failed("no such message".into()))?;
            s.effect(Effect::CopyText(text));
            Ok(())
        },
        params = "{index: number}"
    ),
    cmd!(
        "ai.selectModel",
        "Choose Model",
        None,
        0,
        None,
        always,
        |s, p| {
            s.ai.config.provider = param_str(p, "provider")?.to_string();
            s.ai.config.model = param_str(p, "model")?.to_string();
            s.save_ai_config()
        },
        params = "{provider: string, model: string} — a provider id and one of its models"
    ),
    cmd!(
        "ai.toggleDocument",
        "Include the Open Document",
        None,
        0,
        None,
        always,
        |s, _| {
            s.ai.config.include_document = !s.ai.config.include_document;
            s.save_ai_config()
        },
        checked = |s| s.ai.config.include_document
    ),
    cmd!(
        "ai.inline.toggle",
        "AI Suggestions While Typing",
        Edit,
        6,
        None,
        always,
        |s, _| {
            s.ai.config.inline.enabled = !s.ai.config.inline.enabled;
            if !s.ai.config.inline.enabled {
                s.inline_dismiss();
            }
            s.save_ai_config()
        },
        checked = |s| s.ai.config.inline.enabled
    ),
    cmd!(
        "ai.inline.trigger",
        "Suggest Completion",
        Edit,
        6,
        Some(Shortcut::alt("\\")),
        has_doc,
        |s, _| s.inline_trigger()
    ),
    cmd!(
        "ai.inline.accept",
        "Accept Suggestion",
        None,
        0,
        None,
        has_inline_suggestion,
        |s, _| s.inline_accept(crate::inline::Accept::All)
    ),
    cmd!(
        "ai.inline.acceptSentence",
        "Accept Next Sentence of Suggestion",
        None,
        0,
        None,
        has_inline_suggestion,
        |s, _| s.inline_accept(crate::inline::Accept::Sentence)
    ),
    cmd!(
        "ai.inline.acceptWord",
        "Accept Next Word of Suggestion",
        None,
        0,
        None,
        has_inline_suggestion,
        |s, _| s.inline_accept(crate::inline::Accept::Word)
    ),
    cmd!(
        "ai.inline.dismiss",
        "Dismiss Suggestion",
        None,
        0,
        None,
        always,
        |s, _| {
            s.inline_dismiss();
            Ok(())
        }
    ),
    cmd!(
        "ai.inline.settings",
        "AI Suggestion Settings…",
        None,
        0,
        None,
        always,
        |s, _| {
            s.effect(Effect::InlineSettings);
            Ok(())
        }
    ),
    cmd!(
        "ai.settings",
        "AI Settings…",
        None,
        0,
        None,
        always,
        |s, _| {
            s.effect(Effect::AiSettings);
            Ok(())
        }
    ),
    // ------------------------------------------------- Source Control (sidebar Git tab)
    cmd!("git.refresh", "Refresh", None, 0, None, always, |s, _| s
        .git_run(texcraft_git::Op::Refresh)),
    cmd!(
        "git.init",
        "Initialize Repository",
        None,
        0,
        None,
        always,
        |s, _| s.git_run(texcraft_git::Op::Init)
    ),
    cmd!(
        "git.stage",
        "Stage Changes",
        None,
        0,
        None,
        always,
        |s, p| s.git_stage(param_str(p, "path")?),
        params = "{path: string} — relative to the repository, or absolute"
    ),
    cmd!(
        "git.unstage",
        "Unstage Changes",
        None,
        0,
        None,
        always,
        |s, p| s.git_unstage(param_str(p, "path")?),
        params = "{path: string}"
    ),
    cmd!(
        "git.stageAll",
        "Stage All Changes",
        None,
        0,
        None,
        always,
        |s, _| s.git_run(texcraft_git::Op::StageAll)
    ),
    cmd!(
        "git.unstageAll",
        "Unstage All Changes",
        None,
        0,
        None,
        always,
        |s, _| s.git_run(texcraft_git::Op::UnstageAll)
    ),
    cmd!(
        "git.discard",
        "Discard Changes",
        None,
        0,
        None,
        always,
        |s, p| s.git_discard(param_str(p, "path")?),
        params = "{path: string} — tracked files return to the staged version, untracked ones go to the Trash"
    ),
    cmd!(
        "git.commit",
        "Commit",
        None,
        0,
        None,
        always,
        |s, p| s.git_commit(p.get("message").and_then(Value::as_str)),
        params = "{message?: string} — default: the message box; stages everything when nothing is staged"
    ),
    cmd!("git.pull", "Pull", None, 0, None, always, |s, _| s
        .git_run(texcraft_git::Op::Pull)),
    cmd!("git.push", "Push", None, 0, None, always, |s, _| s
        .git_run(texcraft_git::Op::Push)),
    cmd!("git.fetch", "Fetch", None, 0, None, always, |s, _| s
        .git_run(texcraft_git::Op::Fetch)),
    cmd!(
        "git.diff",
        "Open Changes",
        None,
        0,
        None,
        always,
        |s, p| s.git_diff(
            param_str(p, "path")?,
            p.get("staged").and_then(Value::as_bool).unwrap_or(false)
        ),
        params = "{path: string, staged?: bool}"
    ),
    // ---------------------------------------------------------------- Edit
    cmd!(
        "edit.undo",
        "Undo",
        Edit,
        0,
        Some(Shortcut::cmd("Z")),
        native_edit,
        |s, _| {
            s.effect(Effect::Native(NativeEdit::Undo));
            Ok(())
        }
    ),
    cmd!(
        "edit.redo",
        "Redo",
        Edit,
        0,
        Some(Shortcut::cmd_shift("Z")),
        native_edit,
        |s, _| {
            s.effect(Effect::Native(NativeEdit::Redo));
            Ok(())
        }
    ),
    cmd!(
        "edit.cut",
        "Cut",
        Edit,
        1,
        Some(Shortcut::cmd("X")),
        native_edit,
        |s, _| {
            s.effect(Effect::Native(NativeEdit::Cut));
            Ok(())
        }
    ),
    cmd!(
        "edit.copy",
        "Copy",
        Edit,
        1,
        Some(Shortcut::cmd("C")),
        native_edit,
        |s, _| {
            s.effect(Effect::Native(NativeEdit::Copy));
            Ok(())
        }
    ),
    cmd!(
        "edit.paste",
        "Paste",
        Edit,
        1,
        Some(Shortcut::cmd("V")),
        native_edit,
        |s, _| {
            s.effect(Effect::Native(NativeEdit::Paste));
            Ok(())
        }
    ),
    cmd!(
        "edit.selectAll",
        "Select All",
        Edit,
        1,
        Some(Shortcut::cmd("A")),
        native_edit,
        |s, _| {
            s.effect(Effect::Native(NativeEdit::SelectAll));
            Ok(())
        }
    ),
    cmd!(
        "edit.find",
        "Find…",
        Edit,
        2,
        Some(Shortcut::cmd("F")),
        has_doc,
        |s, _| {
            s.effect(Effect::Find { replace: false });
            Ok(())
        }
    ),
    cmd!(
        "edit.findReplace",
        "Find and Replace…",
        Edit,
        2,
        Some(Shortcut::cmd_alt("F")),
        has_doc,
        |s, _| {
            s.effect(Effect::Find { replace: true });
            Ok(())
        }
    ),
    cmd!(
        "edit.findNext",
        "Find Next",
        Edit,
        2,
        Some(Shortcut::cmd("G")),
        has_doc,
        |s, _| {
            s.effect(Effect::FindNext { backwards: false });
            Ok(())
        }
    ),
    cmd!(
        "edit.findPrevious",
        "Find Previous",
        Edit,
        2,
        Some(Shortcut::cmd_shift("G")),
        has_doc,
        |s, _| {
            s.effect(Effect::FindNext { backwards: true });
            Ok(())
        }
    ),
    cmd!(
        "edit.globalSearch",
        "Global Search",
        Edit,
        2,
        Some(Shortcut::cmd_shift("F")),
        always,
        |s, _| {
            s.view.sidebar = true;
            s.view.sidebar_tab = SidebarTab::Search;
            s.view.search_mode = SearchMode::Text;
            s.effect(Effect::FocusSidebarSearch);
            Ok(())
        }
    ),
    cmd!(
        "edit.gotoLine",
        "Go to Line…",
        Edit,
        2,
        Some(Shortcut::cmd("L")),
        has_doc,
        |s, p| {
            match p.get("line").and_then(Value::as_u64) {
                Some(line) => {
                    let line = u32::try_from(line).unwrap_or(u32::MAX);
                    let i = s.active;
                    if let Some(d) = s.docs.get_mut(i) {
                        d.selection = crate::Selection::caret(edit::char_of_line(&d.text, line));
                    }
                    s.effect(Effect::RevealLine { doc: i, line });
                }
                None => s.effect(Effect::GotoLineDialog),
            }
            Ok(())
        },
        params = "{line?: number}"
    ),
    cmd!(
        "edit.toggleComment",
        "Comment / Uncomment",
        Edit,
        3,
        Some(Shortcut::cmd("/")),
        has_doc,
        |s, _| s.edit_active(edit::toggle_comment)
    ),
    cmd!(
        "edit.indent",
        "Indent Block",
        Edit,
        3,
        Some(Shortcut::cmd("]")),
        has_doc,
        |s, _| s.edit_active(edit::indent)
    ),
    cmd!(
        "edit.outdent",
        "Unindent Block",
        Edit,
        3,
        Some(Shortcut::cmd("[")),
        has_doc,
        |s, _| s.edit_active(edit::outdent)
    ),
    cmd!(
        "edit.toggleSpellcheck",
        "Check Spelling While Typing",
        Edit,
        3,
        Some(Shortcut::cmd_alt(";")),
        always,
        |s, _| {
            s.view.spellcheck = !s.view.spellcheck;
            Ok(())
        },
        checked = |s| s.view.spellcheck
    ),
    cmd!(
        "edit.autocomplete",
        "Autocomplete",
        Edit,
        3,
        None,
        has_doc,
        |s, _| {
            s.effect(Effect::Autocomplete);
            Ok(())
        }
    ),
    cmd!(
        "edit.insertText",
        "Insert Text",
        None,
        0,
        None,
        has_doc,
        |s, p| {
            let text = param_str(p, "text")?.to_string();
            s.edit_active(|t, sel| edit::replace_selection(t, sel, &text))
        },
        params = "{text: string} — replaces the selection"
    ),
    cmd!(
        "edit.setText",
        "Set Text",
        None,
        0,
        None,
        has_doc,
        |s, p| {
            let text = param_str(p, "text")?.to_string();
            s.edit_active(move |_, _| (text, crate::Selection::default()))
        },
        params = "{text: string} — replaces the whole active document"
    ),
    cmd!(
        "edit.select",
        "Select Range",
        None,
        0,
        None,
        has_doc,
        |s, p| {
            let anchor = param_u32(p, "anchor")? as usize;
            let cursor = p
                .get("cursor")
                .and_then(Value::as_u64)
                .map_or(anchor, |v| v as usize);
            let i = s.active;
            if let Some(d) = s.docs.get_mut(i) {
                let n = d.text.chars().count();
                d.selection = crate::Selection {
                    anchor: anchor.min(n),
                    cursor: cursor.min(n),
                };
            }
            s.effect(Effect::SelectionChanged { doc: i });
            Ok(())
        },
        params = "{anchor: number, cursor?: number} (char indices)"
    ),
    // ---------------------------------------------------------------- Snippets (⌃⇧ + letter)
    cmd!(
        "snippet.preamble",
        "Preamble",
        Snippets,
        0,
        Some(Shortcut::ctrl_shift("P")),
        has_doc,
        |s, _| snippet(
            s,
            "\\documentclass[11pt]{article}\n\\usepackage{amsmath,amssymb}\n\\usepackage{graphicx}\n\n\\begin{document}\n$0\n\\end{document}\n"
        )
    ),
    cmd!(
        "snippet.bold",
        "Bold",
        Snippets,
        1,
        Some(Shortcut::ctrl_shift("B")),
        has_doc,
        |s, _| s.edit_active(|t, sel| edit::wrap(t, sel, "\\textbf{", "}"))
    ),
    cmd!(
        "snippet.italic",
        "Italic",
        Snippets,
        1,
        Some(Shortcut::ctrl_shift("I")),
        has_doc,
        |s, _| s.edit_active(|t, sel| edit::wrap(t, sel, "\\textit{", "}"))
    ),
    cmd!(
        "snippet.typewriter",
        "Typewriter",
        Snippets,
        1,
        Some(Shortcut::ctrl_shift("T")),
        has_doc,
        |s, _| s.edit_active(|t, sel| edit::wrap(t, sel, "\\texttt{", "}"))
    ),
    cmd!(
        "snippet.underline",
        "Underline",
        Snippets,
        1,
        Some(Shortcut::ctrl_shift("U")),
        has_doc,
        |s, _| s.edit_active(|t, sel| edit::wrap(t, sel, "\\underline{", "}"))
    ),
    cmd!(
        "snippet.emphasis",
        "Emphasis",
        Snippets,
        1,
        Some(Shortcut::ctrl_shift("G")),
        has_doc,
        |s, _| s.edit_active(|t, sel| edit::wrap(t, sel, "\\emph{", "}"))
    ),
    cmd!(
        "snippet.colourRed",
        "Colour Red",
        Snippets,
        2,
        Some(Shortcut::ctrl_shift("R")),
        has_doc,
        |s, _| s.edit_active(|t, sel| edit::wrap(t, sel, "\\textcolor{red}{", "}"))
    ),
    cmd!(
        "snippet.table",
        "Floating Table",
        Snippets,
        3,
        Some(Shortcut::ctrl_shift("L")),
        has_doc,
        |s, _| snippet(
            s,
            "\\begin{table}[htbp]\n  \\centering\n  \\begin{tabular}{$1lll}\n    $SEL$0\n  \\end{tabular}\n  \\caption{}\n  \\label{tab:}\n\\end{table}"
        )
    ),
    cmd!(
        "snippet.figure",
        "Figure",
        Snippets,
        3,
        Some(Shortcut::ctrl_shift("F")),
        has_doc,
        |s, _| snippet(
            s,
            "\\begin{figure}[htbp]\n  \\centering\n  \\includegraphics[width=0.8\\linewidth]{$1}\n  \\caption{$SEL}\n  \\label{fig:}\n\\end{figure}"
        )
    ),
    cmd!(
        "snippet.displaymath",
        "Displaymath",
        Snippets,
        4,
        Some(Shortcut::ctrl_shift("Y")),
        has_doc,
        |s, _| snippet(s, "\\[\n  $SEL$0\n\\]")
    ),
    cmd!(
        "snippet.itemize",
        "Itemize",
        Snippets,
        4,
        Some(Shortcut::ctrl_shift("Z")),
        has_doc,
        |s, _| snippet(s, "\\begin{itemize}\n  \\item $SEL$0\n\\end{itemize}")
    ),
    cmd!(
        "snippet.enumerate",
        "Enumerate",
        Snippets,
        4,
        Some(Shortcut::ctrl_shift("M")),
        has_doc,
        |s, _| snippet(s, "\\begin{enumerate}\n  \\item $SEL$0\n\\end{enumerate}")
    ),
    cmd!(
        "snippet.quotation",
        "Quotation",
        Snippets,
        4,
        Some(Shortcut::ctrl_shift("Q")),
        has_doc,
        |s, _| snippet(s, "\\begin{quotation}\n  $SEL$0\n\\end{quotation}")
    ),
    cmd!(
        "snippet.equation",
        "Equation",
        Snippets,
        4,
        Some(Shortcut::ctrl_shift("E")),
        has_doc,
        |s, _| snippet(
            s,
            "\\begin{equation}\n  $SEL$0\n  \\label{eq:}\n\\end{equation}"
        )
    ),
    cmd!(
        "snippet.verbatim",
        "Verbatim",
        Snippets,
        4,
        Some(Shortcut::ctrl_shift("V")),
        has_doc,
        |s, _| snippet(s, "\\begin{verbatim}\n$SEL$0\n\\end{verbatim}")
    ),
    cmd!(
        "snippet.tabular",
        "Tabular",
        Snippets,
        4,
        Some(Shortcut::ctrl_shift("A")),
        has_doc,
        |s, _| snippet(s, "\\begin{tabular}{$1ll}\n  $SEL$0\n\\end{tabular}")
    ),
    cmd!(
        "snippet.description",
        "Description",
        Snippets,
        4,
        Some(Shortcut::ctrl_shift("D")),
        has_doc,
        |s, _| snippet(
            s,
            "\\begin{description}\n  \\item[$1] $SEL$0\n\\end{description}"
        )
    ),
    cmd!(
        "snippet.matrix",
        "Matrix",
        Snippets,
        4,
        Some(Shortcut::ctrl_shift("X")),
        has_doc,
        |s, _| snippet(
            s,
            "\\begin{pmatrix}\n  $1 & \\\\\n  $SEL$0 & \n\\end{pmatrix}"
        )
    ),
    // ---------------------------------------------------------------- Typeset
    cmd!(
        "typeset.run",
        "Typeset",
        Typeset,
        0,
        Some(Shortcut::cmd("T")),
        can_typeset,
        |s, _| {
            // A full typeset writes the PDF in every mode; pressing again while one runs aborts
            // it (a live-preview pass in progress gives way instead).
            if s.typeset.is_full_running() {
                s.stop_typeset();
                return Ok(());
            }
            s.typeset()
        }
    ),
    cmd!(
        "typeset.stop",
        "Abort Typesetting",
        Typeset,
        0,
        Some(Shortcut::cmd(".")),
        is_running,
        |s, _| {
            s.stop_typeset();
            Ok(())
        }
    ),
    cmd!(
        "typeset.mode.live",
        "Live Preview",
        Typeset,
        1,
        None,
        always,
        |s, _| set_mode(s, TypesetMode::Live),
        checked = |s| s.settings.mode() == TypesetMode::Live
    ),
    cmd!(
        "typeset.mode.auto",
        "Auto-Typeset When Typing Pauses",
        Typeset,
        1,
        None,
        always,
        |s, _| set_mode(s, TypesetMode::Auto),
        checked = |s| s.settings.mode() == TypesetMode::Auto
    ),
    cmd!(
        "typeset.mode.manual",
        "Manual Typesetting",
        Typeset,
        1,
        None,
        always,
        |s, _| set_mode(s, TypesetMode::Manual),
        checked = |s| s.settings.mode() == TypesetMode::Manual
    ),
    cmd!(
        "typeset.cycleMode",
        "Next Typesetting Mode",
        None,
        0,
        None,
        always,
        |s, _| {
            let next = s.settings.mode().next();
            set_mode(s, next)
        }
    ),
    cmd!(
        "typeset.toggleLive",
        "Toggle Auto-Typeset",
        Typeset,
        1,
        Some(Shortcut::cmd_shift("L")),
        always,
        |s, _| {
            // Manual ⇄ the previous automatic mode (LIVE or AUTO).
            s.settings.live_typeset = !s.settings.live_typeset;
            s.live_paused = false;
            Ok(())
        }
    ),
    cmd!(
        "typeset.pauseLive",
        "Pause Auto-Typeset",
        Typeset,
        1,
        None,
        |s| s.settings.live_typeset,
        |s, _| {
            s.live_paused = !s.live_paused;
            if s.live_paused {
                s.typeset.live_due = None;
            }
            Ok(())
        },
        checked = |s| s.live_paused
    ),
    cmd!(
        "typeset.clean",
        "Remove Intermediate Files",
        Typeset,
        2,
        Some(Shortcut::cmd("K")),
        can_typeset,
        |s, _| {
            let root = s
                .root_path()
                .ok_or_else(|| CommandError::Failed("nothing to clean".into()))?;
            let dir = texcraft_project::build_dir(&root);
            let removed = std::fs::read_dir(&dir)
                .map(|rd| {
                    rd.filter_map(Result::ok)
                        .map(|e| e.path())
                        .filter(|p| p.is_file() && texcraft_project::is_aux_file(p))
                        .filter(|p| std::fs::remove_file(p).is_ok())
                        .count()
                })
                .unwrap_or(0);
            s.set_status(format!(
                "Removed {removed} intermediate file{}",
                if removed == 1 { "" } else { "s" }
            ));
            Ok(())
        }
    ),
    cmd!(
        "typeset.engine.pdflatex",
        "pdfLaTeX",
        Typeset,
        3,
        None,
        engine_choosable,
        |s, _| set_engine(s, Engine::PdfLatex),
        checked = |s| engine_checked(s, Engine::PdfLatex)
    ),
    cmd!(
        "typeset.engine.xelatex",
        "XeLaTeX",
        Typeset,
        3,
        None,
        engine_choosable,
        |s, _| set_engine(s, Engine::XeLatex),
        checked = |s| engine_checked(s, Engine::XeLatex)
    ),
    cmd!(
        "typeset.engine.lualatex",
        "LuaLaTeX",
        Typeset,
        3,
        None,
        engine_choosable,
        |s, _| set_engine(s, Engine::LuaLatex),
        checked = |s| engine_checked(s, Engine::LuaLatex)
    ),
    cmd!(
        "typeset.engine.tectonic",
        "Tectonic",
        Typeset,
        3,
        None,
        engine_choosable,
        |s, _| set_engine(s, Engine::Tectonic),
        checked = |s| engine_checked(s, Engine::Tectonic)
    ),
    cmd!(
        "typeset.configuration",
        "Typeset Configuration…",
        Typeset,
        4,
        None,
        always,
        |s, _| {
            s.effect(Effect::TypesetConfiguration);
            Ok(())
        }
    ),
    cmd!(
        "typeset.syncPdf",
        "Show Position in PDF",
        Typeset,
        5,
        Some(Shortcut::cmd("D")),
        has_pdf,
        |s, _| {
            s.forward_sync(true);
            Ok(())
        }
    ),
    cmd!(
        "typeset.inverseSync",
        "Show Source for PDF Point",
        None,
        0,
        None,
        has_pdf,
        |s, p| {
            let page = param_u32(p, "page")?;
            let x = param_f32(p, "x")?;
            let y = param_f32(p, "y")?;
            s.inverse_sync(page, x, y)
        },
        params = "{page: number (1-based), x: points, y: points from top}"
    ),
    cmd!(
        "typeset.revealIssue",
        "Reveal Issue",
        None,
        0,
        None,
        always,
        |s, p| {
            let idx = param_u32(p, "index")? as usize;
            let issue = s
                .typeset
                .issues
                .get(idx)
                .cloned()
                .ok_or_else(|| CommandError::Params("no such issue".into()))?;
            match (issue.file, issue.line) {
                (Some(f), Some(l)) => s.jump(&f, l),
                _ => {
                    s.view.results_mode = ResultsMode::Log;
                    Ok(())
                }
            }
        },
        params = "{index: number}"
    ),
    // ---------------------------------------------------------------- Workspace
    cmd!(
        "view.toggleSidebar",
        "Show Sidebar",
        Workspace,
        0,
        Some(Shortcut::cmd("1")),
        always,
        |s, _| {
            s.view.sidebar = !s.view.sidebar;
            Ok(())
        },
        checked = |s| s.view.sidebar
    ),
    cmd!(
        "view.editorOnly",
        "Editor Only",
        Workspace,
        0,
        Some(Shortcut::cmd("2")),
        always,
        |s, _| {
            s.view.layout = Layout::EditorOnly;
            Ok(())
        },
        checked = |s| s.view.layout == Layout::EditorOnly
    ),
    cmd!(
        "view.split",
        "Editor and PDF",
        Workspace,
        0,
        Some(Shortcut::cmd("3")),
        always,
        |s, _| {
            s.view.layout = Layout::Split;
            Ok(())
        },
        checked = |s| s.view.layout == Layout::Split
    ),
    cmd!(
        "view.pdfOnly",
        "PDF Only",
        Workspace,
        0,
        Some(Shortcut::cmd("4")),
        always,
        |s, _| {
            s.view.layout = Layout::PdfOnly;
            Ok(())
        },
        checked = |s| s.view.layout == Layout::PdfOnly
    ),
    cmd!(
        "view.contents",
        "Contents",
        Workspace,
        1,
        Some(Shortcut::cmd_alt("1")),
        always,
        |s, _| {
            s.show_tab(SidebarTab::Contents);
            Ok(())
        },
        checked = |s| s.view.sidebar && s.view.sidebar_tab == SidebarTab::Contents
    ),
    cmd!(
        "view.todos",
        "TODOs",
        Workspace,
        1,
        Some(Shortcut::cmd_alt("2")),
        always,
        |s, _| {
            s.show_tab(SidebarTab::Todos);
            Ok(())
        },
        checked = |s| s.view.sidebar && s.view.sidebar_tab == SidebarTab::Todos
    ),
    cmd!(
        "view.files",
        "File Browser",
        Workspace,
        1,
        Some(Shortcut::cmd_alt("3")),
        always,
        |s, _| {
            s.show_tab(SidebarTab::Files);
            Ok(())
        },
        checked = |s| s.view.sidebar && s.view.sidebar_tab == SidebarTab::Files
    ),
    cmd!(
        "view.results",
        "Typeset Results",
        Workspace,
        1,
        Some(Shortcut::cmd_alt("4")),
        always,
        |s, _| {
            s.show_tab(SidebarTab::Results);
            Ok(())
        },
        checked = |s| s.view.sidebar && s.view.sidebar_tab == SidebarTab::Results
    ),
    cmd!(
        "view.search",
        "Search",
        Workspace,
        1,
        Some(Shortcut::cmd_alt("5")),
        always,
        |s, _| {
            s.show_tab(SidebarTab::Search);
            Ok(())
        },
        checked = |s| s.view.sidebar && s.view.sidebar_tab == SidebarTab::Search
    ),
    cmd!(
        "view.git",
        "Source Control",
        Workspace,
        1,
        Some(Shortcut::cmd_alt("6")),
        always,
        |s, _| {
            s.show_tab(SidebarTab::Git);
            Ok(())
        },
        checked = |s| s.view.sidebar && s.view.sidebar_tab == SidebarTab::Git
    ),
    cmd!(
        "view.assistant",
        "AI Assistant",
        Workspace,
        1,
        Some(Shortcut::cmd_alt("7")),
        always,
        |s, _| {
            s.show_tab(SidebarTab::Assistant);
            Ok(())
        },
        checked = |s| s.view.sidebar && s.view.sidebar_tab == SidebarTab::Assistant
    ),
    cmd!(
        "view.structure",
        "Structure",
        None,
        0,
        None,
        always,
        |s, _| {
            s.view.contents_mode = ContentsMode::Structure;
            Ok(())
        }
    ),
    cmd!(
        "view.subfiles",
        "Subfiles",
        None,
        0,
        None,
        always,
        |s, _| {
            s.view.contents_mode = ContentsMode::Subfiles;
            Ok(())
        }
    ),
    cmd!("view.issues", "Issues", None, 0, None, always, |s, _| {
        s.view.results_mode = ResultsMode::Issues;
        Ok(())
    }),
    cmd!("view.log", "Log", None, 0, None, always, |s, _| {
        s.view.results_mode = ResultsMode::Log;
        Ok(())
    }),
    cmd!(
        "pdf.zoomIn",
        "Zoom In",
        Workspace,
        2,
        Some(Shortcut::cmd("=")),
        always,
        |s, _| {
            zoom_step(s, 1.25);
            Ok(())
        }
    ),
    cmd!(
        "pdf.zoomOut",
        "Zoom Out",
        Workspace,
        2,
        Some(Shortcut::cmd("-")),
        always,
        |s, _| {
            zoom_step(s, 0.8);
            Ok(())
        }
    ),
    cmd!(
        "pdf.actualSize",
        "Actual Size (Fit Width)",
        Workspace,
        2,
        Some(Shortcut::cmd("0")),
        always,
        |s, _| {
            s.view.zoom = Zoom::Fixed(1.0);
            Ok(())
        },
        checked = |s| s.view.zoom == Zoom::Fixed(1.0)
    ),
    cmd!(
        "pdf.fitWidth",
        "Zoom to Fit Width",
        Workspace,
        2,
        None,
        always,
        |s, _| {
            s.view.zoom = Zoom::FitWidth;
            Ok(())
        },
        checked = |s| s.view.zoom == Zoom::FitWidth
    ),
    cmd!(
        "pdf.fitPage",
        "Zoom to Fit Page",
        Workspace,
        2,
        None,
        always,
        |s, _| {
            s.view.zoom = Zoom::FitPage;
            Ok(())
        },
        checked = |s| s.view.zoom == Zoom::FitPage
    ),
    cmd!(
        "pdf.setZoom",
        "Set Zoom",
        None,
        0,
        None,
        always,
        |s, p| {
            let z = param_f32(p, "zoom")?;
            s.view.zoom = Zoom::Fixed(z.clamp(0.1, 8.0));
            Ok(())
        },
        params = "{zoom: number (1.0 = 100%)}"
    ),
    cmd!(
        "pdf.singleContinuous",
        "Single Page Continuous",
        Workspace,
        3,
        None,
        always,
        |s, _| {
            s.view.page_mode = PageMode::SingleContinuous;
            Ok(())
        },
        checked = |s| s.view.page_mode == PageMode::SingleContinuous
    ),
    cmd!(
        "pdf.twoContinuous",
        "Two Pages Continuous",
        Workspace,
        3,
        None,
        always,
        |s, _| {
            s.view.page_mode = PageMode::TwoContinuous;
            Ok(())
        },
        checked = |s| s.view.page_mode == PageMode::TwoContinuous
    ),
    cmd!(
        "pdf.singlePage",
        "Single Page",
        Workspace,
        3,
        None,
        always,
        |s, _| {
            s.view.page_mode = PageMode::Single;
            Ok(())
        },
        checked = |s| s.view.page_mode == PageMode::Single
    ),
    cmd!(
        "pdf.gradeDark",
        "Tint PDF in Dark Mode",
        Workspace,
        3,
        None,
        always,
        |s, _| {
            s.view.grading = if s.view.grading == Grading::Entire {
                Grading::None
            } else {
                Grading::Entire
            };
            Ok(())
        },
        checked = |s| s.view.grading == Grading::Entire
    ),
    cmd!(
        "pdf.scrollLock",
        "Scroll Lock",
        Workspace,
        3,
        None,
        always,
        |s, _| {
            s.view.scroll_lock = !s.view.scroll_lock;
            Ok(())
        },
        checked = |s| s.view.scroll_lock
    ),
    cmd!(
        "pdf.openExternally",
        "Open PDF in Default Viewer",
        Workspace,
        3,
        None,
        has_pdf,
        |s, _| {
            if let Some(p) = s.pdf_file() {
                s.effect(Effect::OpenExternally(p));
            }
            Ok(())
        }
    ),
    cmd!(
        "pdf.gotoPage",
        "Go to Page",
        None,
        0,
        None,
        has_pdf,
        |s, p| {
            let page = param_u32(p, "page")?;
            s.effect(Effect::GotoPage(page.max(1)));
            Ok(())
        },
        params = "{page: number (1-based)}"
    ),
    cmd!(
        "nav.back",
        "Back",
        Workspace,
        4,
        Some(Shortcut::ctrl_cmd("Left")),
        has_back,
        |s, _| s.navigate(-1)
    ),
    cmd!(
        "nav.forward",
        "Forward",
        Workspace,
        4,
        Some(Shortcut::ctrl_cmd("Right")),
        has_forward,
        |s, _| s.navigate(1)
    ),
    cmd!(
        "view.fontBigger",
        "Bigger Font",
        Workspace,
        5,
        Some(Shortcut::cmd_alt("=")),
        always,
        |s, _| {
            s.view.editor_font_size = (s.view.editor_font_size + 1.0).min(36.0);
            Ok(())
        }
    ),
    cmd!(
        "view.fontSmaller",
        "Smaller Font",
        Workspace,
        5,
        Some(Shortcut::cmd_alt("-")),
        always,
        |s, _| {
            s.view.editor_font_size = (s.view.editor_font_size - 1.0).max(8.0);
            Ok(())
        }
    ),
    cmd!(
        "view.wrapLines",
        "Wrap Lines",
        Workspace,
        5,
        None,
        always,
        |s, _| {
            s.view.wrap_lines = !s.view.wrap_lines;
            Ok(())
        },
        checked = |s| s.view.wrap_lines
    ),
    cmd!(
        "view.lineNumbers",
        "Line Numbers",
        Workspace,
        5,
        None,
        always,
        |s, _| {
            s.view.line_numbers = !s.view.line_numbers;
            Ok(())
        },
        checked = |s| s.view.line_numbers
    ),
    cmd!(
        "view.theme.system",
        "Appearance: System",
        Workspace,
        6,
        None,
        always,
        |s, _| {
            s.view.theme = ThemeChoice::System;
            Ok(())
        },
        checked = |s| s.view.theme == ThemeChoice::System
    ),
    cmd!(
        "view.theme.light",
        "Appearance: Light",
        Workspace,
        6,
        None,
        always,
        |s, _| {
            s.view.theme = ThemeChoice::Light;
            Ok(())
        },
        checked = |s| s.view.theme == ThemeChoice::Light
    ),
    cmd!(
        "view.theme.dark",
        "Appearance: Dark",
        Workspace,
        6,
        None,
        always,
        |s, _| {
            s.view.theme = ThemeChoice::Dark;
            Ok(())
        },
        checked = |s| s.view.theme == ThemeChoice::Dark
    ),
    // ---------------------------------------------------------------- Window
    cmd!(
        "window.nextTab",
        "Show Next Tab",
        Window,
        0,
        Some(Shortcut::ctrl("Tab")),
        many_docs,
        |s, _| {
            s.active = (s.active + 1) % s.docs.len().max(1);
            Ok(())
        }
    ),
    cmd!(
        "window.previousTab",
        "Show Previous Tab",
        Window,
        0,
        Some(Shortcut {
            key: "Tab",
            cmd: false,
            shift: true,
            alt: false,
            ctrl: true
        }),
        many_docs,
        |s, _| {
            let n = s.docs.len().max(1);
            s.active = (s.active + n - 1) % n;
            Ok(())
        }
    ),
    cmd!(
        "window.selectTab",
        "Select Tab",
        None,
        0,
        None,
        has_doc,
        |s, p| {
            let i = param_u32(p, "index")? as usize;
            if i >= s.docs.len() {
                return Err(CommandError::Params("no such tab".into()));
            }
            s.active = i;
            Ok(())
        },
        params = "{index: number}"
    ),
    cmd!(
        "window.welcome",
        "Welcome Window",
        Window,
        1,
        Some(Shortcut::ctrl_cmd("T")),
        always,
        |s, _| {
            s.effect(Effect::Welcome);
            Ok(())
        }
    ),
    // ---------------------------------------------------------------- Help
    cmd!(
        "help.shortcuts",
        "Keyboard Shortcuts",
        Help,
        0,
        Some(Shortcut::cmd_shift("/")),
        always,
        |s, _| {
            s.effect(Effect::Shortcuts);
            Ok(())
        }
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_and_shortcuts_are_unique() {
        let mut ids = HashSet::new();
        let mut keys = HashSet::new();
        for c in registry() {
            assert!(ids.insert(c.id), "duplicate id {}", c.id);
            if let Some(sc) = c.shortcut {
                assert!(
                    keys.insert(sc),
                    "shortcut {} used twice ({})",
                    sc.display(),
                    c.id
                );
            }
            assert!(!c.label.is_empty());
        }
    }

    #[test]
    fn texifier_shortcuts() {
        let sc = |id: &str| {
            registry()
                .iter()
                .find(|c| c.id == id)
                .and_then(|c| c.shortcut)
                .map(Shortcut::display)
        };
        assert_eq!(sc("typeset.run").as_deref(), Some("⌘T"));
        assert_eq!(sc("typeset.toggleLive").as_deref(), Some("⇧⌘L"));
        assert_eq!(sc("typeset.clean").as_deref(), Some("⌘K"));
        assert_eq!(sc("typeset.stop").as_deref(), Some("⌘."));
        assert_eq!(sc("view.toggleSidebar").as_deref(), Some("⌘1"));
        assert_eq!(sc("view.pdfOnly").as_deref(), Some("⌘4"));
        assert_eq!(sc("edit.toggleComment").as_deref(), Some("⌘/"));
        assert_eq!(sc("snippet.bold").as_deref(), Some("⌃⇧B"));
        assert_eq!(sc("file.jumpTo").as_deref(), Some("⇧⌘O"));
    }
}
