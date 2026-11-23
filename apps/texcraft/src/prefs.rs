//! Preferences every window shares (`app.ron`, eframe's storage). Each project window is its
//! own process with its own copy, and eframe writes the whole file from that copy; so each save
//! merges instead ([`texcraft_engine::prefs::merge3`]): what this window changed since it last
//! read or wrote the file wins, the rest is the file's. A change another window saved is taken
//! over here too, as other editors apply a setting in every window. The layout of a window
//! (sidebar, split, zoom, …) is its own, and only the last one saved opens new windows.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;
use texcraft_engine::View;

/// eframe's storage key for [`Persisted`].
pub const STORAGE_KEY: &str = "texcraft-state-v2";

/// Fields of [`View`] that describe one window's layout, not a preference.
const WINDOW_FIELDS: &[&str] = &[
    "sidebar",
    "assistant",
    "sidebar_tab",
    "contents_mode",
    "results_mode",
    "search_mode",
    "layout",
    "zoom",
    "page_mode",
    "grading",
    "scroll_lock",
];

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
pub struct Persisted {
    pub view: Option<View>,
    pub settings: Option<texcraft_project::Settings>,
    pub recent: Vec<PathBuf>,
    pub split: Option<f32>,
    /// Bumped when defaults change in a way saved settings should pick up.
    #[serde(default)]
    pub version: u32,
}

/// Recent documents of two windows: this one's additions first, then the file's, without what
/// this window removed.
fn merge_recent(base: &[PathBuf], ours: &[PathBuf], disk: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let removed = |p: &PathBuf| base.contains(p) && !ours.contains(p);
    for p in ours
        .iter()
        .filter(|p| !base.contains(p))
        .chain(disk.iter().filter(|p| !removed(p)))
        .chain(ours.iter())
    {
        if !out.contains(p) {
            out.push(p.clone());
        }
    }
    out.truncate(12);
    out
}

/// `ours` merged into `disk` since `base`.
pub fn merge(base: &Persisted, ours: &Persisted, disk: &Persisted) -> Persisted {
    let json = |p: &Persisted| serde_json::to_value(p).unwrap_or(Value::Null);
    let merged = texcraft_engine::prefs::merge3(&json(base), &json(ours), &json(disk));
    let mut out: Persisted = serde_json::from_value(merged).unwrap_or_else(|_| ours.clone());
    out.recent = merge_recent(&base.recent, &ours.recent, &disk.recent);
    out.version = ours.version.max(disk.version);
    out
}

/// `view` with the preferences of `shared` (its window layout stays).
pub fn adopt_view(view: &View, shared: &View) -> View {
    let (Ok(Value::Object(mut mine)), Ok(Value::Object(theirs))) =
        (serde_json::to_value(view), serde_json::to_value(shared))
    else {
        return view.clone();
    };
    for (key, value) in theirs {
        if !WINDOW_FIELDS.contains(&key.as_str()) {
            mine.insert(key, value);
        }
    }
    serde_json::from_value(Value::Object(mine)).unwrap_or_else(|_| view.clone())
}

/// Reads the preferences eframe stored in `file` (`None`: missing, or being written).
pub fn read(file: &Path) -> Option<Persisted> {
    let text = std::fs::read_to_string(file).ok()?;
    let kv: std::collections::HashMap<String, String> = ron::from_str(&text).ok()?;
    ron::from_str(kv.get(STORAGE_KEY)?).ok()
}

/// The preferences in `p` every window shares (its layout left out).
fn shared_part(p: &Persisted) -> Value {
    let mut value = serde_json::to_value(p).unwrap_or(Value::Null);
    if let Some(Value::Object(view)) = value.get_mut("view") {
        for field in WINDOW_FIELDS {
            view.remove(*field);
        }
    }
    if let Value::Object(all) = &mut value {
        all.remove("split");
        all.remove("version");
    }
    value
}

fn modified(file: &Path) -> Option<SystemTime> {
    std::fs::metadata(file).and_then(|m| m.modified()).ok()
}

/// This window's view of the shared file.
pub struct Shared {
    file: PathBuf,
    /// The preferences as this window last read or wrote them.
    base: Persisted,
    /// The file's date then.
    seen: Option<SystemTime>,
}

impl Shared {
    pub fn new(file: PathBuf, loaded: Persisted) -> Self {
        let seen = modified(&file);
        Self {
            file,
            base: loaded,
            seen,
        }
    }

    /// What to write: `ours` merged with what other windows wrote meanwhile. It becomes what
    /// this window and the file agree on.
    pub fn for_save(&mut self, ours: &Persisted) -> Persisted {
        let merged = match read(&self.file) {
            Some(disk) => merge(&self.base, ours, &disk),
            None => ours.clone(),
        };
        self.base = merged.clone();
        merged
    }

    /// Whether this window changed preferences it has not saved yet (layout aside).
    pub fn unsaved(&self, ours: &Persisted) -> bool {
        shared_part(ours) != shared_part(&self.base)
    }

    /// What another window saved since, merged with this window's own unsaved changes
    /// (`None`: nothing new).
    pub fn poll(&mut self, ours: &Persisted) -> Option<Persisted> {
        let date = modified(&self.file);
        if date.is_none() || date == self.seen {
            return None;
        }
        // Being written (eframe truncates first): next time.
        let disk = read(&self.file)?;
        let merged = merge(&self.base, ours, &disk);
        // This window's unsaved changes still differ from the file, so they stay its own.
        self.base = disk;
        self.seen = date;
        Some(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persisted(view: View, recent: &[&str]) -> Persisted {
        Persisted {
            view: Some(view),
            settings: Some(texcraft_project::Settings::default()),
            recent: recent.iter().map(PathBuf::from).collect(),
            split: Some(0.5),
            version: 3,
        }
    }

    /// Window A changes the theme while window B changes the font size and opens a project:
    /// whoever saves last keeps both, and A's recents keep B's project.
    #[test]
    fn two_windows_keep_each_others_changes() {
        let base = persisted(View::default(), &["/p/a.tex"]);
        let mut a = base.clone();
        a.view.as_mut().unwrap().theme = texcraft_engine::ThemeChoice::Dark;
        a.recent.insert(0, "/p/c.tex".into());
        let mut b = base.clone();
        b.view.as_mut().unwrap().editor_font_size = 20.0;
        b.recent.insert(0, "/p/b.tex".into());
        // B saved first; A merges.
        let merged = merge(&base, &a, &b);
        let view = merged.view.unwrap();
        assert_eq!(view.theme, texcraft_engine::ThemeChoice::Dark);
        assert_eq!(view.editor_font_size, 20.0);
        assert_eq!(
            merged.recent,
            ["/p/c.tex", "/p/b.tex", "/p/a.tex"].map(PathBuf::from)
        );
        // A recent entry A removed stays removed.
        let mut cleared = base.clone();
        cleared.recent.clear();
        assert!(merge(&base, &cleared, &b).recent == vec![PathBuf::from("/p/b.tex")]);
    }

    /// Taking another window's preferences leaves this window's layout alone.
    #[test]
    fn another_windows_layout_is_not_taken_over() {
        let mine = View {
            sidebar: false,
            ..View::default()
        };
        let theirs = View {
            sidebar: true,
            editor_font_size: 21.0,
            ..View::default()
        };
        let adopted = adopt_view(&mine, &theirs);
        assert!(!adopted.sidebar);
        assert_eq!(adopted.editor_font_size, 21.0);
    }

    /// The shared file as eframe writes it: another window's save is picked up once, merged
    /// with this window's unsaved change.
    #[test]
    fn another_windows_save_is_taken_over_with_this_ones_unsaved_change() {
        let dir = std::env::temp_dir().join(format!("texcraft-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("app.ron");
        let write = |p: &Persisted| {
            let mut kv = std::collections::HashMap::new();
            kv.insert(STORAGE_KEY.to_string(), ron::to_string(p).unwrap());
            std::fs::write(&file, ron::to_string(&kv).unwrap()).unwrap();
        };
        let base = persisted(View::default(), &[]);
        write(&base);
        let mut shared = Shared::new(file.clone(), base.clone());
        let mut mine = base.clone();
        mine.view.as_mut().unwrap().wrap_lines = false;
        assert!(shared.unsaved(&mine));
        assert!(shared.poll(&mine).is_none(), "nothing new yet");
        // Another window saves a font size (a moment later: a new file date).
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let mut other = base.clone();
        other.view.as_mut().unwrap().editor_font_size = 19.0;
        write(&other);
        let taken = shared.poll(&mine).unwrap();
        let view = taken.view.clone().unwrap();
        assert_eq!(view.editor_font_size, 19.0);
        assert!(!view.wrap_lines, "this window's unsaved change stays");
        assert!(shared.unsaved(&taken), "and is still this window's to save");
        assert!(shared.poll(&taken).is_none(), "taken once");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
