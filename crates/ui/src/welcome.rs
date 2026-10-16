//! The welcome window, laid out like the target's: a separate 780 × 480 pt window on the
//! system's translucent material. Left: the app icon, version, Documents / About / Licence tabs
//! and the create/open actions. Right: Recent Documents / Templates tabs over a list, and a filter
//! field. At launch without a document it is the only window (the main window waits hidden).

use egui::{
    Align2, Color32, Context, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui, ViewportBuilder,
    ViewportCommand, ViewportId, pos2, vec2,
};
use texcraft_engine::Session;
use texcraft_project::templates::TEMPLATES;

use crate::dialogs::DialogAction;
use crate::icons;
use crate::prefs::popup_control;
use crate::theme::{Tokens, semibold};

/// Title of the welcome window (hidden; it finds the native window).
pub const WELCOME_TITLE: &str = "Welcome to TexCraft";
const W: f32 = 780.0;
const H: f32 = 480.0;
/// Width of the left column.
const LEFT_W: f32 = 380.0;
const LEFT_TABS: [&str; 3] = ["Documents", "About", "Licence"];
const RIGHT_TABS: [&str; 2] = ["Recent Documents", "Templates"];

/// The welcome window's viewport.
pub fn welcome_viewport() -> ViewportId {
    ViewportId::from_hash_of("texcraft-welcome")
}

/// Welcome window state.
#[derive(Default)]
pub struct Welcome {
    /// The window is open.
    pub open: bool,
    /// Opened at launch: the main window stays hidden until the user picks something.
    pub launch: bool,
    /// Draw the contents inside the main window instead (control channel screenshots).
    pub inline: bool,
    /// Left tab (Documents, About, Licence).
    pub left_tab: usize,
    /// Right tab (Recent Documents, Templates).
    pub right_tab: usize,
    /// Text in the filter field.
    pub filter: String,
    /// The translucent material is in place (draw no background).
    glass: bool,
    /// Bring the window to the front on the next frame.
    focus: bool,
}

impl Welcome {
    /// Whether the window shows the system's translucent material.
    pub fn glass(&self) -> bool {
        self.glass
    }

    /// Opens the window; `launch`: as the only window at start-up.
    pub fn show_window(&mut self, launch: bool) {
        self.open = true;
        self.launch = launch;
        self.focus = true;
    }
}

/// What the user picked.
enum Pick {
    Open,
    OpenFolder,
    Done,
    /// The window's close button (or ⌘W).
    Close,
}

/// Shows the welcome window while it is open. Returns a platform action to perform.
/// Draws the welcome window while it is open. `update`: the update window's open flag when it is
/// to show on top of this window (at start-up, when the main window is hidden behind it).
pub fn show(
    ctx: &Context,
    t: &Tokens,
    session: &mut Session,
    w: &mut Welcome,
    template: &mut usize,
    mut update: Option<&mut bool>,
) -> Option<DialogAction> {
    if !w.open {
        return None;
    }
    let mut pick = None;
    if w.inline {
        egui::Area::new(Id::new("welcome-inline"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(40.0, 60.0))
            .fade_in(false)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(W, H), Sense::hover());
                ui.painter().rect_filled(rect, 12.0, t.welcome_bg);
                ui.painter().rect_stroke(
                    rect,
                    12.0,
                    Stroke::new(1.0, t.welcome_divider),
                    StrokeKind::Outside,
                );
                // Stand-in for the window's close button.
                ui.painter().circle_filled(
                    pos2(rect.left() + 15.75, rect.top() + 16.0),
                    7.0,
                    t.error,
                );
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                c.set_clip_rect(rect);
                pick = body(&mut c, t, session, w, template, rect);
            });
    } else {
        let mut builder = ViewportBuilder::default()
            .with_title(WELCOME_TITLE)
            .with_inner_size([W, H])
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_transparent(true);
        if cfg!(target_os = "macos") {
            builder = builder
                .with_fullsize_content_view(true)
                .with_titlebar_shown(false)
                .with_title_shown(false);
        }
        ctx.show_viewport_immediate(welcome_viewport(), builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            if ctx.input(|i| i.viewport().close_requested()) {
                pick = Some(Pick::Close);
            }
            if w.focus {
                ctx.send_viewport_cmd(ViewportCommand::Focus);
                w.focus = false;
            }
            if cfg!(target_os = "macos") {
                let _ =
                    texcraft_macos::place_traffic_lights_titled(WELCOME_TITLE, 32.0, 8.75, true);
                w.glass = texcraft_macos::set_vibrancy_titled(WELCOME_TITLE);
            }
            let rect = ctx.content_rect();
            if !w.glass {
                ui.painter().rect_filled(rect, 0.0, t.welcome_bg);
            }
            if let Some(p) = body(ui, t, session, w, template, rect) {
                pick = Some(p);
            }
            if let Some(open) = update.as_deref_mut() {
                crate::update::show(&ctx, t, session, open);
            }
        });
    }
    let action = match pick? {
        Pick::Open => Some(DialogAction::Open),
        Pick::OpenFolder => Some(DialogAction::OpenFolder),
        Pick::Done => None,
        // Closing the start-up window (the only window) quits, as on macOS; opened later from
        // the menu, it just closes.
        Pick::Close if w.launch => Some(DialogAction::Quit),
        Pick::Close => None,
    };
    w.open = false;
    w.filter.clear();
    action
}

/// The app icon (assets/icon/texcraft.svg) as grey line-art, the way the target shows its icon
/// here: the tile, the tilted page with a sum and lines, and the backslash key in front.
pub(crate) fn icon(ui: &Ui, t: &Tokens, center: egui::Pos2, side: f32) {
    let p = ui.painter();
    let c = t.welcome_icon;
    // SVG coordinates: the tile spans 100..924.
    let s = side / 824.0;
    let at = |x: f32, y: f32| pos2(center.x + (x - 512.0) * s, center.y + (y - 512.0) * s);
    // The page is rotated by -6° about (470, 500).
    let (sin, cos) = (-6.0_f32).to_radians().sin_cos();
    let page = |x: f32, y: f32| {
        let (dx, dy) = (x - 470.0, y - 500.0);
        at(470.0 + dx * cos - dy * sin, 500.0 + dx * sin + dy * cos)
    };
    let key = Rect::from_min_max(at(590.0, 604.0), at(826.0, 840.0));
    // Page strokes hidden behind the key are left out (no fill to hide them on glass).
    let seg = |a: egui::Pos2, b: egui::Pos2, w: f32| {
        let n = 48;
        for k in 0..n {
            let (t0, t1) = (k as f32 / n as f32, (k + 1) as f32 / n as f32);
            let (q0, q1) = (a + (b - a) * t0, a + (b - a) * t1);
            if !key.expand(w).contains(q0 + (q1 - q0) * 0.5) {
                p.line_segment([q0, q1], Stroke::new(w, c));
            }
        }
        for q in [a, b] {
            if !key.expand(w).contains(q) {
                p.circle_filled(q, w / 2.0, c);
            }
        }
    };
    let poly = |pts: &[egui::Pos2], w: f32| {
        for pair in pts.windows(2) {
            if let [a, b] = pair {
                seg(*a, *b, w);
            }
        }
    };
    let w = 18.0 * s;
    p.rect_stroke(
        Rect::from_min_max(at(100.0, 100.0), at(924.0, 924.0)).shrink(2.0),
        186.0 * s,
        Stroke::new(4.0, c),
        StrokeKind::Inside,
    );
    let outline = [
        (250.0, 190.0),
        (610.0, 190.0),
        (700.0, 280.0),
        (700.0, 790.0),
        (250.0, 790.0),
        (250.0, 190.0),
    ];
    poly(&outline.map(|(x, y)| page(x, y)), w);
    poly(
        &[page(610.0, 190.0), page(610.0, 280.0), page(700.0, 280.0)],
        w,
    );
    seg(page(316.0, 264.0), page(516.0, 264.0), 32.0 * s);
    seg(page(300.0, 324.0), page(560.0, 324.0), 16.0 * s);
    let sum = [
        (472.0, 398.0),
        (326.0, 398.0),
        (408.0, 474.0),
        (326.0, 550.0),
        (472.0, 550.0),
    ];
    poly(&sum.map(|(x, y)| page(x, y)), 32.0 * s);
    seg(page(526.0, 436.0), page(618.0, 436.0), 20.0 * s);
    seg(page(512.0, 474.0), page(634.0, 474.0), 12.0 * s);
    seg(page(530.0, 512.0), page(614.0, 512.0), 18.0 * s);
    for (y, x1) in [(616.0, 652.0), (658.0, 610.0), (700.0, 512.0)] {
        seg(page(300.0, y), page(x1, y), 16.0 * s);
    }
    p.rect_stroke(key, 46.0 * s, Stroke::new(w, c), StrokeKind::Inside);
    p.rect_stroke(
        Rect::from_min_max(at(612.0, 620.0), at(804.0, 800.0)),
        34.0 * s,
        Stroke::new(10.0 * s, c),
        StrokeKind::Inside,
    );
    p.line_segment(
        [at(664.0, 656.0), at(752.0, 764.0)],
        Stroke::new(30.0 * s, c),
    );
    for q in [at(664.0, 656.0), at(752.0, 764.0)] {
        p.circle_filled(q, 15.0 * s, c);
    }
}

/// Text tabs centred on `centers`; the selected one in an accent box. Returns the clicked one.
#[allow(clippy::too_many_arguments)]
fn tabs(
    ui: &mut Ui,
    t: &Tokens,
    id: &str,
    names: &[&str],
    centers: &[f32],
    cy: f32,
    h: f32,
    selected: usize,
) -> Option<usize> {
    let font = FontId::proportional(11.75);
    let mut clicked = None;
    for (i, (name, cx)) in names.iter().zip(centers).enumerate() {
        let tw = ui
            .painter()
            .layout_no_wrap((*name).to_string(), font.clone(), t.form_label)
            .size()
            .x;
        let r = Rect::from_center_size(pos2(*cx, cy), vec2(tw + 22.0, h));
        let resp = ui.interact(r, Id::new((id, i)), Sense::click());
        let color = if i == selected {
            ui.painter().rect_filled(r, 5.0, t.accent);
            Color32::WHITE
        } else {
            if resp.hovered() {
                ui.painter().rect_filled(r, 5.0, t.welcome_hover);
            }
            t.form_label
        };
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            *name,
            font.clone(),
            color,
        );
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// A "+ Title / subtitle" action at `title_y`. Returns whether it was clicked.
fn action(
    ui: &mut Ui,
    t: &Tokens,
    id: &str,
    x0: f32,
    title_y: f32,
    title: &str,
    subtitle: &str,
) -> bool {
    let r = Rect::from_min_max(
        pos2(x0 + 12.0, title_y - 12.0),
        pos2(x0 + LEFT_W - 14.0, title_y + 26.0),
    );
    let resp = ui.interact(r, Id::new(("welcome-action", id)), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(r, 7.0, t.welcome_hover);
    }
    let c = pos2(x0 + 26.5, title_y + 4.5);
    p.line_segment(
        [c - vec2(6.5, 0.0), c + vec2(6.5, 0.0)],
        Stroke::new(1.3, t.form_label),
    );
    p.line_segment(
        [c - vec2(0.0, 6.5), c + vec2(0.0, 6.5)],
        Stroke::new(1.3, t.form_label),
    );
    p.text(
        pos2(x0 + 39.0, title_y),
        Align2::LEFT_CENTER,
        title,
        semibold(12.0),
        t.form_label,
    );
    p.text(
        pos2(x0 + 39.0, title_y + 15.5),
        Align2::LEFT_CENTER,
        subtitle,
        FontId::proportional(11.0),
        t.text_weak,
    );
    resp.clicked()
}

/// A wrapped paragraph at `pos`; returns its height.
fn paragraph(ui: &Ui, text: &str, pos: egui::Pos2, width: f32, size: f32, color: Color32) -> f32 {
    let g = ui
        .painter()
        .layout(text.to_string(), FontId::proportional(size), color, width);
    let h = g.size().y;
    ui.painter().galley(pos, g, color);
    h
}

fn body(
    ui: &mut Ui,
    t: &Tokens,
    session: &mut Session,
    w: &mut Welcome,
    template: &mut usize,
    rect: Rect,
) -> Option<Pick> {
    let (x0, y0) = (rect.left(), rect.top());
    let mut pick = None;
    // Dragging the background moves the window (there is no title bar).
    if !w.inline {
        let drag = ui.interact(rect, Id::new("welcome-drag"), Sense::click_and_drag());
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }

    // ---- Left column.
    let cx = x0 + LEFT_W / 2.0;
    icon(ui, t, pos2(cx, y0 + 95.0), 150.0);
    let os = if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Linux"
    };
    ui.painter().text(
        pos2(cx, y0 + 181.0),
        Align2::CENTER_CENTER,
        format!("TexCraft {os} v{}", env!("CARGO_PKG_VERSION")),
        FontId::proportional(11.0),
        t.form_label,
    );
    let left_centers = [cx - 79.25, cx, cx + 79.25];
    if let Some(i) = tabs(
        ui,
        t,
        "welcome-left",
        &LEFT_TABS,
        &left_centers,
        y0 + 225.0,
        24.5,
        w.left_tab,
    ) {
        w.left_tab = i;
    }
    match w.left_tab {
        0 => {
            if action(
                ui,
                t,
                "open",
                x0,
                y0 + 264.5,
                "Open a local file or project…",
                "Open an existing local file or project.",
            ) {
                pick = Some(Pick::Open);
            }
            ui.painter().text(
                pos2(x0 + 22.5, y0 + 317.0),
                Align2::LEFT_CENTER,
                "Template",
                FontId::proportional(11.75),
                t.text_weak,
            );
            let popup =
                Rect::from_min_max(pos2(x0 + 82.0, y0 + 305.0), pos2(x0 + 364.5, y0 + 329.0));
            let options: Vec<(usize, String)> = TEMPLATES
                .iter()
                .enumerate()
                .map(|(i, tp)| (i, tp.name.to_string()))
                .collect();
            let pal = crate::prefs::Pal {
                control: t.welcome_popup,
                control_hover: t.welcome_hover,
                ..crate::prefs::pal(t)
            };
            popup_control(
                ui,
                &pal,
                Id::new("welcome-template"),
                popup,
                template,
                &options,
                11.75,
            );
            if action(
                ui,
                t,
                "create",
                x0,
                y0 + 338.5,
                "Create a local file…",
                "Start a new document from the template.",
            ) {
                if let Some(tp) = TEMPLATES.get(*template) {
                    let _ =
                        session.run("file.newFromTemplate", &serde_json::json!({ "id": tp.id }));
                }
                pick = Some(Pick::Done);
            }
            if action(
                ui,
                t,
                "folder",
                x0,
                y0 + 388.5,
                "Open a project folder…",
                "Open a folder; TexCraft finds the root document.",
            ) {
                pick = Some(Pick::OpenFolder);
            }
        }
        1 => {
            let mut y = y0 + 258.0;
            for (text, size, color) in [
                (
                    "A native LaTeX editor written in Rust, with live preview, SyncTeX and an outline of your document.".to_string(),
                    12.0,
                    t.form_label,
                ),
                (format!("TeX programs found: {}", session.toolchain.describe()), 11.0, t.text_weak),
                (
                    "A clean-room re-creation of the Texifier workflow: no code or assets of the original are used.".to_string(),
                    11.0,
                    t.text_weak,
                ),
            ] {
                y += paragraph(ui, &text, pos2(x0 + 30.0, y), LEFT_W - 60.0, size, color) + 10.0;
            }
        }
        _ => {
            let mut y = y0 + 258.0;
            for (text, size, color) in [
                (
                    "TexCraft is free and open-source software, licensed under the MIT or Apache 2.0 licence, at your option.",
                    12.0,
                    t.form_label,
                ),
                (
                    "It is built on egui, wgpu, hayro and other open-source libraries; their licences are listed in ATTRIBUTION.md. Typesetting uses your TeX distribution.",
                    11.0,
                    t.text_weak,
                ),
            ] {
                y += paragraph(ui, text, pos2(x0 + 30.0, y), LEFT_W - 60.0, size, color) + 10.0;
            }
        }
    }

    // ---- Divider and right column.
    let side = Rect::from_min_max(pos2(x0 + LEFT_W, y0), rect.max);
    ui.painter().line_segment(
        [
            pos2(side.left() + 0.25, y0),
            pos2(side.left() + 0.25, rect.bottom()),
        ],
        Stroke::new(0.5, t.welcome_divider),
    );
    let rc = side.center().x;
    if let Some(i) = tabs(
        ui,
        t,
        "welcome-right",
        &RIGHT_TABS,
        &[rc - 60.5, rc + 60.5],
        y0 + 23.75,
        22.5,
        w.right_tab,
    ) {
        w.right_tab = i;
    }
    ui.painter().line_segment(
        [
            pos2(side.left(), y0 + 39.75),
            pos2(side.right(), y0 + 39.75),
        ],
        Stroke::new(0.5, t.welcome_divider),
    );
    let list = Rect::from_min_max(pos2(side.left(), y0 + 40.5), pos2(side.right(), y0 + 452.0));
    let filter = w.filter.trim().to_lowercase();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
    child.set_clip_rect(list);
    egui::ScrollArea::vertical()
        .id_salt(("welcome-list", w.right_tab))
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(3.5);
            if w.right_tab == 0 {
                if let Some(p) = recent_list(ui, t, session, &filter) {
                    pick = Some(p);
                }
            } else {
                for (i, tp) in TEMPLATES.iter().enumerate() {
                    let hit = |s: &str| s.to_lowercase().contains(&filter);
                    if !filter.is_empty() && !hit(tp.name) && !hit(tp.description) {
                        continue;
                    }
                    if list_row(ui, t, tp.name, tp.description, None) {
                        *template = i;
                        let _ = session
                            .run("file.newFromTemplate", &serde_json::json!({ "id": tp.id }));
                        pick = Some(Pick::Done);
                    }
                }
            }
            ui.add_space(6.0);
        });

    // ---- Filter field.
    let field = Rect::from_min_max(
        pos2(side.left() + 4.0, y0 + 455.0),
        pos2(side.right() - 3.5, y0 + 476.5),
    );
    let p = ui.painter();
    p.rect_filled(field, 4.0, t.filter_bg);
    p.rect_stroke(
        field,
        4.0,
        Stroke::new(0.5, t.filter_border),
        StrokeKind::Inside,
    );
    icons::funnel_circle(
        p,
        Rect::from_center_size(
            pos2(field.left() + 9.75, field.center().y),
            vec2(11.5, 11.5),
        ),
        t.text_weak,
    );
    let edit = Rect::from_min_max(
        pos2(field.left() + 19.0, field.top() + 1.0),
        pos2(field.right() - 4.0, field.bottom() - 1.0),
    );
    ui.put(
        edit,
        egui::TextEdit::singleline(&mut w.filter)
            .id(Id::new("welcome-filter"))
            .hint_text(egui::RichText::new("Filter").color(t.text_weak))
            .frame(egui::Frame::NONE)
            .font(FontId::proportional(10.75))
            .vertical_align(egui::Align::Center),
    );
    pick
}

/// The recent documents that still exist and match `filter`.
fn recent_list(ui: &mut Ui, t: &Tokens, session: &mut Session, filter: &str) -> Option<Pick> {
    let recent: Vec<std::path::PathBuf> = session
        .recent
        .iter()
        .filter(|p| p.exists())
        .filter(|p| filter.is_empty() || p.to_string_lossy().to_lowercase().contains(filter))
        .cloned()
        .collect();
    if recent.is_empty() {
        ui.add_space(150.0);
        ui.vertical_centered(|ui| {
            let text = if filter.is_empty() {
                "No Recent Documents"
            } else {
                "No Matches"
            };
            ui.label(egui::RichText::new(text).size(13.0).color(t.text_weak));
        });
    }
    let mut pick = None;
    for path in &recent {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Generic names get their folder, as in "Week12/main.tex".
        let same = recent
            .iter()
            .filter(|q| q.file_name() == path.file_name())
            .count()
            > 1;
        let title = match path.parent().and_then(|d| d.file_name()) {
            Some(dir) if same || name == "main.tex" => format!("{}/{name}", dir.to_string_lossy()),
            _ => name,
        };
        let when = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .map(|m| format!("Last Modified on {}", crate::clock::date_time(m)))
            .unwrap_or_default();
        if list_row(ui, t, &title, &when, Some(&path.display().to_string())) {
            if let Err(e) = session.open_path(path) {
                session.set_status(e.to_string());
            }
            pick = Some(Pick::Done);
        }
    }
    pick
}

/// One list row: document icon, title and grey subtitle. Returns whether it was clicked.
fn list_row(ui: &mut Ui, t: &Tokens, title: &str, subtitle: &str, tip: Option<&str>) -> bool {
    let (r, mut resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    if let Some(tip) = tip {
        resp = resp.on_hover_text(tip);
    }
    let p = ui.painter();
    let band = Rect::from_min_max(
        pos2(r.left() + 6.0, r.top() + 2.0),
        pos2(r.right() - 18.0, r.bottom() - 2.0),
    );
    if resp.hovered() {
        p.rect_filled(band, 6.0, t.welcome_hover);
    }
    icons::tex_document(
        p,
        Rect::from_center_size(pos2(r.left() + 20.75, r.top() + 22.5), vec2(16.0, 21.0)),
        t.text_weak,
    );
    let width = r.width() - 60.0;
    let tf = FontId::proportional(12.6);
    p.text(
        pos2(r.left() + 36.5, r.top() + 16.5),
        Align2::LEFT_CENTER,
        crate::widgets::elide(p, title, &tf, width),
        tf,
        t.form_label,
    );
    let sf = FontId::proportional(11.25);
    p.text(
        pos2(r.left() + 36.5, r.top() + 35.0),
        Align2::LEFT_CENTER,
        crate::widgets::elide(p, subtitle, &sf, width),
        sf,
        t.text_weak,
    );
    resp.clicked()
}
