//! CJK text in documents that do not set it up themselves: TexCraft loads xeCJK right after
//! `\documentclass` (through code run before the document, never by editing it) with Chinese
//! fonts the system has, so XeLaTeX can typeset the text.

use std::path::Path;
use std::process::Command;

/// Fonts for CJK text, by the names XeTeX looks them up with (or file names on the TeX path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CjkFonts {
    /// Body text (a Song/Ming serif face).
    pub serif: String,
    /// Bold body text, when the face has no bold sibling XeTeX finds by itself.
    pub serif_bold: Option<String>,
    /// Sans-serif and monospaced text (a Hei/Gothic face).
    pub sans: String,
}

const SERIF: &[(&str, &[&str])] = &[
    (
        "Songti SC",
        &[
            "/System/Library/Fonts/Supplemental/Songti.ttc",
            "/Library/Fonts/Songti.ttc",
        ],
    ),
    ("SimSun", &["C:\\Windows\\Fonts\\simsun.ttc"]),
    (
        "Noto Serif CJK SC",
        &[
            "/usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSerifCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSerifCJK-Regular.ttc",
        ],
    ),
];

const SANS: &[(&str, &[&str])] = &[
    (
        "Hiragino Sans GB",
        &["/System/Library/Fonts/Hiragino Sans GB.ttc"],
    ),
    ("STHeiti", &["/System/Library/Fonts/STHeiti Light.ttc"]),
    ("Microsoft YaHei", &["C:\\Windows\\Fonts\\msyh.ttc"]),
    (
        "Noto Sans CJK SC",
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        ],
    ),
];

fn first_installed(list: &[(&str, &[&str])]) -> Option<String> {
    list.iter()
        .find(|(_, paths)| paths.iter().any(|p| Path::new(p).is_file()))
        .map(|(name, _)| (*name).to_string())
}

/// Finds CJK fonts: system fonts first, then TeX Live's Fandol fonts (via `kpsewhich`).
pub fn detect(kpsewhich: Option<&Path>) -> Option<CjkFonts> {
    let serif = first_installed(SERIF);
    let sans = first_installed(SANS);
    match (serif, sans) {
        (Some(serif), sans) => Some(CjkFonts {
            sans: sans.unwrap_or_else(|| serif.clone()),
            serif,
            serif_bold: None,
        }),
        (None, Some(sans)) => Some(CjkFonts {
            serif: sans.clone(),
            serif_bold: None,
            sans,
        }),
        (None, None) => {
            let found = Command::new(kpsewhich?)
                .arg("FandolSong-Regular.otf")
                .output()
                .ok()
                .is_some_and(|o| o.status.success() && !o.stdout.is_empty());
            found.then(|| CjkFonts {
                serif: "FandolSong-Regular.otf".into(),
                serif_bold: Some("FandolSong-Bold.otf".into()),
                sans: "FandolHei-Regular.otf".into(),
            })
        }
    }
}

/// Code run before the document (`latexmk -usepretex`, or ahead of `\input` on the engine's
/// command line): loads xeCJK after the document class unless the class already did.
pub fn pretex(fonts: &CjkFonts) -> String {
    format!(
        "\\ifdefined\\AddToHook\\AddToHook{{class/after}}{{\\ifdefined\\xeCJKsetup\\else\\RequirePackage{{xeCJK}}{}\\fi}}\\fi",
        font_setup(fonts)
    )
}

/// The font part of [`pretex`] (`\setCJKmainfont` …): it loads system fonts, so with a
/// precompiled preamble it runs at the start of each pass, after the format (see `live`).
pub fn font_setup(fonts: &CjkFonts) -> String {
    let main = match &fonts.serif_bold {
        Some(b) => format!("\\setCJKmainfont[BoldFont={b}]{{{}}}", fonts.serif),
        None => format!("\\setCJKmainfont{{{}}}", fonts.serif),
    };
    format!(
        "{main}\\setCJKsansfont{{{s}}}\\setCJKmonofont{{{s}}}",
        s = fonts.sans
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretex_loads_xecjk_after_the_class() {
        let f = CjkFonts {
            serif: "Songti SC".into(),
            serif_bold: None,
            sans: "Hiragino Sans GB".into(),
        };
        let p = pretex(&f);
        assert!(p.starts_with("\\ifdefined\\AddToHook\\AddToHook{class/after}{"));
        assert!(p.contains("\\RequirePackage{xeCJK}\\setCJKmainfont{Songti SC}"));
        assert!(p.contains("\\setCJKsansfont{Hiragino Sans GB}"));
        assert_eq!(p.matches('{').count(), p.matches('}').count());
        let fandol = CjkFonts {
            serif: "FandolSong-Regular.otf".into(),
            serif_bold: Some("FandolSong-Bold.otf".into()),
            sans: "FandolHei-Regular.otf".into(),
        };
        assert!(
            pretex(&fandol)
                .contains("\\setCJKmainfont[BoldFont=FandolSong-Bold.otf]{FandolSong-Regular.otf}")
        );
    }
}
