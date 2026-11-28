//! Saved AI conversations, kept per project in a folder of their own, never in the project (a
//! project sent to someone carries no chats). With conversation sync explicitly enabled in
//! Settings and iCloud Drive available they live in
//! `iCloud Drive/TexCraft/AI Assistant/`, so every Mac signed in to the same account sees them;
//! otherwise in TexCraft's data folder's `chats/` ([`ChatStore::locate`]).
//!
//! A project's conversations live in `<name>-<hash>/`: one `<id>.json` per conversation (each
//! its own file, so two Macs adding conversations never edit the same file), the images sent
//! with them in `attachments/` (named by their SHA-256, written once), `index.json` (a cache of
//! titles; the conversation files are what counts) and `project.json` (the project's path).
//! The hash is of the project's path relative to iCloud Drive or the home folder when it is
//! inside one, so the same project has the same folder on every Mac. Conversations without a
//! project folder go to `_none/`. Nothing is deleted behind the user's back: a project that is
//! missing here (moved, on a drive that is not connected, or only on another Mac) keeps its
//! conversations.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use texcraft_ai::{Image, Role};

use crate::ai::ChatMessage;
use crate::ai_tools::ToolUse;
use crate::ai_usage::{ChatUsage, PromptSample, RunUsage};

/// The folder of conversations without a project.
const NO_PROJECT: &str = "_none";
/// The folder of a project's attachments, beside its conversations.
const ATTACHMENTS: &str = "attachments";
/// The longest title (characters): the start of the first question.
const TITLE_CHARS: usize = 48;

/// One saved conversation in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSummary {
    /// Its id (the file name).
    pub id: String,
    /// The start of its first question.
    pub title: String,
    /// When it last changed (seconds since 1970).
    pub updated: u64,
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    path: PathBuf,
    #[serde(default)]
    aliases: Vec<Alias>,
    #[serde(default)]
    legacy: Vec<String>,
}

/// A rename inside the project: the files of conversations saved before it are found under
/// the new name. One saved after it already has the new paths, and `from` may by then be
/// another file (a new one with the old name).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum Alias {
    Timed {
        from: PathBuf,
        to: PathBuf,
        /// Event timestamp (milliseconds since 1970, strictly ordered within this process).
        at: u64,
    },
    /// Recorded before renames had times: only for conversations saved before saves had them.
    Untimed(PathBuf, PathBuf),
}

impl Alias {
    fn paths_mut(&mut self) -> (&mut PathBuf, &mut PathBuf) {
        match self {
            Self::Timed { from, to, .. } | Self::Untimed(from, to) => (from, to),
        }
    }

    /// Whether it applies to a conversation saved at `saved` (ms; `None`: by an older TexCraft).
    fn applies_to(&self, saved: Option<u64>) -> bool {
        match (self, saved) {
            (Self::Timed { at, .. }, Some(saved)) => *at > saved,
            (Self::Timed { .. }, None) | (Self::Untimed(..), None) => true,
            (Self::Untimed(..), Some(_)) => false,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct StoredImage {
    name: String,
    media_type: String,
    /// The file in `attachments/` holding it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    /// Base64 (conversations saved before attachments had files of their own).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    data: String,
}

#[derive(Serialize, Deserialize)]
struct StoredMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<crate::ai::MessageSource>,
    role: Role,
    text: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    images: Vec<StoredImage>,
    #[serde(default)]
    tools: Vec<ToolUse>,
}

#[derive(Serialize, Deserialize)]
struct StoredChat {
    /// The start of its first question (older files have none: see [`title`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    /// When it was saved (seconds since 1970; older files have none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated: Option<u64>,
    /// The strictly ordered millisecond event stamp: which renames came after it
    /// ([`Alias::applies_to`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    saved: Option<u64>,
    messages: Vec<StoredMessage>,
    /// What its answers used (older files have none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    runs: Vec<RunUsage>,
    /// The last prompt the provider counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sample: Option<PromptSample>,
}

/// What the history list needs of a conversation file (the rest is skipped).
#[derive(Deserialize)]
struct StoredHead {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    updated: Option<u64>,
    #[serde(default)]
    messages: Vec<StoredHeadMessage>,
}

#[derive(Deserialize)]
struct StoredHeadMessage {
    role: Role,
    #[serde(default)]
    text: String,
}

/// Where conversations are kept, and the folders project paths are relative to.
#[derive(Debug, Clone)]
pub struct ChatStore {
    root: PathBuf,
    /// (name, folder): a project inside one of these is identified by its path relative to it,
    /// the same on every Mac. The most specific first.
    anchors: Vec<(String, PathBuf)>,
}

/// FNV-1a: a hash that stays the same from one build to the next (std's does not promise to).
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A conversation id: only letters and digits (it names a file).
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// An attachment's file name as [`ChatStore::save`] writes it: hex digits, a dot, an extension.
fn valid_attachment(name: &str) -> bool {
    name.split_once('.').is_some_and(|(hash, ext)| {
        !hash.is_empty()
            && hash.len() <= 64
            && hash.chars().all(|c| c.is_ascii_hexdigit())
            && !ext.is_empty()
            && ext.len() <= 8
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

fn now_ms() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    ordered_ms(&LAST, wall)
}

/// Saves and renames need a strict order: equal wall-clock milliseconds would either skip
/// a rename of an older answer or apply it to a newer answer about a reused file name.
fn ordered_ms(clock: &AtomicU64, wall: u64) -> u64 {
    let mut previous = clock.load(Ordering::Relaxed);
    loop {
        let next = wall.max(previous.saturating_add(1));
        match clock.compare_exchange_weak(previous, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(current) => previous = current,
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// When `path` last changed (seconds since 1970), 0 if unknown.
fn modified_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// Writes `bytes` to `path` through a temporary file (a crash leaves the old file whole).
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_file_name(format!(".{name}.{}.{n}.part", std::process::id()));
    std::fs::write(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("cannot save the conversation: {e}")
        })
}

/// A conversation's save time, independent of iCloud's download time on this Mac.
fn saved_at(path: &Path) -> u64 {
    #[derive(Deserialize)]
    struct Time {
        saved: Option<u64>,
        updated: Option<u64>,
    }
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Time>(&bytes).ok())
        .and_then(|time| {
            time.saved
                .or_else(|| time.updated.map(|t| t.saturating_mul(1000)))
        })
        .unwrap_or_else(|| modified_secs(path).saturating_mul(1000))
}

/// Copies a store without deleting the source. A newer conversation wins, attachments are
/// immutable, and the two metadata files merge. Runs on the storage worker, never per frame.
fn copy_dir(from: &Path, to: &Path, cancel: &AtomicBool) -> std::io::Result<()> {
    if !from.try_exists()? {
        return Ok(());
    }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        if cancel.load(Ordering::Relaxed) {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || kind.is_symlink() {
            continue;
        }
        let source = entry.path();
        let dest = to.join(name.as_ref());
        if kind.is_dir() {
            copy_dir(&source, &dest, cancel)?;
        } else if kind.is_file() {
            if dest.try_exists()? {
                let merged = match name.as_ref() {
                    "index.json" => Some(merge_index(&source, &dest)),
                    "project.json" => Some(merge_project(&source, &dest)),
                    _ => None,
                };
                if let Some(ok) = merged {
                    if !ok {
                        return Err(std::io::Error::other(
                            "could not merge conversation metadata",
                        ));
                    }
                    continue;
                }
                let conversation = name.strip_suffix(".json").is_some_and(valid_id);
                if !conversation || saved_at(&source) <= saved_at(&dest) {
                    continue;
                }
            }
            let bytes = std::fs::read(&source)?;
            write_atomic(&dest, &bytes).map_err(std::io::Error::other)?;
        }
    }
    Ok(())
}

/// The SHA-256 of `bytes` in hex: an attachment's name.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The file extension of an image's media type.
fn extension(media_type: &str) -> &'static str {
    match media_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "bin",
    }
}

/// iCloud Drive's folder in `home`, if iCloud Drive is on.
pub fn icloud_drive(home: &Path) -> Option<PathBuf> {
    let dir = home
        .join("Library")
        .join("Mobile Documents")
        .join("com~apple~CloudDocs");
    dir.is_dir().then_some(dir)
}

/// A new conversation id: the time in milliseconds and a counter, in hex.
pub fn new_id() -> String {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{ms:x}{:x}{n:08x}", std::process::id())
}

/// The title of a conversation: its first question's first line, shortened.
fn title(messages: &[ChatMessage]) -> String {
    title_of(
        messages
            .iter()
            .find(|m| m.role == Role::User)
            .map(|m| m.text.as_str()),
    )
}

fn title_of(first_question: Option<&str>) -> String {
    let first = first_question
        .map(|text| text.trim().lines().next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    if first.chars().count() <= TITLE_CHARS {
        return first;
    }
    let cut: String = first.chars().take(TITLE_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// Moves everything in `src` into `dst` without overwriting anything there, then removes
/// `src` if that emptied it. A conversation already in `dst` stays as it is (and so does
/// `src`'s copy); the two folders' `index.json`/`project.json` are combined.
fn merge_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if std::fs::rename(src, dst).is_ok() {
            return Ok(());
        }
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            merge_dir(&from, &to)?;
            continue;
        }
        if !to.exists() {
            if std::fs::rename(&from, &to).is_err() {
                std::fs::copy(&from, &to)?;
                std::fs::remove_file(&from)?;
            }
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let combined = match name.as_str() {
            "index.json" => merge_index(&from, &to),
            "project.json" => merge_project(&from, &to),
            // Named by their contents: the same name holds the same bytes.
            _ if src.file_name().is_some_and(|n| n == ATTACHMENTS) => true,
            _ => false,
        };
        if combined {
            std::fs::remove_file(&from)?;
        }
    }
    let _ = std::fs::remove_dir(src);
    Ok(())
}

/// Adds `from`'s entries to the index `to`. Returns whether it did.
fn merge_index(from: &Path, to: &Path) -> bool {
    let read = |path: &Path| -> Option<Vec<ChatSummary>> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    };
    let (Some(older), Some(mut list)) = (read(from), read(to)) else {
        return false;
    };
    for chat in older {
        if !list.iter().any(|c| c.id == chat.id) {
            list.push(chat);
        }
    }
    serde_json::to_vec_pretty(&list)
        .ok()
        .is_some_and(|json| write_atomic(to, &json).is_ok())
}

/// Adds `from`'s renames and older folders to the project file `to`. Returns whether it did.
fn merge_project(from: &Path, to: &Path) -> bool {
    let read = |path: &Path| -> Option<ProjectFile> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    };
    let (Some(older), Some(mut file)) = (read(from), read(to)) else {
        return false;
    };
    for alias in older.aliases {
        if !file.aliases.contains(&alias) {
            file.aliases.push(alias);
        }
    }
    for key in older.legacy {
        if !file.legacy.contains(&key) {
            file.legacy.push(key);
        }
    }
    serde_json::to_vec_pretty(&file)
        .ok()
        .is_some_and(|json| write_atomic(to, &json).is_ok())
}

impl ChatStore {
    /// Conversations kept under `root`, with project paths taken as they are.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            anchors: Vec::new(),
        }
    }

    /// Selects where this Mac keeps conversations. Merely having iCloud Drive never opts in.
    /// Copies and legacy folder adoption run separately on the storage worker.
    pub fn locate(local: PathBuf, home: Option<&Path>, sync_icloud: bool) -> Self {
        let icloud = home.and_then(icloud_drive);
        let mut anchors = Vec::new();
        if let Some(dir) = &icloud {
            anchors.push((
                "icloud".to_string(),
                dir.canonicalize().unwrap_or(dir.clone()),
            ));
        }
        if let Some(home) = home {
            anchors.push((
                "home".to_string(),
                home.canonicalize().unwrap_or(home.to_path_buf()),
            ));
        }
        let root = icloud.filter(|_| sync_icloud).map_or_else(
            || local.clone(),
            |dir| dir.join("TexCraft").join("AI Assistant"),
        );
        Self { root, anchors }
    }

    /// Imports old storage without removing anything there. Called off the UI thread.
    pub(crate) fn prepare(&self, sources: &[PathBuf], cancel: &AtomicBool) -> Result<(), String> {
        std::fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        for source in sources {
            if source != &self.root {
                if self.root.starts_with(source) || source.starts_with(&self.root) {
                    return Err("conversation storage folders overlap".into());
                }
                copy_dir(source, &self.root, cancel).map_err(|e| e.to_string())?;
            }
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(std::io::ErrorKind::Interrupted.to_string());
        }
        self.adopt(&self.root);
        Ok(())
    }

    /// The folder conversations are kept in.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `path` as it is written down: relative to an anchor (`$icloud/…`, `$home/…`) when it is
    /// inside one, so it means the same on another Mac.
    fn portable(&self, path: &Path) -> PathBuf {
        for (name, base) in &self.anchors {
            if let Ok(rest) = path.strip_prefix(base) {
                return Path::new(&format!("${name}")).join(rest);
            }
        }
        path.to_path_buf()
    }

    /// The path on this Mac of a path written down by [`ChatStore::portable`].
    fn local(&self, path: &Path) -> PathBuf {
        let mut parts = path.components();
        if let Some(Component::Normal(first)) = parts.next()
            && let Some(name) = first.to_str().and_then(|s| s.strip_prefix('$'))
            && let Some((_, base)) = self.anchors.iter().find(|(n, _)| n == name)
        {
            return base.join(parts.as_path());
        }
        path.to_path_buf()
    }

    fn project_file(&self, directory: &Path) -> Option<ProjectFile> {
        let mut file: ProjectFile =
            serde_json::from_str(&std::fs::read_to_string(directory.join("project.json")).ok()?)
                .ok()?;
        file.path = self.local(&file.path);
        for alias in &mut file.aliases {
            let (from, to) = alias.paths_mut();
            *from = self.local(from);
            *to = self.local(to);
        }
        Some(file)
    }

    fn write_project_file(&self, directory: &Path, file: &ProjectFile) -> Result<(), String> {
        let stored = ProjectFile {
            path: self.portable(&file.path),
            aliases: file
                .aliases
                .iter()
                .cloned()
                .map(|mut alias| {
                    let (from, to) = alias.paths_mut();
                    *from = self.portable(from);
                    *to = self.portable(to);
                    alias
                })
                .collect(),
            legacy: file.legacy.clone(),
        };
        write_atomic(
            &directory.join("project.json"),
            &serde_json::to_vec_pretty(&stored).map_err(|error| error.to_string())?,
        )
    }

    /// Moves the project folders in `from` (an older location, or this one under an older
    /// naming) to the folders they belong in now.
    fn adopt(&self, from: &Path) {
        let Ok(entries) = std::fs::read_dir(from) else {
            return;
        };
        let entries: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        let mut renamed: Vec<(String, String)> = Vec::new();
        for dir in entries {
            let Some(name) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            let target = match self.project_file(&dir) {
                Some(file) => self.dir(Some(&file.path)),
                // `_none`, `_retained-…`: kept by name.
                None => self.root.join(&name),
            };
            if target == dir || merge_dir(&dir, &target).is_err() {
                continue;
            }
            if let Some(new) = target.file_name().map(|n| n.to_string_lossy().into_owned())
                && new != name
            {
                renamed.push((name, new));
            }
        }
        if renamed.is_empty() {
            return;
        }
        // Histories that name a moved folder as one of their older ones follow it.
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        for dir in entries.flatten().map(|e| e.path()) {
            let Some(mut file) = self.project_file(&dir) else {
                continue;
            };
            let mut changed = false;
            for key in &mut file.legacy {
                if let Some((_, new)) = renamed.iter().find(|(old, _)| old == key) {
                    *key = new.clone();
                    changed = true;
                }
            }
            if changed {
                let _ = self.write_project_file(&dir, &file);
            }
        }
    }

    fn history_dirs(&self, project: Option<&Path>) -> Vec<PathBuf> {
        let primary = self.dir(project);
        let mut directories = vec![primary.clone()];
        if let Some(file) = self.project_file(&primary) {
            for key in file.legacy {
                if !key.is_empty()
                    && key.chars().all(|character| {
                        character.is_alphanumeric() || matches!(character, '-' | '_')
                    })
                {
                    let path = self.root.join(key);
                    if !directories.contains(&path) {
                        directories.push(path);
                    }
                }
            }
        }
        if let Some(legacy) = self.legacy_dir(project)
            && !directories.contains(&legacy)
        {
            directories.push(legacy);
        }
        directories
    }

    /// Re-key a moved project; its conversation files stay in place.
    pub(crate) fn relocate_project(
        &self,
        old: &Path,
        new: &Path,
        from: &Path,
        to: &Path,
    ) -> Result<(), String> {
        let old_dir = self.dir(Some(old));
        let new_dir = self.dir(Some(new));
        let mut directories = self.history_dirs(Some(old));
        if old != new && new_dir.exists() {
            // The destination did not exist on disk before the rename. A cached history
            // at that name belongs to a previously removed project; retain it separately.
            let archive = self.root.join(format!("_retained-{}", new_id()));
            std::fs::rename(&new_dir, archive).map_err(|error| error.to_string())?;
        }
        if old != new && old_dir.exists() && !new_dir.exists() {
            std::fs::rename(&old_dir, &new_dir).map_err(|error| error.to_string())?;
            directories.retain(|directory| *directory != old_dir);
        }
        if self.legacy_dir(Some(new)).is_some()
            && let Some(parent) = old.parent()
        {
            let directory = self.dir(Some(parent));
            if directory.exists() && !directories.contains(&directory) {
                directories.push(directory);
            }
        }
        let mut legacy = Vec::new();
        let at = now_ms();
        for directory in directories {
            if directory.exists() && directory != new_dir {
                if let Some(name) = directory.file_name().and_then(|name| name.to_str()) {
                    legacy.push(name.to_string());
                }
                let mut file = self.project_file(&directory).unwrap_or(ProjectFile {
                    path: new.to_path_buf(),
                    aliases: Vec::new(),
                    legacy: Vec::new(),
                });
                file.path = new.to_path_buf();
                file.aliases.push(Alias::Timed {
                    from: from.to_path_buf(),
                    to: to.to_path_buf(),
                    at,
                });
                self.write_project_file(&directory, &file)?;
            }
        }
        if !new_dir.exists() && legacy.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&new_dir).map_err(|error| error.to_string())?;
        let mut file = self.project_file(&new_dir).unwrap_or(ProjectFile {
            path: new.to_path_buf(),
            aliases: Vec::new(),
            legacy: Vec::new(),
        });
        file.path = new.to_path_buf();
        file.aliases.push(Alias::Timed {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            at,
        });
        for key in legacy {
            if !file.legacy.contains(&key) {
                file.legacy.push(key);
            }
        }
        self.write_project_file(&new_dir, &file)
    }
    // Legacy histories were keyed by directory. They are eligible only when that directory
    // has exactly one root, so two independent projects never inherit each other's history.
    fn legacy_dir(&self, project: Option<&Path>) -> Option<PathBuf> {
        let root = project.filter(|path| path.is_file())?;
        let parent = root.parent()?;
        let roots: Vec<_> = std::fs::read_dir(parent)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "tex"))
            .filter(|path| {
                std::fs::read_to_string(path)
                    .is_ok_and(|text| texcraft_syntax::is_root_document(&text))
            })
            .collect();
        (roots.len() == 1 && roots.first().is_some_and(|path| path == root))
            .then(|| self.dir(Some(parent)))
    }

    /// The folder of `project`'s conversations (`None`: no project folder).
    fn dir(&self, project: Option<&Path>) -> PathBuf {
        let Some(project) = project else {
            return self.root.join(NO_PROJECT);
        };
        let name: String = project
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .chars()
            .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
            .take(32)
            .collect();
        let hash = fnv(&self.portable(project).to_string_lossy());
        self.root.join(format!("{name}-{hash:016x}"))
    }

    /// The conversation files in `directory`: (id, path).
    fn conversation_files(directory: &Path) -> Vec<(String, PathBuf)> {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let id = name.strip_suffix(".json")?;
                (valid_id(id) && id != "index" && id != "project")
                    .then(|| (id.to_string(), entry.path()))
            })
            .collect()
    }

    /// `project`'s conversations, the latest first: every conversation file there (including
    /// those another Mac added), with titles from `index.json` where it knows them.
    pub fn list(&self, project: Option<&Path>) -> Vec<ChatSummary> {
        let mut list: Vec<ChatSummary> = Vec::new();
        for directory in self.history_dirs(project) {
            let indexed: Vec<ChatSummary> = std::fs::read_to_string(directory.join("index.json"))
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default();
            for (id, path) in Self::conversation_files(&directory) {
                if list.iter().any(|current| current.id == id) {
                    continue;
                }
                let modified = modified_secs(&path);
                let summary = match indexed.iter().find(|c| c.id == id) {
                    Some(known) => ChatSummary {
                        id,
                        title: known.title.clone(),
                        updated: known.updated.max(modified),
                    },
                    None => {
                        let Some(head) = std::fs::read_to_string(&path)
                            .ok()
                            .and_then(|text| serde_json::from_str::<StoredHead>(&text).ok())
                        else {
                            continue;
                        };
                        let title = head.title.unwrap_or_else(|| {
                            title_of(
                                head.messages
                                    .iter()
                                    .find(|m| m.role == Role::User)
                                    .map(|m| m.text.as_str()),
                            )
                        });
                        ChatSummary {
                            id,
                            title,
                            updated: head.updated.unwrap_or(0).max(modified),
                        }
                    }
                };
                list.push(summary);
            }
        }
        list.sort_by_key(|c| std::cmp::Reverse(c.updated));
        list
    }

    /// Conversation `id` of `project`, and what it used.
    pub fn load(&self, project: Option<&Path>, id: &str) -> Option<(Vec<ChatMessage>, ChatUsage)> {
        if !valid_id(id) {
            return None;
        }
        let (text, directory) = self
            .history_dirs(project)
            .into_iter()
            .find_map(|directory| {
                std::fs::read_to_string(directory.join(format!("{id}.json")))
                    .ok()
                    .map(|text| (text, directory))
            })?;
        let aliases = self
            .project_file(&directory)
            .map(|file| file.aliases)
            .unwrap_or_default();
        let chat: StoredChat = serde_json::from_str(&text).ok()?;
        let aliases: Vec<Alias> = aliases
            .into_iter()
            .filter(|alias| alias.applies_to(chat.saved))
            .collect();
        let usage = ChatUsage {
            runs: chat.runs,
            sample: chat.sample,
        };
        let messages = chat
            .messages
            .into_iter()
            .map(|m| {
                let mut source = m.source;
                if let Some(source) = &mut source {
                    for path in [&mut source.project, &mut source.file, &mut source.root]
                        .into_iter()
                        .flatten()
                    {
                        *path = self.local(path);
                        for alias in &aliases {
                            let (Alias::Timed { from, to, .. } | Alias::Untimed(from, to)) = alias;
                            if let Some(moved) = crate::files::rebase(path, from, to) {
                                *path = moved;
                            }
                        }
                    }
                }
                ChatMessage {
                    source,
                    role: m.role,
                    text: m.text,
                    blocks: Vec::new(),
                    model: m.model,
                    provider: m.provider,
                    pending: false,
                    thinking: false,
                    error: m.error,
                    note: m.note,
                    images: m
                        .images
                        .iter()
                        .filter_map(|i| match &i.file {
                            // Not there (yet: iCloud may still be bringing it): left out.
                            Some(file) if valid_attachment(file) => {
                                let bytes =
                                    std::fs::read(directory.join(ATTACHMENTS).join(file)).ok()?;
                                Some(Image {
                                    media_type: i.media_type.clone(),
                                    data: bytes.into(),
                                    name: i.name.clone(),
                                })
                            }
                            Some(_) => None,
                            None => Image::from_base64(&i.name, &i.media_type, &i.data),
                        })
                        .collect(),
                    tools: m.tools,
                }
            })
            .collect();
        Some((messages, usage))
    }

    /// Writes `image` into `dir`'s attachments (once: it is named by its contents) and returns
    /// its file name.
    fn store_image(dir: &Path, image: &Image) -> Result<String, String> {
        let name = format!(
            "{}.{}",
            sha256_hex(&image.data),
            extension(&image.media_type)
        );
        let folder = dir.join(ATTACHMENTS);
        let path = folder.join(&name);
        if !path.is_file() {
            std::fs::create_dir_all(&folder)
                .map_err(|e| format!("cannot save the conversation: {e}"))?;
            write_atomic(&path, &image.data)?;
        }
        Ok(name)
    }

    /// Saves conversation `id` of `project` (an answer still arriving is left out).
    pub fn save(
        &self,
        project: Option<&Path>,
        id: &str,
        messages: &[ChatMessage],
        usage: &ChatUsage,
    ) -> Result<(), String> {
        if !valid_id(id) {
            return Err(format!("not a conversation id: {id}"));
        }
        let dir = self.dir(project);
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot save the conversation: {e}"))?;
        if let Some(path) = project {
            let mut file = self.project_file(&dir).unwrap_or(ProjectFile {
                path: path.to_path_buf(),
                aliases: Vec::new(),
                legacy: Vec::new(),
            });
            file.path = path.to_path_buf();
            self.write_project_file(&dir, &file)?;
        }
        let title = title(messages);
        let updated = now_secs();
        let mut stored = Vec::new();
        for m in messages.iter().filter(|m| !m.pending) {
            let mut images = Vec::new();
            for image in &m.images {
                images.push(StoredImage {
                    name: image.name.clone(),
                    media_type: image.media_type.clone(),
                    file: Some(Self::store_image(&dir, image)?),
                    data: String::new(),
                });
            }
            stored.push(StoredMessage {
                source: m.source.as_ref().map(|source| crate::ai::MessageSource {
                    project: source.project.as_deref().map(|p| self.portable(p)),
                    file: source.file.as_deref().map(|p| self.portable(p)),
                    root: source.root.as_deref().map(|p| self.portable(p)),
                }),
                role: m.role,
                text: m.text.clone(),
                model: m.model.clone(),
                provider: m.provider.clone(),
                error: m.error.clone(),
                note: m.note.clone(),
                images,
                tools: m.tools.clone(),
            });
        }
        let chat = StoredChat {
            title: Some(title.clone()),
            updated: Some(updated),
            saved: Some(now_ms()),
            messages: stored,
            runs: usage.runs.clone(),
            sample: usage.sample.clone(),
        };
        let json = serde_json::to_vec(&chat).map_err(|e| e.to_string())?;
        write_atomic(&dir.join(format!("{id}.json")), &json)?;
        let mut list = self.list(project);
        list.retain(|c| c.id != id);
        list.push(ChatSummary {
            id: id.to_string(),
            title,
            updated,
        });
        let json = serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?;
        write_atomic(&dir.join("index.json"), &json)
    }

    /// The attachments the conversation file `path` refers to.
    fn attachments_of(path: &Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<StoredChat>(&text).ok())
            .map(|chat| {
                chat.messages
                    .into_iter()
                    .flat_map(|m| m.images)
                    .filter_map(|i| i.file)
                    .filter(|f| valid_attachment(f))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Deletes conversation `id` of `project`, and the attachments only it used.
    pub fn delete(&self, project: Option<&Path>, id: &str) -> Result<(), String> {
        if !valid_id(id) {
            return Err(format!("not a conversation id: {id}"));
        }
        for directory in self.history_dirs(project) {
            let path = directory.join(format!("{id}.json"));
            let attachments = Self::attachments_of(&path);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("cannot delete the conversation: {e}")),
            }
            if attachments.is_empty() {
                continue;
            }
            let used: Vec<String> = Self::conversation_files(&directory)
                .iter()
                .flat_map(|(_, other)| Self::attachments_of(other))
                .collect();
            for file in attachments {
                if !used.contains(&file) {
                    let _ = std::fs::remove_file(directory.join(ATTACHMENTS).join(file));
                }
            }
        }
        let dir = self.dir(project);
        let mut list = self.list(project);
        list.retain(|c| c.id != id);
        if !dir.exists() {
            return Ok(());
        }
        let json = serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?;
        write_atomic(&dir.join("index.json"), &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_saves_and_renames_have_distinct_ordered_timestamps() {
        let mut previous = now_ms();
        for _ in 0..64 {
            let next = now_ms();
            assert!(
                next > previous,
                "save and rename share timestamp {previous}"
            );
            previous = next;
        }
    }

    #[test]
    fn a_coarse_or_backward_clock_preserves_which_answers_a_rename_applies_to() {
        let clock = AtomicU64::new(0);
        let saved_before = ordered_ms(&clock, 42);
        let alias = Alias::Timed {
            from: "old.tex".into(),
            to: "new.tex".into(),
            at: ordered_ms(&clock, 42),
        };
        let saved_after = ordered_ms(&clock, 41);
        assert!(alias.applies_to(Some(saved_before)));
        assert!(!alias.applies_to(Some(saved_after)));
        assert_eq!(ordered_ms(&clock, 100), 100);
        assert_eq!(ordered_ms(&clock, u64::MAX), u64::MAX);
    }

    fn message(role: Role, text: &str) -> ChatMessage {
        ChatMessage {
            source: None,
            role,
            text: text.into(),
            blocks: Vec::new(),
            model: Some("m".into()),
            provider: None,
            pending: false,
            thinking: false,
            error: None,
            note: None,
            images: Vec::new(),
            tools: Vec::new(),
        }
    }

    /// Conversations are kept per project outside it, listed latest first, read back whole,
    /// deleted, and kept when their project is missing.
    #[test]
    fn conversations_live_outside_the_project_and_go_with_it() {
        let base = std::env::temp_dir().join(format!("texcraft-chats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let project = base.join("paper");
        let other = base.join("thesis");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let store = ChatStore::new(base.join("data/chats"));

        let mut first = vec![
            message(
                Role::User,
                "Why does \\ref{fig:a} print ??\nThe figure is there.",
            ),
            message(Role::Assistant, "Typeset twice."),
        ];
        first[0].images.push(Image {
            media_type: "image/png".into(),
            data: vec![1u8, 2, 3].into(),
            name: "shot.png".into(),
        });
        first[1].tools.push(ToolUse {
            kind: crate::ai_tools::ToolKind::Read,
            target: "main.tex".into(),
            lines: None,
            failed: false,
        });
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        let usage = ChatUsage {
            runs: vec![RunUsage {
                message: 1,
                calls: 2,
                tool_calls: 1,
                measured: 2,
                input: 3000,
                output: 40,
                cache_hit: 2048,
                cache_reported: true,
                cost_usd: None,
                model_ms: 1500,
            }],
            sample: Some(PromptSample {
                provider: "p".into(),
                model: "m".into(),
                estimated: 1400,
                input: 1500,
            }),
        };
        store.save(Some(&project), &a, &first, &usage).unwrap();
        let long = "x".repeat(100);
        let none = ChatUsage::default();
        store
            .save(Some(&project), &b, &[message(Role::User, &long)], &none)
            .unwrap();
        store
            .save(
                Some(&other),
                &a,
                &[message(Role::User, "Other project")],
                &none,
            )
            .unwrap();

        // Nothing in the project folder.
        assert_eq!(std::fs::read_dir(&project).unwrap().count(), 0);
        let list = store.list(Some(&project));
        assert_eq!(list.len(), 2);
        assert!(
            list.iter()
                .any(|c| c.title == "Why does \\ref{fig:a} print ??")
        );
        assert!(
            list.iter()
                .any(|c| c.title.chars().count() == TITLE_CHARS && c.title.ends_with('…'))
        );
        let (back, back_usage) = store.load(Some(&project), &a).unwrap();
        assert_eq!(back_usage, usage, "what it used comes back with it");
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].text, first[0].text);
        assert_eq!(back[0].images, first[0].images);
        assert_eq!(back[1].tools, first[1].tools);
        assert_eq!(store.list(Some(&other))[0].title, "Other project");
        // Ids name files: nothing else is read or written.
        assert!(store.load(Some(&project), "../index").is_none());
        assert!(store.save(Some(&project), "../x", &first, &none).is_err());

        store.delete(Some(&project), &b).unwrap();
        assert_eq!(store.list(Some(&project)).len(), 1);

        // The project is gone from this Mac (moved, on a drive not connected, or only on
        // another Mac): its conversations stay.
        std::fs::remove_dir_all(&project).unwrap();
        let _ = ChatStore::locate(base.join("data/chats"), None, false);
        assert_eq!(store.list(Some(&project)).len(), 1);
        assert_eq!(store.list(Some(&other)).len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    fn image(bytes: &[u8]) -> Image {
        Image {
            media_type: "image/png".into(),
            data: bytes.to_vec().into(),
            name: "shot.png".into(),
        }
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// Images are files of their own in `attachments/` (written once, named by their
    /// contents), not base64 inside the conversation; deleting a conversation removes the
    /// attachments only it used. Conversations saved the old way still open.
    #[test]
    fn attachments_are_files_of_their_own() {
        let base = std::env::temp_dir().join(format!("texcraft-attach-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let project = base.join("paper");
        std::fs::create_dir_all(&project).unwrap();
        let store = ChatStore::new(base.join("chats"));
        let none = ChatUsage::default();
        let mut shared = message(Role::User, "Both");
        shared.images.push(image(b"shared-bytes"));
        let mut only = message(Role::User, "One");
        only.images = vec![image(b"only-bytes"), image(b"shared-bytes")];
        let (a, b) = (new_id(), new_id());
        store
            .save(Some(&project), &a, &[shared.clone()], &none)
            .unwrap();
        store
            .save(Some(&project), &b, &[only.clone()], &none)
            .unwrap();
        let dir = store.dir(Some(&project));
        let attachments = dir.join(ATTACHMENTS);
        assert_eq!(
            files_in(&attachments).len(),
            2,
            "the same image is kept once"
        );
        let json = std::fs::read_to_string(dir.join(format!("{b}.json"))).unwrap();
        assert!(!json.contains(&Image::base64(&image(b"only-bytes"))));
        assert_eq!(
            store.load(Some(&project), &b).unwrap().0[0].images,
            only.images
        );

        store.delete(Some(&project), &b).unwrap();
        assert_eq!(
            files_in(&attachments),
            vec![format!("{}.png", sha256_hex(b"shared-bytes"))],
            "the image the other conversation uses stays"
        );
        assert_eq!(
            store.load(Some(&project), &a).unwrap().0[0].images,
            shared.images
        );

        // A conversation saved before attachments had files, without a title or index entry.
        let old = r#"{"messages":[{"role":"User","text":"Old question\nmore","images":[{"name":"x.png","media_type":"image/png","data":"AQID"}]}]}"#;
        std::fs::write(dir.join("abc123.json"), old).unwrap();
        let list = store.list(Some(&project));
        assert!(
            list.iter()
                .any(|c| c.id == "abc123" && c.title == "Old question")
        );
        let back = store.load(Some(&project), "abc123").unwrap().0;
        assert_eq!(&back[0].images[0].data[..], &[1u8, 2, 3]);
        // A missing or malformed attachment is left out, never read from elsewhere.
        let bad = r#"{"messages":[{"role":"User","text":"Q","images":[{"name":"x","media_type":"image/png","file":"../../secret.png"},{"name":"y","media_type":"image/png","file":"00ff.png"}]}]}"#;
        std::fs::write(dir.join("bad1.json"), bad).unwrap();
        assert!(
            store.load(Some(&project), "bad1").unwrap().0[0]
                .images
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// An available iCloud folder must not opt a user into uploading conversations.
    #[test]
    fn conversations_stay_local_until_sync_is_enabled() {
        let base =
            std::env::temp_dir().join(format!("texcraft-chat-opt-in-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("home");
        let cloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
        std::fs::create_dir_all(&cloud).unwrap();
        let local = base.join("data/chats");
        let store = ChatStore::locate(local.clone(), Some(&home), false);
        assert_eq!(
            store.root(),
            local,
            "iCloud sync must require an explicit opt-in"
        );
        store
            .save(
                None,
                "abc123",
                &[message(Role::User, "Local only")],
                &ChatUsage::default(),
            )
            .unwrap();
        assert!(!cloud.join("TexCraft").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    /// With sync explicitly on, conversations copy there from the data folder (and from the
    /// naming by absolute path); another Mac with a different home folder sees the same
    /// history for the same project, with its own paths; conversations another Mac added
    /// show up although this Mac's index never heard of them.
    #[test]
    fn conversations_copy_to_icloud_and_follow_the_project_between_macs() {
        let base = std::env::temp_dir().join(format!("texcraft-icloud-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let cloud = |home: &Path| home.join("Library/Mobile Documents/com~apple~CloudDocs");
        let (home, other_home) = (base.join("mac1/alice"), base.join("mac2/bob"));
        for h in [&home, &other_home] {
            std::fs::create_dir_all(cloud(h).join("CS6215")).unwrap();
            std::fs::write(
                cloud(h).join("CS6215/sigconf.tex"),
                "\\documentclass{article}",
            )
            .unwrap();
        }
        let home = home.canonicalize().unwrap();
        let other_home = other_home.canonicalize().unwrap();
        let root_file = cloud(&home).join("CS6215/sigconf.tex");

        // Saved by an older TexCraft in the data folder, keyed by the absolute path.
        let local = base.join("mac1/support/chats");
        let mut question = message(Role::User, "Why is subsubsection run in?");
        question.source = Some(crate::ai::MessageSource {
            project: root_file.parent().map(Path::to_path_buf),
            file: Some(root_file.clone()),
            root: Some(root_file.clone()),
        });
        question.images.push(image(b"screenshot"));
        let id = new_id();
        ChatStore::new(local.clone())
            .save(
                Some(&root_file),
                &id,
                &[question.clone()],
                &ChatUsage::default(),
            )
            .unwrap();

        let store = ChatStore::locate(local.clone(), Some(&home), true);
        store
            .prepare(std::slice::from_ref(&local), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(store.root(), cloud(&home).join("TexCraft/AI Assistant"));
        assert!(local.exists(), "the original history is retained");
        assert_eq!(store.list(Some(&root_file)).len(), 1);
        let (back, _) = store.load(Some(&root_file), &id).unwrap();
        assert_eq!(back[0].source, question.source);
        assert_eq!(back[0].images, question.images);
        // Written down relative to iCloud Drive, so it means the same on another Mac.
        store
            .save(Some(&root_file), &id, &back, &ChatUsage::default())
            .unwrap();
        let file = std::fs::read_to_string(store.dir(Some(&root_file)).join(format!("{id}.json")))
            .unwrap();
        assert!(!file.contains("alice"), "{file}");

        // iCloud brings the folder to the other Mac.
        let synced = cloud(&other_home).join("TexCraft");
        std::fs::create_dir_all(&synced).unwrap();
        let copy = std::process::Command::new("cp")
            .arg("-R")
            .arg(store.root())
            .arg(&synced)
            .status()
            .unwrap();
        assert!(copy.success());
        let there = ChatStore::locate(base.join("mac2/support/chats"), Some(&other_home), true);
        there.prepare(&[], &AtomicBool::new(false)).unwrap();
        let there_root = cloud(&other_home).join("CS6215/sigconf.tex");
        assert_eq!(there.list(Some(&there_root)).len(), 1);
        let (back, _) = there.load(Some(&there_root), &id).unwrap();
        let source = back[0].source.as_ref().unwrap();
        assert_eq!(source.file.as_deref(), Some(there_root.as_path()));
        assert_eq!(back[0].images, question.images);

        // A conversation that Mac adds arrives as a file only.
        let added = new_id();
        there
            .save(
                Some(&there_root),
                &added,
                &[message(Role::User, "From the other Mac")],
                &ChatUsage::default(),
            )
            .unwrap();
        std::fs::copy(
            there.dir(Some(&there_root)).join(format!("{added}.json")),
            store.dir(Some(&root_file)).join(format!("{added}.json")),
        )
        .unwrap();
        let list = store.list(Some(&root_file));
        assert!(
            list.iter()
                .any(|c| c.id == added && c.title == "From the other Mac")
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The Assistant saves each conversation as it goes, lists the project's history, starts
    /// afresh and reopens; another project has its own history; nothing goes into a project.
    #[test]
    fn the_assistant_keeps_conversations_per_project() {
        use crate::Session;
        use crate::ai_tools::tests::{fake_model, sse};
        use serde_json::json;
        let base =
            std::env::temp_dir().join(format!("texcraft-chats-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (a, b) = (base.join("paper"), base.join("thesis"));
        for d in [&a, &b] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("main.tex"), "Hello\n").unwrap();
        }
        let mut s = Session::new(std::sync::Arc::new(|| {}));
        s.settings.live_typeset = false;
        s.update.url = "/nonexistent/texcraft-latest.json".into();
        s.set_chat_store(base.join("data/chats"));
        s.set_project_dir(a.clone());
        s.tick(std::time::Instant::now());
        let (port, server) =
            fake_model(vec![sse(json!({"content": "First answer."}), Some("stop"))]);
        let ollama = texcraft_ai::PRESETS
            .iter()
            .find(|p| p.id == "ollama")
            .unwrap();
        let i = s.ai.config.add(ollama);
        s.ai.config.providers[i].base_url = format!("http://127.0.0.1:{port}/v1");
        s.ai.config.providers[i].models = vec!["m".into()];
        s.ai.config.normalize();
        s.run("ai.send", &json!({"text": "First question"}))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while s.ai.is_busy() && std::time::Instant::now() < deadline {
            s.tick(std::time::Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        server.join().unwrap();
        let id = s.ai.chat_id.clone().unwrap();
        assert_eq!(s.ai.history.len(), 1);
        assert_eq!(s.ai.history[0].title, "First question");

        // New conversation: empty, the history keeps the first; it opens again whole.
        s.run("ai.clear", &json!(null)).unwrap();
        assert!(s.ai.messages.is_empty() && s.ai.chat_id.is_none());
        assert_eq!(s.ai.history.len(), 1);
        s.run("ai.openChat", &json!({"id": id})).unwrap();
        assert_eq!(s.ai.messages.len(), 2);
        assert_eq!(s.ai.messages[1].text, "First answer.");

        // Another project: its own (empty) history; back again, the first one's.
        s.set_project_dir(b.clone());
        s.tick(std::time::Instant::now());
        assert!(s.ai.messages.is_empty() && s.ai.history.is_empty());
        s.set_project_dir(a.clone());
        s.tick(std::time::Instant::now());
        assert_eq!(s.ai.history.len(), 1);

        s.run("ai.deleteChat", &json!({"id": id})).unwrap();
        assert!(s.ai.history.is_empty());
        for d in [&a, &b] {
            let names: Vec<_> = std::fs::read_dir(d)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(names, vec!["main.tex"], "nothing written in {d:?}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
