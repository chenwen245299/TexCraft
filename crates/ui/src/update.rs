//! The update window, in the style of the author's Argus app: a version badge and "Release Notes"
//! header, the notes set straight on the window (simple Markdown), a "star / issues" strip, and a
//! footer that turns into a progress bar while downloading. It follows `Session::update`; the
//! work happens in the engine and `texcraft-update`.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Color32, Context, FontId, Galley, Id, Key, Rect, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use texcraft_engine::update::REPO_URL;
use texcraft_engine::{CURRENT_VERSION, Session, UpdateStatus};
use texcraft_i18n::{tr, trf};

use crate::prefs::{Pal, pal};
use crate::theme::{Tokens, editor_font, semibold};

const W: f32 = 520.0;
const PAD: f32 = 18.0;
const HEADER_H: f32 = 52.0;
const CTA_H: f32 = 86.0;
const FOOTER_H: f32 = 58.0;
/// Release notes taller than this scroll.
const NOTES_MAX: f32 = 300.0;

/// Draws the update window while `open`. Closing it keeps a downloaded update for installing
/// on quit.
pub fn show(ctx: &Context, t: &Tokens, session: &mut Session, open: &mut bool) {
    if !*open {
        return;
    }
    if session.update.status == UpdateStatus::Idle && !session.update.is_busy() {
        *open = false;
        return;
    }
    let p = pal(t);
    let frame = crate::menu::sheet_frame(t, 0);
    let mut close = false;
    egui::Modal::new(Id::new("update-window"))
        .frame(frame)
        .backdrop_color(crate::menu::backdrop())
        .show(ctx, |ui| close = window(ui, t, &p, session));
    let downloading = matches!(session.update.status, UpdateStatus::Downloading { .. });
    if ctx.input(|i| i.key_pressed(Key::Escape)) && !downloading {
        close = true;
    }
    if close {
        *open = false;
        session.update.interactive = false;
        match session.update.status {
            // "Later": the next automatic check offers it again; a download keeps going and a
            // downloaded update installs on quit.
            UpdateStatus::Available(_) | UpdateStatus::UpToDate | UpdateStatus::Failed(_) => {
                session.update.status = UpdateStatus::Idle;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Release notes: a small Markdown subset (headings, lists, paragraphs, **bold**, `code`).

enum Block {
    Heading(String),
    Bullet(String, String),
    Text(String),
    Gap,
}

fn blocks(notes: &str) -> Vec<Block> {
    let mut out = Vec::new();
    for line in notes.lines() {
        let l = line.trim();
        if l.is_empty() {
            if !matches!(out.last(), Some(Block::Gap) | None) {
                out.push(Block::Gap);
            }
        } else if let Some(h) = l
            .strip_prefix("### ")
            .or(l.strip_prefix("## "))
            .or(l.strip_prefix("# "))
        {
            out.push(Block::Heading(h.to_string()));
        } else if let Some(b) = l
            .strip_prefix("- ")
            .or(l.strip_prefix("* "))
            .or(l.strip_prefix("+ "))
        {
            out.push(Block::Bullet("•".into(), b.to_string()));
        } else if let Some((n, rest)) = l.split_once(". ")
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
        {
            out.push(Block::Bullet(format!("{n}."), rest.to_string()));
        } else {
            out.push(Block::Text(l.to_string()));
        }
    }
    while matches!(out.last(), Some(Block::Gap)) {
        out.pop();
    }
    out
}

/// `text` with **bold** and `code` spans, wrapped to `width`.
fn inline_job(
    ui: &Ui,
    t: &Tokens,
    p: &Pal,
    text: &str,
    size: f32,
    bold_all: bool,
    width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    let line_height = Some((size * 1.6).round());
    let plain = TextFormat {
        font_id: if bold_all {
            semibold(size)
        } else {
            FontId::proportional(size)
        },
        color: if bold_all { p.label } else { p.desc },
        line_height,
        ..Default::default()
    };
    let bold = TextFormat {
        font_id: semibold(size),
        color: p.label,
        line_height,
        ..Default::default()
    };
    let code = TextFormat {
        font_id: editor_font(size * 0.9),
        color: p.label,
        background: p.control,
        line_height,
        ..Default::default()
    };
    let mut rest = text;
    while !rest.is_empty() {
        let next_bold = rest.find("**");
        let next_code = rest.find('`');
        let (pos, marker) = match (next_bold, next_code) {
            (Some(b), Some(c)) if c < b => (c, "`"),
            (Some(b), _) => (b, "**"),
            (None, Some(c)) => (c, "`"),
            (None, None) => (rest.len(), ""),
        };
        job.append(rest.get(..pos).unwrap_or(""), 0.0, plain.clone());
        if marker.is_empty() {
            break;
        }
        let after = rest.get(pos + marker.len()..).unwrap_or("");
        match after.find(marker) {
            Some(end) => {
                let inner = after.get(..end).unwrap_or("");
                let fmt = if marker == "`" {
                    code.clone()
                } else {
                    bold.clone()
                };
                job.append(inner, 0.0, fmt);
                rest = after.get(end + marker.len()..).unwrap_or("");
            }
            None => {
                job.append(marker, 0.0, plain.clone());
                rest = after;
            }
        }
    }
    job.wrap.max_width = width;
    let _ = t;
    ui.fonts_mut(|f| f.layout_job(job))
}

/// One laid-out block of notes: indent, optional bullet, text, space above.
type NoteLine = (f32, Option<Arc<Galley>>, Arc<Galley>, f32);

/// Lays the notes out, one [`NoteLine`] per block.
fn layout_notes(ui: &Ui, t: &Tokens, p: &Pal, notes: &str, width: f32) -> Vec<NoteLine> {
    let mut out = Vec::new();
    let mut gap: f32 = 0.0;
    for b in blocks(notes) {
        match b {
            Block::Gap => gap = 8.0,
            Block::Heading(h) => {
                let g = inline_job(ui, t, p, &h, 13.0, true, width);
                out.push((0.0, None, g, if out.is_empty() { 0.0 } else { 12.0 }));
                gap = 0.0;
            }
            Block::Bullet(mark, text) => {
                let indent = 18.0;
                let m = inline_job(ui, t, p, &mark, 12.5, false, indent);
                let g = inline_job(ui, t, p, &text, 12.5, false, width - indent);
                out.push((
                    indent,
                    Some(m),
                    g,
                    if out.is_empty() { 0.0 } else { gap.max(3.0) },
                ));
                gap = 0.0;
            }
            Block::Text(text) => {
                let g = inline_job(ui, t, p, &text, 12.5, false, width);
                out.push((0.0, None, g, if out.is_empty() { 0.0 } else { gap }));
                gap = 0.0;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Pieces

fn hairline(ui: &Ui, p: &Pal, x0: f32, x1: f32, y: f32) {
    ui.painter()
        .line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(1.0, p.hairline));
}

/// A flat button (`accent`: filled with the accent colour). Returns whether it was clicked.
fn flat_button(
    ui: &mut Ui,
    t: &Tokens,
    p: &Pal,
    id: &str,
    r: Rect,
    label: &str,
    accent: bool,
) -> bool {
    let resp = ui.interact(r, Id::new(("update-button", id)), Sense::click());
    let (fill, text) = if accent {
        (
            if resp.hovered() {
                t.accent.gamma_multiply(0.85)
            } else {
                t.accent
            },
            Color32::WHITE,
        )
    } else {
        (
            if resp.hovered() {
                p.control_hover
            } else {
                p.control
            },
            p.desc,
        )
    };
    ui.painter().rect_filled(r, 7.0, fill);
    ui.painter().text(
        r.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.5),
        text,
    );
    resp.clicked()
}

/// Footer buttons from the right edge; returns their rects in label order.
fn footer_rects(ui: &Ui, right: f32, cy: f32, labels: &[&str]) -> Vec<Rect> {
    let mut x = right;
    let mut out = Vec::new();
    for label in labels.iter().rev() {
        let w = ui
            .painter()
            .layout_no_wrap(
                (*label).to_string(),
                FontId::proportional(12.5),
                Color32::BLACK,
            )
            .size()
            .x
            + 30.0;
        out.push(Rect::from_min_size(pos2(x - w, cy - 14.0), vec2(w, 28.0)));
        x -= w + 8.0;
    }
    out.reverse();
    out
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

// ---------------------------------------------------------------------------------------------

/// The window. Returns true when it should close.
fn window(ui: &mut Ui, t: &Tokens, p: &Pal, session: &mut Session) -> bool {
    let status = session.update.status.clone();
    let release = match &status {
        UpdateStatus::Available(r) | UpdateStatus::Downloading { release: r, .. } => {
            Some(r.clone())
        }
        UpdateStatus::Ready(s) => Some(s.release.clone()),
        _ => None,
    };
    let inner_w = W - 2.0 * PAD;
    let notes = release.as_ref().map(|r| {
        let text = if r.notes.trim().is_empty() {
            trf!(
                "A new version v{version} is available. Update now?",
                version = r.version
            )
        } else {
            r.notes.clone()
        };
        layout_notes(ui, t, p, &text, inner_w)
    });
    let message = match &status {
        UpdateStatus::Checking | UpdateStatus::Idle => tr("Checking for updates…").to_string(),
        UpdateStatus::UpToDate => trf!(
            "You are up to date: TexCraft {version} is the newest version.",
            version = CURRENT_VERSION
        ),
        UpdateStatus::Failed(msg) if release.is_none() => msg.clone(),
        _ => String::new(),
    };
    let notes_h = notes.as_ref().map_or(0.0, |n| {
        n.iter().map(|(_, _, g, gap)| g.size().y + gap).sum::<f32>()
    });
    // In a small window (the welcome window at start-up) the notes scroll sooner.
    let room = ui.ctx().content_rect().height() - HEADER_H - CTA_H - FOOTER_H - 64.0;
    let notes_max = room.clamp(80.0, NOTES_MAX);
    let body_h = if notes.is_some() {
        notes_h.clamp(40.0, notes_max) + 32.0
    } else {
        58.0
    };
    let cta_h = if release.is_some() { CTA_H } else { 0.0 };
    let (rect, _) = ui.allocate_exact_size(
        vec2(W, HEADER_H + body_h + cta_h + FOOTER_H),
        Sense::hover(),
    );
    let (x0, x1) = (rect.left(), rect.right());
    let mut close = false;

    // ---- Header: version badge, title, close button.
    let hy = rect.top() + HEADER_H / 2.0 + 2.0;
    let mut tx = x0 + PAD;
    if let Some(r) = &release {
        let badge_font = semibold(11.0);
        let text = format!("v{}", r.version);
        let w = ui
            .painter()
            .layout_no_wrap(text.clone(), badge_font.clone(), Color32::WHITE)
            .size()
            .x
            + 14.0;
        let badge = Rect::from_min_size(pos2(tx, hy - 9.0), vec2(w, 18.0));
        ui.painter().rect_filled(badge, 6.0, t.accent);
        ui.painter().text(
            badge.center(),
            Align2::CENTER_CENTER,
            text,
            badge_font,
            Color32::WHITE,
        );
        tx = badge.right() + 10.0;
    }
    let title = if release.is_some() {
        tr("Release Notes")
    } else {
        tr("Check for Updates")
    };
    ui.painter().text(
        pos2(tx, hy),
        Align2::LEFT_CENTER,
        title,
        semibold(15.0),
        p.label,
    );
    let xr = Rect::from_center_size(pos2(x1 - PAD - 13.0, hy), vec2(26.0, 26.0));
    let xresp = ui.interact(xr, Id::new("update-close"), Sense::click());
    ui.painter().rect_filled(
        xr,
        6.0,
        if xresp.hovered() {
            p.control_hover
        } else {
            p.control
        },
    );
    let c = xr.center();
    for (a, b) in [
        (vec2(-4.0, -4.0), vec2(4.0, 4.0)),
        (vec2(-4.0, 4.0), vec2(4.0, -4.0)),
    ] {
        ui.painter()
            .line_segment([c + a, c + b], Stroke::new(1.3, p.desc));
    }
    if xresp.clicked() {
        close = true;
    }
    let mut y = rect.top() + HEADER_H;
    hairline(ui, p, x0, x1, y);

    // ---- Body: the notes, straight on the window.
    let body = Rect::from_min_max(pos2(x0 + PAD, y + 16.0), pos2(x1 - PAD, y + body_h - 16.0));
    if let Some(blocks) = notes {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        child.set_clip_rect(body.expand2(vec2(PAD - 4.0, 2.0)));
        egui::ScrollArea::vertical()
            .id_salt("update-notes")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (indent, mark, g, gap) in blocks {
                    ui.add_space(gap);
                    let (r, _) = ui.allocate_exact_size(vec2(inner_w, g.size().y), Sense::hover());
                    if let Some(m) = mark {
                        ui.painter().galley(r.min + vec2(2.0, 0.0), m, p.desc);
                    }
                    ui.painter().galley(r.min + vec2(indent, 0.0), g, p.desc);
                }
            });
    } else {
        let g = ui
            .painter()
            .layout(message, FontId::proportional(13.0), p.desc, inner_w);
        ui.painter().galley(
            pos2(body.left(), body.center().y - g.size().y / 2.0),
            g,
            p.desc,
        );
    }
    y += body_h;

    // ---- "Star / issues" strip.
    if release.is_some() {
        let strip = Rect::from_min_max(pos2(x0, y), pos2(x1, y + cta_h));
        ui.painter()
            .rect_filled(strip, 0.0, t.accent.gamma_multiply(0.06));
        hairline(ui, p, x0, x1, y);
        for (i, (text, label, url)) in [
            (
                tr("If you find TexCraft useful, please star the project on GitHub."),
                tr("★  Star"),
                REPO_URL.to_string(),
            ),
            (
                tr("Have a feature request or found a bug? Open a GitHub issue."),
                tr("Issues"),
                format!("{REPO_URL}/issues"),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let cy = y + 12.0 + 15.0 + 31.0 * i as f32;
            ui.painter().text(
                pos2(x0 + PAD, cy),
                Align2::LEFT_CENTER,
                text,
                FontId::proportional(12.0),
                p.desc,
            );
            let b = Rect::from_min_size(pos2(x1 - PAD - 88.0, cy - 13.0), vec2(88.0, 26.0));
            let resp = ui.interact(b, Id::new(("update-cta", i)), Sense::click());
            ui.painter().rect_filled(
                b,
                7.0,
                if resp.hovered() {
                    p.control_hover
                } else {
                    p.control
                },
            );
            ui.painter().rect_stroke(
                b,
                7.0,
                Stroke::new(0.5, p.hairline_strong),
                StrokeKind::Inside,
            );
            ui.painter().text(
                b.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(12.0),
                if resp.hovered() { t.accent } else { p.label },
            );
            if resp.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            }
        }
        y += cta_h;
    }

    // ---- Footer.
    hairline(ui, p, x0, x1, y);
    let cy = y + FOOTER_H / 2.0 - 1.0;
    let right = x1 - PAD;
    match &status {
        UpdateStatus::Downloading { done, total, .. } => {
            let label = trf!(
                "Downloading… {percent}%",
                percent = (done.saturating_mul(100))
                    .checked_div(*total)
                    .unwrap_or(0)
                    .min(100)
            );
            let lw = 128.0;
            let bar =
                Rect::from_min_max(pos2(x0 + PAD, cy - 3.0), pos2(right - lw - 10.0, cy + 3.0));
            ui.painter().rect_filled(bar, 3.0, p.control);
            let f = if *total > 0 {
                (*done as f32 / *total as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ui.painter().rect_filled(
                Rect::from_min_size(bar.min, vec2(bar.width() * f, 6.0)),
                3.0,
                t.accent,
            );
            ui.painter().text(
                pos2(right, cy),
                Align2::RIGHT_CENTER,
                label,
                FontId::proportional(12.0),
                p.desc,
            );
            let tip = trf!("{done} of {total}", done = mb(*done), total = mb(*total));
            ui.interact(bar, Id::new("update-progress"), Sense::hover())
                .on_hover_text(tip);
        }
        UpdateStatus::Checking => {
            let r = footer_rects(ui, right, cy, &[tr("Cancel")]);
            if let Some(b) = r.first()
                && flat_button(ui, t, p, "cancel", *b, tr("Cancel"), false)
            {
                let _ = session.run("app.cancelUpdate", &serde_json::Value::Null);
                close = true;
            }
        }
        UpdateStatus::Available(_) | UpdateStatus::Ready(_) => {
            // Skip, as a quiet text button on the left.
            let skip = ui.painter().layout_no_wrap(
                tr("Skip this version").into(),
                FontId::proportional(12.0),
                p.note,
            );
            let sr = Rect::from_min_size(pos2(x0 + PAD, cy - skip.size().y / 2.0), skip.size());
            let sresp = ui.interact(sr.expand(4.0), Id::new("update-skip"), Sense::click());
            ui.painter()
                .galley(sr.min, skip, if sresp.hovered() { p.desc } else { p.note });
            if sresp.clicked() {
                let _ = session.run("app.skipUpdate", &serde_json::Value::Null);
                close = true;
            }
            let ready = matches!(status, UpdateStatus::Ready(_));
            let primary = if ready {
                tr("Restart Now")
            } else {
                tr("Update Now")
            };
            let r = footer_rects(ui, right, cy, &[tr("Later"), primary]);
            if let Some(b) = r.first()
                && flat_button(ui, t, p, "later", *b, tr("Later"), false)
            {
                close = true;
            }
            if let Some(b) = r.get(1)
                && flat_button(ui, t, p, "primary", *b, primary, true)
            {
                let id = if ready {
                    "app.installUpdate"
                } else {
                    "app.downloadUpdate"
                };
                if let Err(e) = session.run(id, &serde_json::Value::Null) {
                    session.update.status = UpdateStatus::Failed(e.to_string());
                }
            }
            if ready {
                ui.painter().text(
                    pos2(r.first().map_or(right, |b| b.left()) - 10.0, cy),
                    Align2::RIGHT_CENTER,
                    tr("or installs when you quit"),
                    FontId::proportional(11.5),
                    p.note,
                );
            }
        }
        _ => {
            let r = footer_rects(ui, right, cy, &[tr("OK")]);
            if let Some(b) = r.first()
                && flat_button(ui, t, p, "ok", *b, tr("OK"), true)
            {
                close = true;
            }
        }
    }
    close
}
