//! Bounded recovery of generated preview state. Source buffers are never rewritten here.

use std::path::Path;

use super::{LiveRequest, READ_BACK};

/// Two roots in one folder must never share a mirror manifest, formats or auxiliary files.
pub fn cache_dir(project: &Path, root: &Path) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut hash);
    project
        .join(".texcraft/live")
        .join(format!("root-{:016x}", hash.finish()))
}

/// Remove only generated state inside this root's mirror. Keep the source snapshots and the
/// last PDF, so a failed recovery can still leave the last good preview available.
pub(crate) fn reset(req: &LiveRequest) -> std::io::Result<()> {
    let live = req.live_dir();
    let mut dirs = vec![live.clone()];
    while let Some(dir) = dirs.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if req.buffers.iter().any(|(rel, _)| live.join(rel) == path) {
                continue;
            }
            // Never traverse a link into the project or TeX installation.
            if kind.is_symlink() {
                continue;
            }
            let name = entry.file_name();
            if kind.is_dir() {
                if name == super::FAST_DIR {
                    std::fs::remove_dir_all(path)?;
                } else {
                    dirs.push(path);
                }
                continue;
            }
            let generated = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    READ_BACK.contains(&ext) || matches!(ext, "fmt" | "nofmt" | "log" | "fls")
                })
                || [
                    super::LOCAL_MAP,
                    super::XE_CFG,
                    super::XE_MAP,
                    super::XE_NOMAP,
                ]
                .iter()
                .any(|item| name == *item);
            if generated {
                std::fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separate_roots_keep_their_snapshots_and_recovery_never_touches_sources() {
        let dir =
            std::env::temp_dir().join(format!("texcraft-recovery-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let request = |name: &str, text: &str| LiveRequest {
            project_dir: dir.clone(),
            root_rel: name.into(),
            buffers: vec![(name.into(), text.into())],
            dirs: vec![],
            engine: texcraft_project::Engine::PdfLatex,
            shell_escape: false,
            cache_preamble: true,
            pretex: None,
        };
        let a = request("a.tex", "Original A");
        let b = request("b.tex", "Original B");
        std::fs::write(dir.join("a.tex"), "Saved A").unwrap();
        super::super::prepare_mirror(&a, |_| None).unwrap();
        std::fs::write(a.live_dir().join("a.aux"), "bad generated state").unwrap();
        std::fs::write(a.live_dir().join("a.pdf"), "last good PDF").unwrap();
        super::super::prepare_mirror(&b, |_| None).unwrap();
        assert_eq!(
            std::fs::read_to_string(a.live_dir().join("a.tex")).unwrap(),
            "Original A"
        );
        reset(&a).unwrap();
        assert!(!a.live_dir().join("a.aux").exists());
        assert_eq!(
            std::fs::read_to_string(a.live_dir().join("a.pdf")).unwrap(),
            "last good PDF"
        );
        assert_eq!(
            std::fs::read_to_string(a.live_dir().join("a.tex")).unwrap(),
            "Original A"
        );
        assert_eq!(
            std::fs::read_to_string(b.live_dir().join("b.tex")).unwrap(),
            "Original B"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("a.tex")).unwrap(),
            "Saved A"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
