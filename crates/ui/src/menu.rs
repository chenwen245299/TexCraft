//! Menus, pop-up lists and dialog buttons in one style, after the target's (macOS) menus measured
//! at 2 px per pt: a light-grey panel with 10 pt corners and a soft shadow, 24 pt rows of 13 pt
//! text 17 pt from the edge, a full-width accent highlight under the pointer, hairline
//! separators inset 12 pt. Every context menu, toolbar menu, pop-up button list and sheet uses
//! these helpers so they cannot drift apart.

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

use crate::icons;
use crate::theme::{Tokens, semibold};

/// Row height of a menu item.
pub const ROW: f32 = 25.0;
/// Menu text size.
pub const FONT: f32 = 13.0;
/// Inner margin of the menu panel.
pub const MARGIN: i8 = 5;
/// Text indent from the panel's inner edge (no check-mark column).
const INDENT: f32 = 12.0;
/// Text indent when the menu has a check-mark column.
const CHECK_INDENT: f32 = 22.0;
/// Room after the longest label (as in macOS menus, which fit their widest item).
const TRAILING: f32 = 22.0;
/// The least gap between a label and its shortcut.
const SHORTCUT_GAP: f32 = 30.0;

/// The menu panel (also used for tool tips and pop-overs): see [`Tokens::visuals`], which gives
/// egui's own pop-ups the same look.
pub fn frame(t: &Tokens) -> egui::Frame {
    egui::Frame::NONE
        .fill(t.popup_bg)
        .stroke(Stroke::new(0.5, t.menu_border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(MARGIN))
        .shadow(menu_shadow())
}

/// A sheet or dialog: the window background, 14 pt corners, a hairline and a deep shadow (the
/// Typeset Configuration sheet, the update window, alerts). `margin` is the inner margin.
pub fn sheet_frame(t: &Tokens, margin: i8) -> egui::Frame {
    egui::Frame::NONE
        .fill(t.form_bg)
        .corner_radius(CornerRadius::same(14))
        .stroke(Stroke::new(0.5, t.form_hairline_strong))
        .inner_margin(egui::Margin::same(margin))
        .shadow(egui::epaint::Shadow {
            offset: [0, 10],
            blur: 36,
            spread: 0,
            color: Color32::from_black_alpha(if t.dark { 120 } else { 55 }),
        })
}

/// The dimming behind a sheet.
pub fn backdrop() -> Color32 {
    Color32::from_black_alpha(46)
}

/// The menu panel's shadow.
pub fn menu_shadow() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 6],
        blur: 22,
        spread: 0,
        color: Color32::from_black_alpha(46),
    }
}

/// Prepares `ui` for menu rows (no gaps between rows) with at least `min_width`. The menu is as
/// wide as its widest row, never wider: see [`row_width`].
pub fn begin(ui: &mut Ui, min_width: f32) {
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    ui.set_min_width(min_width);
}

/// The width a menu row takes. While the menu measures itself (egui's sizing pass, the frame it
/// opens) each row asks only for its `natural` width, so the panel fits the widest one; after
/// that rows span the panel, so highlights and separators reach across. (Asking for the whole
/// available width up front would make every menu egui's default 400–600 pt wide.)
pub fn row_width(ui: &Ui, natural: f32) -> f32 {
    if ui.is_sizing_pass() {
        natural
    } else {
        ui.available_width().max(natural)
    }
}

fn text_width(ui: &Ui, text: &str, font: &FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE)
        .size()
        .x
}

/// One menu item's options.
#[derive(Debug, Clone, Copy, Default)]
pub struct Item<'a> {
    /// Shortcut shown at the right (`⌘T`).
    pub shortcut: &'a str,
    /// Leaves room for a check mark at the left; `Some(true)` draws it.
    pub checked: Option<bool>,
    /// Greyed out and not clickable.
    pub disabled: bool,
    /// A second, smaller line under the label.
    pub detail: Option<&'a str>,
    /// The label in semibold (a title-like entry).
    pub strong: bool,
    /// A logo before the label (an [`crate::ai_logos`] key).
    pub logo: Option<&'a str>,
    /// Leave the logo's room even without one, so the label lines up with its neighbours'.
    pub logo_column: bool,
}

/// A menu item; returns its response (`clicked()` when chosen). Closes the menu on click.
pub fn item(ui: &mut Ui, t: &Tokens, label: &str, opts: Item<'_>) -> Response {
    let h = if opts.detail.is_some() { 38.0 } else { ROW };
    let sense = if opts.disabled {
        Sense::hover()
    } else {
        Sense::click()
    };
    let font = if opts.strong {
        semibold(FONT)
    } else {
        FontId::proportional(FONT)
    };
    let lead = if opts.checked.is_some() {
        CHECK_INDENT
    } else {
        INDENT
    } + match (
        opts.logo.is_some() || opts.logo_column,
        opts.detail.is_some(),
    ) {
        (true, true) => 25.0,
        (true, false) => 22.0,
        (false, _) => 0.0,
    };
    let shortcut_w = if opts.shortcut.is_empty() {
        0.0
    } else {
        text_width(ui, opts.shortcut, &FontId::proportional(FONT))
    };
    let natural = (lead
        + text_width(ui, label, &font)
        + if opts.shortcut.is_empty() {
            TRAILING
        } else {
            SHORTCUT_GAP + shortcut_w + 10.0
        })
    .max(opts.detail.map_or(0.0, |d| {
        lead + text_width(ui, d, &FontId::proportional(11.0)) + TRAILING
    }));
    let (r, resp) = ui.allocate_exact_size(vec2(row_width(ui, natural), h), sense);
    let p = ui.painter();
    let hot = !opts.disabled && (resp.hovered() || resp.is_pointer_button_down_on());
    if hot {
        p.rect_filled(r, 5.0, t.accent);
    }
    let (fg, weak) = if opts.disabled {
        (t.menu_disabled, t.menu_disabled)
    } else if hot {
        (Color32::WHITE, Color32::from_white_alpha(215))
    } else {
        (t.text, t.text_weak)
    };
    let mut x = r.left()
        + if opts.checked.is_some() {
            CHECK_INDENT
        } else {
            INDENT
        };
    let line1 = if opts.detail.is_some() {
        r.top() + 12.5
    } else {
        r.center().y
    };
    if opts.checked == Some(true) {
        icons::check_mark(p, pos2(r.left() + 11.0, line1), 12.0, fg);
    }
    if let Some(key) = opts.logo {
        let side = if opts.detail.is_some() { 18.0 } else { 15.0 };
        crate::ai_logos::paint(
            ui,
            key,
            Rect::from_min_size(pos2(x, line1 - side / 2.0), vec2(side, side)),
            fg,
        );
        x += side + 7.0;
    } else if opts.logo_column {
        x += if opts.detail.is_some() { 25.0 } else { 22.0 };
    }
    let shortcut_room = if opts.shortcut.is_empty() {
        0.0
    } else {
        shortcut_w + 16.0
    };
    let text = crate::widgets::elide(p, label, &font, r.right() - 10.0 - shortcut_room - x);
    p.text(pos2(x, line1), Align2::LEFT_CENTER, text, font, fg);
    if let Some(detail) = opts.detail {
        let f = FontId::proportional(11.0);
        let d = crate::widgets::elide(p, detail, &f, r.right() - 10.0 - x);
        p.text(pos2(x, r.top() + 27.5), Align2::LEFT_CENTER, d, f, weak);
    }
    if !opts.shortcut.is_empty() {
        p.text(
            pos2(r.right() - 10.0, line1),
            Align2::RIGHT_CENTER,
            opts.shortcut,
            FontId::proportional(FONT),
            weak,
        );
    }
    if resp.clicked() {
        ui.close();
    }
    resp
}

/// A menu item that runs command `id`: label (unless `label` is given), shortcut, enabled and
/// check-mark state come from the command registry. `check_column` leaves room for marks.
pub fn command(
    ui: &mut Ui,
    t: &Tokens,
    session: &mut texcraft_engine::Session,
    id: &str,
    label: Option<&str>,
    check_column: bool,
) {
    let Some(spec) = texcraft_engine::registry().iter().find(|c| c.id == id) else {
        return;
    };
    let shortcut = spec.shortcut.map(|s| s.display()).unwrap_or_default();
    let opts = Item {
        shortcut: &shortcut,
        checked: (check_column || spec.checked.is_some()).then(|| session.is_checked(id)),
        disabled: !session.is_enabled(id),
        ..Default::default()
    };
    if item(ui, t, label.unwrap_or(spec.label()), opts).clicked()
        && let Err(e) = session.run(id, &serde_json::Value::Null)
    {
        session.set_status(e.to_string());
    }
}

/// A plain menu item: `item` with default options; returns whether it was chosen.
pub fn button(ui: &mut Ui, t: &Tokens, label: &str) -> bool {
    item(ui, t, label, Item::default()).clicked()
}

/// A hairline between groups of items.
pub fn separator(ui: &mut Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(row_width(ui, 0.0), 11.0), Sense::hover());
    ui.painter().line_segment(
        [
            pos2(r.left() + INDENT, r.center().y),
            pos2(r.right() - INDENT, r.center().y),
        ],
        Stroke::new(1.0, t.menu_separator),
    );
}

/// A small grey heading over a group of items ("Select Page Mode").
/// A section heading with a logo before it (an [`crate::ai_logos`] key): a provider over its
/// models.
pub fn heading_with_logo(ui: &mut Ui, t: &Tokens, text: &str, logo: Option<&str>) {
    let Some(key) = logo else {
        heading(ui, t, text);
        return;
    };
    let side = 13.0;
    let natural = INDENT + side + 6.0 + text_width(ui, text, &semibold(11.0)) + TRAILING;
    let (r, _) = ui.allocate_exact_size(vec2(row_width(ui, natural), 24.0), Sense::hover());
    let cy = r.center().y + 1.0;
    crate::ai_logos::paint(
        ui,
        key,
        Rect::from_min_size(pos2(r.left() + INDENT, cy - side / 2.0), vec2(side, side)),
        t.text_weak,
    );
    ui.painter().text(
        pos2(r.left() + INDENT + side + 6.0, cy),
        Align2::LEFT_CENTER,
        text,
        semibold(11.0),
        t.text_weak,
    );
}

pub fn heading(ui: &mut Ui, t: &Tokens, text: &str) {
    let natural = INDENT + text_width(ui, text, &semibold(11.0)) + TRAILING;
    let (r, _) = ui.allocate_exact_size(vec2(row_width(ui, natural), 22.0), Sense::hover());
    ui.painter().text(
        pos2(r.left() + INDENT, r.center().y + 1.0),
        Align2::LEFT_CENTER,
        text,
        semibold(11.0),
        t.text_weak,
    );
}

/// What a dialog button looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// The default action: accent fill, white text (Return).
    Default,
    /// Any other action: grey fill.
    Normal,
}

/// A sheet / dialog push button (macOS 26 style: a rounded capsule) of `width` points.
pub fn dialog_button(ui: &mut Ui, t: &Tokens, label: &str, kind: ButtonKind, width: f32) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    let p = ui.painter();
    let pressed = resp.is_pointer_button_down_on();
    let (fill, fg) = match kind {
        ButtonKind::Default => (
            if pressed {
                t.accent.gamma_multiply(0.85)
            } else {
                t.accent
            },
            Color32::WHITE,
        ),
        ButtonKind::Normal => (
            if pressed {
                t.control_hover
            } else if resp.hovered() {
                t.form_popup
            } else {
                t.control_bg
            },
            t.control_text,
        ),
    };
    p.rect_filled(r, 14.0, fill);
    if kind == ButtonKind::Normal {
        p.rect_stroke(
            r,
            14.0,
            Stroke::new(0.5, t.menu_border),
            egui::StrokeKind::Inside,
        );
    }
    p.text(
        r.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0),
        fg,
    );
    resp.clicked()
}

/// A row of dialog buttons, right-aligned, the default one last (rightmost) as on macOS.
/// Returns the index of the clicked button.
pub fn dialog_buttons(ui: &mut Ui, t: &Tokens, buttons: &[(&str, ButtonKind)]) -> Option<usize> {
    let mut clicked = None;
    let widths: Vec<f32> = buttons
        .iter()
        .map(|(l, _)| {
            ui.painter()
                .layout_no_wrap((*l).to_string(), FontId::proportional(13.0), t.text)
                .size()
                .x
                .max(52.0)
                + 28.0
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 8.0 * (buttons.len().saturating_sub(1)) as f32;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let mut x = row.right() - total;
    for (i, ((label, kind), w)) in buttons.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(pos2(x, row.top()), vec2(*w, 28.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        if dialog_button(&mut child, t, label, *kind, *w) {
            clicked = Some(i);
        }
        x += w + 8.0;
    }
    clicked
}
