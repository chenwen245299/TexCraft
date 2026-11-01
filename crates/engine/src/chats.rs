//! Saved AI conversations, kept per project in TexCraft's own data folder, never in the project
//! (a project sent to someone carries no chats). A project's conversations live in
//! `chats/<name>-<hash>/`, the hash taken from the project folder's path; `project.json` there
//! records that path, so [`ChatStore::sweep`] can remove the conversations of a project that
//! no longer exists. Conversations without a project folder go to `chats/_none/`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use texcraft_ai::{Image, Role};

use crate::ai::ChatMessage;
use crate::ai_tools::ToolUse;

/// The folder of conversations without a project.
const NO_PROJECT: &str = "_none";
/// The longest title (characters): the start of the first question.
const TITLE_CHARS: usize = 48;

/// One saved conversation in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSummary {
    /// Its id (the file name).
    pub id: String,
    /// The start of its first question.
    pub title: String,
    /// When it last changed (seconds since 1970).
    pub updated: u64,
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    path: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct StoredImage {
    name: String,
    media_type: String,
    /// Base64.
    data: String,
}

#[derive(Serialize, Deserialize)]
struct StoredMessage {
    role: Role,
    text: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    images: Vec<StoredImage>,
    #[serde(default)]
    tools: Vec<ToolUse>,
}

#[derive(Serialize, Deserialize)]
struct StoredChat {
    messages: Vec<StoredMessage>,
}

/// Where conversations are kept (TexCraft's data folder's `chats`).
#[derive(Debug, Clone)]
pub struct ChatStore {
    root: PathBuf,
}

/// FNV-1a: a hash that stays the same from one build to the next (std's does not promise to).
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A conversation id: only letters and digits (it names a file).
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Writes `bytes` to `path` through a temporary file (a crash leaves the old file whole).
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("json.part");
    std::fs::write(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("cannot save the conversation: {e}")
        })
}

/// A new conversation id: the time in milliseconds and a counter, in hex.
pub fn new_id() -> String {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) & 0xffff;
    format!("{ms:x}{n:04x}")
}

/// The title of a conversation: its first question's first line, shortened.
fn title(messages: &[ChatMessage]) -> String {
    let first = messages
        .iter()
        .find(|m| m.role == Role::User)
        .map(|m| {
            m.text
                .trim()
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_default();
    if first.chars().count() <= TITLE_CHARS {
        return first;
    }
    let cut: String = first.chars().take(TITLE_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

impl ChatStore {
    /// Conversations kept under `root`.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The folder of `project`'s conversations (`None`: no project folder).
    fn dir(&self, project: Option<&Path>) -> PathBuf {
        let Some(project) = project else {
            return self.root.join(NO_PROJECT);
        };
        let name: String = project
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .chars()
            .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
            .take(32)
            .collect();
        let hash = fnv(&project.to_string_lossy());
        self.root.join(format!("{name}-{hash:016x}"))
    }

    /// `project`'s conversations, the latest first.
    pub fn list(&self, project: Option<&Path>) -> Vec<ChatSummary> {
        let mut list: Vec<ChatSummary> =
            std::fs::read_to_string(self.dir(project).join("index.json"))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
        list.sort_by_key(|c| std::cmp::Reverse(c.updated));
        list
    }

    /// Conversation `id` of `project`.
    pub fn load(&self, project: Option<&Path>, id: &str) -> Option<Vec<ChatMessage>> {
        if !valid_id(id) {
            return None;
        }
        let text = std::fs::read_to_string(self.dir(project).join(format!("{id}.json"))).ok()?;
        let chat: StoredChat = serde_json::from_str(&text).ok()?;
        Some(
            chat.messages
                .into_iter()
                .map(|m| ChatMessage {
                    role: m.role,
                    text: m.text,
                    blocks: Vec::new(),
                    model: m.model,
                    provider: m.provider,
                    pending: false,
                    thinking: false,
                    error: m.error,
                    note: m.note,
                    images: m
                        .images
                        .iter()
                        .filter_map(|i| Image::from_base64(&i.name, &i.media_type, &i.data))
                        .collect(),
                    tools: m.tools,
                })
                .collect(),
        )
    }

    /// Saves conversation `id` of `project` (an answer still arriving is left out).
    pub fn save(
        &self,
        project: Option<&Path>,
        id: &str,
        messages: &[ChatMessage],
    ) -> Result<(), String> {
        if !valid_id(id) {
            return Err(format!("not a conversation id: {id}"));
        }
        let dir = self.dir(project);
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot save the conversation: {e}"))?;
        if let Some(path) = project {
            let file = ProjectFile {
                path: path.to_path_buf(),
            };
            let json = serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?;
            write_atomic(&dir.join("project.json"), &json)?;
        }
        let chat = StoredChat {
            messages: messages
                .iter()
                .filter(|m| !m.pending)
                .map(|m| StoredMessage {
                    role: m.role,
                    text: m.text.clone(),
                    model: m.model.clone(),
                    provider: m.provider.clone(),
                    error: m.error.clone(),
                    note: m.note.clone(),
                    images: m
                        .images
                        .iter()
                        .map(|i| StoredImage {
                            name: i.name.clone(),
                            media_type: i.media_type.clone(),
                            data: i.base64(),
                        })
                        .collect(),
                    tools: m.tools.clone(),
                })
                .collect(),
        };
        let json = serde_json::to_vec(&chat).map_err(|e| e.to_string())?;
        write_atomic(&dir.join(format!("{id}.json")), &json)?;
        let mut list = self.list(project);
        list.retain(|c| c.id != id);
        list.push(ChatSummary {
            id: id.to_string(),
            title: title(messages),
            updated: now_secs(),
        });
        let json = serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?;
        write_atomic(&dir.join("index.json"), &json)
    }

    /// Deletes conversation `id` of `project`.
    pub fn delete(&self, project: Option<&Path>, id: &str) -> Result<(), String> {
        if !valid_id(id) {
            return Err(format!("not a conversation id: {id}"));
        }
        let dir = self.dir(project);
        let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
        let mut list = self.list(project);
        list.retain(|c| c.id != id);
        let json = serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?;
        write_atomic(&dir.join("index.json"), &json)
    }

    /// Removes the conversations of projects that no longer exist (deleted or moved away).
    /// Returns how many projects' conversations went.
    pub fn sweep(&self) -> usize {
        let Ok(rd) = std::fs::read_dir(&self.root) else {
            return 0;
        };
        let mut removed = 0;
        for e in rd.filter_map(Result::ok) {
            let dir = e.path();
            let Some(file) = std::fs::read_to_string(dir.join("project.json"))
                .ok()
                .and_then(|s| serde_json::from_str::<ProjectFile>(&s).ok())
            else {
                continue;
            };
            if !file.path.is_dir() && std::fs::remove_dir_all(&dir).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: Role, text: &str) -> ChatMessage {
        ChatMessage {
            role,
            text: text.into(),
            blocks: Vec::new(),
            model: Some("m".into()),
            provider: None,
            pending: false,
            thinking: false,
            error: None,
            note: None,
            images: Vec::new(),
            tools: Vec::new(),
        }
    }

    /// Conversations are kept per project outside it, listed latest first, read back whole,
    /// deleted, and swept away with their project.
    #[test]
    fn conversations_live_outside_the_project_and_go_with_it() {
        let base = std::env::temp_dir().join(format!("texcraft-chats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let project = base.join("paper");
        let other = base.join("thesis");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let store = ChatStore::new(base.join("data/chats"));

        let mut first = vec![
            message(
                Role::User,
                "Why does \\ref{fig:a} print ??\nThe figure is there.",
            ),
            message(Role::Assistant, "Typeset twice."),
        ];
        first[0].images.push(Image {
            media_type: "image/png".into(),
            data: vec![1u8, 2, 3].into(),
            name: "shot.png".into(),
        });
        first[1].tools.push(ToolUse {
            kind: crate::ai_tools::ToolKind::Read,
            target: "main.tex".into(),
            lines: None,
            failed: false,
        });
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        store.save(Some(&project), &a, &first).unwrap();
        let long = "x".repeat(100);
        store
            .save(Some(&project), &b, &[message(Role::User, &long)])
            .unwrap();
        store
            .save(Some(&other), &a, &[message(Role::User, "Other project")])
            .unwrap();

        // Nothing in the project folder.
        assert_eq!(std::fs::read_dir(&project).unwrap().count(), 0);
        let list = store.list(Some(&project));
        assert_eq!(list.len(), 2);
        assert!(
            list.iter()
                .any(|c| c.title == "Why does \\ref{fig:a} print ??")
        );
        assert!(
            list.iter()
                .any(|c| c.title.chars().count() == TITLE_CHARS && c.title.ends_with('…'))
        );
        let back = store.load(Some(&project), &a).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].text, first[0].text);
        assert_eq!(back[0].images, first[0].images);
        assert_eq!(back[1].tools, first[1].tools);
        assert_eq!(store.list(Some(&other))[0].title, "Other project");
        // Ids name files: nothing else is read or written.
        assert!(store.load(Some(&project), "../index").is_none());
        assert!(store.save(Some(&project), "../x", &first).is_err());

        store.delete(Some(&project), &b).unwrap();
        assert_eq!(store.list(Some(&project)).len(), 1);

        // The project is deleted: its conversations go; the other project's stay.
        std::fs::remove_dir_all(&project).unwrap();
        assert_eq!(store.sweep(), 1);
        assert!(store.list(Some(&project)).is_empty());
        assert_eq!(store.list(Some(&other)).len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The Assistant saves each conversation as it goes, lists the project's history, starts
    /// afresh and reopens; another project has its own history; nothing goes into a project.
    #[test]
    fn the_assistant_keeps_conversations_per_project() {
        use crate::Session;
        use crate::ai_tools::tests::{fake_model, sse};
        use serde_json::json;
        let base =
            std::env::temp_dir().join(format!("texcraft-chats-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (a, b) = (base.join("paper"), base.join("thesis"));
        for d in [&a, &b] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("main.tex"), "Hello\n").unwrap();
        }
        let mut s = Session::new(std::sync::Arc::new(|| {}));
        s.settings.live_typeset = false;
        s.update.url = "/nonexistent/texcraft-latest.json".into();
        s.set_chat_store(base.join("data/chats"));
        s.set_project_dir(a.clone());
        s.tick(std::time::Instant::now());
        let (port, server) =
            fake_model(vec![sse(json!({"content": "First answer."}), Some("stop"))]);
        let ollama = texcraft_ai::PRESETS
            .iter()
            .find(|p| p.id == "ollama")
            .unwrap();
        let i = s.ai.config.add(ollama);
        s.ai.config.providers[i].base_url = format!("http://127.0.0.1:{port}/v1");
        s.ai.config.providers[i].models = vec!["m".into()];
        s.ai.config.normalize();
        s.run("ai.send", &json!({"text": "First question"}))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while s.ai.is_busy() && std::time::Instant::now() < deadline {
            s.tick(std::time::Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        server.join().unwrap();
        let id = s.ai.chat_id.clone().unwrap();
        assert_eq!(s.ai.history.len(), 1);
        assert_eq!(s.ai.history[0].title, "First question");

        // New conversation: empty, the history keeps the first; it opens again whole.
        s.run("ai.clear", &json!(null)).unwrap();
        assert!(s.ai.messages.is_empty() && s.ai.chat_id.is_none());
        assert_eq!(s.ai.history.len(), 1);
        s.run("ai.openChat", &json!({"id": id})).unwrap();
        assert_eq!(s.ai.messages.len(), 2);
        assert_eq!(s.ai.messages[1].text, "First answer.");

        // Another project: its own (empty) history; back again, the first one's.
        s.set_project_dir(b.clone());
        s.tick(std::time::Instant::now());
        assert!(s.ai.messages.is_empty() && s.ai.history.is_empty());
        s.set_project_dir(a.clone());
        s.tick(std::time::Instant::now());
        assert_eq!(s.ai.history.len(), 1);

        s.run("ai.deleteChat", &json!({"id": id})).unwrap();
        assert!(s.ai.history.is_empty());
        for d in [&a, &b] {
            let names: Vec<_> = std::fs::read_dir(d)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(names, vec!["main.tex"], "nothing written in {d:?}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
