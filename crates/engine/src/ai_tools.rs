//! Read-only project tools for the AI Assistant: the model may list, read and search the files
//! of the project folder, never change them. Paths are relative to the project folder and may
//! not leave it (`..` and symbolic links out of it are refused). Open documents are read as
//! they are in the editor, unsaved changes included.
//!
//! What goes to the model (descriptions, answers, errors) stays in English, as every prompt.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use texcraft_ai::{Tool, ToolCall};

use crate::Session;

/// The most lines one read returns.
const READ_MAX_LINES: usize = 1500;
/// The most characters one answer carries.
const ANSWER_MAX: usize = 60_000;
/// The most entries a listing shows.
const LIST_MAX: usize = 400;
/// The most matches a search returns.
const SEARCH_MAX: usize = 100;
/// Files larger than this are not searched.
const SEARCH_FILE_MAX: u64 = 2 * 1024 * 1024;
/// How deep a listing or search goes below its folder.
const DEPTH: usize = 6;

/// Which tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ToolKind {
    /// `list_files`.
    List,
    /// `read_file`.
    Read,
    /// `search_files`.
    Search,
}

/// What a tool call did, shown above the answer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolUse {
    /// The tool.
    pub kind: ToolKind,
    /// The file or folder, relative to the project (`""`: the project folder), or the text
    /// searched for.
    pub target: String,
    /// The lines read, when the model asked for a range.
    pub lines: Option<(usize, usize)>,
    /// The call failed (the model was told why).
    pub failed: bool,
}

/// The tools the Assistant offers the model.
pub fn specs() -> Vec<Tool> {
    vec![
        Tool {
            name: "list_files".into(),
            description: "List the files and folders of the LaTeX project (read-only). Paths are \
                relative to the project folder; folders end with '/'. Hidden files and \
                TexCraft's build folder are left out."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Folder to list, relative to the project folder (default: the whole project)."}
                }
            }),
        },
        Tool {
            name: "read_file".into(),
            description: "Read a text file of the project (read-only), with line numbers. Open \
                documents are read as they are in the editor, unsaved changes included. Long \
                files come in parts: ask again with start_line for more."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File path relative to the project folder, e.g. sections/intro.tex."},
                    "start_line": {"type": "integer", "description": "First line to read (1-based, default 1)."},
                    "end_line": {"type": "integer", "description": "Last line to read (default: up to 1500 lines from start_line)."}
                },
                "required": ["path"]
            }),
        },
        Tool {
            name: "search_files".into(),
            description: "Search the project's text files for a piece of text (case-insensitive, \
                read-only). Returns matching lines as path:line: text."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Text to look for, e.g. \\label{sec:intro} or a word."},
                    "path": {"type": "string", "description": "Folder or file to search in, relative to the project folder (default: the whole project)."}
                },
                "required": ["query"]
            }),
        },
    ]
}

/// What the model is told about the tools and about linking files.
pub const PROMPT: &str = "\n\nYou can look at the files of the user's project with the tools \
list_files, read_file and search_files (paths relative to the project folder). Use them when \
the question needs files other than the open document. They only read: you cannot change \
files, so give changes as LaTeX in code blocks. When you mention a file of the project, link \
it in Markdown with its path, as [intro.tex](sections/intro.tex), or to a line, as \
[intro.tex:42](sections/intro.tex#L42), so the user can open it with a click.";

/// `rel` inside `root` (a canonical folder): the canonical path, or why not.
pub(crate) fn resolve(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim();
    let p = Path::new(rel);
    let joined = if rel.is_empty() || rel == "." {
        root.to_path_buf()
    } else if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };
    let real = joined
        .canonicalize()
        .map_err(|_| format!("{rel}: no such file or folder in the project"))?;
    if !real.starts_with(root) {
        return Err(format!("{rel}: outside the project folder"));
    }
    Ok(real)
}

/// `path` relative to `root`, with `/` between parts (`""` for the root itself).
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|r| {
            r.components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

/// Hidden entries and TexCraft's build folder are skipped.
fn skipped(name: &str) -> bool {
    name.starts_with('.') || name == texcraft_project::TEMP_FOLDER
}

/// The entries of `dir` (no symbolic links: they may lead out of the project), sorted, folders
/// marked.
fn entries(dir: &Path) -> Vec<(PathBuf, bool)> {
    let mut out: Vec<(PathBuf, bool)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| !skipped(&e.file_name().to_string_lossy()))
                .filter_map(|e| {
                    let ty = e.file_type().ok()?;
                    (!ty.is_symlink()).then(|| (e.path(), ty.is_dir()))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Calls `f` for every entry below `dir` (depth first, up to [`DEPTH`]) until it returns false.
fn walk(dir: &Path, depth: usize, f: &mut dyn FnMut(&Path, bool, usize) -> bool) -> bool {
    for (path, is_dir) in entries(dir) {
        if !f(&path, is_dir, depth) {
            return false;
        }
        if is_dir && depth < DEPTH && !walk(&path, depth + 1, f) {
            return false;
        }
    }
    true
}

/// `text` cut to [`ANSWER_MAX`] characters, saying so.
fn capped(mut text: String) -> String {
    if let Some((cut, _)) = text.char_indices().nth(ANSWER_MAX) {
        text.truncate(cut);
        text.push_str("\n[answer cut here: ask for less]");
    }
    text
}

/// Whether `bytes` look like text (UTF-8 without NUL bytes).
fn is_text(bytes: &[u8]) -> bool {
    !bytes.iter().take(8000).any(|&b| b == 0) && std::str::from_utf8(bytes).is_ok()
}

impl Session {
    /// The folder the Assistant's tools see: the project folder, else the root file's
    /// (canonical).
    pub(crate) fn ai_project_root(&self) -> Option<PathBuf> {
        self.git_dir().and_then(|d| d.canonicalize().ok())
    }

    /// The text of `path` as the editor has it (unsaved changes included), else from disk.
    fn ai_read_text(&self, path: &Path) -> Result<String, String> {
        if let Some(d) = self.docs.iter().find(|d| {
            d.typeset_path()
                .and_then(|p| p.canonicalize().ok())
                .is_some_and(|p| p == path)
        }) {
            return Ok(d.text.clone());
        }
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read the file: {e}"))?;
        if !is_text(&bytes) {
            return Err(format!(
                "a binary file ({} bytes), not readable as text",
                bytes.len()
            ));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Runs `call` in the project folder `root` (read-only). Returns the answer for the model
    /// and what to show in the conversation.
    pub(crate) fn run_ai_tool(&self, root: &Path, call: &ToolCall) -> (String, ToolUse) {
        let args: Value = serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
        let arg = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let line = |k: &str| {
            args.get(k)
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
        };
        let (kind, target) = match call.name.as_str() {
            "list_files" => (ToolKind::List, arg("path")),
            "read_file" => (ToolKind::Read, arg("path")),
            "search_files" => (ToolKind::Search, arg("query")),
            other => {
                let used = ToolUse {
                    kind: ToolKind::Read,
                    target: other.to_string(),
                    lines: None,
                    failed: true,
                };
                return (
                    format!(
                        "Error: there is no tool named {other}; use list_files, read_file or search_files."
                    ),
                    used,
                );
            }
        };
        let mut used = ToolUse {
            kind,
            target: target.clone(),
            lines: None,
            failed: false,
        };
        let result = match kind {
            ToolKind::List => self.ai_list(root, &target),
            ToolKind::Read => {
                let (from, to) = (line("start_line"), line("end_line"));
                if from.is_some() || to.is_some() {
                    used.lines = Some((from.unwrap_or(1), to.unwrap_or(0)));
                }
                self.ai_read(root, &target, from, to)
                    .map(|(text, path, range)| {
                        used.target = path;
                        if used.lines.is_some() {
                            used.lines = Some(range);
                        }
                        text
                    })
            }
            ToolKind::Search => self.ai_search(root, &target, &arg("path")),
        };
        match result {
            Ok(text) => (capped(text), used),
            Err(e) => {
                used.failed = true;
                (format!("Error: {e}"), used)
            }
        }
    }

    fn ai_list(&self, root: &Path, rel: &str) -> Result<String, String> {
        let dir = resolve(root, rel)?;
        if !dir.is_dir() {
            return Err(format!("{rel}: not a folder (read it with read_file)"));
        }
        let mut out = String::new();
        let mut n = 0usize;
        let mut more = 0usize;
        walk(&dir, 0, &mut |path, is_dir, _| {
            if n >= LIST_MAX {
                more += 1;
                return true;
            }
            n += 1;
            let name = relative(root, path);
            if is_dir {
                out.push_str(&format!("{name}/\n"));
            } else {
                let size = std::fs::metadata(path).map_or(0, |m| m.len());
                out.push_str(&format!("{name}  ({size} bytes)\n"));
            }
            true
        });
        if n == 0 {
            out.push_str("(empty folder)\n");
        }
        if more > 0 {
            out.push_str(&format!(
                "… and {more} more: list a subfolder to see them\n"
            ));
        }
        Ok(out)
    }

    /// The numbered lines `from..=to` of `rel`, its relative path and the lines read.
    fn ai_read(
        &self,
        root: &Path,
        rel: &str,
        from: Option<usize>,
        to: Option<usize>,
    ) -> Result<(String, String, (usize, usize)), String> {
        if rel.trim().is_empty() {
            return Err("give the path of a file".into());
        }
        let path = resolve(root, rel)?;
        if path.is_dir() {
            return Err(format!("{rel}: a folder (list it with list_files)"));
        }
        let text = self.ai_read_text(&path)?;
        let name = relative(root, &path);
        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len();
        let first = from.unwrap_or(1).max(1);
        if total == 0 {
            return Ok((format!("{name} is empty.\n"), name, (1, 0)));
        }
        if first > total {
            return Err(format!("{name} has only {total} lines"));
        }
        let last = to
            .unwrap_or(usize::MAX)
            .min(first.saturating_add(READ_MAX_LINES - 1))
            .min(total)
            .max(first);
        let mut out = format!("{name} (lines {first}–{last} of {total}):\n");
        for (i, l) in lines
            .iter()
            .enumerate()
            .take(last)
            .skip(first.saturating_sub(1))
        {
            out.push_str(&format!("{:>5}  {l}\n", i + 1));
        }
        if last < total {
            out.push_str(&format!(
                "[{} more lines: read on with start_line {}]\n",
                total - last,
                last + 1
            ));
        }
        Ok((out, name, (first, last)))
    }

    fn ai_search(&self, root: &Path, query: &str, rel: &str) -> Result<String, String> {
        if query.trim().is_empty() {
            return Err("give the text to search for".into());
        }
        let start = resolve(root, rel)?;
        let needle = query.to_lowercase();
        let mut out = String::new();
        let mut hits = 0usize;
        let mut full = false;
        let mut search = |path: &Path| {
            let small = std::fs::metadata(path).is_ok_and(|m| m.len() <= SEARCH_FILE_MAX);
            let Ok(text) = (if small {
                self.ai_read_text(path)
            } else {
                Err(String::new())
            }) else {
                return true;
            };
            for (i, l) in text.lines().enumerate() {
                if l.to_lowercase().contains(&needle) {
                    if hits >= SEARCH_MAX {
                        full = true;
                        return false;
                    }
                    hits += 1;
                    let shown: String = l.trim().chars().take(200).collect();
                    out.push_str(&format!("{}:{}: {shown}\n", relative(root, path), i + 1));
                }
            }
            true
        };
        if start.is_file() {
            search(&start);
        } else {
            walk(&start, 0, &mut |path, is_dir, _| is_dir || search(path));
        }
        if hits == 0 {
            out.push_str(&format!("No matches for “{query}”.\n"));
        } else if full {
            out.push_str(&format!(
                "[stopped at {SEARCH_MAX} matches: search a subfolder or a longer text]\n"
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Arc;

    pub(crate) fn project(name: &str) -> (Session, PathBuf) {
        let dir = std::env::temp_dir().join(format!("texcraft-ai-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sections")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join(texcraft_project::TEMP_FOLDER)).unwrap();
        std::fs::write(
            dir.join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\\input{sections/intro}\n\\end{document}\n",
        )
        .unwrap();
        let intro: String = (1..=2000).map(|n| format!("Intro line {n}.\n")).collect();
        std::fs::write(dir.join("sections/intro.tex"), intro).unwrap();
        std::fs::write(dir.join("figure.pdf"), b"%PDF-1.5\n\0\x01binary").unwrap();
        let mut s = Session::new(Arc::new(|| {}));
        s.settings.live_typeset = false;
        s.set_project_dir(dir.clone());
        let root = s.ai_project_root().unwrap();
        (s, root)
    }

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            name: name.into(),
            arguments: args.to_string(),
        }
    }

    /// The tools read the project, in parts, as the editor has it; they cannot leave it.
    #[test]
    fn tools_read_the_project_and_stay_inside_it() {
        let (mut s, root) = project("tools");
        // Listing: hidden and build folders left out.
        let (list, used) = s.run_ai_tool(&root, &call("list_files", json!({})));
        assert!(!used.failed, "{list}");
        assert!(
            list.contains("main.tex  (") && list.contains("sections/\n"),
            "{list}"
        );
        assert!(list.contains("sections/intro.tex"), "{list}");
        assert!(
            !list.contains(".git") && !list.contains(".texcraft"),
            "{list}"
        );

        // Reading: numbered, in parts.
        let (text, used) = s.run_ai_tool(
            &root,
            &call("read_file", json!({"path": "sections/intro.tex"})),
        );
        assert!(
            text.starts_with("sections/intro.tex (lines 1–1500 of 2000):"),
            "{}",
            &text[..80]
        );
        assert!(text.contains("    1  Intro line 1.") && text.contains(" 1500  Intro line 1500."));
        assert!(text.contains("[500 more lines: read on with start_line 1501]"));
        assert_eq!(used.target, "sections/intro.tex");
        let (text, used) = s.run_ai_tool(
            &root,
            &call(
                "read_file",
                json!({"path": "sections/intro.tex", "start_line": 1999, "end_line": 5000}),
            ),
        );
        assert!(
            text.contains(" 2000  Intro line 2000.") && !text.contains("more lines"),
            "{text}"
        );
        assert_eq!(used.lines, Some((1999, 2000)));

        // An open document is read as the editor has it.
        let main = root.join("main.tex");
        s.open_path(&main).unwrap();
        s.run("edit.setText", &json!({"text": "Unsaved edit.\n"}))
            .unwrap();
        let (text, _) = s.run_ai_tool(&root, &call("read_file", json!({"path": "main.tex"})));
        assert!(text.contains("Unsaved edit."), "{text}");

        // Binary files, folders and missing files say why.
        for (args, why) in [
            (json!({"path": "figure.pdf"}), "binary"),
            (json!({"path": "sections"}), "a folder"),
            (json!({"path": "nope.tex"}), "no such file"),
        ] {
            let (text, used) = s.run_ai_tool(&root, &call("read_file", args));
            assert!(used.failed && text.contains(why), "{text}");
        }

        // Nothing outside the project: `..`, absolute paths, symbolic links.
        let outside = root
            .parent()
            .unwrap()
            .join(format!("texcraft-ai-outside-{}", std::process::id()));
        std::fs::write(&outside, "classified-content").unwrap();
        let name = outside.file_name().unwrap().to_string_lossy().to_string();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("link.tex")).unwrap();
        for path in [
            format!("../{name}"),
            outside.to_string_lossy().to_string(),
            "link.tex".into(),
        ] {
            let (text, used) = s.run_ai_tool(&root, &call("read_file", json!({"path": path})));
            assert!(
                used.failed && !text.contains("classified"),
                "{path}: {text}"
            );
        }
        let (list, _) = s.run_ai_tool(&root, &call("list_files", json!({})));
        assert!(
            !list.contains("link.tex"),
            "symbolic links are not listed: {list}"
        );
        let _ = std::fs::remove_file(&outside);

        // Search: case-insensitive, path:line, open documents as edited, binaries skipped.
        let (hits, _) = s.run_ai_tool(
            &root,
            &call("search_files", json!({"query": "INTRO LINE 42."})),
        );
        assert_eq!(hits, "sections/intro.tex:42: Intro line 42.\n");
        let (hits, _) = s.run_ai_tool(&root, &call("search_files", json!({"query": "unsaved"})));
        assert!(hits.starts_with("main.tex:1:"), "{hits}");
        let (hits, _) = s.run_ai_tool(&root, &call("search_files", json!({"query": "Intro line"})));
        assert!(hits.contains("[stopped at 100 matches"), "{hits}");
        let (hits, _) = s.run_ai_tool(&root, &call("search_files", json!({"query": "zebra"})));
        assert!(hits.contains("No matches"), "{hits}");

        // No other tools (nothing writes).
        let (text, used) = s.run_ai_tool(&root, &call("write_file", json!({"path": "main.tex"})));
        assert!(
            used.failed && text.contains("no tool named write_file"),
            "{text}"
        );
        assert_eq!(specs().len(), 3);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A local OpenAI-compatible server answering one request per entry of `answers` (whole
    /// SSE bodies); returns the requests it saw.
    pub(crate) fn fake_model(answers: Vec<String>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for body in answers {
                let (mut s, _) = listener.accept().unwrap();
                let mut req = Vec::new();
                let mut buf = [0u8; 65536];
                loop {
                    let n = s.read(&mut buf).unwrap();
                    req.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&req).to_string();
                    if let Some(h) = text.find("\r\n\r\n") {
                        let len = text
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if req.len() >= h + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                seen.push(String::from_utf8_lossy(&req).to_string());
            }
            seen
        });
        (port, handle)
    }

    pub(crate) fn sse(delta: Value, finish: Option<&str>) -> String {
        let chunk = json!({"choices": [{"delta": delta, "finish_reason": finish}]});
        format!("data: {chunk}\n\ndata: [DONE]\n\n")
    }

    /// The model reads a file, gets its lines back and answers with a link to it; the answer
    /// records what it read.
    #[test]
    fn the_assistant_reads_files_and_answers() {
        let (mut s, root) = project("loop");
        let first = sse(
            json!({"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
                "function": {"name": "read_file", "arguments": "{\"path\": \"sections/intro.tex\", \"end_line\": 3}"}}]}),
            Some("tool_calls"),
        );
        let second = sse(
            json!({"content": "It starts with [intro.tex:1](sections/intro.tex#L1)."}),
            Some("stop"),
        );
        let (port, server) = fake_model(vec![first, second]);
        let ollama = texcraft_ai::PRESETS
            .iter()
            .find(|p| p.id == "ollama")
            .unwrap();
        let i = s.ai.config.add(ollama);
        s.ai.config.providers[i].base_url = format!("http://127.0.0.1:{port}/v1");
        s.ai.config.providers[i].models = vec!["m".into()];
        s.ai.config.normalize();
        s.run("ai.send", &json!({"text": "How does the intro start?"}))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while s.ai.is_busy() && std::time::Instant::now() < deadline {
            s.tick(std::time::Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let m = s.ai.messages.last().unwrap();
        assert_eq!(m.error, None);
        assert_eq!(
            m.text,
            "It starts with [intro.tex:1](sections/intro.tex#L1)."
        );
        assert_eq!(
            m.tools,
            vec![ToolUse {
                kind: ToolKind::Read,
                target: "sections/intro.tex".into(),
                lines: Some((1, 3)),
                failed: false,
            }]
        );
        let seen = server.join().unwrap();
        assert!(
            seen[0].contains("\"tools\"") && seen[0].contains("read_file"),
            "tools offered"
        );
        assert!(
            seen[0].contains("list_files, read_file and search_files"),
            "and explained"
        );
        // The second request carries the call and the lines read.
        assert!(
            seen[1].contains("\"tool_call_id\":\"call_1\""),
            "{}",
            seen[1]
        );
        assert!(seen[1].contains("Intro line 3.") && !seen[1].contains("Intro line 4."));
        let _ = std::fs::remove_dir_all(&root);
    }
}
