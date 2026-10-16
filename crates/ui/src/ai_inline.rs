//! Inline AI suggestions in the editor: the grey suggestion text spliced into the layout, the
//! button in the editor header and its settings popover.

use egui::text::{ByteIndex, LayoutJob, LayoutSection, TextFormat};
use egui::{Align2, Color32, FontId, Id, Key, Rect, Sense, Stroke, Ui, pos2, vec2};
use texcraft_engine::Session;

use crate::icons;
use crate::prefs::{Pal, check_control, pal, popup_control};
use crate::theme::Tokens;

/// Inserts the suggestion `ghost` at byte `at` of `job` in `color`, moving the sections after it
/// (a section across `at` is split), so the text after the caret makes room for it the way an
/// IDE shows a suggestion.
pub fn splice_ghost(job: &mut LayoutJob, at: usize, ghost: &str, font: &FontId, color: Color32) {
    if ghost.is_empty() || at > job.text.len() || !job.text.is_char_boundary(at) {
        return;
    }
    let n = ghost.len();
    job.text.insert_str(at, ghost);
    let section = LayoutSection {
        leading_space: 0.0,
        byte_range: ByteIndex(at)..ByteIndex(at + n),
        format: TextFormat {
            font_id: font.clone(),
            color,
            ..Default::default()
        },
    };
    let mut out = Vec::with_capacity(job.sections.len() + 2);
    let mut inserted = false;
    for sec in std::mem::take(&mut job.sections) {
        let r = sec.byte_range.start.0..sec.byte_range.end.0;
        if r.end <= at {
            out.push(sec);
        } else if r.start >= at {
            if !inserted {
                out.push(section.clone());
                inserted = true;
            }
            out.push(LayoutSection {
                byte_range: ByteIndex(r.start + n)..ByteIndex(r.end + n),
                ..sec
            });
        } else {
            out.push(LayoutSection {
                byte_range: ByteIndex(r.start)..ByteIndex(at),
                ..sec.clone()
            });
            out.push(section.clone());
            inserted = true;
            out.push(LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(at + n)..ByteIndex(r.end + n),
                ..sec
            });
        }
    }
    if !inserted {
        out.push(section);
    }
    job.sections = out;
}

/// The suggestion button in the editor header (`r`): sparkles, filled while suggestions are on,
/// with a dot while one is on its way. Toggles the settings popover.
pub fn header_button(ui: &mut Ui, t: &Tokens, session: &Session, open: &mut bool, r: Rect) {
    let on = session.ai.config.inline.enabled;
    let available = session.inline_available();
    let tip = if !available {
        "AI Suggestions: set up a provider in Settings → AI"
    } else if on {
        "AI Suggestions: on (Tab accepts, Esc dismisses)"
    } else {
        "AI Suggestions: off (⌥\\ asks once)"
    };
    let resp = ui
        .interact(r, Id::new("ai-inline-button"), Sense::click())
        .on_hover_text(tip);
    if *open || resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.row_hover);
    }
    let ink = if *open {
        t.accent
    } else if on && available {
        t.pdf_icon
    } else {
        t.pdf_icon.gamma_multiply(0.55)
    };
    let icon = Rect::from_center_size(r.center(), vec2(15.0, 15.0));
    icons::sparkles(ui.painter(), icon, ink, on && available);
    if session.inline.is_busy() {
        let phase = (ui.input(|i| i.time) * 4.0).sin() as f32 * 0.5 + 0.5;
        ui.painter().circle_filled(
            pos2(r.right() - 5.0, r.top() + 5.0),
            2.2,
            t.accent.gamma_multiply(0.45 + 0.55 * phase),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
    if resp.clicked() {
        *open = !*open;
    }
}

const POP_W: f32 = 320.0;

/// The settings popover under the header button `button`.
pub fn popover(ui: &mut Ui, t: &Tokens, session: &mut Session, open: &mut bool, button: Rect) {
    let pal = pal(t);
    // A choice menu open inside the popover counts as inside.
    let menu_open = egui::Popup::is_any_open(ui.ctx());
    let anchor = pos2(button.right() - POP_W + 8.0, button.bottom() + 6.0);
    let mut changed = false;
    let area = egui::Area::new(Id::new("ai-inline-popover"))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor)
        .show(ui.ctx(), |ui| {
            crate::menu::sheet_frame(t, 14).show(ui, |ui| {
                ui.set_width(POP_W - 28.0);
                changed = body(ui, t, &pal, session);
            });
        });
    if changed {
        let _ = session.save_ai_config();
    }
    let pointer = ui.input(|i| i.pointer.interact_pos().unwrap_or_default());
    let clicked_outside = ui.input(|i| i.pointer.any_click())
        && !menu_open
        && !area.response.rect.contains(pointer)
        && !button.contains(pointer);
    if clicked_outside || (ui.input(|i| i.key_pressed(Key::Escape)) && !menu_open) {
        *open = false;
    }
}

/// A row: label at the left, value text at the right.
fn row_label(ui: &Ui, pal: &Pal, rect: Rect, label: &str, value: &str) {
    let p = ui.painter();
    p.text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.5),
        pal.label,
    );
    p.text(
        pos2(rect.right(), rect.center().y),
        Align2::RIGHT_CENTER,
        value,
        FontId::proportional(12.0),
        pal.desc,
    );
}

/// The popover's contents; returns whether a setting changed.
fn body(ui: &mut Ui, t: &Tokens, pal: &Pal, session: &mut Session) -> bool {
    let mut changed = false;
    let w = ui.available_width();
    let p = ui.painter().clone();
    // Title.
    let (r, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    icons::sparkles(
        &p,
        Rect::from_min_size(pos2(r.left(), r.center().y - 7.5), vec2(15.0, 15.0)),
        t.accent,
        true,
    );
    p.text(
        pos2(r.left() + 22.0, r.center().y),
        Align2::LEFT_CENTER,
        "AI Suggestions",
        crate::theme::semibold(13.5),
        pal.label,
    );
    ui.add_space(8.0);
    // On / off.
    let (r, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    let mut on = session.ai.config.inline.enabled;
    if check_control(
        ui,
        pal,
        Id::new("ai-inline-on"),
        pos2(r.left(), r.center().y),
        14.0,
        "Suggest while typing",
        12.5,
        &mut on,
        true,
    ) {
        session.ai.config.inline.enabled = on;
        if !on {
            session.inline_dismiss();
        }
        changed = true;
    }
    ui.add_space(10.0);

    // Model.
    let (r, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
    row_label(
        ui,
        pal,
        Rect::from_min_size(r.min, vec2(70.0, r.height())),
        "Model",
        "",
    );
    let assistant = session
        .ai
        .config
        .selection()
        .map(|(_, m)| format!("Same as the Assistant ({m})"))
        .unwrap_or_else(|| "Same as the Assistant".into());
    let mut options: Vec<((String, String), String)> =
        vec![((String::new(), String::new()), assistant)];
    for prov in session.ai.config.providers.iter().filter(|p| p.enabled) {
        for m in &prov.models {
            options.push(((prov.id.clone(), m.clone()), format!("{} · {m}", prov.name)));
        }
    }
    let current_known = options.iter().any(|((p, m), _)| {
        *p == session.ai.config.inline.provider && *m == session.ai.config.inline.model
    });
    let mut value = if current_known {
        (
            session.ai.config.inline.provider.clone(),
            session.ai.config.inline.model.clone(),
        )
    } else {
        (String::new(), String::new())
    };
    let field = Rect::from_min_max(pos2(r.left() + 70.0, r.top()), r.max);
    if popup_control(
        ui,
        pal,
        Id::new("ai-inline-model"),
        field,
        &mut value,
        &options,
        12.0,
    ) {
        session.ai.config.inline.provider = value.0;
        session.ai.config.inline.model = value.1;
        changed = true;
    }
    ui.add_space(12.0);

    // Numbers.
    let s = &mut session.ai.config.inline;
    let mut delay = s.delay_ms as f32;
    if slider_row(
        ui,
        t,
        pal,
        "ai-inline-delay",
        "Suggest after a pause of",
        &format!("{:.1} s", delay / 1000.0),
        &mut delay,
        100.0..=2000.0,
        50.0,
    ) {
        s.delay_ms = delay as u32;
        changed = true;
    }
    let mut lines = s.max_lines as f32;
    if slider_row(
        ui,
        t,
        pal,
        "ai-inline-lines",
        "Longest suggestion",
        &format!(
            "{} line{}",
            lines as u32,
            if lines as u32 == 1 { "" } else { "s" }
        ),
        &mut lines,
        1.0..=20.0,
        1.0,
    ) {
        s.max_lines = lines as u32;
        changed = true;
    }
    let mut before = s.context_before as f32;
    if slider_row(
        ui,
        t,
        pal,
        "ai-inline-before",
        "Context before the cursor",
        &chars(before),
        &mut before,
        500.0..=16000.0,
        500.0,
    ) {
        s.context_before = before as u32;
        changed = true;
    }
    let mut after = s.context_after as f32;
    if slider_row(
        ui,
        t,
        pal,
        "ai-inline-after",
        "Context after the cursor",
        &chars(after),
        &mut after,
        0.0..=8000.0,
        250.0,
    ) {
        s.context_after = after as u32;
        changed = true;
    }

    // Status and keys.
    ui.add_space(4.0);
    crate::prefs::hairline(
        ui,
        ui.min_rect().left(),
        ui.min_rect().left() + w,
        ui.cursor().top(),
        pal.hairline,
    );
    ui.add_space(8.0);
    let status = if !session.inline_available() {
        Some((
            "Set up a provider and its API key in Settings → AI.".to_string(),
            t.error,
        ))
    } else if let Some(e) = &session.inline.last_error {
        Some((e.clone(), t.error))
    } else {
        session.inline.last_latency.map(|d| {
            (
                format!("Last suggestion in {:.1} s.", d.as_secs_f32()),
                pal.desc,
            )
        })
    };
    if let Some((text, color)) = status {
        ui.label(egui::RichText::new(text).size(11.5).color(color));
        ui.add_space(4.0);
    }
    if !session.inline_available() {
        let (r, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
        let b = Rect::from_min_size(r.min, vec2(110.0, 22.0));
        if crate::prefs::push_button(ui, pal, Id::new("ai-inline-setup"), b, "AI Settings…", 12.0)
        {
            let _ = session.run("ai.settings", &serde_json::Value::Null);
        }
        ui.add_space(4.0);
    }
    ui.label(
        egui::RichText::new("Tab accepts · ⌘→ next word · Esc dismisses · ⌥\\ suggests now")
            .size(11.0)
            .color(pal.note),
    );
    ui.label(
        egui::RichText::new(
            "The text around the cursor goes to the chosen model after each pause in typing.",
        )
        .size(11.0)
        .color(pal.note),
    );
    changed
}

fn chars(n: f32) -> String {
    let n = n as u32;
    if n == 0 {
        "none".into()
    } else if n >= 1000 {
        format!("{:.1}k characters", n as f32 / 1000.0)
    } else {
        format!("{n} characters")
    }
}

/// A labelled slider row (label and value above, the slider below). Returns whether it changed.
#[allow(clippy::too_many_arguments)]
fn slider_row(
    ui: &mut Ui,
    t: &Tokens,
    pal: &Pal,
    id: &str,
    label: &str,
    value_text: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    step: f32,
) -> bool {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
    row_label(ui, pal, r, label, value_text);
    let (track_row, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    let changed = slider(ui, t, Id::new(id), track_row, value, range, step);
    ui.add_space(8.0);
    changed
}

/// A macOS-style slider: a thin track, the accent up to the knob, a white knob.
fn slider(
    ui: &mut Ui,
    t: &Tokens,
    id: Id,
    r: Rect,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    step: f32,
) -> bool {
    let (lo, hi) = (*range.start(), *range.end());
    let knob_r = 8.5;
    let x0 = r.left() + knob_r;
    let x1 = r.right() - knob_r;
    let resp = ui.interact(r, id, Sense::click_and_drag());
    let mut changed = false;
    if (resp.dragged() || resp.clicked())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let f = ((pos.x - x0) / (x1 - x0)).clamp(0.0, 1.0);
        let mut v = lo + f * (hi - lo);
        if step > 0.0 {
            v = (v / step).round() * step;
        }
        let v = v.clamp(lo, hi);
        if (v - *value).abs() > f32::EPSILON {
            *value = v;
            changed = true;
        }
    }
    let f = if hi > lo {
        ((*value - lo) / (hi - lo)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let cy = r.center().y;
    let kx = x0 + f * (x1 - x0);
    let p = ui.painter();
    let track = Rect::from_min_max(pos2(x0, cy - 2.0), pos2(x1, cy + 2.0));
    p.rect_filled(track, 2.0, t.control_border);
    p.rect_filled(
        Rect::from_min_max(track.min, pos2(kx, track.max.y)),
        2.0,
        t.accent,
    );
    p.circle_filled(
        pos2(kx, cy + 0.5),
        knob_r + 0.5,
        Color32::from_black_alpha(40),
    );
    p.circle(
        pos2(kx, cy),
        knob_r,
        Color32::WHITE,
        Stroke::new(0.5, t.control_border),
    );
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(text: &str, cuts: &[usize]) -> LayoutJob {
        let mut j = LayoutJob {
            text: text.to_string(),
            ..Default::default()
        };
        let mut start = 0;
        for &c in cuts.iter().chain(std::iter::once(&text.len())) {
            j.sections.push(LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(start)..ByteIndex(c),
                format: TextFormat::default(),
            });
            start = c;
        }
        j
    }

    #[test]
    fn ghost_text_is_spliced_in_and_moves_what_follows() {
        let font = FontId::monospace(12.0);
        // Inside a section: it is split around the suggestion.
        let mut j = job("abc def", &[4]);
        splice_ghost(&mut j, 2, "XY", &font, Color32::RED);
        assert_eq!(j.text, "abXYc def");
        let ranges: Vec<_> = j
            .sections
            .iter()
            .map(|s| s.byte_range.start.0..s.byte_range.end.0)
            .collect();
        assert_eq!(ranges, vec![0..2, 2..4, 4..6, 6..9]);
        assert_eq!(j.sections[1].format.color, Color32::RED);
        // At a section boundary and at the end.
        let mut j = job("ab", &[1]);
        splice_ghost(&mut j, 1, "Z", &font, Color32::RED);
        assert_eq!(j.text, "aZb");
        let ranges: Vec<_> = j
            .sections
            .iter()
            .map(|s| s.byte_range.start.0..s.byte_range.end.0)
            .collect();
        assert_eq!(ranges, vec![0..1, 1..2, 2..3]);
        let mut j = job("ab", &[]);
        splice_ghost(&mut j, 2, "\nnext", &font, Color32::RED);
        assert_eq!(j.text, "ab\nnext");
        let last = &j.sections.last().unwrap().byte_range;
        assert_eq!((last.start.0, last.end.0), (2, 7));
        // Not on a character boundary: left alone.
        let mut j = job("你好", &[]);
        splice_ghost(&mut j, 1, "x", &font, Color32::RED);
        assert_eq!(j.text, "你好");
    }
}
