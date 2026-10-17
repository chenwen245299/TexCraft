//! TexCraft desktop app: the eframe window, native menu bar, file panels, persistence and the
//! agent control channel around the `texcraft-ui` workbench.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod control;
#[cfg(target_os = "macos")]
mod menu;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Event, Key, Modifiers, ViewportCommand};
use serde_json::{Value, json};
use texcraft_engine::{NativeEdit, Session, View, registry};
use texcraft_ui::{Request, Workbench};

const STORAGE_KEY: &str = "texcraft-state-v2";

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Persisted {
    view: Option<View>,
    settings: Option<texcraft_project::Settings>,
    recent: Vec<PathBuf>,
    split: Option<f32>,
    /// Bumped when defaults change in a way saved settings should pick up.
    #[serde(default)]
    version: u32,
}

/// Current [`Persisted::version`]. 1: live preview starts on the first keystroke (no delay).
/// 2: the editor's default font size is 16 pt, as measured on the target.
const PERSISTED_VERSION: u32 = 2;

/// Command-line options.
#[derive(Default)]
struct Options {
    paths: Vec<PathBuf>,
    control_port: Option<u16>,
    control_token_file: Option<PathBuf>,
    screenshot: Option<PathBuf>,
    screenshot_delay: Duration,
    size: Option<(f32, f32)>,
    theme: Option<String>,
    welcome: bool,
    commands: Vec<String>,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options {
        welcome: true,
        screenshot_delay: Duration::from_millis(2500),
        ..Options::default()
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--control" => {
                o.control_port = Some(
                    value("--control")?
                        .parse()
                        .map_err(|_| "bad --control port")?,
                )
            }
            "--control-token-file" => {
                o.control_token_file = Some(PathBuf::from(value("--control-token-file")?))
            }
            "--screenshot" => o.screenshot = Some(PathBuf::from(value("--screenshot")?)),
            "--screenshot-delay-ms" => {
                o.screenshot_delay = Duration::from_millis(
                    value("--screenshot-delay-ms")?
                        .parse()
                        .map_err(|_| "bad delay")?,
                )
            }
            "--size" => {
                let v = value("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size WxH")?;
                o.size = Some((
                    w.parse().map_err(|_| "bad width")?,
                    h.parse().map_err(|_| "bad height")?,
                ));
            }
            "--theme" => o.theme = Some(value("--theme")?),
            "--run" => o.commands.push(value("--run")?),
            "--no-welcome" => o.welcome = false,
            "--write-icon" => {
                // Used by scripts/bundle-macos.sh to make the .icns.
                let path = value("--write-icon")?;
                std::fs::write(&path, ICON_PNG).map_err(|e| format!("cannot write {path}: {e}"))?;
                std::process::exit(0);
            }
            "-h" | "--help" => {
                println!(
                    "texcraft [FILE_OR_FOLDER…] [--control PORT] [--control-token-file PATH] [--screenshot OUT.png] [--screenshot-delay-ms N] [--size WxH] [--theme light|dark] [--run COMMAND_ID] [--no-welcome]"
                );
                std::process::exit(0);
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            path => o.paths.push(PathBuf::from(path)),
        }
    }
    Ok(o)
}

/// A control request waiting for something (a screenshot, a typeset, some frames).
enum Deferred {
    Screenshot {
        path: PathBuf,
        reply: std::sync::mpsc::Sender<Result<Value, String>>,
    },
    Typeset {
        until: Instant,
        reply: std::sync::mpsc::Sender<Result<Value, String>>,
    },
    Frames {
        left: u32,
        reply: std::sync::mpsc::Sender<Result<Value, String>>,
    },
}

struct App {
    session: Session,
    bench: Workbench,
    control: Option<control::Server>,
    deferred: Vec<Deferred>,
    native: Vec<NativeEdit>,
    /// The window title last sent (every viewport command costs a frame, so send changes only).
    title: String,
    pending_keys: Vec<(Key, Modifiers)>,
    /// Injected pointer events, one batch per frame (`ui.pointer`).
    pending_pointer: std::collections::VecDeque<Vec<Event>>,
    screenshot_cli: Option<(PathBuf, Instant)>,
    screenshot_requested: bool,
    #[cfg(target_os = "macos")]
    menu: Option<menu::NativeMenu>,
    started: Instant,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, opts: Options) -> Self {
        let ctx = cc.egui_ctx.clone();
        let waker = {
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint()) as Arc<dyn Fn() + Send + Sync>
        };
        let mut session = Session::new(waker);
        // AI Assistant providers live next to the app's settings (owner-only file: API keys).
        if let Some(dir) = eframe::storage_dir("TexCraft") {
            session.load_ai_config(dir.join("ai.json"));
        }
        // Move to Trash through AppKit (Finder's Put Back works).
        #[cfg(target_os = "macos")]
        {
            session.trash = texcraft_macos::trash;
        }
        // Automatic update checks (off for one-shot screenshots and with TEXCRAFT_NO_UPDATES);
        // TEXCRAFT_UPDATE_URL points them at another manifest (testing).
        session.update.enabled =
            opts.screenshot.is_none() && std::env::var_os("TEXCRAFT_NO_UPDATES").is_none();
        if let Ok(url) = std::env::var("TEXCRAFT_UPDATE_URL") {
            session.update.url = url;
        }
        let mut bench = Workbench::new(&ctx);
        if let Some(p) = cc
            .storage
            .and_then(|s| eframe::get_value::<Persisted>(s, STORAGE_KEY))
        {
            if let Some(v) = p.view {
                session.view = v;
            }
            if let Some(s) = p.settings {
                session.settings = s;
            }
            if p.version < 1 {
                session.settings.live_delay_ms = 0;
            }
            if p.version < 2 && (session.view.editor_font_size - 15.0).abs() < 0.1 {
                session.view.editor_font_size = 16.0;
            }
            session.set_recent(p.recent);
            if let Some(split) = p.split {
                bench.split = split.clamp(0.2, 0.8);
            }
        }
        match opts.theme.as_deref() {
            Some("dark") => session.view.theme = texcraft_engine::ThemeChoice::Dark,
            Some("light") => session.view.theme = texcraft_engine::ThemeChoice::Light,
            _ => {}
        }
        let mut opened = false;
        for p in &opts.paths {
            match session.open_path(p) {
                Ok(_) => opened = true,
                Err(e) => session.set_status(e.to_string()),
            }
        }
        if opened {
            // Typeset right away so the PDF appears (like opening a project in the target app).
            if let Err(e) = session.typeset() {
                session.set_status(e.to_string());
            }
        } else if opts.welcome && session.view.show_welcome && opts.screenshot.is_none() {
            bench.welcome.show_window(true);
        }
        for id in &opts.commands {
            if let Err(e) = session.run(id, &Value::Null) {
                eprintln!("--run {id}: {e}");
            }
        }

        let control = opts.control_port.and_then(|port| {
            let token = match &opts.control_token_file {
                Some(f) => control::token_from_file(f)
                    .map_err(|e| eprintln!("control token: {e}"))
                    .ok()?,
                None => {
                    let t = control::new_token();
                    eprintln!("TexCraft control token: {t}");
                    t
                }
            };
            let wake_ctx = ctx.clone();
            match control::start(port, token, move || wake_ctx.request_repaint()) {
                Ok(s) => {
                    eprintln!("TexCraft control channel on 127.0.0.1:{}", s.port);
                    Some(s)
                }
                Err(e) => {
                    eprintln!("control channel: {e}");
                    None
                }
            }
        });

        #[cfg(target_os = "macos")]
        let menu = {
            let wake_ctx = ctx.clone();
            let m = menu::NativeMenu::install(move || wake_ctx.request_repaint());
            bench.handle_shortcuts = m.is_none();
            m
        };

        Self {
            session,
            bench,
            control,
            deferred: Vec::new(),
            native: Vec::new(),
            title: String::new(),
            pending_keys: Vec::new(),
            pending_pointer: std::collections::VecDeque::new(),
            screenshot_cli: opts
                .screenshot
                .map(|p| (p, Instant::now() + opts.screenshot_delay)),
            screenshot_requested: false,
            #[cfg(target_os = "macos")]
            menu,
            started: Instant::now(),
        }
    }

    /// Runs a command chosen in the native menu bar (macOS only).
    #[cfg(target_os = "macos")]
    fn run_command(&mut self, id: &str) {
        if let Some(t) = id.strip_prefix("template:") {
            let _ = self
                .session
                .run("file.newFromTemplate", &json!({ "id": t }));
            return;
        }
        if let Some(p) = id.strip_prefix("recent:") {
            if let Err(e) = self.session.open_path(std::path::Path::new(p)) {
                self.session.set_status(e.to_string());
            }
            return;
        }
        if let Err(e) = self.session.run(id, &Value::Null) {
            self.session.set_status(e.to_string());
        }
    }

    fn handle_requests(&mut self, ctx: &egui::Context) {
        for r in std::mem::take(&mut self.bench.requests) {
            match r {
                Request::Open => {
                    if let Some(paths) = rfd::FileDialog::new()
                        .add_filter(
                            "TeX documents",
                            &["tex", "ltx", "bib", "sty", "cls", "dtx", "txt", "md", "pdf"],
                        )
                        .add_filter("All files", &["*"])
                        .pick_files()
                    {
                        let typeset_first = self.session.pdf.path.is_none();
                        for p in paths {
                            if let Err(e) = self.session.open_path(&p) {
                                self.bench.dialog = texcraft_ui::dialogs::Dialog::Alert {
                                    title: "Cannot open".into(),
                                    message: e.to_string(),
                                };
                            }
                        }
                        if typeset_first && self.session.pdf.path.is_none() {
                            let _ = self.session.typeset();
                        }
                    }
                }
                Request::OpenFolder => {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        if let Err(e) = self.session.open_path(&dir) {
                            self.session.set_status(e.to_string());
                        } else if self.session.pdf.path.is_none() {
                            let _ = self.session.typeset();
                        }
                    }
                }
                Request::SaveAs(doc) => {
                    let name = self
                        .session
                        .docs
                        .get(doc)
                        .and_then(|d| {
                            d.path
                                .as_ref()
                                .and_then(|p| p.file_name())
                                .map(|n| n.to_string_lossy().into_owned())
                        })
                        .unwrap_or_else(|| "Untitled.tex".into());
                    let mut dialog = rfd::FileDialog::new()
                        .set_file_name(name)
                        .add_filter("TeX document", &["tex"]);
                    if let Some(dir) = &self.session.project_dir {
                        dialog = dialog.set_directory(dir);
                    }
                    if let Some(path) = dialog.save_file()
                        && let Err(e) = self.session.save_as(doc, path)
                    {
                        self.session.set_status(e.to_string());
                    }
                }
                Request::ExportPdf => {
                    let name = self
                        .session
                        .root_path()
                        .and_then(|r| {
                            r.file_stem()
                                .map(|s| format!("{}.pdf", s.to_string_lossy()))
                        })
                        .unwrap_or_else(|| "document.pdf".into());
                    if let Some(path) = rfd::FileDialog::new()
                        .set_file_name(name)
                        .add_filter("PDF", &["pdf"])
                        .save_file()
                    {
                        let _ = self.session.run("file.exportPdf", &json!({ "path": path }));
                    }
                }
                Request::Reveal(p) => reveal(&p),
                Request::OpenExternally(p) => open_externally(&p),
                Request::Native(n) => {
                    self.native.push(n);
                    ctx.request_repaint();
                }
                Request::CopyText(text) => ctx.copy_text(text),
                Request::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }

    fn handle_control(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(server) = &self.control else { return };
        let mut pending = Vec::new();
        while let Some(p) = server.try_recv() {
            pending.push(p);
        }
        for p in pending {
            let result: Result<Value, String> = match p.method.as_str() {
                "commands.list" => Ok(Value::Array(
                    registry()
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id, "label": c.label, "menu": c.menu.title(),
                                "shortcut": c.shortcut.map(texcraft_engine::Shortcut::display),
                                "params": c.params,
                                "enabled": self.session.is_enabled(c.id),
                                "checked": c.checked.map(|_| self.session.is_checked(c.id)),
                            })
                        })
                        .collect(),
                )),
                "command.run" => {
                    let id = p.params.get("id").and_then(Value::as_str).unwrap_or("");
                    let params = p.params.get("params").cloned().unwrap_or(Value::Null);
                    self.session
                        .run(id, &params)
                        .map(|()| json!("ok"))
                        .map_err(|e| e.to_string())
                }
                "state.inspect" => {
                    let mut v = self.session.inspect();
                    v["ui"] = json!({
                        "pdfPages": self.bench.pdf.page_count(),
                        "pdfCurrentPage": self.bench.pdf.current_page + 1,
                        "pdfRenderedPages": self.bench.pdf.rendered_pages(),
                        "dialog": format!("{:?}", self.bench.dialog),
                        "settings": {
                            "open": self.bench.prefs.open,
                            "pane": format!("{:?}", self.bench.prefs.pane),
                            // Whether the native Settings window was found and its button placed.
                            "windowButtons": self.bench.prefs.open && !self.bench.prefs.inline
                                && texcraft_macos::place_traffic_lights_titled(texcraft_ui::prefs::SETTINGS_TITLE, 51.0, 19.0, true),
                        },
                        "outerRect": ctx.input(|i| i.viewport().outer_rect.map(|r| [r.min.x, r.min.y, r.width(), r.height()])),
                        "fullScreen": ctx.input(|i| i.viewport().fullscreen),
                        "appKitFullScreen": texcraft_macos::is_full_screen(frame),
                        "monitorSize": ctx.input(|i| i.viewport().monitor_size.map(|s| [s.x, s.y])),
                        "trafficLights": texcraft_macos::traffic_light_frames(frame),
                        "windows": texcraft_macos::windows(),
                        "welcome": { "open": self.bench.welcome.open, "glass": self.bench.welcome.glass() },
                        "modelPicker": self.bench.prefs.model_picker_info(),
                        "updateWindow": self.bench.update_window,
                        // Why the last frame asked for another one (idle CPU diagnostics).
                        "repaintCauses": ctx.repaint_causes().iter().map(|c| format!("{c:?}")).collect::<Vec<_>>(),
                        "completion": self.bench.editor.completion.as_ref().map(|c| c.items.iter().take(10).map(|i| i.label.clone()).collect::<Vec<_>>()),
                    });
                    Ok(v)
                }
                "editor.getText" => Ok(json!(
                    self.session
                        .doc()
                        .map(|d| d.text.clone())
                        .unwrap_or_default()
                )),
                "ui.screenshot" => {
                    let path = p
                        .params
                        .get("path")
                        .and_then(Value::as_str)
                        .map(PathBuf::from)
                        .unwrap_or_else(|| std::env::temp_dir().join("texcraft-screenshot.png"));
                    ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
                    self.deferred.push(Deferred::Screenshot {
                        path,
                        reply: p.reply,
                    });
                    ctx.request_repaint();
                    continue;
                }
                "ui.welcome" => {
                    // {open?: bool, inline?: bool, leftTab?: n, rightTab?: n}
                    let w = &mut self.bench.welcome;
                    if let Some(inline) = p.params.get("inline").and_then(Value::as_bool) {
                        w.inline = inline;
                    }
                    if let Some(open) = p.params.get("open").and_then(Value::as_bool) {
                        if open {
                            w.show_window(false);
                        } else {
                            w.open = false;
                        }
                    }
                    if let Some(n) = p.params.get("leftTab").and_then(Value::as_u64) {
                        w.left_tab = n.min(2) as usize;
                    }
                    if let Some(n) = p.params.get("rightTab").and_then(Value::as_u64) {
                        w.right_tab = n.min(1) as usize;
                    }
                    // {close: true}: what the window's close button does (a close request to
                    // its native window).
                    if p.params.get("close").and_then(Value::as_bool) == Some(true) && w.open {
                        ctx.send_viewport_cmd_to(
                            texcraft_ui::welcome::welcome_viewport(),
                            ViewportCommand::Close,
                        );
                    }
                    ctx.request_repaint();
                    Ok(json!({ "open": w.open, "launch": w.launch }))
                }
                "ui.prefs" => {
                    // {open?: bool, pane?: "general"|"sidebar"|"editor"|"viewer"|"typesetting",
                    //  editorTab?: n, sheet?: bool, sheetTab?: n, inline?: bool, models?: bool,
                    //  modelGroup?: "text"|"vision"|"other"|null}
                    let prefs = &mut self.bench.prefs;
                    if let Some(inline) = p.params.get("inline").and_then(Value::as_bool) {
                        prefs.inline = inline;
                    }
                    if let Some(open) = p.params.get("open").and_then(Value::as_bool) {
                        prefs.open = open;
                        prefs.focus = open;
                    }
                    if let Some(pane) = p
                        .params
                        .get("pane")
                        .and_then(Value::as_str)
                        .and_then(texcraft_ui::prefs::Pane::from_name)
                    {
                        prefs.pane = pane;
                    }
                    if let Some(n) = p.params.get("editorTab").and_then(Value::as_u64) {
                        prefs.editor_tab = n.min(1) as usize;
                    }
                    if let Some(n) = p.params.get("sheetTab").and_then(Value::as_u64) {
                        prefs.sheet_tab = n.min(3) as usize;
                    }
                    // models: open (true) or close the model picker for the selected provider.
                    if let Some(models) = p.params.get("models").and_then(Value::as_bool) {
                        if models {
                            prefs.open_model_picker(ctx, &self.session);
                        } else {
                            prefs.model_picker().close();
                        }
                    }
                    // modelGroup: the picker's tab ("text", "vision", "other"; anything else: all).
                    if let Some(g) = p.params.get("modelGroup") {
                        prefs.model_picker().set_group(g.as_str());
                    }
                    if let Some(sheet) = p.params.get("sheet").and_then(Value::as_bool) {
                        self.bench.dialog = if sheet {
                            texcraft_ui::dialogs::Dialog::TypesetConfiguration
                        } else {
                            texcraft_ui::dialogs::Dialog::None
                        };
                    }
                    ctx.request_repaint();
                    Ok(json!({ "open": prefs.open, "pane": format!("{:?}", prefs.pane) }))
                }
                "typeset.wait" => {
                    let ms = p
                        .params
                        .get("timeoutMs")
                        .and_then(Value::as_u64)
                        .unwrap_or(60_000);
                    self.deferred.push(Deferred::Typeset {
                        until: Instant::now() + Duration::from_millis(ms),
                        reply: p.reply,
                    });
                    continue;
                }
                "ui.wait" => {
                    let frames = p.params.get("frames").and_then(Value::as_u64).unwrap_or(5) as u32;
                    self.deferred.push(Deferred::Frames {
                        left: frames.max(1),
                        reply: p.reply,
                    });
                    continue;
                }
                "ui.resize" => {
                    let w = p
                        .params
                        .get("width")
                        .and_then(Value::as_f64)
                        .unwrap_or(1388.0) as f32;
                    let h = p
                        .params
                        .get("height")
                        .and_then(Value::as_f64)
                        .unwrap_or(824.0) as f32;
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(w, h)));
                    Ok(json!("ok"))
                }
                "ui.key" => {
                    // {key: "Enter", command?: bool, shift?: bool, alt?: bool, ctrl?: bool} or {text: "…"}
                    if let Some(text) = p.params.get("text").and_then(Value::as_str) {
                        let t = text.to_string();
                        ctx.input_mut(|i| i.events.push(Event::Text(t)));
                        Ok(json!("ok"))
                    } else if let Some(k) = p
                        .params
                        .get("key")
                        .and_then(Value::as_str)
                        .and_then(Key::from_name)
                    {
                        let b = |n: &str| p.params.get(n).and_then(Value::as_bool).unwrap_or(false);
                        let modifiers = Modifiers {
                            alt: b("alt"),
                            ctrl: b("ctrl"),
                            shift: b("shift"),
                            mac_cmd: b("command") && cfg!(target_os = "macos"),
                            command: b("command"),
                        };
                        self.pending_keys.push((k, modifiers));
                        ctx.request_repaint();
                        Ok(json!("ok"))
                    } else {
                        Err("give `key` (egui key name) or `text`".into())
                    }
                }
                "ui.fullscreen" => {
                    // {on?: bool}: enters or leaves full screen (toggles without `on`); returns
                    // the state before.
                    // On macOS it goes through AppKit as the green window button does.
                    let was = texcraft_macos::is_full_screen(frame)
                        .unwrap_or_else(|| ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)));
                    let on = p.params.get("on").and_then(Value::as_bool).unwrap_or(!was);
                    if on != was && !texcraft_macos::toggle_full_screen(frame) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
                    }
                    Ok(json!({ "was": was, "on": on }))
                }
                "ui.drag" => {
                    // {from: [x, y], to: [x, y], steps?} in points: press, move in steps, release.
                    let pt = |n: &str| {
                        let v = p.params.get(n)?.as_array()?;
                        Some(egui::pos2(
                            v.first()?.as_f64()? as f32,
                            v.get(1)?.as_f64()? as f32,
                        ))
                    };
                    match (pt("from"), pt("to")) {
                        (Some(a), Some(b)) => {
                            let steps = p
                                .params
                                .get("steps")
                                .and_then(Value::as_u64)
                                .unwrap_or(8)
                                .clamp(1, 60);
                            let button = |pos, pressed| Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Modifiers::NONE,
                            };
                            self.pending_pointer.push_back(vec![Event::PointerMoved(a)]);
                            self.pending_pointer.push_back(vec![button(a, true)]);
                            for k in 1..=steps {
                                let f = k as f32 / steps as f32;
                                self.pending_pointer
                                    .push_back(vec![Event::PointerMoved(a + (b - a) * f)]);
                            }
                            self.pending_pointer.push_back(vec![button(b, false)]);
                            ctx.request_repaint();
                            Ok(json!("ok"))
                        }
                        _ => Err("give `from` and `to` as [x, y] (points)".into()),
                    }
                }
                "ui.pointer" => {
                    // {x, y, click?: "primary" | "secondary"} in points: moves the pointer there
                    // (hover) and optionally clicks (press this frame, release the next).
                    let f = |n: &str| p.params.get(n).and_then(Value::as_f64);
                    match (f("x"), f("y")) {
                        (Some(x), Some(y)) => {
                            let pos = egui::pos2(x as f32, y as f32);
                            let button = match p.params.get("click").and_then(Value::as_str) {
                                Some("secondary") => Some(egui::PointerButton::Secondary),
                                Some(_) => Some(egui::PointerButton::Primary),
                                None => None,
                            };
                            let press = |pressed| Event::PointerButton {
                                pos,
                                button: button.unwrap_or(egui::PointerButton::Primary),
                                pressed,
                                modifiers: Modifiers::NONE,
                            };
                            let mut first = vec![Event::PointerMoved(pos)];
                            if button.is_some() {
                                first.push(press(true));
                                self.pending_pointer.push_back(first);
                                self.pending_pointer.push_back(vec![press(false)]);
                            } else {
                                self.pending_pointer.push_back(first);
                            }
                            ctx.request_repaint();
                            Ok(json!("ok"))
                        }
                        _ => Err("give `x` and `y` (points)".into()),
                    }
                }
                other => Err(format!("unknown method `{other}`")),
            };
            let _ = p.reply.send(result);
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn handle_deferred(&mut self, ctx: &egui::Context) {
        let screenshot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let mut keep = Vec::new();
        for d in std::mem::take(&mut self.deferred) {
            match d {
                Deferred::Screenshot { path, reply } => match &screenshot {
                    Some(img) => {
                        let _ = reply.send(save_png(img, &path).map(|()| json!({ "path": path, "width": img.size[0], "height": img.size[1] })));
                    }
                    None => keep.push(Deferred::Screenshot { path, reply }),
                },
                Deferred::Typeset { until, reply } => {
                    let busy = self.session.typeset.is_running()
                        || self.session.typeset.live_due.is_some()
                        || self.session.typeset.queued;
                    if !busy {
                        let _ = reply.send(Ok(self.session.inspect()["typeset"].clone()));
                    } else if Instant::now() > until {
                        let _ = reply.send(Err("timeout: still typesetting".into()));
                    } else {
                        keep.push(Deferred::Typeset { until, reply });
                    }
                }
                Deferred::Frames { left, reply } => {
                    if left <= 1 {
                        let _ = reply.send(Ok(json!("ok")));
                    } else {
                        keep.push(Deferred::Frames {
                            left: left - 1,
                            reply,
                        });
                    }
                }
            }
        }
        if !keep.is_empty() {
            ctx.request_repaint();
        }
        self.deferred = keep;

        // --screenshot: wait for the delay and for typesetting/rendering to settle, then save and quit.
        if let Some((path, at)) = self.screenshot_cli.clone() {
            let busy = self.session.typeset.is_running() || self.session.typeset.live_due.is_some();
            if Instant::now() >= at && (!busy || self.started.elapsed() > Duration::from_secs(60)) {
                if let Some(img) = &screenshot {
                    match save_png(img, &path) {
                        Ok(()) => eprintln!("saved {}", path.display()),
                        Err(e) => eprintln!("screenshot: {e}"),
                    }
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                    self.screenshot_cli = None;
                } else if !self.screenshot_requested {
                    ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
                    self.screenshot_requested = true;
                }
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

fn save_png(img: &egui::ColorImage, path: &std::path::Path) -> Result<(), String> {
    let [w, h] = img.size;
    let mut buf = Vec::with_capacity(w * h * 4);
    for p in &img.pixels {
        buf.extend_from_slice(&p.to_srgba_unmultiplied());
    }
    let out = image::RgbaImage::from_raw(w as u32, h as u32, buf).ok_or("bad image size")?;
    out.save(path)
        .map_err(|e| format!("cannot save {}: {e}", path.display()))
}

fn open_externally(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn();
}

/// A clipboard picture as PNG, at most 2048 pixels on its longer side (chat APIs scale larger
/// ones down anyway, and a Retina screenshot is several megabytes).
fn png_from_clipboard(img: &arboard::ImageData<'_>) -> Option<Vec<u8>> {
    const LONGEST: u32 = 2048;
    let (w, h) = (
        u32::try_from(img.width).ok()?,
        u32::try_from(img.height).ok()?,
    );
    let mut rgba = image::RgbaImage::from_raw(w, h, img.bytes.to_vec())?;
    let longest = w.max(h);
    if longest > LONGEST {
        let scale = LONGEST as f32 / longest as f32;
        let size = |n: u32| ((n as f32 * scale).round() as u32).max(1);
        rgba = image::imageops::resize(
            &rgba,
            size(w),
            size(h),
            image::imageops::FilterType::Triangle,
        );
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()?;
    Some(out.into_inner())
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let mut edits = Vec::new();
        for n in std::mem::take(&mut self.native) {
            let key = |key: Key, shift: bool| Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: if shift {
                    Modifiers::COMMAND | Modifiers::SHIFT
                } else {
                    Modifiers::COMMAND
                },
            };
            match n {
                NativeEdit::Undo => edits.push(key(Key::Z, false)),
                NativeEdit::Redo => edits.push(key(Key::Z, true)),
                NativeEdit::SelectAll => edits.push(key(Key::A, false)),
                NativeEdit::Copy => edits.push(Event::Copy),
                NativeEdit::Cut => edits.push(Event::Cut),
                NativeEdit::Paste => {
                    let mut clip = arboard::Clipboard::new().ok();
                    match clip
                        .as_mut()
                        .and_then(|c| c.get_text().ok())
                        .filter(|t| !t.is_empty())
                    {
                        Some(text) => edits.push(Event::Paste(text)),
                        // A copied picture (a screenshot) goes with the Assistant's question
                        // when its box has focus.
                        None if texcraft_ui::ai_panel::composer_focused(ctx) => {
                            if let Some(png) = clip
                                .as_mut()
                                .and_then(|c| c.get_image().ok())
                                .and_then(|img| png_from_clipboard(&img))
                            {
                                self.session.ai.notice = self
                                    .session
                                    .ai_attach_image("Pasted Image.png", png)
                                    .err()
                                    .map(|e| e.to_string());
                            }
                        }
                        None => {}
                    }
                }
            }
        }
        // The focused Settings or welcome window gets its own (its text fields: API keys…).
        raw.events.extend(self.bench.route_edit_events(ctx, edits));
        if let Some(batch) = self.pending_pointer.pop_front() {
            raw.events.extend(batch);
            if !self.pending_pointer.is_empty() {
                ctx.request_repaint();
            }
        }
        for (key, modifiers) in std::mem::take(&mut self.pending_keys) {
            raw.events.push(Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
            raw.events.push(Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            });
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if cfg!(target_os = "macos") {
            let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            // Leaving full screen AppKit animates without asking us for frames, from our last
            // full-screen frame (toolbar at the window's edge), with the buttons at its default
            // place: over the capsule. So they go transparent the moment the exit begins (the
            // observer), and the first frame after it puts the toolbar and the buttons back in
            // place at once and shows them.
            let _ = texcraft_macos::hide_traffic_lights_on_full_screen_exit(frame);
            // Centre the window buttons in our taller toolbar (not in full screen, where AppKit
            // keeps them in its own title bar).
            if !full {
                let _ = texcraft_macos::place_traffic_lights(
                    frame,
                    f64::from(texcraft_ui::theme::TOOLBAR_HEIGHT),
                    f64::from(texcraft_ui::theme::TRAFFIC_LIGHTS_LEFT),
                );
                let _ = texcraft_macos::set_traffic_lights_alpha(frame, 1.0);
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(m) = &mut self.menu {
            for id in m.events() {
                // ⌘W in the Settings or PDF window closes that window, not a document.
                if matches!(id.as_str(), "file.closeTab" | "file.closeWindow")
                    && self.bench.close_focused_window(&ctx)
                {
                    continue;
                }
                self.run_command(&id);
            }
        }
        self.handle_control(&ctx, frame);

        // Closing with unsaved changes: ask first.
        if ctx.input(|i| i.viewport().close_requested())
            && let Some(i) = self
                .session
                .docs
                .iter()
                .position(|d| d.is_dirty() && d.path.is_some())
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.session.active = i;
            self.bench.dialog = texcraft_ui::dialogs::Dialog::ConfirmClose(i);
        }

        self.bench.ui(ui, &mut self.session);
        self.handle_requests(&ctx);
        self.handle_deferred(&ctx);

        // Window title: document name.
        let title = self
            .session
            .doc()
            .map_or_else(|| "TexCraft".to_string(), |d| d.display_name());
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        #[cfg(target_os = "macos")]
        if let Some(m) = &mut self.menu {
            m.sync(&self.session);
        }
    }

    /// Installs a downloaded update once TexCraft has quit.
    fn on_exit(&mut self) {
        self.session.install_update_on_quit();
    }

    /// Transparent: the welcome window shows the system's translucent material through it
    /// (the opaque main window paints every pixel itself).
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let p = Persisted {
            view: Some(self.session.view.clone()),
            settings: Some(self.session.settings.clone()),
            recent: self.session.recent.clone(),
            split: Some(self.bench.split),
            version: PERSISTED_VERSION,
        };
        eframe::set_value(storage, STORAGE_KEY, &p);
    }
}

/// The app icon (assets/icon/texcraft-1024.png, rendered from texcraft.svg).
const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon/texcraft-1024.png");

fn app_icon() -> egui::IconData {
    match image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            egui::IconData {
                width: rgba.width(),
                height: rgba.height(),
                rgba: rgba.into_raw(),
            }
        }
        Err(_) => egui::IconData::default(),
    }
}

fn main() -> eframe::Result {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("texcraft: {e}");
            std::process::exit(2);
        }
    };
    let (w, h) = opts.size.unwrap_or((1388.0, 824.0));
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("TexCraft")
        .with_app_id("texcraft")
        .with_inner_size([w, h])
        .with_min_inner_size([760.0, 480.0])
        .with_icon(Arc::new(app_icon()));
    if cfg!(target_os = "macos") {
        // Transparent back buffers (egui-wgpu decides this for all windows from the main one)
        // let the welcome window show the system's translucent material; the main window paints
        // every pixel itself.
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_transparent(true);
    }
    let native = eframe::NativeOptions {
        viewport,
        persist_window: opts.screenshot.is_none() && opts.size.is_none(),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "TexCraft",
        native,
        Box::new(move |cc| Ok(Box::new(App::new(cc, opts)))),
    )
}
