//! The native macOS menu bar, built from the command registry (muda).

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::mpsc::{Receiver, channel};

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use texcraft_engine::commands::Menu as CmdMenu;
use texcraft_engine::{Session, Shortcut, registry};
use texcraft_project::templates::TEMPLATES;

enum Item {
    Normal(MenuItem),
    Check(CheckMenuItem),
}

/// A menu item for a command, with the (enabled, checked) state last pushed to it.
type Entry = (&'static str, Item, Option<(bool, bool)>);

/// The installed menu bar and the handles needed to keep it in sync.
pub struct NativeMenu {
    _menu: Menu,
    items: Vec<Entry>,
    recent: Submenu,
    recent_shown: Vec<std::path::PathBuf>,
    rx: Receiver<String>,
}

fn accelerator(sc: Shortcut) -> Option<Accelerator> {
    let code = match sc.key {
        "/" => Code::Slash,
        "\\" => Code::Backslash,
        "[" => Code::BracketLeft,
        "]" => Code::BracketRight,
        "=" => Code::Equal,
        "-" => Code::Minus,
        "." => Code::Period,
        "," => Code::Comma,
        ";" => Code::Semicolon,
        "Tab" => Code::Tab,
        "Left" => Code::ArrowLeft,
        "Right" => Code::ArrowRight,
        "Up" => Code::ArrowUp,
        "Down" => Code::ArrowDown,
        "Enter" => Code::Enter,
        "Escape" => Code::Escape,
        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => {
            Code::from_str(&format!("Digit{k}")).ok()?
        }
        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphabetic()) => {
            Code::from_str(&format!("Key{}", k.to_ascii_uppercase())).ok()?
        }
        _ => return None,
    };
    let mut m = Modifiers::empty();
    if sc.cmd {
        m |= Modifiers::META;
    }
    if sc.shift {
        m |= Modifiers::SHIFT;
    }
    if sc.alt {
        m |= Modifiers::ALT;
    }
    if sc.ctrl {
        m |= Modifiers::CONTROL;
    }
    Some(Accelerator::new(m, code))
}

impl NativeMenu {
    /// Builds and installs the menu bar. `wake` is called on every menu event.
    pub fn install(wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let (tx, rx) = channel::<String>();
        let tx = std::sync::Mutex::new(tx);
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if let Ok(tx) = tx.lock() {
                let _ = tx.send(e.id.0.clone());
            }
            wake();
        }));

        let menu = Menu::new();
        let mut items: Vec<Entry> = Vec::new();
        let recent = Submenu::new("Open Recent", true);

        for top in CmdMenu::BAR {
            let sub = Submenu::new(top.title(), true);
            if top == CmdMenu::Window {
                let _ = sub.append(&PredefinedMenuItem::minimize(None));
                let _ = sub.append(&PredefinedMenuItem::maximize(Some("Zoom")));
                let _ = sub.append(&PredefinedMenuItem::fullscreen(None));
                let _ = sub.append(&PredefinedMenuItem::separator());
            }
            let mut last_group = None;
            for c in registry().iter().filter(|c| c.menu == top) {
                if last_group.is_some_and(|g| g != c.group) {
                    let _ = sub.append(&PredefinedMenuItem::separator());
                }
                last_group = Some(c.group);
                // The app menu gets the standard Services / Hide items before Quit.
                if top == CmdMenu::App && c.id == "app.quit" {
                    let _ = sub.append(&PredefinedMenuItem::services(None));
                    let _ = sub.append(&PredefinedMenuItem::separator());
                    let _ = sub.append(&PredefinedMenuItem::hide(Some("Hide TexCraft")));
                    let _ = sub.append(&PredefinedMenuItem::hide_others(None));
                    let _ = sub.append(&PredefinedMenuItem::show_all(None));
                    let _ = sub.append(&PredefinedMenuItem::separator());
                }
                if c.id == "file.newFromTemplate" {
                    let tsub = Submenu::new("New from Template", true);
                    for t in TEMPLATES {
                        let _ = tsub.append(&MenuItem::with_id(
                            format!("template:{}", t.id),
                            t.name,
                            true,
                            None,
                        ));
                    }
                    let _ = tsub.append(&PredefinedMenuItem::separator());
                    let _ = tsub.append(&MenuItem::with_id(
                        "file.newFromTemplate",
                        "Template Gallery…",
                        true,
                        None,
                    ));
                    let _ = sub.append(&tsub);
                    continue;
                }
                let acc = c.shortcut.and_then(accelerator);
                if c.checked.is_some() {
                    let item = CheckMenuItem::with_id(c.id, c.label, true, false, acc);
                    let _ = sub.append(&item);
                    items.push((c.id, Item::Check(item), None));
                } else {
                    let item = MenuItem::with_id(c.id, c.label, true, acc);
                    let _ = sub.append(&item);
                    items.push((c.id, Item::Normal(item), None));
                }
                if c.id == "file.openFolder" {
                    let _ = sub.append(&recent);
                }
            }
            if top == CmdMenu::Window {
                let _ = sub.append(&PredefinedMenuItem::separator());
                let _ = sub.append(&PredefinedMenuItem::bring_all_to_front(None));
                sub.set_as_windows_menu_for_nsapp();
            }
            if top == CmdMenu::Help {
                sub.set_as_help_menu_for_nsapp();
            }
            let _ = menu.append(&sub);
        }
        menu.init_for_nsapp();
        Some(Self {
            _menu: menu,
            items,
            recent,
            recent_shown: Vec::new(),
            rx,
        })
    }

    /// Menu item ids clicked since the last call.
    pub fn events(&self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(id) = self.rx.try_recv() {
            out.push(id);
        }
        out
    }

    /// Updates enabled/checked states and the Open Recent submenu.
    pub fn sync(&mut self, session: &Session) {
        for (id, item, last) in &mut self.items {
            let enabled = session.is_enabled(id);
            let checked = session.is_checked(id);
            if *last == Some((enabled, checked)) {
                continue;
            }
            match item {
                Item::Normal(m) => m.set_enabled(enabled),
                Item::Check(m) => {
                    m.set_enabled(enabled);
                    m.set_checked(checked);
                }
            }
            *last = Some((enabled, checked));
        }
        if self.recent_shown != session.recent {
            for _ in 0..self.recent.items().len() {
                let _ = self.recent.remove_at(0);
            }
            let mut names: HashMap<String, usize> = HashMap::new();
            for p in &session.recent {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                *names.entry(name).or_default() += 1;
            }
            for p in &session.recent {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let label = if names.get(&name).copied().unwrap_or(0) > 1 {
                    format!(
                        "{name} — {}",
                        p.parent()
                            .map(|d| d.display().to_string())
                            .unwrap_or_default()
                    )
                } else {
                    name
                };
                let _ = self.recent.append(&MenuItem::with_id(
                    format!("recent:{}", p.display()),
                    label,
                    true,
                    None,
                ));
            }
            if session.recent.is_empty() {
                let _ = self
                    .recent
                    .append(&MenuItem::new("No Recent Documents", false, None));
            }
            self.recent_shown = session.recent.clone();
        }
    }
}
