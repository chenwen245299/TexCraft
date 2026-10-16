//! A pure-Rust SyncTeX reader.
//!
//! Loads the `.synctex` / `.synctex.gz` file a TeX engine writes with `-synctex=1` and answers
//! the two questions an editor asks: where in the PDF is this source line (forward search), and
//! which source line produced this point on the page (inverse search). Never panics on any input.
//!
//! The answers follow the reference `synctex` command-line tool: [`SyncTex::forward_with_hint`]
//! returns the records `synctex view` prints, in the same order, and
//! [`SyncTex::inverse_candidates`] the ones `synctex edit` prints. Coordinates are PDF points
//! with the origin at the top-left corner of the page.
//!
//! Truncated files are accepted: everything up to the damage stays searchable.

mod parser;
mod search;

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Errors from loading a SyncTeX file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file could not be read.
    #[error("cannot read SyncTeX file: {0}")]
    Io(#[from] std::io::Error),
    /// The content is not valid SyncTeX.
    #[error("malformed SyncTeX data: {0}")]
    Format(String),
}

/// A rectangle on a PDF page, in PDF points (1/72 in) with the origin at the **top-left** corner
/// of the page and y growing downwards (the convention a page viewer draws in).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PdfRect {
    /// 1-based page number.
    pub page: u32,
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width (>= 0).
    pub width: f32,
    /// Height (>= 0).
    pub height: f32,
}

/// A position in a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    /// The input file as recorded by the engine, with `./` segments removed.
    pub file: PathBuf,
    /// 1-based line.
    pub line: u32,
    /// 1-based column, when the engine recorded one.
    pub column: Option<u32>,
}

/// A parsed SyncTeX file.
#[derive(Clone, Default)]
pub struct SyncTex {
    data: Arc<parser::Data>,
    /// Every recorded input as (tag, normalized path), in file order.
    inputs: Vec<(i32, PathBuf)>,
    /// Distinct input paths in first-seen order (for [`SyncTex::inputs`]).
    distinct_inputs: Vec<PathBuf>,
    /// Directory of the SyncTeX file, when it was loaded from disk.
    base_dir: Option<PathBuf>,
}

impl std::fmt::Debug for SyncTex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncTex")
            .field("pages", &self.pages())
            .field("inputs", &self.distinct_inputs.len())
            .field("nodes", &self.data.nodes.len())
            .field("base_dir", &self.base_dir)
            .finish()
    }
}

impl SyncTex {
    /// Parses SyncTeX data; gzip-compressed data is detected and inflated.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let text = parser::inflate(bytes)?;
        let data = parser::parse(&text)?;
        let inputs: Vec<(i32, PathBuf)> = data
            .inputs
            .iter()
            .map(|i| (i.tag, normalize(Path::new(&i.name))))
            .collect();
        let mut distinct_inputs: Vec<PathBuf> = Vec::new();
        for (_, path) in &inputs {
            if !distinct_inputs.contains(path) {
                distinct_inputs.push(path.clone());
            }
        }
        Ok(SyncTex {
            data: Arc::new(data),
            inputs,
            distinct_inputs,
            base_dir: None,
        })
    }

    /// Reads and parses a `.synctex` or `.synctex.gz` file.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let mut synctex = Self::from_bytes(&std::fs::read(path)?)?;
        synctex.base_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf);
        Ok(synctex)
    }

    /// The SyncTeX file that belongs to `pdf` (`x.pdf` → `x.synctex.gz`, else `x.synctex`), if
    /// it exists.
    pub fn path_for_pdf(pdf: &Path) -> Option<PathBuf> {
        ["synctex.gz", "synctex"]
            .into_iter()
            .map(|ext| pdf.with_extension(ext))
            .find(|p| p.is_file())
    }

    /// Every input file the engine recorded.
    pub fn inputs(&self) -> Vec<&Path> {
        self.distinct_inputs.iter().map(PathBuf::as_path).collect()
    }

    /// Number of pages (sheets) recorded.
    pub fn pages(&self) -> u32 {
        u32::try_from(self.data.sheets.len()).unwrap_or(u32::MAX)
    }

    /// Forward search: the boxes on the page(s) produced by `line` (1-based) of `file`. File
    /// matching is tolerant: exact path, canonicalized path, path relative to the SyncTeX file's
    /// directory, then file name. Returns an empty vector when nothing matches.
    ///
    /// When `line` itself produced nothing, the nearest line that did is used (`line + 1`,
    /// `line - 1`, `line + 2`, … like `synctex view`). Identical rectangles are reported once.
    pub fn forward(&self, file: &Path, line: u32) -> Vec<PdfRect> {
        let mut out: Vec<PdfRect> = Vec::new();
        for rect in self.forward_with_hint(file, line, 0) {
            if !out.contains(&rect) {
                out.push(rect);
            }
        }
        out
    }

    /// Forward search exactly as `synctex view -i line:0:page_hint:file` reports it: one
    /// rectangle per record, best first, results on the page closest to `page_hint` (the page
    /// currently displayed, 0 if unknown) first. May contain duplicates.
    pub fn forward_with_hint(&self, file: &Path, line: u32, page_hint: u32) -> Vec<PdfRect> {
        let line = i32::try_from(line).unwrap_or(i32::MAX);
        let hint = i32::try_from(page_hint).unwrap_or(i32::MAX);
        self.tags_for(file)
            .into_iter()
            .flat_map(|tag| self.data.view(tag, line, hint))
            .filter_map(|r| {
                let page = u32::try_from(r.page).ok()?;
                let (left, right) = (r.h, r.h + r.width);
                let (top, bottom) = (r.v - r.height, r.v + r.depth);
                Some(PdfRect {
                    page,
                    x: left.min(right),
                    y: top.min(bottom),
                    width: (right - left).abs(),
                    height: (bottom - top).abs(),
                })
            })
            .collect()
    }

    /// Inverse search: the source position that produced the point (`x`, `y`) in PDF points
    /// (top-left origin) on 1-based `page`.
    pub fn inverse(&self, page: u32, x: f32, y: f32) -> Option<SourceLocation> {
        self.inverse_candidates(page, x, y).into_iter().next()
    }

    /// Inverse search with the alternatives `synctex edit` reports: when the point lies between
    /// material from two different source positions both are returned, best first.
    pub fn inverse_candidates(&self, page: u32, x: f32, y: f32) -> Vec<SourceLocation> {
        let Ok(page) = i32::try_from(page) else {
            return Vec::new();
        };
        self.data
            .edit(page, x, y)
            .into_iter()
            .filter_map(|r| {
                Some(SourceLocation {
                    file: self.name_for_tag(r.tag)?.to_path_buf(),
                    line: u32::try_from(r.line).unwrap_or(0),
                    // SyncTeX columns are 0-based, -1 when not recorded.
                    column: u32::try_from(r.column).ok().map(|c| c.saturating_add(1)),
                })
            })
            .collect()
    }

    /// The name of the input with `tag` (the last record wins, like the reference tool).
    fn name_for_tag(&self, tag: i32) -> Option<&Path> {
        self.inputs
            .iter()
            .rev()
            .find(|(t, _)| *t == tag)
            .map(|(_, p)| p.as_path())
    }

    /// Input tags matching `file`, most recently recorded first.
    fn tags_for(&self, file: &Path) -> Vec<i32> {
        let query = normalize(file);
        if query.as_os_str().is_empty() {
            return Vec::new();
        }
        // 1. Exact (modulo `./`).
        let tags = self.select(|p| *p == query);
        if !tags.is_empty() {
            return tags;
        }
        // 2. Canonicalized (symlinks such as /tmp -> /private/tmp). Only inputs with the
        //    same file name are canonicalized, to keep the I/O small.
        let resolve = |p: &Path| -> Option<PathBuf> {
            match (&self.base_dir, p.is_relative()) {
                (Some(base), true) => std::fs::canonicalize(base.join(p)).ok(),
                _ => std::fs::canonicalize(p).ok(),
            }
        };
        if let Some(canonical) = resolve(&query) {
            let name = query.file_name();
            let tags = self
                .select(|p| p.file_name() == name && resolve(p).is_some_and(|c| c == canonical));
            if !tags.is_empty() {
                return tags;
            }
        }
        // 3. Relative to the SyncTeX file's directory.
        if let Some(base) = &self.base_dir {
            let tags = self.select(|p| {
                if query.is_relative() && normalize(&base.join(&query)) == *p {
                    return true;
                }
                p.is_relative() && normalize(&base.join(p)) == query
            });
            if !tags.is_empty() {
                return tags;
            }
        }
        // 4. Longest common trailing path (at least the file name), then the same ignoring
        //    ASCII case.
        for fold in [false, true] {
            let score = |p: &Path| common_suffix(p, &query, fold);
            let best = self.inputs.iter().map(|(_, p)| score(p)).max().unwrap_or(0);
            if best > 0 {
                return self.select(|p| score(p) == best);
            }
        }
        Vec::new()
    }

    /// Distinct tags of the inputs whose path satisfies `pred`, most recent record first.
    fn select(&self, pred: impl Fn(&PathBuf) -> bool) -> Vec<i32> {
        let mut tags = Vec::new();
        for (tag, path) in self.inputs.iter().rev() {
            if !tags.contains(tag) && pred(path) {
                tags.push(*tag);
            }
        }
        tags
    }
}

/// Removes `.` components (`/a/./b.tex` → `/a/b.tex`, `./b.tex` → `b.tex`).
fn normalize(path: &Path) -> PathBuf {
    let out: PathBuf = path
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    if out.as_os_str().is_empty() {
        path.to_path_buf()
    } else {
        out
    }
}

/// Number of equal trailing normal components of `a` and `b`.
fn common_suffix(a: &Path, b: &Path, fold_case: bool) -> usize {
    let normal = |p: &Path| -> Vec<String> {
        p.components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect()
    };
    let (a, b) = (normal(a), normal(b));
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| {
            if fold_case {
                x.eq_ignore_ascii_case(y)
            } else {
                x == y
            }
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "SyncTeX Version:1
Input:1:/proj/./main.tex
Input:2:/proj/./sub/part.tex
Output:pdf
Magnification:1000
Unit:1
X Offset:0
Y Offset:0
Content:
!100
{1
[1,3:4736286,46220574:26673152,41484288,0
(1,3:4736286,10000000:20000000,655360,196608
x1,3:5000000,10000000
g1,3:6000000,10000000
k1,4:7000000,10000000:100000
x2,7:8000000,10000000
)
]
}1
Postamble:
Count:5
Post scriptum:
";

    #[test]
    fn parses_header_and_inputs() {
        let s = SyncTex::from_bytes(SAMPLE.as_bytes()).unwrap();
        assert_eq!(s.pages(), 1);
        assert_eq!(
            s.inputs(),
            vec![Path::new("/proj/main.tex"), Path::new("/proj/sub/part.tex")]
        );
        let rects = s.forward(Path::new("/proj/main.tex"), 3);
        assert!(!rects.is_empty());
        let r = rects[0];
        assert_eq!(r.page, 1);
        // 4736286 sp = 72 bp
        assert!((r.x - 72.0).abs() < 0.01, "{r:?}");
    }

    #[test]
    fn file_matching_tiers() {
        let s = SyncTex::from_bytes(SAMPLE.as_bytes()).unwrap();
        assert_eq!(s.tags_for(Path::new("/proj/./sub/part.tex")), vec![2]);
        assert_eq!(s.tags_for(Path::new("sub/part.tex")), vec![2]);
        assert_eq!(s.tags_for(Path::new("/elsewhere/sub/PART.TEX")), vec![2]);
        assert_eq!(s.tags_for(Path::new("main.tex")), vec![1]);
        assert!(s.tags_for(Path::new("nothing.tex")).is_empty());
        assert!(s.tags_for(Path::new("")).is_empty());
    }

    #[test]
    fn inverse_finds_the_line() {
        let s = SyncTex::from_bytes(SAMPLE.as_bytes()).unwrap();
        // Point inside the hbox, near the glue at h=6000000sp (~91.2bp), baseline ~152bp.
        let loc = s.inverse(1, 91.0, 150.0).unwrap();
        assert_eq!(loc.file, Path::new("/proj/main.tex"));
        assert_eq!(loc.column, None);
        assert!(s.inverse(2, 91.0, 150.0).is_none());
        assert!(s.inverse(1, f32::NAN, 150.0).is_none());
    }

    #[test]
    fn rejects_non_synctex() {
        assert!(SyncTex::from_bytes(b"").is_err());
        assert!(SyncTex::from_bytes(b"hello world").is_err());
        assert!(SyncTex::from_bytes(&[0x1f, 0x8b, 0, 1, 2]).is_err());
        assert!(SyncTex::load(Path::new("/definitely/not/here.synctex.gz")).is_err());
    }

    #[test]
    fn default_is_empty_and_safe() {
        let s = SyncTex::default();
        assert_eq!(s.pages(), 0);
        assert!(s.inputs().is_empty());
        assert!(s.forward(Path::new("a.tex"), 1).is_empty());
        assert!(s.inverse(1, 0.0, 0.0).is_none());
    }
}
