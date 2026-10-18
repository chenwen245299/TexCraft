//! Text selection in the PDF viewer: a press and a drag (or a double or triple click) become a
//! range of glyphs, resolved against each page's text as the worker extracts it. Also the text
//! around an inverse-search click, which lets the editor land on the clicked word.

use std::collections::HashMap;
use std::sync::Arc;

use egui::Pos2;
use texcraft_engine::TextUnit;
use texcraft_engine::sync::SyncHint;
use texcraft_pdf::{Granularity, PageText, TextPos, is_cjk_letter, range_on_page};

/// A point on a page: 0-based page, PDF points from its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PagePoint {
    /// 0-based page.
    pub page: usize,
    /// Points from the left edge.
    pub x: f32,
    /// Points from the top edge.
    pub y: f32,
}

/// A selection: where it started, where it is now, and by what unit it grows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    /// Where the press was.
    pub anchor: PagePoint,
    /// Where the pointer is (or was released).
    pub focus: PagePoint,
    /// Characters, words or lines.
    pub unit: Granularity,
}

/// The engine's unit as the PDF crate's.
pub fn granularity(unit: TextUnit) -> Granularity {
    match unit {
        TextUnit::Char => Granularity::Char,
        TextUnit::Word => Granularity::Word,
        TextUnit::Line => Granularity::Line,
    }
}

/// The selected glyphs, from `start` to `end` in document order, once the text of both ends'
/// pages is known; `None` when nothing is selected.
pub fn resolve(
    sel: &Selection,
    texts: &HashMap<usize, Arc<PageText>>,
) -> Option<(TextPos, TextPos)> {
    let span = |p: &PagePoint| -> Option<(TextPos, TextPos)> {
        let text = texts.get(&p.page)?;
        let at = |index| TextPos {
            page: p.page,
            index,
        };
        let range = match sel.unit {
            Granularity::Char => {
                let c = text.caret_at(p.x, p.y);
                c..c
            }
            Granularity::Word => text.word_at(text.glyph_at(p.x, p.y)?),
            Granularity::Line => text.line_range(text.glyph_at(p.x, p.y)?),
        };
        Some((at(range.start), at(range.end)))
    };
    let a = span(&sel.anchor)?;
    let f = span(&sel.focus)?;
    let (start, end) = (a.0.min(f.0), a.1.max(f.1));
    (start < end).then_some((start, end))
}

/// Glyphs of page `page` inside the selection `start..end`.
pub fn page_range(
    (start, end): (TextPos, TextPos),
    page: usize,
    text: &PageText,
) -> std::ops::Range<usize> {
    range_on_page(start, end, page, text.len())
}

/// The press count for double and triple clicks: presses within half a second and a few points
/// of the last one count up (1, 2, 3, then 1 again).
pub fn press_count(last: Option<(f64, Pos2, u8)>, time: f64, pos: Pos2) -> u8 {
    match last {
        Some((t, p, n)) if time - t < 0.5 && time >= t && p.distance(pos) < 5.0 && n < 3 => n + 1,
        _ => 1,
    }
}

/// The unit a press of `count` selects by.
pub fn unit_for(count: u8) -> Granularity {
    match count {
        2 => Granularity::Word,
        3 => Granularity::Line,
        _ => Granularity::Char,
    }
}

/// The PDF text around a click, for inverse search: the word (a single character in CJK text)
/// and a few characters on either side on its line. `None` when the click is not on text.
pub fn sync_hint(text: &PageText, x: f32, y: f32) -> Option<SyncHint> {
    let line = text.lines.get(text.line_at(x, y, 2.0)?)?.glyphs.clone();
    let i = text.glyph_at(x, y)?;
    let g = text.glyphs.get(i)?;
    let cjk = g.text.chars().next().is_some_and(is_cjk_letter);
    let w = if cjk { i..i + 1 } else { text.word_at(i) };
    let spaced = |k: usize| text.glyphs.get(k).is_some_and(|g| g.space_before);
    let mut before = text.text(line.start.max(w.start.saturating_sub(8))..w.start);
    if spaced(w.start) && !before.is_empty() {
        before.push(' ');
    }
    let mut after = String::new();
    if w.end < line.end {
        if spaced(w.end) {
            after.push(' ');
        }
        after.push_str(&text.text(w.end..line.end.min(w.end + 8)));
    }
    Some(SyncHint {
        word: text.text(w.clone()),
        offset: text.text(w.start..i).chars().count(),
        before,
        after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(name: &str) -> Arc<PageText> {
        let path = format!("{}/../pdf/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
        let data = Arc::new(std::fs::read(path).unwrap());
        Arc::new(texcraft_pdf::page_text(data, 0).unwrap())
    }

    fn pt(page: usize, x: f32, y: f32) -> PagePoint {
        PagePoint { page, x, y }
    }

    #[test]
    fn selections_resolve_to_text() {
        let mut texts = HashMap::new();
        texts.insert(0, load("select-en.pdf"));
        texts.insert(1, load("select-zh.pdf"));
        let text_of = |sel: Selection| {
            let r = resolve(&sel, &texts)?;
            Some(texcraft_pdf::selected_text(r.0, r.1, |p| {
                texts.get(&p).map(AsRef::as_ref)
            }))
        };
        let drag = |a, b| Selection {
            anchor: a,
            focus: b,
            unit: Granularity::Char,
        };
        // "here." (pdftotext: x 193.39–216.45, y 157.5–167.2), dragged either way.
        assert_eq!(
            text_of(drag(pt(0, 193.6, 162.0), pt(0, 216.4, 162.0))).as_deref(),
            Some("here.")
        );
        assert_eq!(
            text_of(drag(pt(0, 216.4, 162.0), pt(0, 193.6, 162.0))).as_deref(),
            Some("here.")
        );
        // A click selects nothing.
        assert_eq!(
            text_of(drag(pt(0, 200.0, 162.0), pt(0, 200.0, 162.0))),
            None
        );
        // Double-click: the word; dragging on extends by words.
        let word = |a, b| Selection {
            anchor: a,
            focus: b,
            unit: Granularity::Word,
        };
        assert_eq!(
            text_of(word(pt(0, 260.0, 162.0), pt(0, 260.0, 162.0))).as_deref(),
            Some("office")
        );
        assert_eq!(
            text_of(word(pt(0, 260.0, 162.0), pt(0, 290.0, 162.0))).as_deref(),
            Some("office staff")
        );
        // Triple-click: the line.
        let line = Selection {
            anchor: pt(1, 200.0, 160.0),
            focus: pt(1, 200.0, 160.0),
            unit: Granularity::Line,
        };
        assert_eq!(
            text_of(line).as_deref(),
            Some("测试实时编译预览。你好，世界！")
        );
        // Across pages: from "Method" to "引言".
        let across = drag(pt(0, 150.0, 268.0), pt(1, 333.0, 131.0));
        assert_eq!(
            text_of(across).as_deref(),
            Some("Method\nSecond paragraph with a hyphen-ated word and the end.\n1 引言")
        );
        // A page whose text has not arrived: nothing yet.
        assert_eq!(text_of(drag(pt(0, 150.0, 268.0), pt(2, 10.0, 10.0))), None);
    }

    #[test]
    fn presses_count_up_to_triple() {
        let p = Pos2::new(10.0, 10.0);
        assert_eq!(press_count(None, 1.0, p), 1);
        assert_eq!(press_count(Some((1.0, p, 1)), 1.3, p), 2);
        assert_eq!(press_count(Some((1.0, p, 2)), 1.3, p), 3);
        assert_eq!(press_count(Some((1.0, p, 3)), 1.3, p), 1);
        // Too late or too far: a new first click.
        assert_eq!(press_count(Some((1.0, p, 1)), 1.6, p), 1);
        assert_eq!(
            press_count(Some((1.0, p, 1)), 1.2, Pos2::new(20.0, 10.0)),
            1
        );
        assert_eq!(unit_for(2), Granularity::Word);
        assert_eq!(unit_for(3), Granularity::Line);
        assert_eq!(unit_for(1), Granularity::Char);
    }

    #[test]
    fn inverse_search_hints() {
        let en = load("select-en.pdf");
        // On "office" (x 247.88–272.14), the third letter.
        let h = sync_hint(&en, 256.0, 162.0).unwrap();
        assert_eq!(h.word, "office");
        assert!(h.before.ends_with("The "), "{h:?}");
        assert!(h.after.starts_with(" staff"), "{h:?}");
        assert!(h.offset >= 1 && h.offset <= 3, "{h:?}");
        // Off the text: no hint.
        assert_eq!(sync_hint(&en, 50.0, 50.0), None);
        // CJK: the clicked character with its neighbours.
        let zh = load("select-zh.pdf");
        let x = 154.768 + 10.5 * 9.5; // "你"
        let h = sync_hint(&zh, x, 160.0).unwrap();
        assert_eq!(h.word, "你");
        assert_eq!(h.before, "试实时编译预览。");
        assert_eq!(h.after, "好，世界！");
        assert_eq!(h.offset, 0);
    }
}
