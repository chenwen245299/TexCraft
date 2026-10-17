//! The Settings window and the Typeset Configuration sheet, laid out like the target app's
//! preferences: a toolbar of panes, then rows with a right-aligned description on the left and
//! the control on the right (a checkbox with a bold label, or a bold title over a pop-up menu).
//! Positions are measured from the target's screenshots, in points.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, Align2, Color32, Context, FontId, Galley, Id, Key, Rect, Sense, Stroke, StrokeKind, Ui,
    ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2,
};
use texcraft_engine::{EditorTheme, Grading, PageMode, Session, ThemeChoice, Zoom};
use texcraft_project::{BibTool, Engine, TypesetMode, Typesetter};

use crate::icons;
use crate::theme::{Tokens, semibold};

/// Title of the Settings window (hidden; it finds the native window for its buttons).
pub const SETTINGS_TITLE: &str = "TexCraft Settings";
/// Settings window width.
const WINDOW_W: f32 = 700.0;
/// Height of the pane toolbar (icons over labels).
const PANE_BAR_H: f32 = 66.0;
/// The sub-tab bar under the toolbar (Editor pane), ending in a hairline.
const SUBTAB_LINE_Y: f32 = 88.0;
/// Typeset Configuration sheet size.
const SHEET_W: f32 = 700.0;
const SHEET_H: f32 = 547.0;

/// The Settings window's viewport.
pub fn settings_viewport() -> ViewportId {
    ViewportId::from_hash_of("texcraft-settings")
}

/// A pane of the Settings window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Pane {
    /// Appearance and start-up.
    #[default]
    General,
    /// The outline in the sidebar.
    Sidebar,
    /// Editing aids and the editor's look.
    Editor,
    /// The PDF viewer.
    Viewer,
    /// Typesetting and live preview.
    Typesetting,
    /// AI Assistant providers and models.
    Ai,
}

impl Pane {
    /// All panes, in toolbar order.
    pub const ALL: [Pane; 6] = [
        Self::General,
        Self::Sidebar,
        Self::Editor,
        Self::Ai,
        Self::Viewer,
        Self::Typesetting,
    ];

    /// Toolbar label.
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Sidebar => "Sidebar",
            Self::Editor => "Editor",
            Self::Viewer => "Document Viewer",
            Self::Typesetting => "Typesetting",
            Self::Ai => "AI",
        }
    }

    /// Parses a control-channel name.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "general" => Self::General,
            "sidebar" => Self::Sidebar,
            "editor" => Self::Editor,
            "viewer" | "documentViewer" => Self::Viewer,
            "typesetting" => Self::Typesetting,
            "ai" => Self::Ai,
            _ => return None,
        })
    }
}

/// Typeset Configuration tabs.
const SHEET_TABS: [&str; 4] = ["Summary", "General", "Bibliographies", "Indices"];
/// Editor pane tabs.
const EDITOR_TABS: [&str; 2] = ["Editing Aids", "Editor Look & Feel"];

/// State of the Settings window and the Typeset Configuration sheet.
#[derive(Default)]
pub struct Prefs {
    /// The Settings window is open.
    pub open: bool,
    /// Bring the Settings window to the front on the next frame.
    pub focus: bool,
    /// The selected pane.
    pub pane: Pane,
    /// Editor pane tab (index into Editing Aids, Editor Look & Feel).
    pub editor_tab: usize,
    /// Typeset Configuration tab (Summary, General, Bibliographies, Indices).
    pub sheet_tab: usize,
    /// Draw the Settings window's contents inside the main window instead (control channel:
    /// eframe cannot take screenshots of secondary windows).
    pub inline: bool,
    /// Window height that fits the current pane.
    height: f32,
    /// The AI pane.
    ai: AiPaneState,
    /// Edit events from the menu bar (⌘V and co.) for the window's text fields.
    pub(crate) edit_events: Vec<egui::Event>,
}

impl Prefs {
    /// The model picker window (Settings → AI → Add Models…).
    pub fn model_picker(&mut self) -> &mut crate::model_picker::ModelPicker {
        &mut self.ai.picker
    }

    /// Opens the model picker for the provider selected in the AI pane.
    pub fn open_model_picker(&mut self, ctx: &Context, session: &Session) {
        let i = self.ai.selected;
        self.ai.picker.open(ctx, session, i);
    }

    /// What the model picker shows, for `state.inspect`.
    pub fn model_picker_info(&self) -> serde_json::Value {
        self.ai.picker.info()
    }
}

/// Colours of the preference forms (from the design tokens).
pub(crate) struct Pal {
    pub(crate) bg: Color32,
    pub(crate) hairline: Color32,
    pub(crate) hairline_strong: Color32,
    pub(crate) label: Color32,
    pub(crate) desc: Color32,
    pub(crate) note: Color32,
    pub(crate) control: Color32,
    pub(crate) control_hover: Color32,
    pub(crate) control_text: Color32,
    pub(crate) off: Color32,
    pub(crate) off_border: Color32,
    pub(crate) item: Color32,
    pub(crate) item_selected: Color32,
    pub(crate) accent: Color32,
    pub(crate) live: Color32,
    pub(crate) close: Color32,
    /// The full design tokens (menus drawn by [`crate::menu`]).
    pub(crate) tokens: Tokens,
}

pub(crate) fn pal(t: &Tokens) -> Pal {
    Pal {
        bg: t.form_bg,
        hairline: t.form_hairline,
        hairline_strong: t.form_hairline_strong,
        label: t.form_label,
        desc: t.form_desc,
        note: t.form_note,
        control: t.form_popup,
        control_hover: t.control_hover,
        control_text: t.control_text,
        off: t.check_off,
        off_border: t.check_off_border,
        item: t.pane_item,
        item_selected: t.pane_item_selected,
        accent: t.accent,
        live: t.success,
        close: t.error,
        tokens: t.clone(),
    }
}

/// Text sizes and offsets of a form (the Settings window and the sheet differ slightly).
#[derive(Clone, Copy)]
struct Style {
    desc_size: f32,
    desc_line: f32,
    label_size: f32,
    check: f32,
    /// Checkbox centre below the row top.
    check_dy: f32,
    /// Pop-up title centre below the row top.
    title_dy: f32,
    /// Pop-up button top below the row top.
    popup_dy: f32,
    popup_h: f32,
    /// Pop-up and push-button text size.
    control_size: f32,
    /// Minimum row steps.
    check_step: f32,
    popup_step: f32,
}

const WINDOW_STYLE: Style = Style {
    desc_size: 11.1,
    desc_line: 13.0,
    label_size: 11.25,
    check: 14.0,
    check_dy: 10.0,
    title_dy: 6.5,
    popup_dy: 18.0,
    popup_h: 23.0,
    control_size: 11.2,
    check_step: 40.0,
    popup_step: 50.0,
};

const SHEET_STYLE: Style = Style {
    desc_size: 12.0,
    desc_line: 14.1,
    label_size: 12.0,
    check: 15.0,
    check_dy: 9.0,
    title_dy: 9.0,
    popup_dy: 19.0,
    popup_h: 23.0,
    control_size: 11.5,
    check_step: 40.0,
    popup_step: 50.0,
};

/// A two-column form: descriptions right-aligned against `split`, controls from `ctrl` to
/// `right`. Rows stack down from `y`.
struct Form<'a> {
    ui: &'a mut Ui,
    pal: &'a Pal,
    style: Style,
    desc_color: Color32,
    left: f32,
    split: f32,
    ctrl: f32,
    right: f32,
    y: f32,
    id: Id,
}

impl Form<'_> {
    fn desc_galley(&self, text: &str) -> Arc<Galley> {
        let mut job = LayoutJob::default();
        job.append(
            text,
            0.0,
            TextFormat {
                font_id: FontId::proportional(self.style.desc_size),
                color: self.desc_color,
                line_height: Some(self.style.desc_line),
                ..Default::default()
            },
        );
        job.wrap.max_width = self.split - self.left;
        job.halign = Align::RIGHT;
        self.ui.fonts_mut(|f| f.layout_job(job))
    }

    /// Paints the description of a row starting at `top`; returns its height.
    fn desc(&self, text: &str, top: f32) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let g = self.desc_galley(text);
        let h = g.size().y;
        self.ui
            .painter()
            .galley(pos2(self.split, top), g, self.desc_color);
        h
    }

    fn row_id(&self, label: &str) -> Id {
        self.id.with(label)
    }

    /// A checkbox row. Returns whether the value changed.
    fn checkbox(&mut self, desc: &str, label: &str, value: &mut bool) -> bool {
        self.checkbox_enabled(desc, label, value, true)
    }

    fn checkbox_enabled(
        &mut self,
        desc: &str,
        label: &str,
        value: &mut bool,
        enabled: bool,
    ) -> bool {
        let top = self.y;
        let dh = self.desc(desc, top);
        let cy = top + self.style.check_dy;
        let changed = check_control(
            self.ui,
            self.pal,
            self.row_id(label),
            pos2(self.ctrl, cy),
            self.style.check,
            label,
            self.style.label_size,
            value,
            enabled,
        );
        self.y += self.style.check_step.max(dh + 15.0);
        changed
    }

    /// A pop-up row: bold title over a pop-up button. Returns whether the value changed.
    fn popup<T: PartialEq + Clone>(
        &mut self,
        desc: &str,
        title: &str,
        value: &mut T,
        options: &[(T, String)],
    ) -> bool {
        self.popup_enabled(desc, title, value, options, true)
    }

    fn popup_enabled<T: PartialEq + Clone>(
        &mut self,
        desc: &str,
        title: &str,
        value: &mut T,
        options: &[(T, String)],
        enabled: bool,
    ) -> bool {
        let top = self.y;
        let dh = self.desc(desc, top);
        let p = self.ui.painter();
        p.text(
            pos2(self.ctrl, top + self.style.title_dy),
            Align2::LEFT_CENTER,
            title,
            semibold(self.style.label_size),
            if enabled {
                self.pal.label
            } else {
                self.pal.note
            },
        );
        let r = Rect::from_min_max(
            pos2(self.ctrl, top + self.style.popup_dy),
            pos2(self.right, top + self.style.popup_dy + self.style.popup_h),
        );
        let changed = popup_control_enabled(
            self.ui,
            self.pal,
            self.row_id(title),
            r,
            value,
            options,
            self.style.control_size,
            enabled,
        );
        self.y += self.style.popup_step.max(dh + 15.0);
        changed
    }

    /// A row with a bold value instead of a control (Summary).
    fn value(&mut self, desc: &str, value: &str) {
        let top = self.y;
        let dh = self.desc(desc, top);
        let font = semibold(self.style.label_size);
        let g = self.ui.painter().layout(
            value.to_string(),
            font,
            self.pal.label,
            self.right - self.ctrl,
        );
        let vh = g.size().y;
        self.ui.painter().galley(
            pos2(
                self.ctrl,
                top + self.style.title_dy - self.style.desc_line / 2.0,
            ),
            g,
            self.pal.label,
        );
        self.y += 34.0_f32.max(dh.max(vh) + 14.0);
    }

    /// A row with a push button. Returns whether it was clicked.
    fn button(&mut self, desc: &str, label: &str) -> bool {
        let top = self.y;
        let dh = self.desc(desc, top);
        let font = FontId::proportional(self.style.control_size);
        let w = self
            .ui
            .painter()
            .layout_no_wrap(label.to_string(), font, self.pal.control_text)
            .size()
            .x
            + 26.0;
        let r = Rect::from_min_size(pos2(self.ctrl, top + 1.0), vec2(w, 22.0));
        let clicked = push_button(
            self.ui,
            self.pal,
            self.row_id(label),
            r,
            label,
            self.style.control_size,
        );
        self.y += self.style.check_step.max(dh + 15.0);
        clicked
    }
}

/// Draws a checkbox with a bold label at `left_center`. Returns whether the value changed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_control(
    ui: &mut Ui,
    pal: &Pal,
    id: Id,
    left_center: egui::Pos2,
    side: f32,
    label: &str,
    label_size: f32,
    value: &mut bool,
    enabled: bool,
) -> bool {
    let p = ui.painter().clone();
    let font = semibold(label_size);
    let label_color = if enabled { pal.label } else { pal.note };
    let g = p.layout_no_wrap(label.to_string(), font, label_color);
    let label_x = left_center.x + side + 6.0;
    let hit = Rect::from_min_max(
        pos2(left_center.x - 2.0, left_center.y - side / 2.0 - 3.0),
        pos2(label_x + g.size().x + 4.0, left_center.y + side / 2.0 + 3.0),
    );
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let resp = ui.interact(hit, id, sense);
    let bx = Rect::from_center_size(
        pos2(left_center.x + side / 2.0, left_center.y),
        vec2(side, side),
    );
    check_box(
        &p,
        pal,
        bx,
        *value,
        enabled,
        resp.is_pointer_button_down_on(),
    );
    p.galley(
        pos2(label_x, left_center.y - g.size().y / 2.0),
        g,
        label_color,
    );
    if resp.clicked() {
        *value = !*value;
        true
    } else {
        false
    }
}

/// Paints a checkbox in `bx`: accent with a white tick when on (paler when disabled), grey when
/// off.
pub(crate) fn check_box(
    p: &egui::Painter,
    pal: &Pal,
    bx: Rect,
    value: bool,
    enabled: bool,
    pressed: bool,
) {
    if value {
        let fill = if enabled {
            pal.accent
        } else {
            pal.accent.gamma_multiply(0.45)
        };
        p.rect_filled(bx, 3.5, fill);
        icons::check_mark(p, bx.center(), bx.width(), Color32::WHITE);
    } else {
        p.rect_filled(bx, 3.5, if pressed { pal.control_hover } else { pal.off });
        p.rect_stroke(
            bx,
            3.5,
            Stroke::new(0.6, pal.off_border),
            StrokeKind::Inside,
        );
    }
}

/// Draws a radio button (filled grey when off, accent with a white dot when on).
fn radio(p: &egui::Painter, pal: &Pal, center: egui::Pos2, on: bool) {
    if on {
        p.circle_filled(center, 8.0, pal.accent);
        p.circle_filled(center, 3.2, Color32::WHITE);
    } else {
        p.circle_filled(center, 8.0, pal.off);
        p.circle_stroke(center, 8.0, Stroke::new(0.6, pal.off_border));
    }
}

/// A pop-up button in `r` listing `options`. Returns whether the value changed.
pub(crate) fn popup_control<T: PartialEq + Clone>(
    ui: &mut Ui,
    pal: &Pal,
    id: Id,
    r: Rect,
    value: &mut T,
    options: &[(T, String)],
    size: f32,
) -> bool {
    popup_control_enabled(ui, pal, id, r, value, options, size, true)
}

/// [`popup_control`]; a disabled one shows `value` greyed out and does not open.
#[allow(clippy::too_many_arguments)]
fn popup_control_enabled<T: PartialEq + Clone>(
    ui: &mut Ui,
    pal: &Pal,
    id: Id,
    r: Rect,
    value: &mut T,
    options: &[(T, String)],
    size: f32,
    enabled: bool,
) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let resp = ui.interact(r, id, sense);
    let p = ui.painter();
    p.rect_filled(
        r,
        5.5,
        if enabled && resp.hovered() {
            pal.control_hover
        } else {
            pal.control
        },
    );
    let current = options
        .iter()
        .find(|(v, _)| v == value)
        .map_or("—", |(_, s)| s.as_str());
    let font = FontId::proportional(size);
    let text = crate::widgets::elide(p, current, &font, r.width() - 36.0);
    let text_color = if enabled { pal.control_text } else { pal.note };
    p.text(
        pos2(r.left() + 11.5, r.center().y),
        Align2::LEFT_CENTER,
        text,
        font,
        text_color,
    );
    icons::updown(p, pos2(r.right() - 13.0, r.center().y), text_color);
    if !enabled {
        return false;
    }
    let mut changed = false;
    egui::Popup::menu(&resp).width(r.width()).show(|ui| {
        crate::menu::begin(ui, r.width());
        for (v, label) in options {
            let opts = crate::menu::Item {
                checked: Some(v == value),
                ..Default::default()
            };
            if crate::menu::item(ui, &pal.tokens, label, opts).clicked() && v != value {
                *value = v.clone();
                changed = true;
            }
        }
    });
    changed
}

/// A rounded push button. Returns whether it was clicked.
pub(crate) fn push_button(ui: &mut Ui, pal: &Pal, id: Id, r: Rect, label: &str, size: f32) -> bool {
    let resp = ui.interact(r, id, Sense::click());
    let p = ui.painter();
    let fill = if resp.is_pointer_button_down_on() {
        pal.control_hover
    } else if resp.hovered() {
        pal.control
    } else {
        pal.bg
    };
    p.rect_filled(r, 6.0, fill);
    p.rect_stroke(
        r,
        6.0,
        Stroke::new(0.6, pal.hairline_strong),
        StrokeKind::Inside,
    );
    p.text(
        r.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(size),
        pal.control_text,
    );
    resp.clicked()
}

/// Text tabs: the selected one filled with the accent colour. Returns the clicked index.
#[allow(clippy::too_many_arguments)]
pub(crate) fn text_tabs(
    ui: &mut Ui,
    pal: &Pal,
    id: Id,
    center_x: f32,
    top: f32,
    h: f32,
    seg_w: f32,
    names: &[&str],
    selected: usize,
    size: f32,
) -> Option<usize> {
    let total = seg_w * names.len() as f32;
    let mut x = center_x - total / 2.0;
    let mut clicked = None;
    for (i, name) in names.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, top), vec2(seg_w, h));
        x += seg_w;
        let resp = ui.interact(r, id.with(i), Sense::click());
        let p = ui.painter();
        let color = if i == selected {
            p.rect_filled(r, 5.5, pal.accent);
            Color32::WHITE
        } else {
            if resp.hovered() {
                p.rect_filled(r, 5.5, pal.item_selected);
            }
            pal.label
        };
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            *name,
            FontId::proportional(size),
            color,
        );
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

pub(crate) fn hairline(ui: &Ui, x0: f32, x1: f32, y: f32, c: Color32) {
    ui.painter()
        .line_segment([pos2(x0, y + 0.5), pos2(x1, y + 0.5)], Stroke::new(1.0, c));
}

pub(crate) fn opts<T: Clone>(list: &[(T, &str)]) -> Vec<(T, String)> {
    list.iter()
        .map(|(v, s)| (v.clone(), (*s).to_string()))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Settings window

/// Shows the Settings window while it is open. Returns true when the Typeset Configuration
/// sheet should open (its button in the Typesetting pane).
pub fn settings_window(
    ctx: &Context,
    t: &Tokens,
    session: &mut Session,
    prefs: &mut Prefs,
    outline_depth: &mut u8,
) -> bool {
    if !prefs.open {
        if prefs.ai.picker.is_open() {
            prefs.ai.picker.close();
        }
        return false;
    }
    let open_sheet = settings_contents(ctx, t, session, prefs, outline_depth);
    // The model picker (Add Models…), over the Settings window.
    let pal = pal(t);
    if let Some(n) =
        crate::model_picker::show(ctx, &pal, session, &mut prefs.ai.picker, prefs.inline)
    {
        let msg = match n {
            0 => "Those models were already added.".to_string(),
            1 => "Added 1 model.".to_string(),
            n => format!("Added {n} models."),
        };
        prefs.ai.message = Some((msg, true));
    }
    open_sheet
}

/// The Settings window itself (or its stand-in inside the main window).
fn settings_contents(
    ctx: &Context,
    t: &Tokens,
    session: &mut Session,
    prefs: &mut Prefs,
    outline_depth: &mut u8,
) -> bool {
    if prefs.inline {
        let mut open_sheet = false;
        let pal = pal(t);
        egui::Area::new(Id::new("prefs-inline"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(60.0, 70.0))
            .fade_in(false)
            .show(ctx, |ui| {
                let h = prefs.height.max(300.0);
                let (rect, _) = ui.allocate_exact_size(vec2(WINDOW_W, h), Sense::hover());
                ui.painter().rect_stroke(
                    rect.expand(0.5),
                    10.0,
                    Stroke::new(1.0, pal.hairline_strong),
                    StrokeKind::Outside,
                );
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                c.set_clip_rect(rect);
                open_sheet = settings_body(&mut c, &pal, session, prefs, outline_depth, rect);
            });
        return open_sheet;
    }
    if prefs.height < 200.0 {
        prefs.height = 520.0;
    }
    let mut builder = ViewportBuilder::default()
        .with_title(SETTINGS_TITLE)
        .with_inner_size([WINDOW_W, prefs.height])
        .with_resizable(false)
        .with_minimize_button(false)
        .with_maximize_button(false);
    if cfg!(target_os = "macos") {
        builder = builder
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    let pal = pal(t);
    let mut open_sheet = false;
    ctx.show_viewport_immediate(settings_viewport(), builder, |ui, _class| {
        let ctx = ui.ctx().clone();
        crate::widgets::feed_edit_events(&ctx, &mut prefs.edit_events);
        if ctx.input(|i| i.viewport().close_requested()) {
            prefs.open = false;
        }
        if prefs.focus {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            prefs.focus = false;
        }
        if cfg!(target_os = "macos") {
            let _ = texcraft_macos::place_traffic_lights_titled(SETTINGS_TITLE, 51.0, 19.0, true);
        }
        let rect = ctx.content_rect();
        open_sheet = settings_body(ui, &pal, session, prefs, outline_depth, rect);
    });
    open_sheet
}

/// The Settings window's contents in `rect`. Returns true when the Typeset Configuration sheet
/// should open.
fn settings_body(
    ui: &mut Ui,
    pal: &Pal,
    session: &mut Session,
    prefs: &mut Prefs,
    outline_depth: &mut u8,
    rect: Rect,
) -> bool {
    let mut open_sheet = false;
    {
        ui.painter().rect_filled(rect, 0.0, pal.bg);
        if prefs.inline {
            // Stand-in for the window's close button.
            ui.painter()
                .circle_filled(pos2(rect.left() + 25.0, rect.top() + 25.0), 6.0, pal.close);
        }
        let content_top = pane_bar(ui, pal, prefs, rect);
        let mut form = Form {
            ui,
            pal,
            style: WINDOW_STYLE,
            desc_color: pal.desc,
            left: rect.left() + 13.0,
            split: rect.left() + 409.0,
            ctrl: rect.left() + 420.0,
            right: rect.right() - 13.0,
            y: content_top,
            id: Id::new(("prefs", prefs.pane, prefs.editor_tab)),
        };
        match prefs.pane {
            Pane::General => general_pane(&mut form, session),
            Pane::Sidebar => sidebar_pane(&mut form, outline_depth),
            Pane::Editor => {
                if prefs.editor_tab == 0 {
                    editing_aids(&mut form, session);
                } else {
                    look_and_feel(&mut form, session);
                }
            }
            Pane::Viewer => viewer_pane(&mut form, session),
            Pane::Typesetting => open_sheet = typesetting_pane(&mut form, session),
            Pane::Ai => ai_pane(&mut form, session, &mut prefs.ai),
        }
        let wanted = (form.y - rect.top() + 36.0).max(300.0).round();
        if (wanted - prefs.height).abs() > 0.5 {
            prefs.height = wanted;
            ui.ctx().request_repaint();
        }
    }
    open_sheet
}

/// The pane toolbar (right-aligned items, like the target) and, for the Editor pane, its
/// sub-tabs. Returns the top of the first form row.
fn pane_bar(ui: &mut Ui, pal: &Pal, prefs: &mut Prefs, rect: Rect) -> f32 {
    let top = rect.top();
    // Dragging the empty toolbar moves the window (the title bar is hidden).
    let bar = Rect::from_min_max(rect.min, pos2(rect.right(), top + PANE_BAR_H));
    let drag = ui.interact(bar, Id::new("prefs-drag"), Sense::click_and_drag());
    if drag.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    let label_font = FontId::proportional(11.0);
    let mut x = rect.right() - 2.0;
    for pane in Pane::ALL.iter().rev() {
        let lw = ui
            .painter()
            .layout_no_wrap(pane.label().to_string(), label_font.clone(), pal.item)
            .size()
            .x;
        let w = (lw + 23.0).round();
        let item = Rect::from_min_max(pos2(x - w, top + 2.0), pos2(x, top + 61.0));
        x -= w;
        let resp = ui.interact(item, Id::new(("prefs-pane", *pane)), Sense::click());
        let selected = prefs.pane == *pane;
        let p = ui.painter();
        if selected || resp.hovered() {
            p.rect_filled(item, 9.0, pal.item_selected);
        }
        let c = if selected { pal.accent } else { pal.item };
        let icon = Rect::from_center_size(pos2(item.center().x, top + 25.0), vec2(24.0, 24.0));
        match pane {
            Pane::General => icons::prefs_general(p, icon, c),
            Pane::Sidebar => icons::prefs_sidebar(p, icon, c),
            Pane::Editor => icons::prefs_editor(p, icon, c),
            Pane::Viewer => icons::prefs_viewer(p, icon, c),
            Pane::Typesetting => icons::gears_filled(p, icon.expand(1.0), c, pal.bg, 0.0),
            Pane::Ai => icons::sparkles(p, icon.shrink(1.5), c, true),
        }
        p.text(
            pos2(item.center().x, top + 53.0),
            Align2::CENTER_CENTER,
            pane.label(),
            label_font.clone(),
            c,
        );
        if resp.clicked() {
            prefs.pane = *pane;
        }
    }
    if prefs.pane == Pane::Editor {
        if let Some(i) = text_tabs(
            ui,
            pal,
            Id::new("prefs-editor-tabs"),
            rect.center().x,
            top + PANE_BAR_H - 0.5,
            20.0,
            112.0,
            &EDITOR_TABS,
            prefs.editor_tab,
            11.8,
        ) {
            prefs.editor_tab = i;
        }
        hairline(
            ui,
            rect.left(),
            rect.right(),
            top + SUBTAB_LINE_Y,
            pal.hairline,
        );
        top + SUBTAB_LINE_Y + 12.0
    } else {
        hairline(
            ui,
            rect.left(),
            rect.right(),
            top + PANE_BAR_H,
            pal.hairline,
        );
        top + PANE_BAR_H + 14.0
    }
}

fn general_pane(f: &mut Form<'_>, session: &mut Session) {
    f.popup(
        "Choose whether TexCraft follows the system appearance or always uses the light or dark appearance.",
        "Appearance",
        &mut session.view.theme,
        &opts(&[
            (ThemeChoice::System, "Use System Setting"),
            (ThemeChoice::Light, "Light"),
            (ThemeChoice::Dark, "Dark"),
        ]),
    );
    f.checkbox(
        "Show the welcome window with recent documents and templates when TexCraft starts without a document.",
        "Show welcome window at startup",
        &mut session.view.show_welcome,
    );
    f.checkbox(
        "Show the issues list in the sidebar when a typeset ends with errors.",
        "Show issues after a failed typeset",
        &mut session.view.show_issues_after_error,
    );
    f.checkbox(
        "TexCraft looks for a new version when it starts and every few hours.",
        "Check for updates automatically",
        &mut session.view.auto_check_updates,
    );
    f.checkbox(
        "New versions are downloaded in the background and installed when you quit TexCraft.",
        "Download updates automatically",
        &mut session.view.auto_download_updates,
    );
    let last = if session.view.last_update_check == 0 {
        "Never checked".to_string()
    } else {
        let at =
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(session.view.last_update_check);
        format!("Last checked {}", crate::clock::date_time(at))
    };
    if f.button(
        &format!(
            "This is TexCraft {}. {last}.",
            texcraft_engine::CURRENT_VERSION
        ),
        "Check for Updates…",
    ) {
        let _ = session.run("app.checkForUpdates", &serde_json::Value::Null);
    }
}

fn sidebar_pane(f: &mut Form<'_>, outline_depth: &mut u8) {
    let mut depth = if *outline_depth == 0 {
        3
    } else {
        *outline_depth
    };
    if f.popup(
        "The deepest sectioning level listed in the Table of Contents of the Structure view.",
        "Table of contents depth",
        &mut depth,
        &opts(&[
            (1u8, "Parts and chapters"),
            (2, "Sections"),
            (3, "Subsections"),
            (4, "Subsubsections"),
            (5, "Paragraphs"),
        ]),
    ) {
        *outline_depth = depth;
    }
}

fn editing_aids(f: &mut Form<'_>, session: &mut Session) {
    let v = &mut session.view;
    f.checkbox(
        "Selecting this will display line numbers in the editor.",
        "Show line numbers in the editor",
        &mut v.line_numbers,
    );
    f.checkbox(
        "TexCraft can automatically complete LaTeX commands as you type your documents.",
        "Auto-complete commands",
        &mut v.auto_complete,
    );
    f.checkbox(
        "Automatically indent each line based on the line before.",
        "Auto Indent",
        &mut v.auto_indent,
    );
    f.checkbox(
        "Automatically insert an \\end{…} command after each \\begin{…}.",
        "Automatch \\begin with \\end",
        &mut v.auto_end,
    );
    f.popup(
        "Number of spaces to indent by. This is used for inserting tabs and auto-indentations.",
        "Editor tab indent size",
        &mut v.tab_spaces,
        &opts(&[
            (0u8, "Tab"),
            (2, "2 Spaces"),
            (4, "4 Spaces"),
            (8, "8 Spaces"),
        ]),
    );
    // Wrapping: 0 none, 1 soft wrap, 2 soft wrap with hanging indent.
    let mut wrap = match (v.wrap_lines, v.hanging_indent) {
        (false, _) => 0u8,
        (true, false) => 1,
        (true, true) => 2,
    };
    if f.popup(
        "The type of wrapping to be applied to lines in the editor that are too long to fit in the current window width.",
        "Line wrapping in the editor",
        &mut wrap,
        &opts(&[(0u8, "No wrapping"), (1, "Soft wrap"), (2, "Soft wrap with hanging indent")]),
    ) {
        v.wrap_lines = wrap > 0;
        v.hanging_indent = wrap == 2;
    }
    f.popup(
        "A hanging indent is a small blank space that's added to all but the first line when the text is too long to fit inside the line. This setting determines the width of the space used for hanging indent. It is valid when the line wrapping mode is chosen to be a soft wrap with hanging indent.",
        "Hanging indent size",
        &mut v.hanging_chars,
        &opts(&[(0u8, "Tab Width"), (2, "2 Characters"), (4, "4 Characters"), (8, "8 Characters")]),
    );
    f.checkbox(
        "Underline misspelled words in text and comments, using the system spelling dictionary.",
        "Check spelling while typing",
        &mut v.spellcheck,
    );
}

fn look_and_feel(f: &mut Form<'_>, session: &mut Session) {
    let v = &mut session.view;
    f.popup(
        "The colours of the editor in the light appearance. Classic is cream paper with olive comments; Plain is white paper.",
        "Editor theme",
        &mut v.editor_theme,
        &opts(&[(EditorTheme::Classic, "Classic"), (EditorTheme::Plain, "Plain")]),
    );
    let mut size = v.editor_font_size.round() as u32;
    let sizes: Vec<(u32, String)> = [11u32, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24]
        .iter()
        .map(|s| (*s, format!("{s} pt")))
        .collect();
    if f.popup(
        "The size of the text in the editor.",
        "Editor font size",
        &mut size,
        &sizes,
    ) {
        v.editor_font_size = size as f32;
    }
}

fn viewer_pane(f: &mut Form<'_>, session: &mut Session) {
    let v = &mut session.view;
    f.checkbox(
        "Clicking in the PDF shows the matching line in the editor. When this is off, use ⌘-click.",
        "Jump to the source with a single click",
        &mut v.single_click_sync,
    );
    f.checkbox(
        "After each typeset, the PDF scrolls to the part of the document you are editing.",
        "Show the editing position after typesetting",
        &mut v.sync_after_typeset,
    );
    f.checkbox(
        "The PDF follows the caret as you move through the source.",
        "Scroll lock",
        &mut v.scroll_lock,
    );
    f.popup(
        "How pages are arranged in the document viewer.",
        "Page layout",
        &mut v.page_mode,
        &opts(&[
            (PageMode::SingleContinuous, "Single Page Continuous"),
            (PageMode::TwoContinuous, "Two Pages Continuous"),
            (PageMode::Single, "Single Page"),
        ]),
    );
    f.popup(
        "The zoom level of the document viewer.",
        "Zoom",
        &mut v.zoom,
        &opts(&[
            (Zoom::FitWidth, "Fit Width"),
            (Zoom::FitPage, "Fit Page"),
            (Zoom::Fixed(0.75), "75%"),
            (Zoom::Fixed(1.0), "100%"),
            (Zoom::Fixed(1.25), "125%"),
            (Zoom::Fixed(1.5), "150%"),
            (Zoom::Fixed(2.0), "200%"),
        ]),
    );
    f.popup(
        "In the dark appearance, TexCraft can re-tint the PDF so that pages are not bright white. The PDF file itself is not changed.",
        "Image grading in dark mode",
        &mut v.grading,
        &opts(&[(Grading::Entire, "Grade Entire Viewer Output"), (Grading::None, "No Grading")]),
    );
}

fn typesetting_pane(f: &mut Form<'_>, session: &mut Session) -> bool {
    let s = &mut session.settings;
    f.checkbox(
        "Save edited files before a full typeset. Live previews never save: they typeset the text in the editor.",
        "Save files before typesetting",
        &mut s.save_before_typeset,
    );
    f.popup(
        "How long the live preview waits after a keystroke. Immediately starts on the first keystroke and keeps up while you type.",
        "Live preview delay",
        &mut s.live_delay_ms,
        &opts(&[
            (0u32, "Immediately"),
            (100, "100 ms"),
            (250, "250 ms"),
            (500, "500 ms"),
            (1000, "1 second"),
        ]),
    );
    f.checkbox(
        "While the document has errors, the live preview keeps showing the last PDF that typeset cleanly.",
        "Keep the last good preview",
        &mut s.live_keep_good,
    );
    f.checkbox(
        "Store the preamble in a format file so that live previews only typeset the body (pdfLaTeX).",
        "Precompile the preamble",
        &mut s.cache_preamble,
    );
    f.button(
        "The typesetter, engine, bibliography and index settings of this document.",
        "Typeset Configuration…",
    )
}

// ---------------------------------------------------------------------------------------------
// Typeset Configuration sheet

/// Draws the Typeset Configuration sheet. Returns true when it should close.
pub fn typeset_sheet(ctx: &Context, t: &Tokens, session: &mut Session, prefs: &mut Prefs) -> bool {
    let pal = pal(t);
    let screen = ctx.content_rect();
    let w = SHEET_W.min(screen.width() - 32.0).max(480.0);
    let h = SHEET_H.min(screen.height() - 24.0).max(400.0);
    let frame = crate::menu::sheet_frame(t, 0);
    let mut close = false;
    egui::Modal::new(Id::new("typeset-config"))
        .frame(frame)
        .backdrop_color(crate::menu::backdrop())
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
            close = sheet_contents(ui, &pal, session, prefs, rect);
        });
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        close = true;
    }
    close
}

fn sheet_contents(
    ui: &mut Ui,
    pal: &Pal,
    session: &mut Session,
    prefs: &mut Prefs,
    rect: Rect,
) -> bool {
    let (sx, sy, w, h) = (rect.left(), rect.top(), rect.width(), rect.height());
    let heading = semibold(13.0);
    let p = ui.painter().clone();

    // ---- Typesetter: two columns.
    p.text(
        pos2(sx + 10.0, sy + 14.0),
        Align2::LEFT_CENTER,
        "Typesetter",
        heading.clone(),
        pal.label,
    );
    hairline(ui, sx, sx + w, sy + 27.0, pal.hairline);
    let col_w = w / 2.0;
    // The two columns show the mode: LIVE is the left one, AUTO and MANUAL the right one
    // (with Auto-Typeset on or off).
    let live = session.settings.mode() == TypesetMode::Live;
    for (i, kind) in [Typesetter::Live, Typesetter::External]
        .into_iter()
        .enumerate()
    {
        let cx = sx + 10.0 + col_w * i as f32 - if i == 1 { 10.0 } else { 0.0 };
        let cy = sy + 43.5;
        let title = if kind == Typesetter::Live {
            "Live Preview Typesetter"
        } else {
            "External Typesetter"
        };
        let tg = p.layout_no_wrap(title.to_string(), heading.clone(), pal.label);
        let tw = tg.size().x;
        p.galley(pos2(cx + 22.5, cy - tg.size().y / 2.0), tg, pal.label);
        radio(
            &p,
            pal,
            pos2(cx + 8.0, cy),
            live == (kind == Typesetter::Live),
        );
        let mut end = cx + 22.5 + tw;
        if kind == Typesetter::Live {
            let font = FontId::proportional(11.0);
            let bw = p
                .layout_no_wrap("LIVE".into(), font.clone(), pal.live)
                .size()
                .x
                + 8.0;
            let badge = Rect::from_min_size(pos2(end + 5.0, cy - 7.5), vec2(bw, 15.0));
            p.rect_filled(badge, 3.0, pal.live.gamma_multiply(0.08));
            p.rect_stroke(
                badge,
                3.0,
                Stroke::new(1.0, pal.live.gamma_multiply(0.8)),
                StrokeKind::Inside,
            );
            p.text(
                badge.center(),
                Align2::CENTER_CENTER,
                "LIVE",
                font,
                pal.live,
            );
            end = badge.right();
        }
        let info = pos2(end + 12.0, cy);
        let tip = if kind == Typesetter::Live {
            "TexCraft runs one fast pass of your TeX engine over the unsaved text after every change, in .texcraft/live. Bibliographies and indices update on a full typeset (⌘T)."
        } else {
            "latexmk (or tectonic) runs the full typeset — bibliography, index and reruns — on the saved files, like typesetting in a terminal."
        };
        ui.interact(
            Rect::from_center_size(info, vec2(14.0, 14.0)),
            Id::new(("ts-info", i)),
            Sense::hover(),
        )
        .on_hover_text(tip);
        icons::info_chevron(&p, info, 5.5, pal.note);
        // The radio and title select the typesetter.
        let hit = Rect::from_min_max(pos2(cx - 2.0, cy - 10.0), pos2(end + 2.0, cy + 10.0));
        if ui
            .interact(hit, Id::new(("ts-kind", i)), Sense::click())
            .clicked()
            && live != (kind == Typesetter::Live)
        {
            // The live typesetter exists to follow your typing; the external one starts with
            // Auto-Typeset on.
            session.settings.set_mode(if kind == Typesetter::Live {
                TypesetMode::Live
            } else {
                TypesetMode::Auto
            });
        }
        let sub = if kind == Typesetter::Live {
            "Realtime Typesetting · Lean · Unsaved Text"
        } else {
            "Traditional · Complete · Saved Files"
        };
        p.text(
            pos2(cx + 20.0, sy + 66.0),
            Align2::LEFT_CENTER,
            sub,
            FontId::proportional(12.0),
            pal.label,
        );
        if kind == Typesetter::Live {
            note(
                ui,
                pal,
                "TexCraft's live typesetter updates the preview as you type. It uses the TeX installation you already have; bibliographies update on a full typeset.",
                sy + 78.0,
                cx + 20.0,
                col_w - 44.0,
            );
        } else {
            note(
                ui,
                pal,
                "External typesetter, such as MacTeX.",
                sy + 78.0,
                cx + 20.0,
                col_w - 30.0,
            );
            let mut auto = session.settings.mode() == TypesetMode::Auto;
            if check_control(
                ui,
                pal,
                Id::new("ts-auto"),
                pos2(cx + 20.0, sy + 108.5),
                15.0,
                "Auto-Typeset",
                12.0,
                &mut auto,
                !live,
            ) {
                session.settings.typesetter = Typesetter::External;
                session.settings.set_mode(if auto {
                    TypesetMode::Auto
                } else {
                    TypesetMode::Manual
                });
            }
            note(
                ui,
                pal,
                "Automatically start a typeset when typing is paused.",
                sy + 118.5,
                cx + 20.0,
                col_w - 30.0,
            );
        }
    }
    hairline(ui, sx, sx + w, sy + 142.0, pal.hairline_strong);

    // ---- Typeset Configuration: Auto-Sense, then the tabs.
    p.text(
        pos2(sx + 10.0, sy + 158.0),
        Align2::LEFT_CENTER,
        "Typeset Configuration",
        heading,
        pal.label,
    );
    hairline(ui, sx, sx + w, sy + 172.0, pal.hairline);
    // Column split as measured: descriptions end at 413 pt, controls start at 418 (Auto-Sense) or 424.
    let split = sx + (w * 0.606).round();
    {
        let mut f = Form {
            ui: &mut *ui,
            pal,
            style: SHEET_STYLE,
            desc_color: pal.note,
            left: sx + 10.0,
            split: split - 11.0,
            ctrl: split - 6.0,
            right: sx + w - 33.0,
            y: sy + 180.5,
            id: Id::new("ts-sense"),
        };
        f.style.check_dy = 7.5;
        f.checkbox(
            "When this is enabled, TexCraft reads your document and picks the engine it needs (XeLaTeX for fontspec, ctex or Chinese text; LuaLaTeX for luacode) and sets up Chinese text. Disable it to choose manually.",
            "Auto-Sense",
            &mut session.settings.auto_sense,
        );
    }
    if let Some(i) = text_tabs(
        ui,
        pal,
        Id::new("ts-tabs"),
        sx + w / 2.0,
        sy + 248.5,
        23.0,
        93.0,
        &SHEET_TABS,
        prefs.sheet_tab,
        12.0,
    ) {
        prefs.sheet_tab = i;
    }
    hairline(ui, sx, sx + w, sy + 281.0, pal.hairline);
    {
        let mut f = Form {
            ui: &mut *ui,
            pal,
            style: SHEET_STYLE,
            desc_color: pal.note,
            left: sx + 20.0,
            split: split - 11.0,
            ctrl: split,
            right: sx + w - 33.0,
            y: sy + 292.0,
            id: Id::new(("ts-tab", prefs.sheet_tab)),
        };
        match prefs.sheet_tab {
            0 => summary_tab(&mut f, session),
            1 => general_tab(&mut f, session),
            2 => {
                f.popup(
                    "This is the engine used for the (optional) compilation of bibliography.",
                    "Bibliography Engine",
                    &mut session.settings.bib_tool,
                    &opts(&[
                        (BibTool::Auto, "Automatic (BibTeX or Biber)"),
                        (BibTool::None, "None"),
                    ]),
                );
            }
            _ => {
                let mut index = 0u8;
                f.popup(
                    "This is the program used to build the index when the document calls \\makeindex. latexmk runs it as needed during a full typeset.",
                    "Index Engine",
                    &mut index,
                    &opts(&[(0u8, "Automatic (makeindex)")]),
                );
            }
        }
    }

    // ---- Footer.
    hairline(ui, sx, sx + w, sy + h - 41.0, pal.hairline_strong);
    let close_r = Rect::from_min_size(pos2(sx + w - 99.0, sy + h - 30.5), vec2(87.0, 20.0));
    push_button(ui, pal, Id::new("ts-close"), close_r, "Close", 11.7)
        || ui.input(|i| i.key_pressed(Key::Enter))
}

/// Grey explanatory text under a typesetter, wrapped to `width`.
fn note(ui: &Ui, pal: &Pal, text: &str, top: f32, x: f32, width: f32) {
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: FontId::proportional(12.0),
            color: pal.note,
            line_height: Some(15.2),
            ..Default::default()
        },
    );
    job.wrap.max_width = width;
    let g = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().galley(pos2(x, top), g, pal.note);
}

fn summary_tab(f: &mut Form<'_>, session: &mut Session) {
    let plan = session.typeset_plan();
    let engine = plan.engine;
    let how = match plan.source {
        texcraft_project::EngineSource::MagicComment => " (from % !TEX program)",
        texcraft_project::EngineSource::AutoSense => " (Auto-Sense)",
        texcraft_project::EngineSource::Setting => "",
    };
    f.value(
        "The engine that typesets this document.",
        &format!("{}{how}", engine.label()),
    );
    if plan.add_cjk {
        let fonts = session.toolchain.cjk_fonts.as_ref().map_or_else(
            || "no Chinese fonts found".to_string(),
            |c| format!("{}, {}", c.serif, c.sans),
        );
        f.value(
            "Chinese text is set up with xeCJK; the file is not changed.",
            &fonts,
        );
    }
    let s = &session.settings;
    let typing = match s.mode() {
        TypesetMode::Live => "LIVE: live preview of the unsaved text",
        TypesetMode::Auto => "AUTO: full typeset when typing pauses",
        TypesetMode::Manual => "MANUAL: full typeset on ⌘T only",
    };
    f.value("What happens while you type.", typing);
    let tool = if engine == Engine::Tectonic {
        "tectonic"
    } else {
        "latexmk"
    };
    f.value(
        "A full typeset (⌘T) runs bibliography, index and reruns as needed.",
        &format!("{} via {tool}", engine.label()),
    );
    f.value(
        "How bibliographies are compiled.",
        if s.bib_tool == BibTool::Auto {
            "Automatically (BibTeX or Biber)"
        } else {
            "Never"
        },
    );
    let tc = session.toolchain.describe();
    f.value("The TeX programs TexCraft found.", &tc);
}

fn general_tab(f: &mut Form<'_>, session: &mut Session) {
    let tc = session.toolchain.clone();
    let sensed = session.settings.auto_sense;
    let engines: Vec<(Engine, String)> = Engine::ALL
        .iter()
        .map(|e| {
            let name = if tc.supports(*e) {
                e.label().to_string()
            } else {
                format!("{} — not installed", e.label())
            };
            (*e, name)
        })
        .collect();
    if sensed {
        // Auto-Sense picks the engine: show the one in use, locked.
        let mut used = session.effective_engine();
        f.popup_enabled(
            "Auto-Sense picks the engine from the document. Turn Auto-Sense off to choose it yourself.",
            "Typesetting Engine",
            &mut used,
            &engines,
            false,
        );
    } else {
        f.popup(
            "The engine used when the document has no % !TEX program comment.",
            "Typesetting Engine",
            &mut session.settings.engine,
            &engines,
        );
    }
    f.checkbox(
        "Lets packages such as minted and gnuplot run programs. Only enable it for documents you trust.",
        "Allow shell escape",
        &mut session.settings.shell_escape,
    );
}

// ---------------------------------------------------------------------------------------------
// AI: providers and their models.

/// State of the AI pane.
#[derive(Default)]
pub(crate) struct AiPaneState {
    /// The provider shown on the right.
    selected: usize,
    /// A new API key being typed (cleared once saved; a saved key is never shown again).
    key_input: String,
    /// Replacing a saved key.
    editing_key: bool,
    /// The model being edited (index into the provider's models), and its fields as typed.
    editing: Option<usize>,
    edit_name: String,
    edit_id: String,
    edit_context: String,
    /// The outcome of the last action (key saved, models added, …).
    message: Option<(String, bool)>,
    /// The model picker window (Add Models…).
    pub(crate) picker: crate::model_picker::ModelPicker,
}

/// Added models shown before their list scrolls, and their row height.
const MODEL_ROWS: usize = 6;
const MODEL_ROW_H: f32 = 26.0;

/// A rounded text field in the preference style.
pub(crate) fn text_field(
    ui: &mut Ui,
    pal: &Pal,
    id: Id,
    r: Rect,
    text: &mut String,
    password: bool,
    hint: &str,
) -> egui::Response {
    let focused = ui.memory(|m| m.has_focus(id));
    ui.painter().rect(
        r,
        5.5,
        pal.control,
        Stroke::new(
            if focused { 1.5 } else { 0.0 },
            if focused {
                pal.accent
            } else {
                Color32::TRANSPARENT
            },
        ),
        StrokeKind::Inside,
    );
    let inner = r.shrink2(vec2(9.0, 3.0));
    ui.put(
        inner,
        egui::TextEdit::singleline(text)
            .id(id)
            .password(password)
            .hint_text(hint)
            .font(FontId::proportional(12.0))
            .text_color(pal.control_text)
            .frame(egui::Frame::NONE)
            .margin(vec2(0.0, 2.0))
            .vertical_align(Align::Center),
    )
}

/// A bold field title.
fn field_title(ui: &Ui, pal: &Pal, x: f32, y: f32, title: &str) {
    ui.painter().text(
        pos2(x, y),
        Align2::LEFT_CENTER,
        title,
        semibold(12.0),
        pal.label,
    );
}

/// The editor of model `k` of `p` at (`x`, `y`), `w` wide: display name, id, context length and
/// capability chips. Returns its height, whether the model changed, and whether Done was pressed.
#[allow(clippy::too_many_arguments)]
fn model_editor(
    ui: &mut Ui,
    pal: &Pal,
    p: &mut texcraft_ai::Provider,
    k: usize,
    st: &mut AiPaneState,
    x: f32,
    top: f32,
    w: f32,
) -> (f32, bool, bool) {
    use texcraft_ai::Capability;
    let mut changed = false;
    let selected = st.selected;
    let id = move |s: &str| Id::new(("ai-model-edit", s, selected, k));
    let panel_top = top;
    let (x, w) = (x + 10.0, w - 20.0);
    let mut y = top + 10.0;
    // Name and id side by side.
    let half = (w - 10.0) / 2.0;
    field_title(ui, pal, x, y + 8.0, "Display Name");
    field_title(ui, pal, x + half + 10.0, y + 8.0, "Model ID");
    y += 18.0;
    let nr = Rect::from_min_size(pos2(x, y), vec2(half, 24.0));
    let hint = p.models.get(k).map(|m| m.id.clone()).unwrap_or_default();
    if text_field(ui, pal, id("name"), nr, &mut st.edit_name, false, &hint).changed()
        && let Some(m) = p.models.get_mut(k)
    {
        m.name = st.edit_name.trim().to_string();
        changed = true;
    }
    let ir = Rect::from_min_size(pos2(x + half + 10.0, y), vec2(half, 24.0));
    if text_field(ui, pal, id("id"), ir, &mut st.edit_id, false, "Model id").changed() {
        // A new id only when it is one and no other model has it.
        let new_id = st.edit_id.trim().to_string();
        let taken = p
            .models
            .iter()
            .enumerate()
            .any(|(j, m)| j != k && m.id == new_id);
        if !new_id.is_empty()
            && !taken
            && let Some(m) = p.models.get_mut(k)
        {
            m.id = new_id;
            changed = true;
        }
    }
    y += 32.0;
    // Context length.
    field_title(ui, pal, x, y + 8.0, "Context Length");
    y += 18.0;
    let cr = Rect::from_min_size(pos2(x, y), vec2(120.0, 24.0));
    if text_field(
        ui,
        pal,
        id("context"),
        cr,
        &mut st.edit_context,
        false,
        "e.g. 128K",
    )
    .changed()
        && let Some(m) = p.models.get_mut(k)
    {
        m.context_length = texcraft_ai::models::parse_context(&st.edit_context);
        changed = true;
    }
    let known = p.models.get(k).and_then(|m| m.context_length);
    ui.painter().text(
        pos2(cr.right() + 10.0, cr.center().y),
        Align2::LEFT_CENTER,
        match known {
            Some(n) => format!("{n} tokens"),
            None if st.edit_context.trim().is_empty() => "Unknown".to_string(),
            None => "Write a number of tokens, 128K or 1M".to_string(),
        },
        FontId::proportional(11.0),
        pal.note,
    );
    y += 32.0;
    // Capabilities: chips that switch on and off.
    field_title(ui, pal, x, y + 8.0, "Capabilities");
    y += 20.0;
    let mut cx = x;
    for cap in Capability::ALL {
        let on = p.models.get(k).is_some_and(|m| m.has(cap));
        let font = FontId::proportional(11.5);
        let g = ui
            .painter()
            .layout_no_wrap(cap.label().to_string(), font.clone(), pal.label);
        let cw = g.size().x + 22.0;
        if cx + cw > x + w {
            cx = x;
            y += 28.0;
        }
        let r = Rect::from_min_size(pos2(cx, y), vec2(cw, 22.0));
        let resp = ui
            .interact(r, id(cap.label()), Sense::click())
            .on_hover_text(match cap {
                Capability::Vision => "Reads images: the Assistant can attach them",
                Capability::Reasoning => "Reasons before it answers",
                Capability::ToolCalling => "Calls tools",
                Capability::Embedding => "Makes embeddings (not for chat)",
                Capability::Audio => "Hears or speaks (not for chat unless it does more)",
                Capability::ImageGen => "Makes images (not for chat unless it does more)",
            });
        let (fill, text) = if on {
            (pal.accent, Color32::WHITE)
        } else if resp.hovered() {
            (pal.control_hover, pal.label)
        } else {
            (pal.control, pal.label)
        };
        ui.painter().rect_filled(r, 11.0, fill);
        ui.painter()
            .text(r.center(), Align2::CENTER_CENTER, cap.label(), font, text);
        if resp.clicked()
            && let Some(m) = p.models.get_mut(k)
        {
            if on {
                m.capabilities.retain(|c| *c != cap);
            } else {
                m.capabilities.push(cap);
                m.capabilities.sort();
            }
            changed = true;
        }
        cx += cw + 6.0;
    }
    y += 30.0;
    let chats = p.models.get(k).is_none_or(texcraft_ai::Model::chats);
    let note = if chats {
        "With Vision the Assistant can attach images to questions."
    } else {
        "Not offered for chat: it only embeds, hears or speaks, or makes images."
    };
    ui.painter().text(
        pos2(x, y + 6.0),
        Align2::LEFT_CENTER,
        note,
        FontId::proportional(11.0),
        pal.note,
    );
    let dr = Rect::from_min_size(pos2(x + w - 70.0, y - 4.0), vec2(70.0, 22.0));
    let done = push_button(ui, pal, id("done"), dr, "Done", 11.5);
    y += 24.0;
    let h = y - panel_top;
    // The panel behind (painted after, under the controls' layer order is the same: a frame).
    ui.painter().rect_stroke(
        Rect::from_min_size(pos2(x - 10.0, panel_top), vec2(w + 20.0, h)),
        7.0,
        Stroke::new(0.5, pal.hairline_strong),
        StrokeKind::Inside,
    );
    (h, changed, done)
}

fn ai_pane(f: &mut Form<'_>, session: &mut Session, st: &mut AiPaneState) {
    let ui = &mut *f.ui;
    let pal = f.pal;
    let t = &pal.tokens;
    let top = f.y + 6.0;
    let left = f.left + 7.0;
    let right = f.right - 7.0;
    let mut changed = false;

    // ---- Provider list.
    let list = Rect::from_min_size(pos2(left, top), vec2(196.0, 262.0));
    ui.painter().rect(
        list,
        7.0,
        pal.bg,
        Stroke::new(0.5, pal.hairline_strong),
        StrokeKind::Inside,
    );
    let n = session.ai.config.providers.len();
    st.selected = st.selected.min(n.saturating_sub(1));
    if n == 0 {
        ui.painter().text(
            list.center(),
            Align2::CENTER_CENTER,
            "No Providers",
            FontId::proportional(12.0),
            pal.note,
        );
    }
    let mut y = list.top() + 5.0;
    for (i, p) in session.ai.config.providers.iter().enumerate() {
        let row = Rect::from_min_size(pos2(list.left() + 5.0, y), vec2(list.width() - 10.0, 36.0));
        y += 37.0;
        if row.bottom() > list.bottom() {
            break;
        }
        let resp = ui.interact(row, Id::new(("ai-provider", i)), Sense::click());
        let selected = i == st.selected;
        if selected {
            ui.painter().rect_filled(row, 6.0, pal.accent);
        } else if resp.hovered() {
            ui.painter().rect_filled(row, 6.0, pal.control);
        }
        let (fg, weak) = if selected {
            (Color32::WHITE, Color32::from_white_alpha(200))
        } else {
            (pal.label, pal.note)
        };
        let ready = p.has_key || texcraft_ai::is_local(p);
        let logo_r =
            Rect::from_center_size(pos2(row.left() + 13.0, row.center().y), vec2(18.0, 18.0));
        if selected {
            // A white plate keeps coloured logos legible on the accent.
            ui.painter()
                .rect_filled(logo_r.expand(2.0), 5.0, Color32::WHITE);
        }
        match crate::ai_logos::provider(p) {
            Some(key) => crate::ai_logos::paint(
                ui,
                key,
                logo_r,
                if selected { Color32::BLACK } else { pal.label },
            ),
            None => icons::sparkles(ui.painter(), logo_r, pal.accent, true),
        }
        // Status on the logo's corner: green ready, amber without a key, grey off.
        let dot = logo_r.right_bottom() + vec2(1.0, 0.0);
        ui.painter()
            .circle_filled(dot, 4.2, if selected { pal.accent } else { pal.bg });
        ui.painter().circle_filled(
            dot,
            3.0,
            if !p.enabled {
                t.menu_disabled
            } else if ready {
                t.success
            } else {
                t.git_modified
            },
        );
        let f1 = semibold(12.0);
        let name = crate::widgets::elide(ui.painter(), &p.name, &f1, row.width() - 38.0);
        ui.painter().text(
            pos2(row.left() + 30.0, row.top() + 12.0),
            Align2::LEFT_CENTER,
            name,
            f1,
            fg,
        );
        let sub = if !p.enabled {
            "Off".to_string()
        } else if !ready {
            "No API key".to_string()
        } else {
            format!(
                "{} model{}",
                p.models.len(),
                if p.models.len() == 1 { "" } else { "s" }
            )
        };
        ui.painter().text(
            pos2(row.left() + 30.0, row.top() + 26.0),
            Align2::LEFT_CENTER,
            sub,
            FontId::proportional(10.5),
            weak,
        );
        if resp.clicked() {
            st.selected = i;
            st.message = None;
            st.key_input.clear();
            st.editing_key = false;
            st.editing = None;
        }
    }
    // + / − under the list.
    let plus = Rect::from_min_size(pos2(list.left(), list.bottom() + 6.0), vec2(26.0, 22.0));
    let minus = Rect::from_min_size(pos2(plus.right(), plus.top()), vec2(26.0, 22.0));
    ui.painter().rect(
        plus.union(minus),
        5.0,
        pal.control,
        Stroke::NONE,
        StrokeKind::Inside,
    );
    ui.painter().line_segment(
        [
            pos2(plus.right(), plus.top() + 4.0),
            pos2(plus.right(), plus.bottom() - 4.0),
        ],
        Stroke::new(1.0, pal.hairline_strong),
    );
    let plus_resp = ui
        .interact(plus, Id::new("ai-add"), Sense::click())
        .on_hover_text("Add a Provider");
    icons::plus(ui.painter(), plus.shrink(5.0), pal.control_text);
    let minus_resp = ui
        .interact(minus, Id::new("ai-remove"), Sense::click())
        .on_hover_text("Remove the Provider");
    icons::minus(
        ui.painter(),
        minus.shrink(5.0),
        if n > 0 { pal.control_text } else { pal.note },
    );
    egui::Popup::menu(&plus_resp).show(|ui| {
        crate::menu::begin(ui, 230.0);
        crate::menu::heading(ui, t, "Add a Provider");
        for preset in texcraft_ai::PRESETS {
            let probe = texcraft_ai::Provider {
                id: preset.id.into(),
                base_url: preset.base_url.into(),
                ..Default::default()
            };
            let opts = crate::menu::Item {
                logo: crate::ai_logos::provider(&probe),
                logo_column: true,
                ..Default::default()
            };
            if crate::menu::item(ui, t, preset.name, opts).clicked() {
                st.selected = session.ai.config.add(preset);
                st.message = None;
                changed = true;
            }
        }
    });
    if minus_resp.clicked() && st.selected < n {
        // The provider's key goes with it.
        if let Err(e) = session.ai_remove_provider(st.selected) {
            st.message = Some((e.to_string(), false));
        }
        st.selected = st.selected.saturating_sub(1);
        st.message = None;
        st.key_input.clear();
        st.editing_key = false;
    }
    // Key and fetch actions wait until the provider is no longer borrowed.
    let mut save_key: Option<(String, String)> = None;
    let mut remove_key: Option<String> = None;
    let mut open_picker: Option<usize> = None;
    // A model's id changed: (provider, old id, new id), for the choices that name it.
    let mut renamed: Option<(String, String, String)> = None;

    // ---- The selected provider.
    let x = list.right() + 22.0;
    let w = right - x;
    let mut y = top + 4.0;
    let selected = st.selected;
    let id = move |k: &str| Id::new(("ai-field", k, selected));
    if let Some(p) = session.ai.config.providers.get_mut(st.selected) {
        field_title(ui, pal, x, y + 8.0, "Name");
        let r = Rect::from_min_size(pos2(x, y + 18.0), vec2(w, 24.0));
        changed |=
            text_field(ui, pal, id("name"), r, &mut p.name, false, "Provider name").changed();
        y += 54.0;

        field_title(ui, pal, x, y + 8.0, "API");
        let r = Rect::from_min_size(pos2(x, y + 18.0), vec2(w, 24.0));
        changed |= popup_control(
            ui,
            pal,
            id("kind"),
            r,
            &mut p.kind,
            &[
                (
                    texcraft_ai::Kind::Anthropic,
                    texcraft_ai::Kind::Anthropic.label().to_string(),
                ),
                (
                    texcraft_ai::Kind::OpenAi,
                    texcraft_ai::Kind::OpenAi.label().to_string(),
                ),
            ],
            12.0,
        );
        y += 54.0;

        field_title(ui, pal, x, y + 8.0, "Base URL");
        let r = Rect::from_min_size(pos2(x, y + 18.0), vec2(w, 24.0));
        let hint = if p.kind == texcraft_ai::Kind::Anthropic {
            "https://api.anthropic.com"
        } else {
            "https://…/v1"
        };
        changed |= text_field(ui, pal, id("url"), r, &mut p.base_url, false, hint).changed();
        y += 54.0;

        field_title(ui, pal, x, y + 8.0, "API Key");
        let r = Rect::from_min_size(pos2(x, y + 18.0), vec2(w, 24.0));
        if p.has_key && !st.editing_key {
            // A saved key is never shown or copied out again: only replaced or removed.
            let bw = 70.0;
            let field = r.with_max_x(r.right() - 2.0 * (bw + 6.0));
            ui.painter().rect_filled(field, 5.5, pal.control);
            icons::check_mark(
                ui.painter(),
                pos2(field.left() + 14.0, field.center().y),
                11.0,
                t.success,
            );
            ui.painter().text(
                pos2(field.left() + 26.0, field.center().y),
                Align2::LEFT_CENTER,
                "Saved, encrypted on this computer",
                FontId::proportional(12.0),
                pal.desc,
            );
            let cr = Rect::from_min_size(pos2(field.right() + 6.0, r.top()), vec2(bw, 24.0));
            if push_button(ui, pal, id("key-change"), cr, "Change…", 11.5) {
                st.editing_key = true;
                st.key_input.clear();
            }
            let rr = Rect::from_min_size(pos2(cr.right() + 6.0, r.top()), vec2(bw, 24.0));
            if push_button(ui, pal, id("key-remove"), rr, "Remove", 11.5) {
                remove_key = Some(p.id.clone());
            }
        } else {
            let bw = 60.0;
            let field = r.with_max_x(r.right() - bw - 6.0);
            let key_resp = text_field(
                ui,
                pal,
                id("key"),
                field,
                &mut st.key_input,
                true,
                "Paste the key from the provider's console",
            );
            let enter = key_resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let sr = Rect::from_min_size(pos2(field.right() + 6.0, r.top()), vec2(bw, 24.0));
            if (push_button(ui, pal, id("key-save"), sr, "Save", 11.5) || enter)
                && !st.key_input.trim().is_empty()
            {
                save_key = Some((p.id.clone(), std::mem::take(&mut st.key_input)));
                st.editing_key = false;
            }
        }
        y += 54.0;

        // Models: display name, id and capabilities, in a list that scrolls past MODEL_ROWS
        // rows. A click edits one below the list (name, id, context length, capabilities, as in
        // Argus); more come from the picker window.
        field_title(ui, pal, x, y + 8.0, "Models");
        y += 18.0;
        if st.editing.is_some_and(|k| k >= p.models.len()) {
            st.editing = None;
        }
        let mut remove = None;
        let mut edit = None;
        if p.models.is_empty() {
            ui.painter().text(
                pos2(x + 9.0, y + 10.0),
                Align2::LEFT_CENTER,
                "No models yet: add some from the provider's list.",
                FontId::proportional(11.5),
                pal.note,
            );
            y += 23.0;
        } else {
            let rows = p.models.len().min(MODEL_ROWS);
            let list_r = Rect::from_min_size(pos2(x, y), vec2(w, rows as f32 * MODEL_ROW_H));
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(list_r));
            c.set_clip_rect(list_r.intersect(ui.clip_rect()));
            c.spacing_mut().item_spacing.y = 0.0;
            // Rows stop short of the scroll bar.
            let room = if p.models.len() > MODEL_ROWS {
                16.0
            } else {
                0.0
            };
            let sel = st.selected;
            let editing = st.editing;
            egui::ScrollArea::vertical()
                .id_salt(("ai-models", sel))
                .auto_shrink([false, false])
                .show_rows(&mut c, MODEL_ROW_H, p.models.len(), |ui, range| {
                    for k in range {
                        let Some(m) = p.models.get(k) else { continue };
                        let rw = (ui.available_width() - room).max(0.0);
                        let (row, resp) =
                            ui.allocate_exact_size(vec2(rw, MODEL_ROW_H), Sense::click());
                        let row = row.shrink2(vec2(0.0, 1.0));
                        let on = editing == Some(k);
                        // Edit and remove at the right, while hovered or edited.
                        let xr = Rect::from_center_size(
                            pos2(row.right() - 11.0, row.center().y),
                            vec2(18.0, 18.0),
                        );
                        let er = Rect::from_center_size(
                            pos2(row.right() - 31.0, row.center().y),
                            vec2(18.0, 18.0),
                        );
                        let xresp = ui
                            .interact(xr, Id::new(("ai-model-x", sel, k)), Sense::click())
                            .on_hover_text("Remove");
                        let eresp = ui
                            .interact(er, Id::new(("ai-model-e", sel, k)), Sense::click())
                            .on_hover_text("Edit");
                        let hot = resp.hovered() || xresp.hovered() || eresp.hovered();
                        let painter = ui.painter();
                        if on {
                            painter.rect_filled(row, 5.0, pal.item_selected);
                        } else if hot {
                            painter.rect_filled(row, 5.0, pal.control);
                        }
                        let icons_w = if hot || on {
                            icons::close(painter, xr.shrink(4.5), pal.note);
                            icons::prefs_editor(painter, er.shrink(3.0), pal.note);
                            44.0
                        } else {
                            4.0
                        };
                        // Capability badges from the right.
                        let badge_font = FontId::proportional(9.5);
                        let mut bx = row.right() - icons_w;
                        let mut badges_w = 0.0;
                        for cap in m.capabilities.iter().rev() {
                            let g = painter.layout_no_wrap(
                                cap.label().to_string(),
                                badge_font.clone(),
                                pal.accent,
                            );
                            let bw = g.size().x + 10.0;
                            if badges_w + bw > row.width() * 0.45 {
                                break;
                            }
                            let br = Rect::from_min_size(
                                pos2(bx - bw, row.center().y - 8.0),
                                vec2(bw, 16.0),
                            );
                            painter.rect_filled(br, 8.0, pal.accent.gamma_multiply(0.12));
                            painter.galley(
                                pos2(br.left() + 5.0, br.center().y - g.size().y / 2.0),
                                g,
                                pal.accent,
                            );
                            bx -= bw + 4.0;
                            badges_w += bw + 4.0;
                        }
                        // The display name, then the id when it differs.
                        let text_room = (bx - row.left() - 14.0).max(0.0);
                        let label_font = FontId::proportional(12.0);
                        let label =
                            crate::widgets::elide(painter, m.label(), &label_font, text_room);
                        let lg = painter.layout_no_wrap(label, label_font, pal.label);
                        let lw = lg.size().x;
                        painter.galley(
                            pos2(row.left() + 9.0, row.center().y - lg.size().y / 2.0),
                            lg,
                            pal.label,
                        );
                        if !m.name.is_empty() && text_room - lw > 36.0 {
                            let id_font = crate::theme::editor_font(10.5);
                            let id_text = crate::widgets::elide(
                                painter,
                                &m.id,
                                &id_font,
                                text_room - lw - 8.0,
                            );
                            painter.text(
                                pos2(row.left() + 9.0 + lw + 8.0, row.center().y),
                                Align2::LEFT_CENTER,
                                id_text,
                                id_font,
                                pal.note,
                            );
                        }
                        if xresp.clicked() {
                            remove = Some(k);
                        } else if eresp.clicked() || resp.clicked() {
                            edit = Some(k);
                        }
                    }
                });
            y += list_r.height();
        }
        if let Some(k) = remove {
            p.models.remove(k);
            st.editing = None;
            changed = true;
        } else if let Some(k) = edit {
            if st.editing == Some(k) {
                st.editing = None;
            } else if let Some(m) = p.models.get(k) {
                st.editing = Some(k);
                st.edit_name = m.name.clone();
                st.edit_id = m.id.clone();
                st.edit_context = m
                    .context_length
                    .map(texcraft_ai::models::format_context)
                    .unwrap_or_default();
            }
        }
        y += 6.0;
        let ar = Rect::from_min_size(pos2(x, y), vec2(112.0, 24.0));
        if push_button(ui, pal, id("add-models"), ar, "Add Models…", 11.5) {
            open_picker = Some(st.selected);
        }
        y += 32.0;
        // The model being edited.
        if let Some(k) = st.editing {
            let before = p.models.get(k).map(|m| m.id.clone());
            let (h, edited, done) = model_editor(ui, pal, p, k, st, x, y, w);
            let after = p.models.get(k).map(|m| m.id.clone());
            if let (Some(old), Some(new)) = (before, after)
                && old != new
            {
                renamed = Some((p.id.clone(), old, new));
            }
            changed |= edited;
            if done {
                st.editing = None;
            }
            y += h + 10.0;
        }
        if let Some((msg, ok)) = &st.message {
            let g = ui.painter().layout(
                msg.clone(),
                FontId::proportional(11.0),
                if *ok { pal.note } else { pal.close },
                w,
            );
            let h = g.size().y;
            ui.painter().galley(pos2(x, y), g, pal.note);
            y += h + 6.0;
        }
        y += 6.0;
        let mut enabled = p.enabled;
        if check_control(
            ui,
            pal,
            id("enabled"),
            pos2(x, y + 8.0),
            15.0,
            "Show in the Assistant's model menu",
            12.0,
            &mut enabled,
            true,
        ) {
            p.enabled = enabled;
            changed = true;
        }
        y += 26.0;
    } else {
        let g = ui.painter().layout(
            "Add a provider with + (SoCLaaS at NUS, Anthropic, OpenAI, Google Gemini, OpenRouter, DeepSeek, Kimi, Qwen, MiMo, GLM, MiniMax, StepFun, MoleAPI, a local Ollama, or any OpenAI-compatible server), then paste its API key.".to_string(),
            FontId::proportional(12.0),
            pal.desc,
            w,
        );
        y += g.size().y;
        ui.painter().galley(pos2(x, top + 8.0), g, pal.desc);
    }

    if let Some((pid, key)) = save_key {
        match session.ai_set_key(&pid, &key) {
            Ok(()) => st.message = Some(("The key is saved, encrypted.".into(), true)),
            Err(e) => st.message = Some((e.to_string(), false)),
        }
    }
    if let Some(pid) = remove_key
        && let Err(e) = session.ai_remove_key(&pid)
    {
        st.message = Some((e.to_string(), false));
    }
    if let Some((pid, old, new)) = renamed {
        let c = &mut session.ai.config;
        if c.provider == pid && c.model == old {
            c.model = new.clone();
        }
        if c.inline.provider == pid && c.inline.model == old {
            c.inline.model = new;
        }
    }
    if let Some(i) = open_picker {
        st.picker.open(ui.ctx(), session, i);
        st.message = None;
    }

    // ---- Below both columns.
    let mut y = y.max(plus.bottom() + 12.0) + 6.0;
    hairline(ui, f.left, f.right, y, pal.hairline);
    y += 14.0;
    let mut include = session.ai.config.include_document;
    if check_control(
        ui,
        pal,
        Id::new("ai-include"),
        pos2(left, y + 8.0),
        15.0,
        "Send the open document with each question",
        12.0,
        &mut include,
        true,
    ) {
        session.ai.config.include_document = include;
        changed = true;
    }
    y += 24.0;
    f.y = y + 12.0;

    if changed && let Err(e) = session.save_ai_config() {
        st.message = Some((e.to_string(), false));
    }
}
