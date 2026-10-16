//! LaTeX-aware spell checking.
//!
//! [`words`] picks the prose words out of a document (text and comments; never commands, maths,
//! labels, citation keys, verbatim, file names or URLs). [`WordList`] decides whether a word is
//! spelled correctly from a plain word list (the system's `/usr/share/dict/words` and the user's
//! learned words) with inflection rules, since such lists hold base forms ("file", not "files").
//! The app confirms words the list rejects with the platform's checker when there is one.

use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;

use texcraft_syntax::{Span, TokenKind};

/// Commands whose arguments are names, options or code rather than prose.
const NON_PROSE: &[&str] = &[
    "usepackage",
    "RequirePackage",
    "documentclass",
    "LoadClass",
    "input",
    "include",
    "includeonly",
    "includegraphics",
    "includepdf",
    "includesvg",
    "bibliography",
    "bibliographystyle",
    "addbibresource",
    "newcommand",
    "renewcommand",
    "providecommand",
    "newenvironment",
    "renewenvironment",
    "DeclareMathOperator",
    "usetikzlibrary",
    "usepgfplotslibrary",
    "url",
    "href",
    "hypersetup",
    "geometry",
    "pagestyle",
    "thispagestyle",
    "pagenumbering",
    "setlength",
    "addtolength",
    "setcounter",
    "addtocounter",
    "color",
    "textcolor",
    "colorbox",
    "definecolor",
    "newtheorem",
    "theoremstyle",
    "lstset",
    "lstinputlisting",
    "inputminted",
    "graphicspath",
    "selectlanguage",
    "setmainfont",
    "setsansfont",
    "setmonofont",
    "setCJKmainfont",
    "fontfamily",
    "fontsize",
    "nocite",
    "bibitem",
    "newcounter",
    "numberwithin",
    "captionsetup",
    "setlist",
    "newlist",
    "crefname",
    "Crefname",
    "pgfplotsset",
    "tikzset",
    "usetheme",
    "usecolortheme",
    "usefonttheme",
    "setbeamertemplate",
    "setbeamercolor",
    "setbeamerfont",
    "jnlcitation",
];

/// Environments whose first braced argument is a column specification.
const COLUMN_ENVS: &[&str] = &[
    "tabular",
    "tabular*",
    "tabularx",
    "array",
    "longtable",
    "tabu",
    "supertabular",
    "xltabular",
];

/// Byte ranges that hold no prose: option lists in `[...]`, arguments of [`NON_PROSE`]
/// commands and table column specifications.
fn excluded(text: &str, spans: &[Span]) -> Vec<Range<usize>> {
    use texcraft_syntax::outline::{braced_argument, commands, skip_optional};
    let mut out: Vec<Range<usize>> = Vec::new();
    // [...] groups (optional arguments; rare in prose).
    let mut open: Option<usize> = None;
    let mut depth = 0u32;
    for sp in spans {
        if sp.kind != TokenKind::Bracket {
            continue;
        }
        match text.as_bytes().get(sp.range.start) {
            Some(b'[') => {
                if depth == 0 {
                    open = Some(sp.range.end);
                }
                depth += 1;
            }
            Some(b']') if depth > 0 => {
                depth -= 1;
                if depth == 0
                    && let Some(o) = open.take()
                {
                    out.push(o..sp.range.start);
                }
            }
            _ => {}
        }
    }
    for (offset, name) in commands(text) {
        let mut p = offset + 1 + name.len();
        if text.as_bytes().get(p) == Some(&b'*') {
            p += 1;
        }
        if NON_PROSE.contains(&name) {
            if let Some(q) = skip_optional(text, p)
                && let Some((arg, end)) = braced_argument(text, q)
            {
                out.push(end - 1 - arg.len()..end);
            }
        } else if name == "begin"
            && let Some((env, end)) = braced_argument(text, p)
            && COLUMN_ENVS.contains(&env.trim())
        {
            // \begin{tabular*}{width}{cols}: skip the width too.
            let mut q = end;
            if (env.trim() == "tabular*" || env.trim() == "tabularx")
                && let Some((w, e)) = braced_argument(text, q)
            {
                out.push(e - 1 - w.len()..e);
                q = e;
            }
            if let Some(q) = skip_optional(text, q)
                && let Some((cols, e)) = braced_argument(text, q)
            {
                out.push(e - 1 - cols.len()..e);
            }
        }
    }
    out.sort_by_key(|r| r.start);
    out
}

/// Byte ranges of the words worth checking.
pub fn words(text: &str, spans: &[Span]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let skip_ranges = excluded(text, spans);
    let in_excluded = |pos: usize| -> bool {
        let i = skip_ranges.partition_point(|r| r.start <= pos);
        skip_ranges
            .get(..i)
            .is_some_and(|before| before.iter().rev().take(8).any(|r| r.contains(&pos)))
    };
    for span in spans {
        if !matches!(span.kind, TokenKind::Text | TokenKind::Comment) {
            continue;
        }
        let mut i = span.range.start;
        let end = span.range.end.min(text.len());
        while i < end {
            let Some(&b) = bytes.get(i) else { break };
            if !b.is_ascii_alphabetic() {
                // Skip non-letters, and whole non-ASCII words (CJK, accented text).
                if b >= 0x80 {
                    while bytes
                        .get(i)
                        .is_some_and(|c| *c >= 0x80 || c.is_ascii_alphabetic())
                        && i < end
                    {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
                continue;
            }
            let start = i;
            while i < end
                && bytes.get(i).is_some_and(|c| {
                    c.is_ascii_alphabetic()
                        || (*c == b'\'' && bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic))
                })
            {
                i += 1;
            }
            let before = start.checked_sub(1).and_then(|k| bytes.get(k)).copied();
            let after = bytes.get(i).copied();
            // Commands inside comments, identifiers, paths, URLs, e-mail, non-ASCII neighbours.
            let skip = matches!(
                before,
                Some(b'\\' | b'@' | b'_' | b'/' | b':' | b'.' | b'$' | b'#')
            ) || matches!(after, Some(b'_' | b'@' | b'/' | b':' | b'{' | b'('))
                || after.is_some_and(|c| c >= 0x80 || c.is_ascii_digit())
                || before.is_some_and(|c| c >= 0x80 || c.is_ascii_digit());
            if !skip && i - start >= 3 && !in_excluded(start) {
                out.push(start..i);
            }
        }
    }
    out
}

/// Words a technical writer uses that general word lists lack.
const EXTRA_WORDS: &[&str] = &[
    "latex",
    "tex",
    "bibtex",
    "biber",
    "biblatex",
    "pdflatex",
    "xelatex",
    "lualatex",
    "tectonic",
    "synctex",
    "texifier",
    "texcraft",
    "overleaf",
    "pdf",
    "pdfs",
    "url",
    "urls",
    "email",
    "emails",
    "online",
    "website",
    "websites",
    "dataset",
    "datasets",
    "preprint",
    "preprints",
    "arxiv",
    "doi",
    "isbn",
    "github",
    "gitlab",
    "png",
    "jpeg",
    "jpg",
    "svg",
    "eps",
    "unicode",
    "utf",
    "tikz",
    "pgf",
    "beamer",
    "amsmath",
    "macos",
    "ios",
    "linux",
    "windows",
    "rust",
    "python",
    "keyword",
    "keywords",
    "workflow",
    "workflows",
    "toolchain",
    "toolchains",
    "typeset",
    "typesets",
    "typesetter",
    "typesetters",
    "preview",
    "previews",
    "subsection",
    "subsections",
    "subsubsection",
    "eq",
    "eqs",
    "fig",
    "figs",
    "sec",
    "ref",
    "refs",
    "etc",
    "et",
    "al",
    "vs",
    "ok",
    "app",
    "apps",
    "cpu",
    "gpu",
    "api",
    "apis",
    "html",
    "css",
    "json",
    "xml",
    "csv",
    "sql",
    "runtime",
    "metadata",
    "codebase",
    "username",
    "login",
    "plugin",
    "plugins",
    "hyperlink",
    "hyperlinks",
    "pseudocode",
    "subfigure",
    "subfigures",
    "appendices",
    "lemma",
    "lemmas",
    "corollary",
    "corollaries",
    "theorem",
    "theorems",
    "eigenvalue",
    "eigenvalues",
    "eigenvector",
    "eigenvectors",
    "nonlinear",
    "nonzero",
    "nonnegative",
    "lineno",
];

/// A word list with inflection rules.
#[derive(Debug, Clone, Default)]
pub struct WordList {
    words: HashSet<String>,
}

impl WordList {
    /// The built-in technical words only.
    pub fn builtin() -> Self {
        let mut w = Self::default();
        for e in EXTRA_WORDS {
            w.words.insert((*e).to_string());
        }
        w
    }

    /// The built-in words plus the system word lists and the user's learned words that exist.
    pub fn system() -> Self {
        let mut w = Self::builtin();
        for p in [
            "/usr/share/dict/words",
            "/usr/share/dict/propernames",
            "/usr/share/dict/american-english",
            "/usr/share/dict/british-english",
        ] {
            w.load(Path::new(p));
        }
        if let Some(home) = std::env::var_os("HOME") {
            w.load(&Path::new(&home).join("Library/Spelling/LocalDictionary"));
        }
        w
    }

    /// Adds the words of a one-word-per-line file (missing files are ignored).
    pub fn load(&mut self, path: &Path) {
        if let Ok(text) = std::fs::read_to_string(path) {
            self.words.extend(
                text.lines()
                    .map(|l| l.trim().to_ascii_lowercase())
                    .filter(|l| !l.is_empty()),
            );
        }
    }

    /// Adds one word.
    pub fn add(&mut self, word: &str) {
        self.words.insert(word.to_ascii_lowercase());
    }

    /// Number of words.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// No words loaded.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    fn has(&self, w: &str) -> bool {
        self.words.contains(w)
    }

    /// Whether `word` is spelled correctly (or not worth flagging: acronyms, CamelCase).
    pub fn is_correct(&self, word: &str) -> bool {
        if word.len() < 3 || word.bytes().all(|b| b.is_ascii_uppercase()) {
            return true;
        }
        // Internal capitals (LaTeX, iPhone, McDonald) are names, not typos.
        if word.chars().skip(1).any(|c| c.is_ascii_uppercase()) {
            return true;
        }
        let lower = word.to_ascii_lowercase();
        // Contractions and possessives: check the base word.
        let mut w = lower.as_str();
        for c in ["n't", "'s", "'re", "'ll", "'ve", "'d", "'m"] {
            if let Some(base) = w.strip_suffix(c) {
                w = base;
                break;
            }
        }
        let w = w.trim_end_matches('\'');
        if w.len() < 3 || self.has(w) {
            return true;
        }
        self.candidates(w).iter().any(|c| self.has(c))
    }

    /// Base forms to try for an inflected or British-spelled word.
    fn candidates(&self, w: &str) -> Vec<String> {
        let mut c: Vec<String> = Vec::new();
        fn push(c: &mut Vec<String>, s: String) {
            if s.len() >= 2 {
                c.push(s);
            }
        }
        let undouble = |stem: &str| -> Option<String> {
            let b = stem.as_bytes();
            let n = b.len();
            (n >= 2 && b.get(n - 1) == b.get(n - 2))
                .then(|| stem.get(..n - 1).unwrap_or(stem).to_string())
        };
        for (suffix, repl) in [
            ("ies", "y"),
            ("ied", "y"),
            ("ier", "y"),
            ("iest", "y"),
            ("ily", "y"),
            ("ses", "s"),
            ("xes", "x"),
            ("ches", "ch"),
            ("shes", "sh"),
            ("es", ""),
            ("s", ""),
            ("ed", ""),
            ("ed", "e"),
            ("d", ""),
            ("ing", ""),
            ("ing", "e"),
            ("ly", ""),
            ("ally", ""),
            ("er", ""),
            ("er", "e"),
            ("ers", ""),
            ("ers", "e"),
            ("est", ""),
            ("ness", ""),
            ("ment", ""),
            ("ments", ""),
            ("able", ""),
            ("able", "e"),
            ("ability", "able"),
            ("ation", "e"),
            ("ations", "e"),
            ("ation", ""),
            ("ations", ""),
            ("ion", "e"),
            ("ions", "e"),
            ("ion", ""),
            ("ions", ""),
            ("ive", "e"),
            ("ity", ""),
            ("al", ""),
            ("ize", ""),
            ("izes", ""),
            ("ized", ""),
            ("izing", ""),
            ("ization", "ize"),
        ] {
            if let Some(stem) = w.strip_suffix(suffix) {
                push(&mut c, format!("{stem}{repl}"));
                if repl.is_empty()
                    && let Some(u) = undouble(stem)
                {
                    push(&mut c, u);
                }
            }
        }
        // British → American: colour, behaviour, centre, organise, analyse, modelling.
        if w.contains("our") {
            push(&mut c, w.replacen("our", "or", 1));
        }
        if let Some(stem) = w.strip_suffix("re") {
            push(&mut c, format!("{stem}er"));
        }
        for (gb, us) in [
            ("ise", "ize"),
            ("ised", "ized"),
            ("ises", "izes"),
            ("ising", "izing"),
            ("isation", "ization"),
            ("yse", "yze"),
            ("ysed", "yzed"),
            ("lled", "led"),
            ("lling", "ling"),
            ("ogue", "og"),
        ] {
            if let Some(stem) = w.strip_suffix(gb) {
                push(&mut c, format!("{stem}{us}"));
            }
        }
        // Prefixes: non-, re-, un-, pre-, sub-, multi-.
        for p in [
            "non", "re", "un", "pre", "sub", "multi", "co", "de", "in", "over", "under",
        ] {
            if let Some(rest) = w.strip_prefix(p)
                && rest.len() >= 3
            {
                push(&mut c, rest.trim_start_matches('-').to_string());
            }
        }
        // Inflected compound of the candidates (e.g. "reconfigured" -> "configure").
        let first: Vec<String> = c.clone();
        for base in first {
            for (suffix, repl) in [
                ("s", ""),
                ("ed", ""),
                ("ed", "e"),
                ("ing", ""),
                ("ing", "e"),
                ("d", ""),
            ] {
                if let Some(stem) = base.strip_suffix(suffix) {
                    push(&mut c, format!("{stem}{repl}"));
                }
            }
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texcraft_syntax::highlight;

    fn picked(text: &str) -> Vec<&str> {
        words(text, &highlight(text))
            .into_iter()
            .map(|r| &text[r])
            .collect()
    }

    #[test]
    fn picks_prose_only() {
        let t = "Hello \\emph{wrold} $x + speling$ \\cite{knuth} % a comment wiht \\documentclass[referee]{sn-jnl}\nsee fig.~3 and file.tex, http://example.com/path 中文 naïve x_abc";
        let w = picked(t);
        assert!(
            w.contains(&"Hello")
                && w.contains(&"wrold")
                && w.contains(&"wiht")
                && w.contains(&"referee")
        );
        assert!(!w.contains(&"emph") && !w.contains(&"speling") && !w.contains(&"knuth"));
        assert!(!w.contains(&"documentclass") && !w.contains(&"tex") && !w.contains(&"example"));
        assert!(
            !w.iter().any(|x| x.contains("na")),
            "accented words are skipped: {w:?}"
        );
        assert!(!w.contains(&"abc"));
    }

    #[test]
    fn skips_options_and_package_names() {
        let t = "\\usepackage[utf8]{inputenc,amssymb}\\begin{figure}[htbp]\n\\begin{tabular}{lrr} Prose wrold \\end{tabular}\n\\section{Introducton}\\href{http://x.org/foo}{Click heer}";
        let w = picked(t);
        assert!(w.contains(&"Prose") && w.contains(&"wrold"), "{w:?}");
        assert!(
            w.contains(&"Introducton") && w.contains(&"heer"),
            "section titles and link text are prose: {w:?}"
        );
        for not in ["utf", "inputenc", "amssymb", "htbp", "lrr", "foo"] {
            assert!(!w.contains(&not), "{not} in {w:?}");
        }
    }

    #[test]
    fn inflections_and_british_spelling() {
        let mut l = WordList::builtin();
        for b in [
            "file",
            "author",
            "embed",
            "submit",
            "figure",
            "equation",
            "include",
            "reference",
            "contribute",
            "color",
            "behavior",
            "organize",
            "center",
            "study",
            "box",
            "model",
            "configure",
        ] {
            l.add(b);
        }
        for ok in [
            "files",
            "authors",
            "embedded",
            "submitted",
            "figures",
            "equations",
            "including",
            "referencing",
            "contributed",
            "colour",
            "behaviour",
            "organised",
            "centre",
            "studies",
            "boxes",
            "modelling",
            "reconfigured",
            "LaTeX",
            "NASA",
            "keywords",
            "it's",
        ] {
            assert!(l.is_correct(ok), "{ok}");
        }
        for bad in ["wrold", "speling", "jnl", "recieve"] {
            assert!(!l.is_correct(bad), "{bad}");
        }
    }

    #[test]
    fn system_list_loads_when_present() {
        let l = WordList::system();
        if Path::new("/usr/share/dict/words").exists() {
            assert!(l.len() > 100_000);
            assert!(
                l.is_correct("introduction") && l.is_correct("submitted") && !l.is_correct("teh")
            );
        }
    }

    #[test]
    fn garbage_never_panics() {
        let l = WordList::builtin();
        for t in [
            "", "'", "''a", "a'", "\\", "ééé", "x\u{0}y", "s", "ies", "ed",
        ] {
            let _ = picked(t);
            let _ = l.is_correct(t);
        }
    }
}
