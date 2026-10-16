//! The sidebar's AI Assistant tab: a model menu, the conversation (answers rendered from a
//! small Markdown subset, with LaTeX code blocks that insert into the document in one click),
//! and the question box. Providers and models are set up in Settings → AI.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Color32, FontId, Galley, Id, Key, Modifiers, Rect, RichText, Sense, Stroke, Ui, pos2,
    vec2,
};
use texcraft_ai::Role;
use texcraft_engine::Session;

use crate::menu::{self, ButtonKind};
use crate::theme::{Tokens, editor_font, semibold};
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
}

/// Questions offered on an empty conversation.
const SUGGESTIONS: [&str; 4] = [
    "Find and fix the errors in this document",
    "Explain the selected LaTeX",
    "Write a table for my results",
    "Improve the wording of the selection",
];

fn send(session: &mut Session, st: &mut AiPanelState, text: Option<&str>) {
    let params = match text {
        Some(t) => serde_json::json!({ "text": t }),
        None => serde_json::Value::Null,
    };
    match session.run("ai.send", &params) {
        Ok(()) => st.error = None,
        Err(e) => st.error = Some(e.to_string()),
    }
}

/// Draws the Assistant tab into the sidebar body.
pub fn show(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut AiPanelState) {
    let body = ui.max_rect();
    let configured = session.ai.config.selection().is_some();

    // ---- Header: model menu, new conversation, settings.
    let header = Rect::from_min_size(body.min, vec2(body.width(), 32.0));
    header_bar(ui, t, session, st, header);

    if !configured {
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(
            pos2(body.left() + 14.0, header.bottom()),
            pos2(body.right() - 14.0, body.bottom()),
        )));
        c.add_space(36.0);
        c.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
            icons::sparkles(ui.painter(), r, t.accent, true);
            ui.add_space(8.0);
            ui.label(RichText::new("AI Assistant").font(semibold(13.0)).color(t.sidebar_text));
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Connect a provider — SoCLaaS (NUS), Anthropic, OpenAI, Gemini, OpenRouter, DeepSeek, Kimi, Qwen, MiMo, GLM, MiniMax, StepFun or a local Ollama — to ask about your document, fix errors and write LaTeX.",
                    )
                    .size(11.5)
                    .color(t.text_weak),
                )
                .wrap(),
            );
            ui.add_space(12.0);
            let w = (ui.available_width() - 10.0).clamp(80.0, 200.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
            let mut b = ui.new_child(egui::UiBuilder::new().max_rect(r));
            if menu::dialog_button(&mut b, t, "Set Up a Provider…", ButtonKind::Default, w) {
                let _ = session.run("ai.settings", &serde_json::Value::Null);
            }
        });
        return;
    }

    // ---- Composer at the bottom.
    let composer_h = 98.0;
    let composer = Rect::from_min_max(
        pos2(body.left(), body.bottom() - composer_h),
        pos2(body.right(), body.bottom()),
    );
    let error_h = if st.error.is_some() { 34.0 } else { 0.0 };
    let messages = Rect::from_min_max(
        pos2(body.left(), header.bottom()),
        pos2(body.right(), composer.top() - error_h),
    );

    // ---- Conversation.
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(messages));
    c.set_clip_rect(messages.intersect(ui.clip_rect()));
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
                    user_bubble(ui, t, &m.text);
                } else {
                    answer(ui, t, session, i, &m);
                }
            }
            ui.add_space(12.0);
            if grew {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            }
        });

    if let Some(e) = st.error.clone() {
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
        .and_then(|(p, m)| crate::ai_logos::for_model(p, m))
    {
        Some(key) => crate::ai_logos::paint(ui, key, logo_r, t.sidebar_text),
        None => icons::sparkles(&p, logo_r.shrink(0.5), t.accent, true),
    }
    // Right: new conversation, settings.
    let new_r = Rect::from_center_size(pos2(r.right() - 44.0, r.center().y), vec2(22.0, 22.0));
    let set_r = Rect::from_center_size(pos2(r.right() - 18.0, r.center().y), vec2(22.0, 22.0));
    for (rect, id, tip) in [
        (new_r, "ai-new", "New Conversation"),
        (set_r, "ai-settings", "AI Settings…"),
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
    icons::gear_solid(&p, set_r.shrink(4.0), t.gear, t.sidebar_bg);

    // The model menu.
    let label = session
        .ai
        .config
        .selection()
        .map_or_else(|| "No model".to_string(), |(_, m)| m.to_string());
    let menu_r = Rect::from_min_max(
        pos2(r.left() + 28.0, r.center().y - 11.0),
        pos2(new_r.left() - 6.0, r.center().y + 11.0),
    );
    let resp = ui
        .interact(menu_r, Id::new("ai-model"), Sense::click())
        .on_hover_text("Choose the model");
    if resp.hovered() {
        p.rect_filled(menu_r, 5.0, t.row_hover);
    }
    let f = semibold(12.0);
    let text = widgets::elide(&p, &label, &f, menu_r.width() - 26.0);
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
        for p in providers
            .iter()
            .filter(|p| p.enabled && !p.models.is_empty())
        {
            if !first {
                menu::separator(ui, t);
            }
            first = false;
            menu::heading(ui, t, &p.name);
            for m in &p.models {
                let checked = current.as_ref() == Some(&(p.id.clone(), m.clone()));
                let opts = menu::Item {
                    checked: Some(checked),
                    logo: crate::ai_logos::for_model(p, m),
                    ..Default::default()
                };
                if menu::item(ui, t, m, opts).clicked() {
                    let _ = session.run(
                        "ai.selectModel",
                        &serde_json::json!({ "provider": p.id, "model": m }),
                    );
                }
            }
        }
        menu::separator(ui, t);
        menu::command(
            ui,
            t,
            session,
            "ai.toggleDocument",
            Some("Include the Open Document"),
            true,
        );
        menu::command(ui, t, session, "ai.settings", None, true);
    });
}

fn empty_conversation(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut AiPanelState) {
    ui.add_space(26.0);
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
        icons::sparkles(ui.painter(), r, t.accent, true);
        ui.add_space(8.0);
        ui.label(
            RichText::new("Ask about your document")
                .font(semibold(13.0))
                .color(t.sidebar_text),
        );
    });
    ui.add_space(12.0);
    for s in SUGGESTIONS {
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

fn user_bubble(ui: &mut Ui, t: &Tokens, text: &str) {
    let w = ui.available_width();
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

/// `text` with **bold** and `code` spans.
fn inline(ui: &Ui, t: &Tokens, text: &str, font: FontId, width: f32) -> Arc<Galley> {
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
    let mut rest = text;
    while !rest.is_empty() {
        let b = rest.find("**");
        let c = rest.find('`');
        let (pos, marker) = match (b, c) {
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
                let fmt = if marker == "`" { &code } else { &bold };
                job.append(after.get(..end).unwrap_or(""), 0.0, fmt.clone());
                rest = after.get(end + marker.len()..).unwrap_or("");
            }
            None => {
                job.append(marker, 0.0, plain.clone());
                rest = after;
            }
        }
    }
    job.wrap.max_width = width;
    ui.fonts_mut(|f| f.layout_job(job))
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
    for (k, seg) in segments(&m.text).into_iter().enumerate() {
        match seg {
            Segment::Text { text, level } => {
                let font = if level > 0 {
                    semibold(if level == 1 { 13.5 } else { 12.5 })
                } else {
                    FontId::proportional(12.0)
                };
                let g = inline(ui, t, &text, font, text_w);
                let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y + 6.0), Sense::hover());
                ui.painter()
                    .galley(pos2(r.left() + x, r.top() + 3.0), g, t.text);
            }
            Segment::Code { lang, body } => {
                // Code keeps its lines: long ones scroll sideways inside the block.
                let g = ui
                    .painter()
                    .layout_no_wrap(body.clone(), editor_font(11.5), t.editor_text);
                let h =
                    g.size().y + 16.0 + 24.0 + if g.size().x > text_w - 16.0 { 8.0 } else { 0.0 };
                let (row, _) = ui.allocate_exact_size(vec2(w, h + 8.0), Sense::hover());
                let card =
                    Rect::from_min_size(pos2(row.left() + x, row.top() + 4.0), vec2(text_w, h));
                let p = ui.painter().clone();
                p.rect(
                    card,
                    7.0,
                    t.editor_bg,
                    Stroke::new(0.5, t.form_hairline_strong),
                    egui::StrokeKind::Inside,
                );
                let bar = Rect::from_min_size(card.min, vec2(card.width(), 24.0));
                p.line_segment(
                    [bar.left_bottom(), bar.right_bottom()],
                    Stroke::new(0.5, t.form_hairline_strong),
                );
                p.text(
                    pos2(bar.left() + 9.0, bar.center().y),
                    Align2::LEFT_CENTER,
                    if lang.is_empty() {
                        "code"
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
                let insert_r = Rect::from_min_max(
                    pos2(copy_r.left() - 56.0, bar.top() + 2.0),
                    pos2(copy_r.left() - 2.0, bar.bottom() - 2.0),
                );
                if session.doc().is_some()
                    && chip(
                        ui,
                        t,
                        insert_r,
                        Id::new(("ai-ins", index, k)),
                        "Insert",
                        icons::insert,
                    )
                {
                    let _ = session.run("edit.insertText", &serde_json::json!({ "text": body }));
                }
                if chip(
                    ui,
                    t,
                    copy_r,
                    Id::new(("ai-copy", index, k)),
                    "Copy",
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
                        let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                        ui.painter().galley(r.min, g, t.editor_text);
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
                "Thinking…",
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
    let mut cx = r.left() + x - 4.0;
    if session.doc().is_some() && session.ai_snippet(index).is_some() {
        let b = Rect::from_min_size(pos2(cx, r.top() + 1.0), vec2(54.0, 20.0));
        if chip(
            ui,
            t,
            b,
            Id::new(("ai-insert-msg", index)),
            "Insert",
            icons::insert,
        ) {
            let _ = session.run("ai.insert", &serde_json::json!({ "index": index }));
        }
        cx += 56.0;
    }
    let b = Rect::from_min_size(pos2(cx, r.top() + 1.0), vec2(50.0, 20.0));
    if chip(
        ui,
        t,
        b,
        Id::new(("ai-copy-msg", index)),
        "Copy",
        icons::copy,
    ) {
        let _ = session.run("ai.copy", &serde_json::json!({ "index": index }));
    }
    if let Some(model) = &m.model {
        // The model that answered, with its logo.
        let f = FontId::proportional(10.0);
        let logo = match m
            .provider
            .as_ref()
            .and_then(|id| session.ai.config.providers.iter().find(|p| &p.id == id))
        {
            Some(p) => crate::ai_logos::for_model(p, model),
            None => crate::ai_logos::model(model),
        };
        let room =
            (r.right() - b.right() - 16.0 - if logo.is_some() { 16.0 } else { 0.0 }).max(0.0);
        let text = widgets::elide(ui.painter(), model, &f, room);
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
    // A free-standing card over the conversation (no divider line), as in chat apps.
    let field = Rect::from_min_max(
        pos2(area.left() + 8.0, area.top() + 6.0),
        pos2(area.right() - 8.0, area.bottom() - 10.0),
    );
    let id = Id::new("ai-question");
    let focused = ui.memory(|m| m.has_focus(id));
    p.add(
        egui::epaint::Shadow {
            offset: [0, 2],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(if t.dark { 60 } else { 16 }),
        }
        .as_shape(field, 14.0),
    );
    p.rect(
        field,
        14.0,
        t.form_bg,
        Stroke::new(
            if focused { 1.0 } else { 0.5 },
            if focused {
                t.accent.gamma_multiply(0.6)
            } else {
                t.badge_border
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
    let edit_rect = Rect::from_min_max(
        pos2(field.left() + 12.0, field.top() + 9.0),
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
                    .hint_text("Ask about your LaTeX…")
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
    // Bottom row: include the document, send / stop.
    let row_y = field.bottom() - 16.0;
    let include = session.ai.config.include_document;
    let doc_name = session
        .doc()
        .map(texcraft_engine::Document::display_name)
        .unwrap_or_default();
    let chip_text = if include {
        doc_name
    } else {
        "No document".into()
    };
    let f = FontId::proportional(10.5);
    let tw = p
        .layout_no_wrap(chip_text.clone(), f.clone(), t.text_weak)
        .size()
        .x;
    let chip_r = Rect::from_min_size(
        pos2(field.left() + 9.0, row_y - 9.0),
        vec2((tw + 26.0).min(field.width() - 50.0), 18.0),
    );
    let resp = ui
        .interact(chip_r, Id::new("ai-doc-chip"), Sense::click())
        .on_hover_text(if include {
            "The open document goes along with each question. Click to leave it out."
        } else {
            "Questions go without the document. Click to include it."
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
    let busy = session.ai.is_busy();
    let b = Rect::from_center_size(pos2(field.right() - 18.0, row_y), vec2(24.0, 24.0));
    let can_send = !session.ai.draft.trim().is_empty();
    let bresp = ui
        .interact(b, Id::new("ai-send"), Sense::click())
        .on_hover_text(if busy { "Stop" } else { "Send (↩)" });
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
}
