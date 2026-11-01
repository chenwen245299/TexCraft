//! The sidebar's AI Assistant tab: a model menu, the conversation (answers rendered from a
//! small Markdown subset, with LaTeX code blocks that insert into the document in one click),
//! and the question box. Providers and models are set up in Settings → AI.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Color32, CornerRadius, FontId, Galley, Id, Key, Modifiers, Rect, RichText, Sense,
    Stroke, Ui, pos2, vec2,
};
use texcraft_ai::Role;
use texcraft_engine::Session;
use texcraft_engine::ai_tools::{ToolKind, ToolUse};
use texcraft_engine::ai_usage::ContextUsage;
use texcraft_i18n::{tr, trf};

use crate::menu::{self, ButtonKind};
use crate::theme::{HEADER_HEIGHT, Tokens, editor_font, semibold};
use crate::{icons, widgets};

/// UI state of the Assistant tab.
#[derive(Default)]
pub struct AiPanelState {
    /// Why the last question could not be sent.
    error: Option<String>,
    /// Focus the question box on the next frame.
    focus: bool,
    /// Messages seen last frame (scroll to the bottom when it grows).
    seen: usize,
    /// The context meter's popover is open.
    context_open: bool,
    /// When it closes, the pointer having left the meter and the popover (Argus waits 180 ms, so
    /// the pointer can move from one to the other).
    context_close_at: Option<f64>,
    /// Esc or a click closed it while the pointer was on the meter: it opens again only once the
    /// pointer has left.
    context_dismissed: bool,
    /// Where the popover was drawn last frame.
    context_pop: Option<Rect>,
}

/// The app icon (assets/icon/texcraft.svg) over the Assistant's empty states.
const APP_ICON: &[u8] = include_bytes!("../../../assets/icon/texcraft.svg");

/// Paints the app icon into `r` (the artwork's tile fills about four fifths of it).
fn paint_app_icon(ui: &Ui, r: Rect) {
    egui::Image::new(egui::ImageSource::Bytes {
        uri: "bytes://texcraft/app-icon.svg".into(),
        bytes: egui::load::Bytes::Static(APP_ICON),
    })
    .fit_to_exact_size(r.size())
    .paint_at(ui, r);
}

/// Height of the attached images' strip in the question box.
const THUMBS_H: f32 = 50.0;

/// Widest the question box's document chip grows (pt): a long file name is elided, leaving the
/// rest of the row to what the conversation used.
const CHIP_MAX: f32 = 160.0;

/// The usage row's font size (that of the document chip beside it).
const STAT_SIZE: f32 = 10.5;
/// The context meter's ring: its diameter and line width (pt).
const RING: f32 = 13.0;
const RING_LINE: f32 = 2.0;
/// The context popover's width and padding (Argus's 290 and 14 pt).
const POP_W: f32 = 290.0;
const POP_PAD: f32 = 14.0;
/// How long the popover stays once the pointer has left (s).
const POP_GRACE: f64 = 0.18;

/// Widest the header's model menu grows (pt); longer model names are elided.
pub(crate) const MODEL_MENU_MAX: f32 = 240.0;

/// The question box's id (the app pastes images into it).
fn composer_id() -> Id {
    Id::new("ai-question")
}

/// Whether the Assistant's question box has keyboard focus (⌘V then attaches a copied image).
pub fn composer_focused(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.has_focus(composer_id()))
}

/// Whether a dropped or hovered file looks like an image the chat APIs take.
fn image_file(path: Option<&std::path::Path>, mime: &str) -> bool {
    mime.starts_with("image/")
        || path
            .and_then(|p| p.extension())
            .map(|e| e.to_string_lossy().to_lowercase())
            .is_some_and(|e| matches!(e.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp"))
}

/// Attaches the images among files dropped on the window.
fn attach_dropped(session: &mut Session, files: &[egui::DroppedFileHandle]) {
    let mut errors = Vec::new();
    for f in files {
        let path = f.path();
        if !image_file(Some(path), "") {
            continue;
        }
        if let Err(e) = session.ai_attach_file(path) {
            errors.push(e.to_string());
        }
    }
    session.ai.notice = (!errors.is_empty()).then(|| errors.join(" "));
}

/// Paints image `img` into `r`, cropped to fill it (PNG and JPEG; other formats as their name).
fn paint_image(ui: &Ui, t: &Tokens, img: &texcraft_ai::Image, r: Rect) {
    let p = ui.painter();
    p.rect_filled(r, 6.0, t.row_hover);
    let ext = match img.media_type.as_str() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        _ => None,
    };
    let texture = ext.and_then(|ext| {
        let uri = format!("bytes://texcraft/ai-image/{:p}.{ext}", img.data.as_ptr());
        let image = egui::Image::new(egui::ImageSource::Bytes {
            uri: uri.into(),
            bytes: egui::load::Bytes::Shared(img.data.clone()),
        });
        match image.load_for_size(ui.ctx(), r.size() * 2.0) {
            Ok(egui::load::TexturePoll::Ready { texture }) => Some(texture),
            _ => None,
        }
    });
    if let Some(tex) = texture {
        // Crop the longer side so the image covers the square.
        let (w, h) = (tex.size.x.max(1.0), tex.size.y.max(1.0));
        let (rw, rh) = (r.width().max(1.0), r.height().max(1.0));
        let scale = (rw / w).max(rh / h);
        let (u, v) = ((rw / (w * scale)).min(1.0), (rh / (h * scale)).min(1.0));
        let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(u, v));
        p.image(tex.id, r, uv, Color32::WHITE);
    } else {
        let label = img
            .media_type
            .strip_prefix("image/")
            .unwrap_or("image")
            .to_uppercase();
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(9.5),
            t.text_weak,
        );
    }
    p.rect_stroke(
        r,
        6.0,
        Stroke::new(0.5, t.badge_border),
        egui::StrokeKind::Inside,
    );
}

/// Questions offered on an empty conversation, in the interface language (a click sends one as
/// the user's question, so it goes in the language they would type it in).
fn suggestions() -> [&'static str; 4] {
    [
        tr("Find and fix the errors in this document"),
        tr("Explain the selected LaTeX"),
        tr("Write a table for my results"),
        tr("Improve the wording of the selection"),
    ]
}

fn send(session: &mut Session, st: &mut AiPanelState, text: Option<&str>) {
    let params = match text {
        Some(t) => serde_json::json!({ "text": t }),
        None => serde_json::Value::Null,
    };
    match session.run("ai.send", &params) {
        Ok(()) => {
            st.error = None;
            session.ai.notice = None;
        }
        Err(e) => st.error = Some(e.to_string()),
    }
}

/// Draws the Assistant tab into the sidebar body.
pub fn show(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut AiPanelState) {
    let body = ui.max_rect();
    let configured = session.ai.config.selection().is_some();

    // ---- Header: model menu, new conversation, settings. As tall as the PDF's header bar
    // beside it, so the two rows and their dividers line up.
    let header = Rect::from_min_size(body.min, vec2(body.width(), HEADER_HEIGHT));
    header_bar(ui, t, session, st, header);

    if !configured {
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(
            pos2(body.left() + 14.0, header.bottom()),
            pos2(body.right() - 14.0, body.bottom()),
        )));
        c.add_space(36.0);
        c.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
            paint_app_icon(ui, r);
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr("AI Assistant"))
                    .font(semibold(13.0))
                    .color(t.sidebar_text),
            );
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(tr(
                        "Connect a provider — SoCLaaS (NUS), Anthropic, OpenAI, Gemini, OpenRouter, DeepSeek, Kimi, Qwen, MiMo, GLM, MiniMax, StepFun or a local Ollama — to ask about your document, fix errors and write LaTeX.",
                    ))
                    .size(11.5)
                    .color(t.text_weak),
                )
                .wrap(),
            );
            ui.add_space(12.0);
            let w = (ui.available_width() - 10.0).clamp(80.0, 200.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
            let mut b = ui.new_child(egui::UiBuilder::new().max_rect(r));
            if menu::dialog_button(&mut b, t, tr("Set Up a Provider…"), ButtonKind::Default, w) {
                let _ = session.run("ai.settings", &serde_json::Value::Null);
            }
        });
        return;
    }

    // Images dropped on the window while the Assistant shows go with the question.
    let dropped = ui.input(|i| i.raw.dropped_files.clone());
    if !dropped.is_empty() {
        attach_dropped(session, &dropped);
    }

    // ---- Composer at the bottom (taller with images attached).
    let composer_h = if session.ai.draft_images.is_empty() {
        98.0
    } else {
        98.0 + THUMBS_H
    };
    let composer = Rect::from_min_max(
        pos2(body.left(), body.bottom() - composer_h),
        pos2(body.right(), body.bottom()),
    );
    let shown_error = st.error.clone().or_else(|| session.ai.notice.clone());
    let error_h = if shown_error.is_some() { 34.0 } else { 0.0 };
    let messages = Rect::from_min_max(
        pos2(body.left(), header.bottom()),
        pos2(body.right(), composer.top() - error_h),
    );

    // ---- Conversation.
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(messages));
    c.set_clip_rect(messages.intersect(ui.clip_rect()));
    // The scroll bar floats, but over its own strip: the conversation gives way to it rather
    // than running under it.
    let sc = &mut c.spacing_mut().scroll;
    sc.floating_allocated_width = sc.bar_width + sc.bar_outer_margin;
    let grew = session.ai.messages.len() != st.seen || session.ai.is_busy();
    st.seen = session.ai.messages.len();
    egui::ScrollArea::vertical()
        .id_salt("ai-scroll")
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(&mut c, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            if session.ai.messages.is_empty() {
                empty_conversation(ui, t, session, st);
            }
            let n = session.ai.messages.len();
            for i in 0..n {
                let Some(m) = session.ai.messages.get(i).cloned() else {
                    continue;
                };
                ui.add_space(10.0);
                if m.role == Role::User {
                    user_bubble(ui, t, &m.text, &m.images);
                } else {
                    answer(ui, t, session, i, &m);
                }
            }
            ui.add_space(12.0);
            if grew {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            }
        });

    if let Some(e) = shown_error {
        let r = Rect::from_min_max(
            pos2(body.left() + 8.0, composer.top() - error_h + 2.0),
            pos2(body.right() - 8.0, composer.top() - 2.0),
        );
        ui.painter()
            .rect_filled(r, 6.0, t.error.gamma_multiply(0.10));
        let f = FontId::proportional(11.0);
        let text = widgets::elide(ui.painter(), &e, &f, r.width() - 16.0);
        ui.painter().text(
            pos2(r.left() + 8.0, r.center().y),
            Align2::LEFT_CENTER,
            text,
            f,
            t.error,
        );
        ui.interact(r, Id::new("ai-error"), Sense::hover())
            .on_hover_text(e);
    }
    composer_box(ui, t, session, st, composer);
}

/// The project's saved conversations: a click opens one, × deletes it.
fn history_button(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut AiPanelState, r: Rect) {
    let resp = ui
        .interact(r, Id::new("ai-history"), Sense::click())
        .on_hover_text(tr("Conversation History"));
    if resp.hovered() {
        ui.painter().rect_filled(r, 5.0, t.row_hover);
    }
    icons::history(ui.painter(), r.shrink(3.5), t.sidebar_icon);
    egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            menu::begin(ui, 280.0);
            menu::heading(ui, t, tr("Conversations in This Project"));
            let history = session.ai.history.clone();
            if history.is_empty() {
                let (row, _) = ui.allocate_exact_size(vec2(280.0, 30.0), Sense::hover());
                ui.painter().text(
                    pos2(row.left() + 12.0, row.center().y),
                    Align2::LEFT_CENTER,
                    tr("No saved conversations yet"),
                    FontId::proportional(12.0),
                    t.text_weak,
                );
                return;
            }
            let mut open = None;
            let mut delete = None;
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .show(ui, |ui| {
                    for c in &history {
                        let w = menu::row_width(ui, 280.0);
                        let (row, resp) = ui.allocate_exact_size(vec2(w, 36.0), Sense::click());
                        let current = session.ai.chat_id.as_deref() == Some(c.id.as_str());
                        let p = ui.painter();
                        if resp.hovered() || current {
                            p.rect_filled(row.shrink2(vec2(4.0, 1.0)), 5.0, t.row_hover);
                        }
                        let x = Rect::from_center_size(
                            pos2(row.right() - 18.0, row.center().y),
                            vec2(18.0, 18.0),
                        );
                        let f = FontId::proportional(12.0);
                        let title = if c.title.is_empty() {
                            tr("Untitled conversation").to_string()
                        } else {
                            c.title.clone()
                        };
                        let shown = widgets::elide(p, &title, &f, row.width() - 52.0);
                        p.text(
                            pos2(row.left() + 12.0, row.top() + 12.0),
                            Align2::LEFT_CENTER,
                            shown,
                            f,
                            t.text,
                        );
                        p.text(
                            pos2(row.left() + 12.0, row.top() + 26.0),
                            Align2::LEFT_CENTER,
                            crate::git_panel::ago(i64::try_from(c.updated).unwrap_or(0)),
                            FontId::proportional(10.0),
                            t.text_weak,
                        );
                        let del = ui
                            .interact(x, Id::new(("ai-history-del", &c.id)), Sense::click())
                            .on_hover_text(tr("Delete Conversation"));
                        if resp.hovered() || del.hovered() {
                            if del.hovered() {
                                ui.painter().rect_filled(x, 4.0, t.row_hover);
                            }
                            icons::close(ui.painter(), x.shrink(4.0), t.text_weak);
                        }
                        if del.clicked() {
                            delete = Some(c.id.clone());
                        } else if resp.clicked() {
                            open = Some(c.id.clone());
                        }
                    }
                });
            if let Some(id) = delete {
                let _ = session.run("ai.deleteChat", &serde_json::json!({ "id": id }));
            }
            if let Some(id) = open {
                if let Err(e) = session.run("ai.openChat", &serde_json::json!({ "id": id })) {
                    st.error = Some(e.to_string());
                }
                ui.close();
            }
        });
}

fn header_bar(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut AiPanelState, r: Rect) {
    let p = ui.painter().clone();
    p.line_segment(
        [r.left_bottom(), r.right_bottom()],
        Stroke::new(1.0, t.divider),
    );
    let logo_r = Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(15.0, 15.0));
    match session
        .ai
        .config
        .selection()
        .and_then(|(p, _)| crate::ai_logos::provider(p))
    {
        Some(key) => crate::ai_logos::paint(ui, key, logo_r, t.sidebar_text),
        None => icons::sparkles(&p, logo_r.shrink(0.5), t.accent, true),
    }
    // Right: history, new conversation, settings.
    let hist_r = Rect::from_center_size(pos2(r.right() - 70.0, r.center().y), vec2(22.0, 22.0));
    history_button(ui, t, session, st, hist_r);
    let new_r = Rect::from_center_size(pos2(r.right() - 44.0, r.center().y), vec2(22.0, 22.0));
    let set_r = Rect::from_center_size(pos2(r.right() - 18.0, r.center().y), vec2(22.0, 22.0));
    for (rect, id, tip) in [
        (new_r, "ai-new", tr("New Conversation")),
        (set_r, "ai-settings", tr("AI Settings…")),
    ] {
        let resp = ui
            .interact(rect, Id::new(id), Sense::click())
            .on_hover_text(tip);
        if resp.hovered() {
            p.rect_filled(rect, 5.0, t.row_hover);
        }
        if resp.clicked() {
            let cmd = if id == "ai-new" {
                "ai.clear"
            } else {
                "ai.settings"
            };
            let _ = session.run(cmd, &serde_json::Value::Null);
            st.error = None;
            st.focus = true;
        }
    }
    icons::plus(&p, new_r.shrink(4.5), t.sidebar_icon);
    icons::gear_solid(&p, set_r.shrink(4.0), t.gear, t.assistant_bg);

    // The model menu: as wide as its label, the chevron right after it (a long name is elided
    // at `MODEL_MENU_MAX`, or short of the history button).
    let label = session
        .ai_model()
        .map_or_else(|| tr("No model").to_string(), |m| m.label().to_string());
    let f = semibold(12.0);
    let label_w = p
        .layout_no_wrap(label.clone(), f.clone(), t.sidebar_text)
        .size()
        .x;
    let room = (hist_r.left() - 6.0 - (r.left() + 28.0)).max(0.0);
    let menu_w = (label_w + 24.0).min(MODEL_MENU_MAX).min(room);
    let menu_r = Rect::from_min_size(
        pos2(r.left() + 28.0, r.center().y - 11.0),
        vec2(menu_w, 22.0),
    );
    let resp = ui
        .interact(menu_r, Id::new("ai-model"), Sense::click())
        .on_hover_text(tr("Choose the model"));
    if resp.hovered() {
        p.rect_filled(menu_r, 5.0, t.row_hover);
    }
    let text = widgets::elide(&p, &label, &f, menu_r.width() - 24.0);
    p.text(
        pos2(menu_r.left() + 5.0, menu_r.center().y),
        Align2::LEFT_CENTER,
        text,
        f,
        t.sidebar_text,
    );
    icons::updown(
        &p,
        pos2(menu_r.right() - 9.0, menu_r.center().y),
        t.text_weak,
    );
    egui::Popup::menu(&resp).show(|ui| {
        menu::begin(ui, 240.0);
        let current = session
            .ai
            .config
            .selection()
            .map(|(p, m)| (p.id.clone(), m.to_string()));
        let providers = session.ai.config.providers.clone();
        let mut first = true;
        // Chat models only (embedding, audio and image models are not for talking to), by
        // their display names, with what they can do under them.
        for p in providers
            .iter()
            .filter(|p| p.enabled && p.models.iter().any(texcraft_ai::Model::chats))
        {
            if !first {
                menu::separator(ui, t);
            }
            first = false;
            menu::heading_with_logo(ui, t, &p.name, crate::ai_logos::provider(p));
            for m in p.models.iter().filter(|m| m.chats()) {
                let checked = current.as_ref() == Some(&(p.id.clone(), m.id.clone()));
                let summary = m.summary();
                let opts = menu::Item {
                    checked: Some(checked),
                    logo: crate::ai_logos::for_entry(p, m),
                    detail: (!summary.is_empty()).then_some(summary.as_str()),
                    ..Default::default()
                };
                if menu::item(ui, t, m.label(), opts).clicked() {
                    let _ = session.run(
                        "ai.selectModel",
                        &serde_json::json!({ "provider": p.id, "model": m.id }),
                    );
                }
            }
        }
        // (The document chip and the gear already include the document and open Settings.)
    });
}

fn empty_conversation(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut AiPanelState) {
    ui.add_space(26.0);
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
        paint_app_icon(ui, r);
        ui.add_space(4.0);
        ui.label(
            RichText::new(tr("Ask about your document"))
                .font(semibold(13.0))
                .color(t.sidebar_text),
        );
    });
    ui.add_space(12.0);
    for s in suggestions() {
        let w = ui.available_width();
        let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
        let card = r.shrink2(vec2(10.0, 2.5));
        ui.painter().rect(
            card,
            7.0,
            if resp.hovered() {
                t.row_hover
            } else {
                Color32::TRANSPARENT
            },
            Stroke::new(0.5, t.badge_border),
            egui::StrokeKind::Inside,
        );
        let f = FontId::proportional(11.5);
        let text = widgets::elide(ui.painter(), s, &f, card.width() - 20.0);
        ui.painter().text(
            pos2(card.left() + 10.0, card.center().y),
            Align2::LEFT_CENTER,
            text,
            f,
            t.sidebar_text,
        );
        if resp.clicked() {
            send(session, st, Some(s));
        }
    }
}

fn user_bubble(ui: &mut Ui, t: &Tokens, text: &str, images: &[texcraft_ai::Image]) {
    let w = ui.available_width();
    // Its images over the bubble, right-aligned, wrapping.
    if !images.is_empty() {
        let side = 56.0;
        let per_row = (((w * 0.86) / (side + 6.0)).floor() as usize).max(1);
        for chunk in images.chunks(per_row) {
            let (row, _) = ui.allocate_exact_size(vec2(w, side + 6.0), Sense::hover());
            let mut x = row.right() - 10.0 - side;
            for img in chunk.iter().rev() {
                let r = Rect::from_min_size(pos2(x, row.top()), vec2(side, side));
                paint_image(ui, t, img, r);
                ui.interact(
                    r,
                    Id::new(("ai-sent-image", img.data.as_ptr())),
                    Sense::hover(),
                )
                .on_hover_text(&img.name);
                x -= side + 6.0;
            }
        }
    }
    let max = (w * 0.86 - 20.0).max(60.0);
    let g = ui
        .painter()
        .layout(text.to_string(), FontId::proportional(12.0), t.text, max);
    let size = g.size() + vec2(20.0, 14.0);
    let (row, _) = ui.allocate_exact_size(vec2(w, size.y), Sense::hover());
    let bubble = Rect::from_min_size(pos2(row.right() - 10.0 - size.x, row.top()), size);
    ui.painter()
        .rect_filled(bubble, 10.0, t.accent.gamma_multiply(0.14));
    ui.painter().galley(bubble.min + vec2(10.0, 7.0), g, t.text);
}

/// A piece of an answer.
enum Segment {
    /// A paragraph, heading or list item (`level` > 0 for headings).
    Text { text: String, level: u8 },
    /// A fenced code block.
    Code { lang: String, body: String },
}

fn segments(text: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut para = String::new();
    let mut code: Option<(String, String)> = None;
    let flush = |para: &mut String, out: &mut Vec<Segment>| {
        if !para.trim().is_empty() {
            out.push(Segment::Text {
                text: para.trim_end().to_string(),
                level: 0,
            });
        }
        para.clear();
    };
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(info) = trimmed.strip_prefix("```") {
            match code.take() {
                Some((lang, body)) => out.push(Segment::Code {
                    lang,
                    body: body.trim_end_matches('\n').to_string(),
                }),
                None => {
                    flush(&mut para, &mut out);
                    code = Some((info.trim().to_string(), String::new()));
                }
            }
            continue;
        }
        if let Some((_, body)) = code.as_mut() {
            body.push_str(line);
            body.push('\n');
            continue;
        }
        if trimmed.is_empty() {
            flush(&mut para, &mut out);
        } else if let Some(h) = trimmed.strip_prefix('#') {
            flush(&mut para, &mut out);
            let level = 1 + h.chars().take_while(|c| *c == '#').count() as u8;
            out.push(Segment::Text {
                text: h.trim_start_matches('#').trim().to_string(),
                level,
            });
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            flush(&mut para, &mut out);
            out.push(Segment::Text {
                text: format!("•  {item}"),
                level: 0,
            });
        } else {
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(trimmed);
        }
    }
    // An unfinished block (the answer is still arriving).
    if let Some((lang, body)) = code {
        out.push(Segment::Code {
            lang,
            body: body.trim_end_matches('\n').to_string(),
        });
    }
    flush(&mut para, &mut out);
    out
}

/// A piece of a paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Span {
    Plain(String),
    Bold(String),
    Code(String),
    /// A Markdown link `[label](target)`.
    Link {
        label: String,
        target: String,
    },
}

/// `text`'s **bold**, `code` and `[label](target)` spans (unclosed markers stay text).
fn spans(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let pos = [rest.find("**"), rest.find('`'), rest.find('[')]
            .into_iter()
            .flatten()
            .min();
        let Some(pos) = pos else {
            out.push(Span::Plain(rest.to_string()));
            break;
        };
        if pos > 0 {
            out.push(Span::Plain(rest.get(..pos).unwrap_or("").to_string()));
        }
        let at = rest.get(pos..).unwrap_or("");
        let (span, used) = if let Some(after) = at.strip_prefix("**") {
            match after.find("**") {
                Some(end) => (
                    Span::Bold(after.get(..end).unwrap_or("").to_string()),
                    end + 4,
                ),
                None => (Span::Plain("**".into()), 2),
            }
        } else if let Some(after) = at.strip_prefix('`') {
            match after.find('`') {
                Some(end) => (
                    Span::Code(after.get(..end).unwrap_or("").to_string()),
                    end + 2,
                ),
                None => (Span::Plain("`".into()), 1),
            }
        } else {
            // `[label](target)`, on one line, both non-empty.
            let link = at.find("](").and_then(|mid| {
                let label = at.get(1..mid)?;
                let after = at.get(mid + 2..)?;
                let end = after.find(')')?;
                let target = after.get(..end)?;
                (!label.is_empty()
                    && !label.contains(['\n', '['])
                    && !target.trim().is_empty()
                    && !target.contains('\n'))
                .then(|| {
                    (
                        Span::Link {
                            label: label.to_string(),
                            target: target.trim().to_string(),
                        },
                        mid + 2 + end + 1,
                    )
                })
            });
            link.unwrap_or_else(|| (Span::Plain("[".into()), 1))
        };
        out.push(span);
        rest = at.get(used..).unwrap_or("");
    }
    out
}

/// A link in a laid-out paragraph: its characters and where it goes.
struct Link {
    chars: std::ops::Range<usize>,
    target: String,
}

/// A link target: its path and line (`sections/intro.tex#L42`, `main.tex:42`, `<a b.tex>`).
fn split_target(target: &str) -> (String, Option<u32>) {
    let t = target
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .replace("%20", " ");
    let t = t.strip_prefix("./").unwrap_or(&t).to_string();
    if let Some((path, frag)) = t.split_once('#') {
        let line = frag
            .trim_start_matches('L')
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|n| n.parse().ok());
        return (path.to_string(), line);
    }
    if let Some((path, n)) = t.rsplit_once(':')
        && let Ok(line) = n.parse()
    {
        return (path.to_string(), Some(line));
    }
    (t, None)
}

/// Whether `code` (an inline code span) looks like a file path worth checking for.
fn path_like(code: &str) -> bool {
    code.len() < 200
        && !code.contains(char::is_whitespace)
        && !code.starts_with('\\')
        && std::path::Path::new(code)
            .extension()
            .is_some_and(|e| e.len() <= 5 && e.to_string_lossy().chars().all(char::is_alphanumeric))
}

/// Opens link `target`: a file of the project in the editor (at its line), or a web page.
fn open_link(ctx: &egui::Context, session: &mut Session, target: &str) {
    if target.starts_with("https://") || target.starts_with("http://") {
        ctx.open_url(egui::OpenUrl::new_tab(target));
        return;
    }
    let (path, line) = split_target(target);
    let Some(root) = session.git_dir().and_then(|r| r.canonicalize().ok()) else {
        return;
    };
    let Some(file) = root
        .join(&path)
        .canonicalize()
        .ok()
        .filter(|f| f.starts_with(&root) && f.is_file())
    else {
        session.set_status(trf!("{file} is not in the project", file = path));
        return;
    };
    let result = match line {
        Some(line) => session.jump(&file, line.max(1)),
        None => session.open_path(&file).map(|_| ()),
    };
    if let Err(e) = result {
        session.set_status(e.to_string());
    }
}

/// `text` with **bold**, `code` and link spans; returns the galley and its links. Code that
/// names a file of the project (`is_file`) links to it too.
fn inline(
    ui: &Ui,
    t: &Tokens,
    text: &str,
    font: FontId,
    width: f32,
    is_file: &dyn Fn(&str) -> bool,
) -> (Arc<Galley>, Vec<Link>) {
    let mut job = LayoutJob::default();
    let size = font.size;
    let plain = TextFormat {
        font_id: font,
        color: t.text,
        line_height: Some((size * 1.45).round()),
        ..Default::default()
    };
    let bold = TextFormat {
        font_id: semibold(size),
        ..plain.clone()
    };
    let code = TextFormat {
        font_id: editor_font(size * 0.92),
        color: t.syn_command,
        background: t.row_hover,
        ..plain.clone()
    };
    let underline = Stroke::new(1.0, t.accent.gamma_multiply(0.6));
    let link = TextFormat {
        color: t.accent,
        underline,
        ..plain.clone()
    };
    let code_link = TextFormat {
        color: t.accent,
        underline,
        ..code.clone()
    };
    let mut links = Vec::new();
    let mut chars = 0usize;
    for span in spans(text) {
        let (s, fmt, target) = match span {
            Span::Plain(s) => (s, &plain, None),
            Span::Bold(s) => (s, &bold, None),
            Span::Code(s) if path_like(&s) && is_file(&s) => {
                let target = s.clone();
                (s, &code_link, Some(target))
            }
            Span::Code(s) => (s, &code, None),
            Span::Link { label, target } => (label, &link, Some(target)),
        };
        let n = s.chars().count();
        if let Some(target) = target {
            links.push(Link {
                chars: chars..chars + n,
                target,
            });
        }
        job.append(&s, 0.0, fmt.clone());
        chars += n;
    }
    job.wrap.max_width = width;
    (ui.fonts_mut(|f| f.layout_job(job)), links)
}

/// A small icon-and-label button; returns whether it was clicked.
fn chip(
    ui: &mut Ui,
    t: &Tokens,
    r: Rect,
    id: Id,
    label: &str,
    icon: fn(&egui::Painter, Rect, Color32),
) -> bool {
    let resp = ui.interact(r, id, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 5.0, t.row_hover);
    }
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(r.left() + 9.0, r.center().y), vec2(11.0, 11.0)),
        t.text_weak,
    );
    ui.painter().text(
        pos2(r.left() + 18.0, r.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(10.5),
        t.text_weak,
    );
    resp.clicked()
}

/// The row of tool use `k` of answer `index`.
fn tool_row_id(index: usize, k: usize) -> Id {
    Id::new(("ai-tool", index, k))
}

/// One file the model looked at (above its answer).
fn tool_row(
    ui: &mut Ui,
    t: &Tokens,
    session: &mut Session,
    id: Id,
    used: &ToolUse,
    x: f32,
    text_w: f32,
) {
    let w = ui.available_width();
    let label = match used.kind {
        ToolKind::Read => match used.lines {
            Some((first, last)) => trf!(
                "Read {file}, lines {first}–{last}",
                file = used.target,
                first = first,
                last = last
            ),
            None => trf!("Read {file}", file = used.target),
        },
        ToolKind::List if used.target.is_empty() => tr("Listed the project's files").into(),
        ToolKind::List => trf!("Listed {folder}", folder = used.target),
        ToolKind::Search => trf!("Searched for “{text}”", text = used.target),
    };
    let opens = used.kind == ToolKind::Read && !used.failed;
    let sense = if opens {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
    let resp = ui.interact(r, id, sense);
    let color = if used.failed {
        t.error
    } else if opens && resp.hovered() {
        t.accent
    } else {
        t.text_weak
    };
    let icon = Rect::from_center_size(pos2(r.left() + x + 5.5, r.center().y), vec2(11.0, 11.0));
    let p = ui.painter();
    match used.kind {
        ToolKind::Read => icons::document_pt(p, icon, color, ""),
        ToolKind::List => icons::folder_pt(p, icon, color),
        ToolKind::Search => icons::magnifier(p, icon, color, None),
    }
    let f = FontId::proportional(10.5);
    let text = widgets::elide(p, &label, &f, (text_w - 16.0).max(0.0));
    p.text(
        pos2(icon.right() + 5.0, r.center().y),
        Align2::LEFT_CENTER,
        text,
        f,
        color,
    );
    if opens {
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            let target = match used.lines {
                Some((first, _)) => format!("{}#L{first}", used.target),
                None => used.target.clone(),
            };
            open_link(ui.ctx(), session, &target);
        }
    }
}

fn answer(
    ui: &mut Ui,
    t: &Tokens,
    session: &mut Session,
    index: usize,
    m: &texcraft_engine::ai::ChatMessage,
) {
    let w = ui.available_width();
    let x = 12.0;
    let text_w = (w - 2.0 * x).max(40.0);
    if let Some(note) = &m.note {
        let (r, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
        let f = FontId::proportional(10.5);
        let text = widgets::elide(ui.painter(), note, &f, text_w);
        ui.painter().text(
            pos2(r.left() + x, r.center().y),
            Align2::LEFT_CENTER,
            text,
            f,
            t.text_weak,
        );
    }
    // The project files the model looked at: a click opens a file that was read.
    for (k, used) in m.tools.iter().enumerate() {
        tool_row(ui, t, session, tool_row_id(index, k), used, x, text_w);
    }
    let root = session.git_dir();
    let is_file = |s: &str| root.as_ref().is_some_and(|r| r.join(s).is_file());
    for (k, seg) in segments(&m.text).into_iter().enumerate() {
        match seg {
            Segment::Text { text, level } => {
                let font = if level > 0 {
                    semibold(if level == 1 { 13.5 } else { 12.5 })
                } else {
                    FontId::proportional(12.0)
                };
                let (g, links) = inline(ui, t, &text, font, text_w, &is_file);
                let sense = if links.is_empty() {
                    Sense::hover()
                } else {
                    Sense::click()
                };
                let (r, resp) = ui.allocate_exact_size(vec2(w, g.size().y + 6.0), sense);
                let at = pos2(r.left() + x, r.top() + 3.0);
                // The link under the pointer: a hand, and a click opens it.
                let under = resp.hover_pos().and_then(|p| {
                    let i = g.cursor_from_pos(p - at).index.0;
                    links
                        .iter()
                        .find(|l| l.chars.start <= i && i < l.chars.end.max(l.chars.start + 1))
                });
                if let Some(l) = under {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    if resp.clicked() {
                        open_link(ui.ctx(), session, &l.target);
                    }
                }
                ui.painter().galley(at, g, t.text);
            }
            Segment::Code { lang, body } => {
                // Code keeps its lines: long ones scroll sideways inside the block.
                let g = ui
                    .painter()
                    .layout_no_wrap(body.clone(), editor_font(11.5), t.text);
                // A sideways scroll bar gets room of its own under the last line: it floats over
                // the content, so the content is that much taller.
                let bar_room = if g.size().x > text_w - 16.0 {
                    let sc = &ui.spacing().scroll;
                    sc.bar_width + sc.bar_outer_margin + 2.0
                } else {
                    0.0
                };
                let h = g.size().y + 16.0 + 24.0 + bar_room;
                let (row, _) = ui.allocate_exact_size(vec2(w, h + 8.0), Sense::hover());
                let card =
                    Rect::from_min_size(pos2(row.left() + x, row.top() + 4.0), vec2(text_w, h));
                let p = ui.painter().clone();
                p.rect(
                    card,
                    7.0,
                    // The sidebar's own box colours (the editor's paper looked out of place).
                    t.composer_bg,
                    Stroke::new(0.5, t.composer_border),
                    egui::StrokeKind::Inside,
                );
                let bar = Rect::from_min_size(card.min, vec2(card.width(), 24.0));
                p.line_segment(
                    [bar.left_bottom(), bar.right_bottom()],
                    Stroke::new(0.5, t.composer_border),
                );
                p.text(
                    pos2(bar.left() + 9.0, bar.center().y),
                    Align2::LEFT_CENTER,
                    if lang.is_empty() {
                        tr("code")
                    } else {
                        lang.as_str()
                    },
                    FontId::proportional(10.5),
                    t.text_weak,
                );
                let copy_r = Rect::from_min_max(
                    pos2(bar.right() - 52.0, bar.top() + 2.0),
                    pos2(bar.right() - 4.0, bar.bottom() - 2.0),
                );
                if chip(
                    ui,
                    t,
                    copy_r,
                    Id::new(("ai-copy", index, k)),
                    tr("Copy"),
                    icons::copy,
                ) {
                    ui.ctx().copy_text(body.clone());
                }
                let body_rect = Rect::from_min_max(
                    pos2(card.left() + 8.0, bar.bottom() + 8.0),
                    pos2(card.right() - 8.0, card.bottom() - 2.0),
                );
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(body_rect));
                c.set_clip_rect(body_rect.intersect(ui.clip_rect()));
                egui::ScrollArea::horizontal()
                    .id_salt(("ai-code", index, k))
                    .show(&mut c, |ui| {
                        let (r, _) =
                            ui.allocate_exact_size(g.size() + vec2(0.0, bar_room), Sense::hover());
                        ui.painter().galley(r.min, g, t.text);
                    });
            }
        }
    }
    if m.pending {
        let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
        icons::spinner(
            ui.painter(),
            Rect::from_center_size(pos2(r.left() + x + 6.0, r.center().y), vec2(11.0, 11.0)),
            t.text_weak,
            ui.input(|i| i.time) as f32 * 6.0,
        );
        if m.thinking || m.text.is_empty() {
            ui.painter().text(
                pos2(r.left() + x + 17.0, r.center().y),
                Align2::LEFT_CENTER,
                tr("Thinking…"),
                FontId::proportional(11.0),
                t.text_weak,
            );
        }
        ui.ctx().request_repaint();
        return;
    }
    if let Some(e) = &m.error {
        let g = ui.painter().layout(
            e.clone(),
            FontId::proportional(11.5),
            t.error,
            text_w - 16.0,
        );
        let (row, _) = ui.allocate_exact_size(vec2(w, g.size().y + 14.0), Sense::hover());
        let card = Rect::from_min_size(
            pos2(row.left() + x, row.top()),
            vec2(text_w, g.size().y + 12.0),
        );
        ui.painter()
            .rect_filled(card, 6.0, t.error.gamma_multiply(0.10));
        ui.painter().galley(card.min + vec2(8.0, 6.0), g, t.error);
        return;
    }
    // Actions under a finished answer.
    let (r, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    let cx = r.left() + x - 4.0;
    let b = Rect::from_min_size(pos2(cx, r.top() + 1.0), vec2(50.0, 20.0));
    if chip(
        ui,
        t,
        b,
        Id::new(("ai-copy-msg", index)),
        tr("Copy"),
        icons::copy,
    ) {
        let _ = session.run("ai.copy", &serde_json::json!({ "index": index }));
    }
    if let Some(model) = &m.model {
        // The model that answered (by its display name), with its logo.
        let f = FontId::proportional(10.0);
        let provider = m
            .provider
            .as_ref()
            .and_then(|id| session.ai.config.providers.iter().find(|p| &p.id == id));
        let logo = match provider {
            Some(p) => crate::ai_logos::for_model(p, model),
            None => crate::ai_logos::model(model),
        };
        let name = provider
            .and_then(|p| p.models.iter().find(|x| &x.id == model))
            .map_or(model.as_str(), texcraft_ai::Model::label);
        let room =
            (r.right() - b.right() - 16.0 - if logo.is_some() { 16.0 } else { 0.0 }).max(0.0);
        let text = widgets::elide(ui.painter(), name, &f, room);
        let g = ui.painter().layout_no_wrap(text, f, t.menu_disabled);
        let tx = r.right() - x - g.size().x;
        if let Some(key) = logo {
            crate::ai_logos::paint(
                ui,
                key,
                Rect::from_center_size(pos2(tx - 9.0, r.center().y), vec2(11.0, 11.0)),
                t.text_weak,
            );
        }
        ui.painter().galley(
            pos2(tx, r.center().y - g.size().y / 2.0),
            g,
            t.menu_disabled,
        );
    }
}

fn composer_box(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut AiPanelState, area: Rect) {
    let p = ui.painter().clone();
    // A box of the sidebar's own family (a shade lighter, a hairline, no shadow), not a white
    // card on it.
    let field = Rect::from_min_max(
        pos2(area.left() + 8.0, area.top() + 6.0),
        pos2(area.right() - 8.0, area.bottom() - 10.0),
    );
    let id = composer_id();
    let focused = ui.memory(|m| m.has_focus(id));
    p.rect(
        field,
        12.0,
        t.composer_bg,
        Stroke::new(
            1.0,
            if focused {
                t.accent.gamma_multiply(0.55)
            } else {
                t.composer_border
            },
        ),
        egui::StrokeKind::Inside,
    );
    // Return sends, ⇧Return starts a new line.
    let mut send_now = false;
    if focused && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        send_now = true;
    }
    if focused && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
        send_now = true;
    }
    // Attached images: thumbnails over the question, each removable (× on hover).
    let mut text_top = field.top() + 9.0;
    let n_images = session.ai.draft_images.len();
    if n_images > 0 {
        let side = 40.0;
        let fit = (((field.width() - 20.0) / (side + 6.0)).floor() as usize).max(1);
        let shown = if n_images > fit { fit - 1 } else { n_images };
        let mut x = field.left() + 10.0;
        let mut remove = None;
        for (k, img) in session.ai.draft_images.iter().take(shown).enumerate() {
            let r = Rect::from_min_size(pos2(x, field.top() + 8.0), vec2(side, side));
            paint_image(ui, t, img, r);
            let xr = Rect::from_center_size(r.right_top() + vec2(-6.0, 6.0), vec2(15.0, 15.0));
            let resp = ui
                .interact(r, Id::new(("ai-draft-image", k)), Sense::hover())
                .on_hover_text(&img.name);
            let xresp = ui
                .interact(xr, Id::new(("ai-draft-x", k)), Sense::click())
                .on_hover_text(tr("Remove"));
            if resp.hovered() || xresp.hovered() {
                p.circle_filled(xr.center(), 7.5, Color32::from_black_alpha(170));
                icons::close(&p, xr.shrink(4.0), Color32::WHITE);
            }
            if xresp.clicked() {
                remove = Some(k);
            }
            x += side + 6.0;
        }
        if shown < n_images {
            let r = Rect::from_min_size(pos2(x, field.top() + 8.0), vec2(side, side));
            p.rect_filled(r, 6.0, t.row_hover);
            p.text(
                r.center(),
                Align2::CENTER_CENTER,
                format!("+{}", n_images - shown),
                FontId::proportional(12.0),
                t.text_weak,
            );
        }
        if let Some(k) = remove {
            let _ = session.run("ai.removeImage", &serde_json::json!({ "index": k }));
        }
        text_top += THUMBS_H;
    }
    let edit_rect = Rect::from_min_max(
        pos2(field.left() + 12.0, text_top),
        pos2(field.right() - 12.0, field.bottom() - 30.0),
    );
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
    egui::ScrollArea::vertical()
        .id_salt("ai-question-scroll")
        .max_height(edit_rect.height())
        .show(&mut c, |ui| {
            let resp = ui.add(
                egui::TextEdit::multiline(&mut session.ai.draft)
                    .id(id)
                    .hint_text(tr("Ask about your LaTeX…"))
                    .font(FontId::proportional(12.0))
                    .frame(egui::Frame::NONE)
                    .margin(vec2(0.0, 0.0))
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            if std::mem::take(&mut st.focus) {
                resp.request_focus();
            }
        });
    // Bottom row: include the document; what the conversation used and the context meter
    // (Argus's row), the meter by the send button to the last; send / stop.
    let row_y = field.bottom() - 16.0;
    let busy = session.ai.is_busy();
    let b = Rect::from_center_size(pos2(field.right() - 18.0, row_y), vec2(24.0, 24.0));
    let context = session.ai_context();
    let ring_r = {
        let w = context.as_ref().map_or(0.0, |c| meter_width(&p, c));
        Rect::from_min_max(
            pos2(b.left() - 4.0 - w, row_y - 11.0),
            pos2(b.left() - 4.0, row_y + 11.0),
        )
    };
    let include = session.ai.config.include_document;
    let doc_name = session
        .doc()
        .map(texcraft_engine::Document::display_name)
        .unwrap_or_default();
    let chip_text = if include {
        doc_name
    } else {
        tr("No document").into()
    };
    let f = FontId::proportional(10.5);
    let tw = p
        .layout_no_wrap(chip_text.clone(), f.clone(), t.text_weak)
        .size()
        .x;
    let chip_r = Rect::from_min_size(
        pos2(field.left() + 9.0, row_y - 9.0),
        vec2(
            (tw + 26.0)
                .min(CHIP_MAX)
                .min(ring_r.left() - 8.0 - (field.left() + 9.0))
                .max(26.0),
            18.0,
        ),
    );
    let resp = ui
        .interact(chip_r, Id::new("ai-doc-chip"), Sense::click())
        .on_hover_text(if include {
            tr("The open document goes along with each question. Click to leave it out.")
        } else {
            tr("Questions go without the document. Click to include it.")
        });
    p.rect(
        chip_r,
        5.0,
        if resp.hovered() {
            t.row_hover
        } else {
            Color32::TRANSPARENT
        },
        Stroke::new(0.5, t.badge_border),
        egui::StrokeKind::Inside,
    );
    icons::document_pt(
        &p,
        Rect::from_min_size(
            pos2(chip_r.left() + 6.0, chip_r.center().y - 5.0),
            vec2(8.0, 10.0),
        ),
        if include { t.accent } else { t.text_weak },
        "",
    );
    let label = widgets::elide(&p, &chip_text, &f, chip_r.width() - 22.0);
    p.text(
        pos2(chip_r.left() + 18.0, chip_r.center().y),
        Align2::LEFT_CENTER,
        label,
        f,
        if include { t.text } else { t.text_weak },
    );
    if resp.clicked() {
        let _ = session.run("ai.toggleDocument", &serde_json::Value::Null);
    }
    let stats = session.ai_stats();
    usage_row(
        ui,
        t,
        &stats,
        Rect::from_min_max(
            pos2(chip_r.right() + 10.0, row_y - 9.0),
            pos2(ring_r.left() - 4.0, row_y + 9.0),
        ),
    );
    if let Some(c) = &context {
        context_meter(ui, t, st, c, ring_r);
    }
    let can_send = !session.ai.draft.trim().is_empty() || !session.ai.draft_images.is_empty();
    let bresp = ui
        .interact(b, Id::new("ai-send"), Sense::click())
        .on_hover_text(if busy { tr("Stop") } else { tr("Send (↩)") });
    if busy {
        icons::stop(&p, b, t.text, t.filter_bg);
        if bresp.clicked() {
            let _ = session.run("ai.stop", &serde_json::Value::Null);
        }
    } else {
        icons::send(
            &p,
            b,
            if can_send { t.accent } else { t.menu_disabled },
            Color32::WHITE,
        );
        if bresp.clicked() && can_send {
            send_now = true;
        }
    }
    if send_now && !busy && can_send {
        send(session, st, None);
        st.focus = true;
    }
    // Files dragged over the window: where they go. (Images come by ⌘V or a drop.)
    let reads = session.ai_reads_images();
    let hovering = ui.input(|i| {
        i.raw
            .hovered_files
            .iter()
            .any(|f| image_file(f.path.as_deref(), &f.mime))
    });
    if hovering {
        p.rect(
            field,
            12.0,
            t.composer_bg,
            Stroke::new(1.5, if reads { t.accent } else { t.error }),
            egui::StrokeKind::Inside,
        );
        p.text(
            field.center(),
            Align2::CENTER_CENTER,
            if reads {
                tr("Drop images to attach")
            } else {
                tr("This model does not read images")
            },
            semibold(12.0),
            if reads { t.accent } else { t.error },
        );
    }
}

/// `14200000` → "14.2M", `297300` → "297.3k", `824` → "824" (Argus's compact token count).
fn compact_tokens(n: u64) -> String {
    let one = |v: f64| {
        let t = format!("{v:.1}");
        t.strip_suffix(".0").map_or(t.clone(), str::to_string)
    };
    if n >= 1_000_000 {
        format!("{}M", one(n as f64 / 1_000_000.0))
    } else if n >= 1_000 {
        format!("{}k", one(n as f64 / 1_000.0))
    } else {
        n.to_string()
    }
}

/// `326509` → "326,509".
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A cost as Argus writes it: two decimals from 1 up, four below, trailing zeros dropped.
fn cost_text(usd: f64) -> String {
    if usd <= 0.0 {
        return "0.00".into();
    }
    if usd < 0.0001 {
        return "<0.0001".into();
    }
    let t = if usd < 1.0 {
        format!("{usd:.4}")
    } else {
        format!("{usd:.2}")
    };
    let t = t.trim_end_matches('0');
    t.strip_suffix('.').unwrap_or(t).to_string()
}

/// One figure of the usage row.
struct Stat {
    /// Its order of leaving when the row is short of room (Argus's: speed, cache, turns and
    /// steps, tokens; then the cost, so a narrow panel keeps only the context meter).
    rank: u8,
    /// Room before it when a figure precedes it (pt).
    gap: f32,
    icon: Option<fn(&egui::Painter, Rect, Color32)>,
    text: String,
    hint: String,
}

/// The figures of `s` that are known, in their order.
fn stats_items(s: &texcraft_engine::ai_usage::ChatStats) -> Vec<Stat> {
    let mut items = Vec::new();
    if s.turns > 0 || s.steps > 0 {
        let turns = if s.turns == 1 {
            trf!("{n} turn", n = s.turns)
        } else {
            trf!("{n} turns", n = s.turns)
        };
        let text = match s.steps {
            0 => turns,
            1 => format!("{turns} {}", trf!("{n} step", n = s.steps)),
            n => format!("{turns} {}", trf!("{n} steps", n = n)),
        };
        items.push(Stat {
            rank: 2,
            gap: 0.0,
            icon: Some(icons::gauge),
            text,
            hint: trf!(
                "Turns: questions asked. Steps: requests to the model, including the rounds after it read project files. Tools called: {tools}.",
                tools = s.tool_calls
            ),
        });
    }
    if let Some(v) = s.speed {
        items.push(Stat {
            rank: 0,
            gap: 8.0,
            icon: None,
            text: format!("~{v} tok/s"),
            hint: tr(
                "Average output speed of the latest answer, including waiting and reasoning, excluding the time tools took.",
            )
            .into(),
        });
    }
    if let Some(n) = s.tokens {
        items.push(Stat {
            rank: 3,
            gap: 10.0,
            icon: Some(icons::database),
            text: format!("{} tok", compact_tokens(n)),
            hint: trf!(
                "What this conversation's requests used: {input} input and {output} output tokens.",
                input = grouped(s.input),
                output = grouped(s.output)
            ),
        });
    }
    if let Some(p) = s.cache_percent {
        items.push(Stat {
            rank: 1,
            gap: 8.0,
            icon: None,
            text: trf!("Cache hit {n}%", n = p),
            hint: tr(
                "Cached input tokens out of all input tokens, weighted by usage. Shown only when every request reported its cache hits.",
            )
            .into(),
        });
    }
    if let Some(c) = s.cost_usd {
        items.push(Stat {
            rank: 4,
            gap: 8.0,
            icon: None,
            text: format!("${}", cost_text(c)),
            hint: trf!(
                "What the provider billed for this conversation: ${amount}.",
                amount = format!("{c:.6}")
            ),
        });
    }
    items
}

/// Which of `items` fit in `room` (pt), dropping them by rank; and the width they take.
fn fit_stats(p: &egui::Painter, items: &[Stat], room: f32) -> (Vec<bool>, f32) {
    let font = FontId::proportional(STAT_SIZE);
    let widths: Vec<f32> = items
        .iter()
        .map(|i| {
            let text = p
                .layout_no_wrap(i.text.clone(), font.clone(), Color32::WHITE)
                .size()
                .x;
            text + if i.icon.is_some() {
                STAT_SIZE + 5.0
            } else {
                0.0
            }
        })
        .collect();
    let total = |shown: &[bool]| {
        let mut w = 0.0;
        let mut first = true;
        for ((item, width), on) in items.iter().zip(&widths).zip(shown) {
            if *on {
                w += width + if first { 0.0 } else { item.gap };
                first = false;
            }
        }
        w
    };
    let mut shown = vec![true; items.len()];
    for rank in 0..=4 {
        if total(&shown) <= room {
            break;
        }
        for (item, on) in items.iter().zip(shown.iter_mut()) {
            if item.rank == rank {
                *on = false;
            }
        }
    }
    let w = total(&shown);
    if w > room {
        return (vec![false; items.len()], 0.0);
    }
    (shown, w)
}

/// What the conversation used (Argus's run statistics), right-aligned in `area`: the figures
/// that do not fit leave, never overlapping the chip or the meter.
fn usage_row(ui: &Ui, t: &Tokens, s: &texcraft_engine::ai_usage::ChatStats, area: Rect) {
    let items = stats_items(s);
    if items.is_empty() || area.width() <= 0.0 {
        return;
    }
    let p = ui.painter();
    let (shown, width) = fit_stats(p, &items, area.width());
    let font = FontId::proportional(STAT_SIZE);
    let mut x = area.right() - width;
    let mut first = true;
    for (k, (item, on)) in items.iter().zip(&shown).enumerate() {
        if !*on {
            continue;
        }
        if !first {
            x += item.gap;
        }
        first = false;
        let start = x;
        if let Some(icon) = item.icon {
            let r = Rect::from_min_size(
                pos2(x, area.center().y - STAT_SIZE / 2.0 - 0.5),
                vec2(STAT_SIZE + 1.0, STAT_SIZE + 1.0),
            );
            icon(p, r, t.text_weak);
            x += STAT_SIZE + 5.0;
        }
        let g = p.layout_no_wrap(item.text.clone(), font.clone(), t.text_weak);
        let w = g.size().x;
        p.galley(pos2(x, area.center().y - g.size().y / 2.0), g, t.text_weak);
        x += w;
        let r = Rect::from_min_max(pos2(start, area.top()), pos2(x, area.bottom()));
        ui.interact(r, Id::new(("ai-usage", k)), Sense::hover())
            .on_hover_text(&item.hint);
    }
}

/// The context meter's text ("28%").
fn meter_text(c: &ContextUsage) -> String {
    format!("{}%", c.percent)
}

/// How wide the context meter is: ring, gap, percentage and its padding.
fn meter_width(p: &egui::Painter, c: &ContextUsage) -> f32 {
    let w = p
        .layout_no_wrap(
            meter_text(c),
            FontId::proportional(STAT_SIZE),
            Color32::WHITE,
        )
        .size()
        .x;
    6.0 + RING + 5.0 + w + 6.0
}

/// A ring filled clockwise from the top to `fraction`.
fn paint_ring(p: &egui::Painter, center: egui::Pos2, track: Color32, fill: Color32, fraction: f32) {
    let r = (RING - RING_LINE) / 2.0;
    p.circle_stroke(center, r, Stroke::new(RING_LINE, track));
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction <= 0.0 {
        return;
    }
    let n = ((48.0 * fraction).ceil() as usize).max(2);
    let start = -std::f32::consts::FRAC_PI_2;
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let a = start + std::f32::consts::TAU * fraction * i as f32 / n as f32;
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(RING_LINE, fill)));
}

/// The context meter (a ring and the share of the window in use) in `r`; hovering it opens the
/// breakdown above it, as in Argus.
fn context_meter(ui: &Ui, t: &Tokens, st: &mut AiPanelState, c: &ContextUsage, r: Rect) {
    let resp = ui.interact(r, Id::new("ai-context"), Sense::hover());
    let now = ui.input(|i| i.time);
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let on_pop = pointer.is_some_and(|q| st.context_pop.is_some_and(|pop| pop.contains(q)));
    let over = resp.hovered() || on_pop;
    if !over {
        st.context_dismissed = false;
    }
    if over && !st.context_dismissed {
        st.context_open = true;
        st.context_close_at = None;
    } else if st.context_open {
        let at = *st.context_close_at.get_or_insert(now + POP_GRACE);
        if now >= at {
            st.context_open = false;
            st.context_close_at = None;
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(at - now));
        }
    }
    // Esc, or a press anywhere else, closes it.
    let pressed_outside = ui.input(|i| i.pointer.any_pressed()) && !over;
    if st.context_open && (ui.input(|i| i.key_pressed(Key::Escape)) || pressed_outside) {
        st.context_open = false;
        st.context_close_at = None;
        st.context_dismissed = over;
    }

    let warning = c.percent >= 80;
    let p = ui.painter();
    let color = if warning {
        t.warning
    } else if st.context_open {
        t.text
    } else {
        t.text_weak
    };
    if st.context_open {
        p.rect_filled(r, 6.0, t.row_hover);
    }
    let center = pos2(r.left() + 6.0 + RING / 2.0, r.center().y);
    paint_ring(
        p,
        center,
        t.composer_border,
        color,
        c.percent as f32 / 100.0,
    );
    p.text(
        pos2(r.left() + 6.0 + RING + 5.0, r.center().y),
        Align2::LEFT_CENTER,
        meter_text(c),
        FontId::proportional(STAT_SIZE),
        color,
    );
    st.context_pop = if st.context_open {
        Some(context_popover(ui, t, c, r))
    } else {
        None
    };
}

/// The context breakdown over the meter `anchor` (right edges aligned, kept on screen); returns
/// where it went.
fn context_popover(ui: &Ui, t: &Tokens, c: &ContextUsage, anchor: Rect) -> Rect {
    let screen = ui.ctx().content_rect();
    let w = POP_W.min(screen.width() - 24.0).max(120.0);
    let line = 18.0;
    let row = 24.0;
    let h = POP_PAD * 2.0 + line + 5.0 + line + 10.0 + 4.0 + 10.0 + 4.0 * row;
    let left = (anchor.right() - w)
        .min(screen.right() - w - 12.0)
        .max(12.0);
    let top = (anchor.top() - h - 8.0).max(12.0);
    let rect = Rect::from_min_size(pos2(left, top), vec2(w, h));
    egui::Area::new(Id::new("ai-context-popover"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .fade_in(false)
        .interactable(true)
        .show(ui.ctx(), |ui| {
            let (r, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
            let p = ui.painter();
            let frame = menu::frame(t);
            p.add(frame.shadow.as_shape(r, CornerRadius::same(12)));
            p.rect(
                r,
                12.0,
                t.popup_bg,
                Stroke::new(0.5, t.menu_border),
                egui::StrokeKind::Inside,
            );
            let inner = r.shrink(POP_PAD);
            let font = FontId::proportional(12.0);
            let mut y = inner.top();
            // Title and the share used.
            p.text(
                pos2(inner.left(), y + line / 2.0),
                Align2::LEFT_CENTER,
                tr("Context window"),
                semibold(12.0),
                t.text,
            );
            p.text(
                pos2(inner.right(), y + line / 2.0),
                Align2::RIGHT_CENTER,
                trf!("{percent}% used", percent = c.percent),
                font.clone(),
                t.text_weak,
            );
            y += line + 5.0;
            // Tokens in use of the window.
            let total = format!(
                "{}{} / {}{} tokens",
                if c.estimated { "~" } else { "" },
                grouped(c.used),
                if c.window_assumed { "~" } else { "" },
                grouped(c.window)
            );
            let total_r = p.text(
                pos2(inner.left(), y + line / 2.0),
                Align2::LEFT_CENTER,
                total,
                font.clone(),
                t.text,
            );
            let mut notes = Vec::new();
            if c.window_assumed {
                notes.push(trf!(
                    "The model's context length is not known; {n} tokens are assumed. Set it in Settings → AI.",
                    n = grouped(c.window)
                ));
            }
            if c.images {
                notes.push(
                    tr("Images are estimated: what they take depends on how the model reads them.")
                        .to_string(),
                );
            }
            if !notes.is_empty() {
                ui.interact(total_r, Id::new("ai-context-total"), Sense::hover())
                    .on_hover_text(notes.join("\n"));
            }
            y += line + 10.0;
            // The bar: each part's share of the window in use.
            let bar = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 4.0));
            p.rect_filled(bar, 2.0, t.row_hover);
            let parts = [
                (tr("System prompt and document"), c.system, t.context_system),
                (tr("Tool definitions"), c.tools, t.context_tools),
                (tr("Conversation and tool results"), c.messages, t.context_messages),
            ];
            let sum: u64 = parts.iter().map(|(_, n, _)| *n).sum();
            let used_w = bar.width() * c.percent as f32 / 100.0;
            let mut x = bar.left();
            if sum > 0 {
                let painter = p.with_clip_rect(bar);
                for (_, n, color) in &parts {
                    let w = used_w * *n as f32 / sum as f32;
                    painter.rect_filled(
                        Rect::from_min_size(pos2(x, bar.top()), vec2(w, bar.height())),
                        0.0,
                        *color,
                    );
                    x += w;
                }
            }
            y += 4.0 + 10.0;
            // Tokens left, then the parts.
            let value = |n: u64| format!("~{}", grouped(n));
            let mut row_at = |label: &str, n: u64, dot: Option<Color32>| {
                let cy = y + row / 2.0;
                let mut lx = inner.left();
                if let Some(color) = dot {
                    p.rect_filled(
                        Rect::from_center_size(pos2(lx + 3.5, cy), vec2(7.0, 7.0)),
                        2.0,
                        color,
                    );
                    lx += 13.0;
                }
                p.text(
                    pos2(lx, cy),
                    Align2::LEFT_CENTER,
                    label,
                    font.clone(),
                    t.text_weak,
                );
                p.text(
                    pos2(inner.right(), cy),
                    Align2::RIGHT_CENTER,
                    value(n),
                    font.clone(),
                    t.text,
                );
                y += row;
            };
            row_at(tr("Remaining tokens"), c.remaining, None);
            for (label, n, color) in parts {
                row_at(label, n, Some(color));
            }
        });
    rect
}

#[cfg(test)]
mod link_tests {
    use super::*;

    /// Answers' Markdown: bold, code and links; unclosed markers stay text.
    #[test]
    fn spans_find_links_code_and_bold() {
        assert_eq!(
            spans("See [intro.tex:42](sections/intro.tex#L42) and `main.tex`, **now**."),
            vec![
                Span::Plain("See ".into()),
                Span::Link {
                    label: "intro.tex:42".into(),
                    target: "sections/intro.tex#L42".into()
                },
                Span::Plain(" and ".into()),
                Span::Code("main.tex".into()),
                Span::Plain(", ".into()),
                Span::Bold("now".into()),
                Span::Plain(".".into()),
            ]
        );
        assert_eq!(
            spans("a [b] c [d]( ) `e"),
            vec![
                Span::Plain("a ".into()),
                Span::Plain("[".into()),
                Span::Plain("b] c ".into()),
                Span::Plain("[".into()),
                Span::Plain("d]( ) ".into()),
                Span::Plain("`".into()),
                Span::Plain("e".into()),
            ]
        );
        // `\ref{fig:a}`-style code is not a path; file names are.
        assert!(path_like("sections/intro.tex") && path_like("refs.bib"));
        assert!(!path_like("\\ref{x}") && !path_like("a b.tex") && !path_like("equation"));
    }

    #[test]
    fn link_targets_carry_lines() {
        assert_eq!(
            split_target("sections/intro.tex#L42"),
            ("sections/intro.tex".into(), Some(42))
        );
        assert_eq!(
            split_target("./main.tex#L3-L9"),
            ("main.tex".into(), Some(3))
        );
        assert_eq!(split_target("main.tex:7"), ("main.tex".into(), Some(7)));
        assert_eq!(
            split_target("<my%20paper.tex>"),
            ("my paper.tex".into(), None)
        );
    }
}

#[cfg(test)]
mod usage_row_tests {
    use super::*;
    use texcraft_engine::ai::ChatMessage;
    use texcraft_engine::ai_usage::RunUsage;
    use texcraft_i18n::Lang;

    /// A session with a model of a known window, one question answered after two requests, and
    /// every figure of the row known.
    fn session() -> Session {
        let mut s = Session::new(Arc::new(|| {}));
        s.settings.live_typeset = false;
        s.update.url = "/nonexistent/texcraft-latest.json".into();
        let ollama = texcraft_ai::PRESETS
            .iter()
            .find(|p| p.id == "ollama")
            .unwrap();
        let i = s.ai.config.add(ollama);
        s.ai.config.providers[i].models = vec![texcraft_ai::Model {
            id: "m".into(),
            context_length: Some(128_000),
            ..Default::default()
        }];
        s.ai.config.normalize();
        let message = |role, text: &str| ChatMessage {
            role,
            text: text.into(),
            blocks: Vec::new(),
            model: Some("m".into()),
            provider: None,
            pending: false,
            thinking: false,
            error: None,
            note: None,
            images: Vec::new(),
            tools: Vec::new(),
        };
        s.ai.messages = vec![
            message(Role::User, "Why ??"),
            message(Role::Assistant, "Typeset twice."),
        ];
        s.ai.usage.runs = vec![RunUsage {
            message: 1,
            calls: 97,
            tool_calls: 60,
            measured: 97,
            input: 14_100_000,
            output: 120_000,
            cache_hit: 10_434_000,
            cache_reported: true,
            cost_usd: Some(2.065),
            model_ms: 1_000_000,
        }];
        s
    }

    /// Draws the Assistant `width` pt wide (two frames, so nothing is left from a wider one) with
    /// the pointer at `pointer`, at `time` s; returns the document chip, the row's figures (by
    /// their order, `None` when left out), the context meter and the send button.
    #[allow(clippy::type_complexity)]
    fn draw(
        ctx: &egui::Context,
        s: &mut Session,
        st: &mut AiPanelState,
        width: f32,
        pointer: Option<egui::Pos2>,
        time: f64,
        events: Vec<egui::Event>,
    ) -> (Rect, Vec<Option<Rect>>, Option<Rect>, Rect) {
        let t = Tokens::light();
        for k in 0..2 {
            let mut ev: Vec<egui::Event> =
                pointer.map(egui::Event::PointerMoved).into_iter().collect();
            if k == 1 {
                ev.extend(events.clone());
            }
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 640.0))),
                time: Some(time + f64::from(k) * 0.001),
                events: ev,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| show(ui, &t, s, st));
            out.textures_delta.clear();
        }
        let rect = |id: Id| ctx.read_response(id).map(|r| r.rect);
        (
            rect(Id::new("ai-doc-chip")).unwrap(),
            (0..5).map(|k| rect(Id::new(("ai-usage", k)))).collect(),
            rect(Id::new("ai-context")),
            rect(Id::new("ai-send")).unwrap(),
        )
    }

    /// Wide, every figure shows; narrower, they leave in Argus's order (speed, cache, turns and
    /// steps, tokens, then the cost) and never overlap the chip, each other, the meter or the
    /// send button; the context meter stays by the send button to the last. In both languages.
    #[test]
    fn the_row_gives_way_and_keeps_the_context_meter() {
        for lang in [Lang::En, Lang::ZhHans] {
            let _lang = texcraft_i18n::scoped(lang);
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let mut s = session();
            let mut st = AiPanelState::default();
            // Figures by their order: turns/steps, speed, tokens, cache, cost.
            let leaving = [1usize, 3, 0, 2, 4];
            let mut last_shown = 5;
            let mut width = 760.0;
            while width >= 160.0 {
                let (chip, items, meter, send) =
                    draw(&ctx, &mut s, &mut st, width, None, 0.0, Vec::new());
                let Some(meter) = meter else {
                    panic!("{lang:?} {width}: no context meter");
                };
                assert!(
                    meter.right() <= send.left() && meter.left() >= chip.right(),
                    "{lang:?} {width}: {chip:?} {meter:?} {send:?}"
                );
                let shown: Vec<Rect> = items.iter().flatten().copied().collect();
                for r in &shown {
                    assert!(
                        r.left() >= chip.right() && r.right() <= meter.left(),
                        "{lang:?} {width}: {r:?} between {chip:?} and {meter:?}"
                    );
                }
                for pair in shown.windows(2) {
                    assert!(
                        pair[0].right() <= pair[1].left(),
                        "{lang:?} {width}: {pair:?}"
                    );
                }
                // The figures left out are the first to leave.
                let gone = items.iter().filter(|r| r.is_none()).count();
                for (k, r) in items.iter().enumerate() {
                    let rank = leaving.iter().position(|x| *x == k).unwrap();
                    assert_eq!(r.is_none(), rank < gone, "{lang:?} {width}: {items:?}");
                }
                assert!(
                    shown.len() <= last_shown,
                    "{lang:?} {width}: fewer as it narrows"
                );
                last_shown = shown.len();
                if width >= 700.0 {
                    assert_eq!(shown.len(), 5, "{lang:?} {width}: all of them");
                }
                width -= 20.0;
            }
            assert_eq!(last_shown, 0, "{lang:?}: at the narrowest only the meter");
        }
    }

    /// Hovering the meter opens the breakdown above it, right edges aligned; it stays while the
    /// pointer crosses to it, closes 180 ms after the pointer left, and Esc closes it.
    #[test]
    fn the_meter_opens_its_breakdown_on_hover() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut s = session();
        let mut st = AiPanelState::default();
        let (_, _, meter, _) = draw(&ctx, &mut s, &mut st, 420.0, None, 0.0, Vec::new());
        let meter = meter.unwrap();
        assert!(!st.context_open);
        draw(
            &ctx,
            &mut s,
            &mut st,
            420.0,
            Some(meter.center()),
            1.0,
            Vec::new(),
        );
        assert!(st.context_open);
        let pop = st.context_pop.unwrap();
        assert!(
            pop.bottom() <= meter.top() && (pop.right() - meter.right()).abs() < 0.5,
            "{pop:?} {meter:?}"
        );
        assert_eq!(pop.width(), POP_W);
        // Across the gap and onto it: still open.
        let gap = pos2(meter.center().x, meter.top() - 4.0);
        draw(&ctx, &mut s, &mut st, 420.0, Some(gap), 1.1, Vec::new());
        assert!(st.context_open);
        draw(
            &ctx,
            &mut s,
            &mut st,
            420.0,
            Some(pop.center()),
            1.2,
            Vec::new(),
        );
        assert!(st.context_open);
        // Away: open for 180 ms more, then closed.
        let away = pos2(20.0, 100.0);
        draw(&ctx, &mut s, &mut st, 420.0, Some(away), 2.0, Vec::new());
        assert!(st.context_open);
        draw(&ctx, &mut s, &mut st, 420.0, Some(away), 2.3, Vec::new());
        assert!(!st.context_open && st.context_pop.is_none());
        // Esc closes it, and it stays closed while the pointer stays on the meter.
        draw(
            &ctx,
            &mut s,
            &mut st,
            420.0,
            Some(meter.center()),
            3.0,
            Vec::new(),
        );
        assert!(st.context_open);
        let esc = egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        draw(
            &ctx,
            &mut s,
            &mut st,
            420.0,
            Some(meter.center()),
            3.1,
            vec![esc],
        );
        assert!(!st.context_open);
        draw(
            &ctx,
            &mut s,
            &mut st,
            420.0,
            Some(meter.center()),
            3.2,
            Vec::new(),
        );
        assert!(!st.context_open);
    }

    #[test]
    fn numbers_read_like_argus() {
        assert_eq!(compact_tokens(14_200_000), "14.2M");
        assert_eq!(compact_tokens(1_000_000), "1M");
        assert_eq!(compact_tokens(297_300), "297.3k");
        assert_eq!(compact_tokens(824), "824");
        assert_eq!(grouped(326_509), "326,509");
        assert_eq!(grouped(1_048_576), "1,048,576");
        assert_eq!(grouped(999), "999");
        assert_eq!(cost_text(14.87), "14.87");
        assert_eq!(cost_text(0.605), "0.605");
        assert_eq!(cost_text(1.5), "1.5");
        assert_eq!(cost_text(2.0), "2");
        assert_eq!(cost_text(0.00001), "<0.0001");
        assert_eq!(cost_text(0.0), "0.00");
    }
}
