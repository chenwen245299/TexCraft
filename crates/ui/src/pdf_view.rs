//! The PDF viewer pane: header bar (zoom, page navigation, scroll lock, pause, page modes),
//! pages rendered on a background thread, SyncTeX highlight and ⌘-click inverse search, text
//! selection (drag, double/triple click) and copying.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{
    Align, Color32, ColorImage, Id, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Ui, pos2, vec2,
};
use texcraft_engine::{Grading, PageMode, PdfPoint, Session, TextUnit, Zoom};
use texcraft_i18n::{tr, trf};
use texcraft_pdf::{PageSize, PageText, RenderEvent, RenderWorker, RenderedPage, TextPos};
use texcraft_synctex::PdfRect;

use crate::pdf_select::{self, PagePoint, Selection};
use crate::theme::{HEADER_HEIGHT, Tokens};
use crate::{flash, icons, menu, widgets};

/// Space around the pages (fit width fills the pane edge to edge, like the target).
const MARGIN: f32 = 0.0;
/// Hairline between pages.
const GAP: f32 = 1.0;
/// Space around thumbnails.
const THUMB_GAP: f32 = 14.0;

struct PageTexture {
    texture: TextureHandle,
    /// Pixels per PDF point it was rendered at.
    scale: f32,
    doc: u64,
    graded: bool,
}

/// PDF viewer state.
#[derive(Default)]
pub struct PdfView {
    worker: Option<RenderWorker>,
    /// Session generation last opened.
    opened_generation: u64,
    /// Document id whose page sizes we have.
    loaded_doc: u64,
    pages: Vec<PageSize>,
    textures: HashMap<usize, PageTexture>,
    requested: HashMap<usize, (f32, u64)>,
    error: Option<String>,
    scroll_to: Option<(PdfRect, bool)>,
    flash: Option<(Vec<PdfRect>, Instant)>,
    goto_page: Option<usize>,
    /// Show pages as a grid of thumbnails.
    pub thumbnails: bool,
    /// The PDF is shown in its own window.
    pub window: bool,
    /// 0-based page at the top of the view.
    pub current_page: usize,
    last_scale: f32,
    last_render_scale: f32,
    /// The edit the document being loaded reflects.
    edit_at: Option<Instant>,
    /// Keystroke-to-pixels time of the last live update.
    pub latency: Option<Duration>,
    /// Text of pages of `texts_doc` (for selecting and copying), extracted by the worker.
    texts: HashMap<usize, Arc<PageText>>,
    /// The document `texts` belong to.
    texts_doc: u64,
    /// Bumped whenever `texts` changes.
    texts_version: u64,
    /// Pages whose text was asked for (document, page).
    text_requested: std::collections::HashSet<(u64, usize)>,
    /// The text selection.
    selection: Option<Selection>,
    /// A drag is extending the selection.
    selecting: bool,
    /// The last press (input time, position, count): double and triple clicks.
    last_press: Option<(f64, Pos2, u8)>,
    /// The user's last click was in the PDF: ⌘C copies its selection.
    pub active: bool,
    /// The selection last resolved, with the `texts_version` it was resolved against.
    resolved: Option<(Option<(TextPos, TextPos)>, u64)>,
    /// Pages on screen last frame (0-based page, rect in window points), for the control channel.
    shown: Vec<(usize, Rect)>,
}

impl PdfView {
    /// Opens the session's PDF if it changed.
    pub fn sync_document(&mut self, ctx: &egui::Context, session: &Session) {
        if self.worker.is_none() {
            let ctx = ctx.clone();
            self.worker = Some(RenderWorker::spawn(move || ctx.request_repaint()));
        }
        if session.pdf.generation == self.opened_generation {
            return;
        }
        self.opened_generation = session.pdf.generation;
        let Some(path) = session.pdf.path.as_ref() else {
            // The pane was emptied (its PDF belonged to another document): no stale pages.
            self.loaded_doc = self.opened_generation;
            self.pages.clear();
            self.textures.clear();
            self.requested.clear();
            self.selection = None;
            self.resolved = None;
            self.flash = None;
            self.error = None;
            self.current_page = 0;
            return;
        };
        match std::fs::read(path) {
            Ok(bytes) => {
                if let Some(w) = &self.worker {
                    // Re-render what is on screen right away (live updates keep the view still).
                    let mut visible: Vec<usize> = self.textures.keys().copied().collect();
                    visible.sort_by_key(|&k| k.abs_diff(self.current_page));
                    visible.truncate(4);
                    if visible.is_empty() {
                        visible.push(self.current_page);
                    }
                    let scale = self
                        .textures
                        .values()
                        .next()
                        .map_or(self.last_render_scale, |t| t.scale);
                    if scale > 0.0 {
                        w.open_and_render(self.opened_generation, Arc::new(bytes), &visible, scale);
                        for &k in &visible {
                            self.requested.insert(k, (scale, self.opened_generation));
                        }
                    } else {
                        w.open(self.opened_generation, Arc::new(bytes));
                    }
                }
                self.edit_at = session.pdf.edit_at;
                self.error = None;
            }
            Err(e) => {
                self.error = Some(trf!(
                    "Cannot read {path}: {e}",
                    path = path.display(),
                    e = e
                ));
            }
        }
    }

    /// Scrolls to and optionally flashes SyncTeX rectangles.
    pub fn show_rects(&mut self, rects: Vec<PdfRect>, flash: bool) {
        if let Some(first) = rects.first().copied() {
            self.scroll_to = Some((first, flash));
        }
        if flash {
            self.flash = Some((rects, Instant::now()));
        }
    }

    /// Scrolls to a 1-based page.
    pub fn goto_page(&mut self, page: u32) {
        self.goto_page = Some((page.max(1) - 1) as usize);
    }

    /// Number of pages.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn poll(&mut self, ctx: &egui::Context, dark_grade: bool) {
        let Some(worker) = &self.worker else { return };
        while let Some(ev) = worker.try_recv() {
            match ev {
                RenderEvent::Opened { doc, pages } => {
                    if doc == self.opened_generation {
                        self.loaded_doc = doc;
                        self.pages = pages;
                        // Keep the prefetched requests for this document.
                        self.requested.retain(|_, (_, d)| *d == doc);
                        self.error = None;
                    }
                }
                RenderEvent::Rendered(r) => {
                    if r.doc == self.opened_generation && r.doc != self.loaded_doc {
                        // Rendered before the Opened event was polled (same batch): accept it.
                        self.loaded_doc = r.doc;
                    }
                    if r.doc == self.loaded_doc {
                        if let Some(at) = self.edit_at.take() {
                            self.latency = Some(at.elapsed());
                        }
                        self.requested.remove(&r.page);
                        let page = r.page;
                        let scale = r.scale;
                        let doc = r.doc;
                        let image = to_image(r, dark_grade);
                        let texture = ctx.load_texture(
                            format!("pdf-page-{page}"),
                            image,
                            TextureOptions::LINEAR,
                        );
                        self.textures.insert(
                            page,
                            PageTexture {
                                texture,
                                scale,
                                doc,
                                graded: dark_grade,
                            },
                        );
                    }
                }
                RenderEvent::Text { doc, page, text } => {
                    if doc == self.loaded_doc
                        && let Ok(text) = text
                    {
                        if self.texts_doc != doc {
                            self.texts.clear();
                            self.texts_doc = doc;
                        }
                        self.texts.insert(page, text);
                        self.texts_version += 1;
                    }
                }
                RenderEvent::Failed { doc, page, error } => {
                    if doc == self.opened_generation && page.is_none() {
                        self.error = Some(error.to_string());
                    }
                    if let Some(p) = page {
                        self.requested.remove(&p);
                    }
                }
            }
        }
    }
}

impl PdfView {
    /// Drops the text of an older document.
    fn sync_texts(&mut self) {
        if self.texts_doc != self.loaded_doc {
            self.texts.clear();
            self.texts_doc = self.loaded_doc;
            self.texts_version += 1;
        }
    }

    /// Selects text from one point to another (`pdf.selectText`), or clears the selection.
    pub fn select(&mut self, sel: Option<(PdfPoint, PdfPoint, TextUnit)>) {
        let point = |p: PdfPoint| PagePoint {
            page: p.page.saturating_sub(1) as usize,
            x: p.x,
            y: p.y,
        };
        self.selection = sel.map(|(from, to, unit)| Selection {
            anchor: point(from),
            focus: point(to),
            unit: pdf_select::granularity(unit),
        });
        self.selecting = false;
        self.active = self.selection.is_some();
    }

    /// The selected glyphs, if any (as far as the pages' text is known).
    pub fn selected_range(&self) -> Option<(TextPos, TextPos)> {
        self.selection
            .as_ref()
            .and_then(|s| pdf_select::resolve(s, &self.texts))
    }

    /// Highlight boxes of the selection on 0-based `page`, in PDF points from its top-left
    /// corner (for tests and the control channel).
    pub fn selection_rects(&self, page: usize) -> Vec<texcraft_pdf::TextRect> {
        match (self.selected_range(), self.texts.get(&page)) {
            (Some(range), Some(text)) => text.rects(pdf_select::page_range(range, page, text)),
            _ => Vec::new(),
        }
    }

    /// The selection waits for the text of one of its pages.
    fn text_pending(&self, sel: &Option<Selection>) -> bool {
        sel.as_ref().is_some_and(|s| {
            !self.texts.contains_key(&s.anchor.page) || !self.texts.contains_key(&s.focus.page)
        })
    }

    /// The selection for `state.inspect`: its ends, highlight boxes per page (PDF points) and
    /// which pages' text is loaded.
    pub fn inspect_selection(&self) -> serde_json::Value {
        let range = self.selected_range();
        let rects: serde_json::Map<String, serde_json::Value> = range
            .map(|(a, b)| a.page..=b.page)
            .into_iter()
            .flatten()
            .map(|k| {
                let boxes: Vec<[f32; 4]> = self
                    .selection_rects(k)
                    .iter()
                    .map(|r| [r.x0, r.y0, r.x1, r.y1])
                    .collect();
                ((k + 1).to_string(), serde_json::json!(boxes))
            })
            .collect();
        let mut pages: Vec<usize> = self.texts.keys().map(|k| k + 1).collect();
        pages.sort_unstable();
        let shown: serde_json::Map<String, serde_json::Value> = self
            .shown
            .iter()
            .map(|(k, r)| {
                let rect = [r.min.x, r.min.y, r.max.x, r.max.y];
                ((k + 1).to_string(), serde_json::json!(rect))
            })
            .collect();
        serde_json::json!({
            // Window points of the visible pages, and points per PDF point.
            "pageRects": shown,
            "scale": self.last_scale,
            "active": self.active,
            "selecting": self.selecting,
            "range": range.map(|(a, b)| [[a.page + 1, a.index], [b.page + 1, b.index]]),
            "rects": rects,
            "textPages": pages,
        })
    }

    /// ⌘C goes to the PDF: there is selected text and the user's last click was in the PDF.
    pub fn owns_copy(&self, session: &Session) -> bool {
        self.active && !session.pdf.selected_text.is_empty()
    }

    /// Keeps `Session::pdf.selected_text` in step with the selection.
    fn publish_selection(&mut self, session: &mut Session) {
        let range = self.selected_range();
        let key = Some((range, self.texts_version));
        if self.resolved == key {
            return;
        }
        self.resolved = key;
        session.pdf.selected_text = match range {
            Some((a, b)) => {
                texcraft_pdf::selected_text(a, b, |p| self.texts.get(&p).map(AsRef::as_ref))
            }
            None => String::new(),
        };
    }
}

fn to_image(r: RenderedPage, dark_grade: bool) -> ColorImage {
    let size = [r.width as usize, r.height as usize];
    let mut rgba = r.rgba;
    if dark_grade {
        grade_dark(&mut rgba);
    }
    ColorImage::from_rgba_premultiplied(size, &rgba)
}

/// Re-tints a page for dark mode: white paper becomes #2E2E2E and black ink #B4B4B4, keeping hue.
pub fn grade_dark(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let (r, g, b) = (f32::from(px[0]), f32::from(px[1]), f32::from(px[2]));
        let l = 0.299 * r + 0.587 * g + 0.114 * b;
        let target = 46.0 + (180.0 - 46.0) * (1.0 - l / 255.0);
        let d = target - l;
        px[0] = (r + d).clamp(0.0, 255.0) as u8;
        px[1] = (g + d).clamp(0.0, 255.0) as u8;
        px[2] = (b + d).clamp(0.0, 255.0) as u8;
    }
}

/// Draws the PDF pane.
pub fn show(ui: &mut Ui, t: &Tokens, session: &mut Session, view: &mut PdfView) {
    let ctx = ui.ctx().clone();
    let dark_grade = t.dark && session.view.grading == Grading::Entire;
    view.poll(&ctx, dark_grade);
    view.sync_texts();
    // Regrade when the appearance changed.
    if view.textures.values().any(|tex| tex.graded != dark_grade) {
        view.textures.clear();
        view.requested.clear();
    }

    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, 0.0, t.pdf_bg);
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_HEIGHT));
    let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);

    // ---- Layout and scale.
    let thumbs = view.thumbnails;
    let two = session.view.page_mode == PageMode::TwoContinuous;
    let max_w = view
        .pages
        .iter()
        .map(|p| p.width)
        .fold(0.0f32, f32::max)
        .max(1.0);
    let max_h = view
        .pages
        .iter()
        .map(|p| p.height)
        .fold(0.0f32, f32::max)
        .max(1.0);
    let ncols: usize = if thumbs {
        ((body.width() - THUMB_GAP) / 150.0).floor().clamp(2.0, 6.0) as usize
    } else if two {
        2
    } else {
        1
    };
    let cols = ncols as f32;
    let (margin, gap) = if thumbs {
        (THUMB_GAP, THUMB_GAP)
    } else {
        (MARGIN, GAP)
    };
    let fit_w = ((body.width() - margin * 2.0 - gap * (cols - 1.0)) / (max_w * cols)).max(0.02);
    let fit_p = fit_w.min(((body.height() - gap * 2.0) / max_h).max(0.02));
    // Zoom percentages are relative to fit width (100% fills the pane), as in the target app.
    let scale = if thumbs {
        fit_w
    } else {
        match session.view.zoom {
            Zoom::FitWidth => fit_w,
            Zoom::FitPage => fit_p,
            Zoom::Fixed(z) => fit_w * z,
        }
    };
    if !matches!(session.view.zoom, Zoom::Fixed(_)) {
        session.pdf_fit_scale = Some(scale / fit_w);
    }
    view.last_scale = scale;
    let zoom_pct = scale / fit_w * 100.0;

    header_bar(ui, t, session, view, header, zoom_pct);

    if view.pages.is_empty() {
        empty_state(ui, t, session, view, body);
        return;
    }

    // Page rectangles in content coordinates.
    let single = session.view.page_mode == PageMode::Single;
    let mut rects: Vec<Rect> = Vec::with_capacity(view.pages.len());
    let content_w = (max_w * scale * cols + gap * (cols - 1.0) + margin * 2.0).max(body.width());
    let mut y = margin;
    let mut i = 0;
    while i < view.pages.len() {
        let row: Vec<usize> = (i..(i + ncols).min(view.pages.len())).collect();
        let row_w: f32 = row
            .iter()
            .map(|&k| view.pages[k].width * scale)
            .sum::<f32>()
            + gap * (row.len() as f32 - 1.0);
        let row_h = row
            .iter()
            .map(|&k| view.pages[k].height * scale)
            .fold(0.0, f32::max);
        let mut x = (content_w - row_w) / 2.0;
        for &k in &row {
            let p = view.pages[k];
            let r = Rect::from_min_size(pos2(x, y), vec2(p.width * scale, p.height * scale));
            rects.push(r);
            x += p.width * scale + gap;
        }
        y += row_h + gap;
        i += row.len();
    }
    if single && !thumbs {
        // Show only the current page.
        let cur = view.current_page.min(view.pages.len() - 1);
        if let Some(r) = rects.get(cur).copied() {
            let shift = r.top() - MARGIN;
            for (k, rr) in rects.iter_mut().enumerate() {
                if k == cur {
                    *rr = rr.translate(vec2(0.0, -shift));
                } else {
                    *rr = Rect::NOTHING;
                }
            }
            y = r.height() + MARGIN * 2.0;
        }
    }
    let content_h = (y - gap + margin).max(0.0);

    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    let ppp = ctx.pixels_per_point();
    let render_scale = scale * ppp;
    view.last_render_scale = render_scale;
    if let Some(l) = view.latency.take() {
        session.typeset.last_latency = Some(l);
    }
    let mut want: Vec<usize> = Vec::new();
    let mut inverse: Option<(u32, f32, f32, Option<texcraft_engine::sync::SyncHint>)> = None;
    let mut copy = false;
    let mut zoom_by: Option<f32> = None;
    let mut thumb_pick: Option<usize> = None;

    let mut scroll = egui::ScrollArea::both()
        .id_salt("pdf-scroll")
        .auto_shrink([false, false]);
    if let Some(p) = view.goto_page.take() {
        if single {
            view.current_page = p.min(view.pages.len() - 1);
        } else if let Some(r) = rects.get(p) {
            scroll = scroll.vertical_scroll_offset(r.top() - MARGIN);
        }
    }
    let out = scroll.show(&mut child, |ui| {
        let (content, resp) =
            ui.allocate_exact_size(vec2(content_w, content_h), Sense::click_and_drag());
        let origin = content.min.to_vec2();
        if !thumbs {
            ui.painter().rect_filled(content, 0.0, t.page_gap);
        }
        let clip = ui.clip_rect();
        // The page under a screen point (with `clamp`, the nearest page, the point moved onto
        // it), in PDF points.
        let page_at = |pos: Pos2, clamp: bool| -> Option<PagePoint> {
            let mut best: Option<(f32, usize, Rect)> = None;
            for (k, r) in rects.iter().enumerate() {
                if *r == Rect::NOTHING {
                    continue;
                }
                let sr = r.translate(origin);
                let d = sr.distance_to_pos(pos);
                if (d == 0.0 || clamp) && best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, k, sr));
                }
            }
            best.map(|(_, k, sr)| {
                let q = sr.clamp(pos);
                PagePoint {
                    page: k,
                    x: (q.x - sr.left()) / scale,
                    y: (q.y - sr.top()) / scale,
                }
            })
        };
        // ---- Text selection: press and drag; double-click words, triple-click lines.
        let (pressed, down, cmd, time, press_pos, pointer) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.modifiers.command,
                i.time,
                i.pointer.press_origin(),
                i.pointer.interact_pos(),
            )
        });
        if !thumbs {
            if pressed && !cmd && resp.hovered() {
                if let Some(pos) = press_pos.or(pointer) {
                    let count = pdf_select::press_count(view.last_press, time, pos);
                    view.last_press = Some((time, pos, count));
                    view.selection = page_at(pos, false).map(|at| Selection {
                        anchor: at,
                        focus: at,
                        unit: pdf_select::unit_for(count),
                    });
                    view.selecting = view.selection.is_some();
                    view.active = true;
                    resp.request_focus();
                }
            } else if pressed && press_pos.is_some_and(|p| !body.contains(p)) {
                view.active = false;
            }
            if view.selecting {
                if down {
                    if let Some(pos) = pointer {
                        if let (Some(at), Some(sel)) = (page_at(pos, true), &mut view.selection) {
                            sel.focus = at;
                        }
                        // Past the top or bottom edge: scroll along.
                        let dy = if pos.y < clip.top() {
                            clip.top() - pos.y
                        } else if pos.y > clip.bottom() {
                            clip.bottom() - pos.y
                        } else {
                            0.0
                        };
                        if dy != 0.0 {
                            ui.scroll_with_delta_animation(
                                vec2(0.0, (dy * 0.4).clamp(-30.0, 30.0)),
                                egui::style::ScrollAnimation::none(),
                            );
                            ui.ctx().request_repaint();
                        }
                    }
                } else {
                    view.selecting = false;
                    if view.selected_range().is_none() && !view.text_pending(&view.selection) {
                        view.selection = None;
                    }
                }
            }
            if view.active
                && view.selection.is_some()
                && ui.input(|i| i.key_pressed(egui::Key::Escape))
            {
                view.selection = None;
            }
            copy = view.active
                && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
            // The text cursor over text.
            if !cmd
                && (view.selecting || resp.hovered())
                && let Some(at) = pointer.and_then(|pos| page_at(pos, false))
                && let Some(text) = view.texts.get(&at.page)
                && (view.selecting || text.line_at(at.x, at.y, 1.0).is_some())
            {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
        }
        let p = ui.painter();
        view.shown.clear();
        for (k, r) in rects.iter().enumerate() {
            if *r == Rect::NOTHING {
                continue;
            }
            let sr = r.translate(origin);
            if !sr.expand(sr.height()).intersects(clip) {
                continue;
            }
            if sr.intersects(clip) {
                want.push(k);
                view.shown.push((k, sr));
            }
            let paper = if dark_grade {
                Color32::from_gray(46)
            } else {
                Color32::WHITE
            };
            p.rect_filled(sr, 0.0, paper);
            if let Some(tex) = view.textures.get(&k) {
                p.image(
                    tex.texture.id(),
                    sr,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            if thumbs {
                p.rect_stroke(
                    sr,
                    0.0,
                    Stroke::new(0.5, t.page_border),
                    StrokeKind::Outside,
                );
                p.text(
                    sr.center_bottom() + vec2(0.0, 8.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", k + 1),
                    egui::FontId::proportional(10.0),
                    t.text_weak,
                );
            }
        }
        // The text selection, over the ink (dimmer while the focus is elsewhere).
        if let Some(range) = view.selected_range().filter(|_| !thumbs) {
            let fill = if view.active {
                t.selection
            } else {
                t.selection.gamma_multiply(0.5)
            };
            for k in range.0.page..=range.1.page {
                let (Some(text), Some(r)) = (view.texts.get(&k), rects.get(k)) else {
                    continue;
                };
                let sr = r.translate(origin);
                if *r == Rect::NOTHING || !sr.intersects(clip) {
                    continue;
                }
                for b in text.rects(pdf_select::page_range(range, k, text)) {
                    let hl = Rect::from_min_max(
                        sr.min + vec2(b.x0 * scale, b.y0 * scale),
                        sr.min + vec2(b.x1 * scale, b.y1 * scale),
                    );
                    p.rect_filled(hl, 0.0, fill);
                }
            }
        }
        // SyncTeX flash.
        if let Some((frects, started)) = &view.flash {
            let age = started.elapsed().as_secs_f32();
            let alpha = flash::alpha(age);
            if alpha > 0.0 {
                for fr in frects {
                    if let Some(page_rect) = rects.get(fr.page.saturating_sub(1) as usize) {
                        let r = Rect::from_min_size(
                            page_rect.min + vec2(fr.x * scale, fr.y * scale),
                            vec2(fr.width.max(4.0) * scale, fr.height.max(6.0) * scale),
                        )
                        .translate(origin)
                        .expand(2.0);
                        p.rect_filled(r, 3.0, t.sync_flash.gamma_multiply(alpha));
                        p.rect_stroke(
                            r,
                            3.0,
                            Stroke::new(1.0, t.accent.gamma_multiply(alpha * 0.8)),
                            StrokeKind::Outside,
                        );
                    }
                }
                if let Some(next) = flash::next_frame(age) {
                    ui.ctx().request_repaint_after(next);
                }
            } else {
                view.flash = None;
            }
        }
        // Scroll to a SyncTeX target.
        if let Some((target, force)) = view.scroll_to.take()
            && let Some(page_rect) = rects.get(target.page.saturating_sub(1) as usize)
        {
            let r = Rect::from_min_size(
                page_rect.min + vec2(target.x * scale, target.y * scale),
                vec2(
                    target.width.max(1.0) * scale,
                    target.height.max(1.0) * scale,
                ),
            )
            .translate(origin);
            if force || !clip.contains_rect(r) {
                ui.scroll_to_rect(r, Some(Align::Center));
            }
        }
        // Thumbnails: a click opens that page.
        if thumbs
            && resp.clicked()
            && !ui.input(|i| i.modifiers.command)
            && let Some(pointer) = resp.interact_pointer_pos()
        {
            thumb_pick = rects
                .iter()
                .position(|r| r.translate(origin).contains(pointer));
        }
        // ⌘-click: inverse search.
        if resp.clicked()
            && (ui.input(|i| i.modifiers.command) || session.view.single_click_sync)
            && !thumbs
            && let Some(pointer) = resp.interact_pointer_pos()
        {
            for (k, r) in rects.iter().enumerate() {
                let sr = r.translate(origin);
                if sr.contains(pointer) {
                    let x = (pointer.x - sr.left()) / scale;
                    let y = (pointer.y - sr.top()) / scale;
                    // The PDF text there lets the editor land on the clicked word.
                    let hint = view
                        .texts
                        .get(&k)
                        .and_then(|text| pdf_select::sync_hint(text, x, y));
                    inverse = Some((k as u32 + 1, x, y, hint));
                }
            }
        }
        // ⌘-scroll / pinch zoom.
        if resp.hovered() {
            let z = ui.input(|i| i.zoom_delta());
            if (z - 1.0).abs() > 0.001 {
                zoom_by = Some(z);
            }
        }
        // Current page: the first page whose bottom is below the top quarter of the view.
        let probe = clip.top() + clip.height() * 0.25;
        if !single
            && let Some(k) = rects
                .iter()
                .position(|r| r.translate(origin).bottom() >= probe)
        {
            view.current_page = k;
        }
    });
    let _ = out;

    if let Some(k) = thumb_pick {
        view.thumbnails = false;
        view.goto_page = Some(k);
    }
    if let Some(z) = zoom_by {
        session.view.zoom = Zoom::Fixed((scale * z / fit_w).clamp(0.1, 8.0));
    }
    if let Some((page, x, y, hint)) = inverse
        && let Err(e) = session.inverse_sync_with(page, x, y, hint.as_ref())
    {
        session.set_status(e.to_string());
    }
    view.publish_selection(session);
    if copy && let Err(e) = session.run("pdf.copySelection", &serde_json::Value::Null) {
        session.set_status(e.to_string());
    }

    // Request renders for visible pages (plus neighbours), at the current resolution.
    if let Some(worker) = &view.worker {
        let mut wanted = want.clone();
        if let (Some(&first), Some(&last)) = (want.first(), want.last()) {
            if first > 0 {
                wanted.push(first - 1);
            }
            wanted.push(last + 1);
        }
        for k in wanted.into_iter().filter(|&k| k < view.pages.len()) {
            let have = view.textures.get(&k).map(|tex| (tex.scale, tex.doc));
            let fresh = have.is_some_and(|(s, d)| {
                d == view.loaded_doc && (s / render_scale - 1.0).abs() < 0.05
            });
            let pending = view.requested.get(&k).is_some_and(|(s, d)| {
                *d == view.loaded_doc && (s / render_scale - 1.0).abs() < 0.05
            });
            if !fresh && !pending {
                worker.render(view.loaded_doc, k, render_scale);
                view.requested.insert(k, (render_scale, view.loaded_doc));
            }
        }
        // Text of the visible pages and of the selection's ends, after the renders.
        let mut need = want.clone();
        if let Some(sel) = &view.selection {
            need.extend([sel.anchor.page, sel.focus.page]);
        }
        for k in need.into_iter().filter(|&k| k < view.pages.len()) {
            if !view.texts.contains_key(&k) && view.text_requested.insert((view.loaded_doc, k)) {
                worker.text(view.loaded_doc, k);
            }
        }
        // Drop textures far from the view.
        let cur = view.current_page;
        view.textures.retain(|&k, _| k + 8 >= cur && k <= cur + 8);
        // And text, unless the selection is on it.
        let keep = view.selection.map(|s| {
            (
                s.anchor.page.min(s.focus.page),
                s.anchor.page.max(s.focus.page),
            )
        });
        let near = |k: usize| {
            (k + 24 >= cur && k <= cur + 24) || keep.is_some_and(|(a, b)| k >= a && k <= b)
        };
        let before = view.texts.len();
        view.texts.retain(|&k, _| near(k));
        if view.texts.len() != before {
            view.texts_version += 1;
        }
        let doc = view.loaded_doc;
        view.text_requested
            .retain(|&(d, k)| d == doc && (near(k) || view.texts.contains_key(&k)));
    }
}

fn empty_state(ui: &mut Ui, t: &Tokens, session: &Session, view: &PdfView, body: Rect) {
    let p = ui.painter();
    let c = body.center();
    if session.typeset.is_running() {
        let angle = ui.input(|i| i.time) as f32 * 5.0;
        icons::spinner(
            p,
            Rect::from_center_size(c - vec2(0.0, 20.0), vec2(28.0, 28.0)),
            t.text_weak,
            angle,
        );
        p.text(
            c + vec2(0.0, 14.0),
            egui::Align2::CENTER_CENTER,
            tr("Typesetting…"),
            egui::FontId::proportional(14.0),
            t.text_weak,
        );
        ui.ctx().request_repaint_after(Duration::from_millis(30));
    } else if let Some(err) = &view.error {
        p.text(
            c,
            egui::Align2::CENTER_CENTER,
            err,
            egui::FontId::proportional(13.0),
            t.error,
        );
    } else {
        p.text(
            c - vec2(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            tr("No PDF yet"),
            egui::FontId::proportional(18.0),
            t.text_weak.gamma_multiply(0.8),
        );
        p.text(
            c + vec2(0.0, 14.0),
            egui::Align2::CENTER_CENTER,
            tr("Typeset with ⌘T to see your document here"),
            egui::FontId::proportional(12.0),
            t.text_weak,
        );
    }
}

/// An item of the PDF header. When the pane is too narrow for all of them, the page label is
/// elided first (as in Texifier: "Pag…"), then buttons move into the ••• menu, lowest
/// [`rank`](HeaderItem::rank) first; ••• itself always stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum HeaderItem {
    Grid,
    ZoomOut,
    Zoom,
    ZoomIn,
    First,
    Prev,
    Next,
    Last,
    Lock,
    Open,
    Window,
    Pause,
    More,
}

impl HeaderItem {
    /// Every item, in the bar's order.
    pub(crate) const ALL: [Self; 13] = [
        Self::Grid,
        Self::ZoomOut,
        Self::Zoom,
        Self::ZoomIn,
        Self::First,
        Self::Prev,
        Self::Next,
        Self::Last,
        Self::Lock,
        Self::Open,
        Self::Window,
        Self::Pause,
        Self::More,
    ];

    /// How long the item stays in the bar as the pane narrows: the higher, the longer. Pairs
    /// go together.
    fn rank(self) -> u8 {
        match self {
            Self::Open => 0,
            Self::Window => 1,
            Self::First | Self::Last => 2,
            Self::Grid => 3,
            Self::Lock => 4,
            Self::ZoomOut | Self::ZoomIn => 5,
            Self::Pause => 6,
            Self::Prev | Self::Next => 7,
            Self::Zoom => TOP_RANK,
            Self::More => u8::MAX,
        }
    }

    /// The id of its interaction.
    pub(crate) fn id(self) -> Id {
        Id::new(("pdf-hdr", self))
    }
}

/// The highest rank that can give way.
const TOP_RANK: u8 = 8;
/// The least room between a cluster's edge and the anchor of its first item.
const EDGE: f32 = 10.0;

/// A slot of a header cluster: an item (`None`: a separator) and the room it takes before and
/// after its anchor (a glyph's centre, where the zoom percentage starts), counted from the
/// cluster's edge. At full width the anchors are the measured positions and neighbouring slots
/// meet halfway between them, so they never overlap.
#[derive(Debug, Clone, Copy)]
struct Slot {
    item: Option<HeaderItem>,
    lead: f32,
    trail: f32,
}

const fn slot(item: Option<HeaderItem>, lead: f32, trail: f32) -> Slot {
    Slot { item, lead, trail }
}

/// The left cluster from the pane's left edge: anchors at 12, 33.25, 42.75, 81.5, 95.25
/// (separator), 109 and 127 pt.
const LEFT: [Slot; 7] = [
    slot(Some(HeaderItem::Grid), 12.0, 10.625),
    slot(Some(HeaderItem::ZoomOut), 10.625, 7.5),
    slot(Some(HeaderItem::Zoom), 2.0, 28.25),
    slot(Some(HeaderItem::ZoomIn), 10.5, 6.875),
    slot(None, 6.875, 6.875),
    slot(Some(HeaderItem::First), 6.875, 9.0),
    slot(Some(HeaderItem::Prev), 9.0, 10.75),
];

/// The right cluster from the pane's right edge: anchors at 11 (•••), 34.25, 58.25, 81,
/// 104.5, 120.75 (separator), 135 and 153 pt.
const RIGHT: [Slot; 8] = [
    slot(Some(HeaderItem::More), 11.0, 11.625),
    slot(Some(HeaderItem::Pause), 11.625, 12.0),
    slot(Some(HeaderItem::Window), 12.0, 11.375),
    slot(Some(HeaderItem::Open), 11.375, 11.75),
    slot(Some(HeaderItem::Lock), 11.75, 8.125),
    slot(None, 8.125, 7.125),
    slot(Some(HeaderItem::Last), 7.125, 9.0),
    slot(Some(HeaderItem::Next), 9.0, 10.75),
];

/// A slot placed by [`pack`]: where its room starts, its anchor and where its room ends, as
/// distances from the cluster's edge.
#[derive(Debug, Clone, Copy)]
struct Placed {
    item: Option<HeaderItem>,
    near: f32,
    at: f32,
    far: f32,
}

/// Packs a cluster's items of rank `keep` and up towards its edge; a separator stays only
/// between two of them.
fn pack(slots: &[Slot], keep: u8) -> Vec<Placed> {
    let shown = |s: &Slot| s.item.is_some_and(|i| i.rank() >= keep);
    let mut out: Vec<Placed> = Vec::new();
    let mut x = 0.0;
    for (i, s) in slots.iter().enumerate() {
        let on = match s.item {
            Some(_) => shown(s),
            None => slots.iter().take(i).any(shown) && slots.iter().skip(i + 1).any(shown),
        };
        if !on {
            continue;
        }
        let lead = if out.is_empty() {
            s.lead.max(EDGE)
        } else {
            s.lead
        };
        out.push(Placed {
            item: s.item,
            near: x,
            at: x + lead,
            far: x + lead + s.trail,
        });
        x += lead + s.trail;
    }
    out
}

/// How far a packed cluster reaches from its edge.
fn reach(placed: &[Placed]) -> f32 {
    placed.last().map_or(0.0, |p| p.far)
}

/// The PDF header laid out in a given width.
#[derive(Debug)]
struct HeaderLayout {
    left: Vec<Placed>,
    right: Vec<Placed>,
    /// Items ranked below this are in the ••• menu instead.
    keep: u8,
}

impl HeaderLayout {
    /// Keeps as many items as fit in `width`, dropping the lowest ranks first.
    fn new(width: f32) -> Self {
        let mut keep = 0;
        loop {
            let (left, right) = (pack(&LEFT, keep), pack(&RIGHT, keep));
            if keep > TOP_RANK || reach(&left) + reach(&right) <= width {
                return Self { left, right, keep };
            }
            keep += 1;
        }
    }

    fn shown(&self, item: HeaderItem) -> bool {
        item.rank() >= self.keep
    }

    /// Where `item` is in a header spanning `pl..pr`: its room's left edge, its anchor and its
    /// room's right edge; `None` when it is in the ••• menu.
    fn place(&self, item: HeaderItem, pl: f32, pr: f32) -> Option<(f32, f32, f32)> {
        let is = |p: &&Placed| p.item == Some(item);
        self.left
            .iter()
            .find(is)
            .map(|p| (pl + p.near, pl + p.at, pl + p.far))
            .or_else(|| {
                self.right
                    .iter()
                    .find(is)
                    .map(|p| (pr - p.far, pr - p.at, pr - p.near))
            })
    }
}

fn header_bar(
    ui: &mut Ui,
    t: &Tokens,
    session: &mut Session,
    view: &mut PdfView,
    header: Rect,
    zoom_pct: f32,
) {
    let p = ui.painter().clone();
    p.rect_filled(header, 0.0, t.header_bg);
    p.line_segment(
        [header.left_bottom(), header.right_bottom()],
        Stroke::new(1.0, t.divider),
    );
    let (pl, pr, cy) = (header.left(), header.right(), header.center().y);
    let layout = HeaderLayout::new(header.width());
    let place = |item| layout.place(item, pl, pr);
    // An item's own room around a `w`-wide box centred on `cx`: never into its neighbours'.
    let room = |item, cx: f32, w: f32| {
        place(item).map(|(x0, _, x1)| {
            let r = Rect::from_center_size(pos2(cx, cy), vec2(w, 22.0));
            Rect::from_x_y_ranges(r.left().max(x0)..=r.right().min(x1), r.y_range())
        })
    };
    let n = view.pages.len();
    let has = n > 0;
    let cur = view.current_page;
    let last = n.saturating_sub(1);
    let run = |session: &mut Session, id: &str| {
        if let Err(e) = session.run(id, &serde_json::Value::Null) {
            session.set_status(e.to_string());
        }
    };
    // Page navigation: (label, enabled, page it goes to).
    let nav = |item| match item {
        HeaderItem::First => (tr("First Page"), has && cur > 0, 0),
        HeaderItem::Prev => (tr("Previous Page"), has && cur > 0, cur.saturating_sub(1)),
        HeaderItem::Next => (tr("Next Page"), has && cur < last, (cur + 1).min(last)),
        _ => (tr("Last Page"), has && cur < last, last),
    };
    // A glyph button `w` wide, unless the item is in the ••• menu → clicked.
    let button = |item: HeaderItem,
                  w: f32,
                  tip: &str,
                  enabled: bool,
                  draw: &dyn Fn(&egui::Painter, Rect, Color32)|
     -> bool {
        let Some((_, cx, _)) = place(item) else {
            return false;
        };
        let Some(r) = room(item, cx, w + 6.0) else {
            return false;
        };
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let resp = ui.interact(r, item.id(), sense).on_hover_text(tip);
        if enabled && resp.hovered() {
            p.rect_filled(r, 5.0, t.row_hover);
        }
        let c = if enabled {
            t.pdf_icon
        } else {
            t.pdf_icon_disabled
        };
        draw(&p, Rect::from_center_size(pos2(cx, cy), vec2(w, w)), c);
        resp.clicked()
    };
    for s in layout.left.iter().filter(|s| s.item.is_none()) {
        sep(&p, t, pl + s.at, cy);
    }
    for s in layout.right.iter().filter(|s| s.item.is_none()) {
        sep(&p, t, pr - s.at, cy);
    }

    // ---- Left cluster.
    let thumbs = view.thumbnails;
    if button(
        HeaderItem::Grid,
        16.0,
        tr("Page Thumbnails"),
        has,
        &|p, r, c| icons::grid_rounded(p, r, if thumbs { t.accent } else { c }),
    ) {
        view.thumbnails = !view.thumbnails;
    }
    if button(
        HeaderItem::ZoomOut,
        15.0,
        tr("Zoom Out (⌘−)"),
        has,
        &|p, r, c| icons::magnifier(p, r, c, Some('-')),
    ) {
        run(session, "pdf.zoomOut");
    }
    if let Some((_, x, _)) = place(HeaderItem::Zoom) {
        let pct = p.layout_no_wrap(
            format!("{zoom_pct:.0}%"),
            egui::FontId::proportional(10.75),
            t.pdf_icon,
        );
        let pct_rect = Rect::from_min_size(pos2(x, cy - pct.size().y / 2.0), pct.size());
        let hit = room(
            HeaderItem::Zoom,
            pct_rect.center().x,
            pct_rect.width() + 6.0,
        )
        .unwrap_or(pct_rect);
        let pct_resp = ui
            .interact(hit, HeaderItem::Zoom.id(), Sense::click())
            .on_hover_text(tr("Zoom"));
        p.galley(pct_rect.min, pct, t.pdf_icon);
        egui::Popup::from_toggle_button_response(&pct_resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
            .show(|ui| {
                menu::begin(ui, 170.0);
                for (label, z) in [
                    ("50%", 0.5),
                    ("75%", 0.75),
                    ("100%", 1.0),
                    ("125%", 1.25),
                    ("150%", 1.5),
                    ("200%", 2.0),
                    ("400%", 4.0),
                ] {
                    let opts = menu::Item {
                        checked: Some(session.view.zoom == Zoom::Fixed(z)),
                        ..Default::default()
                    };
                    if menu::item(ui, t, label, opts).clicked() {
                        session.view.zoom = Zoom::Fixed(z);
                    }
                }
                menu::separator(ui, t);
                for (zoom, label) in [
                    (Zoom::FitWidth, tr("Zoom to Fit Width")),
                    (Zoom::FitPage, tr("Zoom to Fit Page")),
                ] {
                    let opts = menu::Item {
                        checked: Some(session.view.zoom == zoom),
                        ..Default::default()
                    };
                    if menu::item(ui, t, label, opts).clicked() {
                        session.view.zoom = zoom;
                    }
                }
            });
    }
    if button(
        HeaderItem::ZoomIn,
        15.0,
        tr("Zoom In (⌘+)"),
        has,
        &|p, r, c| icons::magnifier(p, r, c, Some('+')),
    ) {
        run(session, "pdf.zoomIn");
    }
    let mut target = None;
    for (item, w, double, right) in [
        (HeaderItem::First, 17.0, true, false),
        (HeaderItem::Prev, 15.5, false, false),
        (HeaderItem::Next, 15.5, false, true),
        (HeaderItem::Last, 17.0, true, true),
    ] {
        let (tip, enabled, to) = nav(item);
        if button(item, w, tip, enabled, &|p, r, c| {
            if double {
                icons::chevron_double(p, r, c, right)
            } else {
                icons::chevron_thin(p, r, c, right)
            }
        }) {
            target = Some(to);
        }
    }

    // ---- Right cluster (anchored to the right edge).
    let lock = session.view.scroll_lock;
    if button(
        HeaderItem::Lock,
        15.0,
        tr("Scroll Lock: the PDF follows the editor"),
        true,
        &|p, r, c| icons::sync_lock(p, r, if lock { t.accent } else { c }),
    ) {
        run(session, "pdf.scrollLock");
    }
    if button(
        HeaderItem::Open,
        15.0,
        tr("Open PDF in Default Viewer"),
        session.pdf.path.is_some(),
        &|p, r, c| icons::open_external(p, r, c),
    ) {
        run(session, "pdf.openExternally");
    }
    let detached = view.window;
    if button(
        HeaderItem::Window,
        16.5,
        tr("Show the PDF in a Separate Window"),
        has,
        &|p, r, c| icons::windows_two(p, r, if detached { t.accent } else { c }),
    ) {
        view.window = !view.window;
    }
    let live = session.settings.live_typeset;
    let paused = session.live_paused;
    let pause_tip = if paused {
        tr("Resume Auto-Typeset")
    } else {
        tr("Pause Auto-Typeset")
    };
    if button(HeaderItem::Pause, 16.6, pause_tip, live, &|p, r, c| {
        if paused {
            icons::play(p, r, t.accent)
        } else {
            icons::pause_bold(p, r, c)
        }
    }) {
        run(session, "typeset.pauseLive");
    }
    let dots_x = place(HeaderItem::More).map_or(pr - 11.0, |(_, x, _)| x);
    let dots_r = room(HeaderItem::More, dots_x, 20.0)
        .unwrap_or_else(|| Rect::from_center_size(pos2(dots_x, cy), vec2(20.0, 22.0)));
    let dots = ui
        .interact(dots_r, HeaderItem::More.id(), Sense::click())
        .on_hover_text(tr("More"));
    if dots.hovered() {
        p.rect_filled(dots_r, 5.0, t.row_hover);
    }
    for k in [-1.0, 0.0, 1.0] {
        p.circle_filled(pos2(dots_x - 0.25 + 6.25 * k, cy + 0.5), 1.5, t.pdf_icon);
    }
    egui::Popup::from_toggle_button_response(&dots)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            menu::begin(ui, 250.0);
            // What the bar had no room for.
            let hidden: Vec<HeaderItem> = HeaderItem::ALL
                .into_iter()
                .filter(|&i| !layout.shown(i))
                .collect();
            for &item in &hidden {
                match item {
                    HeaderItem::Grid | HeaderItem::Window => {
                        let (label, on) = if item == HeaderItem::Grid {
                            (tr("Page Thumbnails"), &mut view.thumbnails)
                        } else {
                            (tr("Show the PDF in a Separate Window"), &mut view.window)
                        };
                        let opts = menu::Item {
                            checked: Some(*on),
                            disabled: !has,
                            ..Default::default()
                        };
                        if menu::item(ui, t, label, opts).clicked() {
                            *on = !*on;
                        }
                    }
                    HeaderItem::First | HeaderItem::Prev | HeaderItem::Next | HeaderItem::Last => {
                        let (label, enabled, to) = nav(item);
                        let opts = menu::Item {
                            checked: Some(false),
                            disabled: !enabled,
                            ..Default::default()
                        };
                        if menu::item(ui, t, label, opts).clicked() {
                            target = Some(to);
                        }
                    }
                    HeaderItem::Zoom => {
                        for id in ["pdf.fitWidth", "pdf.fitPage"] {
                            menu::command(ui, t, session, id, None, true);
                        }
                    }
                    HeaderItem::ZoomOut => {
                        menu::command(ui, t, session, "pdf.zoomOut", None, true);
                    }
                    HeaderItem::ZoomIn => menu::command(ui, t, session, "pdf.zoomIn", None, true),
                    HeaderItem::Lock => menu::command(ui, t, session, "pdf.scrollLock", None, true),
                    HeaderItem::Open => {
                        menu::command(ui, t, session, "pdf.openExternally", None, true);
                    }
                    HeaderItem::Pause => {
                        menu::command(ui, t, session, "typeset.pauseLive", None, true);
                    }
                    HeaderItem::More => {}
                }
            }
            if !hidden.is_empty() {
                menu::separator(ui, t);
            }
            menu::heading(ui, t, tr("Select Page Mode"));
            for id in [
                "pdf.singleContinuous",
                "pdf.twoContinuous",
                "pdf.singlePage",
            ] {
                menu::command(ui, t, session, id, None, true);
            }
            menu::separator(ui, t);
            menu::heading(ui, t, tr("Image Grading Type in Dark Mode"));
            for (grading, label) in [
                (Grading::Entire, tr("Grade Entire Viewer Output")),
                (Grading::None, tr("No Grading")),
            ] {
                let opts = menu::Item {
                    checked: Some(session.view.grading == grading),
                    ..Default::default()
                };
                if menu::item(ui, t, label, opts).clicked() {
                    session.view.grading = grading;
                }
            }
        });
    if let Some(p) = target {
        view.goto_page = Some(p);
    }
    // ---- Page label, centred between the two clusters (elided first, before any button
    // gives way).
    let left_end = pl + reach(&layout.left);
    let right_start = pr - reach(&layout.right);
    if has && right_start - left_end >= 24.0 {
        let font = egui::FontId::proportional(10.75);
        let label = widgets::elide(
            &p,
            &trf!("Page {page} of {pages}", page = cur + 1, pages = n),
            &font,
            right_start - left_end,
        );
        p.text(
            pos2((left_end + right_start) / 2.0, cy),
            egui::Align2::CENTER_CENTER,
            label,
            font,
            t.jump_text,
        );
    }
}

/// A vertical hairline between two groups of header buttons.
fn sep(p: &egui::Painter, t: &Tokens, x: f32, cy: f32) {
    p.line_segment(
        [pos2(x, cy - 8.75), pos2(x, cy + 8.75)],
        Stroke::new(1.0, t.divider.gamma_multiply(0.8)),
    );
}

impl PdfView {
    /// For tests and the control channel: which pages have textures.
    pub fn rendered_pages(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.textures.keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// The id used for the scroll area.
    pub fn scroll_id() -> Id {
        Id::new("pdf-scroll")
    }

    /// Screen-independent scale last used.
    pub fn scale(&self) -> f32 {
        self.last_scale
    }

    /// The rectangle of point `p` (unused helper kept for symmetry with tests).
    pub fn point(p: Pos2) -> Pos2 {
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grading_maps_paper_and_ink() {
        let mut px = vec![255, 255, 255, 255, 0, 0, 0, 255, 255, 0, 0, 255];
        grade_dark(&mut px);
        assert_eq!(&px[0..3], &[46, 46, 46]);
        assert_eq!(&px[4..7], &[180, 180, 180]);
        // Red stays reddish.
        assert!(px[8] > px[9] && px[8] > px[10]);
    }

    /// The header keeps its measured positions while everything fits; narrower, the lowest
    /// ranks go into the ••• menu and what stays never overlaps or leaves the pane.
    #[test]
    fn header_items_give_way_by_rank() {
        let full = HeaderLayout::new(400.0);
        assert_eq!(full.keep, 0, "everything fits");
        let at = |item| full.place(item, 0.0, 400.0).map(|(_, x, _)| x);
        for (item, x) in [
            (HeaderItem::Grid, 12.0),
            (HeaderItem::ZoomOut, 33.25),
            (HeaderItem::Zoom, 42.75),
            (HeaderItem::ZoomIn, 81.5),
            (HeaderItem::First, 109.0),
            (HeaderItem::Prev, 127.0),
            (HeaderItem::Next, 400.0 - 153.0),
            (HeaderItem::Last, 400.0 - 135.0),
            (HeaderItem::Lock, 400.0 - 104.5),
            (HeaderItem::Open, 400.0 - 81.0),
            (HeaderItem::Window, 400.0 - 58.25),
            (HeaderItem::Pause, 400.0 - 34.25),
            (HeaderItem::More, 400.0 - 11.0),
        ] {
            assert_eq!(at(item), Some(x), "{item:?}");
        }
        let mut keep = 0;
        for w in (0..=400).map(|w| w as f32) {
            let l = HeaderLayout::new(w);
            let mut slots: Vec<(f32, f32)> = (l.left.iter().map(|p| (p.near, p.far)))
                .chain(l.right.iter().map(|p| (w - p.far, w - p.near)))
                .collect();
            slots.sort_by(|a, b| a.0.total_cmp(&b.0));
            for pair in slots.windows(2) {
                assert!(pair[0].1 <= pair[1].0, "{w}: {pair:?} overlap");
            }
            if l.keep <= TOP_RANK {
                assert!(
                    slots.iter().all(|s| s.0 >= 0.0 && s.1 <= w),
                    "{w}: {slots:?}"
                );
            }
            // Widening never hides anything.
            assert!(l.keep <= keep || w == 0.0, "{w}");
            keep = l.keep;
            assert!(l.shown(HeaderItem::More));
            for item in HeaderItem::ALL {
                assert_eq!(
                    l.place(item, 0.0, w).is_some(),
                    l.shown(item),
                    "{w} {item:?}"
                );
            }
        }
        // About 140 pt (an 880 pt window with the sidebar and the Assistant open): the zoom
        // percentage and the page arrows stay, the rest is in the ••• menu.
        let narrow = HeaderLayout::new(140.0);
        let shown: Vec<HeaderItem> = HeaderItem::ALL
            .into_iter()
            .filter(|&i| narrow.shown(i))
            .collect();
        assert_eq!(
            shown,
            [
                HeaderItem::Zoom,
                HeaderItem::Prev,
                HeaderItem::Next,
                HeaderItem::More
            ]
        );
    }
}
