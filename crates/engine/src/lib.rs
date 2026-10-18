//! The TexCraft engine: the [`Session`] (open documents, project, typesetting, PDF and SyncTeX
//! state, view state) and the command registry. Every user action is a command with an id, so
//! the menu bar, toolbar, keyboard, CLI and the agent control channel all drive the same code.
//!
//! The engine never touches the GUI. It reports what the UI should do through [`Effect`]s.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use texcraft_i18n::{Lang, tr, trf};
use texcraft_log::{Issue, Severity};
use texcraft_project::{Engine, FileKind, FileNode, Settings, Typesetter, find_root};
use texcraft_synctex::{PdfRect, SyncTex};
use texcraft_typeset::{Job, JobEvent, LiveRequest, Mirror, Outcome, Request, Toolchain};

pub mod ai;
pub mod commands;
pub mod edit;
pub mod files;
pub mod git;
pub mod index;
pub mod inline;
pub mod sync;
pub mod update;

pub use commands::{CommandSpec, Shortcut, registry};
pub use edit::Selection;
pub use index::ProjectIndex;
pub use update::{CURRENT_VERSION, UpdateState, UpdateStatus};

/// An open text document.
#[derive(Debug, Clone)]
pub struct Document {
    /// Where it lives on disk; `None` for an untitled document.
    pub path: Option<PathBuf>,
    /// For untitled documents: the scratch file typesetting writes to.
    pub scratch: Option<PathBuf>,
    /// Current text.
    pub text: String,
    /// Increases on every change.
    pub revision: u64,
    /// `revision` when last saved or loaded.
    pub saved_revision: u64,
    /// Selection in char indices, mirrored from the editor every frame.
    pub selection: Selection,
}

impl Document {
    /// A document with `text`, not yet saved anywhere.
    pub fn untitled(text: String, scratch: PathBuf) -> Self {
        Self {
            path: None,
            scratch: Some(scratch),
            text,
            revision: 1,
            saved_revision: 0,
            selection: Selection::default(),
        }
    }

    /// Has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    /// An untitled document nobody has typed into yet.
    pub fn is_pristine(&self) -> bool {
        self.path.is_none() && self.revision == 1
    }

    /// File name, or "Untitled" (in the interface language).
    pub fn display_name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_name()).map_or_else(
            || tr("Untitled").to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    /// The path typesetting uses: the real file, else the scratch file.
    pub fn typeset_path(&self) -> Option<&Path> {
        self.path.as_deref().or(self.scratch.as_deref())
    }

    /// Replaces the text (an edit made by a command), bumping the revision.
    pub fn replace_text(&mut self, text: String, selection: Selection) {
        if text != self.text {
            self.text = text;
            self.revision += 1;
        }
        self.selection = selection;
    }

    /// 1-based line of the caret.
    pub fn caret_line(&self) -> u32 {
        edit::line_of_char(&self.text, self.selection.cursor)
    }

    /// Whether it is a `.bib` file.
    pub fn is_bib(&self) -> bool {
        self.path
            .as_ref()
            .is_some_and(|p| FileKind::of(p) == FileKind::Bib)
    }
}

/// Sidebar modes (the icon tabs at the top of the sidebar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum SidebarTab {
    /// Document structure and subfiles, with file/word counts.
    #[default]
    Contents,
    /// TODO comments and `\todo{}` notes.
    Todos,
    /// Source Control: the project's Git repository (where the target has its cloud service).
    #[serde(alias = "Connect")]
    Git,
    /// The project folder.
    Files,
    /// Issues and the raw log of the last typeset.
    Results,
    /// Project-wide text search and jump-to (tag) search.
    Search,
    /// Chat with an AI model about the document.
    Assistant,
}

impl SidebarTab {
    /// All tabs in display order (the AI Assistant before Search). The ⌘⌥1–5 shortcuts keep the
    /// target's numbering (Contents, Todos, Files, Results, Search); Source Control is ⌘⌥6 and
    /// the Assistant ⌘⌥7.
    pub const ALL: [SidebarTab; 7] = [
        Self::Contents,
        Self::Todos,
        Self::Git,
        Self::Files,
        Self::Results,
        Self::Assistant,
        Self::Search,
    ];

    /// Tooltip / accessibility name (in the interface language).
    pub fn label(self) -> &'static str {
        match self {
            Self::Contents => tr("Contents"),
            Self::Todos => tr("TODOs"),
            Self::Git => tr("Source Control"),
            Self::Files => tr("File Browser"),
            Self::Results => tr("Typeset Results"),
            Self::Search => tr("Search"),
            Self::Assistant => tr("AI Assistant"),
        }
    }
}

/// What the Contents tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ContentsMode {
    /// Sections, figures, tables, labels.
    #[default]
    Structure,
    /// Files by inclusion.
    Subfiles,
}

/// What the Typeset Results tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ResultsMode {
    /// Parsed issues.
    #[default]
    Issues,
    /// Raw console output.
    Log,
}

/// What the Search tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum SearchMode {
    /// Full-text search (and replace) across the project.
    #[default]
    Text,
    /// Jump to sections, labels, citations, files.
    Tag,
}

/// Editor / PDF arrangement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Layout {
    /// Editor only.
    EditorOnly,
    /// Editor and PDF side by side.
    #[default]
    Split,
    /// PDF only.
    PdfOnly,
}

/// How the PDF is scaled.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Zoom {
    /// Page width fills the pane.
    FitWidth,
    /// The whole page fits.
    FitPage,
    /// Fixed scale (1.0 = 100 %).
    Fixed(f32),
}

/// How PDF pages are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum PageMode {
    /// One column, scrolling.
    #[default]
    SingleContinuous,
    /// Two columns, scrolling.
    TwoContinuous,
    /// One page at a time.
    Single,
}

/// Dark-mode re-tinting of the PDF (display only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Grading {
    /// Re-tint the whole page in dark mode.
    #[default]
    Entire,
    /// Show the PDF as it is.
    None,
}

/// Editor colour scheme (light appearance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EditorTheme {
    /// Cream paper, olive comments.
    #[default]
    Classic,
    /// White paper, green comments.
    Plain,
}

/// Light, dark or follow the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    /// Follow the operating system.
    #[default]
    System,
    /// Light.
    Light,
    /// Dark.
    Dark,
}

/// The interface language setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LanguageChoice {
    /// The first of the system's preferred languages TexCraft has (macOS: Language & Region),
    /// else English.
    #[default]
    System,
    /// English.
    English,
    /// Simplified Chinese (简体中文).
    #[serde(rename = "zh-Hans")]
    ChineseSimplified,
}

impl LanguageChoice {
    /// Every choice, in menu order.
    pub const ALL: [LanguageChoice; 3] = [Self::System, Self::English, Self::ChineseSimplified];

    /// The language this choice means, given the system's preferred languages (most preferred
    /// first, BCP 47 or POSIX tags).
    pub fn resolve<S: AsRef<str>>(self, system: &[S]) -> Lang {
        match self {
            Self::System => Lang::from_preferred(system),
            Self::English => Lang::En,
            Self::ChineseSimplified => Lang::ZhHans,
        }
    }

    /// The choice for a `--lang` value: `system`, or a language tag (`en`, `zh-Hans`, `zh-CN`…).
    pub fn from_arg(arg: &str) -> Option<LanguageChoice> {
        if arg.eq_ignore_ascii_case("system") {
            return Some(Self::System);
        }
        Lang::from_tag(arg).map(|l| match l {
            Lang::En => Self::English,
            Lang::ZhHans => Self::ChineseSimplified,
        })
    }
}

/// View state: which panes are shown and how. Persisted between launches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    /// Sidebar visible.
    pub sidebar: bool,
    /// Sidebar mode.
    pub sidebar_tab: SidebarTab,
    /// Contents tab mode.
    pub contents_mode: ContentsMode,
    /// Results tab mode.
    pub results_mode: ResultsMode,
    /// Search tab mode.
    pub search_mode: SearchMode,
    /// Editor / PDF arrangement.
    pub layout: Layout,
    /// PDF zoom.
    pub zoom: Zoom,
    /// PDF page layout.
    pub page_mode: PageMode,
    /// Dark-mode PDF tinting.
    pub grading: Grading,
    /// Keep the PDF scrolled to the editor's position (scroll lock).
    pub scroll_lock: bool,
    /// Theme.
    pub theme: ThemeChoice,
    /// Interface language.
    pub language: LanguageChoice,
    /// Editor colour scheme.
    pub editor_theme: EditorTheme,
    /// Editor font size in points.
    pub editor_font_size: f32,
    /// Soft-wrap long lines.
    pub wrap_lines: bool,
    /// Wrapped continuation lines are indented.
    pub hanging_indent: bool,
    /// Hanging indent in characters (0 = the tab width).
    pub hanging_chars: u8,
    /// What Tab inserts: 0 = a tab character, else that many spaces.
    pub tab_spaces: u8,
    /// Show the autocomplete popup while typing commands.
    pub auto_complete: bool,
    /// Return copies the previous line's indentation.
    pub auto_indent: bool,
    /// Return after `\begin{env}` inserts `\end{env}`.
    pub auto_end: bool,
    /// A plain click in the PDF jumps to the source (else ⌘-click).
    pub single_click_sync: bool,
    /// Show the welcome window when TexCraft starts without a document.
    pub show_welcome: bool,
    /// Underline misspelled words.
    pub spellcheck: bool,
    /// Show line numbers.
    pub line_numbers: bool,
    /// Scroll the PDF to the caret after each typeset.
    pub sync_after_typeset: bool,
    /// Show the issues after a typeset with errors.
    pub show_issues_after_error: bool,
    /// Look for new versions automatically.
    pub auto_check_updates: bool,
    /// Download new versions automatically (installed when TexCraft quits).
    pub auto_download_updates: bool,
    /// A version the user chose to skip.
    pub skipped_update: Option<String>,
    /// When the last update check finished (Unix seconds).
    pub last_update_check: u64,
}

impl Default for View {
    fn default() -> Self {
        Self {
            sidebar: true,
            sidebar_tab: SidebarTab::Contents,
            contents_mode: ContentsMode::Structure,
            results_mode: ResultsMode::Issues,
            search_mode: SearchMode::Text,
            layout: Layout::Split,
            zoom: Zoom::FitWidth,
            page_mode: PageMode::SingleContinuous,
            grading: Grading::Entire,
            scroll_lock: false,
            theme: ThemeChoice::System,
            language: LanguageChoice::System,
            editor_theme: EditorTheme::Classic,
            editor_font_size: 16.0,
            wrap_lines: true,
            hanging_indent: true,
            hanging_chars: 0,
            tab_spaces: 0,
            auto_complete: true,
            auto_indent: true,
            auto_end: true,
            single_click_sync: false,
            show_welcome: true,
            spellcheck: true,
            line_numbers: true,
            sync_after_typeset: true,
            show_issues_after_error: true,
            auto_check_updates: true,
            auto_download_updates: true,
            skipped_update: None,
            last_update_check: 0,
        }
    }
}

/// Edit actions the UI's text widget performs itself (menu items mirror them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeEdit {
    /// Undo.
    Undo,
    /// Redo.
    Redo,
    /// Cut.
    Cut,
    /// Copy.
    Copy,
    /// Paste.
    Paste,
    /// Select all.
    SelectAll,
}

/// A point on a PDF page: 1-based page, points from the page's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PdfPoint {
    /// 1-based page.
    pub page: u32,
    /// Points from the left edge.
    pub x: f32,
    /// Points from the top edge.
    pub y: f32,
}

/// How a PDF text selection grows: by character (drag), word (double-click) or line
/// (triple-click).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextUnit {
    /// Characters.
    #[default]
    Char,
    /// Whole words.
    Word,
    /// Whole lines.
    Line,
}

/// Something the UI should do after a command or a tick.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Move the caret to 1-based `line` of document `doc` and scroll it into view.
    RevealLine {
        /// Document index.
        doc: usize,
        /// 1-based line.
        line: u32,
    },
    /// Push `Document::selection` of `doc` into the editor widget (after a command edit).
    SelectionChanged {
        /// Document index.
        doc: usize,
    },
    /// Scroll the PDF to these rectangles and flash them (forward search).
    ShowInPdf {
        /// Target rectangles (first one is scrolled to).
        rects: Vec<PdfRect>,
        /// Flash a highlight (false: just make visible).
        flash: bool,
    },
    /// The PDF file changed; reload it.
    PdfChanged,
    /// Ask for a file to open.
    OpenDialog,
    /// Ask for a folder to open as a project.
    OpenFolderDialog,
    /// Ask where to save document `doc`.
    SaveAsDialog {
        /// Document index.
        doc: usize,
    },
    /// Ask where to export the PDF.
    ExportPdfDialog,
    /// Show the template gallery.
    TemplateGallery,
    /// Ask for a line number.
    GotoLineDialog,
    /// Show or focus the find bar (`replace`: with the replace field).
    Find {
        /// Show the replace field.
        replace: bool,
    },
    /// Find next / previous match of the current search.
    FindNext {
        /// Search backwards.
        backwards: bool,
    },
    /// Perform an edit action in the text widget.
    Native(NativeEdit),
    /// Confirm closing a document with unsaved changes.
    ConfirmClose {
        /// Document index.
        doc: usize,
    },
    /// Reveal a path in the file manager.
    Reveal(PathBuf),
    /// Open a file with its default application.
    OpenExternally(PathBuf),
    /// Show a message.
    Alert {
        /// Title.
        title: String,
        /// Body.
        message: String,
    },
    /// Show the About window.
    About,
    /// Focus the sidebar search field.
    FocusSidebarSearch,
    /// Open the autocomplete popup.
    Autocomplete,
    /// Show the typeset configuration sheet.
    TypesetConfiguration,
    /// Scroll the PDF to a 1-based page.
    GotoPage(u32),
    /// Show the welcome (organiser) window.
    Welcome,
    /// Show the keyboard-shortcuts window.
    Shortcuts,
    /// Open the settings window.
    Settings,
    /// Show the update window.
    UpdateWindow,
    /// Put text on the clipboard.
    CopyText(String),
    /// Briefly highlight characters `start..end` of document `doc` (where inverse search
    /// landed); an empty range highlights the caret's row.
    FlashSource {
        /// Document index.
        doc: usize,
        /// First character.
        start: usize,
        /// End (exclusive).
        end: usize,
    },
    /// Select text in the PDF from one point to another; `None` clears the selection.
    SelectPdfText(Option<(PdfPoint, PdfPoint, TextUnit)>),
    /// Show the diff in `Session::git.diff`.
    GitDiff,
    /// Open Settings at the AI pane.
    AiSettings,
    /// Open the inline AI suggestion settings (the editor header's popover).
    InlineSettings,
    /// Start renaming a file in the Files list (a file or folder just created).
    BeginRename(PathBuf),
    /// Quit the app.
    Quit,
}

/// Typesetting state.
#[derive(Default)]
pub struct TypesetState {
    /// The running job.
    pub job: Option<Job>,
    /// The last finished run.
    pub last: Option<Outcome>,
    /// Issues of the last run.
    pub issues: Vec<Issue>,
    /// When a live typeset is due.
    pub live_due: Option<Instant>,
    /// Typeset again as soon as the current run ends.
    pub queued: bool,
    /// Run a live pass as soon as the current run ends.
    pub queued_live: bool,
    /// The running job is a live-preview pass (a full typeset started meanwhile replaces it).
    pub live_job: bool,
    /// Duration of the last live pass.
    pub last_live: Option<Duration>,
    /// When the earliest edit not yet handed to a pass was made.
    pub pending_edit_at: Option<Instant>,
    /// The earliest edit the running live pass covers.
    pub job_edit_at: Option<Instant>,
    /// Keystroke-to-pixels time of the last live update (measured by the PDF view).
    pub last_latency: Option<Duration>,
    /// Completed runs.
    pub runs: u64,
    /// Console output of the current/last run.
    pub console: String,
    /// The root file of the current/last run.
    pub root: Option<PathBuf>,
    /// When the last run finished.
    pub finished_at: Option<std::time::SystemTime>,
}

impl TypesetState {
    /// A run is in progress.
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    /// A full typeset (not a live-preview pass) is in progress: the toolbar gears turn and
    /// ⌘T aborts it.
    pub fn is_full_running(&self) -> bool {
        self.job.is_some() && !self.live_job
    }
}

/// The PDF being previewed.
#[derive(Default)]
pub struct PdfState {
    /// The file.
    pub path: Option<PathBuf>,
    /// Increases every time the file is replaced.
    pub generation: u64,
    /// SyncTeX data for it.
    pub synctex: Option<SyncTex>,
    /// Set when the PDF comes from a live pass over mirrored buffers.
    pub mirror: Option<Mirror>,
    /// The edit this PDF reflects (for measuring keystroke-to-pixels latency).
    pub edit_at: Option<Instant>,
    /// The text selected in the PDF viewer (kept up to date by the viewer; `pdf.copySelection`
    /// copies it).
    pub selected_text: String,
}

/// One TexCraft window's worth of state.
pub struct Session {
    /// Open documents.
    pub docs: Vec<Document>,
    /// Index of the document in the editor.
    pub active: usize,
    /// The project folder (file list root).
    pub project_dir: Option<PathBuf>,
    /// The project's file tree (refreshed on open/save).
    pub file_tree: Vec<FileNode>,
    /// Typesetting settings.
    pub settings: Settings,
    /// View state.
    pub view: View,
    /// The installed TeX programs.
    pub toolchain: Toolchain,
    /// Typesetting.
    pub typeset: TypesetState,
    /// Software updates.
    pub update: UpdateState,
    /// Source Control (the Git tab).
    pub git: git::GitState,
    /// The AI Assistant.
    pub ai: ai::AiState,
    /// Inline AI suggestions in the editor.
    pub inline: inline::InlineState,
    /// PDF preview.
    pub pdf: PdfState,
    /// Recently opened files (most recent first).
    pub recent: Vec<PathBuf>,
    /// Pending effects for the UI.
    pub effects: VecDeque<Effect>,
    /// Transient status message.
    pub status: Option<(String, Instant)>,
    /// Auto-typeset is paused (PDF toolbar pause button).
    pub live_paused: bool,
    /// Jump history for Back / Forward: (file, 1-based line).
    pub history: Vec<(PathBuf, u32)>,
    /// Position in `history`.
    pub history_pos: usize,
    /// The project index (refreshed by [`Session::refresh_index`]).
    pub index: ProjectIndex,
    index_stamp: u64,
    /// The last typeset plan and the state it was computed from (the toolbar asks every frame).
    plan_cache: std::cell::Cell<Option<(u64, texcraft_project::Plan)>>,
    index_built: Option<Instant>,
    /// The scale the PDF view currently uses for Fit Width / Fit Page (reported by the UI), so
    /// zoom steps start from what the user sees.
    pub pdf_fit_scale: Option<f32>,
    /// Moves files to the Trash (the macOS app installs AppKit's; tests install a fake).
    pub trash: files::TrashFn,
    /// The system's preferred interface languages, most preferred first (the app reads them at
    /// launch; empty means English). The Language setting's System choice follows them.
    pub system_languages: Vec<String>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

/// Errors from commands. `Failed` carries a message for the user, already in the interface
/// language; the others are worded (translated) when displayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// No command with that id.
    Unknown(String),
    /// The command is disabled in the current state.
    Disabled(String),
    /// Bad or missing parameters (for agents and scripts: the detail stays English).
    Params(String),
    /// The command ran and failed.
    Failed(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(id) => f.write_str(&trf!("unknown command `{id}`", id = id)),
            Self::Disabled(id) => f.write_str(&trf!("`{id}` is not available right now", id = id)),
            Self::Params(detail) => f.write_str(&trf!("bad parameters: {detail}", detail = detail)),
            Self::Failed(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for CommandError {}

impl Session {
    /// A new session with one untitled document. `waker` is called from background threads when
    /// something changed (request a repaint).
    pub fn new(waker: Arc<dyn Fn() + Send + Sync>) -> Self {
        let mut s = Self {
            docs: Vec::new(),
            active: 0,
            project_dir: None,
            file_tree: Vec::new(),
            settings: Settings::default(),
            view: View::default(),
            toolchain: Toolchain::detect(),
            typeset: TypesetState::default(),
            update: UpdateState::default(),
            git: git::GitState::default(),
            ai: ai::AiState::default(),
            inline: inline::InlineState::default(),
            pdf: PdfState::default(),
            recent: Vec::new(),
            effects: VecDeque::new(),
            status: None,
            live_paused: false,
            history: Vec::new(),
            history_pos: 0,
            pdf_fit_scale: None,
            index: ProjectIndex::default(),
            index_stamp: 0,
            plan_cache: std::cell::Cell::new(None),
            index_built: None,
            trash: files::system_trash,
            system_languages: Vec::new(),
            waker,
        };
        s.new_untitled(texcraft_project::templates::TEMPLATES[1].source.to_string());
        s
    }

    /// Runs command `id` with JSON `params` (use `serde_json::Value::Null` for none).
    pub fn run(&mut self, id: &str, params: &serde_json::Value) -> Result<(), CommandError> {
        let spec = registry()
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| CommandError::Unknown(id.to_string()))?;
        if !(spec.enabled)(self) {
            return Err(CommandError::Disabled(id.to_string()));
        }
        (spec.run)(self, params)
    }

    /// Whether command `id` is enabled now.
    pub fn is_enabled(&self, id: &str) -> bool {
        registry().iter().any(|c| c.id == id && (c.enabled)(self))
    }

    /// Whether command `id` shows a check mark now.
    pub fn is_checked(&self, id: &str) -> bool {
        registry()
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.checked)
            .is_some_and(|f| f(self))
    }

    /// The active document.
    pub fn doc(&self) -> Option<&Document> {
        self.docs.get(self.active)
    }

    /// The active document, mutably.
    pub fn doc_mut(&mut self) -> Option<&mut Document> {
        self.docs.get_mut(self.active)
    }

    /// The interface language: the Language setting, with System resolved against
    /// [`Session::system_languages`]. The app applies it ([`texcraft_i18n::set_language`]).
    pub fn lang(&self) -> Lang {
        self.view.language.resolve(&self.system_languages)
    }

    /// Shows a transient status message.
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    /// Next effect for the UI.
    pub fn take_effect(&mut self) -> Option<Effect> {
        self.effects.pop_front()
    }

    pub(crate) fn effect(&mut self, e: Effect) {
        self.effects.push_back(e);
    }

    /// Adds a new untitled document with `text` and makes it active.
    pub fn new_untitled(&mut self, text: String) -> usize {
        // Numbered across the process, not the session: sessions side by side (tests) must not
        // typeset into one folder.
        static UNTITLED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = UNTITLED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let scratch = std::env::temp_dir()
            .join(format!("texcraft-{}-untitled-{n}", std::process::id()))
            .join("Untitled.tex");
        self.docs.push(Document::untitled(text, scratch));
        self.active = self.docs.len() - 1;
        self.schedule_live();
        self.active
    }

    /// Opens a file or folder. Returns the index of the document shown (if any).
    pub fn open_path(&mut self, path: &Path) -> Result<Option<usize>, CommandError> {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if path.is_dir() {
            self.set_project_dir(path.clone());
            // Open the main file: main.tex, else the first file with \documentclass.
            let mut candidates: Vec<PathBuf> = std::fs::read_dir(&path)
                .map(|rd| {
                    rd.filter_map(Result::ok)
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|e| e == "tex"))
                        .collect()
                })
                .unwrap_or_default();
            candidates.sort();
            let main = candidates
                .iter()
                .find(|p| p.file_name().is_some_and(|n| n == "main.tex"))
                .cloned()
                .or_else(|| {
                    candidates
                        .iter()
                        .find(|p| {
                            std::fs::read_to_string(p)
                                .is_ok_and(|t| texcraft_syntax::is_root_document(&t))
                        })
                        .cloned()
                })
                .or_else(|| candidates.first().cloned());
            return match main {
                Some(m) => self.open_file(&m).map(Some),
                None => Ok(None),
            };
        }
        if FileKind::of(&path) == FileKind::Pdf {
            self.show_pdf(path.clone());
            return Ok(None);
        }
        self.open_file(&path).map(Some)
    }

    fn open_file(&mut self, path: &Path) -> Result<usize, CommandError> {
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.path.as_deref() == Some(path))
        {
            self.active = i;
            return Ok(i);
        }
        if !FileKind::of(path).is_editable() && path.extension().is_some() {
            return Err(CommandError::Failed(trf!(
                "{path} is not a text file TexCraft can edit",
                path = path.display()
            )));
        }
        let bytes = std::fs::read(path).map_err(|e| {
            CommandError::Failed(trf!(
                "cannot open {path}: {e}",
                path = path.display(),
                e = e
            ))
        })?;
        let text = String::from_utf8(bytes)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
            .replace("\r\n", "\n");
        let doc = Document {
            path: Some(path.to_path_buf()),
            scratch: None,
            text,
            revision: 0,
            saved_revision: 0,
            selection: Selection::default(),
        };
        // Replace a pristine untitled document instead of piling up tabs.
        let replace = self
            .docs
            .get(self.active)
            .is_some_and(Document::is_pristine)
            && self.docs.len() == 1;
        if replace {
            self.docs[0] = doc;
            self.active = 0;
        } else {
            self.docs.push(doc);
            self.active = self.docs.len() - 1;
        }
        if self.project_dir.is_none()
            && let Some(dir) = path.parent()
        {
            self.set_project_dir(dir.to_path_buf());
        }
        self.remember_recent(path);
        // Show the matching PDF right away if one was typeset before.
        if let Some(root) = self.root_path() {
            let pdf = texcraft_project::output_pdf(&root);
            if pdf.is_file() && self.pdf.path.as_deref() != Some(pdf.as_path()) {
                self.show_pdf(pdf);
            }
        }
        if self.pdf.path.is_none() {
            self.schedule_live();
        }
        Ok(self.active)
    }

    /// Sets the project folder and reloads its file tree.
    pub fn set_project_dir(&mut self, dir: PathBuf) {
        self.file_tree = texcraft_project::file_tree(&dir, 6);
        self.project_dir = Some(dir);
    }

    /// Re-reads the project's file tree.
    pub fn refresh_files(&mut self) {
        if let Some(dir) = self.project_dir.clone() {
            self.file_tree = texcraft_project::file_tree(&dir, 6);
        }
        self.git.stale = true;
    }

    /// Saves document `idx` to its path (untitled documents ask for a path).
    pub fn save(&mut self, idx: usize) -> Result<(), CommandError> {
        let Some(doc) = self.docs.get_mut(idx) else {
            return Err(CommandError::Failed(tr("no such document").into()));
        };
        let Some(path) = doc.path.clone() else {
            self.effect(Effect::SaveAsDialog { doc: idx });
            return Ok(());
        };
        std::fs::write(&path, doc.text.as_bytes()).map_err(|e| {
            CommandError::Failed(trf!(
                "cannot save {path}: {e}",
                path = path.display(),
                e = e
            ))
        })?;
        doc.saved_revision = doc.revision;
        self.git.stale = true;
        Ok(())
    }

    /// Saves document `idx` to `path` (Save As).
    pub fn save_as(&mut self, idx: usize, path: PathBuf) -> Result<(), CommandError> {
        let Some(doc) = self.docs.get_mut(idx) else {
            return Err(CommandError::Failed(tr("no such document").into()));
        };
        std::fs::write(&path, doc.text.as_bytes()).map_err(|e| {
            CommandError::Failed(trf!(
                "cannot save {path}: {e}",
                path = path.display(),
                e = e
            ))
        })?;
        doc.path = Some(path.clone());
        doc.scratch = None;
        doc.saved_revision = doc.revision;
        if self.project_dir.is_none() {
            if let Some(dir) = path.parent() {
                self.set_project_dir(dir.to_path_buf());
            }
        } else {
            self.refresh_files();
        }
        self.remember_recent(&path);
        Ok(())
    }

    /// Puts `path`'s project at the top of the recent documents, by its entry point (the main
    /// file: see [`texcraft_project::project_entry`]), so a project is listed once.
    pub fn remember_recent(&mut self, path: &Path) {
        let entry = texcraft_project::project_entry(path, &|p| self.text_of(p));
        self.recent.retain(|p| p != &entry);
        self.recent.insert(0, entry);
        self.recent.truncate(12);
    }

    /// Sets the recent documents (from the saved state), each project by its entry point and
    /// once, in their order.
    pub fn set_recent(&mut self, list: Vec<PathBuf>) {
        let mut out: Vec<PathBuf> = Vec::new();
        for p in list {
            let entry = texcraft_project::project_entry(&p, &|q| std::fs::read_to_string(q).ok());
            if !out.contains(&entry) {
                out.push(entry);
            }
        }
        out.truncate(12);
        self.recent = out;
    }

    /// Closes document `idx` without asking.
    pub fn close(&mut self, idx: usize) {
        if idx < self.docs.len() {
            self.docs.remove(idx);
        }
        if self.docs.is_empty() {
            self.new_untitled(String::new());
        }
        self.active = self.active.min(self.docs.len().saturating_sub(1));
    }

    /// Copies a full typeset's PDF beside its root ([`texcraft_project::published_pdf`]) when it
    /// is new or the copy is missing. Written to a temporary file first and renamed, so a PDF
    /// viewer watching the file never reads half of it.
    fn publish_pdf(&mut self, pdf: &Path, fresh: bool) {
        let Some(root) = self.typeset.root.clone() else {
            return;
        };
        let dest = texcraft_project::published_pdf(&root);
        if dest == pdf || (!fresh && dest.is_file()) {
            return;
        }
        let name = dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let part = dest.with_file_name(format!(".{name}.part"));
        let copied = std::fs::copy(pdf, &part).and_then(|_| std::fs::rename(&part, &dest));
        if let Err(e) = copied {
            let _ = std::fs::remove_file(&part);
            self.set_status(trf!("Cannot write {name}: {e}", name = name, e = e));
            return;
        }
        self.refresh_files();
    }

    /// The PDF to reveal or open outside TexCraft: the one beside the root when a full typeset
    /// wrote it, else the one on screen.
    pub fn pdf_file(&self) -> Option<PathBuf> {
        self.root_path()
            .map(|r| texcraft_project::published_pdf(&r))
            .filter(|p| p.is_file())
            .or_else(|| self.pdf.path.clone())
    }

    /// The root file for the active document (typesetting target).
    pub fn root_path(&self) -> Option<PathBuf> {
        let doc = self.doc()?;
        let path = doc.typeset_path()?.to_path_buf();
        if doc.path.is_none() {
            return Some(path);
        }
        if doc.is_bib() {
            // A .bib file belongs to the last root that was typeset.
            return self.typeset.root.clone();
        }
        let docs = &self.docs;
        let read = |p: &Path| -> Option<String> {
            docs.iter()
                .find(|d| d.path.as_deref() == Some(p))
                .map(|d| d.text.clone())
                .or_else(|| std::fs::read_to_string(p).ok())
        };
        Some(find_root(&path, &doc.text, &read))
    }

    /// Text of `path`: the open buffer if there is one, else the file on disk.
    pub fn text_of(&self, path: &Path) -> Option<String> {
        self.docs
            .iter()
            .find(|d| d.typeset_path() == Some(path))
            .map(|d| d.text.clone())
            .or_else(|| std::fs::read_to_string(path).ok())
    }

    /// The engine the next typeset will use.
    pub fn effective_engine(&self) -> Engine {
        self.typeset_plan().engine
    }

    /// How the next typeset runs: engine, why, and whether TexCraft sets up CJK text (the
    /// root's text plus the other open files of the project are read).
    pub fn typeset_plan(&self) -> texcraft_project::Plan {
        use std::hash::{Hash, Hasher};
        let root = self.root_path();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        root.hash(&mut h);
        for d in &self.docs {
            d.typeset_path().hash(&mut h);
            d.revision.hash(&mut h);
            d.text.len().hash(&mut h);
        }
        // A root that is not open is read from disk: its modification time counts too.
        if let Some(r) = &root
            && !self
                .docs
                .iter()
                .any(|d| d.typeset_path() == Some(r.as_path()))
        {
            std::fs::metadata(r)
                .and_then(|m| m.modified())
                .ok()
                .hash(&mut h);
        }
        self.settings.engine.hash(&mut h);
        self.settings.auto_sense.hash(&mut h);
        let key = h.finish();
        if let Some((k, plan)) = self.plan_cache.get()
            && k == key
        {
            return plan;
        }
        let plan = self.compute_plan(root);
        self.plan_cache.set(Some((key, plan)));
        plan
    }

    fn compute_plan(&self, root: Option<PathBuf>) -> texcraft_project::Plan {
        let text = root
            .as_ref()
            .and_then(|r| self.text_of(r))
            .unwrap_or_default();
        let project_dir = root
            .as_ref()
            .and_then(|r| r.parent())
            .map(texcraft_project::normalize);
        let cjk_elsewhere = self.docs.iter().any(|d| {
            d.typeset_path().is_some_and(|p| {
                let p = texcraft_project::normalize(p);
                root.as_ref()
                    .is_none_or(|r| texcraft_project::normalize(r) != p)
                    && project_dir.as_ref().is_some_and(|dir| p.starts_with(dir))
            }) && texcraft_project::has_cjk(&d.text)
        });
        texcraft_project::plan(&text, cjk_elsewhere, &self.settings)
    }

    /// The CJK setup code for `plan`, when TexCraft adds it.
    fn pretex_for(&self, plan: &texcraft_project::Plan) -> Option<String> {
        if plan.add_cjk && plan.engine == Engine::XeLatex {
            self.toolchain.cjk_pretex()
        } else {
            None
        }
    }

    /// Notes that the user edited the active document (for live typesetting).
    pub fn note_edit(&mut self) {
        if let Some(d) = self.doc_mut() {
            d.revision += 1;
        }
        self.schedule_live();
        self.inline_after_edit();
    }

    fn schedule_live(&mut self) {
        if !self.settings.live_typeset || self.live_paused {
            return;
        }
        // Remember when the first edit not yet in the PDF happened (for the latency readout).
        self.typeset
            .pending_edit_at
            .get_or_insert_with(Instant::now);
        if self.settings.typesetter == Typesetter::External {
            // Full typesets start when typing pauses.
            let pause = u64::from(self.settings.live_delay_ms).max(800);
            self.typeset.live_due = Some(Instant::now() + Duration::from_millis(pause));
        } else if self.typeset.is_running() {
            // Exactly one more pass right after the current one: back-to-back while typing.
            self.typeset.queued_live = true;
        } else {
            self.typeset.live_due = Some(
                Instant::now() + Duration::from_millis(u64::from(self.settings.live_delay_ms)),
            );
        }
    }

    /// Starts typesetting the active document's root (queues if a run is in progress).
    pub fn typeset(&mut self) -> Result<(), CommandError> {
        self.typeset.live_due = None;
        if self.typeset.is_running() {
            self.typeset.queued = true;
            // A live pass gives way: the full typeset starts as soon as it has stopped.
            if self.typeset.live_job
                && let Some(job) = &self.typeset.job
            {
                job.cancel();
            }
            return Ok(());
        }
        let root = self
            .root_path()
            .ok_or_else(|| CommandError::Failed(tr("nothing to typeset").into()))?;
        // Write what the engine will read: open buffers of this project (saved docs) and the
        // scratch file of an untitled document.
        let save_all = self.settings.save_before_typeset;
        for i in 0..self.docs.len() {
            let Some(d) = self.docs.get(i) else { continue };
            if d.path.is_none() {
                if let Some(scratch) = d.scratch.clone()
                    && Some(scratch.as_path()) == Some(root.as_path())
                {
                    if let Some(dir) = scratch.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    std::fs::write(&scratch, d.text.as_bytes()).map_err(|e| {
                        CommandError::Failed(trf!(
                            "cannot write {path}: {e}",
                            path = scratch.display(),
                            e = e
                        ))
                    })?;
                }
            } else if save_all && d.is_dirty() {
                self.save(i)?;
            }
        }
        let plan = self.typeset_plan();
        let req = Request {
            root: root.clone(),
            engine: plan.engine,
            shell_escape: self.settings.shell_escape,
            bib_tool: self.settings.bib_tool,
            pretex: self.pretex_for(&plan),
        };
        self.typeset.console.clear();
        self.typeset.root = Some(root);
        self.typeset.live_job = false;
        self.typeset.job = Some(texcraft_typeset::start(
            req,
            &self.toolchain,
            self.waker.clone(),
        ));
        Ok(())
    }

    /// Starts a live-preview pass over the unsaved buffers (never saves anything).
    pub fn live_typeset(&mut self) -> Result<(), CommandError> {
        if self.typeset.is_running() {
            self.typeset.queued_live = true;
            return Ok(());
        }
        let root = self
            .root_path()
            .ok_or_else(|| CommandError::Failed(tr("nothing to typeset").into()))?;
        let root = texcraft_project::normalize(&root);
        let project_dir = root.parent().map(Path::to_path_buf).unwrap_or_default();
        let root_rel = root
            .file_name()
            .map(PathBuf::from)
            .ok_or_else(|| CommandError::Failed(tr("the root file has no name").into()))?;
        let buffers: Vec<(PathBuf, String)> = self
            .docs
            .iter()
            .filter_map(|d| {
                let p = texcraft_project::normalize(d.typeset_path()?);
                let rel = p.strip_prefix(&project_dir).ok()?.to_path_buf();
                Some((rel, d.text.clone()))
            })
            .collect();
        if !buffers.iter().any(|(p, _)| *p == root_rel) && !root.is_file() {
            return Err(CommandError::Failed(
                tr("the root file is not saved yet").into(),
            ));
        }
        let plan = self.typeset_plan();
        let mut dirs: Vec<PathBuf> = Vec::new();
        for f in &self.index.files {
            if let Some(rel) = f
                .path
                .parent()
                .and_then(|d| d.strip_prefix(&project_dir).ok())
                && !rel.as_os_str().is_empty()
                && !dirs.iter().any(|d| d == rel)
            {
                dirs.push(rel.to_path_buf());
            }
        }
        let req = LiveRequest {
            project_dir,
            root_rel,
            buffers,
            dirs,
            engine: plan.engine,
            shell_escape: self.settings.shell_escape,
            cache_preamble: self.settings.cache_preamble,
            pretex: self.pretex_for(&plan),
        };
        self.typeset.console.clear();
        self.typeset.root = Some(root);
        self.typeset.job_edit_at = self.typeset.pending_edit_at.take();
        self.typeset.live_job = true;
        self.typeset.job = Some(texcraft_typeset::start_live(
            req,
            &self.toolchain,
            self.waker.clone(),
        ));
        Ok(())
    }

    /// Stops the running typeset.
    pub fn stop_typeset(&mut self) {
        self.typeset.queued = false;
        self.typeset.live_due = None;
        if let Some(job) = &self.typeset.job {
            job.cancel();
        }
    }

    /// Advances background work: typeset output, finished runs, live typesetting. Call once
    /// per frame. Returns the time until it needs to be called again, if it is waiting.
    pub fn tick(&mut self, now: Instant) -> Option<Duration> {
        [
            self.tick_updates(now),
            self.tick_typeset(now),
            self.tick_git(now),
            self.tick_ai(),
            self.tick_inline(now),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn tick_typeset(&mut self, now: Instant) -> Option<Duration> {
        // Drain job events.
        let mut finished = None;
        if let Some(job) = &self.typeset.job {
            while let Some(ev) = job.try_recv() {
                match ev {
                    JobEvent::Output(line) => {
                        self.typeset.console.push_str(&line);
                        self.typeset.console.push('\n');
                    }
                    JobEvent::Finished(o) => finished = Some(o),
                }
            }
        }
        if let Some(outcome) = finished {
            self.typeset.job = None;
            // A live pass stopped to make way for a full typeset leaves no trace.
            let superseded = outcome.live && outcome.cancelled && self.typeset.queued;
            if !superseded {
                self.finish_typeset(*outcome);
            }
            if self.typeset.queued {
                self.typeset.queued = false;
                self.typeset.queued_live = false;
                if let Err(e) = self.typeset() {
                    self.set_status(e.to_string());
                }
            } else if self.typeset.queued_live {
                self.typeset.queued_live = false;
                if let Err(e) = self.live_typeset() {
                    self.set_status(e.to_string());
                }
            }
        }
        // Live typesetting.
        if let Some(due) = self.typeset.live_due {
            if !self.settings.live_typeset || self.live_paused {
                self.typeset.live_due = None;
            } else if now >= due {
                self.typeset.live_due = None;
                let external = self.settings.typesetter == Typesetter::External;
                if self.typeset.is_running() {
                    if external {
                        self.typeset.queued = true;
                    } else {
                        self.typeset.queued_live = true;
                    }
                } else {
                    let started = if external {
                        self.typeset()
                    } else {
                        self.live_typeset()
                    };
                    if let Err(e) = started {
                        self.set_status(e.to_string());
                    }
                }
            } else {
                return Some(due - now);
            }
        }
        if self.typeset.is_running() {
            return Some(Duration::from_millis(100));
        }
        None
    }

    fn finish_typeset(&mut self, outcome: Outcome) {
        self.typeset.runs += 1;
        self.typeset.finished_at = Some(std::time::SystemTime::now());
        self.typeset.issues = outcome.report.issues.clone();
        if let Some(err) = &outcome.error {
            self.set_status(err.clone());
            self.effect(Effect::Alert {
                title: tr("Typesetting could not start").into(),
                message: err.clone(),
            });
        } else if outcome.cancelled {
            self.set_status(tr("Typesetting stopped"));
        } else if outcome.live {
            self.typeset.last_live = Some(outcome.duration);
        } else {
            let errors = outcome.report.errors();
            let warnings = outcome.report.warnings();
            let errors_text = if errors == 1 {
                trf!("{n} error", n = errors)
            } else {
                trf!("{n} errors", n = errors)
            };
            let warnings_text = if warnings == 1 {
                trf!("{n} warning", n = warnings)
            } else {
                trf!("{n} warnings", n = warnings)
            };
            self.set_status(trf!(
                "Typeset in {seconds} s — {errors}, {warnings}",
                seconds = format!("{:.1}", outcome.duration.as_secs_f32()),
                errors = errors_text,
                warnings = warnings_text
            ));
            if errors > 0 && self.view.show_issues_after_error && !outcome.live {
                self.view.sidebar = true;
                self.view.sidebar_tab = SidebarTab::Results;
                self.view.results_mode = ResultsMode::Issues;
            }
            if let Some(pdf) = outcome.pdf.as_deref() {
                self.publish_pdf(pdf, outcome.pdf_fresh);
            }
        }
        // While typing, a pass with errors usually shows half-typed markup: keep the last good
        // preview (the errors still show in the gutter and the issues list).
        let keep_previous = outcome.live
            && self.settings.live_keep_good
            && outcome.report.errors() > 0
            && self.pdf.path.is_some();
        if let Some(pdf) = outcome.pdf.clone().filter(|_| !keep_previous) {
            // Reload only when the file changed (an up-to-date run leaves it alone).
            if outcome.pdf_fresh
                || self.pdf.path.as_ref() != Some(&pdf)
                || self.pdf.synctex.is_none()
            {
                self.pdf.synctex = outcome
                    .synctex
                    .as_deref()
                    .and_then(|p| SyncTex::load(p).ok());
                self.pdf.mirror = outcome.mirror.clone();
                self.pdf.path = Some(pdf);
                self.pdf.generation += 1;
                self.pdf.edit_at = if outcome.live {
                    self.typeset.job_edit_at
                } else {
                    None
                };
                self.effect(Effect::PdfChanged);
            }
            if self.view.sync_after_typeset {
                self.forward_sync(false);
            }
        }
        self.typeset.last = Some(outcome);
    }

    /// Shows an existing PDF (no typesetting).
    pub fn show_pdf(&mut self, pdf: PathBuf) {
        let synctex = SyncTex::path_for_pdf(&pdf).and_then(|p| SyncTex::load(&p).ok());
        self.pdf.synctex = synctex;
        self.pdf.mirror = None;
        self.pdf.path = Some(pdf);
        self.pdf.generation += 1;
        self.effect(Effect::PdfChanged);
    }

    /// Forward search: shows the caret's line in the PDF.
    pub fn forward_sync(&mut self, flash: bool) {
        let Some(doc) = self.doc() else { return };
        let Some(path) = doc.typeset_path().map(Path::to_path_buf) else {
            return;
        };
        let line = doc.caret_line();
        let query = self
            .pdf
            .mirror
            .as_ref()
            .and_then(|m| m.to_mirror(&path))
            .unwrap_or(path);
        let rects = self
            .pdf
            .synctex
            .as_ref()
            .map(|s| s.forward(&query, line))
            .unwrap_or_default();
        if rects.is_empty() {
            if flash {
                self.set_status(tr("No matching position in the PDF"));
            }
            return;
        }
        self.effect(Effect::ShowInPdf { rects, flash });
    }

    /// Inverse search: opens the source at the point on a PDF page.
    pub fn inverse_sync(&mut self, page: u32, x: f32, y: f32) -> Result<(), CommandError> {
        self.inverse_sync_with(page, x, y, None)
    }

    /// Inverse search with the PDF text around the point: the caret lands on the clicked
    /// character when the source line contains it, and its sentence (else the line) flashes.
    pub fn inverse_sync_with(
        &mut self,
        page: u32,
        x: f32,
        y: f32,
        hint: Option<&sync::SyncHint>,
    ) -> Result<(), CommandError> {
        let loc = self
            .pdf
            .synctex
            .as_ref()
            .and_then(|s| s.inverse(page, x, y))
            .ok_or_else(|| CommandError::Failed(tr("No source position for that point").into()))?;
        let base = self
            .typeset
            .root
            .as_ref()
            .and_then(|r| r.parent())
            .map(Path::to_path_buf);
        let file = if loc.file.is_absolute() {
            loc.file.clone()
        } else {
            base.unwrap_or_default().join(&loc.file)
        };
        let file = match &self.pdf.mirror {
            Some(m) => m.to_project(&file),
            None => file,
        };
        self.reveal(&file, loc.line)?;
        let doc = self.active;
        if let Some(d) = self.docs.get_mut(doc) {
            let start = edit::char_of_line(&d.text, loc.line);
            let from = edit::byte_of(&d.text, start);
            let rest = d.text.get(from..).unwrap_or_default();
            let line = rest.split('\n').next().unwrap_or_default();
            let (caret, flash) = sync::locate(line, loc.column, hint);
            d.selection = Selection::caret(start + caret);
            self.effect(Effect::FlashSource {
                doc,
                start: start + flash.start,
                end: start + flash.end,
            });
        }
        Ok(())
    }

    /// Opens `file` (if needed) and moves the caret to 1-based `line`.
    pub fn reveal(&mut self, file: &Path, line: u32) -> Result<(), CommandError> {
        let norm = texcraft_project::normalize(file);
        let idx = if let Some(i) = self.docs.iter().position(|d| {
            d.typeset_path()
                .is_some_and(|p| texcraft_project::normalize(p) == norm)
        }) {
            i
        } else {
            self.open_file(&norm)?
        };
        self.active = idx;
        if let Some(d) = self.docs.get_mut(idx) {
            d.selection = Selection::caret(edit::char_of_line(&d.text, line));
        }
        self.effect(Effect::RevealLine { doc: idx, line });
        Ok(())
    }

    /// Like [`Session::reveal`], and records the jump for Back / Forward.
    pub fn jump(&mut self, file: &Path, line: u32) -> Result<(), CommandError> {
        // Remember where we were, then where we went.
        if let Some(here) = self
            .doc()
            .and_then(|d| d.typeset_path().map(|p| (p.to_path_buf(), d.caret_line())))
        {
            self.history.truncate(self.history_pos + 1);
            if self.history.last() != Some(&here) {
                self.history.push(here);
            }
        }
        self.reveal(file, line)?;
        self.history.push((file.to_path_buf(), line));
        self.history_pos = self.history.len() - 1;
        Ok(())
    }

    /// Moves through the jump history by `delta` (-1 back, +1 forward).
    pub fn navigate(&mut self, delta: i64) -> Result<(), CommandError> {
        let pos = self.history_pos as i64 + delta;
        if pos < 0 || pos as usize >= self.history.len() {
            return Err(CommandError::Disabled("nav".into()));
        }
        self.history_pos = pos as usize;
        let (file, line) = self.history[self.history_pos].clone();
        self.reveal(&file, line)
    }

    /// Rebuilds the project index if documents changed (at most every 400 ms unless `force`).
    pub fn refresh_index(&mut self, force: bool) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let root = self.root_path();
        root.hash(&mut h);
        for d in &self.docs {
            d.typeset_path().hash(&mut h);
            d.revision.hash(&mut h);
        }
        self.typeset.runs.hash(&mut h);
        let stamp = h.finish();
        if stamp == self.index_stamp {
            return;
        }
        if !force
            && self
                .index_built
                .is_some_and(|t| t.elapsed() < Duration::from_millis(400))
        {
            return;
        }
        let Some(root) = root else {
            self.index = ProjectIndex::default();
            self.index_stamp = stamp;
            return;
        };
        let docs = &self.docs;
        let read = |p: &Path| -> Option<String> {
            docs.iter()
                .find(|d| {
                    d.typeset_path()
                        .is_some_and(|dp| texcraft_project::normalize(dp) == p)
                })
                .map(|d| d.text.clone())
                .or_else(|| std::fs::read_to_string(p).ok())
        };
        self.index = index::build(&texcraft_project::normalize(&root), &read);
        self.index_stamp = stamp;
        self.index_built = Some(Instant::now());
    }

    /// Shows the sidebar on `tab` (hides it when that tab is already showing).
    pub fn show_tab(&mut self, tab: SidebarTab) {
        if self.view.sidebar && self.view.sidebar_tab == tab {
            self.view.sidebar = false;
        } else {
            self.view.sidebar = true;
            self.view.sidebar_tab = tab;
        }
    }

    /// Counts of the last run's issues: (errors, warnings, bad boxes).
    pub fn issue_counts(&self) -> (usize, usize, usize) {
        let c = |s: Severity| {
            self.typeset
                .issues
                .iter()
                .filter(|i| i.severity == s)
                .count()
        };
        (
            c(Severity::Error),
            c(Severity::Warning),
            c(Severity::BadBox),
        )
    }

    /// Applies a pure text edit to the active document (used by formatting commands).
    pub(crate) fn edit_active(
        &mut self,
        f: impl FnOnce(&str, Selection) -> (String, Selection),
    ) -> Result<(), CommandError> {
        let idx = self.active;
        let doc = self
            .docs
            .get_mut(idx)
            .ok_or_else(|| CommandError::Failed(tr("no document").into()))?;
        let (text, sel) = f(&doc.text, doc.selection);
        doc.replace_text(text, sel);
        self.effect(Effect::SelectionChanged { doc: idx });
        self.schedule_live();
        Ok(())
    }

    /// A JSON summary of the state, for the control channel and tests.
    pub fn inspect(&self) -> serde_json::Value {
        let (errors, warnings, bad_boxes) = self.issue_counts();
        // English whatever the interface language: agents parse it.
        let toolchain = {
            let _en = texcraft_i18n::scoped(Lang::En);
            self.toolchain.describe()
        };
        serde_json::json!({
            "documents": self.docs.iter().enumerate().map(|(i, d)| serde_json::json!({
                "index": i,
                "name": d.display_name(),
                "path": d.path,
                "dirty": d.is_dirty(),
                "active": i == self.active,
                "chars": d.text.chars().count(),
                "selection": d.selection,
            })).collect::<Vec<_>>(),
            "project": self.project_dir,
            "root": self.root_path(),
            "engine": self.effective_engine().id(),
            "cjkSetup": self.typeset_plan().add_cjk,
            "settings": self.settings,
            "update": {
                "current": CURRENT_VERSION,
                "status": format!("{:?}", self.update.status),
                "busy": self.update.is_busy(),
            },
            "view": self.view,
            // The Language setting is `view.language`; this is what it means now.
            "language": {
                "resolved": self.lang().code(),
                "interface": texcraft_i18n::language().code(),
                "system": self.system_languages,
            },
            "typeset": {
                "mode": self.settings.mode().name(),
                "paused": self.live_paused,
                "running": self.typeset.is_running(),
                "live": self.typeset.live_job && self.typeset.is_running(),
                "lastLive": self.typeset.last.as_ref().map(|o| o.live),
                "runs": self.typeset.runs,
                "errors": errors,
                "warnings": warnings,
                "badBoxes": bad_boxes,
                "success": self.typeset.last.as_ref().map(|o| o.success),
                "seconds": self.typeset.last.as_ref().map(|o| o.duration.as_secs_f32()),
                "fastEnv": self.typeset.last.as_ref().map(|o| o.fast_env),
                "latencyMs": self.typeset.last_latency.map(|d| d.as_millis() as u64),
            },
            "pdf": {
                "path": self.pdf.path,
                "generation": self.pdf.generation,
                "synctex": self.pdf.synctex.is_some(),
                "selectedText": self.pdf.selected_text,
            },
            "status": self.status.as_ref().map(|s| s.0.clone()),
            "toolchain": toolchain,
            "ai": {
                "provider": self.ai.config.selection().map(|(p, _)| p.name.clone()),
                "model": self.ai.config.selection().map(|(_, m)| m.to_string()),
                "providers": self.ai.config.providers.iter().map(|p| serde_json::json!({
                    "id": p.id, "name": p.name, "kind": p.kind.name(), "baseUrl": p.base_url,
                    "hasKey": p.has_key, "models": p.models, "enabled": p.enabled,
                })).collect::<Vec<_>>(),
                "includeDocument": self.ai.config.include_document,
                "inline": self.inline_inspect(),
                "busy": self.ai.is_busy(),
                "readsImages": self.ai_reads_images(),
                "draftImages": self.ai.draft_images.iter().map(|i| serde_json::json!({
                    "name": i.name, "type": i.media_type, "bytes": i.data.len(),
                })).collect::<Vec<_>>(),
                "messages": self.ai.messages.iter().map(|m| serde_json::json!({
                    "role": if m.role == texcraft_ai::Role::User { "user" } else { "assistant" },
                    "text": m.text, "error": m.error, "pending": m.pending, "model": m.model,
                    "images": m.images.len(),
                })).collect::<Vec<_>>(),
            },
            "git": {
                "root": self.git.repo.as_ref().map(|r| r.root.clone()),
                "notRepo": self.git.not_repo,
                "branch": self.git.repo.as_ref().and_then(|r| r.status.branch.clone()),
                "staged": self.git.repo.as_ref().map(|r| r.status.staged.iter().map(|c| format!("{} {}", c.kind.letter(), c.path)).collect::<Vec<_>>()),
                "changes": self.git.repo.as_ref().map(|r| r.status.changes.iter().map(|c| format!("{} {}", c.kind.letter(), c.path)).collect::<Vec<_>>()),
                "commits": self.git.repo.as_ref().map_or(0, |r| r.log.len()),
                "busy": self.git.busy,
                "error": self.git.error,
            },
        })
    }
}

#[cfg(test)]
mod tests;
