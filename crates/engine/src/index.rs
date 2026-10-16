//! The project index: every file reachable from the root by `\input`/`\include`, with its
//! outline, plus labels, bibliography entries, user commands and word counts. It feeds the
//! Contents and TODOs tabs, Jump to, and `\ref`/`\cite` completion.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use texcraft_syntax::outline::{
    OutlineItem, OutlineKind, braced_argument, commands, skip_optional,
};
use texcraft_syntax::{BibEntry, bib_entries, defined_commands, outline, word_count};

/// One file of the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedFile {
    /// Path (normalized).
    pub path: PathBuf,
    /// Its outline (sections, labels, floats, inputs, TODOs).
    pub outline: Vec<OutlineItem>,
    /// Index of the file that includes it (`None` for the root).
    pub parent: Option<usize>,
    /// The file does not exist.
    pub missing: bool,
    /// Visible words in it.
    pub words: usize,
}

/// A non-TeX file the project uses (image, bibliography, style).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// Path as resolved.
    pub path: PathBuf,
    /// Index of the file that references it.
    pub parent: usize,
    /// It exists on disk.
    pub exists: bool,
}

/// Everything known about the current project.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectIndex {
    /// The root file.
    pub root: Option<PathBuf>,
    /// TeX files in inclusion order (root first).
    pub files: Vec<IndexedFile>,
    /// Images, `.bib`, `.sty` and `.cls` files referenced by the TeX files.
    pub assets: Vec<Asset>,
    /// Bibliography entries with the `.bib` file they come from.
    pub bib: Vec<(BibEntry, PathBuf)>,
    /// User-defined command names (no backslash).
    pub commands: Vec<String>,
    /// Citation keys the document cites, with the first place each is cited.
    pub cites: Vec<(String, PathBuf, u32)>,
    /// Total visible words.
    pub words: usize,
}

impl ProjectIndex {
    /// All labels: (key, file, 1-based line, title of the section containing it).
    pub fn labels(&self) -> Vec<(String, PathBuf, u32, String)> {
        let mut out = Vec::new();
        for f in &self.files {
            let mut section = String::new();
            for item in &f.outline {
                if item.kind.is_sectioning() {
                    section = item.title.clone();
                } else if item.kind == OutlineKind::Label {
                    out.push((
                        item.title.clone(),
                        f.path.clone(),
                        item.line,
                        section.clone(),
                    ));
                }
            }
        }
        out
    }

    /// All outline items of `kind` across files: (file, item).
    pub fn items(&self, kind: OutlineKind) -> Vec<(&Path, &OutlineItem)> {
        self.files
            .iter()
            .flat_map(|f| f.outline.iter().map(move |i| (f.path.as_path(), i)))
            .filter(|(_, i)| i.kind == kind)
            .collect()
    }

    /// Every outline item in reading order, as (file index, item index): an `\input` is
    /// replaced by the included file's items, where it appears.
    pub fn reading_order(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        if !self.files.is_empty() {
            self.walk(0, &mut out, &mut seen);
        }
        out
    }

    fn walk(&self, fi: usize, out: &mut Vec<(usize, usize)>, seen: &mut HashSet<usize>) {
        if !seen.insert(fi) {
            return;
        }
        let base = self
            .root
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let Some(f) = self.files.get(fi) else { return };
        for (ii, item) in f.outline.iter().enumerate() {
            if item.kind == OutlineKind::Input {
                let target = resolve_tex(&base, &item.title);
                if let Some(child) = self
                    .files
                    .iter()
                    .position(|c| c.parent == Some(fi) && c.path == target)
                {
                    self.walk(child, out, seen);
                    continue;
                }
            }
            out.push((fi, ii));
        }
    }

    /// The innermost section containing `line` of `file`, as (file index, item index).
    pub fn section_at(&self, file: &Path, line: u32) -> Option<(usize, usize)> {
        let fi = self.files.iter().position(|f| f.path == file)?;
        let f = self.files.get(fi)?;
        let idx = f
            .outline
            .iter()
            .enumerate()
            .filter(|(_, i)| i.kind.is_sectioning() && i.line <= line)
            .map(|(k, _)| k)
            .next_back()?;
        Some((fi, idx))
    }
}

/// Builds the index for `root`. `read` returns a file's current text (open buffer or disk).
pub fn build(root: &Path, read: &dyn Fn(&Path) -> Option<String>) -> ProjectIndex {
    let mut idx = ProjectIndex {
        root: Some(root.to_path_buf()),
        ..ProjectIndex::default()
    };
    let base = root.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut seen = HashSet::new();
    let mut bib_files: Vec<PathBuf> = Vec::new();
    visit(
        root,
        None,
        &base,
        read,
        &mut idx,
        &mut seen,
        &mut bib_files,
        0,
    );
    for bib in bib_files {
        if let Some(text) = read(&bib) {
            for e in bib_entries(&text) {
                idx.bib.push((e, bib.clone()));
            }
        }
    }
    idx.words = idx.files.iter().map(|f| f.words).sum();
    idx
}

#[allow(clippy::too_many_arguments)]
fn visit(
    file: &Path,
    parent: Option<usize>,
    base: &Path,
    read: &dyn Fn(&Path) -> Option<String>,
    idx: &mut ProjectIndex,
    seen: &mut HashSet<PathBuf>,
    bib_files: &mut Vec<PathBuf>,
    depth: usize,
) {
    if depth > 16 || idx.files.len() > 500 || !seen.insert(file.to_path_buf()) {
        return;
    }
    let text = read(file);
    let me = idx.files.len();
    let Some(text) = text else {
        idx.files.push(IndexedFile {
            path: file.to_path_buf(),
            outline: Vec::new(),
            parent,
            missing: true,
            words: 0,
        });
        return;
    };
    let items = outline(&text);
    for c in defined_commands(&text) {
        if !idx.commands.contains(&c) {
            idx.commands.push(c);
        }
    }
    idx.files.push(IndexedFile {
        path: file.to_path_buf(),
        outline: items.clone(),
        parent,
        missing: false,
        words: word_count(&text),
    });

    // Citations: keys in \cite-like commands, first occurrence each.
    let mut lines = texcraft_syntax::outline::LineIndex::new(&text);
    for (offset, name) in commands(&text) {
        if !(name.contains("cite") || name == "nocite") {
            continue;
        }
        let at = offset + 1 + name.len();
        let at = if text.as_bytes().get(at) == Some(&b'*') {
            at + 1
        } else {
            at
        };
        let after_opts = skip_optional(&text, at).and_then(|p| skip_optional(&text, p));
        if let Some((keys, _)) = after_opts.and_then(|p| braced_argument(&text, p)) {
            for k in keys
                .split(',')
                .map(str::trim)
                .filter(|k| !k.is_empty() && *k != "*")
            {
                if !idx.cites.iter().any(|(c, _, _)| c == k) {
                    idx.cites
                        .push((k.to_string(), file.to_path_buf(), lines.line_of(offset)));
                }
            }
        }
    }
    // Assets: graphics, packages/classes that live in the project, bibliographies.
    for (offset, name) in commands(&text) {
        let args_at = offset + 1 + name.len();
        let arg = |t: &str| {
            skip_optional(t, args_at)
                .and_then(|p| braced_argument(t, p))
                .map(|(a, _)| a.trim().to_string())
        };
        match name {
            "includegraphics" => {
                if let Some(a) = arg(&text) {
                    let p = resolve_graphic(base, &a);
                    let exists = p.is_file();
                    idx.assets.push(Asset {
                        path: p,
                        parent: me,
                        exists,
                    });
                }
            }
            "usepackage" | "documentclass" => {
                if let Some(a) = arg(&text) {
                    let ext = if name == "usepackage" { "sty" } else { "cls" };
                    for pkg in a.split(',') {
                        let p = base.join(format!("{}.{ext}", pkg.trim()));
                        if p.is_file() {
                            idx.assets.push(Asset {
                                path: p,
                                parent: me,
                                exists: true,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    for item in &items {
        match item.kind {
            OutlineKind::Input => {
                let child = resolve_tex(base, &item.title);
                visit(
                    &child,
                    Some(me),
                    base,
                    read,
                    idx,
                    seen,
                    bib_files,
                    depth + 1,
                );
            }
            OutlineKind::Bibliography => {
                for b in item.title.split(',') {
                    let b = b.trim();
                    if b.is_empty() {
                        continue;
                    }
                    let p = base.join(if b.ends_with(".bib") {
                        b.to_string()
                    } else {
                        format!("{b}.bib")
                    });
                    let p = texcraft_project::normalize(&p);
                    if !bib_files.contains(&p) {
                        let exists = p.is_file();
                        idx.assets.push(Asset {
                            path: p.clone(),
                            parent: me,
                            exists,
                        });
                        bib_files.push(p);
                    }
                }
            }
            _ => {}
        }
    }
}

/// `\input{chapters/one}` → `<base>/chapters/one.tex`.
pub fn resolve_tex(base: &Path, name: &str) -> PathBuf {
    let p = base.join(name.trim());
    let p = if p.extension().is_none() {
        p.with_extension("tex")
    } else {
        p
    };
    texcraft_project::normalize(&p)
}

fn resolve_graphic(base: &Path, name: &str) -> PathBuf {
    let p = texcraft_project::normalize(&base.join(name));
    if p.extension().is_some() {
        return p;
    }
    for ext in ["pdf", "png", "jpg", "jpeg", "eps"] {
        let c = p.with_extension(ext);
        if c.is_file() {
            return c;
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn follows_inputs_and_collects_everything() {
        let files: HashMap<PathBuf, &str> = [
            ("/p/main.tex", "\\documentclass{article}\n\\newcommand{\\R}{x}\n\\begin{document}\n\\section{Intro}\\label{sec:a}\nOne two.\n\\input{ch/b}\n\\bibliography{refs}\n\\end{document}"),
            ("/p/ch/b.tex", "\\subsection{B}\\label{sec:b}\nThree four five. % TODO later\n\\input{missing}"),
            ("/p/refs.bib", "@book{knuth, title={TeX}, author={Knuth}, year=1984}"),
        ]
        .into_iter()
        .map(|(p, t)| (PathBuf::from(p), t))
        .collect();
        let read = |p: &Path| files.get(p).map(|t| (*t).to_string());
        let idx = build(Path::new("/p/main.tex"), &read);
        let paths: Vec<&str> = idx.files.iter().map(|f| f.path.to_str().unwrap()).collect();
        assert_eq!(paths, vec!["/p/main.tex", "/p/ch/b.tex", "/p/missing.tex"]);
        assert!(idx.files[2].missing);
        assert_eq!(idx.files[1].parent, Some(0));
        let labels: Vec<(String, String)> = idx
            .labels()
            .into_iter()
            .map(|(k, _, _, s)| (k, s))
            .collect();
        assert_eq!(
            labels,
            vec![
                ("sec:a".into(), "Intro".into()),
                ("sec:b".into(), "B".into())
            ]
        );
        assert_eq!(idx.bib.len(), 1);
        assert_eq!(idx.bib[0].0.key, "knuth");
        assert_eq!(idx.commands, vec!["R"]);
        assert!(idx.cites.is_empty());
        assert_eq!(idx.items(OutlineKind::Todo).len(), 1);
        assert!(idx.words >= 7, "{}", idx.words);
        assert_eq!(idx.section_at(Path::new("/p/ch/b.tex"), 2), Some((1, 0)));
        let order: Vec<String> = idx
            .reading_order()
            .into_iter()
            .map(|(f, i)| idx.files[f].outline[i].title.clone())
            .collect();
        assert_eq!(order, vec!["Intro", "sec:a", "B", "sec:b", "later", "refs"]);
    }

    #[test]
    fn include_cycles_terminate() {
        let read = |p: &Path| {
            Some(match p.to_str()? {
                "/p/a.tex" => "\\input{b}".to_string(),
                _ => "\\input{a}".to_string(),
            })
        };
        let idx = build(Path::new("/p/a.tex"), &read);
        assert_eq!(idx.files.len(), 2);
    }
}
