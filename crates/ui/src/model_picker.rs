//! The model picker (Settings → AI → Add Models…): a window listing every model a provider
//! serves, grouped by what they do (Text, Vision, Other) with a search field and ticks, so any
//! number can be added at once. A model the list does not have (a server that cannot list its
//! models) is typed into the search field and added from there.

use std::collections::BTreeSet;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use egui::{
    Align2, Color32, Context, FontId, Id, Key, Rect, Sense, Stroke, StrokeKind, Ui,
    ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2,
};
use texcraft_ai::{Capability, Model};
use texcraft_engine::Session;

use crate::icons;
use crate::prefs::{Pal, check_box, hairline, push_button, text_field, text_tabs};
use crate::theme::{editor_font, semibold};

/// Window size (points).
const W: f32 = 460.0;
const H: f32 = 580.0;
/// Height of a model row and of a group heading.
const ROW_H: f32 = 26.0;
/// Height of the footer with the buttons.
const FOOTER_H: f32 = 52.0;

/// The picker's native window.
pub fn picker_viewport() -> ViewportId {
    ViewportId::from_hash_of("texcraft-model-picker")
}

/// The picker's groups: chat models with and without image input, then everything else
/// (embeddings, speech, image generation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Text,
    Vision,
    Other,
}

impl Group {
    const ALL: [Group; 3] = [Group::Text, Group::Vision, Group::Other];

    fn of(m: &Model) -> Group {
        if !m.chats() {
            Group::Other
        } else if m.has(Capability::Vision) {
            Group::Vision
        } else {
            Group::Text
        }
    }

    fn label(self) -> &'static str {
        match self {
            Group::Text => "Text",
            Group::Vision => "Vision",
            Group::Other => "Other",
        }
    }
}

/// A model list being fetched in the background.
type ModelList = Receiver<Result<Vec<Model>, String>>;

/// State of the picker window.
#[derive(Default)]
pub struct ModelPicker {
    /// Open, for the provider with this id.
    provider: Option<String>,
    /// The provider's ids are aliases (SoCLaaS): typed ones are not read for a kind.
    aliases: bool,
    /// The model list on its way.
    fetch: Option<ModelList>,
    /// The provider's models (sorted), and ids typed by hand first.
    models: Vec<Model>,
    /// Why there is no list.
    error: Option<String>,
    /// The search field.
    filter: String,
    /// The group tab; `None` shows all, grouped.
    group: Option<Group>,
    /// Models ticked to be added.
    ticked: BTreeSet<String>,
    /// Focus the search field (and the window) on the next frame.
    focus: bool,
    /// Edit events from the menu bar (⌘V and co.) for the search field.
    pub(crate) edit_events: Vec<egui::Event>,
}

impl ModelPicker {
    /// Whether the window is open.
    pub fn is_open(&self) -> bool {
        self.provider.is_some()
    }

    /// Opens the picker for provider `index` and fetches its model list.
    pub fn open(&mut self, ctx: &Context, session: &Session, index: usize) {
        let Some(p) = session.ai.config.providers.get(index) else {
            return;
        };
        *self = Self {
            provider: Some(p.id.clone()),
            aliases: texcraft_ai::is_soclaas(p),
            focus: true,
            ..Self::default()
        };
        // Decrypted only for this request, on its own thread.
        let key = session.ai_key_for(p);
        let provider = p.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new()
            .name("texcraft-ai-models".into())
            .spawn(move || {
                let _ = tx.send(texcraft_ai::fetch_models(&provider, &key).map_err(|e| e.0));
                ctx.request_repaint();
            });
        self.fetch = Some(rx);
    }

    /// Closes the window; nothing is added.
    pub fn close(&mut self) {
        *self = Self::default();
    }

    /// What the window shows, for `state.inspect`.
    pub fn info(&self) -> serde_json::Value {
        let count = |g: Group| self.models.iter().filter(|m| Group::of(m) == g).count();
        serde_json::json!({
            "open": self.is_open(),
            "provider": self.provider,
            "fetching": self.fetch.is_some(),
            "models": self.models.len(),
            "text": count(Group::Text),
            "vision": count(Group::Vision),
            "other": count(Group::Other),
            "group": self.group.map(Group::label),
            "shown": matching(&self.models, &self.filter, self.group).len(),
            "ticked": self.ticked.len(),
            "filter": self.filter,
            "error": self.error,
        })
    }

    /// Shows only group `g` (`None`: all of them).
    pub fn set_group(&mut self, g: Option<&str>) {
        self.group = Group::ALL
            .into_iter()
            .find(|x| g.is_some_and(|g| x.label().eq_ignore_ascii_case(g)));
    }

    fn poll(&mut self, ctx: &Context) {
        let Some(rx) = &self.fetch else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(models)) => {
                self.fetch = None;
                self.set_models(models);
            }
            Ok(Err(e)) => {
                self.fetch = None;
                self.error = Some(e);
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(TryRecvError::Disconnected) => {
                self.fetch = None;
                self.error = Some("The model list could not be fetched.".into());
            }
        }
    }

    /// Takes a fetched list: sorted, without duplicates, after any ids typed meanwhile.
    fn set_models(&mut self, fetched: Vec<Model>) {
        let mut list: Vec<Model> = Vec::new();
        for mut m in fetched {
            m.normalize();
            if !m.id.is_empty() && !list.iter().any(|x| x.id == m.id) {
                list.push(m);
            }
        }
        list.sort_by_cached_key(|m| m.id.to_lowercase());
        let typed: Vec<Model> = std::mem::take(&mut self.models)
            .into_iter()
            .filter(|m| !list.iter().any(|x| x.id == m.id))
            .collect();
        self.models = typed.into_iter().chain(list).collect();
        if self.models.is_empty() {
            self.error = Some("The provider listed no models.".into());
        }
    }

    /// Adds `id` (typed by hand) to the list, ticked.
    fn add_typed(&mut self, id: &str) {
        if !self.models.iter().any(|m| m.id == id) {
            // SoCLaaS ids are aliases: nothing is read from them.
            let m = if self.aliases {
                Model {
                    id: id.to_string(),
                    ..Model::default()
                }
            } else {
                Model::from_id(id)
            };
            self.models.insert(0, m);
        }
        self.ticked.insert(id.to_string());
    }
}

/// Indices of the models in `group` (all when `None`) matching `filter`: every word of it,
/// in the id or the display name, ignoring case.
fn matching(models: &[Model], filter: &str, group: Option<Group>) -> Vec<usize> {
    let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    models
        .iter()
        .enumerate()
        .filter(|(_, m)| group.is_none_or(|g| Group::of(m) == g))
        .filter(|(_, m)| {
            let text = format!("{} {}", m.id, m.name).to_lowercase();
            words.iter().all(|w| text.contains(w.as_str()))
        })
        .map(|(i, _)| i)
        .collect()
}

/// A line of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// A group's heading and how many of its models are shown.
    Heading(Group, usize),
    /// A model (index into the list).
    Model(usize),
    /// Add the id typed in the search field.
    Typed,
}

/// The list's lines: with no group tab, each group under its heading.
fn lines(models: &[Model], shown: &[usize], group: Option<Group>, typed: bool) -> Vec<Line> {
    let mut out = Vec::new();
    for g in Group::ALL {
        let in_group: Vec<usize> = shown
            .iter()
            .copied()
            .filter(|&i| models.get(i).is_some_and(|m| Group::of(m) == g))
            .collect();
        if in_group.is_empty() {
            continue;
        }
        if group.is_none() {
            out.push(Line::Heading(g, in_group.len()));
        }
        out.extend(in_group.into_iter().map(Line::Model));
    }
    if typed {
        out.push(Line::Typed);
    }
    out
}

/// The id typed in the search field when it can be added by hand: one word, not listed.
fn typed_id<'a>(filter: &'a str, models: &[Model]) -> Option<&'a str> {
    let id = filter.trim();
    (!id.is_empty() && !id.contains(char::is_whitespace) && !models.iter().any(|m| m.id == id))
        .then_some(id)
}

/// Appends the ticked models (with their names and capabilities) to `have`, in list order,
/// skipping ones already there: what the user set for those stays theirs. Returns how many were
/// added.
fn add_ticked(have: &mut Vec<Model>, models: &[Model], ticked: &BTreeSet<String>) -> usize {
    let mut n = 0;
    for m in models {
        if ticked.contains(&m.id) && !have.iter().any(|h| h.id == m.id) {
            have.push(m.clone());
            n += 1;
        }
    }
    n
}

/// What the user did this frame.
#[derive(PartialEq)]
enum Outcome {
    None,
    Cancel,
    Add,
}

/// Shows the picker while it is open (inside the main window when `inline`, for control-channel
/// screenshots). Returns how many models were added when the user adds them.
pub(crate) fn show(
    ctx: &Context,
    pal: &Pal,
    session: &mut Session,
    picker: &mut ModelPicker,
    inline: bool,
) -> Option<usize> {
    let pid = picker.provider.clone()?;
    picker.poll(ctx);
    let Some(name) = session
        .ai
        .config
        .providers
        .iter()
        .find(|p| p.id == pid)
        .map(|p| p.name.clone())
    else {
        // The provider was removed.
        picker.close();
        return None;
    };
    let mut outcome = Outcome::None;
    if inline {
        // At the main window's right edge, over the Settings stand-in if there is no room.
        let screen = ctx.content_rect();
        egui::Area::new(Id::new("model-picker-inline"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2((screen.right() - W - 20.0).max(20.0), 70.0))
            .fade_in(false)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(W, H), Sense::hover());
                ui.painter().rect_stroke(
                    rect.expand(0.5),
                    10.0,
                    Stroke::new(1.0, pal.hairline_strong),
                    StrokeKind::Outside,
                );
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                c.set_clip_rect(rect);
                outcome = body(&mut c, pal, session, picker, rect);
            });
    } else {
        let builder = ViewportBuilder::default()
            .with_title(format!("Add Models — {name}"))
            .with_inner_size([W, H])
            .with_min_inner_size([400.0, 380.0])
            .with_minimize_button(false)
            .with_maximize_button(false);
        ctx.show_viewport_immediate(picker_viewport(), builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            crate::widgets::feed_edit_events(&ctx, &mut picker.edit_events);
            if picker.focus {
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
            let rect = ctx.content_rect();
            outcome = body(ui, pal, session, picker, rect);
            if ctx.input(|i| i.viewport().close_requested()) {
                outcome = Outcome::Cancel;
            }
        });
    }
    match outcome {
        Outcome::None => None,
        Outcome::Cancel => {
            picker.close();
            None
        }
        Outcome::Add => {
            let p = session
                .ai
                .config
                .providers
                .iter_mut()
                .find(|p| p.id == pid)?;
            let n = add_ticked(&mut p.models, &picker.models, &picker.ticked);
            picker.close();
            let _ = session.save_ai_config();
            Some(n)
        }
    }
}

/// The window's contents in `rect`.
fn body(
    ui: &mut Ui,
    pal: &Pal,
    session: &Session,
    picker: &mut ModelPicker,
    rect: Rect,
) -> Outcome {
    let p = ui.painter().clone();
    p.rect_filled(rect, 0.0, pal.bg);
    let Some(provider) = picker
        .provider
        .as_ref()
        .and_then(|id| session.ai.config.providers.iter().find(|p| &p.id == id))
    else {
        return Outcome::Cancel;
    };
    let have = &provider.models;
    let mut outcome = Outcome::None;
    let left = rect.left() + 16.0;
    let right = rect.right() - 16.0;

    // Header: the provider, then how many models it has.
    let mut y = rect.top() + 14.0;
    let logo = Rect::from_min_size(pos2(left, y), vec2(18.0, 18.0));
    match crate::ai_logos::provider(provider) {
        Some(key) => crate::ai_logos::paint(ui, key, logo, pal.label),
        None => icons::sparkles(&p, logo, pal.accent, true),
    }
    let name = crate::widgets::elide(&p, &provider.name, &semibold(13.0), right - left - 140.0);
    p.text(
        pos2(logo.right() + 8.0, logo.center().y),
        Align2::LEFT_CENTER,
        name,
        semibold(13.0),
        pal.label,
    );
    let status = match picker.models.len() {
        _ if picker.fetch.is_some() => "Fetching…".to_string(),
        0 => String::new(),
        1 => "1 model".to_string(),
        n => format!("{n} models"),
    };
    p.text(
        pos2(right, logo.center().y),
        Align2::RIGHT_CENTER,
        status,
        FontId::proportional(11.5),
        pal.note,
    );
    y += 30.0;

    // Search field.
    let sr = Rect::from_min_max(pos2(left, y), pos2(right, y + 26.0));
    let search = text_field(
        ui,
        pal,
        Id::new("model-picker-search"),
        sr,
        &mut picker.filter,
        false,
        "Search, or type a model id",
    );
    if picker.focus {
        search.request_focus();
        picker.focus = false;
    }
    let enter = search.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    y += 36.0;

    // Group tabs, with how many models each has.
    let count = |g: Option<Group>| matching(&picker.models, "", g).len();
    let names: Vec<String> = [
        None,
        Some(Group::Text),
        Some(Group::Vision),
        Some(Group::Other),
    ]
    .into_iter()
    .map(|g| format!("{} {}", g.map_or("All", Group::label), count(g)))
    .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let selected = match picker.group {
        None => 0,
        Some(Group::Text) => 1,
        Some(Group::Vision) => 2,
        Some(Group::Other) => 3,
    };
    if let Some(i) = text_tabs(
        ui,
        pal,
        Id::new("model-picker-tabs"),
        (left + right) / 2.0,
        y,
        22.0,
        ((right - left) / 4.0).min(100.0),
        &names,
        selected,
        11.5,
    ) {
        picker.group = [
            None,
            Some(Group::Text),
            Some(Group::Vision),
            Some(Group::Other),
        ]
        .get(i)
        .copied()
        .flatten();
    }
    y += 32.0;
    if let Some(e) = &picker.error {
        let g = p.layout(
            format!("{e} You can still type a model id above."),
            FontId::proportional(11.0),
            pal.close,
            right - left,
        );
        let h = g.size().y;
        p.galley(pos2(left, y), g, pal.close);
        y += h + 8.0;
    }

    // The list.
    let list = Rect::from_min_max(pos2(left, y), pos2(right, rect.bottom() - FOOTER_H - 10.0));
    p.rect(
        list,
        7.0,
        pal.bg,
        Stroke::new(0.5, pal.hairline_strong),
        StrokeKind::Inside,
    );
    let shown = matching(&picker.models, &picker.filter, picker.group);
    let typed = typed_id(&picker.filter, &picker.models).map(str::to_string);
    let rows = lines(&picker.models, &shown, picker.group, typed.is_some());
    let mut add_typed = false;
    let mut toggled: Option<String> = None;
    if rows.is_empty() {
        let msg = if picker.fetch.is_some() {
            icons::spinner(
                &p,
                Rect::from_center_size(list.center() - vec2(0.0, 14.0), vec2(14.0, 14.0)),
                pal.note,
                ui.input(|i| i.time) as f32 * 6.0,
            );
            ui.ctx().request_repaint();
            "Fetching the model list…".to_string()
        } else if picker.filter.trim().is_empty() {
            "No models".to_string()
        } else {
            format!("No models match “{}”", picker.filter.trim())
        };
        p.text(
            list.center() + vec2(0.0, 6.0),
            Align2::CENTER_CENTER,
            msg,
            FontId::proportional(12.0),
            pal.note,
        );
    } else {
        let inner = list.shrink2(vec2(5.0, 5.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        c.set_clip_rect(inner.intersect(ui.clip_rect()));
        c.spacing_mut().item_spacing.y = 0.0;
        // Rows stop short of a scroll bar.
        let room = if rows.len() as f32 * ROW_H > inner.height() {
            16.0
        } else {
            0.0
        };
        egui::ScrollArea::vertical()
            .id_salt(("model-picker-list", picker.group.map(Group::label)))
            .auto_shrink([false, false])
            .show_rows(&mut c, ROW_H, rows.len(), |ui, range| {
                for line in range.filter_map(|k| rows.get(k).copied()) {
                    let w = (ui.available_width() - room).max(0.0);
                    let sense = if matches!(line, Line::Heading(..)) {
                        Sense::hover()
                    } else {
                        Sense::click()
                    };
                    let (r, resp) = ui.allocate_exact_size(vec2(w, ROW_H), sense);
                    let painter = ui.painter();
                    let i = match line {
                        Line::Heading(g, n) => {
                            painter.text(
                                pos2(r.left() + 6.0, r.center().y + 2.0),
                                Align2::LEFT_CENTER,
                                format!("{} · {n}", g.label().to_uppercase()),
                                semibold(10.5),
                                pal.note,
                            );
                            continue;
                        }
                        Line::Typed => {
                            if resp.hovered() {
                                painter.rect_filled(r, 5.0, pal.control);
                            }
                            icons::plus(
                                painter,
                                Rect::from_center_size(
                                    pos2(r.left() + 13.5, r.center().y),
                                    vec2(11.0, 11.0),
                                ),
                                pal.accent,
                            );
                            let id = typed.as_deref().unwrap_or_default();
                            painter.text(
                                pos2(r.left() + 30.0, r.center().y),
                                Align2::LEFT_CENTER,
                                format!("Add “{id}” by its id"),
                                FontId::proportional(12.0),
                                pal.accent,
                            );
                            if resp.clicked() {
                                add_typed = true;
                            }
                            continue;
                        }
                        Line::Model(i) => i,
                    };
                    let Some(m) = picker.models.get(i) else {
                        continue;
                    };
                    let added = have.iter().any(|h| h.id == m.id);
                    let on = added || picker.ticked.contains(&m.id);
                    if resp.hovered() && !added {
                        painter.rect_filled(r, 5.0, pal.control);
                    }
                    let bx = Rect::from_center_size(
                        pos2(r.left() + 13.5, r.center().y),
                        vec2(15.0, 15.0),
                    );
                    check_box(
                        painter,
                        pal,
                        bx,
                        on,
                        !added,
                        resp.is_pointer_button_down_on(),
                    );
                    if let Some(key) = crate::ai_logos::for_entry(provider, m) {
                        crate::ai_logos::paint(
                            ui,
                            key,
                            Rect::from_center_size(
                                pos2(r.left() + 37.0, r.center().y),
                                vec2(13.0, 13.0),
                            ),
                            pal.label,
                        );
                    }
                    // The display name, the id after it when they differ; at the right "Added",
                    // or what it can do ("Vision · Reasoning · 128K").
                    let tag = if added {
                        "Added".to_string()
                    } else {
                        m.summary()
                    };
                    let tag_font = FontId::proportional(10.5);
                    let tag_w = if tag.is_empty() {
                        0.0
                    } else {
                        painter
                            .layout_no_wrap(tag.clone(), tag_font.clone(), pal.note)
                            .size()
                            .x
                            .min(r.width() * 0.45)
                    };
                    let room = (r.width() - 52.0 - tag_w - 14.0).max(0.0);
                    let label_font = FontId::proportional(12.0);
                    let label = crate::widgets::elide(painter, m.label(), &label_font, room);
                    let lg = painter.layout_no_wrap(
                        label,
                        label_font,
                        if added { pal.note } else { pal.label },
                    );
                    let lw = lg.size().x;
                    painter.galley(
                        pos2(r.left() + 52.0, r.center().y - lg.size().y / 2.0),
                        lg,
                        pal.label,
                    );
                    if !m.name.is_empty() && room - lw > 40.0 {
                        let id_font = editor_font(10.5);
                        let id = crate::widgets::elide(painter, &m.id, &id_font, room - lw - 8.0);
                        painter.text(
                            pos2(r.left() + 52.0 + lw + 8.0, r.center().y),
                            Align2::LEFT_CENTER,
                            id,
                            id_font,
                            pal.note,
                        );
                    }
                    if !tag.is_empty() {
                        let t = crate::widgets::elide(painter, &tag, &tag_font, tag_w);
                        painter.text(
                            pos2(r.right() - 8.0, r.center().y),
                            Align2::RIGHT_CENTER,
                            t,
                            tag_font,
                            pal.note,
                        );
                    }
                    if resp.clicked() && !added {
                        toggled = Some(m.id.clone());
                    }
                }
            });
    }
    if let Some(m) = toggled
        && !picker.ticked.remove(&m)
    {
        picker.ticked.insert(m);
    }
    if let Some(id) = &typed
        && (add_typed || (enter && shown.is_empty()))
    {
        picker.add_typed(id);
    }

    // Footer: Select All on the left, Cancel and Add on the right.
    hairline(
        ui,
        rect.left(),
        rect.right(),
        rect.bottom() - FOOTER_H,
        pal.hairline,
    );
    let cy = rect.bottom() - FOOTER_H / 2.0;
    let selectable: Vec<&String> = shown
        .iter()
        .filter_map(|&i| picker.models.get(i))
        .map(|m| &m.id)
        .filter(|id| !have.iter().any(|h| &h.id == *id))
        .collect();
    if !selectable.is_empty() {
        let all = selectable.iter().all(|id| picker.ticked.contains(*id));
        let sa = Rect::from_min_size(pos2(left, cy - 11.0), vec2(100.0, 22.0));
        let label = if all { "Deselect All" } else { "Select All" };
        if push_button(ui, pal, Id::new("model-picker-all"), sa, label, 11.5) {
            for id in selectable {
                if all {
                    picker.ticked.remove(id);
                } else {
                    picker.ticked.insert(id.clone());
                }
            }
        }
    }
    let n = picker
        .ticked
        .iter()
        .filter(|m| !have.iter().any(|h| &h.id == *m))
        .count();
    let add_label = match n {
        0 => "Add".to_string(),
        1 => "Add 1 Model".to_string(),
        n => format!("Add {n} Models"),
    };
    let ar = Rect::from_min_size(pos2(right - 124.0, cy - 11.0), vec2(124.0, 22.0));
    let cr = Rect::from_min_size(pos2(ar.left() - 84.0, cy - 11.0), vec2(76.0, 22.0));
    if push_button(ui, pal, Id::new("model-picker-cancel"), cr, "Cancel", 11.5) {
        outcome = Outcome::Cancel;
    }
    if accent_button(ui, pal, Id::new("model-picker-add"), ar, &add_label, n > 0)
        || (enter && !shown.is_empty() && n > 0)
    {
        outcome = Outcome::Add;
    }
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        outcome = Outcome::Cancel;
    }
    outcome
}

/// A button filled with the accent colour (grey and inert when `enabled` is false). Returns
/// whether it was clicked.
fn accent_button(ui: &mut Ui, pal: &Pal, id: Id, r: Rect, label: &str, enabled: bool) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let resp = ui.interact(r, id, sense);
    let (fill, text) = if !enabled {
        (pal.control, pal.note)
    } else if resp.is_pointer_button_down_on() {
        (pal.accent.gamma_multiply(0.8), Color32::WHITE)
    } else {
        (pal.accent, Color32::WHITE)
    };
    ui.painter().rect_filled(r, 6.0, fill);
    ui.painter().text(
        r.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(11.5),
        text,
    );
    enabled && resp.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &[Capability] = &[];
    const VISION: &[Capability] = &[Capability::Vision];
    const EMBEDDING: &[Capability] = &[Capability::Embedding];

    fn info(id: &str, caps: &[Capability]) -> Model {
        Model {
            id: id.into(),
            capabilities: caps.to_vec(),
            ..Model::default()
        }
    }

    fn texts(ids: &[&str]) -> Vec<Model> {
        ids.iter().map(|id| info(id, TEXT)).collect()
    }

    fn ids(models: &[Model]) -> Vec<&str> {
        models.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn search_matches_every_word_ignoring_case_within_the_group() {
        let models = vec![
            info("gpt-4.1", VISION),
            info("GPT-4.1-mini", VISION),
            info("claude-opus-5-5", VISION),
            info("qwen3-coder", TEXT),
            info("text-embedding-3-small", EMBEDDING),
        ];
        assert_eq!(matching(&models, "", None), vec![0, 1, 2, 3, 4]);
        assert_eq!(matching(&models, "gpt", None), vec![0, 1]);
        assert_eq!(matching(&models, "GPT mini", None), vec![1]);
        assert_eq!(matching(&models, "  opus  ", None), vec![2]);
        assert!(matching(&models, "llama", None).is_empty());
        assert_eq!(matching(&models, "", Some(Group::Text)), vec![3]);
        assert_eq!(matching(&models, "", Some(Group::Vision)), vec![0, 1, 2]);
        assert_eq!(matching(&models, "", Some(Group::Other)), vec![4]);
        assert!(matching(&models, "qwen", Some(Group::Vision)).is_empty());
    }

    #[test]
    fn lines_group_models_under_headings() {
        let models = vec![
            info("a-vl", VISION),
            info("b", TEXT),
            info("c-embed", EMBEDDING),
            info("d", TEXT),
        ];
        let all = matching(&models, "", None);
        assert_eq!(
            lines(&models, &all, None, false),
            vec![
                Line::Heading(Group::Text, 2),
                Line::Model(1),
                Line::Model(3),
                Line::Heading(Group::Vision, 1),
                Line::Model(0),
                Line::Heading(Group::Other, 1),
                Line::Model(2),
            ]
        );
        // One group: no headings; the typed id comes last.
        let text = matching(&models, "", Some(Group::Text));
        assert_eq!(
            lines(&models, &text, Some(Group::Text), true),
            vec![Line::Model(1), Line::Model(3), Line::Typed]
        );
        assert_eq!(lines(&models, &[], None, true), vec![Line::Typed]);
    }

    #[test]
    fn a_typed_id_is_offered_only_when_not_listed() {
        let models = texts(&["gpt-4.1"]);
        assert_eq!(typed_id("my-alias", &models), Some("my-alias"));
        assert_eq!(typed_id("  my-alias ", &models), Some("my-alias"));
        assert_eq!(typed_id("gpt-4.1", &models), None);
        assert_eq!(typed_id("two words", &models), None);
        assert_eq!(typed_id("   ", &models), None);
    }

    #[test]
    fn ticked_models_are_added_once_in_list_order() {
        let models = texts(&["a", "b", "c", "d"]);
        let ticked: BTreeSet<String> = ["d", "b", "c"].iter().map(|s| (*s).to_string()).collect();
        let mut have = vec![Model {
            name: "Mine".into(),
            ..info("c", VISION)
        }];
        assert_eq!(add_ticked(&mut have, &models, &ticked), 2);
        assert_eq!(ids(&have), vec!["c", "b", "d"]);
        // What the user set for a model already there stays.
        assert_eq!(
            (have[0].label(), have[0].has(Capability::Vision)),
            ("Mine", true)
        );
        assert_eq!(add_ticked(&mut have, &models, &ticked), 0);
    }

    #[test]
    fn a_fetched_list_is_sorted_and_keeps_typed_ids_first() {
        let mut picker = ModelPicker::default();
        picker.add_typed("my-alias");
        picker.set_models(texts(&["b-model", " A-model ", "b-model", "", "my-alias"]));
        assert_eq!(ids(&picker.models), vec!["A-model", "b-model", "my-alias"]);
        let mut picker = ModelPicker::default();
        picker.add_typed("typed-vl");
        assert!(picker.models[0].has(Capability::Vision));
        picker.set_models(texts(&["z", "y"]));
        assert_eq!(ids(&picker.models), vec!["typed-vl", "y", "z"]);
        assert!(picker.ticked.contains("typed-vl"));
        // SoCLaaS aliases are not read for capabilities.
        let mut soclaas = ModelPicker {
            aliases: true,
            ..ModelPicker::default()
        };
        soclaas.add_typed("typed-vl");
        assert!(soclaas.models[0].capabilities.is_empty());
        let mut empty = ModelPicker::default();
        empty.set_models(Vec::new());
        assert!(empty.error.is_some());
    }

    #[test]
    fn group_tabs_by_name() {
        let mut picker = ModelPicker::default();
        picker.set_group(Some("vision"));
        assert_eq!(picker.group, Some(Group::Vision));
        picker.set_group(Some("nonsense"));
        assert_eq!(picker.group, None);
        picker.set_group(None);
        assert_eq!(picker.group, None);
    }

    /// A long list (OpenRouter serves hundreds) filters by any part of the id.
    #[test]
    fn filters_hundreds_of_models() {
        let models: Vec<Model> = (0..800)
            .map(|i| info(&format!("vendor/model-{i:03}"), TEXT))
            .collect();
        assert_eq!(matching(&models, "model-79", None).len(), 10);
        assert_eq!(matching(&models, "VENDOR", None).len(), 800);
    }
}
