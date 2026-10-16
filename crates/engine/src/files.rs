//! File-browser actions (the Files sidebar's context menu): insert a reference to a file in the
//! editor, rename, duplicate and move to the Trash.

use std::path::{Path, PathBuf};

use texcraft_project::FileKind;

use crate::{CommandError, Effect, Session, edit};

/// Moves a file or folder to the Trash / Recycle Bin. The macOS app installs AppKit's (see
/// `texcraft-macos`); the default uses the system's command-line tools.
pub type TrashFn = fn(&Path) -> Result<(), String>;

/// The default [`TrashFn`]: the Recycle Bin through PowerShell on Windows, `gio trash` on
/// Linux. Elsewhere it fails (the macOS app installs its own).
pub fn system_trash(path: &Path) -> Result<(), String> {
    let p = path.to_string_lossy().replace('\'', "''");
    let mut cmd = if cfg!(windows) {
        let what = if path.is_dir() {
            "DeleteDirectory"
        } else {
            "DeleteFile"
        };
        let mut c = std::process::Command::new("powershell");
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "Add-Type -AssemblyName Microsoft.VisualBasic; \
                 [Microsoft.VisualBasic.FileIO.FileSystem]::{what}('{p}', 'OnlyErrorDialogs', 'SendToRecycleBin')"
            ),
        ]);
        c
    } else if cfg!(target_os = "linux") {
        let mut c = std::process::Command::new("gio");
        c.arg("trash").arg(path);
        c
    } else {
        return Err("moving to the Trash is not available here".into());
    };
    match cmd.output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
        Err(e) => Err(e.to_string()),
    }
}

fn failed(msg: impl Into<String>) -> CommandError {
    CommandError::Failed(msg.into())
}

/// `path` relative to `base`, with `/` separators (what TeX expects on every platform); a
/// path outside `base` stays absolute. Symbolic links are resolved first when the files exist
/// (macOS's `/var` is `/private/var`).
fn relative(path: &Path, base: &Path) -> String {
    let (path, base) = (real(path), real(base));
    match path.strip_prefix(&base) {
        Ok(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => path.to_string_lossy().replace('\\', "/"),
    }
}

/// `p` with symbolic links resolved when it exists, else just normalised.
fn real(p: &Path) -> PathBuf {
    p.canonicalize()
        .unwrap_or_else(|_| texcraft_project::normalize(p))
}

/// The LaTeX that includes `path` from a file in `from_dir`: `\input{…}` for TeX sources,
/// `\includegraphics{…}` for images and PDFs, `\bibliography{…}` for `.bib` files, else the
/// relative path itself.
pub fn reference_to(path: &Path, from_dir: &Path) -> String {
    let rel = relative(path, from_dir);
    let stem = rel
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty() && !stem.ends_with('/'))
        .map_or(rel.as_str(), |(stem, _)| stem);
    match FileKind::of(path) {
        FileKind::Tex => format!("\\input{{{stem}}}"),
        FileKind::Image | FileKind::Pdf => format!("\\includegraphics{{{rel}}}"),
        FileKind::Bib => format!("\\bibliography{{{stem}}}"),
        _ => rel,
    }
}

/// A name for a copy of `path` next to it that does not exist yet: `name copy.ext`,
/// `name copy 2.ext`, ...
fn copy_name(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    (1..1000).find_map(|n| {
        let candidate = if n == 1 {
            dir.join(format!("{stem} copy{ext}"))
        } else {
            dir.join(format!("{stem} copy {n}{ext}"))
        };
        (!candidate.exists()).then_some(candidate)
    })
}

/// `dir/stem.ext`, or `dir/stem 2.ext`, `dir/stem 3.ext`, … whichever does not exist yet.
fn unused_name(dir: &Path, stem: &str, ext: &str) -> Option<PathBuf> {
    (1..1000).find_map(|n| {
        let candidate = if n == 1 {
            dir.join(format!("{stem}{ext}"))
        } else {
            dir.join(format!("{stem} {n}{ext}"))
        };
        (!candidate.exists()).then_some(candidate)
    })
}

fn copy_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_recursive(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

impl Session {
    /// Inserts the LaTeX that includes `path` at the caret of the active document.
    pub fn insert_file_reference(&mut self, path: &Path) -> Result<(), CommandError> {
        let from = self
            .doc()
            .and_then(|d| d.typeset_path())
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| self.project_dir.clone())
            .ok_or_else(|| failed("no document to insert into"))?;
        let text = reference_to(path, &from);
        self.edit_active(|t, sel| edit::replace_selection(t, sel, &text))?;
        // Back to the editor, caret after the insertion.
        if let Some(line) = self.doc().map(|d| d.caret_line()) {
            self.effect(Effect::RevealLine {
                doc: self.active,
                line,
            });
        }
        Ok(())
    }

    /// Renames the file or folder at `path` to `name` (in the same folder). Open documents
    /// follow it.
    pub fn rename_file(&mut self, path: &Path, name: &str) -> Result<PathBuf, CommandError> {
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
            return Err(failed(format!("“{name}” is not a valid name")));
        }
        let dir = path.parent().ok_or_else(|| failed("cannot rename this"))?;
        let to = dir.join(name);
        if to == path {
            return Ok(to);
        }
        // Allow changing only the case of a name on case-insensitive disks.
        let same_file = to.exists()
            && to.file_name().map(|n| n.to_string_lossy().to_lowercase())
                == path.file_name().map(|n| n.to_string_lossy().to_lowercase());
        if to.exists() && !same_file {
            return Err(failed(format!("“{name}” already exists")));
        }
        // Which open documents live at or under `path` (resolved before it moves).
        let from = real(path);
        let to_real = real(dir).join(name);
        let followers: Vec<(usize, PathBuf)> = self
            .docs
            .iter()
            .enumerate()
            .filter_map(|(i, d)| {
                let p = real(d.path.as_ref()?);
                Some((i, p.strip_prefix(&from).ok()?.to_path_buf()))
            })
            .collect();
        std::fs::rename(path, &to)
            .map_err(|e| failed(format!("cannot rename {}: {e}", path.display())))?;
        for (i, rest) in followers {
            if let Some(d) = self.docs.get_mut(i) {
                d.path = Some(if rest.as_os_str().is_empty() {
                    to_real.clone()
                } else {
                    to_real.join(rest)
                });
            }
        }
        for r in &mut self.recent {
            if real(r) == from {
                *r = to_real.clone();
            }
        }
        self.refresh_files();
        self.refresh_index(true);
        Ok(to)
    }

    /// Creates `untitled folder` (or `untitled folder 2`, …) in `dir` and starts renaming it.
    pub fn create_folder(&mut self, dir: &Path) -> Result<PathBuf, CommandError> {
        let to = unused_name(dir, "untitled folder", "")
            .ok_or_else(|| failed("no free name for a new folder"))?;
        std::fs::create_dir(&to)
            .map_err(|e| failed(format!("cannot create {}: {e}", to.display())))?;
        self.refresh_files();
        self.effect(Effect::BeginRename(to.clone()));
        Ok(to)
    }

    /// Creates an empty `untitled.tex` (or `untitled 2.tex`, …) in `dir` and starts renaming it.
    pub fn create_file(&mut self, dir: &Path) -> Result<PathBuf, CommandError> {
        let to = unused_name(dir, "untitled", ".tex")
            .ok_or_else(|| failed("no free name for a new file"))?;
        std::fs::write(&to, "")
            .map_err(|e| failed(format!("cannot create {}: {e}", to.display())))?;
        self.refresh_files();
        self.effect(Effect::BeginRename(to.clone()));
        Ok(to)
    }

    /// Copies `path` next to itself (`name copy.ext`) and returns the copy.
    pub fn duplicate_file(&mut self, path: &Path) -> Result<PathBuf, CommandError> {
        let to = copy_name(path).ok_or_else(|| failed("cannot duplicate this"))?;
        copy_recursive(path, &to)
            .map_err(|e| failed(format!("cannot duplicate {}: {e}", path.display())))?;
        self.refresh_files();
        Ok(to)
    }

    /// Moves `path` to the Trash. Open documents inside it close; one with unsaved changes
    /// stops it.
    pub fn trash_file(&mut self, path: &Path) -> Result<(), CommandError> {
        let target = real(path);
        let inside = |d: &crate::Document| {
            d.path
                .as_ref()
                .is_some_and(|p| real(p).starts_with(&target))
        };
        if let Some(d) = self.docs.iter().find(|d| inside(d) && d.is_dirty()) {
            return Err(failed(format!(
                "{} has unsaved changes: save or close it first",
                d.display_name()
            )));
        }
        (self.trash)(path).map_err(|e| failed(format!("cannot move to the Trash: {e}")))?;
        while let Some(i) = self.docs.iter().position(inside) {
            self.close(i);
        }
        self.recent.retain(|r| !real(r).starts_with(&target));
        self.refresh_files();
        self.refresh_index(true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_use_relative_paths_and_the_right_command() {
        let dir = Path::new("/p");
        assert_eq!(
            reference_to(Path::new("/p/sections/intro.tex"), dir),
            "\\input{sections/intro}"
        );
        assert_eq!(
            reference_to(Path::new("/p/figs/plot.pdf"), dir),
            "\\includegraphics{figs/plot.pdf}"
        );
        assert_eq!(
            reference_to(Path::new("/p/refs.bib"), dir),
            "\\bibliography{refs}"
        );
        assert_eq!(reference_to(Path::new("/p/data.csv"), dir), "data.csv");
        // From a subfolder, outside paths stay absolute rather than guessing `..`.
        assert_eq!(
            reference_to(Path::new("/p/a.png"), Path::new("/p/sections")),
            "\\includegraphics{/p/a.png}"
        );
    }

    #[test]
    fn copy_names_count_up() {
        let d = std::env::temp_dir().join(format!("texcraft-copyname-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("main.tex");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(copy_name(&f).unwrap(), d.join("main copy.tex"));
        std::fs::write(d.join("main copy.tex"), "x").unwrap();
        assert_eq!(copy_name(&f).unwrap(), d.join("main copy 2.tex"));
        assert_eq!(
            copy_name(&d.join("Makefile")).unwrap(),
            d.join("Makefile copy")
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
