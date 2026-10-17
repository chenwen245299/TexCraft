//! Vector icons drawn with the painter (original artwork; no icon files). Each draws into `r`
//! with colour `c`, on a nominal 16 pt grid scaled to the rect.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

fn s(c: Color32, w: f32) -> Stroke {
    Stroke::new(w, c)
}

/// Maps a point on a 16×16 grid into `r` (square, centered).
fn g(r: Rect, x: f32, y: f32) -> Pos2 {
    let side = r.width().min(r.height());
    let o = r.center() - vec2(side / 2.0, side / 2.0);
    pos2(o.x + x / 16.0 * side, o.y + y / 16.0 * side)
}

fn gr(r: Rect, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
    Rect::from_min_max(g(r, x0, y0), g(r, x1, y1))
}

fn unit(r: Rect) -> f32 {
    r.width().min(r.height()) / 16.0
}

/// Sidebar toggle: a window with a left column.
pub fn sidebar(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    let frame = gr(r, 1.5, 3.0, 14.5, 13.0);
    p.rect_stroke(frame, 2.0 * u, s(c, 1.2 * u), StrokeKind::Middle);
    p.line_segment([g(r, 6.0, 3.0), g(r, 6.0, 13.0)], s(c, 1.2 * u));
    for y in [5.5, 7.5, 9.5] {
        p.line_segment([g(r, 3.2, y), g(r, 4.6, y)], s(c, 1.0 * u));
    }
}

fn gear(p: &Painter, center: Pos2, radius: f32, teeth: usize, angle: f32, c: Color32, w: f32) {
    let mut pts = Vec::with_capacity(teeth * 4);
    let outer = radius;
    let inner = radius * 0.74;
    let n = teeth as f32;
    for i in 0..teeth {
        let a0 = angle + (i as f32) / n * std::f32::consts::TAU;
        let step = std::f32::consts::TAU / n;
        for (k, rad) in [(0.0, inner), (0.18, outer), (0.5, outer), (0.68, inner)] {
            let a = a0 + k * step;
            pts.push(center + vec2(a.cos(), a.sin()) * rad);
        }
    }
    p.add(Shape::closed_line(pts, s(c, w)));
    p.circle_stroke(center, radius * 0.32, s(c, w));
}

/// Typeset: two interlocking cogs; `angle` spins them while typesetting.
pub fn cogs(p: &Painter, r: Rect, c: Color32, angle: f32) {
    let u = unit(r);
    gear(p, g(r, 6.2, 9.4), 5.0 * u, 8, angle, c, 1.2 * u);
    gear(
        p,
        g(r, 12.0, 4.6),
        3.2 * u,
        6,
        -angle * 1.6 + 0.3,
        c,
        1.1 * u,
    );
}

/// Typeset configuration: three faders.
pub fn sliders(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    for (x, knob) in [(4.0, 10.0), (8.0, 5.5), (12.0, 8.5)] {
        p.line_segment([g(r, x, 2.5), g(r, x, 13.5)], s(c, 1.2 * u));
        p.circle_filled(g(r, x, knob), 1.8 * u, c);
    }
}

/// Success: a circled check mark.
pub fn check_circle(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.circle_stroke(g(r, 8.0, 8.0), 6.3 * u, s(c, 1.3 * u));
    p.add(Shape::line(
        vec![g(r, 5.0, 8.3), g(r, 7.2, 10.5), g(r, 11.2, 5.8)],
        s(c, 1.5 * u),
    ));
}

/// Share / export: a tray with an up arrow.
pub fn share(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.add(Shape::line(
        vec![
            g(r, 5.5, 6.0),
            g(r, 3.5, 6.0),
            g(r, 3.5, 14.5),
            g(r, 12.5, 14.5),
            g(r, 12.5, 6.0),
            g(r, 10.5, 6.0),
        ],
        s(c, 1.2 * u),
    ));
    p.line_segment([g(r, 8.0, 1.5), g(r, 8.0, 10.0)], s(c, 1.2 * u));
    p.add(Shape::line(
        vec![g(r, 5.5, 4.0), g(r, 8.0, 1.5), g(r, 10.5, 4.0)],
        s(c, 1.2 * u),
    ));
}

/// View mode segment: editor only (an empty page).
pub fn view_editor(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 2.5, 3.5, 13.5, 12.5),
        1.5 * u,
        s(c, 1.2 * u),
        StrokeKind::Middle,
    );
}

/// View mode segment: editor and PDF.
pub fn view_split(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 2.5, 3.5, 13.5, 12.5),
        1.5 * u,
        s(c, 1.2 * u),
        StrokeKind::Middle,
    );
    p.rect_filled(gr(r, 8.0, 3.5, 13.5, 12.5), 1.5 * u, c);
}

/// View mode segment: PDF only.
pub fn view_pdf(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_filled(gr(r, 2.5, 3.5, 13.5, 12.5), 1.5 * u, c);
}

/// `<`
pub fn chevron_left(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.add(Shape::line(
        vec![g(r, 10.0, 3.0), g(r, 5.5, 8.0), g(r, 10.0, 13.0)],
        s(c, 1.5 * u),
    ));
}

/// `>`
pub fn chevron_right(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.add(Shape::line(
        vec![g(r, 6.0, 3.0), g(r, 10.5, 8.0), g(r, 6.0, 13.0)],
        s(c, 1.5 * u),
    ));
}

/// `v`
pub fn chevron_down(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.add(Shape::line(
        vec![g(r, 4.0, 6.0), g(r, 8.0, 10.0), g(r, 12.0, 6.0)],
        s(c, 1.4 * u),
    ));
}

/// Disclosure triangle, pointing right (`open == false`) or down.
pub fn disclosure(p: &Painter, r: Rect, c: Color32, open: bool) {
    let pts = if open {
        vec![g(r, 4.5, 6.0), g(r, 11.5, 6.0), g(r, 8.0, 10.5)]
    } else {
        vec![g(r, 6.0, 4.5), g(r, 10.5, 8.0), g(r, 6.0, 11.5)]
    };
    p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
}

/// New tab: a window with a plus badge.
pub fn new_tab(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 1.5, 4.5, 12.0, 13.5),
        1.5 * u,
        s(c, 1.2 * u),
        StrokeKind::Middle,
    );
    p.line_segment([g(r, 12.5, 0.8), g(r, 12.5, 6.2)], s(c, 1.2 * u));
    p.line_segment([g(r, 9.8, 3.5), g(r, 15.2, 3.5)], s(c, 1.2 * u));
}

/// Contents tab: a page inside corner brackets.
pub fn contents(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    let w = s(c, 1.2 * u);
    for (a, b, d) in [
        ((2.0, 5.0), (2.0, 2.0), (5.0, 2.0)),
        ((11.0, 2.0), (14.0, 2.0), (14.0, 5.0)),
        ((2.0, 11.0), (2.0, 14.0), (5.0, 14.0)),
        ((11.0, 14.0), (14.0, 14.0), (14.0, 11.0)),
    ] {
        p.add(Shape::line(
            vec![g(r, a.0, a.1), g(r, b.0, b.1), g(r, d.0, d.1)],
            w,
        ));
    }
    for y in [6.0, 8.0, 10.0] {
        p.line_segment([g(r, 5.5, y), g(r, 10.5, y)], s(c, 1.0 * u));
    }
}

/// TODOs tab: a checked box.
pub fn todo(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 2.5, 2.5, 13.5, 13.5),
        2.5 * u,
        s(c, 1.2 * u),
        StrokeKind::Middle,
    );
    p.add(Shape::line(
        vec![g(r, 5.0, 8.2), g(r, 7.2, 10.5), g(r, 11.3, 5.5)],
        s(c, 1.4 * u),
    ));
}

/// Folder.
pub fn folder(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.add(Shape::closed_line(
        vec![
            g(r, 1.5, 4.0),
            g(r, 6.0, 4.0),
            g(r, 7.5, 5.5),
            g(r, 14.5, 5.5),
            g(r, 14.5, 13.0),
            g(r, 1.5, 13.0),
        ],
        s(c, 1.2 * u),
    ));
}

/// A filled folder (file lists).
pub fn folder_filled(p: &Painter, r: Rect, c: Color32) {
    p.add(Shape::convex_polygon(
        vec![
            g(r, 1.5, 4.0),
            g(r, 6.0, 4.0),
            g(r, 7.5, 5.5),
            g(r, 14.5, 5.5),
            g(r, 14.5, 13.0),
            g(r, 1.5, 13.0),
        ],
        c,
        Stroke::NONE,
    ));
}

/// A folder in points, filling `r` (Files list: 14.5 × 11.5 pt): a tab at the top left and a
/// slightly lighter front.
pub fn folder_pt(p: &Painter, r: Rect, c: Color32) {
    let back = c.gamma_multiply(0.85);
    p.rect_filled(
        Rect::from_min_size(r.min, vec2(r.width() * 0.42, 3.4)),
        1.3,
        back,
    );
    p.rect_filled(Rect::from_min_max(r.min + vec2(0.0, 1.6), r.max), 1.6, back);
    p.rect_filled(Rect::from_min_max(r.min + vec2(0.0, 3.3), r.max), 1.6, c);
}

/// A document in points, filling `r` (Files list: 12 × 15 pt): folded top-right corner and
/// the file type in small capitals at the bottom.
pub fn document_pt(p: &Painter, r: Rect, c: Color32, label: &str) {
    let fold = 3.6;
    let w = Stroke::new(0.9, c);
    let (l, t, rt, b) = (
        r.left() + 0.45,
        r.top() + 0.45,
        r.right() - 0.45,
        r.bottom() - 0.45,
    );
    p.add(Shape::closed_line(
        vec![
            pos2(l, t),
            pos2(rt - fold, t),
            pos2(rt, t + fold),
            pos2(rt, b),
            pos2(l, b),
        ],
        w,
    ));
    p.add(Shape::line(
        vec![
            pos2(rt - fold, t),
            pos2(rt - fold, t + fold),
            pos2(rt, t + fold),
        ],
        w,
    ));
    if !label.is_empty() {
        p.text(
            pos2(r.center().x, b - 3.6),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(4.4),
            c,
        );
    }
}

/// Warning triangle (Typeset Results tab, issue rows).
pub fn warning(p: &Painter, r: Rect, c: Color32, filled: bool) {
    let u = unit(r);
    let pts = vec![g(r, 8.0, 1.8), g(r, 15.0, 14.0), g(r, 1.0, 14.0)];
    if filled {
        p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
        let fg = Color32::WHITE;
        p.line_segment([g(r, 8.0, 6.0), g(r, 8.0, 10.0)], s(fg, 1.5 * u));
        p.circle_filled(g(r, 8.0, 12.0), 0.9 * u, fg);
    } else {
        p.add(Shape::closed_line(pts, s(c, 1.2 * u)));
        p.line_segment([g(r, 8.0, 6.0), g(r, 8.0, 10.0)], s(c, 1.3 * u));
        p.circle_filled(g(r, 8.0, 12.0), 0.8 * u, c);
    }
}

/// Magnifier, with an optional `+`/`-` inside.
pub fn magnifier(p: &Painter, r: Rect, c: Color32, sign: Option<char>) {
    let u = unit(r);
    p.circle_stroke(g(r, 6.8, 6.8), 4.6 * u, s(c, 1.3 * u));
    p.line_segment([g(r, 10.2, 10.2), g(r, 14.2, 14.2)], s(c, 1.8 * u));
    if let Some(sign) = sign {
        p.line_segment([g(r, 4.6, 6.8), g(r, 9.0, 6.8)], s(c, 1.2 * u));
        if sign == '+' {
            p.line_segment([g(r, 6.8, 4.6), g(r, 6.8, 9.0)], s(c, 1.2 * u));
        }
    }
}

/// Scroll lock: a page with a two-way arrow.
pub fn scroll_lock(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 4.0, 1.5, 12.0, 14.5),
        1.5 * u,
        s(c, 1.2 * u),
        StrokeKind::Middle,
    );
    p.line_segment([g(r, 1.5, 8.0), g(r, 14.5, 8.0)], s(c, 1.2 * u));
    p.add(Shape::line(
        vec![g(r, 3.5, 6.0), g(r, 1.5, 8.0), g(r, 3.5, 10.0)],
        s(c, 1.2 * u),
    ));
    p.add(Shape::line(
        vec![g(r, 12.5, 6.0), g(r, 14.5, 8.0), g(r, 12.5, 10.0)],
        s(c, 1.2 * u),
    ));
}

/// Pause: two bars.
pub fn pause(p: &Painter, r: Rect, c: Color32) {
    p.rect_filled(gr(r, 4.5, 3.0, 6.8, 13.0), 0.8 * unit(r), c);
    p.rect_filled(gr(r, 9.2, 3.0, 11.5, 13.0), 0.8 * unit(r), c);
}

/// Play: a triangle.
pub fn play(p: &Painter, r: Rect, c: Color32) {
    p.add(Shape::convex_polygon(
        vec![g(r, 5.0, 3.0), g(r, 13.0, 8.0), g(r, 5.0, 13.0)],
        c,
        Stroke::NONE,
    ));
}

/// More: three dots.
pub fn dots(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    for x in [3.5, 8.0, 12.5] {
        p.circle_filled(g(r, x, 8.0), 1.3 * u, c);
    }
}

/// Page thumbnails: a 3×2 grid.
pub fn grid(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    for (x, y) in [
        (1.5, 3.0),
        (6.0, 3.0),
        (10.5, 3.0),
        (1.5, 8.5),
        (6.0, 8.5),
        (10.5, 8.5),
    ] {
        p.rect_stroke(
            gr(r, x, y, x + 4.0, y + 4.5),
            0.8 * u,
            s(c, 1.0 * u),
            StrokeKind::Middle,
        );
    }
}

/// A document glyph with a short label (`TEX`, `BIB`, `IMG`, `PDF`).
pub fn document(p: &Painter, r: Rect, c: Color32, label: &str) {
    let u = unit(r);
    p.add(Shape::closed_line(
        vec![
            g(r, 3.0, 1.0),
            g(r, 10.0, 1.0),
            g(r, 13.0, 4.0),
            g(r, 13.0, 15.0),
            g(r, 3.0, 15.0),
        ],
        s(c, 1.0 * u),
    ));
    p.add(Shape::line(
        vec![g(r, 10.0, 1.0), g(r, 10.0, 4.0), g(r, 13.0, 4.0)],
        s(c, 1.0 * u),
    ));
    if !label.is_empty() {
        p.text(
            g(r, 8.0, 10.5),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(4.6 * u),
            c,
        );
    }
}

/// Circle-dot (root file).
pub fn root_dot(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.circle_stroke(g(r, 8.0, 8.0), 5.5 * u, s(c, 1.2 * u));
    p.circle_filled(g(r, 8.0, 8.0), 2.3 * u, c);
}

/// Plus.
pub fn plus(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.line_segment([g(r, 8.0, 3.0), g(r, 8.0, 13.0)], s(c, 1.4 * u));
    p.line_segment([g(r, 3.0, 8.0), g(r, 13.0, 8.0)], s(c, 1.4 * u));
}

/// Filter (funnel).
pub fn filter(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.add(Shape::closed_line(
        vec![
            g(r, 2.0, 3.0),
            g(r, 14.0, 3.0),
            g(r, 9.5, 8.5),
            g(r, 9.5, 13.5),
            g(r, 6.5, 12.0),
            g(r, 6.5, 8.5),
        ],
        s(c, 1.1 * u),
    ));
}

/// Close (×).
pub fn close(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.line_segment([g(r, 4.5, 4.5), g(r, 11.5, 11.5)], s(c, 1.3 * u));
    p.line_segment([g(r, 11.5, 4.5), g(r, 4.5, 11.5)], s(c, 1.3 * u));
}

/// A rounded badge with a letter (outline `C`/`S`/`s` markers).
pub fn letter_badge(
    p: &Painter,
    r: Rect,
    fill: Color32,
    fg: Color32,
    letter: &str,
    outlined: bool,
) {
    let rr = r.shrink(1.0);
    if outlined {
        p.rect_stroke(rr, 3.0, s(fill, 1.0), StrokeKind::Inside);
        p.text(
            rr.center(),
            egui::Align2::CENTER_CENTER,
            letter,
            egui::FontId::proportional(rr.height() * 0.68),
            fill,
        );
    } else {
        p.rect_filled(rr, 3.0, fill);
        p.text(
            rr.center(),
            egui::Align2::CENTER_CENTER,
            letter,
            egui::FontId::proportional(rr.height() * 0.68),
            fg,
        );
    }
}

/// An outlined square (figure/table/label markers in the outline).
pub fn square_marker(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 3.5, 3.5, 12.5, 12.5),
        2.0 * u,
        s(c, 1.4 * u),
        StrokeKind::Middle,
    );
}

/// Hourglass-like begin/end document marker.
pub fn bookend(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.line_segment([g(r, 4.0, 3.0), g(r, 12.0, 3.0)], s(c, 1.3 * u));
    p.line_segment([g(r, 4.0, 13.0), g(r, 12.0, 13.0)], s(c, 1.3 * u));
    p.add(Shape::closed_line(
        vec![
            g(r, 5.0, 3.0),
            g(r, 11.0, 3.0),
            g(r, 5.0, 13.0),
            g(r, 11.0, 13.0),
        ],
        s(c, 1.0 * u),
    ));
}

/// Star in a square (tag search results).
pub fn star_square(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 1.5, 1.5, 14.5, 14.5),
        2.5 * u,
        s(c, 1.0 * u),
        StrokeKind::Middle,
    );
    let center = g(r, 8.0, 8.3);
    let mut pts = Vec::new();
    for i in 0..10 {
        let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        let rad = if i % 2 == 0 { 4.6 * u } else { 2.0 * u };
        pts.push(center + vec2(a.cos(), a.sin()) * rad);
    }
    p.add(Shape::closed_line(pts, s(c, 1.0 * u)));
}

/// Table of contents: lines with bullets.
pub fn list(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 1.5, 2.5, 14.5, 13.5),
        2.0 * u,
        s(c, 1.1 * u),
        StrokeKind::Middle,
    );
    for y in [5.5, 8.0, 10.5] {
        p.circle_filled(g(r, 4.2, y), 0.8 * u, c);
        p.line_segment([g(r, 6.0, y), g(r, 12.5, y)], s(c, 1.0 * u));
    }
}

/// Figures: a framed picture with a mountain.
pub fn picture(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 1.5, 2.5, 14.5, 13.5),
        2.0 * u,
        s(c, 1.1 * u),
        StrokeKind::Middle,
    );
    p.add(Shape::line(
        vec![
            g(r, 2.5, 12.0),
            g(r, 6.5, 7.5),
            g(r, 9.5, 10.5),
            g(r, 11.0, 9.0),
            g(r, 13.5, 11.5),
        ],
        s(c, 1.1 * u),
    ));
    p.circle_filled(g(r, 11.0, 5.5), 1.2 * u, c);
}

/// Tables: a grid.
pub fn table(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 1.5, 2.5, 14.5, 13.5),
        1.5 * u,
        s(c, 1.1 * u),
        StrokeKind::Middle,
    );
    p.line_segment([g(r, 1.5, 6.2), g(r, 14.5, 6.2)], s(c, 1.0 * u));
    p.line_segment([g(r, 1.5, 9.9), g(r, 14.5, 9.9)], s(c, 1.0 * u));
    p.line_segment([g(r, 6.0, 2.5), g(r, 6.0, 13.5)], s(c, 1.0 * u));
    p.line_segment([g(r, 10.5, 2.5), g(r, 10.5, 13.5)], s(c, 1.0 * u));
}

/// Labels: a tag in a circle.
pub fn label_circle(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.circle_stroke(g(r, 8.0, 8.0), 6.6 * u, s(c, 1.1 * u));
    p.add(Shape::closed_line(
        vec![
            g(r, 5.0, 4.5),
            g(r, 9.0, 4.5),
            g(r, 11.5, 8.0),
            g(r, 9.0, 11.5),
            g(r, 5.0, 11.5),
        ],
        s(c, 1.0 * u),
    ));
}

/// Bibliography: a book in a circle.
pub fn book_circle(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.circle_stroke(g(r, 8.0, 8.0), 6.6 * u, s(c, 1.1 * u));
    p.line_segment([g(r, 6.0, 4.5), g(r, 6.0, 11.5)], s(c, 1.1 * u));
    p.line_segment([g(r, 8.0, 4.5), g(r, 8.0, 11.5)], s(c, 1.1 * u));
    p.line_segment([g(r, 9.6, 4.8), g(r, 11.2, 11.2)], s(c, 1.1 * u));
}

/// Gear (settings menu).
pub fn gear_icon(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    let center = g(r, 8.0, 8.0);
    let mut pts = Vec::new();
    for i in 0..8 {
        let a0 = i as f32 / 8.0 * std::f32::consts::TAU;
        let step = std::f32::consts::TAU / 8.0;
        for (k, rad) in [(0.0, 4.6), (0.15, 6.4), (0.45, 6.4), (0.6, 4.6)] {
            let a = a0 + k * step;
            pts.push(center + vec2(a.cos(), a.sin()) * rad * u);
        }
    }
    p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
    p.circle_filled(center, 2.0 * u, Color32::TRANSPARENT);
    p.circle_stroke(center, 1.9 * u, s(Color32::WHITE, 1.3 * u));
}

/// Gutter error badge: a tag-shaped red flag behind the line number.
pub fn error_flag(p: &Painter, r: Rect, fill: Color32) {
    let h = r.height();
    let tip = h * 0.35;
    p.add(Shape::convex_polygon(
        vec![
            r.left_top(),
            pos2(r.right() - tip, r.top()),
            pos2(r.right(), r.center().y),
            pos2(r.right() - tip, r.bottom()),
            r.left_bottom(),
        ],
        fill,
        Stroke::NONE,
    ));
}

/// Spinner arc for progress (angle in radians).
pub fn spinner(p: &Painter, r: Rect, c: Color32, angle: f32) {
    let u = unit(r);
    let center = g(r, 8.0, 8.0);
    let n = 24;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = angle + i as f32 / n as f32 * std::f32::consts::PI * 1.5;
            center + vec2(a.cos(), a.sin()) * 5.5 * u
        })
        .collect();
    p.add(Shape::line(pts, s(c, 1.6 * u)));
}

// ---------------------------------------------------------------------------------------------
// Icons matched to the target's macOS 26 screenshots (filled gears, view-mode glyphs, sidebar
// group icons, PDF header glyphs). Drawn on a 16-unit grid like the ones above.

/// One filled gear: a body disc plus `teeth` tapered teeth (convex pieces, so they fill and
/// anti-alias cleanly) and a hole in `bg`. Radii in points; `phase` puts a tooth at that angle
/// (radians, clockwise from +x); `base_half` / `tip_half` are a tooth's half-widths (radians).
#[allow(clippy::too_many_arguments)]
fn toothed_gear(
    p: &Painter,
    center: Pos2,
    tip: f32,
    body: f32,
    hole: f32,
    teeth: usize,
    phase: f32,
    (base_half, tip_half): (f32, f32),
    c: Color32,
    bg: Color32,
) {
    let step = std::f32::consts::TAU / teeth as f32;
    let at = |a: f32, r: f32| center + vec2(a.cos(), a.sin()) * r;
    p.circle_filled(center, body, c);
    for i in 0..teeth {
        let a = phase + i as f32 * step;
        let base = body - 0.4;
        p.add(Shape::convex_polygon(
            vec![
                at(a - base_half, base),
                at(a - tip_half, tip),
                at(a + tip_half, tip),
                at(a + base_half, base),
            ],
            c,
            Stroke::NONE,
        ));
    }
    p.circle_filled(center, hole, bg);
}

/// The Typeset gears measured from the target (2 px per pt): a 10-tooth gear 14 pt across and a
/// 6-tooth gear 8.6 pt across, below right of it. `o` is the top-left of the 24 × 24 pt slot;
/// `k` scales. While typesetting the two turn together (`angle`, radians).
fn gears_at(p: &Painter, o: Pos2, k: f32, c: Color32, bg: Color32, angle: f32) {
    let deg = |d: f32| d.to_radians();
    toothed_gear(
        p,
        o + vec2(8.56, 10.0) * k,
        7.0 * k,
        5.4 * k,
        2.05 * k,
        10,
        deg(90.0) + angle,
        (deg(11.5), deg(7.0)),
        c,
        bg,
    );
    // Meshes with the big one: turns the other way, 10/6 as fast.
    toothed_gear(
        p,
        o + vec2(20.06, 15.8) * k,
        4.3 * k,
        3.2 * k,
        1.45 * k,
        6,
        deg(10.0) - angle * 10.0 / 6.0,
        (deg(17.5), deg(8.0)),
        c,
        bg,
    );
}

/// Typeset gears in toolbar points: `slot` is the top-left of their 24 × 24 pt slot.
pub fn gears_pt(p: &Painter, slot: Pos2, c: Color32, bg: Color32, angle: f32) {
    gears_at(p, slot, 1.0, c, bg, angle);
}

/// Typeset gears scaled into `r` (preference pane icons).
pub fn gears_filled(p: &Painter, r: Rect, c: Color32, bg: Color32, angle: f32) {
    // The drawing spans x 1.6–24.4, y 3.0–20.1 of its slot; centre that in `r`.
    let k = r.width().min(r.height()) * 0.95 / 22.8;
    let o = r.center() - vec2(12.96, 11.55) * k;
    gears_at(p, o, k, c, bg, angle);
}

/// Thin chevron `<` / `>` (navigation).
pub fn chevron_thin(p: &Painter, r: Rect, c: Color32, right: bool) {
    let u = unit(r);
    let pts = if right {
        vec![g(r, 5.5, 2.0), g(r, 11.0, 8.0), g(r, 5.5, 14.0)]
    } else {
        vec![g(r, 10.5, 2.0), g(r, 5.0, 8.0), g(r, 10.5, 14.0)]
    };
    p.add(Shape::line(pts, s(c, 1.2 * u)));
}

/// Double chevron `«` / `»`.
pub fn chevron_double(p: &Painter, r: Rect, c: Color32, right: bool) {
    let u = unit(r);
    for dx in [-2.5f32, 2.5] {
        let pts = if right {
            vec![
                g(r, 4.5 + dx, 2.5),
                g(r, 9.5 + dx, 8.0),
                g(r, 4.5 + dx, 13.5),
            ]
        } else {
            vec![
                g(r, 11.5 + dx, 2.5),
                g(r, 6.5 + dx, 8.0),
                g(r, 11.5 + dx, 13.5),
            ]
        };
        p.add(Shape::line(pts, s(c, 1.2 * u)));
    }
}

/// Sliders: three vertical lines with filled knobs.
pub fn sliders_filled(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    for (x, knob) in [(3.5, 10.0), (8.0, 4.5), (12.5, 8.0)] {
        p.line_segment([g(r, x, 1.0), g(r, x, 15.0)], s(c, 1.3 * u));
        p.rect_filled(
            Rect::from_center_size(g(r, x, knob), vec2(3.6 * u, 2.6 * u)),
            0.8 * u,
            c,
        );
    }
}

/// Success: a thin circle with a check mark.
pub fn check_thin(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.circle_stroke(g(r, 8.0, 8.0), 7.0 * u, s(c, 1.1 * u));
    p.add(Shape::line(
        vec![g(r, 4.6, 8.4), g(r, 7.0, 10.8), g(r, 11.5, 5.4)],
        s(c, 1.2 * u),
    ));
}

/// Sidebar toggle as drawn inside the blue disc: a window with a left column of lines.
pub fn sidebar_glyph(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 1.0, 2.5, 15.0, 13.5),
        2.2 * u,
        s(c, 1.3 * u),
        StrokeKind::Inside,
    );
    p.line_segment([g(r, 6.0, 2.5), g(r, 6.0, 13.5)], s(c, 1.3 * u));
    for y in [5.3, 7.3, 9.3] {
        p.line_segment([g(r, 2.6, y), g(r, 4.4, y)], s(c, 1.0 * u));
    }
}

/// Contents tab: a page with lines inside corner brackets.
pub fn viewfinder_doc(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    let w = s(c, 1.2 * u);
    for (a, b, d) in [
        ((1.5, 4.5), (1.5, 1.5), (4.5, 1.5)),
        ((11.5, 1.5), (14.5, 1.5), (14.5, 4.5)),
        ((1.5, 11.5), (1.5, 14.5), (4.5, 14.5)),
        ((11.5, 14.5), (14.5, 14.5), (14.5, 11.5)),
    ] {
        p.add(Shape::line(
            vec![g(r, a.0, a.1), g(r, b.0, b.1), g(r, d.0, d.1)],
            w,
        ));
    }
    p.rect_filled(gr(r, 5.0, 4.5, 11.0, 11.5), 1.0 * u, c);
}

/// Checkbox with a check (TODOs tab).
pub fn checkbox_checked(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 1.5, 1.5, 14.5, 14.5),
        2.0 * u,
        s(c, 1.1 * u),
        StrokeKind::Middle,
    );
    p.add(Shape::line(
        vec![g(r, 4.5, 8.2), g(r, 7.0, 10.8), g(r, 11.5, 4.8)],
        s(c, 1.2 * u),
    ));
}

/// Infinity sign (Connect tab).
pub fn infinity(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    let n = 48;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            let d = 1.0 + t.sin().powi(2);
            g(
                r,
                8.0 + 6.8 * t.cos() / d,
                8.0 + 6.8 * t.sin() * t.cos() / d,
            )
        })
        .collect();
    p.add(Shape::line(pts, s(c, 1.2 * u)));
}

/// Folder outline (File Browser tab).
pub fn folder_thin(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.add(Shape::closed_line(
        vec![
            g(r, 1.5, 3.0),
            g(r, 6.0, 3.0),
            g(r, 7.5, 4.8),
            g(r, 14.5, 4.8),
            g(r, 14.5, 13.5),
            g(r, 1.5, 13.5),
        ],
        s(c, 1.1 * u),
    ));
    p.line_segment([g(r, 1.5, 6.5), g(r, 14.5, 6.5)], s(c, 1.0 * u));
}

/// Filled hourglass (Begin / End Document).
pub fn hourglass(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_filled(gr(r, 3.5, 1.0, 12.5, 2.4), 0.5 * u, c);
    p.rect_filled(gr(r, 3.5, 13.6, 12.5, 15.0), 0.5 * u, c);
    p.add(Shape::convex_polygon(
        vec![g(r, 4.5, 2.4), g(r, 11.5, 2.4), g(r, 8.0, 7.8)],
        c,
        Stroke::NONE,
    ));
    p.add(Shape::convex_polygon(
        vec![g(r, 8.0, 8.2), g(r, 11.5, 13.6), g(r, 4.5, 13.6)],
        c,
        Stroke::NONE,
    ));
}

/// Figures group: two stacked pictures.
pub fn pictures(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.add(Shape::line(
        vec![g(r, 2.5, 4.5), g(r, 2.5, 2.0), g(r, 13.0, 2.0)],
        s(c, 1.1 * u),
    ));
    p.rect_stroke(
        gr(r, 4.0, 4.0, 15.0, 14.0),
        1.5 * u,
        s(c, 1.2 * u),
        StrokeKind::Middle,
    );
    p.add(Shape::line(
        vec![
            g(r, 4.8, 13.0),
            g(r, 8.3, 9.2),
            g(r, 10.6, 11.4),
            g(r, 12.0, 10.0),
            g(r, 14.2, 12.4),
        ],
        s(c, 1.1 * u),
    ));
    p.circle_filled(g(r, 11.8, 6.8), 1.1 * u, c);
}

/// Labels group: a bookmark in a circle.
pub fn bookmark_circle(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.circle_stroke(g(r, 8.0, 8.0), 6.8 * u, s(c, 1.2 * u));
    p.add(Shape::convex_polygon(
        vec![
            g(r, 5.6, 4.2),
            g(r, 10.4, 4.2),
            g(r, 10.4, 11.8),
            g(r, 8.0, 9.8),
        ],
        c,
        Stroke::NONE,
    ));
    p.add(Shape::convex_polygon(
        vec![g(r, 5.6, 4.2), g(r, 8.0, 9.8), g(r, 5.6, 11.8)],
        c,
        Stroke::NONE,
    ));
}

/// Citations group: an open book.
pub fn open_book(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    let w = s(c, 1.2 * u);
    p.add(Shape::closed_line(
        vec![
            g(r, 8.0, 4.0),
            g(r, 5.0, 2.5),
            g(r, 1.0, 2.5),
            g(r, 1.0, 13.0),
            g(r, 5.0, 13.0),
            g(r, 8.0, 14.5),
        ],
        w,
    ));
    p.add(Shape::closed_line(
        vec![
            g(r, 8.0, 4.0),
            g(r, 11.0, 2.5),
            g(r, 15.0, 2.5),
            g(r, 15.0, 13.0),
            g(r, 11.0, 13.0),
            g(r, 8.0, 14.5),
        ],
        w,
    ));
}

/// Table of contents group: a page with bulleted lines.
pub fn toc_list(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 0.5, 2.0, 15.5, 14.0),
        1.5 * u,
        s(c, 1.1 * u),
        StrokeKind::Middle,
    );
    for y in [5.2, 8.0, 10.8] {
        p.rect_filled(
            Rect::from_center_size(g(r, 3.4, y), vec2(1.4 * u, 1.4 * u)),
            0.0,
            c,
        );
        p.line_segment([g(r, 5.4, y), g(r, 13.0, y)], s(c, 1.0 * u));
    }
}

/// Filter: a funnel inside a circle.
pub fn funnel_circle(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.circle_stroke(g(r, 8.0, 8.0), 6.8 * u, s(c, 1.1 * u));
    p.add(Shape::convex_polygon(
        vec![
            g(r, 4.3, 5.0),
            g(r, 11.7, 5.0),
            g(r, 9.0, 8.4),
            g(r, 7.0, 8.4),
        ],
        c,
        Stroke::NONE,
    ));
    p.rect_filled(gr(r, 7.0, 8.0, 9.0, 11.8), 0.5 * u, c);
}

/// Settings gear, filled.
pub fn gear_solid(p: &Painter, r: Rect, c: Color32, bg: Color32) {
    let u = unit(r);
    toothed_gear(
        p,
        g(r, 8.0, 8.0),
        7.6 * u,
        5.8 * u,
        2.4 * u,
        8,
        0.0,
        (11.7f32.to_radians(), 8.8f32.to_radians()),
        c,
        bg,
    );
}

/// PDF header: scroll lock (a rounded square with two-way arrows).
pub fn sync_lock(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 1.0, 1.0, 15.0, 15.0),
        2.5 * u,
        s(c, 1.1 * u),
        StrokeKind::Middle,
    );
    p.line_segment([g(r, 4.5, 6.2), g(r, 11.5, 6.2)], s(c, 1.1 * u));
    p.add(Shape::line(
        vec![g(r, 9.6, 4.4), g(r, 11.5, 6.2), g(r, 9.6, 8.0)],
        s(c, 1.1 * u),
    ));
    p.line_segment([g(r, 4.5, 10.0), g(r, 11.5, 10.0)], s(c, 1.1 * u));
    p.add(Shape::line(
        vec![g(r, 6.4, 8.2), g(r, 4.5, 10.0), g(r, 6.4, 11.8)],
        s(c, 1.1 * u),
    ));
}

/// PDF header: open in another app (an arrow out of a rounded square).
pub fn open_external(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 1.0, 1.0, 15.0, 15.0),
        2.5 * u,
        s(c, 1.1 * u),
        StrokeKind::Middle,
    );
    p.line_segment([g(r, 5.0, 11.0), g(r, 11.0, 5.0)], s(c, 1.2 * u));
    p.add(Shape::line(
        vec![g(r, 6.5, 5.0), g(r, 11.0, 5.0), g(r, 11.0, 9.5)],
        s(c, 1.2 * u),
    ));
}

/// PDF header: show in a separate window (two overlapping rounded rectangles).
pub fn windows_two(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.add(Shape::line(
        vec![
            g(r, 4.0, 4.5),
            g(r, 4.0, 2.0),
            g(r, 15.0, 2.0),
            g(r, 15.0, 10.0),
            g(r, 12.5, 10.0),
        ],
        s(c, 1.1 * u),
    ));
    p.rect_stroke(
        gr(r, 1.0, 5.0, 12.5, 14.0),
        2.0 * u,
        s(c, 1.1 * u),
        StrokeKind::Middle,
    );
}

/// Pause: two thick bars.
pub fn pause_bold(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_filled(gr(r, 3.5, 1.5, 6.8, 14.5), 1.0 * u, c);
    p.rect_filled(gr(r, 9.2, 1.5, 12.5, 14.5), 1.0 * u, c);
}

/// Thumbnails: a 3×2 grid of rounded squares.
pub fn grid_rounded(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    for (x, y) in [
        (0.5, 1.6),
        (5.8, 1.6),
        (11.1, 1.6),
        (0.5, 8.6),
        (5.8, 8.6),
        (11.1, 8.6),
    ] {
        p.rect_stroke(
            gr(r, x, y, x + 4.4, y + 5.8),
            1.0 * u,
            s(c, 1.0 * u),
            StrokeKind::Middle,
        );
    }
}

/// A document icon with "TEX" under a small mark (editor jump bar).
pub fn tex_document(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.add(Shape::closed_line(
        vec![
            g(r, 2.5, 0.5),
            g(r, 10.0, 0.5),
            g(r, 13.5, 4.0),
            g(r, 13.5, 15.5),
            g(r, 2.5, 15.5),
        ],
        s(c, 0.9 * u),
    ));
    p.text(
        g(r, 6.0, 5.0),
        egui::Align2::CENTER_CENTER,
        "t",
        egui::FontId::proportional(6.0 * u),
        c,
    );
    p.text(
        g(r, 8.0, 12.0),
        egui::Align2::CENTER_CENTER,
        "TEX",
        egui::FontId::proportional(3.6 * u),
        c,
    );
}

// ---------------------------------------------------------------------------------------------
// Settings window toolbar and form controls.

/// General settings: a gear outline around a three-spoke wheel.
pub fn prefs_general(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    let center = g(r, 8.0, 8.0);
    let teeth = 10;
    let pts: Vec<Pos2> = (0..teeth * 4)
        .map(|i| {
            let a = i as f32 / (teeth * 4) as f32 * std::f32::consts::TAU;
            let rad = if (i % 4) < 2 { 7.3 } else { 6.3 };
            center + vec2(a.cos(), a.sin()) * rad * u
        })
        .collect();
    p.add(Shape::closed_line(pts, s(c, 1.0 * u)));
    p.circle_stroke(center, 4.4 * u, s(c, 1.0 * u));
    for deg in [180.0_f32, -35.0, 90.0] {
        let a = deg.to_radians();
        p.line_segment(
            [center, center + vec2(a.cos(), a.sin()) * 4.4 * u],
            s(c, 1.0 * u),
        );
    }
}

/// Sidebar settings: a window with a left column of lines.
pub fn prefs_sidebar(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 0.8, 2.3, 15.2, 13.7),
        2.0 * u,
        s(c, 1.1 * u),
        StrokeKind::Inside,
    );
    p.line_segment([g(r, 5.6, 2.3), g(r, 5.6, 13.7)], s(c, 1.1 * u));
    for y in [5.0, 7.0, 9.0] {
        p.line_segment([g(r, 2.4, y), g(r, 4.0, y)], s(c, 0.9 * u));
    }
}

/// Editor settings: a pencil over a baseline.
pub fn prefs_editor(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    let w = s(c, 1.1 * u);
    // Pencil body from the tip (bottom left) to the end (top right).
    p.add(Shape::closed_line(
        vec![
            g(r, 3.0, 12.6),
            g(r, 3.6, 10.0),
            g(r, 11.6, 2.0),
            g(r, 13.6, 4.0),
            g(r, 5.6, 12.0),
        ],
        w,
    ));
    p.line_segment([g(r, 10.2, 3.4), g(r, 12.2, 5.4)], w);
    p.line_segment([g(r, 3.6, 10.0), g(r, 5.6, 12.0)], w);
    p.line_segment([g(r, 1.5, 15.0), g(r, 14.5, 15.0)], s(c, 1.2 * u));
}

/// Document Viewer settings: a page with a heading, lines and a picture.
pub fn prefs_viewer(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.rect_stroke(
        gr(r, 3.0, 0.8, 13.0, 15.2),
        1.2 * u,
        s(c, 1.0 * u),
        StrokeKind::Inside,
    );
    // "A" and two lines.
    p.add(Shape::line(
        vec![g(r, 4.9, 6.4), g(r, 6.2, 2.8), g(r, 7.5, 6.4)],
        s(c, 0.9 * u),
    ));
    p.line_segment([g(r, 5.4, 5.1), g(r, 7.0, 5.1)], s(c, 0.8 * u));
    p.line_segment([g(r, 8.6, 3.5), g(r, 11.3, 3.5)], s(c, 0.9 * u));
    p.line_segment([g(r, 8.6, 5.6), g(r, 11.3, 5.6)], s(c, 0.9 * u));
    p.rect_filled(gr(r, 4.6, 8.4, 11.4, 13.2), 0.6 * u, c);
}

/// Pop-up button indicator: a small up and down chevron.
pub fn updown(p: &Painter, center: Pos2, c: Color32) {
    let w = s(c, 1.3);
    p.add(Shape::line(
        vec![
            center + vec2(-3.0, -1.6),
            center + vec2(0.0, -4.6),
            center + vec2(3.0, -1.6),
        ],
        w,
    ));
    p.add(Shape::line(
        vec![
            center + vec2(-3.0, 1.6),
            center + vec2(0.0, 4.6),
            center + vec2(3.0, 1.6),
        ],
        w,
    ));
}

/// More information: a circled right chevron.
pub fn info_chevron(p: &Painter, center: Pos2, radius: f32, c: Color32) {
    p.circle_stroke(center, radius, s(c, 1.0));
    let k = radius * 0.32;
    p.add(Shape::line(
        vec![
            center + vec2(-k * 0.6, -k * 1.4),
            center + vec2(k * 0.8, 0.0),
            center + vec2(-k * 0.6, k * 1.4),
        ],
        s(c, 1.0),
    ));
}

/// A check mark for a filled checkbox of side `side` centred at `center`.
pub fn check_mark(p: &Painter, center: Pos2, side: f32, c: Color32) {
    let k = side / 14.0;
    p.add(Shape::line(
        vec![
            center + vec2(-3.6, 0.2) * k,
            center + vec2(-1.0, 2.9) * k,
            center + vec2(3.8, -3.2) * k,
        ],
        s(c, 1.7 * k),
    ));
}

// ---------------------------------------------------------------------------------------------
// Toolbar icons at the target's measured sizes, in points about a centre (not on the 16 grid).

/// Points of a quarter arc around `c` from angle `a0` to `a1` (radians).
fn arc(c: Pos2, r: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    (0..=6)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / 6.0;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

/// Outline of a rounded rectangle as an open path from `start_x` on the top edge, clockwise,
/// back to `end_x` on the top edge (leaves a gap between them when `end_x < start_x`).
fn rounded_outline(rect: Rect, r: f32, start_x: f32, end_x: f32) -> Vec<Pos2> {
    use std::f32::consts::{FRAC_PI_2, PI};
    let mut pts = vec![pos2(start_x, rect.top())];
    pts.extend(arc(
        pos2(rect.right() - r, rect.top() + r),
        r,
        -FRAC_PI_2,
        0.0,
    ));
    pts.extend(arc(
        pos2(rect.right() - r, rect.bottom() - r),
        r,
        0.0,
        FRAC_PI_2,
    ));
    pts.extend(arc(
        pos2(rect.left() + r, rect.bottom() - r),
        r,
        FRAC_PI_2,
        PI,
    ));
    pts.extend(arc(
        pos2(rect.left() + r, rect.top() + r),
        r,
        PI,
        PI + FRAC_PI_2,
    ));
    pts.push(pos2(end_x, rect.top()));
    pts
}

/// Share (export): a square open at the top with an arrow out of it, in front of a second square
/// offset down and right. 16 × 19.5 pt.
pub fn share_stacked(p: &Painter, c: Pos2, col: Color32, bg: Color32) {
    let w = Stroke::new(1.4, col);
    let front = Rect::from_min_max(c + vec2(-7.6, -4.5), c + vec2(4.9, 6.0));
    let back = front.translate(vec2(3.8, 4.0));
    p.rect_stroke(back, 1.8, w, StrokeKind::Middle);
    // The front square hides the back one.
    p.rect_filled(front.expand(0.7), 2.6, bg);
    let ax = front.center().x;
    p.add(Shape::line(
        rounded_outline(front, 1.8, ax + 3.2, ax - 3.2),
        w,
    ));
    p.line_segment([pos2(ax, c.y + 2.0), pos2(ax, c.y - 9.0)], w);
    p.add(Shape::line(
        vec![
            pos2(ax - 3.0, c.y - 6.1),
            pos2(ax, c.y - 9.1),
            pos2(ax + 3.0, c.y - 6.1),
        ],
        w,
    ));
}

/// View mode "editor": an outlined 19.5 × 16 pt rectangle.
pub fn view_editor_only(p: &Painter, c: Pos2, col: Color32) {
    p.rect_stroke(
        Rect::from_center_size(c, vec2(19.5, 16.0)),
        2.2,
        Stroke::new(1.4, col),
        StrokeKind::Inside,
    );
}

/// View mode "editor and PDF": an outlined square, a bar and a filled square (38 pt wide).
pub fn view_split_pair(p: &Painter, c: Pos2, col: Color32) {
    p.rect_stroke(
        Rect::from_min_max(c + vec2(-19.0, -8.0), c + vec2(-3.5, 8.0)),
        2.2,
        Stroke::new(1.4, col),
        StrokeKind::Inside,
    );
    p.rect_filled(
        Rect::from_min_max(c + vec2(-0.5, -7.0), c + vec2(0.5, 7.0)),
        0.0,
        col,
    );
    p.rect_filled(
        Rect::from_min_max(c + vec2(3.5, -8.0), c + vec2(19.0, 8.0)),
        2.2,
        col,
    );
}

/// View mode "PDF": a filled 19.5 × 16 pt rectangle.
pub fn view_pdf_only(p: &Painter, c: Pos2, col: Color32) {
    p.rect_filled(Rect::from_center_size(c, vec2(19.5, 16.0)), 2.2, col);
}

/// Back / forward: a thin chevron 7 × 13.5 pt.
pub fn nav_chevron(p: &Painter, c: Pos2, col: Color32, right: bool) {
    let d = if right { -1.0 } else { 1.0 };
    p.add(Shape::line(
        vec![
            c + vec2(3.0 * d, -6.4),
            c + vec2(-3.0 * d, 0.0),
            c + vec2(3.0 * d, 6.4),
        ],
        Stroke::new(1.6, col),
    ));
}

/// New tab: an outlined rounded rectangle (17.4 × 13.7 pt) with a filled "+" badge on its top
/// right corner, cut out of the outline. `c` is the rectangle's centre.
pub fn new_tab_rounded(p: &Painter, c: Pos2, col: Color32, bg: Color32) {
    let rect = Rect::from_center_size(c, vec2(17.4, 13.7));
    p.rect_stroke(rect, 2.2, Stroke::new(1.4, col), StrokeKind::Inside);
    let b = pos2(rect.right(), rect.top() + 2.0);
    p.circle_filled(b, 4.9, bg);
    p.circle_filled(b, 3.6, col);
    let w = Stroke::new(1.1, bg);
    p.line_segment([b - vec2(1.9, 0.0), b + vec2(1.9, 0.0)], w);
    p.line_segment([b - vec2(0.0, 1.9), b + vec2(0.0, 1.9)], w);
}

// ---------------------------------------------------------------------------------------------
// Source Control and AI Assistant (original drawings on the 16-unit grid).

/// A Git branch: two commits on a line and a branch curving off to a third (Source Control).
pub fn git_branch(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    let w = s(c, 1.25 * u);
    p.line_segment([g(r, 4.5, 4.6), g(r, 4.5, 11.4)], w);
    p.add(Shape::line(
        vec![
            g(r, 11.5, 5.6),
            g(r, 11.5, 7.0),
            g(r, 10.8, 8.4),
            g(r, 9.0, 9.2),
            g(r, 6.4, 9.8),
            g(r, 4.9, 10.9),
        ],
        w,
    ));
    for (x, y) in [(4.5, 3.0), (4.5, 13.0), (11.5, 4.0)] {
        p.circle_stroke(g(r, x, y), 1.6 * u, w);
    }
}

/// An arrow down onto a line (Pull) or up from it (Push).
pub fn arrow_line(p: &Painter, r: Rect, c: Color32, up: bool) {
    let u = unit(r);
    let w = s(c, 1.3 * u);
    let (tip, tail) = if up { (3.0, 11.0) } else { (11.0, 3.0) };
    p.line_segment([g(r, 8.0, tail), g(r, 8.0, tip)], w);
    let d = if up { 3.2 } else { -3.2 };
    p.add(Shape::line(
        vec![g(r, 4.8, tip + d), g(r, 8.0, tip), g(r, 11.2, tip + d)],
        w,
    ));
    p.line_segment([g(r, 3.5, 14.0), g(r, 12.5, 14.0)], w);
}

/// A circular arrow (Refresh).
pub fn refresh(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    let w = s(c, 1.3 * u);
    let center = g(r, 8.0, 8.5);
    let rad = 5.0 * u;
    let pts: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = -1.2 + 5.4 * i as f32 / 24.0;
            center + vec2(a.cos(), a.sin()) * rad
        })
        .collect();
    let end = pts.last().copied().unwrap_or(center);
    p.add(Shape::line(pts, w));
    let a = 4.2f32;
    let tangent = vec2(-a.sin(), a.cos());
    let normal = vec2(a.cos(), a.sin());
    p.add(Shape::line(
        vec![
            end - tangent * 0.2 * u + normal * 2.4 * u - tangent * 2.0 * u,
            end,
            end - normal * 2.4 * u - tangent * 2.0 * u,
        ],
        w,
    ));
}

/// A minus sign (Unstage).
pub fn minus(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    p.line_segment([g(r, 3.0, 8.0), g(r, 13.0, 8.0)], s(c, 1.4 * u));
}

/// An arrow turning back (Discard Changes).
pub fn discard(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    let w = s(c, 1.3 * u);
    p.add(Shape::line(
        vec![
            g(r, 4.0, 6.0),
            g(r, 10.0, 6.0),
            g(r, 12.2, 6.8),
            g(r, 13.0, 9.0),
            g(r, 12.2, 11.2),
            g(r, 10.0, 12.0),
            g(r, 6.0, 12.0),
        ],
        w,
    ));
    p.add(Shape::line(
        vec![g(r, 6.6, 3.4), g(r, 4.0, 6.0), g(r, 6.6, 8.6)],
        w,
    ));
}

/// The outline of a four-pointed sparkle: concave curved sides (quadratic curves pulled
/// toward the centre, `fullness` × radius off it) and slightly rounded tips.
fn sparkle_outline(center: Pos2, radius: f32, fullness: f32) -> Vec<Pos2> {
    let tip = |i: usize| {
        let a = i as f32 * std::f32::consts::FRAC_PI_2 - std::f32::consts::FRAC_PI_2;
        center + vec2(a.cos(), a.sin()) * radius
    };
    let side = |i: usize, t: f32| {
        let (a, b) = (tip(i), tip(i + 1));
        let mid = i as f32 * std::f32::consts::FRAC_PI_2 - std::f32::consts::FRAC_PI_4;
        let c = center + vec2(mid.cos(), mid.sin()) * fullness * radius;
        let u = 1.0 - t;
        pos2(
            u * u * a.x + 2.0 * u * t * c.x + t * t * b.x,
            u * u * a.y + 2.0 * u * t * c.y + t * t * b.y,
        )
    };
    let (t0, t1) = (0.07, 0.93);
    let mut pts = Vec::with_capacity(64);
    for i in 0..4 {
        for k in 0..=10 {
            pts.push(side(i, t0 + (t1 - t0) * k as f32 / 10.0));
        }
        // Round the tip between this side and the next with a small curve through it.
        let (a, b, c) = (side(i, t1), tip(i + 1), side((i + 1) % 4, t0));
        for s in [0.25f32, 0.5, 0.75] {
            let u = 1.0 - s;
            pts.push(pos2(
                u * u * a.x + 2.0 * u * s * b.x + s * s * c.x,
                u * u * a.y + 2.0 * u * s * b.y + s * s * c.y,
            ));
        }
    }
    pts
}

/// Fills a star-shaped outline around `center` (a fan, then a fine edge for anti-aliasing).
fn fill_star(p: &Painter, center: Pos2, pts: &[Pos2], c: Color32) {
    let mut mesh = egui::epaint::Mesh::default();
    mesh.colored_vertex(center, c);
    for q in pts {
        mesh.colored_vertex(*q, c);
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(Shape::mesh(mesh));
    p.add(Shape::closed_line(pts.to_vec(), s(c, 0.6)));
}

/// Sparkles (AI Assistant): a large, full four-pointed sparkle with rounded tips and a small one
/// above right. `filled` fills them; otherwise they are drawn as outlines.
pub fn sparkles(p: &Painter, r: Rect, c: Color32, filled: bool) {
    let u = unit(r);
    for (cx, cy, rad) in [(6.7, 9.3, 6.5), (12.9, 3.5, 3.0)] {
        let center = g(r, cx, cy);
        let pts = sparkle_outline(center, rad * u, 0.17);
        if filled {
            fill_star(p, center, &pts, c);
        } else {
            p.add(Shape::closed_line(pts, s(c, 1.3 * u)));
        }
    }
}

/// An arrow up in a filled circle (Send).
pub fn send(p: &Painter, r: Rect, c: Color32, fg: Color32) {
    let u = unit(r);
    p.circle_filled(g(r, 8.0, 8.0), 7.5 * u, c);
    let w = s(fg, 1.6 * u);
    p.line_segment([g(r, 8.0, 12.0), g(r, 8.0, 4.5)], w);
    p.add(Shape::line(
        vec![g(r, 4.9, 7.4), g(r, 8.0, 4.3), g(r, 11.1, 7.4)],
        w,
    ));
}

/// A square in a filled circle (Stop).
pub fn stop(p: &Painter, r: Rect, c: Color32, fg: Color32) {
    let u = unit(r);
    p.circle_filled(g(r, 8.0, 8.0), 7.5 * u, c);
    p.rect_filled(gr(r, 5.4, 5.4, 10.6, 10.6), 1.2 * u, fg);
}

/// Two overlapping pages (Copy).
pub fn copy(p: &Painter, r: Rect, c: Color32) {
    let u = unit(r);
    let w = s(c, 1.2 * u);
    p.rect_stroke(gr(r, 5.5, 5.5, 13.5, 14.0), 1.8 * u, w, StrokeKind::Middle);
    p.add(Shape::line(
        vec![
            g(r, 10.5, 3.6),
            g(r, 10.5, 3.0),
            g(r, 9.6, 2.0),
            g(r, 3.4, 2.0),
            g(r, 2.5, 3.0),
            g(r, 2.5, 10.0),
            g(r, 3.4, 11.0),
            g(r, 4.0, 11.0),
        ],
        w,
    ));
}
