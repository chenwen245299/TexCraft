//! Small shared widgets in the macOS idiom: icon buttons, segmented controls, pills, filter
//! fields, list rows.

use egui::{
    Color32, CornerRadius, Painter, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, vec2,
};

use crate::theme::Tokens;

/// Feeds `pending` edit events (menu bar ⌘V and co., see `Workbench::route_edit_events`) to the
/// current window. Call it first in the window's frame, before its widgets read input.
pub(crate) fn feed_edit_events(ctx: &egui::Context, pending: &mut Vec<egui::Event>) {
    if pending.is_empty() {
        return;
    }
    let events = std::mem::take(pending);
    ctx.input_mut(|i| i.events.extend(events));
}

/// An icon-only button. `draw` paints the icon into the given rect with the given colour.
pub fn icon_button(
    ui: &mut Ui,
    t: &Tokens,
    size: Vec2,
    tooltip: &str,
    selected: bool,
    enabled: bool,
    draw: impl FnOnce(&Painter, Rect, Color32),
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if selected {
            p.rect_filled(
                rect,
                5.0,
                if t.dark {
                    Color32::from_white_alpha(28)
                } else {
                    Color32::from_black_alpha(22)
                },
            );
        } else if resp.hovered() && enabled {
            p.rect_filled(rect, 5.0, t.row_hover);
        }
        let color = if !enabled {
            t.toolbar_fg.gamma_multiply(0.3)
        } else {
            t.toolbar_fg
        };
        let icon = Rect::from_center_size(rect.center(), vec2(size.y * 0.62, size.y * 0.62));
        draw(p, icon, color);
    }
    if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    }
}

/// A segmented control of text labels in the target's sidebar style: the selected label sits in
/// a blue pill, the others are plain text. Returns the clicked index.
pub fn segmented_text(
    ui: &mut Ui,
    t: &Tokens,
    labels: &[&str],
    selected: usize,
    width: f32,
) -> Option<usize> {
    let h = 18.5;
    let font = egui::FontId::proportional(11.75);
    let (rect, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
    let p = ui.painter().clone();
    let widths: Vec<f32> = labels
        .iter()
        .map(|l| {
            p.layout_no_wrap((*l).to_string(), font.clone(), t.text)
                .size()
                .x
                + 18.0
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.5 * (labels.len().saturating_sub(1)) as f32;
    let mut x = rect.center().x - total / 2.0;
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let w = widths.get(i).copied().unwrap_or(40.0);
        let r = Rect::from_min_size(egui::pos2(x, rect.top()), vec2(w, h));
        let resp = ui.interact(r, ui.id().with(("seg", label, i)), Sense::click());
        if i == selected {
            p.rect_filled(r, 5.0, t.accent);
        } else if resp.hovered() {
            p.rect_filled(r, 5.0, t.row_hover);
        }
        let color = if i == selected {
            Color32::WHITE
        } else {
            t.sidebar_text
        };
        p.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font.clone(),
            color,
        );
        if resp.clicked() {
            clicked = Some(i);
        }
        x += w + 6.5;
    }
    clicked
}

/// A segmented control of icons (the toolbar's view-mode switch). Returns the clicked index.
pub fn segmented_icons(
    ui: &mut Ui,
    t: &Tokens,
    count: usize,
    selected: usize,
    seg: Vec2,
    tooltips: &[&str],
    draw: impl Fn(usize, &Painter, Rect, Color32),
) -> Option<usize> {
    let rect_size = vec2(seg.x * count as f32, seg.y);
    let (rect, _) = ui.allocate_exact_size(rect_size, Sense::hover());
    let p = ui.painter().clone();
    p.rect(
        rect,
        6.0,
        if t.dark {
            Color32::from_white_alpha(14)
        } else {
            Color32::from_black_alpha(10)
        },
        Stroke::NONE,
        StrokeKind::Inside,
    );
    let mut clicked = None;
    for i in 0..count {
        let r = Rect::from_min_size(rect.min + vec2(seg.x * i as f32, 0.0), seg);
        let mut resp = ui.interact(r, ui.id().with(("segicon", i)), Sense::click());
        if let Some(tip) = tooltips.get(i) {
            resp = resp.on_hover_text(*tip);
        }
        if i == selected {
            p.rect_filled(
                r.shrink(2.0),
                5.0,
                if t.dark {
                    Color32::from_white_alpha(40)
                } else {
                    Color32::WHITE
                },
            );
        } else if resp.hovered() {
            p.rect_filled(r.shrink(2.0), 5.0, t.row_hover);
        }
        let icon = Rect::from_center_size(r.center(), vec2(seg.y * 0.62, seg.y * 0.62));
        draw(
            i,
            &p,
            icon,
            if i == selected {
                t.text
            } else {
                t.toolbar_icon
            },
        );
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// A thin vertical separator for toolbars.
pub fn vsep(ui: &mut Ui, t: &Tokens, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, height), Sense::hover());
    ui.painter().line_segment(
        [
            rect.center_top() + vec2(0.0, 4.0),
            rect.center_bottom() - vec2(0.0, 4.0),
        ],
        Stroke::new(1.0, t.divider.gamma_multiply(0.8)),
    );
}

/// A small outlined pill with text (e.g. `ROOT`).
pub fn pill(p: &Painter, right_center: egui::Pos2, text: &str, color: Color32) -> Rect {
    let font = egui::FontId::proportional(8.5);
    let galley = p.layout_no_wrap(text.to_string(), font, color);
    let size = galley.size() + vec2(8.0, 2.0);
    let rect = Rect::from_min_size(right_center - vec2(size.x, size.y / 2.0), size);
    p.rect_stroke(
        rect,
        CornerRadius::same(3),
        Stroke::new(0.8, color),
        StrokeKind::Inside,
    );
    p.galley(rect.min + vec2(4.0, 1.0), galley, color);
    rect
}

/// A search/filter field with an icon. Returns the response of the text edit.
pub fn filter_field(
    ui: &mut Ui,
    t: &Tokens,
    text: &mut String,
    hint: &str,
    id: egui::Id,
) -> Response {
    let height = 22.0;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let p = ui.painter().clone();
    p.rect(
        rect,
        6.0,
        t.control_bg,
        Stroke::new(1.0, t.control_border),
        StrokeKind::Inside,
    );
    let icon = Rect::from_center_size(rect.left_center() + vec2(12.0, 0.0), vec2(12.0, 12.0));
    crate::icons::filter(&p, icon, t.text_weak);
    let edit_rect = Rect::from_min_max(
        rect.min + vec2(22.0, 2.0),
        rect.max - vec2(if text.is_empty() { 6.0 } else { 22.0 }, 2.0),
    );
    let resp = ui.put(
        edit_rect,
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text(hint)
            .frame(egui::Frame::NONE)
            .font(egui::FontId::proportional(12.0))
            .vertical_align(egui::Align::Center),
    );
    if !text.is_empty() {
        let x = Rect::from_center_size(rect.right_center() - vec2(11.0, 0.0), vec2(14.0, 14.0));
        let xr = ui.interact(x, id.with("clear"), Sense::click());
        p.circle_filled(x.center(), 6.5, t.text_weak.gamma_multiply(0.6));
        crate::icons::close(&p, x.shrink(3.0), t.control_bg);
        if xr.clicked() {
            text.clear();
        }
    }
    resp
}

/// A full-width selectable list row. Returns the row response; `draw` paints the content.
pub fn list_row(
    ui: &mut Ui,
    t: &Tokens,
    height: f32,
    selected: bool,
    id: egui::Id,
    draw: impl FnOnce(&Painter, Rect),
) -> Response {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter_at(rect);
        if selected {
            p.rect_filled(rect.shrink2(vec2(4.0, 0.0)), 4.0, t.row_selected);
        } else if resp.hovered() {
            p.rect_filled(rect.shrink2(vec2(4.0, 0.0)), 4.0, t.row_hover);
        }
        draw(&p, rect);
    }
    resp
}

/// Truncates `text` with an ellipsis to fit `max_width` in `font`.
pub fn elide(p: &Painter, text: &str, font: &egui::FontId, max_width: f32) -> String {
    let width = |s: &str| {
        p.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE)
            .size()
            .x
    };
    if width(text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars.iter().take(mid).collect::<String>() + "…";
        if width(&s) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars.iter().take(lo).collect::<String>() + "…"
}

/// One small capsule push button of [`capsule_buttons`].
#[derive(Debug, Clone, Copy)]
pub struct Capsule<'a> {
    /// The label.
    pub label: &'a str,
    /// The default action: accent fill, white text.
    pub default: bool,
    /// Clickable.
    pub enabled: bool,
}

/// A row of small capsule push buttons (22 pt, 12 pt text), right-aligned `margin` from the
/// edge, as in the sidebar's search panel. Returns the index of the clicked button.
pub fn capsule_buttons(
    ui: &mut Ui,
    t: &Tokens,
    buttons: &[Capsule<'_>],
    margin: f32,
) -> Option<usize> {
    let font = egui::FontId::proportional(12.0);
    let h = 22.0;
    let gap = 6.0;
    let widths: Vec<f32> = buttons
        .iter()
        .map(|b| {
            ui.painter()
                .layout_no_wrap(b.label.to_string(), font.clone(), t.text)
                .size()
                .x
                .max(40.0)
                + 24.0
        })
        .collect();
    let total = widths.iter().sum::<f32>() + gap * buttons.len().saturating_sub(1) as f32;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let p = ui.painter().clone();
    let mut x = row.right() - margin - total;
    let mut clicked = None;
    for (i, (b, w)) in buttons.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(egui::pos2(x, row.top()), vec2(*w, h));
        x += w + gap;
        let sense = if b.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let resp = ui.interact(r, ui.id().with(("capsule", b.label)), sense);
        let pressed = b.enabled && resp.is_pointer_button_down_on();
        let (fill, fg) = if b.default {
            let fill = if !b.enabled {
                t.accent.gamma_multiply(0.45)
            } else if pressed {
                t.accent.gamma_multiply(0.85)
            } else {
                t.accent
            };
            (fill, Color32::WHITE)
        } else {
            let fill = if pressed {
                t.control_hover
            } else if b.enabled && resp.hovered() {
                t.form_popup
            } else {
                t.control_bg
            };
            (
                fill,
                if b.enabled {
                    t.control_text
                } else {
                    t.menu_disabled
                },
            )
        };
        p.rect_filled(r, h / 2.0, fill);
        if !b.default {
            p.rect_stroke(
                r,
                h / 2.0,
                Stroke::new(0.5, t.control_border),
                StrokeKind::Inside,
            );
        }
        p.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            b.label,
            font.clone(),
            fg,
        );
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key pasted from the menu bar lands in the focused (password) field of the window it was
    /// fed to.
    #[test]
    fn fed_edit_events_reach_the_focused_field() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("api-key");
        let mut text = String::new();
        let frame = |pending: &mut Vec<egui::Event>, text: &mut String| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                feed_edit_events(ui.ctx(), pending);
                let field = egui::TextEdit::singleline(text).id(id).password(true);
                ui.add(field).request_focus();
            });
            out.textures_delta.clear();
        };
        frame(&mut Vec::new(), &mut text);
        let mut pending = vec![egui::Event::Paste("sk-test-not-a-key".into())];
        frame(&mut pending, &mut text);
        assert!(pending.is_empty());
        assert_eq!(text, "sk-test-not-a-key");
    }
}
