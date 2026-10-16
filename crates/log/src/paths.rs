//! File-name recognition and path resolution for names printed by TeX and BibTeX.

use std::path::{Component, Path, PathBuf};

/// Extensions of files TeX commonly opens (or that commonly appear in logs). Used to decide
/// where a file name ends when it is wrapped across log lines.
const KNOWN_EXTENSIONS: &[&str] = &[
    "tex", "ltx", "sty", "cls", "clo", "cfg", "def", "fd", "aux", "out", "toc", "lof", "lot",
    "loa", "lol", "bbl", "ind", "idx", "gls", "glo", "nls", "acn", "acr", "nav", "snm", "vrb",
    "ldf", "lua", "luc", "dtx", "ins", "dict", "enc", "map", "lbx", "bbx", "cbx", "dbx", "bcf",
    "pgf", "tikz", "mkii", "mkiv", "ist", "xmpdata", "cnf", "csv", "dat", "txt", "bib", "bst",
    "otf", "ttf", "pfb", "tfm", "vf", "lco", "mp", "eps", "pdf", "png", "jpg", "jpeg", "svg",
    "pygtex", "pygstyle", "thm", "xdv", "dvi", "gz", "code", "trc", "sage", "listing",
];

/// Resolves a file name printed by TeX against `base_dir`: relative names are joined to it,
/// `.` segments are dropped and `..` segments are folded lexically.
pub(crate) fn resolve(name: &str, base_dir: &Path) -> PathBuf {
    let name = name.trim();
    let path = Path::new(name);
    let joined = if path.is_absolute() || is_drive_path(name) {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    };
    normalize(&joined)
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `C:/...` or `C:\...` (printed by TeX on Windows).
fn is_drive_path(s: &str) -> bool {
    let b = s.as_bytes();
    matches!(
        (b.first(), b.get(1), b.get(2)),
        (Some(d), Some(b':'), Some(b'/' | b'\\')) if d.is_ascii_alphabetic()
    )
}

/// Starts like a path: `./`, `../`, `/`, `~/` or a drive letter.
pub(crate) fn path_like_start(s: &str) -> bool {
    s.starts_with("./")
        || s.starts_with("../")
        || s.starts_with(".\\")
        || s.starts_with('/')
        || s.starts_with("~/")
        || is_drive_path(s)
}

/// The extension of the last path component, if it has one.
fn extension(s: &str) -> Option<&str> {
    let name = s.rsplit(['/', '\\']).next()?;
    let dot = name.rfind('.')?;
    if dot == 0 {
        return None;
    }
    name.get(dot + 1..)
}

/// Has a plausible file extension: 2 to 8 ASCII alphanumerics starting with a letter.
pub(crate) fn has_extension(s: &str) -> bool {
    extension(s).is_some_and(|ext| {
        (2..=8).contains(&ext.len())
            && ext.starts_with(|c: char| c.is_ascii_alphabetic())
            && ext.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

/// Ends with one of [`KNOWN_EXTENSIONS`].
pub(crate) fn has_known_extension(s: &str) -> bool {
    extension(s).is_some_and(|ext| KNOWN_EXTENSIONS.iter().any(|k| ext.eq_ignore_ascii_case(k)))
}

/// Could be a file name printed by TeX after `(`.
pub(crate) fn is_file_like(s: &str) -> bool {
    if s.is_empty() || s.len() > 4096 {
        return false;
    }
    (path_like_start(s) && s.len() > 2) || has_extension(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_names_against_base() {
        let base = Path::new("/proj");
        assert_eq!(resolve("./main.tex", base), PathBuf::from("/proj/main.tex"));
        assert_eq!(
            resolve("chapters/./a.tex", base),
            PathBuf::from("/proj/chapters/a.tex")
        );
        assert_eq!(
            resolve("./sub/../b.tex", base),
            PathBuf::from("/proj/b.tex")
        );
        assert_eq!(resolve("../up.tex", base), PathBuf::from("/up.tex"));
        assert_eq!(resolve("/usr/x.sty", base), PathBuf::from("/usr/x.sty"));
        assert_eq!(
            resolve("main.tex", Path::new("")),
            PathBuf::from("main.tex")
        );
        assert_eq!(
            resolve("../a.tex", Path::new("")),
            PathBuf::from("../a.tex")
        );
    }

    #[test]
    fn file_like_names() {
        assert!(is_file_like("./main.tex"));
        assert!(is_file_like("main.tex"));
        assert!(is_file_like("ts1cmr.fd"));
        assert!(is_file_like(
            "/usr/local/texlive/2022/texmf-dist/tex/latex/base"
        ));
        for not_file in [
            "HO", "Font", "hyperref", "e.g.,", "v1.2", "U+4E2D", "i.e", "1.2pt", "",
        ] {
            assert!(!is_file_like(not_file), "{not_file}");
        }
        assert!(has_known_extension("/a/b/gettitlestring.sty"));
        assert!(!has_known_extension("./foo.texLaTeX"));
    }
}
