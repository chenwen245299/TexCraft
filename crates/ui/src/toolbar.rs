//! The unified title bar + toolbar (`docs/texifier-reference.md` §2).

use egui::{Align, Color32, Rect, Sense, Stroke, Ui, ViewportCommand, pos2, vec2};
use texcraft_engine::{Layout, Session};
use texcraft_i18n::{tr, trf};
use texcraft_project::TypesetMode;
use texcraft_syntax::OutlineKind;

use crate::theme::{TOOLBAR_HEIGHT, TRAFFIC_LIGHTS_WIDTH, Tokens};
use crate::{icons, menu, widgets};

fn run(session: &mut Session, id: &str) {
    if let Err(e) = session.run(id, &serde_json::Value::Null) {
        session.set_status(e.to_string());
    }
}

/// The engine's summary of a full typeset (its status message), in English.
const TYPESET_SUMMARY: &str = "Typeset in {seconds} s — {errors}, {warnings}";

/// Whether a status message is the summary of a full typeset ("Typeset in 0.4 s — …"), in
/// English or in the interface language (the text before the first placeholder).
fn is_typeset_summary(msg: &str) -> bool {
    let prefix = |template: &'static str| template.split('{').next().unwrap_or("");
    [prefix(TYPESET_SUMMARY), prefix(tr(TYPESET_SUMMARY))]
        .iter()
        .any(|p| !p.trim().is_empty() && msg.starts_with(p))
}

/// Short description of the tool chain the next typeset runs, e.g. `pdfLaTeX > Bibliography`.
pub fn chain_label(session: &Session) -> String {
    let mut s = session.effective_engine().label().to_string();
    let has_bib = session.index.files.iter().any(|f| {
        f.outline
            .iter()
            .any(|i| i.kind == OutlineKind::Bibliography)
    });
    if has_bib {
        s.push_str(" > ");
        s.push_str(tr("Bibliography"));
    }
    s
}

/// Height of the toolbar capsules (pt).
const CAPSULE_H: f32 = 36.0;

fn capsule_shape(rect: Rect, t: &Tokens) -> egui::Shape {
    let radius = rect.height() / 2.0;
    let shadow = egui::Shape::rect_filled(
        rect.translate(vec2(0.0, 1.0)).expand(0.5),
        radius,
        Color32::from_black_alpha(if t.dark { 70 } else { 9 }),
    );
    let body = egui::Shape::Rect(egui::epaint::RectShape::new(
        rect,
        radius,
        t.capsule_bg,
        Stroke::new(0.5, t.capsule_border),
        egui::StrokeKind::Inside,
    ));
    egui::Shape::Vec(vec![shadow, body])
}

/// A clickable area with a hover tint; returns the response.
fn hit(
    ui: &mut Ui,
    t: &Tokens,
    rect: Rect,
    id: &str,
    tip: &str,
    enabled: bool,
    radius: f32,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let resp = ui.interact(rect, ui.id().with(id), sense);
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, radius, t.row_hover);
    }
    if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    }
}

/// A capsule separator: 16 pt tall, half a point wide (measured on the target).
fn vbar(ui: &Ui, t: &Tokens, x: f32, cy: f32) {
    ui.painter().line_segment(
        [pos2(x, cy - 8.0), pos2(x, cy + 8.0)],
        Stroke::new(0.75, t.toolbar_separator),
    );
}

/// The toolbar's left edge margin with no window buttons beside it (full screen, other
/// systems): the right capsules' margin, mirrored.
pub const EDGE: f32 = 8.0;

/// Where the left capsule starts: after the macOS window buttons, or at the window's edge in
/// full screen, where AppKit hides them. Entering full screen it slides there over a third of a
/// second; leaving it, it is back beside the buttons at once (AppKit runs its own animation).
pub fn left_inset(ctx: &egui::Context, traffic_lights: bool) -> f32 {
    let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
    let beside = traffic_lights && !full;
    let k = ctx.animate_bool_with_time_and_easing(
        egui::Id::new("toolbar-window-buttons"),
        beside,
        if beside { 0.0 } else { 0.35 },
        egui::emath::easing::cubic_in_out,
    );
    egui::lerp(EDGE..=TRAFFIC_LIGHTS_WIDTH, k)
}

/// Draws the toolbar into `rect`, its left capsule `left_inset` from the left edge (see
/// [`left_inset`]). Positions follow the target's toolbar measured at 2 px per pt.
pub fn show(ui: &mut Ui, t: &Tokens, session: &mut Session, rect: Rect, left_inset: f32) {
    let p = ui.painter().clone();
    p.rect_filled(rect, 0.0, t.toolbar_bg);
    p.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, t.divider),
    );

    // Background: drag to move the window, double-click to zoom.
    let bg = ui.interact(rect, ui.id().with("toolbar-drag"), Sense::click_and_drag());
    if bg.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if bg.double_clicked() {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }

    let cy = rect.center().y;
    let font = egui::FontId::proportional(11.75);
    let x0 = rect.left() + left_inset;

    // ---- Lay out the left capsule.
    // The gears turn for full typesets only: live-preview passes run all the time while typing.
    let running = session.typeset.is_full_running();
    let mode = session.settings.mode();
    let paused = session.live_paused && mode != TypesetMode::Manual;
    let auto_g = p.layout_no_wrap(mode.label().into(), font.clone(), Color32::PLACEHOLDER);
    let chain = chain_label(session);
    let mut parts = chain.split(" > ");
    let engine = parts.next().unwrap_or("").to_string();
    let rest: Vec<String> = parts.map(str::to_string).collect();
    let engine_g = p.layout_no_wrap(engine, font.clone(), t.chain_fg);
    let rest_g: Vec<_> = rest
        .iter()
        .map(|r| p.layout_no_wrap(r.clone(), font.clone(), t.chain_weak))
        .collect();

    let disc_x = x0 + 5.0;
    let gears_x = disc_x + 31.0 + 8.0;
    let auto_x = gears_x + 24.0 + 24.0;
    // The label slot is as wide as "AUTO" so the chain does not move when the label changes.
    let auto_w = p
        .layout_no_wrap("AUTO".into(), font.clone(), t.auto_label)
        .size()
        .x;
    let sep1 = auto_x + auto_g.size().x.max(auto_w) + 17.0;
    let mut chain_x = sep1 + 7.5;
    let chain_start = chain_x;
    chain_x += engine_g.size().x;
    for g in &rest_g {
        chain_x += 6.0 + 7.0 + 6.0 + g.size().x;
    }
    let sep2 = chain_x + 8.5;
    let sliders_x = sep2 + 8.5;
    let sep3 = sliders_x + 15.0 + 9.0;
    let check_x = sep3 + 8.5;
    let cap_end = check_x + 15.0 + 6.5;
    let cap = Rect::from_min_max(
        pos2(x0, cy - CAPSULE_H / 2.0),
        pos2(cap_end, cy + CAPSULE_H / 2.0),
    );
    p.add(capsule_shape(cap, t));

    // Sidebar toggle: blue disc when the sidebar shows.
    let disc = Rect::from_min_size(pos2(disc_x, cy - 15.5), vec2(31.0, 31.0));
    let shown = session.view.sidebar;
    let resp = ui
        .interact(disc, ui.id().with("tb-sidebar"), Sense::click())
        .on_hover_text(tr("Show / Hide Sidebar (⌘1)"));
    if shown {
        p.circle_filled(disc.center(), 15.5, t.accent);
    } else if resp.hovered() {
        p.circle_filled(disc.center(), 15.5, t.row_hover);
    }
    icons::sidebar_glyph(
        &p,
        Rect::from_center_size(disc.center(), vec2(17.0, 17.0)),
        if shown { Color32::WHITE } else { t.toolbar_fg },
    );
    if resp.clicked() {
        run(session, "view.toggleSidebar");
    }
    // Typeset gears (turn while a full typeset runs; click again to abort it).
    let gears = Rect::from_min_size(pos2(gears_x, cy - 12.0), vec2(24.0, 24.0));
    let tip = if running {
        tr("Abort Typesetting (⌘T)")
    } else {
        tr("Typeset (⌘T)")
    };
    let enabled = session.is_enabled("typeset.run");
    if hit(ui, t, gears.expand(3.0), "tb-gears", tip, enabled, 8.0).clicked() {
        run(session, "typeset.run");
    }
    let angle = if running {
        ui.ctx().request_repaint();
        ui.input(|i| i.time) as f32 * 1.6
    } else {
        0.0
    };
    icons::gears_pt(&p, gears.min, t.gear, t.capsule_bg, angle);
    // LIVE / AUTO / MANUAL: a menu of the three modes.
    let auto_rect = Rect::from_min_size(pos2(auto_x, cy - auto_g.size().y / 2.0), auto_g.size());
    let auto_color = if paused {
        t.chain_weak
    } else {
        match mode {
            TypesetMode::Live => t.success,
            TypesetMode::Auto => t.auto_label,
            TypesetMode::Manual => t.text_weak,
        }
    };
    let auto_tip = if paused {
        tr("Auto-typeset is paused (resume it in the PDF toolbar)").to_string()
    } else {
        match mode {
            TypesetMode::Live => {
                let speed = match (session.typeset.last_latency, session.typeset.last_live) {
                    (Some(l), _) => format!(
                        "\n{}",
                        trf!(
                            "Last update: {seconds} s from keystroke to PDF",
                            seconds = format!("{:.2}", l.as_secs_f32())
                        )
                    ),
                    (None, Some(d)) => format!(
                        "\n{}",
                        trf!(
                            "Last pass took {seconds} s",
                            seconds = format!("{:.2}", d.as_secs_f32())
                        )
                    ),
                    _ => String::new(),
                };
                format!(
                    "{}{speed}\n{}",
                    tr("LIVE: the PDF follows your typing without saving"),
                    tr("Click for AUTO")
                )
            }
            TypesetMode::Auto => format!(
                "{}\n{}",
                tr("AUTO: a full typeset starts when typing pauses"),
                tr("Click for MANUAL")
            ),
            TypesetMode::Manual => format!(
                "{}\n{}",
                tr("MANUAL: typeset with ⌘T"),
                tr("Click for LIVE")
            ),
        }
    };
    let mode_resp = ui
        .interact(
            auto_rect.expand2(vec2(8.0, 6.0)),
            ui.id().with("tb-auto"),
            Sense::click(),
        )
        .on_hover_text(auto_tip);
    if mode_resp.hovered() {
        p.rect_filled(auto_rect.expand2(vec2(8.0, 6.0)), 6.0, t.row_hover);
    }
    p.galley(auto_rect.min, auto_g.clone(), auto_color);
    // A click switches straight to the next mode.
    if mode_resp.clicked() {
        run(session, "typeset.cycleMode");
    }
    vbar(ui, t, sep1, cy);
    // Chain.
    let mut x = chain_start;
    let h = engine_g.size().y;
    p.galley(pos2(x, cy - h / 2.0), engine_g.clone(), t.chain_fg);
    x += engine_g.size().x;
    for g in rest_g {
        icons::chevron_thin(
            &p,
            Rect::from_min_size(pos2(x + 5.0, cy - 6.5), vec2(9.0, 13.0)),
            t.chain_fg,
            true,
        );
        x += 19.0;
        p.galley(pos2(x, cy - g.size().y / 2.0), g.clone(), t.chain_weak);
        x += g.size().x;
    }
    vbar(ui, t, sep2, cy);
    // Typeset configuration.
    let sl = Rect::from_min_size(pos2(sliders_x, cy - 8.0), vec2(15.0, 16.0));
    if hit(
        ui,
        t,
        sl.expand(5.0),
        "tb-sliders",
        tr("Typeset Configuration"),
        true,
        6.0,
    )
    .clicked()
    {
        run(session, "typeset.configuration");
    }
    icons::sliders_filled(&p, sl, t.toolbar_fg);
    vbar(ui, t, sep3, cy);
    // Status.
    let (errors, warnings, _) = session.issue_counts();
    let st = Rect::from_min_size(pos2(check_x, cy - 7.5), vec2(15.0, 15.0));
    let tip = if running {
        icons::spinner(&p, st, t.text_weak, ui.input(|i| i.time) as f32 * 6.0);
        ui.ctx().request_repaint();
        tr("Typesetting…").to_string()
    } else if errors > 0 {
        p.text(
            st.center(),
            egui::Align2::CENTER_CENTER,
            errors.to_string(),
            egui::FontId::proportional(14.0),
            t.error,
        );
        let errors_text = if errors == 1 {
            trf!("{n} error", n = errors)
        } else {
            trf!("{n} errors", n = errors)
        };
        let warnings_text = if warnings == 1 {
            trf!("{n} warning", n = warnings)
        } else {
            trf!("{n} warnings", n = warnings)
        };
        trf!(
            "{errors}, {warnings}",
            errors = errors_text,
            warnings = warnings_text
        )
    } else if session.typeset.last.is_some() {
        icons::check_thin(&p, st, t.success);
        // The last full typeset's summary ("Typeset in 0.4 s — 0 errors, 0 warnings").
        session
            .status
            .as_ref()
            .map(|(m, _)| m.as_str())
            .filter(|m| is_typeset_summary(m))
            .map_or_else(|| tr("Typeset successfully").to_string(), str::to_string)
    } else {
        icons::check_thin(&p, st, t.chain_weak);
        tr("Not typeset yet").to_string()
    };
    if hit(ui, t, st.expand(5.0), "tb-status", &tip, true, 6.0).clicked() {
        session.view.sidebar = true;
        session.view.sidebar_tab = texcraft_engine::SidebarTab::Results;
    }

    // Transient status message after the capsule (not the routine typeset summary: the
    // target's toolbar shows only the status icon, whose tooltip carries it).
    if let Some((msg, at)) = &session.status
        && at.elapsed().as_secs_f32() < 4.0
        && !is_typeset_summary(msg)
    {
        let x0 = cap_end + 14.0;
        let w = (rect.right() - 300.0 - x0).max(0.0);
        if w > 60.0 {
            let font = egui::FontId::proportional(11.5);
            let text = widgets::elide(&p, msg, &font, w);
            p.text(
                pos2(x0, cy),
                egui::Align2::LEFT_CENTER,
                text,
                font,
                t.text_weak,
            );
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(500));
    }

    // ---- Right capsules: share | view mode | back, forward, new tab.
    let nav_right = rect.right() - EDGE;
    let nav = Rect::from_min_max(
        pos2(nav_right - 92.5, cy - CAPSULE_H / 2.0),
        pos2(nav_right, cy + CAPSULE_H / 2.0),
    );
    let view = Rect::from_min_max(
        pos2(nav.left() - 7.5 - 127.0, nav.top()),
        pos2(nav.left() - 7.5, nav.bottom()),
    );
    let share = Rect::from_min_max(
        pos2(view.left() - 9.0 - 34.5, nav.top()),
        pos2(view.left() - 9.0, nav.bottom()),
    );
    for r in [share, view, nav] {
        p.add(capsule_shape(r, t));
    }
    let share_resp = hit(
        ui,
        t,
        share.shrink(3.0),
        "tb-share",
        tr("Export"),
        true,
        15.0,
    );
    icons::share_stacked(
        &p,
        share.center() + vec2(0.25, 0.25),
        t.toolbar_fg,
        t.capsule_bg,
    );
    egui::Popup::from_toggle_button_response(&share_resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            menu::begin(ui, 210.0);
            menu::heading(ui, t, tr("Export files"));
            menu::command(ui, t, session, "file.revealPdf", None, false);
            menu::command(ui, t, session, "file.exportPdf", None, false);
            menu::command(
                ui,
                t,
                session,
                "file.saveAs",
                Some(tr("Save Source As…")),
                false,
            );
        });
    // View mode: three segments; the selected one sits on a grey pill.
    let sel = match session.view.layout {
        Layout::EditorOnly => 0,
        Layout::Split => 1,
        Layout::PdfOnly => 2,
    };
    let centers = [
        view.left() + 19.25,
        view.left() + 63.5,
        view.left() + 107.75,
    ];
    let widths = [36.0, 48.0, 36.0];
    let tips = [
        tr("Editor Only (⌘2)"),
        tr("Editor and PDF (⌘3)"),
        tr("PDF Only (⌘4)"),
    ];
    let ids = ["view.editorOnly", "view.split", "view.pdfOnly"];
    for i in 0..3 {
        let seg = Rect::from_center_size(pos2(centers[i], cy), vec2(widths[i], 28.0));
        let resp = ui
            .interact(seg, ui.id().with(("tb-view", i)), Sense::click())
            .on_hover_text(tips[i]);
        if i == sel {
            p.rect_filled(seg, 14.0, t.segment_selected);
        } else if resp.hovered() {
            p.rect_filled(seg, 14.0, t.row_hover);
        }
        // Sizes and centres as measured on the target (19.5 × 15.5 pt glyphs).
        let ic = pos2(centers[i], cy + 0.25);
        match i {
            0 => icons::view_editor_only(&p, ic, t.toolbar_fg),
            1 => icons::view_split_pair(&p, ic, t.toolbar_fg),
            _ => icons::view_pdf_only(&p, ic, t.toolbar_fg),
        }
        if resp.clicked() {
            run(session, ids[i]);
        }
    }
    // Back, forward, new tab.
    let back_en = session.is_enabled("nav.back");
    let fwd_en = session.is_enabled("nav.forward");
    let back = Rect::from_center_size(pos2(nav.left() + 13.5, cy), vec2(20.0, 26.0));
    let fwd = Rect::from_center_size(pos2(nav.left() + 40.5, cy), vec2(20.0, 26.0));
    let newt = Rect::from_center_size(pos2(nav.left() + 75.5, cy), vec2(28.0, 26.0));
    if hit(ui, t, back, "tb-back", tr("Back"), back_en, 8.0).clicked() {
        run(session, "nav.back");
    }
    if hit(ui, t, fwd, "tb-fwd", tr("Forward"), fwd_en, 8.0).clicked() {
        run(session, "nav.forward");
    }
    if hit(ui, t, newt, "tb-new", tr("New Document (⌘N)"), true, 8.0).clicked() {
        run(session, "file.new");
    }
    let grey = |en: bool| if en { t.toolbar_fg } else { t.nav_disabled };
    icons::nav_chevron(&p, back.center() + vec2(0.0, 0.25), grey(back_en), false);
    icons::nav_chevron(&p, fwd.center() + vec2(0.0, 0.25), grey(fwd_en), true);
    icons::new_tab_rounded(
        &p,
        pos2(nav.left() + 71.0, cy + 1.15),
        t.toolbar_fg,
        t.capsule_bg,
    );
    let _ = Align::Center;
}

/// The tab strip shown under the toolbar when more than one document is open.
pub fn tabs(ui: &mut Ui, t: &Tokens, session: &mut Session, rect: Rect) {
    let p = ui.painter();
    p.rect_filled(
        rect,
        0.0,
        if t.dark {
            Color32::from_gray(30)
        } else {
            Color32::from_gray(222)
        },
    );
    p.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, t.divider),
    );
    let n = session.docs.len().max(1);
    let w = rect.width() / n as f32;
    let mut select = None;
    let mut close = None;
    for i in 0..n {
        let Some(doc) = session.docs.get(i) else {
            continue;
        };
        let r = Rect::from_min_size(
            pos2(rect.left() + w * i as f32, rect.top()),
            vec2(w, rect.height()),
        );
        let resp = ui.interact(r, ui.id().with(("tab", i)), Sense::click());
        let active = i == session.active;
        let p = ui.painter();
        if active {
            p.rect_filled(r, 0.0, t.toolbar_bg);
        } else if resp.hovered() {
            p.rect_filled(r, 0.0, t.row_hover);
        }
        if i > 0 {
            p.line_segment(
                [r.left_top(), r.left_bottom()],
                Stroke::new(1.0, t.divider.gamma_multiply(0.6)),
            );
        }
        let mut name = doc.display_name();
        if doc.is_dirty() && !doc.is_pristine() {
            name = format!("• {name}");
        }
        let font = egui::FontId::proportional(12.0);
        let text = widgets::elide(p, &name, &font, w - 40.0);
        p.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            text,
            font,
            if active { t.text } else { t.text_weak },
        );
        let cr = Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(14.0, 14.0));
        let cresp = ui.interact(cr, ui.id().with(("tab-close", i)), Sense::click());
        if resp.hovered() || cresp.hovered() {
            if cresp.hovered() {
                ui.painter().rect_filled(cr, 3.0, t.row_hover);
            }
            icons::close(ui.painter(), cr.shrink(2.0), t.text_weak);
        }
        if cresp.clicked() {
            close = Some(i);
        } else if resp.clicked() {
            select = Some(i);
        }
    }
    if let Some(i) = close {
        let _ = session.run("file.closeTab", &serde_json::json!({ "index": i }));
    } else if let Some(i) = select {
        session.active = i;
    }
}

/// Height of the toolbar.
pub fn height() -> f32 {
    TOOLBAR_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;
    use texcraft_i18n::Lang;

    /// The status icon and the transient message tell the typeset summary from other status
    /// messages in either language.
    #[test]
    fn typeset_summary_is_recognised_in_both_languages() {
        let summary = |lang| {
            let _scope = texcraft_i18n::scoped(lang);
            texcraft_i18n::fill(
                tr(TYPESET_SUMMARY),
                &[
                    ("seconds", &"0.4"),
                    ("errors", &"0 errors"),
                    ("warnings", &"1 warning"),
                ],
            )
        };
        let en = summary(Lang::En);
        let zh = summary(Lang::ZhHans);
        assert!(en.starts_with("Typeset in "));
        assert_ne!(en, zh);
        for lang in Lang::ALL {
            let _scope = texcraft_i18n::scoped(lang);
            assert!(is_typeset_summary(&en), "{lang:?}");
            assert!(!is_typeset_summary("Renamed to a.tex"));
            assert!(!is_typeset_summary(""));
        }
        let _zh = texcraft_i18n::scoped(Lang::ZhHans);
        assert!(is_typeset_summary(&zh), "{zh}");
    }
}
