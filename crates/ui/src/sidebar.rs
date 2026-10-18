//! The sidebar: icon tabs (Contents, TODOs, File Browser, Typeset Results, Search) and their
//! panels, following the target's sidebar (`docs/texifier-reference.md` §3).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use egui::{Align, Color32, Id, Key, Rect, Sense, Stroke, Ui, pos2, vec2};
use texcraft_engine::{ContentsMode, ResultsMode, SearchMode, Session, SidebarTab};
use texcraft_i18n::{tr, trf};
use texcraft_log::Severity;
use texcraft_project::{FileKind, FileNode};
use texcraft_syntax::OutlineKind;

use crate::theme::{GROUP_ROW_HEIGHT, HEADER_HEIGHT, ROW_HEIGHT, SIDEBAR_FONT, Tokens};
use crate::{icons, menu, widgets};

/// One text-search hit.
#[derive(Debug, Clone)]
pub struct Hit {
    /// 1-based line.
    pub line: u32,
    /// The line's text (trimmed, elided later).
    pub context: String,
    /// Byte range of the match within `context`.
    pub range: std::ops::Range<usize>,
    /// Byte offset of the match in the file.
    pub offset: usize,
    /// Match length in bytes.
    pub len: usize,
}

/// Sidebar UI state.
#[derive(Default)]
pub struct SidebarState {
    filter: std::collections::HashMap<&'static str, String>,
    collapsed: HashSet<String>,
    /// Focus the search field next frame.
    pub focus_search: bool,
    search_query: String,
    replace_text: String,
    search_regex: bool,
    search_case: bool,
    /// Earlier searches, newest first (the options menu lists them).
    recent_searches: Vec<String>,
    results: Vec<(PathBuf, Vec<Hit>)>,
    searched: Option<String>,
    search_error: Option<String>,
    tag_query: String,
    tag_selected: usize,
    /// Deepest sectioning level shown in the outline (2 sections, 3 subsections, …).
    pub outline_depth: u8,
    /// Outline groups and folders the user opened (they start collapsed).
    expanded: HashSet<String>,
    /// The file being renamed in the Files list: its path, the edited name, focus pending.
    renaming: Option<(PathBuf, String, bool)>,
    /// The Source Control tab.
    pub git: crate::git_panel::GitPanelState,
    /// The AI Assistant tab.
    pub ai: crate::ai_panel::AiPanelState,
}

impl SidebarState {
    /// Starts renaming `path` in the Files list (a file or folder just created): its folders
    /// open so the row shows.
    pub fn begin_rename(&mut self, path: PathBuf) {
        let mut dir = path.parent();
        while let Some(d) = dir {
            self.expanded
                .insert(format!("file:{}", d.to_string_lossy()));
            dir = d.parent();
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.renaming = Some((path, name, true));
    }

    /// Deepest outline level shown (subsections unless chosen otherwise).
    pub fn depth(&self) -> u8 {
        if self.outline_depth == 0 {
            3
        } else {
            self.outline_depth
        }
    }
}

fn tab_key(tab: SidebarTab) -> &'static str {
    match tab {
        SidebarTab::Contents => "contents",
        SidebarTab::Todos => "todos",
        SidebarTab::Git => "git",
        SidebarTab::Assistant => "assistant",
        SidebarTab::Files => "files",
        SidebarTab::Results => "results",
        SidebarTab::Search => "search",
    }
}

/// Draws the sidebar into `rect`.
pub fn show(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut SidebarState, rect: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(rect, 0.0, t.sidebar_bg);
    // ---- Tab row (aligned with the editor and PDF headers): 24 pt apart, centred.
    let tabs = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_HEIGHT));
    p.line_segment(
        [tabs.left_bottom(), tabs.right_bottom()],
        Stroke::new(1.0, t.divider),
    );
    let n = SidebarTab::ALL.len() as f32;
    let first = rect.center().x - 24.0 * (n - 1.0) / 2.0;
    for (i, tab) in SidebarTab::ALL.iter().enumerate() {
        let c = pos2(first + 24.0 * i as f32, tabs.center().y);
        let cell = Rect::from_center_size(c, vec2(22.0, 22.0));
        let resp = ui
            .interact(cell, Id::new(("sidebar-tab", i)), Sense::click())
            .on_hover_text(tab.label());
        let selected = session.view.sidebar_tab == *tab;
        if selected {
            p.rect_filled(cell, 4.5, t.accent);
        } else if resp.hovered() {
            p.rect_filled(cell, 4.5, t.row_hover);
        }
        let col = if selected {
            Color32::WHITE
        } else {
            t.sidebar_icon
        };
        let ir = Rect::from_center_size(c, vec2(16.0, 16.0));
        match tab {
            SidebarTab::Contents => icons::viewfinder_doc(&p, ir, col),
            SidebarTab::Todos => icons::checkbox_checked(&p, ir, col),
            SidebarTab::Git => {
                icons::git_branch(&p, ir, col);
                // The number of changes, as a small badge.
                let n = session
                    .git
                    .repo
                    .as_ref()
                    .map_or(0, |r| r.status.staged.len() + r.status.changes.len());
                if n > 0 && !selected {
                    let text = if n > 99 {
                        "99+".to_string()
                    } else {
                        n.to_string()
                    };
                    let g = p.layout_no_wrap(text, egui::FontId::proportional(7.5), Color32::WHITE);
                    let bw = (g.size().x + 4.5).max(10.0);
                    let b =
                        Rect::from_center_size(cell.right_top() + vec2(-3.5, 3.5), vec2(bw, 10.0));
                    p.rect_filled(b.expand(1.0), 6.0, t.sidebar_bg);
                    p.rect_filled(b, 5.0, t.accent);
                    p.galley(b.center() - g.size() / 2.0, g, Color32::WHITE);
                }
            }
            SidebarTab::Assistant => icons::sparkles(&p, ir.expand(0.5), col, true),
            SidebarTab::Files => icons::folder_thin(&p, ir, col),
            SidebarTab::Results => {
                icons::warning(&p, ir, col, false);
                let (e, _, _) = session.issue_counts();
                if e > 0 && !selected {
                    p.circle_filled(cell.right_top() + vec2(-3.0, 3.0), 3.0, t.error);
                }
            }
            SidebarTab::Search => icons::magnifier(&p, ir, col, None),
        }
        if resp.clicked() {
            session.view.sidebar_tab = *tab;
        }
    }

    // ---- Bottom filter bar.
    let tab = session.view.sidebar_tab;
    let has_filter = matches!(
        tab,
        SidebarTab::Contents | SidebarTab::Todos | SidebarTab::Files
    );
    let bottom_h = if has_filter { 28.5 } else { 0.0 };
    let body = Rect::from_min_max(
        pos2(rect.left(), tabs.bottom() + 0.5),
        pos2(rect.right(), rect.bottom() - bottom_h),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(Align::Min)),
    );
    child.set_clip_rect(body);
    child.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let filter = st
        .filter
        .get(tab_key(tab))
        .cloned()
        .unwrap_or_default()
        .to_lowercase();
    match tab {
        SidebarTab::Contents => contents(&mut child, t, session, st, &filter),
        SidebarTab::Todos => todos(&mut child, t, session, &filter),
        SidebarTab::Git => crate::git_panel::show(&mut child, t, session, &mut st.git),
        SidebarTab::Assistant => crate::ai_panel::show(&mut child, t, session, &mut st.ai),
        SidebarTab::Files => files(&mut child, t, session, st, &filter),
        SidebarTab::Results => results(&mut child, t, session),
        SidebarTab::Search => search(&mut child, t, session, st),
    }
    if has_filter {
        let with_button = tab != SidebarTab::Todos;
        let field = Rect::from_min_max(
            pos2(rect.left() + 3.0, rect.bottom() - 3.5 - 21.5),
            pos2(
                rect.right() - if with_button { 22.5 } else { 3.0 },
                rect.bottom() - 3.5,
            ),
        );
        p.rect(
            field,
            6.0,
            t.filter_bg,
            Stroke::new(0.5, t.filter_border),
            egui::StrokeKind::Inside,
        );
        icons::funnel_circle(
            &p,
            Rect::from_center_size(pos2(field.left() + 9.5, field.center().y), vec2(13.0, 13.0)),
            t.badge_text,
        );
        let text = st.filter.entry(tab_key(tab)).or_default();
        let edit = Rect::from_min_max(
            pos2(field.left() + 20.0, field.top() + 1.0),
            pos2(field.right() - 4.0, field.bottom() - 1.0),
        );
        ui.put(
            edit,
            egui::TextEdit::singleline(text)
                .id(Id::new(("sidebar-filter", tab_key(tab))))
                .hint_text(egui::RichText::new(tr("Filter")).color(t.text_weak))
                .frame(egui::Frame::NONE)
                .font(egui::FontId::proportional(11.75))
                .vertical_align(Align::Center),
        );
        if with_button {
            let gear = Rect::from_min_size(
                pos2(rect.right() - 19.0, field.center().y - 8.0),
                vec2(16.0, 16.0),
            );
            let resp = ui.interact(
                gear.expand(3.0),
                Id::new(("sidebar-gear", tab_key(tab))),
                Sense::click(),
            );
            let tip = if tab == SidebarTab::Files {
                tr("New File")
            } else {
                tr("Choose the level of detail")
            };
            let resp = resp.on_hover_text(tip);
            icons::gear_solid(&p, gear, t.gear, t.sidebar_bg);
            if tab == SidebarTab::Files {
                egui::Popup::from_toggle_button_response(&resp)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                    .show(|ui| {
                        menu::begin(ui, 160.0);
                        if let Some(dir) = session.project_dir.clone() {
                            let dir = serde_json::json!({ "dir": dir.to_string_lossy() });
                            for (id, label) in [
                                ("files.newFolder", tr("Create a new folder")),
                                ("files.newFile", tr("Create a new file")),
                            ] {
                                if menu::button(ui, t, label)
                                    && let Err(e) = session.run(id, &dir)
                                {
                                    session.set_status(e.to_string());
                                }
                            }
                        }
                    });
            } else {
                egui::Popup::from_toggle_button_response(&resp)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                    .show(|ui| {
                        menu::begin(ui, 200.0);
                        menu::heading(ui, t, tr("Choose the level of detail"));
                        for (depth, label) in [
                            (3u8, tr("Show subsections")),
                            (4, tr("Show subsubsections")),
                            (5, tr("Show paragraphs")),
                        ] {
                            let on = st.depth() >= depth;
                            let opts = menu::Item {
                                checked: Some(on),
                                ..Default::default()
                            };
                            if menu::item(ui, t, label, opts).clicked() {
                                st.outline_depth = if on { depth - 1 } else { depth };
                            }
                        }
                    });
            }
        }
    }
}

fn section_header(
    ui: &mut Ui,
    t: &Tokens,
    st: &mut SidebarState,
    key: &str,
    title: &str,
    count: Option<usize>,
    icon: Option<fn(&egui::Painter, Rect, Color32)>,
) -> bool {
    // Outline groups start collapsed except the table of contents (like the target).
    let default_open = key == "toc" || key.starts_with("search:");
    let open = if default_open {
        !st.collapsed.contains(key)
    } else {
        st.expanded.contains(key)
    };
    let resp = widgets::list_row(
        ui,
        t,
        GROUP_ROW_HEIGHT,
        false,
        Id::new(("group", key)),
        |p, r| {
            let chevron =
                Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(10.0, 12.0));
            if open {
                icons::chevron_down(p, chevron, t.text_weak);
            } else {
                icons::chevron_thin(p, chevron, t.text_weak, true);
            }
            let mut x = r.left() + 26.0;
            if let Some(icon) = icon {
                icon(
                    p,
                    Rect::from_min_size(pos2(x, r.center().y - 9.0), vec2(18.0, 18.0)),
                    t.sidebar_text,
                );
                x = r.left() + 49.0;
            }
            let font = egui::FontId::proportional(SIDEBAR_FONT);
            let badge_w = if count.is_some() { 40.0 } else { 0.0 };
            let text = widgets::elide(p, title, &font, r.right() - x - badge_w - 8.0);
            p.text(
                pos2(x, r.center().y),
                egui::Align2::LEFT_CENTER,
                text,
                font,
                t.sidebar_text,
            );
            if let Some(n) = count {
                let br = Rect::from_min_max(
                    pos2(r.right() - 33.5 - 23.5, r.center().y - 8.75),
                    pos2(r.right() - 33.5, r.center().y + 8.75),
                );
                let g = p.layout_no_wrap(
                    n.to_string(),
                    egui::FontId::proportional(11.0),
                    t.badge_text,
                );
                let br = if g.size().x + 10.0 > br.width() {
                    br.with_min_x(br.right() - g.size().x - 10.0)
                } else {
                    br
                };
                p.rect_stroke(
                    br,
                    4.0,
                    Stroke::new(0.5, t.badge_border),
                    egui::StrokeKind::Inside,
                );
                p.galley(br.center() - g.size() / 2.0, g, t.badge_text);
            }
        },
    );
    if resp.clicked() {
        if default_open {
            if open {
                st.collapsed.insert(key.to_string());
            } else {
                st.collapsed.remove(key);
            }
        } else if open {
            st.expanded.remove(key);
        } else {
            st.expanded.insert(key.to_string());
        }
    }
    open
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn jump_and_sync(session: &mut Session, file: &Path, line: u32) {
    if let Err(e) = session.jump(file, line) {
        session.set_status(e.to_string());
        return;
    }
    session.forward_sync(true);
}

fn contents(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut SidebarState, filter: &str) {
    session.refresh_index(false);
    let files = session.index.files.iter().filter(|f| !f.missing).count();
    let words = session.index.words;
    // Files / Words: label left, value right, 20 pt rows.
    for (label, value) in [
        (tr("Files"), files.to_string()),
        (tr("Words"), words.to_string()),
    ] {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
        let font = egui::FontId::proportional(10.75);
        ui.painter().text(
            pos2(r.left() + 10.0, r.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            font.clone(),
            t.text_weak,
        );
        ui.painter().text(
            pos2(r.right() - 10.0, r.center().y),
            egui::Align2::RIGHT_CENTER,
            value,
            font,
            t.text_weak,
        );
    }
    let sep = |ui: &mut Ui| {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().line_segment(
            [r.left_center(), r.right_center()],
            Stroke::new(1.0, t.divider),
        );
    };
    sep(ui);
    ui.add_space(3.75);
    let sel = if session.view.contents_mode == ContentsMode::Structure {
        0
    } else {
        1
    };
    let w = ui.available_width();
    if let Some(i) = widgets::segmented_text(ui, t, &[tr("Structure"), tr("Subfiles")], sel, w) {
        session.view.contents_mode = if i == 0 {
            ContentsMode::Structure
        } else {
            ContentsMode::Subfiles
        };
    }
    ui.add_space(3.75);
    sep(ui);
    ui.add_space(10.5);
    egui::ScrollArea::vertical()
        .id_salt("contents-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            if session.view.contents_mode == ContentsMode::Structure {
                structure(ui, t, session, st, filter);
            } else {
                subfiles(ui, t, session, filter);
            }
        });
}

/// One sectioning entry in the outline tree.
struct TocNode {
    file: usize,
    item: usize,
    level: u8,
    children: Vec<usize>,
}

fn structure(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut SidebarState, filter: &str) {
    let idx = session.index.clone();
    let depth_limit = st.depth();
    let active_path = session
        .doc()
        .and_then(|d| d.typeset_path().map(texcraft_project::normalize));
    let caret_line = session
        .doc()
        .map_or(0, texcraft_engine::Document::caret_line);
    let current = active_path
        .as_deref()
        .and_then(|p| idx.section_at(p, caret_line));
    let matches = |s: &str| filter.is_empty() || s.to_lowercase().contains(filter);
    let mut jump: Option<(PathBuf, u32)> = None;

    // Section tree in reading order (bibliographies sit at section level).
    let mut nodes: Vec<TocNode> = Vec::new();
    let mut roots: Vec<usize> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for (fi, ii) in idx.reading_order() {
        let Some(item) = idx.files.get(fi).and_then(|f| f.outline.get(ii)) else {
            continue;
        };
        let level = match item.kind {
            OutlineKind::Bibliography => 2,
            k => match k.level() {
                Some(l) => l,
                None => continue,
            },
        };
        if level > depth_limit {
            continue;
        }
        while stack.last().is_some_and(|&n| nodes[n].level >= level) {
            stack.pop();
        }
        let id = nodes.len();
        nodes.push(TocNode {
            file: fi,
            item: ii,
            level,
            children: Vec::new(),
        });
        match stack.last() {
            Some(&parent) => nodes[parent].children.push(id),
            None => roots.push(id),
        }
        if item.kind != OutlineKind::Bibliography {
            stack.push(id);
        }
    }
    let section_count = nodes
        .iter()
        .filter(|n| idx.files[n.file].outline[n.item].kind.is_sectioning())
        .count();
    let root_path = idx.root.clone();
    let root_text = root_path
        .as_deref()
        .and_then(|p| session.text_of(p))
        .unwrap_or_default();
    let line_of = |needle: &str| -> Option<u32> {
        root_text
            .lines()
            .position(|l| l.split('%').next().unwrap_or("").contains(needle))
            .map(|i| u32::try_from(i + 1).unwrap_or(u32::MAX))
    };
    // Row drawing shared by every outline entry: icon at 41 pt (+11.5 pt per level), text 17 pt
    // to its right.
    let font = egui::FontId::proportional(SIDEBAR_FONT);
    let row = |ui: &mut Ui,
               id: Id,
               depth: usize,
               label: &str,
               draw: &dyn Fn(&egui::Painter, Rect),
               chevron: Option<bool>,
               current: bool| {
        widgets::list_row(ui, t, ROW_HEIGHT, false, id, |p, r| {
            let ix = r.left() + 41.0 + 11.5 * depth as f32;
            if let Some(open) = chevron {
                let c = Rect::from_center_size(pos2(ix - 13.0, r.center().y), vec2(10.0, 12.0));
                if open {
                    icons::chevron_down(p, c, t.text_weak);
                } else {
                    icons::chevron_thin(p, c, t.text_weak, true);
                }
            }
            draw(
                p,
                Rect::from_min_size(pos2(ix, r.center().y - 6.0), vec2(12.0, 12.0)),
            );
            let text = widgets::elide(p, label, &font, r.right() - ix - 17.0 - 14.0);
            p.text(
                pos2(ix + 17.0, r.center().y),
                egui::Align2::LEFT_CENTER,
                text,
                font.clone(),
                t.sidebar_text,
            );
            if current {
                p.rect_filled(
                    Rect::from_min_size(
                        pos2(r.right() - 7.0, r.top() + 3.0),
                        vec2(3.0, r.height() - 6.0),
                    ),
                    1.5,
                    t.accent,
                );
            }
        })
    };

    if section_header(
        ui,
        t,
        st,
        "toc",
        tr("Table of Contents"),
        Some(section_count),
        Some(icons::toc_list),
    ) {
        let hour = |p: &egui::Painter, r: Rect| {
            icons::hourglass(
                p,
                Rect::from_center_size(r.center(), vec2(12.0, 15.0)),
                t.sidebar_text,
            )
        };
        if let (Some(rp), Some(l)) = (&root_path, line_of("\\begin{document}"))
            && filter.is_empty()
            && row(
                ui,
                Id::new("toc-begin"),
                0,
                tr("Begin Document"),
                &hour,
                None,
                false,
            )
            .clicked()
        {
            jump = Some((rp.clone(), l));
        }
        let mut todo: Vec<(usize, usize)> = roots.iter().rev().map(|&n| (n, 0)).collect();
        while let Some((n, depth)) = todo.pop() {
            let node = &nodes[n];
            let Some(f) = idx.files.get(node.file) else {
                continue;
            };
            let Some(item) = f.outline.get(node.item) else {
                continue;
            };
            let key = format!("toc:{}:{}", f.path.display(), item.line);
            let has_children = !node.children.is_empty();
            let open = has_children && (!filter.is_empty() || !st.collapsed.contains(&key));
            let title = if item.kind == OutlineKind::Bibliography {
                trf!("Bibliography: {title}", title = item.title)
            } else if item.title.is_empty() {
                tr("(untitled)").to_string()
            } else {
                item.title.clone()
            };
            if matches(&title) || has_children {
                let kind = item.kind;
                let draw = move |p: &egui::Painter, r: Rect| {
                    let fill = t.sidebar_text;
                    match kind {
                        OutlineKind::Part => {
                            icons::letter_badge(p, r, fill, t.sidebar_bg, "P", false)
                        }
                        OutlineKind::Chapter => {
                            icons::letter_badge(p, r, fill, t.sidebar_bg, "C", false)
                        }
                        OutlineKind::Section => {
                            icons::letter_badge(p, r, fill, t.sidebar_bg, "S", false)
                        }
                        OutlineKind::Frame => {
                            icons::letter_badge(p, r, fill, t.sidebar_bg, "F", true)
                        }
                        OutlineKind::Bibliography => icons::book_circle(p, r.expand(1.0), fill),
                        _ => icons::letter_badge(p, r, fill, t.sidebar_bg, "S", true),
                    }
                };
                let is_current = current == Some((node.file, node.item));
                let resp = row(
                    ui,
                    Id::new(("toc", node.file, node.item)),
                    depth,
                    &title,
                    &draw,
                    has_children.then_some(open),
                    is_current,
                );
                let ix = resp.rect.left() + 41.0 + 11.5 * depth as f32;
                let on_chevron = resp.interact_pointer_pos().is_some_and(|p| p.x < ix - 3.0);
                if resp.clicked() && has_children && on_chevron {
                    if open {
                        st.collapsed.insert(key.clone());
                    } else {
                        st.collapsed.remove(&key);
                    }
                } else if resp.clicked() {
                    jump = Some((f.path.clone(), item.line));
                }
            }
            if open {
                for &c in node.children.iter().rev() {
                    todo.push((c, depth + 1));
                }
            }
        }
        let hour = |p: &egui::Painter, r: Rect| {
            icons::hourglass(
                p,
                Rect::from_center_size(r.center(), vec2(12.0, 15.0)),
                t.sidebar_text,
            )
        };
        if let (Some(rp), Some(l)) = (&root_path, line_of("\\end{document}"))
            && filter.is_empty()
            && row(
                ui,
                Id::new("toc-end"),
                0,
                tr("End Document"),
                &hour,
                None,
                false,
            )
            .clicked()
        {
            jump = Some((rp.clone(), l));
        }
    }
    // (key, title, one item's kind, kind, icon colour, icon)
    type Group<'a> = (
        &'a str,
        &'a str,
        &'a str,
        OutlineKind,
        Color32,
        fn(&egui::Painter, Rect, Color32),
    );
    let groups: [Group<'_>; 3] = [
        (
            "figures",
            tr("Figures"),
            tr("Figure"),
            OutlineKind::Figure,
            t.syn_math,
            icons::pictures,
        ),
        (
            "tables",
            tr("Tables"),
            tr("Table"),
            OutlineKind::Table,
            t.success,
            icons::table,
        ),
        (
            "labels",
            tr("Labels"),
            tr("Label"),
            OutlineKind::Label,
            t.syn_math,
            icons::bookmark_circle,
        ),
    ];
    for (key, title, singular, kind, color, icon) in groups {
        let items = idx.items(kind);
        if items.is_empty() {
            continue;
        }
        if section_header(ui, t, st, key, title, Some(items.len()), Some(icon)) {
            for (path, item) in items {
                let name = file_label(path);
                let label = match kind {
                    OutlineKind::Label => item.title.clone(),
                    _ if item.title.is_empty() => trf!(
                        "{kind} ({file}: {line})",
                        kind = singular,
                        file = name,
                        line = item.line
                    ),
                    _ => item.title.clone(),
                };
                if !matches(&label) {
                    continue;
                }
                let draw =
                    move |p: &egui::Painter, r: Rect| icons::square_marker(p, r.expand(1.5), color);
                let resp = row(
                    ui,
                    Id::new((key, path, item.line, item.offset)),
                    0,
                    &label,
                    &draw,
                    None,
                    false,
                );
                if resp
                    .on_hover_text(trf!("{file}, line {line}", file = name, line = item.line))
                    .clicked()
                {
                    jump = Some((path.to_path_buf(), item.line));
                }
            }
        }
    }
    // Citations: keys the document cites.
    if !idx.cites.is_empty()
        && section_header(
            ui,
            t,
            st,
            "citations",
            tr("Citations"),
            Some(idx.cites.len()),
            Some(icons::open_book),
        )
    {
        for (key, path, line) in &idx.cites {
            if !matches(key) {
                continue;
            }
            let entry = idx
                .bib
                .iter()
                .find(|(e, _)| &e.key == key)
                .map(|(e, _)| format!("{} — {}", e.author, e.title));
            let draw =
                move |p: &egui::Painter, r: Rect| icons::square_marker(p, r.expand(1.5), t.accent);
            let resp = row(ui, Id::new(("cite", key)), 0, key, &draw, None, false);
            let resp = match entry {
                Some(e) => resp.on_hover_text(e),
                None => resp.on_hover_text(tr("Not found in the bibliography")),
            };
            if resp.clicked() {
                jump = Some((path.clone(), *line));
            }
        }
    }
    if idx.files.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(tr("No structure yet"))
                    .size(13.0)
                    .color(t.text_weak),
            )
        });
    }
    if let Some((f, l)) = jump {
        jump_and_sync(session, &f, l);
    }
}

fn subfiles(ui: &mut Ui, t: &Tokens, session: &mut Session, filter: &str) {
    let idx = session.index.clone();
    let mut open: Option<PathBuf> = None;
    // Depth of each file in the inclusion tree.
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(p) = idx.files.get(i).and_then(|f| f.parent) {
            d += 1;
            i = p;
            if d > 32 {
                break;
            }
        }
        d
    };
    for (fi, f) in idx.files.iter().enumerate() {
        let name = file_label(&f.path);
        if !filter.is_empty() && !name.to_lowercase().contains(filter) {
            continue;
        }
        let indent = 12.0 + 14.0 * depth(fi) as f32;
        let resp = widgets::list_row(
            ui,
            t,
            ROW_HEIGHT,
            false,
            Id::new(("subfile", fi)),
            |p, r| {
                let ir = Rect::from_min_size(
                    pos2(r.left() + indent, r.center().y - 7.0),
                    vec2(14.0, 14.0),
                );
                if fi == 0 {
                    icons::root_dot(p, ir, t.accent);
                } else {
                    icons::document(p, ir, t.text_weak, "TEX");
                }
                let col = if f.missing { t.error } else { t.sidebar_text };
                p.text(
                    pos2(ir.right() + 6.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    &name,
                    egui::FontId::proportional(12.0),
                    col,
                );
                if fi == 0 {
                    widgets::pill(
                        p,
                        r.right_center() - vec2(10.0, 0.0),
                        tr("ROOT"),
                        t.text_weak,
                    );
                }
            },
        );
        if resp.clicked() && !f.missing {
            open = Some(f.path.clone());
        }
        for a in idx.assets.iter().filter(|a| a.parent == fi) {
            let aname = file_label(&a.path);
            if !filter.is_empty() && !aname.to_lowercase().contains(filter) {
                continue;
            }
            let kind = FileKind::of(&a.path);
            let label = match kind {
                FileKind::Image | FileKind::Pdf => "IMG",
                FileKind::Bib => "BIB",
                _ => "STY",
            };
            let resp = widgets::list_row(
                ui,
                t,
                ROW_HEIGHT,
                false,
                Id::new(("asset", fi, &a.path)),
                |p, r| {
                    let ir = Rect::from_min_size(
                        pos2(r.left() + indent + 14.0, r.center().y - 7.0),
                        vec2(14.0, 14.0),
                    );
                    icons::document(p, ir, t.text_weak, label);
                    let col = if a.exists { t.sidebar_text } else { t.error };
                    p.text(
                        pos2(ir.right() + 6.0, r.center().y),
                        egui::Align2::LEFT_CENTER,
                        &aname,
                        egui::FontId::proportional(12.0),
                        col,
                    );
                },
            );
            if resp.clicked() && a.exists && kind.is_editable() {
                open = Some(a.path.clone());
            }
        }
    }
    if let Some(p) = open
        && let Err(e) = session.open_path(&p)
    {
        session.set_status(e.to_string());
    }
}

fn todos(ui: &mut Ui, t: &Tokens, session: &mut Session, filter: &str) {
    session.refresh_index(false);
    let items: Vec<(PathBuf, u32, String)> = session
        .index
        .items(OutlineKind::Todo)
        .into_iter()
        .map(|(p, i)| (p.to_path_buf(), i.line, i.title.clone()))
        .filter(|(_, _, title)| filter.is_empty() || title.to_lowercase().contains(filter))
        .collect();
    ui.add_space(6.0);
    ui.vertical_centered(|ui| {
        let n = items.len();
        let count = if n == 1 {
            trf!("{n} TODO", n = n)
        } else {
            trf!("{n} TODOs", n = n)
        };
        ui.label(egui::RichText::new(count).size(11.5).color(t.text_weak));
    });
    ui.add_space(4.0);
    let mut jump = None;
    egui::ScrollArea::vertical()
        .id_salt("todos-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (path, line, title) in &items {
                let resp = widgets::list_row(
                    ui,
                    t,
                    ROW_HEIGHT,
                    false,
                    Id::new(("todo", path, line)),
                    |p, r| {
                        let ir = Rect::from_min_size(
                            pos2(r.left() + 12.0, r.center().y - 7.0),
                            vec2(14.0, 14.0),
                        );
                        icons::todo(p, ir, t.text_weak);
                        let font = egui::FontId::proportional(12.0);
                        let text = if title.is_empty() {
                            "TODO".to_string()
                        } else {
                            title.clone()
                        };
                        let text = widgets::elide(p, &text, &font, r.width() - 40.0);
                        p.text(
                            pos2(ir.right() + 6.0, r.center().y),
                            egui::Align2::LEFT_CENTER,
                            text,
                            font,
                            t.sidebar_text,
                        );
                    },
                );
                if resp
                    .on_hover_text(format!("{}:{line}", file_label(path)))
                    .clicked()
                {
                    jump = Some((path.clone(), *line));
                }
            }
        });
    if let Some((p, l)) = jump {
        jump_and_sync(session, &p, l);
    }
}

fn files(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut SidebarState, filter: &str) {
    let Some(project) = session.project_dir.clone() else {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(tr("No project folder"))
                    .size(13.0)
                    .color(t.text_weak),
            );
            if ui.button(tr("Open Folder…")).clicked() {
                let _ = session.run("file.openFolder", &serde_json::Value::Null);
            }
        });
        return;
    };
    let tree = session.file_tree.clone();
    let root = session.root_path().map(|p| texcraft_project::normalize(&p));
    let ctx = FileContext {
        accent: t.accent,
        root: root.clone(),
        output: root.as_deref().map(texcraft_project::published_pdf),
        temp: project.join(texcraft_project::TEMP_FOLDER),
        project: session
            .index
            .files
            .iter()
            .map(|f| f.path.clone())
            .chain(session.index.assets.iter().map(|a| a.path.clone()))
            .collect(),
        filter: filter.to_string(),
        has_doc: session.doc().is_some(),
    };
    let mut actions: Vec<FileAction> = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("files-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // The project folder heads the tree (no disclosure: it is always open).
            let name = project.file_name().map_or_else(
                || project.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let resp =
                widgets::list_row(ui, t, ROW_HEIGHT, false, Id::new("file-project"), |p, r| {
                    icons::folder_pt(
                        p,
                        Rect::from_min_size(
                            pos2(r.left() + 8.0, r.center().y - 5.75),
                            vec2(14.5, 11.5),
                        ),
                        t.folder,
                    );
                    let font = egui::FontId::proportional(SIDEBAR_FONT);
                    let text = widgets::elide(p, &name, &font, r.right() - r.left() - 27.0 - 14.0);
                    p.text(
                        pos2(r.left() + 27.0, r.center().y),
                        egui::Align2::LEFT_CENTER,
                        text,
                        font,
                        t.sidebar_text,
                    );
                });
            file_menu(&resp, t, &project, true, &ctx, &mut actions);
            file_rows(ui, t, &tree, 1, &ctx, st, &mut actions);
        });
    for action in actions {
        match action {
            FileAction::Open(p) => {
                if let Err(e) = session.open_path(&p) {
                    session.set_status(e.to_string());
                }
            }
            FileAction::Rename(p) => st.begin_rename(p),
            FileAction::Command(id, params) => {
                if let Err(e) = session.run(id, &params) {
                    session.set_status(e.to_string());
                }
            }
        }
    }
}

/// What the file rows need to know about the project.
struct FileContext {
    accent: Color32,
    root: Option<PathBuf>,
    output: Option<PathBuf>,
    temp: PathBuf,
    project: HashSet<PathBuf>,
    filter: String,
    has_doc: bool,
}

impl FileContext {
    /// The role badge of a row: ROOT, SUB (part of the project), OUTPUT (the PDF) or TEMP.
    fn role(&self, path: &Path) -> Option<&'static str> {
        let path = texcraft_project::normalize(path);
        if self.root.as_deref() == Some(path.as_path()) {
            Some(tr("ROOT"))
        } else if self.output.as_deref() == Some(path.as_path()) {
            Some(tr("OUTPUT"))
        } else if path == self.temp {
            Some(tr("TEMP"))
        } else if self.project.contains(&path) {
            Some(tr("SUB"))
        } else {
            None
        }
    }
}

enum FileAction {
    Open(PathBuf),
    Rename(PathBuf),
    Command(&'static str, serde_json::Value),
}

/// The Files context menu (the target's: Insert in the editor · Reveal in Finder, Copy Full
/// Path · Rename, Duplicate · Move to Trash).
fn file_menu(
    resp: &egui::Response,
    t: &Tokens,
    path: &Path,
    is_project: bool,
    ctx: &FileContext,
    actions: &mut Vec<FileAction>,
) {
    let arg = serde_json::json!({ "path": path.to_string_lossy() });
    let is_dir = is_project || path.is_dir();
    if resp.context_menu_opened() {
        // The row the menu is for gets a focus ring, as in the target.
        resp.ctx.layer_painter(resp.layer_id).rect_stroke(
            resp.rect.shrink2(vec2(5.0, 0.5)),
            5.0,
            Stroke::new(2.0, ctx.accent),
            egui::StrokeKind::Inside,
        );
    }
    resp.context_menu(|ui| {
        menu::begin(ui, 160.0);
        let mut item = |ui: &mut Ui, label: &str, action: FileAction| {
            if menu::button(ui, t, label) {
                actions.push(action);
            }
        };
        // The target's menus: files start with "Insert in the editor", folders with
        // "Create a new folder / file".
        if is_dir {
            let dir = serde_json::json!({ "dir": path.to_string_lossy() });
            item(
                ui,
                tr("Create a new folder"),
                FileAction::Command("files.newFolder", dir.clone()),
            );
            item(
                ui,
                tr("Create a new file"),
                FileAction::Command("files.newFile", dir),
            );
        } else if ctx.has_doc {
            item(
                ui,
                tr("Insert in the editor"),
                FileAction::Command("files.insert", arg.clone()),
            );
        } else {
            let disabled = menu::Item {
                disabled: true,
                ..Default::default()
            };
            menu::item(ui, t, tr("Insert in the editor"), disabled);
        }
        menu::separator(ui, t);
        let reveal = if cfg!(windows) {
            tr("Show in Explorer")
        } else {
            tr("Reveal in Finder")
        };
        item(ui, reveal, FileAction::Command("files.reveal", arg.clone()));
        item(
            ui,
            tr("Copy Full Path"),
            FileAction::Command("files.copyPath", arg.clone()),
        );
        if !is_project {
            menu::separator(ui, t);
            item(ui, tr("Rename"), FileAction::Rename(path.to_path_buf()));
            item(
                ui,
                tr("Duplicate"),
                FileAction::Command("files.duplicate", arg.clone()),
            );
            menu::separator(ui, t);
            let trash = if cfg!(windows) {
                tr("Move to Recycle Bin")
            } else {
                tr("Move to Trash")
            };
            item(ui, trash, FileAction::Command("files.trash", arg.clone()));
        }
    });
}

/// File rows at `depth` (1 = in the project folder). Geometry from the target at 2 px per pt:
/// disclosure chevrons centred at 27.5 pt, icons at 41 pt and names at 59.5 pt, 16 pt more per
/// level; the role badge ends 33.5 pt from the right.
fn file_rows(
    ui: &mut Ui,
    t: &Tokens,
    nodes: &[FileNode],
    depth: usize,
    ctx: &FileContext,
    st: &mut SidebarState,
    actions: &mut Vec<FileAction>,
) {
    let filter = ctx.filter.as_str();
    for n in nodes {
        let is_dir = n.kind == FileKind::Folder;
        if !filter.is_empty() && !is_dir && !n.name.to_lowercase().contains(filter) {
            continue;
        }
        if !filter.is_empty() && is_dir && !node_matches(n, filter) {
            continue;
        }
        let key = format!("file:{}", n.path.to_string_lossy());
        // Folders start collapsed, like the target's.
        let expanded = is_dir && (st.expanded.contains(&key) || !filter.is_empty());
        let indent = 16.0 * depth.saturating_sub(1) as f32;
        let role = ctx.role(&n.path);
        let renaming = st.renaming.as_ref().is_some_and(|(p, _, _)| *p == n.path);
        let draw = |p: &egui::Painter, r: Rect, name: Option<&str>| {
            let ix = r.left() + 41.0 + indent;
            if is_dir {
                let c = Rect::from_center_size(pos2(ix - 13.5, r.center().y), vec2(10.0, 12.0));
                if expanded {
                    icons::chevron_down(p, c, t.text_weak);
                } else {
                    icons::chevron_thin(p, c, t.text_weak, true);
                }
                icons::folder_pt(
                    p,
                    Rect::from_min_size(pos2(ix, r.center().y - 5.75), vec2(14.5, 11.5)),
                    t.folder,
                );
            } else {
                let label = match n.kind {
                    FileKind::Tex => "TEX",
                    FileKind::Bib => "BIB",
                    FileKind::Image => "IMG",
                    FileKind::Pdf => "PDF",
                    _ => "",
                };
                icons::document_pt(
                    p,
                    Rect::from_min_size(pos2(ix + 1.0, r.center().y - 7.5), vec2(12.0, 15.0)),
                    t.text_weak,
                    label,
                );
            }
            let badge_w = role.map_or(0.0, |role| {
                file_badge(p, t, pos2(r.right() - 33.5, r.center().y), role).width() + 6.0
            });
            if let Some(name) = name {
                let font = egui::FontId::proportional(SIDEBAR_FONT);
                let x = ix + 18.5;
                let end = if badge_w > 0.0 {
                    r.right() - 33.5 - badge_w
                } else {
                    r.right() - 12.0
                };
                let text = widgets::elide(p, name, &font, end - x);
                p.text(
                    pos2(x, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    text,
                    font,
                    t.sidebar_text,
                );
            }
        };
        if renaming {
            rename_row(ui, t, n, indent, st, actions, &draw);
        } else {
            let resp = widgets::list_row(ui, t, ROW_HEIGHT, false, Id::new(&key), |p, r| {
                draw(p, r, Some(&n.name));
            });
            file_menu(&resp, t, &n.path, false, ctx, actions);
            if resp.clicked() {
                if is_dir {
                    if expanded {
                        st.expanded.remove(&key);
                    } else {
                        st.expanded.insert(key.clone());
                    }
                } else {
                    actions.push(FileAction::Open(n.path.clone()));
                }
            }
        }
        if expanded {
            file_rows(ui, t, &n.children, depth + 1, ctx, st, actions);
        }
    }
}

/// A row whose name is being edited (Rename…): Return renames, Escape or clicking away cancels.
fn rename_row(
    ui: &mut Ui,
    t: &Tokens,
    n: &FileNode,
    indent: f32,
    st: &mut SidebarState,
    actions: &mut Vec<FileAction>,
    draw: &dyn Fn(&egui::Painter, Rect, Option<&str>),
) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    draw(&ui.painter_at(rect), rect, None);
    let Some((path, name, focus)) = st.renaming.as_mut() else {
        return;
    };
    let x = rect.left() + 41.0 + indent + 18.5 - 3.0;
    let field = Rect::from_min_max(
        pos2(x, rect.center().y - 9.0),
        pos2(rect.right() - 12.0, rect.center().y + 9.0),
    );
    let edit = ui.put(
        field,
        egui::TextEdit::singleline(name)
            .font(egui::FontId::proportional(SIDEBAR_FONT))
            .margin(vec2(3.0, 1.0))
            .text_color(t.sidebar_text),
    );
    if std::mem::take(focus) {
        edit.request_focus();
        // Select the name without its extension, as Finder does.
        let stem = n
            .name
            .rsplit_once('.')
            .filter(|(s, _)| !s.is_empty() && n.kind != FileKind::Folder)
            .map_or(n.name.chars().count(), |(s, _)| s.chars().count());
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit.id) {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(stem),
                )));
            state.store(ui.ctx(), edit.id);
        }
    }
    if edit.lost_focus() {
        if ui.input(|i| i.key_pressed(Key::Enter)) && *name != n.name {
            actions.push(FileAction::Command(
                "files.rename",
                serde_json::json!({ "path": path.to_string_lossy(), "name": name.trim() }),
            ));
        }
        st.renaming = None;
    }
}

/// A role badge (ROOT, SUB, OUTPUT, TEMP) whose right edge is at `right_center`.
fn file_badge(p: &egui::Painter, t: &Tokens, right_center: egui::Pos2, role: &str) -> Rect {
    let g = p.layout_no_wrap(
        role.to_string(),
        egui::FontId::proportional(10.5),
        t.badge_text,
    );
    let w = g.size().x + 11.0;
    let r = Rect::from_min_max(
        pos2(right_center.x - w, right_center.y - 7.5),
        pos2(right_center.x, right_center.y + 7.5),
    );
    p.rect_filled(r, 4.0, t.badge_fill);
    p.rect_stroke(
        r,
        4.0,
        Stroke::new(0.5, t.badge_border),
        egui::StrokeKind::Inside,
    );
    p.galley(r.center() - g.size() / 2.0, g, t.badge_text);
    r
}

fn node_matches(n: &FileNode, filter: &str) -> bool {
    n.name.to_lowercase().contains(filter) || n.children.iter().any(|c| node_matches(c, filter))
}

/// The local UTC offset in seconds, read once (from `date +%z` on Unix).
fn results(ui: &mut Ui, t: &Tokens, session: &mut Session) {
    ui.add_space(6.0);
    let header = match (&session.typeset.finished_at, &session.typeset.last) {
        (Some(at), Some(last)) => trf!(
            "Last typeset Today at {time} (Duration {seconds} seconds)",
            time = crate::clock::clock(*at),
            seconds = format!("{:.1}", last.duration.as_secs_f32())
        ),
        _ => tr("Not typeset yet").to_string(),
    };
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.add(egui::Label::new(egui::RichText::new(header).size(10.5).color(t.text_weak)).wrap());
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        let sel = if session.view.results_mode == ResultsMode::Issues {
            0
        } else {
            1
        };
        let w = ui.available_width() - 10.0;
        if let Some(i) = widgets::segmented_text(ui, t, &[tr("Issues"), tr("Log")], sel, w) {
            session.view.results_mode = if i == 0 {
                ResultsMode::Issues
            } else {
                ResultsMode::Log
            };
        }
    });
    ui.add_space(6.0);
    if session.view.results_mode == ResultsMode::Log {
        let mut log = session.typeset.console.clone();
        egui::ScrollArea::both()
            .id_salt("log-scroll")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut log)
                        .font(egui::FontId::monospace(10.5))
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .interactive(true),
                );
            });
        return;
    }
    let (e, w, b) = session.issue_counts();
    if session.typeset.is_running() && session.typeset.last.is_none() {
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(tr("Typesetting…")).color(t.text_weak))
        });
        return;
    }
    let summary_color = if e > 0 { t.error } else { t.success };
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        let errors = if e == 1 {
            trf!("{n} error", n = e)
        } else {
            trf!("{n} errors", n = e)
        };
        let warnings = if w == 1 {
            trf!("{n} warning", n = w)
        } else {
            trf!("{n} warnings", n = w)
        };
        let s = if b > 0 {
            let boxes = if b == 1 {
                trf!("{n} bad box", n = b)
            } else {
                trf!("{n} bad boxes", n = b)
            };
            trf!(
                "{errors} and {warnings}, {boxes}",
                errors = errors,
                warnings = warnings,
                boxes = boxes
            )
        } else {
            trf!(
                "{errors} and {warnings}",
                errors = errors,
                warnings = warnings
            )
        };
        ui.label(egui::RichText::new(s).size(11.5).color(summary_color));
    });
    if session.typeset.issues.is_empty() {
        if session.typeset.last.as_ref().is_some_and(|o| o.success) {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(tr("Typeset Successful"))
                        .size(17.0)
                        .color(t.text_weak.gamma_multiply(0.8)),
                )
            });
        } else if let Some(err) = session.typeset.last.as_ref().and_then(|o| o.error.clone()) {
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(10.0);
                ui.label(egui::RichText::new(err).size(11.5).color(t.error));
            });
        }
        return;
    }
    let issues = session.typeset.issues.clone();
    let mut reveal = None;
    egui::ScrollArea::vertical()
        .id_salt("issues-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, issue) in issues.iter().enumerate() {
                let color = match issue.severity {
                    Severity::Error => t.error,
                    Severity::Warning => t.warning,
                    _ => t.success,
                };
                let place = match (&issue.file, issue.line) {
                    (Some(f), Some(l)) => {
                        trf!("{file} (line {line})", file = file_label(f), line = l)
                    }
                    (Some(f), None) => file_label(f),
                    _ => tr("Log").to_string(),
                };
                let resp = widgets::list_row(ui, t, 38.0, false, Id::new(("issue", i)), |p, r| {
                    let ir =
                        Rect::from_min_size(pos2(r.left() + 10.0, r.top() + 5.0), vec2(14.0, 14.0));
                    icons::warning(p, ir, color, true);
                    let bold = egui::FontId::new(12.0, egui::FontFamily::Proportional);
                    p.text(
                        pos2(ir.right() + 6.0, ir.center().y),
                        egui::Align2::LEFT_CENTER,
                        &place,
                        bold,
                        t.sidebar_text,
                    );
                    let font = egui::FontId::proportional(11.0);
                    let msg = widgets::elide(p, &issue.message, &font, r.width() - 40.0);
                    p.text(
                        pos2(ir.right() + 6.0, r.bottom() - 10.0),
                        egui::Align2::LEFT_CENTER,
                        msg,
                        font,
                        t.text_weak,
                    );
                });
                let tip = if issue.detail.is_empty() {
                    issue.message.clone()
                } else {
                    format!("{}\n{}", issue.message, issue.detail)
                };
                if resp.on_hover_text(tip).clicked() {
                    reveal = Some(i);
                }
            }
        });
    if let Some(i) = reveal
        && let Err(e) = session.run("typeset.revealIssue", &serde_json::json!({ "index": i }))
    {
        session.set_status(e.to_string());
    }
}

fn search(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut SidebarState) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        let sel = if session.view.search_mode == SearchMode::Text {
            0
        } else {
            1
        };
        let w = ui.available_width() - 10.0;
        if let Some(i) = widgets::segmented_text(ui, t, &[tr("Text Search"), tr("Jump to")], sel, w)
        {
            session.view.search_mode = if i == 0 {
                SearchMode::Text
            } else {
                SearchMode::Tag
            };
            st.focus_search = true;
        }
    });
    ui.add_space(6.0);
    if session.view.search_mode == SearchMode::Text {
        text_search(ui, t, session, st);
    } else {
        tag_search(ui, t, session, st);
    }
}

/// Side margin of the search panel's fields and buttons.
const SEARCH_MARGIN: f32 = 10.0;

/// What a [`search_box`] shows at its left.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldIcon {
    /// A magnifier.
    Search,
    /// A magnifier with a small chevron that opens an options menu.
    SearchOptions,
    /// Nothing (the Replace field).
    None,
}

/// A rounded search field (macOS 26 style capsule on the window background): `icon` at the
/// left, the text, and a clear button while there is text. Returns the text edit's response
/// and, for [`FieldIcon::SearchOptions`], the magnifier's (open the menu from it).
fn search_box(
    ui: &mut Ui,
    t: &Tokens,
    text: &mut String,
    hint: &str,
    id: Id,
    icon: FieldIcon,
) -> (egui::Response, Option<egui::Response>) {
    let options = icon == FieldIcon::SearchOptions;
    let h = 26.0;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let rect = Rect::from_min_max(
        pos2(row.left() + SEARCH_MARGIN, row.top()),
        pos2(row.right() - SEARCH_MARGIN, row.bottom()),
    );
    let p = ui.painter().clone();
    let focused = ui.memory(|m| m.has_focus(id));
    p.rect(
        rect,
        h / 2.0,
        t.control_bg,
        if focused {
            Stroke::new(1.0, t.accent.gamma_multiply(0.7))
        } else {
            Stroke::new(0.5, t.control_border)
        },
        egui::StrokeKind::Inside,
    );
    let icon_w = match icon {
        FieldIcon::SearchOptions => 30.0,
        FieldIcon::Search => 24.0,
        FieldIcon::None => 12.0,
    };
    let icon_rect = Rect::from_min_size(rect.min, vec2(icon_w, h));
    let menu = options.then(|| {
        ui.interact(icon_rect, id.with("options"), Sense::click())
            .on_hover_text(tr("Search options"))
    });
    let ink = if menu
        .as_ref()
        .is_some_and(|m| m.hovered() || m.is_pointer_button_down_on())
    {
        t.text
    } else {
        t.text_weak
    };
    if icon != FieldIcon::None {
        icons::magnifier(
            &p,
            Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(13.0, 13.0)),
            ink,
            None,
        );
    }
    if options {
        icons::chevron_down(
            &p,
            Rect::from_center_size(
                pos2(rect.left() + 24.0, rect.center().y + 0.5),
                vec2(8.0, 8.0),
            ),
            ink,
        );
    }
    let right = if text.is_empty() { 12.0 } else { 26.0 };
    let edit = Rect::from_min_max(
        pos2(rect.left() + icon_w, rect.top() + 2.0),
        pos2(rect.right() - right, rect.bottom() - 2.0),
    );
    let resp = ui.put(
        edit,
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text(egui::RichText::new(hint).color(t.text_weak))
            .frame(egui::Frame::NONE)
            .font(egui::FontId::proportional(12.0))
            .vertical_align(Align::Center),
    );
    if !text.is_empty() {
        let x = Rect::from_center_size(rect.right_center() - vec2(13.0, 0.0), vec2(14.0, 14.0));
        let xr = ui.interact(x, id.with("clear"), Sense::click());
        let fill = if xr.hovered() {
            t.text_weak
        } else {
            t.text_weak.gamma_multiply(0.6)
        };
        p.circle_filled(x.center(), 6.5, fill);
        icons::close(&p, x.shrink(3.5), t.control_bg);
        if xr.clicked() {
            text.clear();
            ui.memory_mut(|m| m.request_focus(id));
        }
    }
    (resp, menu)
}

fn text_search(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut SidebarState) {
    let mut run = false;
    let id = Id::new("global-search-field");
    let hint = if st.search_regex {
        tr("Search project (regex)")
    } else {
        tr("Search project")
    };
    let (resp, options) = search_box(
        ui,
        t,
        &mut st.search_query,
        hint,
        id,
        FieldIcon::SearchOptions,
    );
    if st.focus_search {
        if st.search_query.is_empty()
            && let Some(sel) = session.doc().map(|d| {
                let a = texcraft_engine::edit::byte_of(&d.text, d.selection.start());
                let b = texcraft_engine::edit::byte_of(&d.text, d.selection.end());
                d.text.get(a..b).unwrap_or("").to_string()
            })
            && !sel.contains('\n')
        {
            st.search_query = sel;
        }
        resp.request_focus();
        st.focus_search = false;
    }
    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
        run = true;
    }
    if let Some(options) = &options {
        egui::Popup::menu(options).show(|ui| {
            menu::begin(ui, 200.0);
            let check = |on: bool| menu::Item {
                checked: Some(on),
                ..Default::default()
            };
            if menu::item(ui, t, tr("Ignoring Case"), check(!st.search_case)).clicked() {
                st.search_case = !st.search_case;
                run = true;
            }
            if menu::item(
                ui,
                t,
                tr("Treat Search Term as a Regex"),
                check(st.search_regex),
            )
            .clicked()
            {
                st.search_regex = !st.search_regex;
                run = true;
            }
            menu::separator(ui, t);
            menu::heading(ui, t, tr("Recent Searches"));
            if st.recent_searches.is_empty() {
                let none = menu::Item {
                    checked: Some(false),
                    disabled: true,
                    ..Default::default()
                };
                menu::item(ui, t, tr("No Recent Searches"), none);
            }
            for q in st.recent_searches.clone() {
                if menu::item(ui, t, &q, check(false)).clicked() {
                    st.search_query = q;
                    run = true;
                }
            }
            menu::separator(ui, t);
            let clear = menu::Item {
                checked: Some(false),
                disabled: st.recent_searches.is_empty(),
                ..Default::default()
            };
            if menu::item(ui, t, tr("Clear Recent Searches"), clear).clicked() {
                st.recent_searches.clear();
            }
        });
    }
    ui.add_space(6.0);
    let hint = if st.search_regex {
        tr("Replace ($1, $2…)")
    } else {
        tr("Replace")
    };
    search_box(
        ui,
        t,
        &mut st.replace_text,
        hint,
        Id::new("global-replace-field"),
        FieldIcon::None,
    );
    ui.add_space(7.0);
    let buttons = [
        widgets::Capsule {
            label: tr("Replace All"),
            default: false,
            enabled: !st.results.is_empty(),
        },
        widgets::Capsule {
            label: tr("Search"),
            default: true,
            enabled: !st.search_query.is_empty(),
        },
    ];
    match widgets::capsule_buttons(ui, t, &buttons, SEARCH_MARGIN) {
        Some(0) => {
            replace_all(session, st);
            run = true;
        }
        Some(_) => run = true,
        None => {}
    }
    if run {
        run_search(session, st);
    }
    let total: usize = st.results.iter().map(|(_, h)| h.len()).sum();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(SEARCH_MARGIN + 2.0);
        let status = if let Some(e) = &st.search_error {
            egui::RichText::new(e).color(t.error)
        } else if st.searched.is_some() {
            let n = st.results.len();
            let files = if n == 1 {
                trf!("{n} file", n = n)
            } else {
                trf!("{n} files", n = n)
            };
            let results = if total == 1 {
                trf!("{n} result", n = total)
            } else {
                trf!("{n} results", n = total)
            };
            egui::RichText::new(trf!(
                "{results} in {files}",
                results = results,
                files = files
            ))
            .color(t.text_weak)
        } else {
            egui::RichText::new("").color(t.text_weak)
        };
        ui.label(status.size(11.0));
    });
    if st.searched.is_none() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(tr("Text Search"))
                    .size(17.0)
                    .color(t.text_weak.gamma_multiply(0.8)),
            )
        });
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(tr("Search every file in the project"))
                    .size(11.0)
                    .color(t.text_weak),
            )
        });
        return;
    }
    let mut jump: Option<(PathBuf, usize, usize)> = None;
    let results = st.results.clone();
    egui::ScrollArea::vertical()
        .id_salt("search-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (path, hits) in &results {
                let key = format!("search:{}", path.display());
                if section_header(ui, t, st, &key, &file_label(path), Some(hits.len()), None) {
                    for (k, h) in hits.iter().enumerate() {
                        let resp = widgets::list_row(
                            ui,
                            t,
                            ROW_HEIGHT,
                            false,
                            Id::new(("hit", path, k)),
                            |p, r| {
                                let font = egui::FontId::proportional(11.5);
                                let mut job = egui::text::LayoutJob::default();
                                let fmt = |c: Color32, bg: Color32| egui::TextFormat {
                                    font_id: font.clone(),
                                    color: c,
                                    background: bg,
                                    ..Default::default()
                                };
                                job.append(
                                    &format!("{}. ", h.line),
                                    0.0,
                                    fmt(t.text_weak, Color32::TRANSPARENT),
                                );
                                let pre = h.context.get(..h.range.start).unwrap_or("");
                                let mid = h.context.get(h.range.clone()).unwrap_or("");
                                let post = h.context.get(h.range.end..).unwrap_or("");
                                job.append(pre, 0.0, fmt(t.sidebar_text, Color32::TRANSPARENT));
                                job.append(mid, 0.0, fmt(t.sidebar_text, t.find_match));
                                job.append(post, 0.0, fmt(t.sidebar_text, Color32::TRANSPARENT));
                                job.wrap.max_width = r.width() - 34.0;
                                job.wrap.max_rows = 1;
                                let g = p.layout_job(job);
                                p.galley(
                                    pos2(r.left() + 26.0, r.center().y - g.size().y / 2.0),
                                    g,
                                    t.sidebar_text,
                                );
                            },
                        );
                        if resp.clicked() {
                            jump = Some((path.clone(), h.offset, h.len));
                        }
                    }
                }
            }
        });
    if let Some((path, offset, len)) = jump
        && session.open_path(&path).is_ok()
        && let Some(d) = session.doc()
    {
        let a = texcraft_engine::edit::char_of(&d.text, offset);
        let b = texcraft_engine::edit::char_of(&d.text, offset + len);
        let line = texcraft_engine::edit::line_of_char(&d.text, a);
        let _ = session.jump(&path, line);
        let _ = session.run(
            "edit.select",
            &serde_json::json!({ "anchor": a, "cursor": b }),
        );
    }
}

fn project_files(session: &mut Session) -> Vec<PathBuf> {
    session.refresh_index(true);
    let mut files: Vec<PathBuf> = session
        .index
        .files
        .iter()
        .filter(|f| !f.missing)
        .map(|f| f.path.clone())
        .collect();
    for a in &session.index.assets {
        if a.exists && FileKind::of(&a.path) == FileKind::Bib && !files.contains(&a.path) {
            files.push(a.path.clone());
        }
    }
    files
}

fn build_regex(st: &SidebarState) -> Result<regex::Regex, String> {
    let pat = if st.search_regex {
        st.search_query.clone()
    } else {
        regex::escape(&st.search_query)
    };
    regex::RegexBuilder::new(&pat)
        .case_insensitive(!st.search_case)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| trf!("Invalid pattern: {e}", e = e))
}

fn run_search(session: &mut Session, st: &mut SidebarState) {
    st.results.clear();
    st.search_error = None;
    if st.search_query.is_empty() {
        st.searched = None;
        return;
    }
    let re = match build_regex(st) {
        Ok(r) => r,
        Err(e) => {
            st.search_error = Some(e);
            return;
        }
    };
    st.searched = Some(st.search_query.clone());
    let q = st.search_query.clone();
    st.recent_searches.retain(|r| r != &q);
    st.recent_searches.insert(0, q);
    st.recent_searches.truncate(8);
    for path in project_files(session) {
        let Some(text) = session.text_of(&path) else {
            continue;
        };
        let mut hits = Vec::new();
        let mut line_start = 0usize;
        let mut line_no = 1u32;
        for m in re.find_iter(&text).take(2000) {
            if m.start() == m.end() {
                continue;
            }
            while let Some(nl) = text
                .get(line_start..)
                .and_then(|s| s.find('\n'))
                .map(|i| line_start + i)
            {
                if nl >= m.start() {
                    break;
                }
                line_start = nl + 1;
                line_no += 1;
            }
            let line_end = text
                .get(m.start()..)
                .and_then(|s| s.find('\n'))
                .map_or(text.len(), |i| m.start() + i);
            let raw = text.get(line_start..line_end).unwrap_or("");
            let lead = raw.len() - raw.trim_start().len();
            let ctx_start = line_start + lead;
            let context = text.get(ctx_start..line_end).unwrap_or("").to_string();
            let rs = m.start().saturating_sub(ctx_start);
            let re_ = (m.end().min(line_end)).saturating_sub(ctx_start);
            hits.push(Hit {
                line: line_no,
                context,
                range: rs..re_.max(rs),
                offset: m.start(),
                len: m.end() - m.start(),
            });
        }
        if !hits.is_empty() {
            st.results.push((path, hits));
        }
    }
}

fn replace_all(session: &mut Session, st: &mut SidebarState) {
    let Ok(re) = build_regex(st) else { return };
    let mut count = 0usize;
    let replacement = st.replace_text.clone();
    let files: Vec<PathBuf> = st.results.iter().map(|(p, _)| p.clone()).collect();
    let active = session.active;
    for path in files {
        if session.open_path(&path).is_err() {
            continue;
        }
        let Some(d) = session.doc() else { continue };
        let n = re.find_iter(&d.text).count();
        let new = if st.search_regex {
            re.replace_all(&d.text, replacement.as_str()).into_owned()
        } else {
            re.replace_all(&d.text, regex::NoExpand(&replacement))
                .into_owned()
        };
        count += n;
        let _ = session.run("edit.setText", &serde_json::json!({ "text": new }));
    }
    session.active = active.min(session.docs.len().saturating_sub(1));
    session.set_status(if count == 1 {
        trf!("Replaced {n} occurrence (unsaved)", n = count)
    } else {
        trf!("Replaced {n} occurrences (unsaved)", n = count)
    });
}

struct Tag {
    title: String,
    path: PathBuf,
    line: u32,
    kind: &'static str,
}

fn tag_search(ui: &mut Ui, t: &Tokens, session: &mut Session, st: &mut SidebarState) {
    session.refresh_index(false);
    let id = Id::new("tag-search-field");
    let mut enter = false;
    let (resp, _) = search_box(
        ui,
        t,
        &mut st.tag_query,
        tr("Sections, labels, citations, files, lines"),
        id,
        FieldIcon::Search,
    );
    if st.focus_search {
        resp.request_focus();
        st.focus_search = false;
    }
    if resp.has_focus() {
        ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) {
                st.tag_selected += 1;
            }
            if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
                st.tag_selected = st.tag_selected.saturating_sub(1);
            }
        });
    }
    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
        enter = true;
    }
    if resp.changed() {
        st.tag_selected = 0;
    }
    ui.add_space(4.0);
    let q = st.tag_query.to_lowercase();
    let mut tags: Vec<Tag> = Vec::new();
    let idx = &session.index;
    if let Ok(n) = st.tag_query.trim().parse::<u32>()
        && let Some(p) = session
            .doc()
            .and_then(|d| d.typeset_path().map(Path::to_path_buf))
    {
        tags.push(Tag {
            title: trf!("Line {n}", n = n),
            path: p,
            line: n,
            kind: tr("line"),
        });
    }
    for f in &idx.files {
        if file_label(&f.path).to_lowercase().contains(&q) {
            tags.push(Tag {
                title: file_label(&f.path),
                path: f.path.clone(),
                line: 1,
                kind: tr("file"),
            });
        }
        for i in &f.outline {
            let kind = match i.kind {
                k if k.is_sectioning() => tr("section"),
                OutlineKind::Label => tr("label"),
                OutlineKind::Figure => tr("figure"),
                OutlineKind::Table => tr("table"),
                _ => continue,
            };
            if !i.title.is_empty() && (q.is_empty() || i.title.to_lowercase().contains(&q)) {
                tags.push(Tag {
                    title: i.title.clone(),
                    path: f.path.clone(),
                    line: i.line,
                    kind,
                });
            }
        }
    }
    for (e, bib) in &idx.bib {
        if q.is_empty() || e.key.to_lowercase().contains(&q) || e.title.to_lowercase().contains(&q)
        {
            tags.push(Tag {
                title: e.key.clone(),
                path: bib.clone(),
                line: e.line,
                kind: tr("citation"),
            });
        }
    }
    tags.truncate(500);
    st.tag_selected = st.tag_selected.min(tags.len().saturating_sub(1));
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(if tags.len() == 1 {
                trf!("{n} result", n = tags.len())
            } else {
                trf!("{n} results", n = tags.len())
            })
            .size(11.0)
            .color(t.text_weak),
        );
    });
    let mut jump = None;
    egui::ScrollArea::vertical()
        .id_salt("tag-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (k, tag) in tags.iter().enumerate() {
                let selected = k == st.tag_selected;
                let resp = widgets::list_row(ui, t, 34.0, selected, Id::new(("tag", k)), |p, r| {
                    let ir = Rect::from_min_size(
                        pos2(r.left() + 10.0, r.center().y - 7.0),
                        vec2(14.0, 14.0),
                    );
                    icons::star_square(p, ir, t.text_weak);
                    let font = egui::FontId::proportional(12.0);
                    let title = widgets::elide(p, &tag.title, &font, r.width() - 40.0);
                    p.text(
                        pos2(ir.right() + 6.0, r.top() + 10.0),
                        egui::Align2::LEFT_CENTER,
                        title,
                        font,
                        t.sidebar_text,
                    );
                    p.text(
                        pos2(ir.right() + 6.0, r.bottom() - 9.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{} · {} : {}", tag.kind, file_label(&tag.path), tag.line),
                        egui::FontId::proportional(10.0),
                        t.text_weak,
                    );
                });
                if selected && enter {
                    jump = Some((tag.path.clone(), tag.line));
                }
                if resp.clicked() {
                    jump = Some((tag.path.clone(), tag.line));
                }
            }
        });
    if let Some((p, l)) = jump {
        jump_and_sync(session, &p, l);
    }
}
