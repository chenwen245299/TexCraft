//! The text of a PDF page for selecting and copying in the viewer: every glyph with the Unicode
//! text it stands for and its box, grouped into lines.
//!
//! hayro's interpreter hands each run of positioned glyphs to a [`Device`]; [`Collector`] keeps
//! their positions and Unicode values (`ToUnicode` maps, glyph names) and draws nothing. TeX's
//! PDFs carry no space characters, so word spaces are inferred from the gaps between glyphs.
//! Coordinates are PDF points from the page's top-left corner (y down), like SyncTeX's.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use hayro::hayro_interpret::font::{Glyph, GlyphRun, OutlineGlyph};
use hayro::hayro_interpret::hayro_cmap::{self, BfString, CMap, CMapName};
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, TransformExt, interpret_page,
};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{Affine, BezPath, Point, Rect};
use read_fonts::ps::cff::charset::Charset;
use read_fonts::ps::cff::dict;
use read_fonts::ps::cff::index::Index;
use read_fonts::ps::cff::v1::Cff;
use read_fonts::types::Tag;
use read_fonts::{FontData, FontRead, FontRef};

use crate::{PdfError, guarded};

/// A box in PDF points from the page's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextRect {
    /// Left edge.
    pub x0: f32,
    /// Top edge.
    pub y0: f32,
    /// Right edge.
    pub x1: f32,
    /// Bottom edge.
    pub y1: f32,
}

impl TextRect {
    /// Width (>= 0).
    pub fn width(&self) -> f32 {
        (self.x1 - self.x0).max(0.0)
    }

    /// Height (>= 0).
    pub fn height(&self) -> f32 {
        (self.y1 - self.y0).max(0.0)
    }

    fn union(self, o: Self) -> Self {
        Self {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    /// Horizontal and vertical distance from a point (0 inside).
    fn distance(&self, x: f32, y: f32) -> (f32, f32) {
        let dx = (self.x0 - x).max(x - self.x1).max(0.0);
        let dy = (self.y0 - y).max(y - self.y1).max(0.0);
        (dx, dy)
    }

    /// Whether the point lies inside, grown by `slop` on every side.
    pub fn contains(&self, x: f32, y: f32, slop: f32) -> bool {
        x >= self.x0 - slop && x <= self.x1 + slop && y >= self.y0 - slop && y <= self.y1 + slop
    }
}

/// One glyph of a page.
#[derive(Debug, Clone, PartialEq)]
pub struct TextGlyph {
    /// What it stands for: usually one character, several for a ligature, empty when the font
    /// does not say.
    pub text: String,
    /// Its advance box, from the font's descent to its ascent.
    pub rect: TextRect,
    /// A word space precedes it on its line.
    pub space_before: bool,
    /// Index of its line in [`PageText::lines`].
    pub line: usize,
}

/// Glyphs that sit on one baseline, in the order the page draws them.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    /// Its glyphs (indices into [`PageText::glyphs`]).
    pub glyphs: Range<usize>,
    /// The union of their boxes.
    pub rect: TextRect,
}

/// The text of one page.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageText {
    /// Every glyph, in drawing order (reading order for TeX's output).
    pub glyphs: Vec<TextGlyph>,
    /// Lines, in the same order.
    pub lines: Vec<TextLine>,
}

/// A caret position in a document: before glyph `index` of 0-based page `page`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TextPos {
    /// 0-based page.
    pub page: usize,
    /// Glyph index on that page (its glyph count for the end of the page).
    pub index: usize,
}

/// How a selection grows while dragging: by character, word (double-click) or line
/// (triple-click).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Granularity {
    /// Character by character.
    #[default]
    Char,
    /// Whole words.
    Word,
    /// Whole lines.
    Line,
}

impl PageText {
    /// Number of glyphs.
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    /// The page has no text.
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// The line nearest to a point: the closest vertically, and among lines about as close
    /// (two columns, a raised exponent), the closest horizontally.
    fn nearest_line(&self, x: f32, y: f32) -> Option<&TextLine> {
        let best_dy = self
            .lines
            .iter()
            .map(|l| l.rect.distance(x, y).1)
            .fold(f32::INFINITY, f32::min);
        if !best_dy.is_finite() {
            return None;
        }
        let near = best_dy + 2.0;
        self.lines
            .iter()
            .filter(|l| l.rect.distance(x, y).1 <= near)
            .min_by(|a, b| {
                let (ax, ay) = a.rect.distance(x, y);
                let (bx, by) = b.rect.distance(x, y);
                ax.total_cmp(&bx).then(ay.total_cmp(&by))
            })
    }

    /// Index of the line under a point, grown by `slop` points (for the text cursor).
    pub fn line_at(&self, x: f32, y: f32, slop: f32) -> Option<usize> {
        self.lines.iter().position(|l| l.rect.contains(x, y, slop))
    }

    /// The glyph nearest to a point on the nearest line.
    pub fn glyph_at(&self, x: f32, y: f32) -> Option<usize> {
        let line = self.nearest_line(x, y)?;
        line.glyphs.clone().min_by(|&a, &b| {
            let da = self
                .glyphs
                .get(a)
                .map_or(f32::INFINITY, |g| g.rect.distance(x, y).0);
            let db = self
                .glyphs
                .get(b)
                .map_or(f32::INFINITY, |g| g.rect.distance(x, y).0);
            da.total_cmp(&db)
        })
    }

    /// The caret position nearest to a point: before or after the nearest glyph, whichever
    /// half the point is in.
    pub fn caret_at(&self, x: f32, y: f32) -> usize {
        let Some(i) = self.glyph_at(x, y) else {
            return 0;
        };
        match self.glyphs.get(i) {
            Some(g) if x >= (g.rect.x0 + g.rect.x1) / 2.0 => i.saturating_add(1),
            _ => i,
        }
    }

    /// The word around glyph `i`: a run of letters and digits, or of CJK characters, on one line
    /// without a space in between; punctuation stands alone.
    pub fn word_at(&self, i: usize) -> Range<usize> {
        let Some(g) = self.glyphs.get(i) else {
            return i.min(self.len())..i.min(self.len());
        };
        let class = class_of(g.text.chars().next());
        if !matches!(class, Class::Word | Class::Cjk) {
            return i..i.saturating_add(1);
        }
        let joins = |a: &TextGlyph, b: &TextGlyph| {
            a.line == b.line
                && !b.space_before
                && class_of(a.text.chars().last()) == class
                && class_of(b.text.chars().next()) == class
        };
        let mut start = i;
        while let (Some(prev), Some(cur)) = (
            start.checked_sub(1).and_then(|p| self.glyphs.get(p)),
            self.glyphs.get(start),
        ) {
            if !joins(prev, cur) {
                break;
            }
            start -= 1;
        }
        let mut end = i;
        while let (Some(cur), Some(next)) = (self.glyphs.get(end), self.glyphs.get(end + 1)) {
            if !joins(cur, next) {
                break;
            }
            end += 1;
        }
        start..end + 1
    }

    /// The line around glyph `i`.
    pub fn line_range(&self, i: usize) -> Range<usize> {
        self.glyphs
            .get(i)
            .and_then(|g| self.lines.get(g.line))
            .map_or(i.min(self.len())..i.min(self.len()), |l| l.glyphs.clone())
    }

    /// Highlight boxes for glyphs `range`: one per line, spanning the selected glyphs and the
    /// line's full height.
    pub fn rects(&self, range: Range<usize>) -> Vec<TextRect> {
        let mut out = Vec::new();
        for line in &self.lines {
            let lo = range.start.max(line.glyphs.start);
            let hi = range.end.min(line.glyphs.end);
            if lo >= hi {
                continue;
            }
            let Some(span) = self
                .glyphs
                .get(lo..hi)
                .and_then(|gs| gs.iter().map(|g| g.rect).reduce(TextRect::union))
            else {
                continue;
            };
            out.push(TextRect {
                x0: span.x0,
                x1: span.x1,
                y0: line.rect.y0,
                y1: line.rect.y1,
            });
        }
        out
    }

    /// The text of glyphs `range`: word spaces where the gaps are, a line break between lines
    /// (except where a CJK paragraph merely wraps: Chinese has no spaces to break at).
    pub fn text(&self, range: Range<usize>) -> String {
        // The right edge of the text block, for telling a wrapped line from a paragraph's last.
        let right = self
            .lines
            .iter()
            .map(|l| l.rect.x1)
            .fold(f32::NEG_INFINITY, f32::max);
        let mut out = String::new();
        let mut line: Option<usize> = None;
        for g in self.glyphs.get(range).unwrap_or_default() {
            match line {
                Some(l) if l != g.line => {
                    out.truncate(out.trim_end_matches(' ').len());
                    let wraps = match (self.lines.get(l), self.lines.get(g.line)) {
                        (Some(a), Some(b)) => cjk_wrap(a.rect, b.rect, right),
                        _ => false,
                    };
                    let cjk_ends = out.chars().last().is_some_and(is_cjk)
                        && g.text.chars().next().is_some_and(is_cjk);
                    if !(wraps && cjk_ends) {
                        out.push('\n');
                    }
                }
                Some(_) if g.space_before => out.push(' '),
                _ => {}
            }
            out.push_str(&g.text);
            line = Some(g.line);
        }
        out
    }

    /// Grows a caret range to whole words or lines.
    pub fn expand(&self, range: Range<usize>, granularity: Granularity) -> Range<usize> {
        let grow = |i: usize| match granularity {
            Granularity::Char => i..i,
            Granularity::Word => self.word_at(i),
            Granularity::Line => self.line_range(i),
        };
        if granularity == Granularity::Char {
            return range;
        }
        let first = grow(range.start.min(self.len().saturating_sub(1)));
        let last = grow(
            range
                .end
                .saturating_sub(1)
                .max(range.start)
                .min(self.len().saturating_sub(1)),
        );
        first.start.min(range.start)..last.end.max(range.end)
    }
}

/// Glyphs `[start, end)` of page `page`, for a selection from `start` to `end` (in document
/// order) on a page with `len` glyphs.
pub fn range_on_page(start: TextPos, end: TextPos, page: usize, len: usize) -> Range<usize> {
    if page < start.page || page > end.page {
        return 0..0;
    }
    let lo = if page == start.page { start.index } else { 0 };
    let hi = if page == end.page { end.index } else { len };
    lo.min(len)..hi.min(len).max(lo.min(len))
}

/// The text from `start` to `end`, pages joined by line breaks. Pages whose text is not known
/// (`page` returns `None`) are skipped.
pub fn selected_text<'t>(
    start: TextPos,
    end: TextPos,
    page: impl Fn(usize) -> Option<&'t PageText>,
) -> String {
    let (start, end) = if end < start {
        (end, start)
    } else {
        (start, end)
    };
    let mut parts: Vec<String> = Vec::new();
    for p in start.page..=end.page {
        let Some(text) = page(p) else { continue };
        let s = text.text(range_on_page(start, end, p, text.len()));
        if !s.is_empty() {
            parts.push(s);
        }
    }
    parts.join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Cjk,
    Punct,
}

fn class_of(c: Option<char>) -> Class {
    match c {
        None => Class::Punct,
        Some(c) if c.is_whitespace() => Class::Space,
        Some(c) if is_cjk_letter(c) => Class::Cjk,
        Some(c) if c.is_alphanumeric() || c == '\'' || c == '’' || c == '_' => Class::Word,
        Some(_) => Class::Punct,
    }
}

/// Han, kana and hangul: CJK characters that make up words (not CJK punctuation).
pub fn is_cjk_letter(c: char) -> bool {
    matches!(u32::from(c),
        0x2E80..=0x2FDF   // radicals
        | 0x3005..=0x3007 // 々〆〇
        | 0x3040..=0x30FF // kana
        | 0x3100..=0x312F // bopomofo
        | 0x3131..=0x318E // hangul compatibility jamo
        | 0x31F0..=0x31FF
        | 0x3400..=0x4DBF // extension A
        | 0x4E00..=0x9FFF // unified ideographs
        | 0xAC00..=0xD7AF // hangul syllables
        | 0xF900..=0xFAFF // compatibility ideographs
        | 0xFF66..=0xFF9F // half-width katakana
        | 0x20000..=0x323AF)
}

/// Any CJK character, punctuation and full-width forms included (they set no word spaces).
fn is_cjk(c: char) -> bool {
    is_cjk_letter(c) || matches!(u32::from(c), 0x3000..=0x303F | 0xFE30..=0xFE4F | 0xFF00..=0xFF65)
}

/// Extracts the text of page `page` (0-based).
pub(crate) fn extract<'a>(
    pdf: &'a Pdf,
    cache: &InterpreterCache<'a>,
    settings: &InterpreterSettings,
    fonts: &mut CidFonts,
    page: usize,
) -> Result<PageText, PdfError> {
    let pages = pdf.pages();
    let p = pages.get(page).ok_or(PdfError::NoSuchPage(page))?;
    let (w, h) = p.render_dimensions();
    guarded(|| {
        let mut ctx = Context::new(
            p.initial_transform(true).to_kurbo(),
            Rect::new(0.0, 0.0, f64::from(w), f64::from(h)),
            cache,
            p.xref(),
            settings.clone(),
        );
        let mut collector = Collector::new(w, h, fonts);
        interpret_page(p, &mut ctx, &mut collector);
        collector.finish()
    })
}

/// One glyph as drawn, before lines are formed.
struct Raw {
    text: String,
    rect: TextRect,
    /// Font size in points.
    size: f32,
}

/// Unicode for glyphs of embedded CID-keyed CFF fonts without a `ToUnicode` map (xdvipdfmx
/// writes none for Adobe-GB1, -Japan1, -CNS1 and -Korea1 fonts such as ctex's Fandol): glyph →
/// CID through the font's charset, CID → Unicode through Adobe's UCS2 table for its character
/// collection, as other PDF readers do. Kept for a whole document.
#[derive(Default)]
pub(crate) struct CidFonts {
    /// By hayro's font cache key; `None` for fonts this does not apply to.
    fonts: HashMap<u128, Option<Rc<CidFont>>>,
    /// The UCS2 tables (GB1, Japan1, CNS1, Korea1), parsed when first needed.
    tables: [Option<Option<Rc<CMap>>>; 4],
}

struct CidFont {
    /// Glyph id → CID.
    cids: HashMap<u32, u32>,
    ucs2: Rc<CMap>,
}

const UCS2_TABLES: [CMapName<'static>; 4] = [
    CMapName::AdobeGb1Ucs2,
    CMapName::AdobeJapan1Ucs2,
    CMapName::AdobeCns1Ucs2,
    CMapName::AdobeKorea1Ucs2,
];

impl CidFonts {
    fn unicode(&mut self, glyph: &OutlineGlyph) -> Option<BfString> {
        let data = glyph.font_data()?;
        let key = data.cache_key;
        if !self.fonts.contains_key(&key) {
            let font = self.load((*data.data).as_ref()).map(Rc::new);
            self.fonts.insert(key, font);
        }
        let font = self.fonts.get(&key)?.as_ref()?;
        let cid = font.cids.get(&glyph.glyph_id().to_u32())?;
        font.ucs2.lookup_bf_string(*cid)
    }

    fn load(&mut self, bytes: &[u8]) -> Option<CidFont> {
        let cff = if bytes.starts_with(b"OTTO") {
            FontRef::new(bytes)
                .ok()?
                .table_data(Tag::new(b"CFF "))?
                .as_bytes()
        } else {
            bytes
        };
        let cff = Cff::read(FontData::new(cff)).ok()?;
        let top = cff.top_dicts().get(0).ok()?;
        // `Cff::charset` declines CID-keyed fonts (their charset maps glyphs to CIDs, not
        // names), which is exactly the map wanted here.
        let (mut ordering, mut charset, mut charstrings) = (None, None, None);
        for entry in dict::entries(top, None).flatten() {
            match entry {
                dict::Entry::Ros { ordering: o, .. } => ordering = Some(o),
                dict::Entry::Charset(offset) => charset = Some(offset),
                dict::Entry::CharstringsOffset(offset) => charstrings = Some(offset),
                _ => {}
            }
        }
        let ordering = ordering?;
        let which = match cff.string(ordering)? {
            b"GB1" => 0,
            b"Japan1" => 1,
            b"CNS1" => 2,
            b"Korea1" => 3,
            _ => return None,
        };
        let ucs2 = self.table(which)?;
        let data = cff.offset_data();
        let glyphs = Index::new(data.as_bytes().get(charstrings?..)?, false)
            .ok()?
            .count();
        let cids = Charset::new(data, charset?, glyphs)
            .ok()?
            .iter()
            .map(|(gid, cid)| (gid.to_u32(), u32::from(cid.to_u16())))
            .collect();
        Some(CidFont { cids, ucs2 })
    }

    fn table(&mut self, which: usize) -> Option<Rc<CMap>> {
        let slot = self.tables.get_mut(which)?;
        if slot.is_none() {
            let name = *UCS2_TABLES.get(which)?;
            *slot = Some(
                hayro_cmap::load_embedded(name)
                    .and_then(|data| CMap::parse(data, hayro_cmap::load_embedded))
                    .map(Rc::new),
            );
        }
        slot.clone().flatten()
    }
}

/// A [`Device`] that records glyphs and ignores everything else.
struct Collector<'f> {
    glyphs: Vec<Raw>,
    page: TextRect,
    /// The last filled run (glyph count, first origin): fill-and-stroke text draws twice.
    last_fill: Option<(usize, Point)>,
    fonts: &'f mut CidFonts,
}

impl<'f> Collector<'f> {
    fn new(width: f32, height: f32, fonts: &'f mut CidFonts) -> Self {
        Self {
            fonts,
            glyphs: Vec::new(),
            page: TextRect {
                x0: 0.0,
                y0: 0.0,
                x1: width,
                y1: height,
            },
            last_fill: None,
        }
    }

    fn push(&mut self, glyph: &Glyph<'_>, t: Affine, next: Option<Affine>) {
        // Glyph space has 1000 units per em; `t` maps it to page points.
        let size = (t.determinant().abs().sqrt() * 1000.0) as f32;
        if !size.is_finite() || size < 0.01 {
            return;
        }
        let text = glyph
            .as_unicode()
            .or_else(|| match glyph {
                Glyph::Outline(o) => self.fonts.unicode(o),
                Glyph::Type3(_) => None,
            })
            .map(unicode_text)
            .unwrap_or_default();
        let advance = match glyph {
            Glyph::Outline(o) => o.advance_width().map(f64::from),
            Glyph::Type3(_) => None,
        }
        .filter(|a| a.is_finite() && *a > 0.0)
        .or_else(|| {
            // Without widths: up to where the next glyph of the run starts.
            let x = (t.inverse() * Point::new(next?.translation().x, next?.translation().y)).x;
            (x.is_finite() && x > 0.0 && x < 3000.0).then_some(x)
        })
        .unwrap_or(500.0);
        // Ascent and descent: CJK fonts fill the em box higher than Latin ones.
        let (ascent, descent) = if text.chars().next().is_some_and(is_cjk) {
            (880.0, -120.0)
        } else {
            (780.0, -220.0)
        };
        let corners = [
            Point::new(0.0, descent),
            Point::new(advance, descent),
            Point::new(advance, ascent),
            Point::new(0.0, ascent),
        ]
        .map(|p| t * p);
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for c in corners {
            x0 = x0.min(c.x);
            y0 = y0.min(c.y);
            x1 = x1.max(c.x);
            y1 = y1.max(c.y);
        }
        let rect = TextRect {
            x0: x0 as f32,
            y0: y0 as f32,
            x1: x1 as f32,
            y1: y1 as f32,
        };
        let finite = [rect.x0, rect.y0, rect.x1, rect.y1]
            .iter()
            .all(|v| v.is_finite());
        let on_page = rect.x1 >= self.page.x0
            && rect.x0 <= self.page.x1
            && rect.y1 >= self.page.y0
            && rect.y0 <= self.page.y1;
        if finite && on_page {
            self.glyphs.push(Raw { text, rect, size });
        }
    }

    fn finish(self) -> PageText {
        let mut out = PageText::default();
        let mut prev: Option<(TextRect, f32, Option<char>)> = None;
        for raw in self.glyphs {
            let first = raw.text.chars().next();
            let (new_line, space) = match prev {
                None => (true, false),
                Some((rect, size, last)) => {
                    let same = same_line(rect, size, raw.rect, raw.size);
                    let gap = raw.rect.x0 - rect.x1;
                    let em = size.max(raw.size);
                    let both_cjk = last.is_some_and(is_cjk) && first.is_some_and(is_cjk);
                    let spaced = last.is_some_and(char::is_whitespace)
                        || first.is_some_and(char::is_whitespace)
                        || last.is_none()
                        || first.is_none();
                    let threshold = if both_cjk { 0.6 } else { 0.15 };
                    (!same, same && !spaced && gap > threshold * em)
                }
            };
            let index = out.glyphs.len();
            if new_line {
                out.lines.push(TextLine {
                    glyphs: index..index,
                    rect: raw.rect,
                });
            }
            let line = out.lines.len().saturating_sub(1);
            if let Some(l) = out.lines.last_mut() {
                l.glyphs.end = index + 1;
                l.rect = l.rect.union(raw.rect);
            }
            prev = Some((raw.rect, raw.size, raw.text.chars().last()));
            out.glyphs.push(TextGlyph {
                text: raw.text,
                rect: raw.rect,
                space_before: space,
                line,
            });
        }
        out
    }
}

/// Whether line `b` continues the paragraph of line `a` (for joining CJK text): `a` runs to the
/// right edge of the text block, `b` follows right below it and is not indented further.
fn cjk_wrap(a: TextRect, b: TextRect, right: f32) -> bool {
    let em = b.height().max(1.0);
    a.x1 >= right - em && b.y0 >= a.y0 && b.y0 - a.y1 <= em && b.x0 <= a.x0 + em * 0.5
}

/// Whether glyph `b` continues the line of the glyph `a` before it: their boxes overlap
/// vertically by half the smaller height (exponents and indices stay on the line) and it does
/// not jump back to the left.
fn same_line(a: TextRect, a_size: f32, b: TextRect, b_size: f32) -> bool {
    let overlap = a.y1.min(b.y1) - a.y0.max(b.y0);
    overlap >= 0.5 * a.height().min(b.height()) && b.x0 >= a.x0 - a_size.max(b_size)
}

/// The text of a glyph: ligatures spelt out, control and private-use characters dropped.
fn unicode_text(s: BfString) -> String {
    let raw = match s {
        BfString::Char(c) => c.to_string(),
        BfString::String(s) => s,
    };
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{FB05}' | '\u{FB06}' => out.push_str("st"),
            '\u{E000}'..='\u{F8FF}' | '\u{FFFD}' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

impl<'a> Device<'a> for Collector<'_> {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, mode: &DrawMode) {
        let glyphs = run.glyphs();
        let first = glyphs
            .first()
            .map(|g| Point::ZERO + (props.transform * g.transform()).translation());
        let key = first.map(|p| (glyphs.len(), p));
        match mode {
            DrawMode::Fill(_) | DrawMode::FillAndStroke(..) | DrawMode::Invisible => {
                self.last_fill = key;
            }
            DrawMode::Stroke(_) => {
                if key.is_some() && self.last_fill == key {
                    // The outline of text just filled (fill-and-stroke rendering mode).
                    return;
                }
                self.last_fill = None;
            }
        }
        for (i, g) in glyphs.iter().enumerate() {
            let t = props.transform * g.transform();
            let next = glyphs.get(i + 1).map(|n| props.transform * n.transform());
            self.push(g, t, next);
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}
