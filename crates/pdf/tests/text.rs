//! Text extraction and hit-testing against real PDFs from the TeX toolchain
//! (`tests/data/generate.py`). Word positions come from poppler's `pdftotext -bbox` on the same
//! files.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use texcraft_pdf::{
    Granularity, PageText, PdfError, RenderEvent, RenderWorker, TextPos, page_text, selected_text,
};

fn load(name: &str) -> Arc<Vec<u8>> {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    Arc::new(std::fs::read(path).unwrap())
}

fn lines(text: &PageText) -> Vec<String> {
    text.lines
        .iter()
        .map(|l| text.text(l.glyphs.clone()))
        .collect()
}

/// The text a drag from one point to another selects.
fn drag(text: &PageText, from: (f32, f32), to: (f32, f32)) -> String {
    let a = text.caret_at(from.0, from.1);
    let b = text.caret_at(to.0, to.1);
    text.text(a.min(b)..a.max(b))
}

fn word(text: &PageText, x: f32, y: f32) -> String {
    let i = text.glyph_at(x, y).unwrap();
    text.text(text.word_at(i))
}

#[test]
fn english_lines_words_and_ligatures() {
    let text = page_text(load("select-en.pdf"), 0).unwrap();
    let lines = lines(&text);
    for want in [
        "1 Introduction",
        "Start writing here. The office staff find selection difficult. Equations are",
        "numbered automatically:",
        "Refer back to Section 2.",
        "2 Method",
        "Second paragraph with a hyphen-ated word and the end.",
    ] {
        assert!(
            lines.iter().any(|l| l == want),
            "{want:?} not in {lines:#?}"
        );
    }
    // The display equation: its parts are there (exponents and limits make lines of their own).
    let all = lines.join("\n");
    assert!(all.contains("f(x) ="), "{all}");
    assert!(all.contains("(1)"), "{all}");

    // Ligature glyphs (ffi, ff, fi) spell out their letters.
    assert_eq!(word(&text, 260.0, 162.0), "office");
    assert_eq!(word(&text, 287.0, 162.0), "staff");
    assert_eq!(word(&text, 390.0, 162.0), "difficult");
    // Punctuation stands alone; a hyphen splits words.
    assert_eq!(word(&text, 214.5, 162.0), ".");
    assert_eq!(word(&text, 260.0, 293.0), "hyphen");
}

#[test]
fn english_drag_selection() {
    let text = page_text(load("select-en.pdf"), 0).unwrap();
    // pdftotext: "here." spans x 193.39–216.45 on the line y 157.5–167.2.
    assert_eq!(drag(&text, (193.6, 162.0), (216.4, 162.0)), "here.");
    // Backwards drags select the same.
    assert_eq!(drag(&text, (216.4, 162.0), (193.6, 162.0)), "here.");
    // In the gap after "Start" the caret goes after it.
    assert_eq!(drag(&text, (120.0, 162.0), (152.0, 162.0)), "Start");
    // Across a line break: "Equations are" + "numbered".
    assert_eq!(
        drag(&text, (418.0, 162.0), (172.5, 175.9)),
        "Equations are\nnumbered"
    );
    // Past the right end of a line: up to its end.
    assert_eq!(drag(&text, (418.0, 162.0), (560.0, 162.0)), "Equations are");
    // A point between lines picks the nearer one.
    assert_eq!(drag(&text, (125.0, 169.0), (172.5, 169.0)), "numbered");

    // One highlight box per line, as tall as the line.
    let a = text.caret_at(418.0, 162.0);
    let b = text.caret_at(172.5, 175.9);
    let rects = text.rects(a..b);
    assert_eq!(rects.len(), 2);
    assert!((rects[0].x0 - 417.4).abs() < 1.0, "{rects:?}");
    assert!((rects[0].x1 - 484.45).abs() < 1.0, "{rects:?}");
    assert!((rects[1].x0 - 125.8).abs() < 1.0, "{rects:?}");
    assert!((rects[1].x1 - 172.8).abs() < 1.0, "{rects:?}");
    assert!(rects[0].y1 <= rects[1].y0 + 0.5, "{rects:?}");
    assert!(
        rects[0].height() > 9.0 && rects[0].height() < 13.0,
        "{rects:?}"
    );
}

#[test]
fn chinese_lines_and_selection() {
    let text = page_text(load("select-zh.pdf"), 0).unwrap();
    assert_eq!(
        lines(&text),
        [
            "1 引言",
            "测试实时编译预览。你好，世界！",
            "中文与 English 混排的句子，结束。",
            "这是一个很长的段落，用来测试换行的中文文本在复制时不会被拆成",
            "两行。这是一个很长的段落，用来测试换行的中文文本在复制时不会被拆成",
            "两行。",
        ]
    );
    // A wrapped CJK paragraph copies as one line (no space or break where it wrapped); a new
    // paragraph still starts on a line of its own.
    let para = "这是一个很长的段落，用来测试换行的中文文本在复制时不会被拆成两行。";
    assert_eq!(
        drag(&text, (154.0, 192.0), (166.0, 224.0)),
        format!("{para}{para}")
    );
    assert_eq!(drag(&text, (291.0, 176.0), (176.0, 192.0)), "结束。\n这是");
    // pdftotext: the line is x 154.77–312.27, 15 characters of 10.5 pt.
    let x = |k: f32| 154.768 + 10.5 * k;
    assert_eq!(
        drag(&text, (x(9.0) + 1.0, 160.0), (x(14.0) - 1.0, 160.0)),
        "你好，世界"
    );
    // Across lines: "世界！" then "中文".
    assert_eq!(
        drag(&text, (x(12.0) + 1.0, 160.0), (x(2.0) - 1.0, 176.0)),
        "世界！\n中文"
    );
    // Double-click words: a CJK run up to punctuation, a Latin word inside CJK text.
    assert_eq!(word(&text, x(2.5), 160.0), "测试实时编译预览");
    assert_eq!(word(&text, 205.0, 176.0), "English");
    assert_eq!(word(&text, x(8.5), 160.0), "。");
}

#[test]
fn granularity_and_pages() {
    let text = page_text(load("select-en.pdf"), 0).unwrap();
    let i = text.caret_at(262.0, 162.0); // inside "office"
    let word = text.expand(i..i, Granularity::Word);
    assert_eq!(text.text(word), "office");
    let line = text.expand(i..i + 1, Granularity::Line);
    assert!(text.text(line).starts_with("Start writing"));
    assert_eq!(text.expand(3..5, Granularity::Char), 3..5);

    // Two pages: the end of the first, the start of the second.
    let zh = page_text(load("select-zh.pdf"), 0).unwrap();
    let pages = [text, zh];
    let get = |p: usize| pages.get(p);
    let end_en = pages[0].caret_at(339.0, 293.0); // before "and the end."
    let start = TextPos {
        page: 0,
        index: end_en,
    };
    let end = TextPos { page: 1, index: 4 }; // "1 引言" is 3 glyphs, then "测"
    let s = selected_text(start, end, get);
    assert_eq!(s, "and the end.\n1 引言\n测");
    // Order does not matter; unknown pages are skipped.
    assert_eq!(selected_text(end, start, get), s);
    let none = |_: usize| None;
    assert_eq!(selected_text(start, end, none), "");
}

#[test]
fn bad_input_is_an_error() {
    assert!(matches!(
        page_text(Arc::new(b"%PDF-1.4 nonsense".to_vec()), 0),
        Err(PdfError::Invalid)
    ));
    assert!(matches!(
        page_text(load("select-en.pdf"), 7),
        Err(PdfError::NoSuchPage(7))
    ));
    // A truncated file never panics.
    let data = load("select-zh.pdf");
    for cut in [100, data.len() / 3, data.len() / 2, data.len() - 40] {
        let _ = page_text(Arc::new(data[..cut].to_vec()), 0);
    }
}

#[test]
fn worker_extracts_text_after_renders() {
    let worker = RenderWorker::spawn(|| {});
    worker.open(5, load("select-zh.pdf"));
    worker.render(5, 0, 0.5);
    worker.text(5, 0);
    worker.text(4, 0); // stale document: ignored
    let deadline = Instant::now() + Duration::from_secs(20);
    let (mut rendered, mut text) = (false, None);
    while Instant::now() < deadline && text.is_none() {
        match worker.try_recv() {
            Some(RenderEvent::Rendered(r)) => rendered = r.doc == 5,
            Some(RenderEvent::Text { doc, page, text: t }) => {
                assert_eq!((doc, page), (5, 0));
                text = Some(t.unwrap());
            }
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    assert!(rendered, "the render comes first");
    let text = text.expect("text event");
    assert_eq!(text.lines.len(), 6);
}
