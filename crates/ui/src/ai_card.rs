//! The result card of a right-click AI action (`texcraft_engine::ai_actions`), as macOS Writing
//! Tools and Notion AI show one: it floats over the editor under the text the action ran on and
//! the answer streams into it. A rewrite shows its changes marked, as proofreading does (added
//! words on a green tint, removed ones struck through in red), with Replace, Copy, Try Again and
//! Discard; an answer offers Continue in Assistant instead of Replace.

use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{Align2, FontId, Galley, Id, Key, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use texcraft_ai::actions::{Change, Output, word_diff};
use texcraft_engine::Session;
use texcraft_i18n::tr;

use crate::icons;
use crate::menu::{self, ButtonKind};
use crate::theme::{Tokens, semibold};

/// Where the card hangs: the text the action ran on and the editor's visible area (points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    /// Around the text.
    pub text: Rect,
    /// The editor's visible text area.
    pub view: Rect,
}

/// The widest card, and the tallest its text gets before it scrolls.
const WIDTH: f32 = 460.0;
const BODY_MAX: f32 = 240.0;
/// Inner margin.
const MARGIN: f32 = 12.0;

fn card_id() -> Id {
    Id::new("ai-action-card")
}

/// Where the card for the action shown hangs in the active document laid out as `galley` at
/// `gpos` with `view` visible (`None`: no action result, or it is about another document).
pub fn anchor(session: &Session, galley: &Galley, gpos: Pos2, view: Rect) -> Option<Anchor> {
    let run = session.ai.action.as_ref()?;
    let doc = session.doc()?;
    if doc.typeset_path() != run.doc.as_deref() {
        return None;
    }
    let n = doc.text.chars().count();
    let start = run.start.min(n);
    let end = start.saturating_add(run.original.chars().count()).min(n);
    let text = crate::editor::flash_rects(galley, start, end)
        .into_iter()
        .reduce(|a, b| a.union(b))
        .unwrap_or_else(|| galley.pos_from_cursor(CCursor::new(start)))
        .translate(gpos.to_vec2());
    Some(Anchor { text, view })
}

/// Text in `font` and `color`, wrapped to `width`.
fn plain(text: &str, font: FontId, color: egui::Color32, width: f32) -> LayoutJob {
    let mut job = LayoutJob::simple(text.to_string(), font, color, width);
    job.wrap.max_width = width;
    job
}

/// A rewrite with its changes against `original` marked.
fn marked(t: &Tokens, original: &str, rewrite: &str, width: f32) -> LayoutJob {
    let font = FontId::proportional(13.0);
    let mut job = LayoutJob::default();
    let mut after_removed = false;
    for (change, piece) in word_diff(original, rewrite) {
        // A replacement stands a little apart from the words it replaces.
        let gap = if after_removed && change == Change::Added {
            3.0
        } else {
            0.0
        };
        after_removed = change == Change::Removed;
        let format = match change {
            Change::Same => TextFormat::simple(font.clone(), t.text),
            Change::Added => TextFormat {
                background: t.success.gamma_multiply(0.22),
                ..TextFormat::simple(font.clone(), t.text)
            },
            Change::Removed => TextFormat {
                strikethrough: Stroke::new(1.0, t.error),
                ..TextFormat::simple(font.clone(), t.error)
            },
        };
        job.append(&piece, gap, format);
    }
    job.wrap.max_width = width;
    job
}

/// Draws the card for the action shown at `anchor` (nothing without one). `escape`: Esc is
/// the card's (no completion list or menu is up to take it).
pub fn show(ui: &Ui, t: &Tokens, session: &mut Session, anchor: Option<Anchor>, escape: bool) {
    let Some(a) = anchor else { return };
    let Some(run) = session.ai.action.as_ref() else {
        return;
    };
    let id = card_id();
    let w = WIDTH.min(a.view.width() - 24.0).max(260.0);
    let h = ui
        .ctx()
        .memory(|m| m.area_rect(id))
        .map_or(160.0, |r| r.height());
    // Under the text when it fits in view, else over it, else at the bottom of the view.
    let x = a
        .text
        .left()
        .min(a.view.right() - w - 8.0)
        .max(a.view.left() + 8.0);
    let below = a.text.bottom() + 8.0;
    let above = a.text.top() - 8.0 - h;
    let y = if below + h <= a.view.bottom() - 8.0 {
        below
    } else if above >= a.view.top() + 8.0 {
        above
    } else {
        (a.view.bottom() - 8.0 - h).max(a.view.top() + 8.0)
    };
    let reply = run.reply();
    let ready = run.is_ready();
    let mut clicked: Option<&'static str> = None;
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(x, y))
        .show(ui.ctx(), |ui| {
            menu::frame(t)
                .inner_margin(egui::Margin::same(MARGIN as i8))
                .show(ui, |ui| {
                    let inner = w - 2.0 * MARGIN;
                    ui.set_width(inner);
                    ui.spacing_mut().item_spacing = vec2(0.0, 8.0);
                    // ---- The action, the model, close.
                    let (hr, _) = ui.allocate_exact_size(vec2(inner, 18.0), Sense::hover());
                    let p = ui.painter().clone();
                    icons::sparkles(
                        &p,
                        Rect::from_center_size(
                            pos2(hr.left() + 7.0, hr.center().y),
                            vec2(14.0, 14.0),
                        ),
                        t.accent,
                        true,
                    );
                    let title = p.layout_no_wrap(run.label.clone(), semibold(12.5), t.text);
                    let title_w = title.size().x;
                    p.galley(
                        pos2(hr.left() + 20.0, hr.center().y - title.size().y / 2.0),
                        title,
                        t.text,
                    );
                    let xr = Rect::from_center_size(
                        pos2(hr.right() - 8.0, hr.center().y),
                        vec2(18.0, 18.0),
                    );
                    let close = ui
                        .interact(xr, id.with("close"), Sense::click())
                        .on_hover_text(tr("Discard Result"));
                    if close.hovered() {
                        p.rect_filled(xr, 5.0, t.row_hover);
                    }
                    icons::close(&p, xr.shrink(5.0), t.text_weak);
                    if close.clicked() {
                        clicked = Some("ai.action.dismiss");
                    }
                    let f = FontId::proportional(10.5);
                    let room = (xr.left() - 8.0 - (hr.left() + 20.0 + title_w + 12.0)).max(0.0);
                    let model = crate::widgets::elide(&p, &run.model, &f, room);
                    p.text(
                        pos2(xr.left() - 6.0, hr.center().y),
                        Align2::RIGHT_CENTER,
                        model,
                        f,
                        t.text_weak,
                    );

                    // ---- The answer.
                    egui::ScrollArea::vertical()
                        .id_salt(("ai-action-card-body", run.serial))
                        .max_height(BODY_MAX)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            let width = ui.available_width() - 4.0;
                            if let Some(e) = &run.error {
                                let job = plain(e, FontId::proportional(12.0), t.error, width);
                                ui.add(egui::Label::new(job).selectable(true));
                            } else if run.pending && reply.text.is_empty() {
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
                                icons::spinner(
                                    ui.painter(),
                                    Rect::from_center_size(
                                        pos2(r.left() + 6.0, r.center().y),
                                        vec2(11.0, 11.0),
                                    ),
                                    t.text_weak,
                                    ui.input(|i| i.time) as f32 * 6.0,
                                );
                                ui.painter().text(
                                    pos2(r.left() + 17.0, r.center().y),
                                    Align2::LEFT_CENTER,
                                    tr("Thinking…"),
                                    FontId::proportional(11.5),
                                    t.text_weak,
                                );
                            } else {
                                let job = match run.output {
                                    // Changes are marked once the rewrite is whole.
                                    Output::Rewrite if !run.pending => {
                                        marked(t, &run.original, &reply.text, width)
                                    }
                                    Output::Rewrite => plain(
                                        &reply.text,
                                        FontId::proportional(13.0),
                                        t.text,
                                        width,
                                    ),
                                    Output::Answer => plain(
                                        run.answer.trim(),
                                        FontId::proportional(12.5),
                                        t.text,
                                        width,
                                    ),
                                };
                                ui.add(egui::Label::new(job).selectable(true));
                                if run.output == Output::Rewrite && !reply.note.is_empty() {
                                    let job = plain(
                                        &reply.note,
                                        FontId::proportional(11.5),
                                        t.text_weak,
                                        width,
                                    );
                                    ui.add(egui::Label::new(job).selectable(true));
                                }
                            }
                            if run.pending {
                                ui.ctx().request_repaint();
                            }
                        });

                    // ---- What to do with it (the default action rightmost, as on macOS).
                    let mut buttons: Vec<(&str, ButtonKind, &'static str)> =
                        vec![(tr("Discard"), ButtonKind::Normal, "ai.action.dismiss")];
                    if !run.pending {
                        buttons.push((tr("Try Again"), ButtonKind::Normal, "ai.action.retry"));
                    }
                    if ready {
                        buttons.push((tr("Copy"), ButtonKind::Normal, "ai.action.copy"));
                        buttons.push(match run.output {
                            Output::Rewrite => {
                                (tr("Replace"), ButtonKind::Default, "ai.action.replace")
                            }
                            Output::Answer => (
                                tr("Continue in Assistant"),
                                ButtonKind::Default,
                                "ai.action.toAssistant",
                            ),
                        });
                    }
                    let labels: Vec<(&str, ButtonKind)> =
                        buttons.iter().map(|(l, k, _)| (*l, *k)).collect();
                    if let Some(i) = menu::dialog_buttons(ui, t, &labels) {
                        clicked = buttons.get(i).map(|(_, _, c)| *c).or(clicked);
                    }
                });
        });
    if escape && clicked.is_none() && ui.input(|i| i.key_pressed(Key::Escape)) {
        clicked = Some("ai.action.dismiss");
    }
    if let Some(cmd) = clicked
        && let Err(e) = session.run(cmd, &serde_json::Value::Null)
    {
        session.set_status(e.to_string());
    }
}
