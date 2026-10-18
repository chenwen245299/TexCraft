//! `texcraft-cli`: the TexCraft engine without a window. Every subcommand prints JSON so
//! scripts and agents can consume it.
//!
//! ```text
//! texcraft-cli typeset <file.tex> [--engine pdflatex|xelatex|lualatex|tectonic] [--live]
//! texcraft-cli outline <file.tex>
//! texcraft-cli issues <file.log> [--base DIR]
//! texcraft-cli forward <file.synctex.gz> <source.tex> <line>
//! texcraft-cli inverse <file.synctex.gz> <page> <x> <y>
//! texcraft-cli commands [--lang en|zh-Hans]
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use serde_json::{Value, json};
use texcraft_project::Engine;

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  texcraft-cli typeset <file.tex> [--engine pdflatex|xelatex|lualatex|tectonic] [--live]\n  texcraft-cli outline <file.tex>\n  texcraft-cli issues <file.log> [--base DIR]\n  texcraft-cli forward <file.synctex.gz> <source.tex> <line>\n  texcraft-cli inverse <file.synctex.gz> <page> <x> <y>\n  texcraft-cli commands [--lang en|zh-Hans]"
    );
    ExitCode::from(2)
}

fn issues_json(issues: &[texcraft_log::Issue]) -> Value {
    Value::Array(
        issues
            .iter()
            .map(|i| {
                json!({
                    "severity": format!("{:?}", i.severity),
                    "file": i.file,
                    "line": i.line,
                    "message": i.message,
                    "detail": i.detail,
                    "logLine": i.log_line,
                })
            })
            .collect(),
    )
}

fn typeset(args: &[String]) -> Result<Value, String> {
    let file = args.first().ok_or("typeset needs a file")?;
    let root = std::fs::canonicalize(file).map_err(|e| format!("{file}: {e}"))?;
    let text = std::fs::read_to_string(&root).map_err(|e| format!("{file}: {e}"))?;
    let plan = texcraft_project::plan(&text, false, &texcraft_project::Settings::default());
    let mut engine = plan.engine;
    let mut live = false;
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--engine" => {
                let e = it.next().ok_or("--engine needs a value")?;
                engine = Engine::from_program(e).ok_or_else(|| format!("unknown engine {e}"))?;
            }
            "--live" => live = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    let tc = texcraft_typeset::Toolchain::detect();
    // CJK text without its own setup: load xeCJK (Auto-Sense, as in the app).
    let pretex = (plan.add_cjk && engine == Engine::XeLatex)
        .then(|| tc.cjk_pretex())
        .flatten();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    let job = if live {
        let project_dir = root.parent().map(Path::to_path_buf).unwrap_or_default();
        let root_rel = PathBuf::from(root.file_name().ok_or("no file name")?);
        texcraft_typeset::start_live(
            texcraft_typeset::LiveRequest {
                project_dir,
                root_rel,
                buffers: Vec::new(),
                dirs: Vec::new(),
                engine,
                shell_escape: false,
                cache_preamble: true,
                pretex: pretex.clone(),
            },
            &tc,
            wake,
        )
    } else {
        texcraft_typeset::start(
            texcraft_typeset::Request {
                root: root.clone(),
                engine,
                shell_escape: false,
                bib_tool: texcraft_project::BibTool::Auto,
                pretex,
            },
            &tc,
            wake,
        )
    };
    let o = job.wait(|line| eprintln!("{line}"));
    Ok(json!({
        "success": o.success,
        "engine": engine.id(),
        "live": o.live,
        "seconds": o.duration.as_secs_f64(),
        "pdf": o.pdf,
        "synctex": o.synctex,
        "log": o.log_path,
        "errors": o.report.errors(),
        "warnings": o.report.warnings(),
        "badBoxes": o.report.bad_boxes(),
        "rerunSuggested": o.report.rerun_suggested,
        "preambleCached": o.preamble_cached,
        "fastEnv": o.fast_env,
        "error": o.error,
        "issues": issues_json(&o.report.issues),
    }))
}

fn outline(args: &[String]) -> Result<Value, String> {
    let file = args.first().ok_or("outline needs a file")?;
    let root = std::fs::canonicalize(file).map_err(|e| format!("{file}: {e}"))?;
    let read = |p: &Path| std::fs::read_to_string(p).ok();
    let idx = texcraft_engine::index::build(&root, &read);
    let items: Vec<Value> = idx
        .reading_order()
        .into_iter()
        .filter_map(|(fi, ii)| {
            let f = idx.files.get(fi)?;
            let i = f.outline.get(ii)?;
            Some(json!({ "kind": format!("{:?}", i.kind), "title": i.title, "file": f.path, "line": i.line }))
        })
        .collect();
    Ok(json!({
        "files": idx.files.iter().map(|f| json!({ "path": f.path, "missing": f.missing, "words": f.words })).collect::<Vec<_>>(),
        "words": idx.words,
        "bibliography": idx.bib.iter().map(|(e, _)| e.key.clone()).collect::<Vec<_>>(),
        "outline": items,
    }))
}

fn issues(args: &[String]) -> Result<Value, String> {
    let file = args.first().ok_or("issues needs a log file")?;
    let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
    let base = match args.get(1).map(String::as_str) {
        Some("--base") => PathBuf::from(args.get(2).ok_or("--base needs a directory")?),
        _ => Path::new(file)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default(),
    };
    let text = String::from_utf8_lossy(&bytes);
    let report = if file.ends_with(".blg") {
        texcraft_log::LogReport {
            issues: texcraft_log::parse_blg(&text, &base),
            ..Default::default()
        }
    } else {
        texcraft_log::parse_log(&text, &base)
    };
    Ok(json!({
        "errors": report.errors(),
        "warnings": report.warnings(),
        "badBoxes": report.bad_boxes(),
        "fatal": report.fatal,
        "rerunSuggested": report.rerun_suggested,
        "pages": report.output_pages,
        "issues": issues_json(&report.issues),
    }))
}

fn num<T: std::str::FromStr>(args: &[String], i: usize, what: &str) -> Result<T, String> {
    args.get(i)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("missing or bad {what}"))
}

fn forward(args: &[String]) -> Result<Value, String> {
    let st = texcraft_synctex::SyncTex::load(Path::new(
        args.first().ok_or("forward needs a .synctex file")?,
    ))
    .map_err(|e| e.to_string())?;
    let src = args.get(1).ok_or("forward needs a source file")?;
    let line: u32 = num(args, 2, "line")?;
    let rects: Vec<Value> = st
        .forward(Path::new(src), line)
        .into_iter()
        .map(
            |r| json!({ "page": r.page, "x": r.x, "y": r.y, "width": r.width, "height": r.height }),
        )
        .collect();
    Ok(Value::Array(rects))
}

fn inverse(args: &[String]) -> Result<Value, String> {
    let st = texcraft_synctex::SyncTex::load(Path::new(
        args.first().ok_or("inverse needs a .synctex file")?,
    ))
    .map_err(|e| e.to_string())?;
    let page: u32 = num(args, 1, "page")?;
    let x: f32 = num(args, 2, "x")?;
    let y: f32 = num(args, 3, "y")?;
    Ok(match st.inverse(page, x, y) {
        Some(l) => json!({ "file": l.file, "line": l.line, "column": l.column }),
        None => Value::Null,
    })
}

/// Every command; labels and menu titles in English unless `--lang` asks for another language.
fn commands(args: &[String]) -> Result<Value, String> {
    let mut lang = texcraft_i18n::Lang::En;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--lang" => {
                let tag = it.next().ok_or("--lang needs a value")?;
                lang = texcraft_i18n::Lang::from_tag(tag)
                    .ok_or_else(|| format!("unknown language {tag} (en, zh-Hans)"))?;
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    let _lang = texcraft_i18n::scoped(lang);
    Ok(Value::Array(
        texcraft_engine::registry()
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "label": c.label(),
                    "menu": c.menu.title(),
                    "shortcut": c.shortcut.map(texcraft_engine::Shortcut::display),
                    "params": c.params,
                })
            })
            .collect(),
    ))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        return usage();
    };
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match cmd.as_str() {
        "typeset" => typeset(rest),
        "outline" => outline(rest),
        "issues" => issues(rest),
        "forward" => forward(rest),
        "inverse" => inverse(rest),
        "commands" => commands(rest),
        "-h" | "--help" | "help" => return usage(),
        other => Err(format!("unknown subcommand {other}")),
    };
    match result {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            if v.get("success").and_then(Value::as_bool) == Some(false) {
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("texcraft-cli: {e}");
            ExitCode::from(2)
        }
    }
}
