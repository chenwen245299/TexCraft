//! Simplified Chinese (zh-Hans). One table per area of the app, each owned by whoever works on
//! that area. Keys are the English strings exactly as the code passes them to `tr`/`trf!`
//! (after escapes: `"\\end"` in the source is `\end`). A key may appear in several tables only
//! with the same translation (`tests/catalogue.rs`).
//!
//! Style: macOS Simplified Chinese (文件、编辑、显示、窗口、帮助; 设置、存储、拷贝、排版);
//! full-width punctuation (，。：；？！“”（）) in sentences; a space between Chinese and Latin
//! words or numbers ("退出 TexCraft", "{n} 个文件"); "…" stays "…".

mod ai;
mod app;
mod commands;
mod editor;
mod engine;
mod git;
mod prefs;
mod sidebar;
mod workbench;

type Entries = &'static [(&'static str, &'static str)];

const TABLES: &[Entries] = &[
    commands::ENTRIES,
    engine::ENTRIES,
    app::ENTRIES,
    workbench::ENTRIES,
    editor::ENTRIES,
    sidebar::ENTRIES,
    git::ENTRIES,
    prefs::ENTRIES,
    ai::ENTRIES,
];

pub(crate) fn entries() -> impl Iterator<Item = (&'static str, &'static str)> {
    TABLES.iter().flat_map(|t| t.iter().copied())
}
