//! TexCraft's interface languages.
//!
//! English is the source language. Every user-visible string is written in English where it is
//! used and wrapped in [`tr`], or in [`trf!`] when it has `{name}` placeholders:
//!
//! ```
//! use texcraft_i18n::{Lang, tr, trf};
//! let _scope = texcraft_i18n::scoped(Lang::ZhHans);
//! assert_eq!(tr("Typeset"), "排版");
//! assert_eq!(trf!("Typesetter: {engine}", engine = "XeLaTeX"), "排版引擎：XeLaTeX");
//! assert_eq!(tr("not in the catalogue"), "not in the catalogue");
//! ```
//!
//! The English text is the key: the catalogue (`src/zh_hans/`) maps it to the translation, and
//! a string with no entry shows in English. Nothing here panics. `tests/catalogue.rs` scans the
//! workspace for `tr("…")` and `trf!("…", …)` and checks that every string has a translation,
//! that translations keep their placeholders, and that `trf!` gets exactly those arguments.
//!
//! The language in use is process-wide ([`set_language`], set by the app from the Language
//! setting); [`scoped`] overrides it on the current thread (tests, one surface in a given
//! language). Pure: no dependencies, no I/O.

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt::{Display, Write as _};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

mod zh_hans;

/// An interface language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    /// English (the source language).
    #[default]
    En,
    /// Simplified Chinese.
    ZhHans,
}

impl Lang {
    /// Every language, English first.
    pub const ALL: [Lang; 2] = [Lang::En, Lang::ZhHans];

    /// BCP 47 tag (`en`, `zh-Hans`).
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhHans => "zh-Hans",
        }
    }

    /// The language's name in itself, as language menus show it ("English", "简体中文").
    pub fn native_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::ZhHans => "简体中文",
        }
    }

    /// The language a BCP 47 or POSIX locale tag asks for, if TexCraft has it: `en`, `en-GB`,
    /// `en_US.UTF-8`, `zh-Hans`, `zh-Hans-CN`, `zh-CN`, `zh_SG`, plain `zh`. Traditional
    /// Chinese (`zh-Hant`, `zh-TW`, `zh-HK`, `zh-MO`) is not Simplified Chinese: `None`.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        // `zh_CN.UTF-8@modifier` → `zh-cn`.
        let tag = tag.split(['.', '@']).next().unwrap_or("").trim();
        let tag = tag.replace('_', "-").to_ascii_lowercase();
        let mut parts = tag.split('-');
        match parts.next()? {
            "en" => Some(Self::En),
            "zh" => {
                let rest: Vec<&str> = parts.collect();
                if rest.contains(&"hans") {
                    Some(Self::ZhHans)
                } else if rest
                    .iter()
                    .any(|p| matches!(*p, "hant" | "tw" | "hk" | "mo"))
                {
                    None
                } else {
                    Some(Self::ZhHans)
                }
            }
            _ => None,
        }
    }

    /// The first of the user's preferred languages (most preferred first) that TexCraft has,
    /// else English: how macOS picks an app's localization.
    pub fn from_preferred<S: AsRef<str>>(preferred: &[S]) -> Lang {
        preferred
            .iter()
            .find_map(|t| Self::from_tag(t.as_ref()))
            .unwrap_or_default()
    }

    fn from_u8(n: u8) -> Lang {
        match n {
            1 => Self::ZhHans,
            _ => Self::En,
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            Self::En => 0,
            Self::ZhHans => 1,
        }
    }
}

/// The user's preferred languages from POSIX environment variables (`LANGUAGE`, a `:` list,
/// then `LC_ALL`, `LC_MESSAGES`, `LANG`), most preferred first, for systems without a native
/// list. `get` reads a variable (`|k| std::env::var(k).ok()`). `C` and `POSIX` are skipped.
pub fn languages_from_env(get: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(v) = get(key) {
            for tag in v.split(':') {
                let tag = tag.trim();
                let bare = tag.split(['.', '@']).next().unwrap_or("");
                if !tag.is_empty() && bare != "C" && bare != "POSIX" {
                    out.push(tag.to_string());
                }
            }
        }
    }
    out
}

static GLOBAL: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static OVERRIDE: Cell<Option<Lang>> = const { Cell::new(None) };
}

/// The language strings are translated into on this thread: the [`scoped`] override if one is
/// active, else the process-wide language.
pub fn language() -> Lang {
    OVERRIDE
        .with(Cell::get)
        .unwrap_or_else(|| Lang::from_u8(GLOBAL.load(Ordering::Relaxed)))
}

/// Sets the process-wide language (the app does, from the Language setting).
pub fn set_language(lang: Lang) {
    GLOBAL.store(lang.to_u8(), Ordering::Relaxed);
}

/// Translates into `lang` on the current thread until the returned guard is dropped (nested
/// scopes restore the outer one). Other threads keep the process-wide language.
pub fn scoped(lang: Lang) -> Scoped {
    let previous = OVERRIDE.with(|o| o.replace(Some(lang)));
    Scoped {
        previous,
        _not_send: std::marker::PhantomData,
    }
}

/// Guard returned by [`scoped`].
#[must_use = "the language reverts when the guard is dropped"]
pub struct Scoped {
    previous: Option<Lang>,
    // The override is per thread: the guard must be dropped on the thread that made it.
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Drop for Scoped {
    fn drop(&mut self) {
        let previous = self.previous;
        OVERRIDE.with(|o| o.set(previous));
    }
}

type Table = HashMap<&'static str, &'static str>;

fn table(lang: Lang) -> Option<&'static Table> {
    match lang {
        Lang::En => None,
        Lang::ZhHans => {
            static ZH: OnceLock<Table> = OnceLock::new();
            Some(ZH.get_or_init(|| zh_hans::entries().collect()))
        }
    }
}

/// `en` in the current [`language`]; `en` itself when the catalogue has no entry for it.
pub fn tr(en: &str) -> &str {
    tr_in(language(), en)
}

/// `en` in `lang`; `en` itself when the catalogue has no entry for it.
pub fn tr_in(lang: Lang, en: &str) -> &str {
    table(lang).and_then(|t| t.get(en).copied()).unwrap_or(en)
}

/// Whether the catalogue translates `en` into `lang` (always true for English).
pub fn has(lang: Lang, en: &str) -> bool {
    table(lang).is_none_or(|t| t.contains_key(en))
}

/// Every `(English, translation)` pair of `lang`'s catalogue, in catalogue order, duplicates
/// included (for checks; empty for English).
pub fn catalogue(lang: Lang) -> Vec<(&'static str, &'static str)> {
    match lang {
        Lang::En => Vec::new(),
        Lang::ZhHans => zh_hans::entries().collect(),
    }
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The `{name}` placeholders of a template, in order (`{…}` and `\begin{document}`-style
/// braces whose content is not a plain identifier are text; so is `{document}`, which a
/// [`trf!`] without that argument leaves alone).
pub fn placeholders(template: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some((_, after)) = rest.split_once('{') {
        match after.split_once('}') {
            Some((name, tail)) if is_ident(name) => {
                out.push(name);
                rest = tail;
            }
            _ => rest = after,
        }
    }
    out
}

/// Replaces each `{name}` in `template` with the argument of that name. Placeholders without
/// an argument, and braces around anything but an identifier, are kept as they are.
pub fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('{') {
        out.push_str(before);
        match after.split_once('}') {
            Some((name, tail)) if is_ident(name) => {
                match args.iter().find(|(n, _)| *n == name) {
                    Some((_, value)) => {
                        let _ = write!(out, "{value}");
                    }
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = tail;
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Translates a string literal with `{name}` placeholders and fills them in:
/// `trf!("Renamed to {name}", name = path.display())`. Arguments are `name = expression` with
/// any `Display` value; `tests/catalogue.rs` checks they match the placeholders.
#[macro_export]
macro_rules! trf {
    ($en:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::fill(
            $crate::tr($en),
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),*],
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        for (tag, lang) in [
            ("en", Some(Lang::En)),
            ("en-GB", Some(Lang::En)),
            ("en_US.UTF-8", Some(Lang::En)),
            ("zh-Hans", Some(Lang::ZhHans)),
            ("zh-Hans-CN", Some(Lang::ZhHans)),
            ("zh-Hans-HK", Some(Lang::ZhHans)),
            ("zh-CN", Some(Lang::ZhHans)),
            ("zh_SG", Some(Lang::ZhHans)),
            ("zh_CN.UTF-8", Some(Lang::ZhHans)),
            ("zh", Some(Lang::ZhHans)),
            ("zh-Hant", None),
            ("zh-Hant-CN", None),
            ("zh-TW", None),
            ("zh_HK.UTF-8", None),
            ("ja-JP", None),
            ("", None),
            ("C", None),
            ("-", None),
            ("é", None),
        ] {
            assert_eq!(Lang::from_tag(tag), lang, "{tag}");
        }
    }

    #[test]
    fn preferred_languages() {
        let none: [&str; 0] = [];
        assert_eq!(Lang::from_preferred(&none), Lang::En);
        assert_eq!(Lang::from_preferred(&["zh-Hans-CN", "en-CN"]), Lang::ZhHans);
        assert_eq!(Lang::from_preferred(&["en-CN", "zh-Hans-CN"]), Lang::En);
        // Languages TexCraft lacks are skipped, as macOS does.
        assert_eq!(
            Lang::from_preferred(&["ja-JP", "zh-Hant-TW", "zh-Hans"]),
            Lang::ZhHans
        );
        assert_eq!(Lang::from_preferred(&["fr-FR", "de"]), Lang::En);
    }

    #[test]
    fn environment_languages() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                vars.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            languages_from_env(env(&[("LANG", "zh_CN.UTF-8")])),
            ["zh_CN.UTF-8"]
        );
        assert_eq!(
            languages_from_env(env(&[("LANGUAGE", "fr:zh_CN"), ("LANG", "en_US.UTF-8")])),
            ["fr", "zh_CN", "en_US.UTF-8"]
        );
        assert!(languages_from_env(env(&[("LANG", "C"), ("LC_ALL", "POSIX")])).is_empty());
        assert_eq!(
            Lang::from_preferred(&languages_from_env(env(&[("LANG", "zh_CN.UTF-8")]))),
            Lang::ZhHans
        );
    }

    #[test]
    fn scoped_language_is_per_thread_and_nests() {
        let base = language();
        {
            let _zh = scoped(Lang::ZhHans);
            assert_eq!(language(), Lang::ZhHans);
            assert_eq!(tr("Typeset"), "排版");
            std::thread::spawn(move || assert_eq!(language(), base))
                .join()
                .ok();
            {
                let _en = scoped(Lang::En);
                assert_eq!(tr("Typeset"), "Typeset");
            }
            assert_eq!(language(), Lang::ZhHans);
        }
        assert_eq!(language(), base);
    }

    #[test]
    fn missing_entries_fall_back_to_english() {
        let _zh = scoped(Lang::ZhHans);
        assert_eq!(tr("no such string, surely"), "no such string, surely");
        assert_eq!(tr(""), "");
        assert!(!has(Lang::ZhHans, "no such string, surely"));
        assert!(has(Lang::En, "no such string, surely"));
        assert_eq!(tr_in(Lang::En, "Typeset"), "Typeset");
    }

    #[test]
    fn filling_placeholders() {
        let n = 3;
        assert_eq!(fill("{n} files", &[("n", &n)]), "3 files");
        assert_eq!(fill("{a}{b}{a}", &[("a", &"x"), ("b", &1.5)]), "x1.5x");
        // No argument: kept. Not an identifier: text.
        assert_eq!(fill("{missing} left", &[]), "{missing} left");
        assert_eq!(
            fill("\\end{…} after \\begin{…}", &[]),
            "\\end{…} after \\begin{…}"
        );
        assert_eq!(fill("{ {n}} {", &[("n", &1)]), "{ 1} {");
        assert_eq!(fill("}{", &[]), "}{");
        assert_eq!(fill("{1x} {x-y} {}", &[]), "{1x} {x-y} {}");
        assert_eq!(
            fill("“{name}” — 已存在", &[("name", &"a.tex")]),
            "“a.tex” — 已存在"
        );
        assert_eq!(placeholders("{a} {b} {1} {…} {_c2}"), vec!["a", "b", "_c2"]);
        assert!(placeholders("\\begin{…}").is_empty());
    }

    #[test]
    fn trf_translates_then_fills() {
        let _zh = scoped(Lang::ZhHans);
        let engine = "XeLaTeX";
        assert_eq!(
            trf!("Typesetter: {engine}", engine = engine),
            "排版引擎：XeLaTeX"
        );
        let _en = scoped(Lang::En);
        assert_eq!(
            trf!("Typesetter: {engine}", engine = engine.to_lowercase(),),
            "Typesetter: xelatex"
        );
    }
}
