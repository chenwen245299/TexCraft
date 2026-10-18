//! The sidebar's Source Control tab (where the target has its Connect cloud service): the
//! project's Git repository in the manner of VS Code's view — a message box and Commit button,
//! Staged Changes and Changes with per-file actions, recent commits — drawn in the sidebar's
//! own style (rows, group headers and badges of the other tabs).

use std::collections::HashSet;

use egui::{Align2, Color32, FontId, Id, Key, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use texcraft_engine::Session;
use texcraft_git::{Change, ChangeKind};
use texcraft_i18n::{tr, trf};

use crate::menu::{self, ButtonKind};
use crate::theme::{GROUP_ROW_HEIGHT, ROW_HEIGHT, SIDEBAR_FONT, Tokens, semibold};
use crate::{icons, widgets};

/// UI state of the Source Control tab.
#[derive(Default)]
pub struct GitPanelState {
    /// Groups the user closed.
    collapsed: HashSet<&'static str>,
    /// Discard waiting for confirmation: (path, untracked).
    confirm_discard: Option<(String, bool)>,
    /// The diff sheet is open.
    pub diff_open: bool,
}

fn run(session: &mut Session, id: &str, params: serde_json::Value) {
    if let Err(e) = session.run(id, &params) {
        session.git.error = Some(e.to_string());
    }
}

fn no_params() -> serde_json::Value {
    serde_json::Value::Null
}

/// The colour of a status letter (VS Code's: modified amber, added/untracked green, deleted
/// and conflicts red, renames blue).
fn kind_color(t: &Tokens, k: ChangeKind) -> Color32 {
    match k {
        ChangeKind::Modified | ChangeKind::TypeChanged => t.git_modified,
        ChangeKind::Added | ChangeKind::Untracked | ChangeKind::Copied => t.success,
        ChangeKind::Deleted | ChangeKind::Conflicted => t.error,
        ChangeKind::Renamed => t.accent,
    }
}

/// "3 min ago", "2 h ago", "4 days ago", else the date.
fn ago(time: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let d = now.saturating_sub(time).max(0);
    match d {
        0..60 => tr("just now").into(),
        60..3600 => trf!("{n} min ago", n = d / 60),
        3600..86_400 => trf!("{n} h ago", n = d / 3600),
        86_400..2_592_000 => {
            let days = d / 86_400;
            if days == 1 {
                trf!("{n} day ago", n = days)
            } else {
                trf!("{n} days ago", n = days)
            }
        }
        _ => {
            let when = crate::clock::date_time(
                std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(u64::try_from(time).unwrap_or(0)),
            );
            // The date without the time: "24 Sep 2026 at 16:21:54", "2026年9月24日 16:21:54".
            let date = when.split(" at ").next().unwrap_or("");
            match date.split_once("日 ") {
                Some((ymd, _)) => format!("{ymd}日"),
                None => date.to_string(),
            }
        }
    }
}

/// A centred message with an optional button (empty states).
fn empty(ui: &mut Ui, t: &Tokens, title: &str, text: &str, button: Option<&str>) -> bool {
    let mut clicked = false;
    ui.add_space(36.0);
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
        icons::git_branch(ui.painter(), r, t.sidebar_icon);
        ui.add_space(8.0);
        ui.label(
            RichText::new(title)
                .font(semibold(13.0))
                .color(t.sidebar_text),
        );
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(text).size(11.5).color(t.text_weak)).wrap());
        if let Some(label) = button {
            ui.add_space(12.0);
            let w = (ui.available_width() - 28.0).clamp(80.0, 200.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(r));
            clicked = menu::dialog_button(&mut c, t, label, ButtonKind::Default, w);
        }
    });
    clicked
}

/// A small icon button; returns whether it was clicked.
fn icon_button(
    ui: &mut Ui,
    t: &Tokens,
    r: Rect,
    id: Id,
    tip: &str,
    enabled: bool,
    draw: impl FnOnce(&egui::Painter, Rect, Color32),
) -> bool {
    let resp = ui.interact(
        r,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if enabled && resp.hovered() {
        ui.painter().rect_filled(r, 5.0, t.row_hover);
    }
    let c = if enabled {
        t.sidebar_icon
    } else {
        t.menu_disabled
    };
    draw(ui.painter(), r.shrink(3.5), c);
    resp.on_hover_text(tip).clicked()
}

/// Draws the Source Control tab.
pub fn show(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut GitPanelState) {
    dialogs(ui.ctx(), t, session, st);
    if session.git_dir().is_none() {
        empty(
            ui,
            t,
            tr("No Project Folder"),
            tr("Open a project folder to manage it with Git."),
            None,
        );
        return;
    }
    if session.git.available == Some(false) {
        empty(
            ui,
            t,
            tr("Git Is Not Installed"),
            tr(
                "Install Git (git-scm.com, or the Xcode Command Line Tools on macOS) to use Source Control.",
            ),
            None,
        );
        return;
    }
    let Some(repo) = session.git.repo.clone() else {
        if session.git.not_repo {
            if empty(
                ui,
                t,
                tr("No Repository"),
                tr(
                    "The project folder is not a Git repository yet. Initialize one to track the history of your document.",
                ),
                Some(tr("Initialize Repository")),
            ) {
                run(session, "git.init", no_params());
            }
            if let Some(e) = session.git.error.clone() {
                error_line(ui, t, &e);
            }
        } else {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                icons::spinner(
                    ui.painter(),
                    r,
                    t.text_weak,
                    ui.input(|i| i.time) as f32 * 6.0,
                );
                ui.ctx().request_repaint();
            });
        }
        return;
    };
    let status = &repo.status;

    // ---- Header: branch, ahead / behind, actions.
    let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let p = ui.painter().clone();
    icons::git_branch(
        &p,
        Rect::from_center_size(pos2(hr.left() + 17.0, hr.center().y), vec2(14.0, 14.0)),
        t.sidebar_icon,
    );
    let busy = session.git.busy;
    let buttons_w = 4.0 * 24.0 + 6.0;
    let branch = status
        .branch
        .clone()
        .or_else(|| {
            status
                .head
                .as_ref()
                .map(|h| trf!("{head} (detached)", head = h))
        })
        .unwrap_or_else(|| tr("No commits yet").into());
    let mut sync = String::new();
    if status.behind > 0 {
        sync.push_str(&format!("↓{} ", status.behind));
    }
    if status.ahead > 0 {
        sync.push_str(&format!("↑{}", status.ahead));
    }
    let sync = sync.trim().to_string();
    let sync_w = if sync.is_empty() {
        0.0
    } else {
        p.layout_no_wrap(sync.clone(), FontId::proportional(11.0), t.text_weak)
            .size()
            .x
            + 6.0
    };
    let font = semibold(12.0);
    let name = widgets::elide(
        &p,
        &branch,
        &font,
        (hr.width() - 30.0 - buttons_w - sync_w).max(20.0),
    );
    let name_w = p
        .layout_no_wrap(name.clone(), font.clone(), t.sidebar_text)
        .size()
        .x;
    p.text(
        pos2(hr.left() + 29.0, hr.center().y),
        Align2::LEFT_CENTER,
        name,
        font,
        t.sidebar_text,
    );
    // Progress of an action (pull, push, commit…) stays on this line, so nothing below moves.
    if let Some(label) = busy {
        let sx = hr.left() + 29.0 + name_w + 8.0;
        icons::spinner(
            &p,
            Rect::from_center_size(pos2(sx + 5.5, hr.center().y), vec2(11.0, 11.0)),
            t.text_weak,
            ui.input(|i| i.time) as f32 * 6.0,
        );
        let room = (hr.right() - 6.0 - 4.0 * 24.0 - sx - 18.0).max(0.0);
        let text = widgets::elide(&p, &format!("{label}…"), &FontId::proportional(11.0), room);
        p.text(
            pos2(sx + 16.0, hr.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(11.0),
            t.text_weak,
        );
        ui.ctx().request_repaint();
    } else if !sync.is_empty() {
        p.text(
            pos2(hr.left() + 29.0 + name_w + 6.0, hr.center().y),
            Align2::LEFT_CENTER,
            &sync,
            FontId::proportional(11.0),
            t.text_weak,
        );
    }
    let idle = busy.is_none();
    let mut x = hr.right() - 6.0 - 4.0 * 24.0;
    let mut next = || {
        let r = Rect::from_center_size(pos2(x + 12.0, hr.center().y), vec2(22.0, 22.0));
        x += 24.0;
        r
    };
    let (r_refresh, r_pull, r_push, r_more) = (next(), next(), next(), next());
    if icon_button(
        ui,
        t,
        r_refresh,
        Id::new("git-refresh"),
        tr("Refresh"),
        idle,
        icons::refresh,
    ) {
        run(session, "git.refresh", no_params());
    }
    let pull_tip = if status.behind == 1 {
        trf!("Pull {n} commit", n = status.behind)
    } else if status.behind > 0 {
        trf!("Pull {n} commits", n = status.behind)
    } else {
        tr("Pull").into()
    };
    if icon_button(
        ui,
        t,
        r_pull,
        Id::new("git-pull"),
        &pull_tip,
        idle,
        |p, r, c| icons::arrow_line(p, r, c, false),
    ) {
        run(session, "git.pull", no_params());
    }
    let push_tip = if status.ahead == 1 {
        trf!("Push {n} commit", n = status.ahead)
    } else if status.ahead > 0 {
        trf!("Push {n} commits", n = status.ahead)
    } else if status.upstream.is_none() {
        tr("Publish Branch").into()
    } else {
        tr("Push").into()
    };
    if icon_button(
        ui,
        t,
        r_push,
        Id::new("git-push"),
        &push_tip,
        idle,
        |p, r, c| icons::arrow_line(p, r, c, true),
    ) {
        run(session, "git.push", no_params());
    }
    let more = ui
        .interact(r_more, Id::new("git-more"), Sense::click())
        .on_hover_text(tr("More Actions"));
    if more.hovered() {
        p.rect_filled(r_more, 5.0, t.row_hover);
    }
    for k in [-1.0, 0.0, 1.0] {
        p.circle_filled(r_more.center() + vec2(4.5 * k, 0.0), 1.3, t.sidebar_icon);
    }
    egui::Popup::from_toggle_button_response(&more)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            menu::begin(ui, 190.0);
            for (id, label) in [
                ("git.pull", tr("Pull")),
                ("git.push", tr("Push")),
                ("git.fetch", tr("Fetch")),
            ] {
                let opts = menu::Item {
                    disabled: !idle,
                    ..Default::default()
                };
                if menu::item(ui, t, label, opts).clicked() {
                    run(session, id, no_params());
                }
            }
            menu::separator(ui, t);
            if menu::button(ui, t, tr("Stage All Changes")) {
                run(session, "git.stageAll", no_params());
            }
            if menu::button(ui, t, tr("Unstage All Changes")) {
                run(session, "git.unstageAll", no_params());
            }
            menu::separator(ui, t);
            let reveal = if cfg!(windows) {
                tr("Show Repository in Explorer")
            } else {
                tr("Reveal Repository in Finder")
            };
            if menu::button(ui, t, reveal) {
                run(
                    session,
                    "files.reveal",
                    serde_json::json!({ "path": repo.root.to_string_lossy() }),
                );
            }
        });

    // ---- Message box and Commit.
    ui.add_space(4.0);
    let w = ui.available_width();
    let inner = Rect::from_min_size(ui.cursor().min + vec2(10.0, 0.0), vec2(w - 20.0, 0.0));
    let branch_name = status.branch.clone().unwrap_or_else(|| "HEAD".into());
    let frame = egui::Frame::NONE
        .fill(t.filter_bg)
        .stroke(Stroke::new(0.5, t.filter_border))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(7, 5));
    let mut commit_now = false;
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(Rect::from_min_size(inner.min, vec2(inner.width(), 200.0))),
        |ui| {
            frame.show(ui, |ui| {
                let edit = ui.add(
                    egui::TextEdit::multiline(&mut session.git.message)
                        .id(Id::new("git-message"))
                        .hint_text(trf!(
                            "Message (⌘↩ to commit on “{branch}”)",
                            branch = branch_name
                        ))
                        .font(FontId::proportional(12.0))
                        .frame(egui::Frame::NONE)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                if edit.has_focus()
                    && ui.input(|i| i.key_pressed(Key::Enter) && i.modifiers.command)
                {
                    commit_now = true;
                }
            });
            ui.add_space(6.0);
            let label = if status.is_clean() {
                tr("Commit")
            } else if status.staged.is_empty() {
                tr("Commit All")
            } else {
                tr("Commit")
            };
            let kind = if status.is_clean() || session.git.message.trim().is_empty() {
                ButtonKind::Normal
            } else {
                ButtonKind::Default
            };
            if menu::dialog_button(ui, t, &format!("✓ {label}"), kind, ui.available_width()) {
                commit_now = true;
            }
        },
    );
    if commit_now {
        run(session, "git.commit", no_params());
    }
    if let Some(e) = session.git.error.clone() {
        error_line(ui, t, &e);
    }
    ui.add_space(6.0);
    widgets_hairline(ui, t);

    // ---- Groups. The overlay scroll bar gets its own room while the list scrolls, so it does
    // not cover the status letters and counts at the right.
    ui.spacing_mut().scroll.floating_allocated_width = 14.0;
    egui::ScrollArea::vertical()
        .id_salt("git-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            if !status.staged.is_empty() {
                let open = group_header(
                    ui,
                    t,
                    st,
                    "staged",
                    tr("Staged Changes"),
                    status.staged.len(),
                    Some((tr("Unstage All Changes"), false)),
                    session,
                );
                if open {
                    for c in &status.staged {
                        change_row(ui, t, session, st, c, true);
                    }
                }
            }
            let open = group_header(
                ui,
                t,
                st,
                "changes",
                tr("Changes"),
                status.changes.len(),
                (!status.changes.is_empty()).then_some((tr("Stage All Changes"), true)),
                session,
            );
            if open {
                if status.changes.is_empty() && status.staged.is_empty() {
                    let (r, _) = ui.allocate_exact_size(
                        vec2(ui.available_width(), ROW_HEIGHT),
                        Sense::hover(),
                    );
                    ui.painter().text(
                        pos2(r.left() + 30.0, r.center().y),
                        Align2::LEFT_CENTER,
                        tr("No changes"),
                        FontId::proportional(11.5),
                        t.text_weak,
                    );
                }
                for c in &status.changes {
                    change_row(ui, t, session, st, c, false);
                }
            }
            let open = group_header(
                ui,
                t,
                st,
                "commits",
                tr("Commits"),
                repo.log.len(),
                None,
                session,
            );
            if open {
                for c in &repo.log {
                    commit_row(ui, t, c);
                }
            }
            ui.add_space(12.0);
        });
}

fn widgets_hairline(ui: &mut Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment(
        [r.left_center(), r.right_center()],
        Stroke::new(1.0, t.divider),
    );
}

fn error_line(ui: &mut Ui, t: &Tokens, e: &str) {
    ui.add_space(6.0);
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(12, 0))
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(e).size(11.0).color(t.error)).wrap());
        });
}

/// A group header (Staged Changes, Changes, Commits): chevron, title, count badge, and a hover
/// action (`(tip, stage)`: stage or unstage everything). Returns whether the group is open.
#[allow(clippy::too_many_arguments)]
fn group_header(
    ui: &mut Ui,
    t: &Tokens,
    st: &mut GitPanelState,
    key: &'static str,
    title: &str,
    count: usize,
    action: Option<(&str, bool)>,
    session: &mut Session,
) -> bool {
    let open = !st.collapsed.contains(key);
    let (r, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), GROUP_ROW_HEIGHT), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(r.shrink2(vec2(4.0, 0.0)), 4.0, t.row_hover);
    }
    let chevron = Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(10.0, 12.0));
    if open {
        icons::chevron_down(p, chevron, t.text_weak);
    } else {
        icons::chevron_thin(p, chevron, t.text_weak, true);
    }
    p.text(
        pos2(r.left() + 27.0, r.center().y),
        Align2::LEFT_CENTER,
        title,
        semibold(SIDEBAR_FONT - 0.75),
        t.sidebar_text,
    );
    // Count badge, as in the Contents tab.
    let g = p.layout_no_wrap(count.to_string(), FontId::proportional(11.0), t.badge_text);
    let bw = (g.size().x + 10.0).max(22.0);
    let br = Rect::from_min_max(
        pos2(r.right() - 12.0 - bw, r.center().y - 8.0),
        pos2(r.right() - 12.0, r.center().y + 8.0),
    );
    p.rect_stroke(
        br,
        4.0,
        Stroke::new(0.5, t.badge_border),
        egui::StrokeKind::Inside,
    );
    p.galley(br.center() - g.size() / 2.0, g, t.badge_text);
    let mut acted = false;
    if let Some((tip, stage)) = action
        && resp.hovered()
    {
        let b = Rect::from_center_size(pos2(br.left() - 14.0, r.center().y), vec2(20.0, 20.0));
        let hit = ui
            .interact(b, Id::new(("git-group-action", key)), Sense::click())
            .on_hover_text(tip);
        if hit.hovered() {
            ui.painter().rect_filled(b, 5.0, t.row_hover);
        }
        let ir = b.shrink(4.0);
        if stage {
            icons::plus(ui.painter(), ir, t.sidebar_icon);
        } else {
            icons::minus(ui.painter(), ir, t.sidebar_icon);
        }
        if hit.clicked() {
            acted = true;
            run(
                session,
                if stage {
                    "git.stageAll"
                } else {
                    "git.unstageAll"
                },
                no_params(),
            );
        }
    }
    if resp.clicked() && !acted {
        if open {
            st.collapsed.insert(key);
        } else {
            st.collapsed.remove(key);
        }
    }
    open
}

/// One changed file: icon, name, folder (weak), status letter; on hover the actions (open file,
/// discard, stage / unstage). A click opens the changes.
fn change_row(
    ui: &mut Ui,
    t: &Tokens,
    session: &mut Session,
    st: &mut GitPanelState,
    c: &Change,
    staged: bool,
) {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let p = ui.painter().clone();
    let hovered = resp.hovered() || resp.context_menu_opened();
    if hovered {
        p.rect_filled(r.shrink2(vec2(4.0, 0.0)), 4.0, t.row_hover);
    }
    let (dir, name) = c.path.rsplit_once('/').unwrap_or(("", c.path.as_str()));
    let label = match c.kind {
        ChangeKind::Untracked | ChangeKind::Added => "NEW",
        _ => match name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()) {
            Some(e) if e == "tex" || e == "sty" || e == "cls" => "TEX",
            Some(e) if e == "bib" => "BIB",
            Some(e) if e == "pdf" => "PDF",
            Some(e) if ["png", "jpg", "jpeg", "eps", "svg"].contains(&e.as_str()) => "IMG",
            _ => "",
        },
    };
    let label = if label == "NEW" { "" } else { label };
    icons::document_pt(
        &p,
        Rect::from_min_size(pos2(r.left() + 29.0, r.center().y - 6.5), vec2(10.4, 13.0)),
        t.text_weak,
        label,
    );
    let letter_x = r.right() - 16.0;
    let actions_w = if hovered { 3.0 * 22.0 } else { 0.0 };
    let font = FontId::proportional(SIDEBAR_FONT);
    let x = r.left() + 45.0;
    let avail = (letter_x - 10.0 - actions_w - x).max(10.0);
    let name_text = widgets::elide(&p, name, &font, avail);
    let name_w = p
        .layout_no_wrap(name_text.clone(), font.clone(), t.sidebar_text)
        .size()
        .x;
    let struck = c.kind == ChangeKind::Deleted;
    p.text(
        pos2(x, r.center().y),
        Align2::LEFT_CENTER,
        &name_text,
        font,
        t.sidebar_text,
    );
    if struck {
        p.line_segment(
            [pos2(x, r.center().y), pos2(x + name_w, r.center().y)],
            Stroke::new(1.0, t.sidebar_text),
        );
    }
    if !dir.is_empty() && avail - name_w > 24.0 {
        let f = FontId::proportional(10.5);
        let d = widgets::elide(&p, dir, &f, avail - name_w - 6.0);
        p.text(
            pos2(x + name_w + 6.0, r.center().y + 0.5),
            Align2::LEFT_CENTER,
            d,
            f,
            t.text_weak,
        );
    }
    p.text(
        pos2(letter_x, r.center().y),
        Align2::CENTER_CENTER,
        c.kind.letter().to_string(),
        semibold(11.5),
        kind_color(t, c.kind),
    );
    let path = serde_json::json!({ "path": c.path });
    let abs = session
        .git
        .repo
        .as_ref()
        .map(|repo| repo.root.join(&c.path));
    let mut acted = false;
    if hovered {
        let mut bx = letter_x - 12.0 - 3.0 * 22.0;
        let mut slot = || {
            let b = Rect::from_min_size(pos2(bx, r.center().y - 10.0), vec2(20.0, 20.0));
            bx += 22.0;
            b
        };
        let (b_open, b_discard, b_stage) = (slot(), slot(), slot());
        let exists = abs.as_ref().is_some_and(|a| a.is_file());
        if icon_button(
            ui,
            t,
            b_open,
            Id::new(("git-open", &c.path, staged)),
            tr("Open File"),
            exists,
            |p, r, col| icons::document_pt(p, r.shrink2(vec2(1.5, 0.0)), col, ""),
        ) {
            acted = true;
            if let Some(a) = &abs
                && let Err(e) = session.open_path(a)
            {
                session.git.error = Some(e.to_string());
            }
        }
        if !staged
            && icon_button(
                ui,
                t,
                b_discard,
                Id::new(("git-discard", &c.path)),
                tr("Discard Changes"),
                true,
                icons::discard,
            )
        {
            acted = true;
            st.confirm_discard = Some((c.path.clone(), c.kind == ChangeKind::Untracked));
        }
        let (tip, id) = if staged {
            (tr("Unstage Changes"), "git.unstage")
        } else {
            (tr("Stage Changes"), "git.stage")
        };
        if icon_button(
            ui,
            t,
            b_stage,
            Id::new(("git-stage", &c.path, staged)),
            tip,
            true,
            |p, r, col| {
                if staged {
                    icons::minus(p, r, col);
                } else {
                    icons::plus(p, r, col);
                }
            },
        ) {
            acted = true;
            run(session, id, path.clone());
        }
    }
    let kind = c.kind.describe();
    let resp = resp.on_hover_text(match &c.orig {
        Some(o) => format!("{} ← {o} · {kind}", c.path),
        None => format!("{} · {kind}", c.path),
    });
    if resp.clicked() && !acted {
        run(
            session,
            "git.diff",
            serde_json::json!({ "path": c.path, "staged": staged }),
        );
    }
    resp.context_menu(|ui| {
        menu::begin(ui, 180.0);
        if menu::button(ui, t, tr("Open Changes")) {
            run(
                session,
                "git.diff",
                serde_json::json!({ "path": c.path, "staged": staged }),
            );
        }
        let exists = abs.as_ref().is_some_and(|a| a.is_file());
        let open = menu::Item {
            disabled: !exists,
            ..Default::default()
        };
        if menu::item(ui, t, tr("Open File"), open).clicked()
            && let Some(a) = &abs
            && let Err(e) = session.open_path(a)
        {
            session.git.error = Some(e.to_string());
        }
        menu::separator(ui, t);
        if staged {
            if menu::button(ui, t, tr("Unstage Changes")) {
                run(session, "git.unstage", path.clone());
            }
        } else {
            if menu::button(ui, t, tr("Stage Changes")) {
                run(session, "git.stage", path.clone());
            }
            if menu::button(ui, t, tr("Discard Changes")) {
                st.confirm_discard = Some((c.path.clone(), c.kind == ChangeKind::Untracked));
            }
        }
        if let Some(a) = &abs {
            menu::separator(ui, t);
            let reveal = if cfg!(windows) {
                tr("Show in Explorer")
            } else {
                tr("Reveal in Finder")
            };
            if menu::button(ui, t, reveal) {
                run(
                    session,
                    "files.reveal",
                    serde_json::json!({ "path": a.to_string_lossy() }),
                );
            }
        }
    });
}

/// One commit: subject, then "id · author · when".
fn commit_row(ui: &mut Ui, t: &Tokens, c: &texcraft_git::Commit) {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(r.shrink2(vec2(4.0, 0.0)), 4.0, t.row_hover);
    }
    p.circle_stroke(
        pos2(r.left() + 34.0, r.top() + 12.0),
        3.2,
        Stroke::new(1.3, t.text_weak),
    );
    let x = r.left() + 45.0;
    let font = FontId::proportional(SIDEBAR_FONT);
    let subject = widgets::elide(p, &c.subject, &font, r.right() - 12.0 - x);
    p.text(
        pos2(x, r.top() + 12.0),
        Align2::LEFT_CENTER,
        subject,
        font,
        t.sidebar_text,
    );
    let meta = format!("{} · {} · {}", c.hash, c.author, ago(c.time));
    let f = FontId::proportional(10.5);
    let meta = widgets::elide(p, &meta, &f, r.right() - 12.0 - x);
    p.text(
        pos2(x, r.top() + 26.5),
        Align2::LEFT_CENTER,
        meta,
        f,
        t.text_weak,
    );
    resp.on_hover_text(format!("{}\n{} · {}", c.subject, c.hash, c.author));
}

/// The confirmation before discarding, and the diff sheet.
fn dialogs(ctx: &egui::Context, t: &Tokens, session: &mut Session, st: &mut GitPanelState) {
    if let Some((path, untracked)) = st.confirm_discard.clone() {
        let mut close = false;
        let m = egui::Modal::new(Id::new("git-discard-confirm"))
            .frame(menu::sheet_frame(t, 20))
            .backdrop_color(menu::backdrop())
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    RichText::new(trf!("Discard the changes in “{path}”?", path = path))
                        .font(semibold(14.0))
                        .color(t.form_label),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(if untracked {
                        tr("The file is not in the repository yet: it will be moved to the Trash.")
                    } else {
                        tr(
                            "The file goes back to its last committed (or staged) version. This cannot be undone.",
                        )
                    })
                    .size(12.0)
                    .color(t.form_desc),
                );
                ui.add_space(16.0);
                match menu::dialog_buttons(
                    ui,
                    t,
                    &[
                        (tr("Cancel"), ButtonKind::Normal),
                        (tr("Discard"), ButtonKind::Default),
                    ],
                ) {
                    Some(0) => close = true,
                    Some(_) => {
                        run(session, "git.discard", serde_json::json!({ "path": path }));
                        close = true;
                    }
                    None => {}
                }
            });
        if m.should_close() || close {
            st.confirm_discard = None;
        }
    }
    if st.diff_open {
        let Some((title, text)) = session.git.diff.clone() else {
            st.diff_open = false;
            return;
        };
        let screen = ctx.content_rect();
        let w = (screen.width() - 80.0).clamp(420.0, 860.0);
        let h = (screen.height() - 120.0).clamp(240.0, 640.0);
        let mut close = false;
        let m = egui::Modal::new(Id::new("git-diff"))
            .frame(menu::sheet_frame(t, 18))
            .backdrop_color(menu::backdrop())
            .show(ctx, |ui| {
                ui.set_width(w);
                ui.label(
                    RichText::new(title.as_str())
                        .font(semibold(14.0))
                        .color(t.form_label),
                );
                ui.add_space(10.0);
                egui::Frame::NONE
                    .fill(t.editor_bg)
                    .stroke(Stroke::new(0.5, t.form_hairline_strong))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(0, 6))
                    .show(ui, |ui| {
                        egui::ScrollArea::both()
                            .max_height(h)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                                let font = crate::theme::editor_font(12.0);
                                for line in text.lines() {
                                    diff_line(ui, t, line, &font, w);
                                }
                            });
                    });
                ui.add_space(14.0);
                if menu::dialog_buttons(ui, t, &[(tr("Close"), ButtonKind::Default)]).is_some() {
                    close = true;
                }
            });
        if m.should_close() || close || ctx.input(|i| i.key_pressed(Key::Escape)) {
            st.diff_open = false;
        }
    }
}

fn diff_line(ui: &mut Ui, t: &Tokens, line: &str, font: &FontId, w: f32) {
    let (fill, color) = if line.starts_with("+++") || line.starts_with("---") {
        (Color32::TRANSPARENT, t.text_weak)
    } else if line.starts_with('+') {
        (t.success.gamma_multiply(0.16), t.editor_text)
    } else if line.starts_with('-') {
        (t.error.gamma_multiply(0.14), t.editor_text)
    } else if line.starts_with("@@") {
        (t.accent.gamma_multiply(0.08), t.accent)
    } else if line.starts_with("diff ")
        || line.starts_with("index ")
        || line.starts_with("new file")
    {
        (Color32::TRANSPARENT, t.text_weak)
    } else {
        (Color32::TRANSPARENT, t.editor_text)
    };
    let g = ui
        .painter()
        .layout_no_wrap(line.replace('\t', "    "), font.clone(), color);
    let (r, _) = ui.allocate_exact_size(
        vec2(g.size().x.max(w - 4.0) + 24.0, g.size().y + 2.0),
        Sense::hover(),
    );
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(r, 0.0, fill);
    }
    ui.painter()
        .galley(pos2(r.left() + 12.0, r.top() + 1.0), g, color);
}
