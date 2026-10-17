//! The LaTeX source editor pane: jump bar, find bar, gutter with line numbers and error flags,
//! syntax colouring, bracket matching, autocomplete, auto-indent / auto-itemise / auto `\end`.

use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::text_selection::CCursorRange;
use egui::{Align, Color32, Galley, Id, Key, Modifiers, Pos2, Rect, Sense, Stroke, Ui, vec2};
use texcraft_engine::{Selection, Session, edit};
use texcraft_log::Severity;
use texcraft_syntax::completion::{
    Accept, COMMANDS, CompletionContext, ENVIRONMENTS, completion_edit, context_at, expand_snippet,
    match_score,
};
use texcraft_syntax::{Span, highlight, matching_bracket};

use crate::theme::{HEADER_HEIGHT, Tokens, editor_font};
use crate::{icons, widgets};

/// One completion candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Shown in the list.
    pub label: String,
    /// Grey text at the right of the row.
    pub detail: String,
    /// Longer description in the info panel.
    pub info: String,
    /// What accepting does.
    pub insert: Insert,
}

/// How a candidate is inserted.
#[derive(Debug, Clone, PartialEq)]
pub enum Insert {
    /// A command snippet (inserted after the backslash).
    Command(String),
    /// An environment name after `\begin{`.
    Environment(String),
    /// A key inside braces (label, citation, file).
    Key(String),
}

/// The open completion popup.
#[derive(Debug, Clone)]
pub struct Completion {
    /// Candidates, best first.
    pub items: Vec<Candidate>,
    /// Selected row.
    pub selected: usize,
    /// Char index where the replaced text starts.
    pub start: usize,
    /// Whether it was opened explicitly with Esc (stays open with an empty prefix).
    pub forced: bool,
}

/// Find bar state.
#[derive(Debug, Clone, Default)]
pub struct FindBar {
    /// Shown.
    pub open: bool,
    /// Replace row shown.
    pub replace_open: bool,
    /// Search text.
    pub query: String,
    /// Replacement.
    pub replacement: String,
    /// Match case.
    pub case_sensitive: bool,
    /// Focus the query field next frame.
    pub focus: bool,
    /// Current match index.
    pub current: usize,
}

/// Editor UI state that is not part of the session.
#[derive(Default)]
pub struct EditorState {
    /// Completion popup.
    pub completion: Option<Completion>,
    /// Find bar.
    pub find: FindBar,
    /// Symbols popover open.
    pub symbols_open: bool,
    /// The AI suggestion settings popover is open.
    pub inline_open: bool,
    /// Scroll to this line of the active doc next frame (with focus).
    pub reveal: Option<(usize, u32)>,
    /// Push the session selection into the widget next frame.
    pub push_selection: Option<usize>,
    /// Request keyboard focus.
    pub focus: bool,
    /// Cached syntax spans: (text hash, spans).
    spans: Option<(u64, Arc<Vec<Span>>)>,
    /// Caret rect in screen space (for popups), last frame.
    caret_rect: Option<Rect>,
    /// Last caret line (for scroll lock / outline highlight).
    pub caret_line: u32,
    /// The caret line changed this frame.
    pub caret_moved: bool,
    /// Spell checking.
    pub spell: SpellState,
}

/// Spell-checking state: the word list (loaded in the background), verdicts per word, the
/// misspellings of the current text and the open suggestions menu.
pub struct SpellState {
    list: Option<std::sync::Arc<texcraft_spell::WordList>>,
    loading: Option<std::sync::mpsc::Receiver<texcraft_spell::WordList>>,
    verdicts: std::collections::HashMap<String, bool>,
    ignored: std::collections::HashSet<String>,
    misspelled: Option<(u64, Vec<Range<usize>>)>,
    pending: bool,
    menu: Option<SpellMenu>,
}

/// The editor's right-click menu: spelling suggestions when it opened on a misspelled word,
/// then the edit commands.
struct SpellMenu {
    spell: Option<SpellTarget>,
    at: Pos2,
    /// Just opened: the menu measures itself again (its items differ from the last one's).
    fresh: bool,
}

/// The misspelled word a right-click menu is about.
struct SpellTarget {
    range: Range<usize>,
    word: String,
    guesses: Vec<String>,
}

impl Default for SpellState {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("texcraft-spell-load".into())
            .spawn(move || {
                let _ = tx.send(texcraft_spell::WordList::system());
            });
        Self {
            list: None,
            loading: Some(rx),
            verdicts: std::collections::HashMap::new(),
            ignored: std::collections::HashSet::new(),
            misspelled: None,
            pending: false,
            menu: None,
        }
    }
}

impl SpellState {
    fn poll(&mut self) {
        if let Some(rx) = &self.loading
            && let Ok(list) = rx.try_recv()
        {
            self.list = Some(std::sync::Arc::new(list));
            self.loading = None;
            self.misspelled = None;
        }
    }

    /// Misspelled byte ranges of `text` (cached per text). Words the word list rejects are
    /// confirmed with the system checker, a few per frame.
    fn misspellings(&mut self, text: &str, hash: u64, spans: &[Span]) -> Vec<Range<usize>> {
        self.poll();
        let Some(list) = self.list.clone() else {
            return Vec::new();
        };
        if !self.pending
            && let Some((h, m)) = &self.misspelled
            && *h == hash
        {
            return m.clone();
        }
        let mut budget = 40usize;
        let mut pending = false;
        let mut out = Vec::new();
        for r in texcraft_spell::words(text, spans) {
            let Some(word) = text.get(r.clone()) else {
                continue;
            };
            let key = word.to_ascii_lowercase();
            if self.ignored.contains(&key) {
                continue;
            }
            let ok = match self.verdicts.get(&key) {
                Some(v) => *v,
                None => {
                    let listed = list.is_correct(word);
                    if listed {
                        true
                    } else if budget > 0 {
                        budget -= 1;
                        // The platform checker knows more words; trust it when it answers.
                        let v = texcraft_macos::spelling_ok(word).unwrap_or(false);
                        self.verdicts.insert(key, v);
                        v
                    } else {
                        pending = true;
                        true
                    }
                }
            };
            if !ok {
                out.push(r);
            }
        }
        self.pending = pending;
        self.misspelled = Some((hash, out.clone()));
        out
    }
}

/// Char indices for sorted byte offsets, in one pass over `text`.
fn char_indices_for(text: &str, offsets: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(offsets.len());
    let mut it = offsets.iter().peekable();
    let mut ci = 0usize;
    for (b, _) in text.char_indices() {
        while it.peek().is_some_and(|&&o| o <= b) {
            out.push(ci);
            it.next();
        }
        ci += 1;
    }
    for _ in it {
        out.push(ci);
    }
    out
}

/// Draws red dotted underlines under the misspelled words visible in `clip`.
fn paint_misspellings(
    ui: &Ui,
    t: &Tokens,
    text: &str,
    ranges: &[Range<usize>],
    (galley, gpos): (&Galley, Pos2),
    clip: Rect,
    ghost: Option<(usize, usize)>,
) {
    if ranges.is_empty() {
        return;
    }
    let mut offsets: Vec<usize> = Vec::with_capacity(ranges.len() * 2);
    for r in ranges {
        offsets.push(r.start);
        offsets.push(r.end);
    }
    // Characters after an inline suggestion sit further on in the galley.
    let chars: Vec<usize> = char_indices_for(text, &offsets)
        .into_iter()
        .map(|c| match ghost {
            Some((at, n)) if c >= at => c + n,
            _ => c,
        })
        .collect();
    let p = ui.painter();
    for pair in chars.chunks_exact(2) {
        let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let ra = galley
            .pos_from_cursor(CCursor::new(a))
            .translate(gpos.to_vec2());
        if ra.bottom() < clip.top() || ra.top() > clip.bottom() {
            continue;
        }
        let rb = galley
            .pos_from_cursor(CCursor::new(b))
            .translate(gpos.to_vec2());
        let x1 = if (rb.top() - ra.top()).abs() < 1.0 {
            rb.left()
        } else {
            ra.left() + 40.0
        };
        let y = ra.bottom() - 1.0;
        let mut x = ra.left();
        while x < x1 {
            p.circle_filled(egui::pos2(x + 0.8, y), 0.9, t.spell_error);
            x += 3.0;
        }
    }
}

/// The widget id of the editor for a document path.
pub fn editor_id(path: Option<&Path>) -> Id {
    Id::new(("texcraft-editor", path.map(Path::to_path_buf)))
}

fn text_hash(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// Builds the layout job: syntax colours plus background highlights (sorted, non-overlapping).
fn build_job(
    text: &str,
    spans: &[Span],
    highlights: &[(Range<usize>, Color32)],
    t: &Tokens,
    font: &egui::FontId,
    wrap_width: f32,
) -> LayoutJob {
    let mut job = LayoutJob {
        text: text.to_string(),
        ..LayoutJob::default()
    };
    job.wrap.max_width = wrap_width;
    let mut hi = highlights.iter().peekable();
    for span in spans {
        let mut start = span.range.start;
        let end = span.range.end;
        let color = t.syntax(span.kind);
        while start < end {
            // Skip highlights that ended before this point.
            while hi.peek().is_some_and(|(r, _)| r.end <= start) {
                hi.next();
            }
            let (seg_end, bg) = match hi.peek() {
                Some((r, c)) if r.start <= start => (end.min(r.end), *c),
                Some((r, _)) if r.start < end => (r.start, Color32::TRANSPARENT),
                _ => (end, Color32::TRANSPARENT),
            };
            let seg_end = seg_end.max(start + 1).min(end);
            if text.is_char_boundary(start) && text.is_char_boundary(seg_end) {
                job.sections.push(egui::text::LayoutSection {
                    leading_space: 0.0,
                    byte_range: egui::text::ByteIndex(start)..egui::text::ByteIndex(seg_end),
                    format: TextFormat {
                        font_id: font.clone(),
                        line_height: Some((font.size * 1.25).round()),
                        color,
                        background: bg,
                        ..TextFormat::default()
                    },
                });
            }
            start = seg_end;
        }
    }
    if job.sections.is_empty() {
        job.sections.push(egui::text::LayoutSection {
            leading_space: 0.0,
            byte_range: egui::text::ByteIndex(0)..egui::text::ByteIndex(text.len()),
            format: TextFormat {
                font_id: font.clone(),
                line_height: Some((font.size * 1.25).round()),
                color: t.editor_text,
                ..TextFormat::default()
            },
        });
    }
    job
}

/// Paints the selection the way the target does: a neutral tint under the text that runs to
/// the right edge of the text area on every line it covers to the end (one block, no gaps), the
/// syntax colours unchanged. egui recolours selected glyphs and draws its own highlight into
/// the galley it paints, so that shape (added after `shapes_from`) gets `clean`, the same text
/// laid out again, while the tint goes into the reserved shape `sel_idx` underneath.
#[allow(clippy::too_many_arguments)]
fn paint_selection(
    ui: &Ui,
    t: &Tokens,
    (sel_idx, shapes_from): (egui::layers::ShapeIdx, usize),
    painted: &Arc<Galley>,
    clean: Arc<Galley>,
    galley_pos: Pos2,
    range: egui::text::CCursorRange,
    right: f32,
) {
    let layer = ui.layer_id();
    ui.ctx().graphics_mut(|g| {
        let list = g.entry(layer);
        for i in shapes_from..list.next_idx().0 {
            list.mutate_shape(egui::layers::ShapeIdx(i), |cs| {
                if let egui::Shape::Text(ts) = &mut cs.shape
                    && Arc::ptr_eq(&ts.galley, painted)
                {
                    ts.galley = clean.clone();
                }
            });
        }
    });
    if range.is_empty() {
        return;
    }
    let [a, b] = range.sorted_cursors();
    let start = clean.pos_from_cursor(a).translate(galley_pos.to_vec2());
    let end = clean.pos_from_cursor(b).translate(galley_pos.to_vec2());
    let left_edge = galley_pos.x;
    let clip = ui.clip_rect();
    let mut shapes = Vec::new();
    for row in &clean.rows {
        let r = row.rect().translate(galley_pos.to_vec2());
        if r.bottom() <= start.top() + 0.5 || r.top() >= end.bottom() - 0.5 {
            continue;
        }
        if r.bottom() < clip.top() || r.top() > clip.bottom() {
            continue;
        }
        let first = (r.top() - start.top()).abs() < 0.5;
        let last = (r.top() - end.top()).abs() < 0.5;
        let x0 = if first { start.left() } else { left_edge };
        let x1 = if last { end.left() } else { right };
        if x1 - x0 > 0.5 {
            shapes.push(egui::Shape::rect_filled(
                Rect::from_min_max(pos(x0, r.top()), pos(x1, r.bottom())),
                0.0,
                t.editor_selection,
            ));
        }
    }
    ui.painter().set(sel_idx, egui::Shape::Vec(shapes));
}

/// Lays `job` out with a hanging indent (the target's "soft wrap with hanging indent"): each
/// line's first row as wide as the wrap width, its continuation rows `indent` narrower and
/// shifted right by it. (Shifting the rows of one layout made every first row `indent` short.)
/// Lines that wrap are laid out as their first row plus the rest; everything else in runs.
/// Cursor placement and hit-testing follow, since they use each row's own position and glyphs.
fn layout_hanging(ui: &Ui, job: LayoutJob, indent: f32) -> Arc<Galley> {
    let width = job.wrap.max_width;
    let whole = ui.fonts_mut(|f| f.layout_job(job.clone()));
    if indent <= 0.0 || !width.is_finite() || whole.rows.len() < 2 {
        return whole;
    }
    // (first row's end, line's end) of every line that wraps, in characters.
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    let (mut at, mut first_row, mut open) = (0usize, true, None);
    let last = whole.rows.len().saturating_sub(1);
    for (i, row) in whole.rows.iter().enumerate() {
        let end = at.saturating_add(row.glyphs.len());
        if first_row && !row.ends_with_newline && i < last {
            open = Some(end);
        }
        if row.ends_with_newline || i == last {
            if let Some(c) = open.take() {
                cuts.push((c, end));
            }
            at = end.saturating_add(1);
            first_row = true;
        } else {
            at = end;
            first_row = false;
        }
    }
    if cuts.is_empty() {
        return whole;
    }
    // The cuts in bytes.
    let text = job.text.as_str();
    let wanted: Vec<usize> = cuts.iter().flat_map(|&(a, b)| [a, b]).collect();
    let mut bytes = Vec::with_capacity(wanted.len());
    let mut next = wanted.iter().peekable();
    for (ci, (bi, _)) in text.char_indices().chain([(text.len(), ' ')]).enumerate() {
        while next.peek().is_some_and(|&&c| c == ci) {
            bytes.push(bi);
            next.next();
        }
        if next.peek().is_none() {
            break;
        }
    }
    if bytes.len() != wanted.len() {
        return whole;
    }
    // Parts: text up to a wrapped line's first row end, then that line's rest (indented).
    let mut parts: Vec<(Range<usize>, bool)> = Vec::new();
    let mut from = 0;
    for pair in bytes.chunks(2) {
        let (Some(&b1), Some(&be)) = (pair.first(), pair.get(1)) else {
            return whole;
        };
        parts.push((from..b1, false));
        parts.push((b1..be, true));
        from = be.saturating_add(1);
    }
    if from <= text.len() {
        parts.push((from..text.len(), false));
    }
    let mut sections = SectionCursor::default();
    let galleys: Vec<Arc<Galley>> = parts
        .iter()
        .map(|(range, rest)| {
            let w = if *rest {
                (width - indent).max(20.0)
            } else {
                width
            };
            let sub = sections.sub_job(&job, range.clone(), w);
            ui.fonts_mut(|f| f.layout_job(sub))
        })
        .collect();
    let mut g = Galley::concat(Arc::new(job), &galleys, ui.ctx().pixels_per_point());
    // A part that ends at a first row is no line's end; a line's rest is indented.
    let mut row = 0;
    for (i, ((_, rest), part)) in parts.iter().zip(&galleys).enumerate() {
        let n = part.rows.len();
        for r in g.rows.iter_mut().skip(row).take(n) {
            if *rest {
                r.pos.x += indent;
            }
        }
        let ends_first_row = !*rest
            && parts
                .get(i.saturating_add(1))
                .is_some_and(|(_, next)| *next);
        if ends_first_row && let Some(r) = g.rows.get_mut(row.saturating_add(n).saturating_sub(1)) {
            r.ends_with_newline = false;
        }
        row = row.saturating_add(n);
    }
    let mut rect = Rect::NOTHING;
    let mut mesh = Rect::NOTHING;
    for r in &g.rows {
        rect = rect.union(r.rect());
        mesh = mesh.union(r.visuals.mesh_bounds.translate(r.pos.to_vec2()));
    }
    g.rect = rect;
    g.mesh_bounds = mesh;
    Arc::new(g)
}

/// Cuts a [`LayoutJob`] into consecutive byte ranges, walking its sections once.
#[derive(Default)]
struct SectionCursor {
    next: usize,
}

impl SectionCursor {
    /// The part of `job` in `range` (after every earlier range), wrapped at `width`.
    fn sub_job(&mut self, job: &LayoutJob, range: Range<usize>, width: f32) -> LayoutJob {
        let mut out = LayoutJob {
            text: job.text.get(range.clone()).unwrap_or("").to_string(),
            wrap: job.wrap.clone(),
            first_row_min_height: job.first_row_min_height,
            break_on_newline: job.break_on_newline,
            halign: job.halign,
            justify: job.justify,
            round_output_to_gui: job.round_output_to_gui,
            ..Default::default()
        };
        out.wrap.max_width = width;
        let span = |s: &egui::text::LayoutSection| s.byte_range.start.0..s.byte_range.end.0;
        // Sections wholly before this range are done with (the last one may still give an
        // empty final line its font).
        while job.sections.get(self.next).map(span).is_some_and(|r| {
            r.end <= range.start && self.next.saturating_add(1) < job.sections.len()
        }) {
            self.next += 1;
        }
        for s in job.sections.iter().skip(self.next) {
            let r = span(s);
            if r.start >= range.end && !out.sections.is_empty() {
                break;
            }
            let a = r.start.max(range.start);
            let b = r.end.min(range.end);
            if a < b || (range.is_empty() && out.sections.is_empty()) {
                let (a, b) = (a.min(range.end), b.max(a).min(range.end));
                out.sections.push(egui::text::LayoutSection {
                    leading_space: if a == r.start { s.leading_space } else { 0.0 },
                    byte_range: egui::text::ByteIndex(a.saturating_sub(range.start))
                        ..egui::text::ByteIndex(b.saturating_sub(range.start)),
                    format: s.format.clone(),
                });
            }
        }
        out
    }
}

/// Byte ranges of `query` in `text`.
pub fn find_matches(text: &str, query: &str, case_sensitive: bool) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if case_sensitive {
        let mut from = 0;
        while let Some(i) = text.get(from..).and_then(|s| s.find(query)) {
            out.push(from + i..from + i + query.len());
            from += i + query.len().max(1);
            if out.len() > 10_000 {
                break;
            }
        }
    } else if let Ok(re) = regex::RegexBuilder::new(&regex::escape(query))
        .case_insensitive(true)
        .build()
    {
        out.extend(re.find_iter(text).take(10_000).map(|m| m.range()));
    }
    out
}

/// Draws the editor pane for the active document.
pub fn show(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut EditorState) {
    let Some(active) = session.docs.get(session.active).map(|_| session.active) else {
        return;
    };
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, 0.0, t.editor_bg);

    // ---- Jump bar (header).
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_HEIGHT));
    jump_bar(ui, t, session, st, header);
    let mut body = Rect::from_min_max(pos(rect.left(), header.bottom()), rect.max);

    // ---- Find bar.
    if st.find.open {
        let rows = if st.find.replace_open { 2.0 } else { 1.0 };
        let bar = Rect::from_min_size(body.min, vec2(body.width(), 30.0 * rows));
        find_bar(ui, t, session, st, bar);
        body.min.y = bar.bottom();
    }

    // ---- Symbols popover.
    if st.symbols_open {
        symbols_popover(ui, t, session, st, header);
    }
    // ---- AI suggestion settings.
    if st.inline_open {
        let mut open = true;
        crate::ai_inline::popover(ui, t, session, &mut open, inline_button(header));
        st.inline_open = open;
    }

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    text_area(&mut child, t, session, st, active);
}

fn pos(x: f32, y: f32) -> Pos2 {
    Pos2::new(x, y)
}

fn jump_bar(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut EditorState, header: Rect) {
    let p = ui.painter();
    p.rect_filled(header, 0.0, t.header_bg);
    p.line_segment(
        [header.left_bottom(), header.right_bottom()],
        Stroke::new(1.0, t.divider),
    );
    let Some(doc) = session.doc() else { return };
    let name = doc.display_name();
    let root = session.root_path();
    let cy = header.center().y;
    let font = egui::FontId::proportional(11.75);
    let mut x = header.left() + 10.0;
    let is_root = root.as_deref() == doc.typeset_path();
    if let Some(root) = root.as_ref().filter(|_| !is_root) {
        icons::tex_document(
            p,
            Rect::from_min_size(pos(x, cy - 8.0), vec2(12.0, 16.0)),
            t.jump_text,
        );
        x += 17.5;
        let root_name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let g = p.layout_no_wrap(root_name, font.clone(), t.jump_text);
        let w = g.size().x;
        p.galley(pos(x, cy - g.size().y / 2.0), g, t.jump_text);
        x += w + 5.0;
        icons::chevron_thin(
            p,
            Rect::from_min_size(pos(x, cy - 6.0), vec2(8.0, 12.0)),
            t.jump_text,
            true,
        );
        x += 13.0;
    }
    icons::tex_document(
        p,
        Rect::from_min_size(pos(x, cy - 8.0), vec2(12.0, 16.0)),
        t.jump_text,
    );
    x += 17.5;
    let dirty = doc.is_dirty() && !doc.is_pristine() && doc.path.is_some();
    let label = if dirty {
        format!("{name} — Edited")
    } else {
        name
    };
    let g = p.layout_no_wrap(label, font, t.jump_text);
    p.galley(pos(x, cy - g.size().y / 2.0), g, t.jump_text);

    // A divider between the AI suggestion button and √x (symbols).
    let sep_x = header.right() - 35.0;
    p.line_segment(
        [pos(sep_x, cy - 8.0), pos(sep_x, cy + 8.0)],
        Stroke::new(1.0, t.divider.gamma_multiply(0.8)),
    );
    let mut open = st.inline_open;
    crate::ai_inline::header_button(ui, t, session, &mut open, inline_button(header));
    if open && !st.inline_open {
        st.symbols_open = false;
    }
    st.inline_open = open;
    let br = Rect::from_center_size(pos(header.right() - 17.5, cy), vec2(26.0, 20.0));
    let resp = ui
        .interact(br, Id::new("symbols-button"), Sense::click())
        .on_hover_text("Symbols");
    if st.symbols_open || resp.hovered() {
        ui.painter().rect_filled(br, 4.0, t.row_hover);
    }
    ui.painter().text(
        br.center(),
        egui::Align2::CENTER_CENTER,
        "√x",
        egui::FontId::proportional(14.0),
        if st.symbols_open {
            t.accent
        } else {
            t.pdf_icon
        },
    );
    if resp.clicked() {
        st.symbols_open = !st.symbols_open;
    }
}

/// Where the AI suggestion button sits in the header (left of √x).
fn inline_button(header: Rect) -> Rect {
    Rect::from_center_size(
        pos(header.right() - 52.5, header.center().y),
        vec2(26.0, 20.0),
    )
}

fn find_bar(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut EditorState, bar: Rect) {
    ui.painter().rect_filled(bar, 0.0, t.header_bg);
    ui.painter().line_segment(
        [bar.left_bottom(), bar.right_bottom()],
        Stroke::new(1.0, t.divider.gamma_multiply(0.6)),
    );
    let text = session.doc().map(|d| d.text.clone()).unwrap_or_default();
    let matches = find_matches(&text, &st.find.query, st.find.case_sensitive);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink2(vec2(8.0, 4.0)))
            .layout(egui::Layout::top_down(Align::Min)),
    );
    let mut go: Option<bool> = None; // Some(backwards)
    let mut replace_one = false;
    let mut replace_all = false;
    child.horizontal(|ui| {
        ui.set_height(22.0);
        let id = Id::new("find-query");
        let w = (ui.available_width() - 250.0).max(120.0);
        let resp = ui.add_sized(
            vec2(w, 22.0),
            egui::TextEdit::singleline(&mut st.find.query)
                .id(id)
                .hint_text("Find")
                .font(egui::FontId::proportional(12.0)),
        );
        if st.find.focus {
            resp.request_focus();
            st.find.focus = false;
        }
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            go = Some(ui.input(|i| i.modifiers.shift));
            resp.request_focus();
        }
        let count = if st.find.query.is_empty() {
            String::new()
        } else if matches.is_empty() {
            "No matches".to_string()
        } else {
            format!(
                "{} of {}",
                st.find.current.min(matches.len() - 1) + 1,
                matches.len()
            )
        };
        ui.label(egui::RichText::new(count).size(11.0).color(t.text_weak));
        if widgets::icon_button(
            ui,
            t,
            vec2(22.0, 22.0),
            "Previous",
            false,
            !matches.is_empty(),
            icons::chevron_left,
        )
        .clicked()
        {
            go = Some(true);
        }
        if widgets::icon_button(
            ui,
            t,
            vec2(22.0, 22.0),
            "Next",
            false,
            !matches.is_empty(),
            icons::chevron_right,
        )
        .clicked()
        {
            go = Some(false);
        }
        ui.checkbox(
            &mut st.find.case_sensitive,
            egui::RichText::new("Aa").size(11.0),
        )
        .on_hover_text("Match case");
        ui.checkbox(
            &mut st.find.replace_open,
            egui::RichText::new("Replace").size(11.0),
        );
        if ui.button("Done").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
            st.find.open = false;
            st.focus = true;
        }
    });
    if st.find.replace_open {
        child.horizontal(|ui| {
            ui.set_height(22.0);
            let w = (ui.available_width() - 250.0).max(120.0);
            ui.add_sized(
                vec2(w, 22.0),
                egui::TextEdit::singleline(&mut st.find.replacement)
                    .hint_text("Replace")
                    .font(egui::FontId::proportional(12.0)),
            );
            if ui
                .add_enabled(!matches.is_empty(), egui::Button::new("Replace"))
                .clicked()
            {
                replace_one = true;
            }
            if ui
                .add_enabled(!matches.is_empty(), egui::Button::new("Replace All"))
                .clicked()
            {
                replace_all = true;
            }
        });
    }
    if matches.is_empty() {
        return;
    }
    if replace_all {
        let n = matches.len();
        let replacement = st.find.replacement.clone();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for m in &matches {
            out.push_str(text.get(last..m.start).unwrap_or(""));
            out.push_str(&replacement);
            last = m.end;
        }
        out.push_str(text.get(last..).unwrap_or(""));
        let _ = session.run("edit.setText", &serde_json::json!({ "text": out }));
        session.set_status(format!(
            "Replaced {n} occurrence{}",
            if n == 1 { "" } else { "s" }
        ));
        st.push_selection = Some(session.active);
        return;
    }
    if replace_one {
        let i = st.find.current.min(matches.len() - 1);
        if let Some(m) = matches.get(i) {
            let sel = Selection {
                anchor: edit::char_of(&text, m.start),
                cursor: edit::char_of(&text, m.end),
            };
            let _ = session.run(
                "edit.select",
                &serde_json::json!({ "anchor": sel.anchor, "cursor": sel.cursor }),
            );
            let _ = session.run(
                "edit.insertText",
                &serde_json::json!({ "text": st.find.replacement }),
            );
            st.push_selection = Some(session.active);
        }
        return;
    }
    if let Some(back) = go {
        let caret_b = session
            .doc()
            .map_or(0, |d| edit::byte_of(&d.text, d.selection.end()));
        let idx = if back {
            let caret_start = session
                .doc()
                .map_or(0, |d| edit::byte_of(&d.text, d.selection.start()));
            matches
                .iter()
                .rposition(|m| m.end <= caret_start)
                .unwrap_or(matches.len() - 1)
        } else {
            matches.iter().position(|m| m.start >= caret_b).unwrap_or(0)
        };
        select_match(session, st, &text, &matches, idx);
    }
}

fn select_match(
    session: &mut Session,
    st: &mut EditorState,
    text: &str,
    matches: &[Range<usize>],
    idx: usize,
) {
    let Some(m) = matches.get(idx) else { return };
    st.find.current = idx;
    let anchor = edit::char_of(text, m.start);
    let cursor = edit::char_of(text, m.end);
    let _ = session.run(
        "edit.select",
        &serde_json::json!({ "anchor": anchor, "cursor": cursor }),
    );
    let line = edit::line_of_char(text, anchor);
    st.reveal = Some((session.active, line));
    st.push_selection = Some(session.active);
}

/// Find next/previous from the menu (⌘G / ⇧⌘G).
pub fn find_next(session: &mut Session, st: &mut EditorState, backwards: bool) {
    let Some(doc) = session.doc() else { return };
    let text = doc.text.clone();
    if st.find.query.is_empty() {
        st.find.open = true;
        st.find.focus = true;
        return;
    }
    let matches = find_matches(&text, &st.find.query, st.find.case_sensitive);
    if matches.is_empty() {
        return;
    }
    let caret_end = edit::byte_of(&text, doc.selection.end());
    let caret_start = edit::byte_of(&text, doc.selection.start());
    let idx = if backwards {
        matches
            .iter()
            .rposition(|m| m.end <= caret_start)
            .unwrap_or(matches.len() - 1)
    } else {
        matches
            .iter()
            .position(|m| m.start >= caret_end)
            .unwrap_or(0)
    };
    select_match(session, st, &text, &matches, idx);
}

fn symbols_popover(
    ui: &mut Ui,
    t: &Tokens,
    session: &mut Session,
    st: &mut EditorState,
    header: Rect,
) {
    let width = 300.0f32.min(header.width() - 16.0).max(200.0);
    let anchor = pos(header.right() - width - 6.0, header.bottom() + 4.0);
    let mut insert: Option<&str> = None;
    let area = egui::Area::new(Id::new("symbols-popover"))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .fill(t.popup_bg)
                .show(ui, |ui| {
                    ui.set_width(width);
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            for cat in crate::symbols::CATEGORIES {
                                ui.label(
                                    egui::RichText::new(cat.name)
                                        .size(11.0)
                                        .strong()
                                        .color(t.text_weak),
                                );
                                let cols = 6;
                                let cell = vec2((width - 8.0) / cols as f32, 34.0);
                                for row in cat.symbols.chunks(cols) {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 0.0;
                                        for (glyph, cmd) in row {
                                            let (r, resp) =
                                                ui.allocate_exact_size(cell, Sense::click());
                                            if resp.hovered() {
                                                ui.painter().rect_filled(
                                                    r.shrink(1.0),
                                                    4.0,
                                                    t.row_hover,
                                                );
                                            }
                                            ui.painter().text(
                                                r.center() - vec2(0.0, 5.0),
                                                egui::Align2::CENTER_CENTER,
                                                *glyph,
                                                egui::FontId::proportional(16.0),
                                                t.text,
                                            );
                                            let short = if cmd.len() > 9 { &cmd[..9] } else { cmd };
                                            ui.painter().text(
                                                r.center() + vec2(0.0, 10.0),
                                                egui::Align2::CENTER_CENTER,
                                                short,
                                                egui::FontId::proportional(8.0),
                                                t.text_weak,
                                            );
                                            if resp.on_hover_text(*cmd).clicked() {
                                                insert = Some(cmd);
                                            }
                                        }
                                    });
                                }
                                ui.add_space(6.0);
                            }
                        });
                });
        });
    if let Some(cmd) = insert {
        let _ = session.run("edit.insertText", &serde_json::json!({ "text": cmd }));
        st.push_selection = Some(session.active);
        st.focus = true;
    }
    let clicked_outside = ui.input(|i| i.pointer.any_click())
        && !area
            .response
            .rect
            .contains(ui.input(|i| i.pointer.interact_pos().unwrap_or_default()))
        && !Rect::from_center_size(
            pos(header.right() - 22.0, header.center().y),
            vec2(30.0, 20.0),
        )
        .contains(ui.input(|i| i.pointer.interact_pos().unwrap_or_default()));
    if clicked_outside || ui.input(|i| i.key_pressed(Key::Escape)) {
        st.symbols_open = false;
    }
}

/// Builds completion candidates for `ctx` from the built-in catalogue and the project index.
pub fn candidates(session: &Session, ctx: &CompletionContext) -> Vec<Candidate> {
    let mut scored: Vec<(u32, Candidate)> = Vec::new();
    match ctx {
        CompletionContext::Command { prefix, .. } => {
            // Rank by match class first, then by catalogue order (common commands come first).
            for (rank, c) in COMMANDS.iter().enumerate() {
                if let Some(s) = match_score(c.name, prefix) {
                    let exact = u32::from(c.name != prefix.as_str());
                    let s = exact * 100_000
                        + (s / 1000) * 1000
                        + u32::try_from(rank).unwrap_or(999).min(999);
                    scored.push((
                        s,
                        Candidate {
                            label: format!("\\{}", c.name),
                            detail: c.detail.to_string(),
                            info: format!("\\{}", expand_snippet(c.snippet).0),
                            insert: Insert::Command(c.snippet.to_string()),
                        },
                    ));
                }
            }
            for name in &session.index.commands {
                if COMMANDS.iter().any(|c| c.name == name) {
                    continue;
                }
                if let Some(s) = match_score(name, prefix) {
                    let s = (s / 1000) * 1000 + 500;
                    scored.push((
                        s,
                        Candidate {
                            label: format!("\\{name}"),
                            detail: "user command".into(),
                            info: "Defined in this project".into(),
                            insert: Insert::Command(name.clone()),
                        },
                    ));
                }
            }
        }
        CompletionContext::Environment { prefix, .. } => {
            for e in ENVIRONMENTS {
                if let Some(s) = match_score(e.name, prefix) {
                    scored.push((
                        s,
                        Candidate {
                            label: e.name.to_string(),
                            detail: e.detail.to_string(),
                            info: format!("\\begin{{{}}} … \\end{{{}}}", e.name, e.name),
                            insert: Insert::Environment(e.name.to_string()),
                        },
                    ));
                }
            }
        }
        CompletionContext::Reference { prefix, .. } => {
            for (key, file, line, section) in session.index.labels() {
                if let Some(s) = match_score(&key, prefix) {
                    let fname = file
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    scored.push((
                        s,
                        Candidate {
                            label: key.clone(),
                            detail: format!("{fname}:{line}"),
                            info: if section.is_empty() {
                                format!("{fname}, line {line}")
                            } else {
                                format!("In “{section}” — {fname}, line {line}")
                            },
                            insert: Insert::Key(key),
                        },
                    ));
                }
            }
        }
        CompletionContext::Citation { prefix, .. } => {
            for (e, _) in &session.index.bib {
                if let Some(s) = match_score(&e.key, prefix) {
                    scored.push((
                        s,
                        Candidate {
                            label: e.key.clone(),
                            detail: e.year.clone(),
                            info: format!("{}\n{}\n{} {}", e.title, e.author, e.entry_type, e.year)
                                .trim()
                                .to_string(),
                            insert: Insert::Key(e.key.clone()),
                        },
                    ));
                }
            }
        }
        CompletionContext::File {
            prefix, command, ..
        } => {
            let images = command == "includegraphics";
            let base = session
                .root_path()
                .and_then(|r| r.parent().map(Path::to_path_buf));
            if let Some(base) = base {
                let mut files = Vec::new();
                collect_files(&base, &base, 0, &mut files);
                for rel in files {
                    let kind = texcraft_project::FileKind::of(Path::new(&rel));
                    let ok = match command.as_str() {
                        "includegraphics" => {
                            kind == texcraft_project::FileKind::Image
                                || kind == texcraft_project::FileKind::Pdf
                        }
                        "bibliography" | "addbibresource" => {
                            kind == texcraft_project::FileKind::Bib
                        }
                        "includepdf" => kind == texcraft_project::FileKind::Pdf,
                        _ => kind == texcraft_project::FileKind::Tex,
                    };
                    if !ok {
                        continue;
                    }
                    // \input and \include usually omit .tex; \bibliography omits .bib.
                    let shown = if !images
                        && (rel.ends_with(".tex")
                            || (command == "bibliography" && rel.ends_with(".bib")))
                    {
                        rel.rsplit_once('.')
                            .map_or(rel.clone(), |(a, _)| a.to_string())
                    } else {
                        rel.clone()
                    };
                    if let Some(s) = match_score(&shown, prefix) {
                        scored.push((
                            s,
                            Candidate {
                                label: shown.clone(),
                                detail: String::new(),
                                info: rel,
                                insert: Insert::Key(shown),
                            },
                        ));
                    }
                }
            }
        }
        CompletionContext::None => {}
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
    scored.dedup_by(|a, b| a.1.label == b.1.label);
    scored.into_iter().take(200).map(|(_, c)| c).collect()
}

fn collect_files(base: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
    if depth > 4 || out.len() > 2000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.filter_map(Result::ok) {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || texcraft_project::is_aux_file(&p) {
            continue;
        }
        if p.is_dir() {
            collect_files(base, &p, depth + 1, out);
        } else if let Ok(rel) = p.strip_prefix(base) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn completion_start(ctx: &CompletionContext) -> Option<usize> {
    match ctx {
        CompletionContext::Command { start, .. }
        | CompletionContext::Environment { start, .. }
        | CompletionContext::Reference { start, .. }
        | CompletionContext::Citation { start, .. }
        | CompletionContext::File { start, .. } => Some(*start),
        CompletionContext::None => None,
    }
}

/// Re-evaluates the popup after an edit or caret move.
fn update_completion(session: &Session, st: &mut EditorState, forced: bool) {
    let Some(doc) = session.doc() else {
        st.completion = None;
        return;
    };
    if !doc.selection.is_empty() {
        st.completion = None;
        return;
    }
    let caret_b = edit::byte_of(&doc.text, doc.selection.cursor);
    let ctx = context_at(&doc.text, caret_b);
    let auto = match &ctx {
        CompletionContext::Command { prefix, .. } => !prefix.is_empty(),
        CompletionContext::None => false,
        _ => true,
    };
    let keep_forced = forced || st.completion.as_ref().is_some_and(|c| c.forced);
    if !(auto || keep_forced) {
        st.completion = None;
        return;
    }
    let Some(start_b) = completion_start(&ctx) else {
        st.completion = None;
        return;
    };
    let items = candidates(session, &ctx);
    if items.is_empty() {
        st.completion = None;
        return;
    }
    // For commands the replaced range starts after the backslash.
    let start_b = if matches!(ctx, CompletionContext::Command { .. }) {
        start_b + 1
    } else {
        start_b
    };
    let start = edit::char_of(&doc.text, start_b);
    let selected = st
        .completion
        .as_ref()
        .and_then(|c| c.items.get(c.selected))
        .and_then(|cur| items.iter().position(|i| i.label == cur.label))
        .unwrap_or(0);
    st.completion = Some(Completion {
        items,
        selected,
        start,
        forced: keep_forced,
    });
}

/// Accepts the selected candidate.
fn accept_completion(session: &mut Session, st: &mut EditorState) {
    let Some(c) = st.completion.take() else {
        return;
    };
    let Some(item) = c.items.get(c.selected).cloned() else {
        return;
    };
    let Some(doc) = session.doc_mut() else { return };
    let caret = doc.selection.cursor;
    let text = doc.text.clone();
    let accept = match &item.insert {
        Insert::Command(snippet) => Accept::Command(snippet),
        Insert::Environment(name) => Accept::Environment(name),
        Insert::Key(key) => Accept::Key(key),
    };
    let e = completion_edit(
        &text,
        edit::byte_of(&text, c.start.min(caret)),
        edit::byte_of(&text, caret),
        accept,
    );
    let mut out = String::with_capacity(text.len() + e.insert.len());
    out.push_str(text.get(..e.start).unwrap_or(""));
    out.push_str(&e.insert);
    out.push_str(text.get(e.end..).unwrap_or(""));
    let caret_new = edit::char_of(&out, e.caret);
    doc.replace_text(out, Selection::caret(caret_new));
    session.note_edit();
    st.push_selection = Some(session.active);
}

/// Handles Return: auto-indent, auto-itemise, auto `\end{env}`. Returns true if handled.
fn smart_return(session: &mut Session, auto_indent: bool, auto_end: bool) -> bool {
    let Some(doc) = session.doc_mut() else {
        return false;
    };
    if !doc.selection.is_empty() {
        return false;
    }
    let text = doc.text.clone();
    let caret_b = edit::byte_of(&text, doc.selection.cursor);
    let line_start = text
        .get(..caret_b)
        .and_then(|x| x.rfind('\n'))
        .map_or(0, |i| i + 1);
    let line_end = text
        .get(caret_b..)
        .and_then(|x| x.find('\n'))
        .map_or(text.len(), |i| caret_b + i);
    let before = text.get(line_start..caret_b).unwrap_or("");
    let after = text.get(caret_b..line_end).unwrap_or("");
    let indent: String = if auto_indent {
        before
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    } else {
        String::new()
    };
    let trimmed = before.trim();

    let mut insert = format!("\n{indent}");
    let mut caret_after = insert.chars().count();
    let mut replace_start = caret_b;

    if trimmed == "\\item" && after.trim().is_empty() {
        // Return on an empty item ends the list: drop the empty \item.
        replace_start = line_start;
        insert = indent
            .get(..indent.len().saturating_sub(2))
            .unwrap_or("")
            .to_string();
        caret_after = insert.chars().count();
    } else if trimmed.starts_with("\\item") {
        insert = format!("\n{indent}\\item ");
        caret_after = insert.chars().count();
    } else if let Some(env) = trimmed
        .strip_prefix("\\begin{")
        .and_then(|r| r.split_once('}'))
        .map(|(name, _)| name.to_string())
        .filter(|_| auto_end && after.trim().is_empty())
    {
        let opens = text.matches(&format!("\\begin{{{env}}}")).count();
        let closes = text.matches(&format!("\\end{{{env}}}")).count();
        let item = if matches!(env.as_str(), "itemize" | "enumerate" | "description") {
            "\\item "
        } else {
            ""
        };
        if opens > closes {
            let inner = format!("\n{indent}  {item}");
            caret_after = inner.chars().count();
            insert = format!("{inner}\n{indent}\\end{{{env}}}");
        } else {
            insert = format!("\n{indent}  {item}");
            caret_after = insert.chars().count();
        }
    }
    let mut out = String::with_capacity(text.len() + insert.len());
    out.push_str(text.get(..replace_start).unwrap_or(""));
    out.push_str(&insert);
    out.push_str(text.get(caret_b..).unwrap_or(""));
    let caret = edit::char_of(&text, replace_start) + caret_after;
    doc.replace_text(out, Selection::caret(caret));
    session.note_edit();
    true
}

fn text_area(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut EditorState, active: usize) {
    let font = editor_font(session.view.editor_font_size);
    let wrap = session.view.wrap_lines;
    let hanging = session.view.hanging_indent;
    let hang_chars = session.view.hanging_chars;
    let tab_spaces = session.view.tab_spaces;
    let auto_complete = session.view.auto_complete;
    let (auto_indent, auto_end) = (session.view.auto_indent, session.view.auto_end);
    let show_numbers = session.view.line_numbers;
    let spellcheck = session.view.spellcheck;
    let Some(doc) = session.docs.get(active) else {
        return;
    };
    let id = editor_id(doc.typeset_path());
    let doc_path: Option<PathBuf> = doc.typeset_path().map(Path::to_path_buf);
    let has_focus = ui.memory(|m| m.has_focus(id));

    // ---- Keys we handle before the text widget sees them.
    let mut pushed = st.push_selection.take() == Some(active);
    // An inline AI suggestion: Tab inserts it, ⌘→ its next sentence, Esc drops it.
    if has_focus && st.completion.is_none() && session.inline_suggestion().is_some() {
        let (tab, sentence, esc) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Tab),
                i.consume_key(Modifiers::COMMAND, Key::ArrowRight),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if tab || sentence {
            let part = if tab {
                texcraft_engine::inline::Accept::All
            } else {
                texcraft_engine::inline::Accept::Sentence
            };
            if session.inline_accept(part).is_ok() {
                pushed = true;
            }
        } else if esc {
            session.inline_dismiss();
        }
    }
    if has_focus {
        if st.completion.is_some() {
            let n = st.completion.as_ref().map_or(0, |c| c.items.len());
            let (down, up, accept, esc, right) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::Tab)
                        || i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                    i.key_pressed(Key::ArrowRight),
                )
            });
            if let Some(c) = st.completion.as_mut() {
                if down {
                    c.selected = (c.selected + 1).min(n.saturating_sub(1));
                }
                if up {
                    c.selected = c.selected.saturating_sub(1);
                }
            }
            if accept {
                accept_completion(session, st);
                pushed = true;
            } else if esc || right {
                st.completion = None;
            }
        } else {
            let (esc, enter, tab, shift_tab) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::Escape),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Tab),
                    i.consume_key(Modifiers::SHIFT, Key::Tab),
                )
            });
            if esc {
                update_completion(session, st, true);
            }
            if enter && smart_return(session, auto_indent, auto_end) {
                pushed = true;
            }
            if tab {
                let multi = session.doc().is_some_and(|d| {
                    let s = d.selection;
                    !s.is_empty()
                        && edit::line_of_char(&d.text, s.start())
                            != edit::line_of_char(&d.text, s.end())
                });
                if multi {
                    let _ = session.run("edit.indent", &serde_json::Value::Null);
                } else {
                    let ins = if tab_spaces == 0 {
                        "\t".to_string()
                    } else {
                        " ".repeat(usize::from(tab_spaces))
                    };
                    let _ = session.run("edit.insertText", &serde_json::json!({ "text": ins }));
                }
                session.note_edit();
                pushed = true;
            }
            if shift_tab {
                let _ = session.run("edit.outdent", &serde_json::Value::Null);
                pushed = true;
            }
        }
    }

    // ---- Push the session's selection into the widget (after commands edited the text).
    let reveal = st.reveal.filter(|(d, _)| *d == active);
    if (pushed || reveal.is_some())
        && let Some(doc) = session.docs.get(active)
    {
        let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(doc.selection.anchor),
            CCursor::new(doc.selection.cursor),
        )));
        state.store(ui.ctx(), id);
    }
    if st.focus || reveal.is_some() {
        ui.memory_mut(|m| m.request_focus(id));
        st.focus = false;
    }

    // ---- Highlights: find matches, bracket pair.
    let Some(doc) = session.docs.get(active) else {
        return;
    };
    let hash = text_hash(&doc.text);
    // The suggestion is laid out into the text (grey, at the caret) unless this frame brings
    // input the widget handles: its positions must match the real text then.
    let busy_input = ui.input(|i| {
        i.pointer.any_pressed()
            || i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Cut
                        | egui::Event::Key { pressed: true, .. }
                )
            })
    });
    let ghost: Option<(usize, usize, String)> = session
        .inline_suggestion()
        .filter(|g| g.doc == active && !busy_input)
        .map(|g| (g.at, edit::byte_of(&doc.text, g.at), g.text.clone()));
    let ghost_chars = ghost.as_ref().map(|(at, _, g)| (*at, g.chars().count()));
    let spans = match &st.spans {
        Some((h, s)) if *h == hash => s.clone(),
        _ => {
            let s = Arc::new(highlight(&doc.text));
            st.spans = Some((hash, s.clone()));
            s
        }
    };
    let mut highlights: Vec<(Range<usize>, Color32)> = Vec::new();
    if st.find.open {
        for m in find_matches(&doc.text, &st.find.query, st.find.case_sensitive)
            .into_iter()
            .take(5000)
        {
            highlights.push((m, t.find_match));
        }
    }
    if doc.selection.is_empty() {
        let caret_b = edit::byte_of(&doc.text, doc.selection.cursor);
        let probe = [caret_b, caret_b.wrapping_sub(1)];
        if let Some((a, b)) = probe
            .iter()
            .find_map(|&p| matching_bracket(&doc.text, p).map(|m| (p, m)))
        {
            let (x, y) = (a.min(b), a.max(b));
            highlights.push((x..x + 1, t.bracket_match));
            highlights.push((y..y + 1, t.bracket_match));
        }
    }
    highlights.sort_by_key(|(r, _)| r.start);
    highlights.dedup_by(|a, b| a.0.start < b.0.end);

    // Error lines for the gutter.
    let error_lines: Vec<u32> = session
        .typeset
        .issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .filter(|i| match (&i.file, &doc_path) {
            (Some(f), Some(p)) => texcraft_project::normalize(f) == texcraft_project::normalize(p),
            _ => false,
        })
        .filter_map(|i| i.line)
        .collect();

    let line_count = doc.text.bytes().filter(|&b| b == b'\n').count() + 1;
    let digits = line_count.to_string().len().max(2) as f32;
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let number_font = egui::FontId::new(9.5, font.family.clone());
    let number_right = (digits * 5.8 + 6.0).max(18.5);
    let gutter_w = if show_numbers {
        number_right + 12.5
    } else {
        12.0
    };

    let caret_line_before = st.caret_line;
    let mut changed = false;
    let mut cmd_clicked = false;
    let mut output_galley: Option<(Arc<Galley>, Pos2)> = None;
    let mut cursor_range = None;

    let scroll = if wrap {
        egui::ScrollArea::vertical()
    } else {
        egui::ScrollArea::both()
    };
    scroll
        .id_salt(("editor-scroll", doc_path.clone()))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let band_idx = ui.painter().add(egui::Shape::Noop);
            // The selection, under the text (see `paint_selection`).
            let sel_idx = ui.painter().add(egui::Shape::Noop);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let (gutter, _) = ui.allocate_exact_size(vec2(gutter_w, 1.0), Sense::hover());
                let wrap_width = if wrap {
                    (ui.available_width() - 16.0).max(80.0)
                } else {
                    f32::INFINITY
                };
                // Wrapped continuation rows: indented by the tab width (4 characters) or the chosen
                // number of characters, like the target's "soft wrap with hanging indent".
                let hang = if hanging {
                    f32::from(if hang_chars == 0 { 4 } else { hang_chars }) * char_w
                } else {
                    0.0
                };
                let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, width: f32| {
                    let text = buf.as_str();
                    let spans_now: Arc<Vec<Span>> = if text_hash(text) == hash {
                        spans.clone()
                    } else {
                        Arc::new(highlight(text))
                    };
                    let w = if wrap {
                        (width - hang).max(40.0)
                    } else {
                        f32::INFINITY
                    };
                    let mut job = build_job(
                        text,
                        &spans_now,
                        if text_hash(text) == hash {
                            &highlights
                        } else {
                            &[]
                        },
                        t,
                        &font,
                        w,
                    );
                    if let Some((_, b, g)) = ghost.as_ref().filter(|_| text_hash(text) == hash) {
                        crate::ai_inline::splice_ghost(&mut job, *b, g, &font, t.ghost_text);
                    }
                    if wrap {
                        layout_hanging(ui, job, hang)
                    } else {
                        ui.fonts_mut(|f| f.layout_job(job))
                    }
                };
                let Some(doc) = session.docs.get_mut(active) else {
                    return;
                };
                let shapes_from = ui
                    .ctx()
                    .graphics(|g| g.get(ui.layer_id()).map_or(0, |l| l.next_idx().0));
                let out = egui::TextEdit::multiline(&mut doc.text)
                    .id(id)
                    .font(font.clone())
                    .frame(egui::Frame::NONE)
                    .lock_focus(true)
                    .desired_width(wrap_width)
                    .desired_rows(40)
                    .margin(egui::Margin {
                        left: 0,
                        right: 12,
                        top: 2,
                        bottom: 120,
                    })
                    .layouter(&mut layouter)
                    .show(ui);
                changed = out.response.response.changed();
                if let Some(range) = out.cursor_range {
                    let job = (*out.galley.job).clone();
                    let clean = if wrap {
                        layout_hanging(ui, job, hang)
                    } else {
                        ui.fonts_mut(|f| f.layout_job(job))
                    };
                    let right = ui.max_rect().right().max(ui.clip_rect().right()) - 4.0;
                    paint_selection(
                        ui,
                        t,
                        (sel_idx, shapes_from),
                        &out.galley,
                        clean,
                        out.galley_pos,
                        range,
                        right,
                    );
                }
                cmd_clicked = out.response.response.clicked() && ui.input(|i| i.modifiers.command);
                cursor_range = out.cursor_range;
                // Gutter: line numbers on each logical line's first row.
                let galley = out.galley.clone();
                let gpos = out.galley_pos;
                let clip = ui.clip_rect();
                let caret_row_y = out
                    .cursor_range
                    .map(|c| galley.pos_from_cursor(c.primary).translate(gpos.to_vec2()));
                if let Some(caret) = caret_row_y {
                    let band = Rect::from_min_max(
                        pos(gutter.left(), caret.top()),
                        pos(ui.max_rect().right() + 40.0, caret.bottom()),
                    );
                    if (has_focus || out.response.response.has_focus()) && t.current_line.a() > 0 {
                        ui.painter().set(
                            band_idx,
                            egui::Shape::rect_filled(band, 0.0, t.current_line),
                        );
                    }
                    st.caret_rect = Some(caret);
                }
                if show_numbers {
                    let caret_line = out
                        .cursor_range
                        .map(|c| edit::line_of_char(&doc.text, c.primary.index.0));
                    let mut line = 1u32;
                    let mut new_line = true;
                    let mut row_start = 0usize;
                    // Line breaks inside an inline suggestion start no numbered line.
                    let in_ghost =
                        |ci: usize| ghost_chars.is_some_and(|(at, n)| ci >= at && ci < at + n);
                    for row in &galley.rows {
                        let r = row.rect().translate(gpos.to_vec2());
                        let row_chars = row.char_count_excluding_newline().0;
                        let ghost_break = row.ends_with_newline && in_ghost(row_start + row_chars);
                        row_start += row_chars + usize::from(row.ends_with_newline);
                        if new_line && r.bottom() >= clip.top() && r.top() <= clip.bottom() {
                            let label = line.to_string();
                            let is_err = error_lines.contains(&line);
                            let y = r.center().y;
                            if is_err {
                                let flag = Rect::from_min_max(
                                    pos(gutter.left() + 1.0, y - 7.0),
                                    pos(gutter.left() + number_right + 7.0, y + 7.0),
                                );
                                icons::error_flag(ui.painter(), flag, t.error);
                                ui.painter().text(
                                    pos(gutter.left() + number_right, y),
                                    egui::Align2::RIGHT_CENTER,
                                    label,
                                    number_font.clone(),
                                    Color32::WHITE,
                                );
                            } else {
                                let col = if Some(line) == caret_line {
                                    t.line_number_current
                                } else {
                                    t.line_number
                                };
                                ui.painter().text(
                                    pos(gutter.left() + number_right, y),
                                    egui::Align2::RIGHT_CENTER,
                                    label,
                                    number_font.clone(),
                                    col,
                                );
                            }
                        }
                        new_line = row.ends_with_newline && !ghost_break;
                        if new_line {
                            line += 1;
                        }
                        if r.top() > clip.bottom() {
                            break;
                        }
                    }
                }
                let ranges = if spellcheck {
                    let ranges = st.spell.misspellings(&doc.text, hash, &spans);
                    paint_misspellings(
                        ui,
                        t,
                        &doc.text,
                        &ranges,
                        (&galley, gpos),
                        clip,
                        ghost_chars,
                    );
                    if st.spell.pending {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(30));
                    }
                    ranges
                } else {
                    Vec::new()
                };
                // Right-click: the editor menu, with suggestions on a misspelled word.
                if out.response.response.secondary_clicked()
                    && let Some(pointer) = out.response.response.interact_pointer_pos()
                {
                    let ci = galley.cursor_from_pos(pointer - gpos).index.0;
                    let b = edit::byte_of(&doc.text, ci);
                    let spell = ranges.iter().find(|r| r.start <= b && b <= r.end).map(|r| {
                        let word = doc.text.get(r.clone()).unwrap_or("").to_string();
                        SpellTarget {
                            range: r.clone(),
                            guesses: texcraft_macos::spelling_guesses(&word),
                            word,
                        }
                    });
                    st.spell.menu = Some(SpellMenu {
                        spell,
                        at: pointer,
                        fresh: true,
                    });
                }
                output_galley = Some((galley, gpos));
                // Scroll to a revealed line.
                if let Some((_, line)) = reveal {
                    let ci = edit::char_of_line(&doc.text, line);
                    let r = out
                        .galley
                        .pos_from_cursor(CCursor::new(ci))
                        .translate(gpos.to_vec2());
                    ui.scroll_to_rect(r.expand2(vec2(0.0, 40.0)), Some(Align::Center));
                }
            });
        });
    if reveal.is_some() {
        st.reveal = None;
    }

    // ---- Mirror the selection back into the session.
    let mut selection_moved = false;
    if let (Some(c), Some(doc)) = (cursor_range, session.docs.get_mut(active)) {
        let n = doc.text.chars().count();
        let before = doc.selection;
        doc.selection = Selection {
            anchor: c.secondary.index.0.min(n),
            cursor: c.primary.index.0.min(n),
        };
        selection_moved = doc.selection != before;
        st.caret_line = edit::line_of_char(&doc.text, doc.selection.cursor);
    }
    st.caret_moved = st.caret_line != caret_line_before;
    if !changed && selection_moved {
        // Only typing asks for AI suggestions; moving the caret drops them.
        session.inline_after_caret_move();
    }
    if changed {
        session.note_edit();
        if auto_complete || st.completion.is_some() {
            update_completion(session, st, false);
        }
    } else if st.completion.is_some() && (st.caret_moved || !has_focus) {
        // The caret left the word.
        update_completion(session, st, false);
    }
    if cmd_clicked {
        st.completion = None;
        session.forward_sync(true);
    }
    spell_menu(ui, t, session, st, active);

    // ---- Completion popup.
    if let (Some(c), Some(caret)) = (st.completion.as_ref(), st.caret_rect) {
        completion_popup(ui, t, c, caret, &font);
    }
}

fn spell_menu(ui: &Ui, t: &Tokens, session: &mut Session, st: &mut EditorState, active: usize) {
    let Some(menu) = &st.spell.menu else { return };
    let mut replace: Option<String> = None;
    let mut learn = false;
    let mut ignore = false;
    let mut close = false;
    let mut command: Option<&'static str> = None;
    let area = egui::Area::new(Id::new("spell-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(menu.at + vec2(2.0, 2.0))
        .sizing_pass(menu.fresh)
        .show(ui.ctx(), |ui| {
            crate::menu::frame(t).show(ui, |ui| {
                crate::menu::begin(ui, 220.0);
                if let Some(spell) = &menu.spell {
                    if spell.guesses.is_empty() {
                        let none = crate::menu::Item {
                            disabled: true,
                            ..Default::default()
                        };
                        crate::menu::item(ui, t, "No Guesses Found", none);
                    }
                    for g in &spell.guesses {
                        let strong = crate::menu::Item {
                            strong: true,
                            ..Default::default()
                        };
                        if crate::menu::item(ui, t, g, strong).clicked() {
                            replace = Some(g.clone());
                        }
                    }
                    crate::menu::separator(ui, t);
                    if crate::menu::button(ui, t, "Learn Spelling") {
                        learn = true;
                    }
                    if crate::menu::button(ui, t, "Ignore Spelling") {
                        ignore = true;
                    }
                    crate::menu::separator(ui, t);
                }
                for id in ["edit.cut", "edit.copy", "edit.paste"] {
                    command = command.or(command_item(ui, t, session, id));
                }
                crate::menu::separator(ui, t);
                for id in ["edit.selectAll", "edit.toggleComment"] {
                    command = command.or(command_item(ui, t, session, id));
                }
                crate::menu::separator(ui, t);
                command = command.or(command_item(ui, t, session, "typeset.syncPdf"));
            });
        });
    if ui.input(|i| i.key_pressed(Key::Escape))
        || (ui.input(|i| i.pointer.any_click())
            && !area
                .response
                .rect
                .contains(ui.input(|i| i.pointer.interact_pos().unwrap_or_default()))
            && !ui.input(|i| i.pointer.secondary_clicked()))
    {
        close = true;
    }
    if let Some(id) = command {
        // Cut / Copy / Paste act on the text widget: give it the focus back first.
        st.focus = true;
        if let Err(e) = session.run(id, &serde_json::Value::Null) {
            session.set_status(e.to_string());
        }
        close = true;
    }
    let (word, range) = menu
        .spell
        .as_ref()
        .map(|s| (s.word.clone(), s.range.clone()))
        .unwrap_or_default();
    if let Some(g) = replace {
        if let Some(d) = session.docs.get(active) {
            let a = edit::char_of(&d.text, range.start);
            let b = edit::char_of(&d.text, range.end);
            let _ = session.run(
                "edit.select",
                &serde_json::json!({ "anchor": a, "cursor": b }),
            );
            let _ = session.run("edit.insertText", &serde_json::json!({ "text": g }));
            st.push_selection = Some(active);
        }
        close = true;
    }
    if learn {
        texcraft_macos::learn_word(&word);
        st.spell.verdicts.insert(word.to_ascii_lowercase(), true);
        st.spell.misspelled = None;
        close = true;
    }
    if ignore {
        st.spell.ignored.insert(word.to_ascii_lowercase());
        st.spell.misspelled = None;
        close = true;
    }
    if close {
        st.spell.menu = None;
    } else if let Some(m) = st.spell.menu.as_mut() {
        m.fresh = false;
    }
}

/// A menu item for command `id` (label and shortcut from the registry); returns the id when
/// chosen. The command runs after the menu has closed.
fn command_item(
    ui: &mut Ui,
    t: &Tokens,
    session: &Session,
    id: &'static str,
) -> Option<&'static str> {
    let spec = texcraft_engine::registry().iter().find(|c| c.id == id)?;
    let shortcut = spec.shortcut.map(|s| s.display()).unwrap_or_default();
    let opts = crate::menu::Item {
        shortcut: &shortcut,
        disabled: !session.is_enabled(id),
        ..Default::default()
    };
    crate::menu::item(ui, t, spec.label, opts)
        .clicked()
        .then_some(id)
}

fn completion_popup(ui: &Ui, t: &Tokens, c: &Completion, caret: Rect, font: &egui::FontId) {
    let row_h = 20.0;
    let width = 340.0;
    let visible = c.items.len().min(8);
    let start = c
        .selected
        .saturating_sub(visible.saturating_sub(1))
        .min(c.items.len().saturating_sub(visible));
    egui::Area::new(Id::new("completion-popup"))
        .order(egui::Order::Tooltip)
        .fixed_pos(caret.left_bottom() + vec2(-6.0, 3.0))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .fill(t.popup_bg)
                .inner_margin(4)
                .show(ui, |ui| {
                    ui.set_width(width);
                    for (i, item) in c.items.iter().enumerate().skip(start).take(visible) {
                        let (r, _) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
                        let selected = i == c.selected;
                        if selected {
                            ui.painter().rect_filled(r, 4.0, t.accent);
                        }
                        let fg = if selected {
                            Color32::WHITE
                        } else {
                            t.syn_command
                        };
                        let weak = if selected {
                            Color32::from_white_alpha(200)
                        } else {
                            t.text_weak
                        };
                        let mono = egui::FontId::new(12.0, font.family.clone());
                        let label = widgets::elide(ui.painter(), &item.label, &mono, width - 110.0);
                        ui.painter().text(
                            r.left_center() + vec2(8.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            label,
                            mono,
                            fg,
                        );
                        ui.painter().text(
                            r.right_center() - vec2(8.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            &item.detail,
                            egui::FontId::proportional(11.0),
                            weak,
                        );
                    }
                    if let Some(item) = c.items.get(c.selected)
                        && !item.info.is_empty()
                    {
                        ui.separator();
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&item.info)
                                    .size(11.5)
                                    .color(t.text_weak),
                            )
                            .wrap(),
                        );
                    }
                    ui.label(
                        egui::RichText::new(format!(
                            "{} of {}   ⇥ insert   ⎋ close",
                            c.selected + 1,
                            c.items.len()
                        ))
                        .size(10.0)
                        .color(t.text_weak),
                    );
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Soft wrap with a hanging indent: first rows as wide as unwrapped text allows, the rest
    /// indented, every character in place (the ghost text's first row ended a few characters
    /// early, 2026-10-17).
    #[test]
    fn hanging_indent_keeps_first_rows_full_width() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let font = egui::FontId::monospace(12.0);
            let (w, indent) = (300.0, 24.0);
            let long = "word ".repeat(40);
            let cjk = "这回次旅行恰逢其时，我们于金秋九月出发，踏上了这座城市。".repeat(3);
            let mut job = LayoutJob::default();
            for (text, color) in [
                ("short\n", Color32::WHITE),
                (long.as_str(), Color32::RED),
                ("\n\n", Color32::WHITE),
                (cjk.as_str(), Color32::GREEN),
                ("\nend\n", Color32::WHITE),
            ] {
                job.append(text, 0.0, TextFormat::simple(font.clone(), color));
            }
            job.wrap.max_width = w;
            let text = job.text.clone();
            let g = layout_hanging(ui, job.clone(), indent);
            let chars: usize = g
                .rows
                .iter()
                .map(|r| r.glyphs.len() + usize::from(r.ends_with_newline))
                .sum();
            assert_eq!(chars, text.chars().count());
            let plain = ui.fonts_mut(|f| f.layout_job(job));
            // Line starts at the margin, as wide as without the indent; wrapped rows indented.
            let mut first = true;
            for (i, r) in g.rows.iter().enumerate() {
                if first {
                    assert_eq!(r.pos.x, 0.0, "row {i}");
                } else {
                    assert_eq!(r.pos.x, indent, "row {i}");
                }
                assert!(r.rect().right() <= w + 0.5, "row {i} too wide");
                first = r.ends_with_newline;
            }
            // The long lines' first rows hold what an unindented layout puts there.
            assert_eq!(g.rows[1].glyphs.len(), plain.rows[1].glyphs.len());
            assert!(g.rows.len() >= plain.rows.len());
            // A cursor round trip through the laid-out rows.
            let end = egui::text::CCursor::new(text.chars().count());
            assert_eq!(
                g.cursor_from_pos(g.pos_from_cursor(end).center().to_vec2()),
                end
            );
        });
        out.textures_delta.clear();
    }

    #[test]
    fn job_sections_cover_text_and_split_on_highlights() {
        let t = Tokens::light();
        let text = "\\section{Hi} $x$";
        let spans = highlight(text);
        let font = egui::FontId::monospace(12.0);
        let job = build_job(text, &spans, &[(9..11, Color32::RED)], &t, &font, 100.0);
        let mut at = 0;
        for s in &job.sections {
            assert_eq!(s.byte_range.start.0, at);
            at = s.byte_range.end.0;
        }
        assert_eq!(at, text.len());
        assert!(job.sections.iter().any(|s| s.byte_range.start.0 == 9
            && s.byte_range.end.0 == 11
            && s.format.background == Color32::RED));
    }

    #[test]
    fn find_is_case_insensitive_by_default() {
        assert_eq!(find_matches("Abc abc ABC", "abc", false).len(), 3);
        assert_eq!(find_matches("Abc abc ABC", "abc", true), vec![4..7]);
        assert!(find_matches("x", "", false).is_empty());
        assert_eq!(find_matches("a.b a+b", ".", false), vec![1..2]);
    }
}

#[cfg(test)]
mod ranking_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn common_commands_rank_first() {
        let session = Session::new(Arc::new(|| {}));
        let ctx = CompletionContext::Command {
            prefix: "sub".into(),
            start: 0,
        };
        let labels: Vec<String> = candidates(&session, &ctx)
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(
            labels.first().map(String::as_str),
            Some("\\subsection"),
            "{labels:?}"
        );
        let ctx = CompletionContext::Command {
            prefix: "sec".into(),
            start: 0,
        };
        let labels: Vec<String> = candidates(&session, &ctx)
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(
            labels.first().map(String::as_str),
            Some("\\section"),
            "{labels:?}"
        );
        let ctx = CompletionContext::Command {
            prefix: "al".into(),
            start: 0,
        };
        let labels: Vec<String> = candidates(&session, &ctx)
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(
            labels.first().map(String::as_str),
            Some("\\alpha"),
            "{labels:?}"
        );
    }
}
