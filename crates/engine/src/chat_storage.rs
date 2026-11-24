//! Opt-in conversation storage. Switching stores copies history off the UI thread and keeps
//! the source intact; new messages immediately use the selected store, including mid-answer.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use texcraft_i18n::{tr, trf};

use crate::chats::{ChatStore, icloud_drive};
use crate::{CommandError, Session};

/// Storage settings shared by project windows through the local AI configuration.
pub struct ChatStorage {
    local: PathBuf,
    home: Option<PathBuf>,
    applied: Option<(bool, PathBuf)>,
    /// Imports not yet completed. A changed preference cancels the old worker, retaining its
    /// sources for the next one so a partial copy never loses the remaining history.
    sources: Vec<PathBuf>,
    job: Option<Transfer>,
    /// Why a copy could not finish. The source remains available for Retry.
    pub error: Option<String>,
}

impl ChatStorage {
    /// Whether iCloud Drive is available on this Mac (not an assertion about upload progress).
    pub fn icloud_available(&self) -> bool {
        self.home.as_deref().and_then(icloud_drive).is_some()
    }

    /// A storage copy is in progress.
    pub fn is_busy(&self) -> bool {
        self.job.is_some()
    }

    /// Whether the selected store is in iCloud Drive.
    pub fn uses_icloud(&self) -> bool {
        self.applied
            .as_ref()
            .is_some_and(|(_, root)| *root != self.local)
    }

    /// A readable location, with the user's home abbreviated as in Finder.
    pub fn display_path(&self, path: &Path) -> String {
        if let Some(home) = &self.home
            && let Ok(rest) =
                path.strip_prefix(home.join("Library/Mobile Documents/com~apple~CloudDocs"))
        {
            return format!("iCloud Drive/{}", rest.display());
        }
        self.home
            .as_deref()
            .and_then(|home| path.strip_prefix(home).ok())
            .map_or_else(
                || path.display().to_string(),
                |rest| format!("~/{}", rest.display()),
            )
    }
}

struct Transfer {
    cancel: Arc<AtomicBool>,
    result: Receiver<Result<(), String>>,
}

impl Drop for Transfer {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Session {
    /// Configures the local fallback and iCloud anchor once at startup. Default/legacy
    /// settings stay local; automatic-cloud histories from older releases are copied back.
    pub fn configure_chat_storage(&mut self, local: PathBuf, home: Option<PathBuf>) {
        self.ai.storage = Some(ChatStorage {
            local,
            home,
            applied: None,
            sources: Vec::new(),
            job: None,
            error: None,
        });
        self.ai_follow_chat_storage();
    }

    /// Applies the chosen store now, also when another project window changes the setting or
    /// iCloud Drive becomes available. Copies run in the background; drafts/answers stay put.
    pub fn ai_follow_chat_storage(&mut self) {
        let Some(storage) = self.ai.storage.as_mut() else {
            return;
        };
        let enabled = self.ai.config.sync_chats_icloud;
        let store = ChatStore::locate(storage.local.clone(), storage.home.as_deref(), enabled);
        let applied = (enabled, store.root().to_path_buf());
        if storage.applied.as_ref() == Some(&applied) {
            return;
        }
        storage.job = None;
        if let Some(previous) = &self.ai.chats {
            storage.sources.push(previous.root().to_path_buf());
        } else {
            storage.sources.push(storage.local.clone());
        }
        if self.ai.config.chat_storage_version == 0
            && !enabled
            && let Some(cloud) = storage.home.as_deref().and_then(icloud_drive)
        {
            // A read-only import from the old implicit cloud location, once. Turning sync
            // off thereafter never polls or imports the cloud history again.
            storage.sources.push(cloud.join("TexCraft/AI Assistant"));
        }
        storage.sources.sort();
        storage.sources.dedup();
        let sources: Vec<PathBuf> = storage
            .sources
            .iter()
            .filter(|path| path.as_path() != store.root())
            .cloned()
            .collect();
        storage.applied = Some(applied);
        storage.error = None;
        self.set_chats(store.clone());
        self.ai_save_chat();

        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let (tx, result) = mpsc::channel();
        let wake = self.waker.clone();
        let spawned = std::thread::Builder::new()
            .name("texcraft-chat-storage".into())
            .spawn(move || {
                let copied = store.prepare(&sources, &stop);
                let _ = tx.send(copied);
                wake();
            });
        if let Some(storage) = &mut self.ai.storage {
            match spawned {
                Ok(_) => storage.job = Some(Transfer { cancel, result }),
                Err(e) => storage.error = Some(trf!("Could not copy conversations: {e}", e = e)),
            }
        }
    }

    /// Changes the local preference through `ai.setICloudSync`. An unavailable cloud cannot
    /// be opted into, but an existing opt-in can always be turned off.
    pub fn ai_set_icloud_sync(&mut self, enabled: bool) -> Result<(), CommandError> {
        let Some(storage) = self.ai.storage.as_ref() else {
            return Err(CommandError::Failed(
                tr("Conversation storage is not configured.").into(),
            ));
        };
        if enabled && !storage.icloud_available() {
            return Err(CommandError::Failed(
                tr("Enable iCloud Drive in macOS System Settings to sync conversations.").into(),
            ));
        }
        let previous = self.ai.config.sync_chats_icloud;
        let previous_version = self.ai.config.chat_storage_version;
        self.ai.config.sync_chats_icloud = enabled;
        if previous && !enabled {
            // Persist the unfinished import before copying: closing the app during a large
            // transfer must resume it on the next launch instead of hiding uncopied history.
            self.ai.config.chat_storage_version = 0;
        }
        if let Err(e) = self.save_ai_config() {
            self.ai.config.sync_chats_icloud = previous;
            self.ai.config.chat_storage_version = previous_version;
            return Err(e);
        }
        // The same value also retries a failed copy.
        if let Some(storage) = &mut self.ai.storage
            && storage.error.is_some()
        {
            storage.applied = None;
        }
        self.ai_follow_chat_storage();
        Ok(())
    }

    pub(crate) fn tick_chat_storage(&mut self) -> Option<Duration> {
        let job = self.ai.storage.as_ref()?.job.as_ref()?;
        let result = match job.result.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return Some(Duration::from_millis(50)),
            Err(TryRecvError::Disconnected) => {
                Err(tr("Conversation storage stopped unexpectedly.").into())
            }
        };
        if let Some(storage) = &mut self.ai.storage {
            storage.job = None;
            match result {
                Ok(()) => storage.sources.clear(),
                Err(e) => {
                    storage.error = Some(trf!("Could not copy conversations: {e}", e = e));
                    return None;
                }
            }
        }
        self.ai_refresh_history();
        if self.ai.config.chat_storage_version == 0 {
            self.ai.config.chat_storage_version = 1;
            if let Err(e) = self.save_ai_config()
                && let Some(storage) = &mut self.ai.storage
            {
                storage.error = Some(e.to_string());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::ChatMessage;
    use crate::ai_usage::ChatUsage;
    use serde_json::json;
    use std::time::Instant;
    use texcraft_ai::{Config, Image, Role};

    fn message(text: &str) -> ChatMessage {
        ChatMessage {
            source: None,
            role: Role::User,
            text: text.into(),
            blocks: Vec::new(),
            model: None,
            provider: None,
            pending: false,
            thinking: false,
            error: None,
            note: None,
            images: vec![Image {
                name: "test.png".into(),
                media_type: "image/png".into(),
                data: b"synthetic image bytes".to_vec().into(),
            }],
            tools: Vec::new(),
        }
    }

    fn setup(name: &str, cloud: bool) -> (PathBuf, PathBuf, Session) {
        let base = std::env::temp_dir().join(format!(
            "texcraft-chat-storage-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("home");
        std::fs::create_dir_all(&home).unwrap();
        if cloud {
            std::fs::create_dir_all(home.join("Library/Mobile Documents/com~apple~CloudDocs"))
                .unwrap();
        }
        let mut s = Session::empty(Arc::new(|| {}));
        s.settings.live_typeset = false;
        s.load_ai_config(base.join("data/ai.json"));
        s.configure_chat_storage(base.join("data/chats"), Some(home.clone()));
        wait(&mut s);
        (base, home, s)
    }

    fn wait(s: &mut Session) {
        let start = Instant::now();
        while s.ai.storage.as_ref().is_some_and(ChatStorage::is_busy) {
            assert!(start.elapsed() < Duration::from_secs(5));
            s.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn set(s: &mut Session, enabled: bool) {
        s.run("ai.setICloudSync", &json!({"enabled": enabled}))
            .unwrap();
        wait(s);
        assert!(s.ai.storage.as_ref().unwrap().error.is_none());
    }

    #[test]
    fn switching_copies_history_and_images_keeps_the_draft_and_stops_cloud_writes() {
        let (base, home, mut s) = setup("switch", true);
        assert!(!s.ai.config.sync_chats_icloud);
        assert!(!s.ai.storage.as_ref().unwrap().uses_icloud());
        s.ai.messages = vec![message("My question")];
        let mut answer = message("Partial answer");
        answer.role = Role::Assistant;
        answer.pending = true;
        answer.images.clear();
        s.ai.messages.push(answer);
        s.ai.draft = "Unsent question".into();
        s.ai_save_chat();
        let id = s.ai.chat_id.clone().unwrap();
        let local = s.ai.chats.clone().unwrap();
        set(&mut s, true);
        let cloud = s.ai.chats.clone().unwrap();
        assert_ne!(cloud.root(), local.root());
        assert_eq!(
            s.ai.storage.as_ref().unwrap().display_path(cloud.root()),
            "iCloud Drive/TexCraft/AI Assistant"
        );
        assert_eq!(s.ai.chat_id.as_deref(), Some(id.as_str()));
        assert_eq!(s.ai.draft, "Unsent question");
        assert!(
            s.ai.messages[1].pending,
            "switching storage leaves the answer in progress"
        );
        assert_eq!(
            cloud.load(None, &id).unwrap().0[0].images,
            s.ai.messages[0].images
        );
        cloud
            .save(
                None,
                "archived",
                &[message("Cloud history")],
                &ChatUsage::default(),
            )
            .unwrap();

        set(&mut s, false);
        assert_eq!(s.ai.chats.as_ref().unwrap().root(), local.root());
        assert_eq!(
            local.load(None, "archived").unwrap().0[0].text,
            "Cloud history"
        );
        s.ai.messages[0].text = "Edited locally".into();
        s.ai_save_chat();
        local
            .save(
                None,
                "archived",
                &[message("Updated offline history")],
                &ChatUsage::default(),
            )
            .unwrap();
        assert_eq!(cloud.load(None, &id).unwrap().0[0].text, "My question");
        assert_eq!(
            cloud.load(None, "archived").unwrap().0[0].text,
            "Cloud history"
        );

        set(&mut s, true);
        assert_eq!(cloud.load(None, &id).unwrap().0[0].text, "Edited locally");
        assert_eq!(
            cloud.load(None, "archived").unwrap().0[0].text,
            "Updated offline history"
        );
        assert!(Config::load(&base.join("data/ai.json")).sync_chats_icloud);
        let mut restarted = Session::empty(Arc::new(|| {}));
        restarted.load_ai_config(base.join("data/ai.json"));
        restarted.configure_chat_storage(base.join("data/chats"), Some(home));
        wait(&mut restarted);
        assert!(restarted.ai.storage.as_ref().unwrap().uses_icloud());
        assert_eq!(restarted.ai.history.len(), 2);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn legacy_automatic_cloud_history_is_imported_once_without_enabling_sync() {
        let (base, home, mut s) = setup("legacy", true);
        let cloud = ChatStore::locate(base.join("data/chats"), Some(&home), true);
        cloud
            .save(
                None,
                "legacy",
                &[message("Existing cloud chat")],
                &ChatUsage::default(),
            )
            .unwrap();
        // A configuration written before the opt-in preference existed.
        std::fs::write(base.join("data/ai.json"), "{}").unwrap();
        s.load_ai_config(base.join("data/ai.json"));
        s.configure_chat_storage(base.join("data/chats"), Some(home.clone()));
        wait(&mut s);
        assert!(!s.ai.config.sync_chats_icloud);
        assert_eq!(s.ai.history.len(), 1);
        assert_eq!(
            s.ai.chats.as_ref().unwrap().load(None, "legacy").unwrap().0[0].images,
            cloud.load(None, "legacy").unwrap().0[0].images
        );
        assert!(
            cloud.load(None, "legacy").is_some(),
            "cloud copy is retained"
        );

        cloud
            .save(
                None,
                "newremote",
                &[message("Not synced while off")],
                &ChatUsage::default(),
            )
            .unwrap();
        s.load_ai_config(base.join("data/ai.json"));
        s.configure_chat_storage(base.join("data/chats"), Some(home));
        wait(&mut s);
        assert_eq!(
            s.ai.history.len(),
            1,
            "off must not keep importing cloud changes"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn other_windows_follow_the_sync_preference() {
        let (base, home, mut first) = setup("windows", true);
        let mut second = Session::empty(Arc::new(|| {}));
        second.load_ai_config(base.join("data/ai.json"));
        second.configure_chat_storage(base.join("data/chats"), Some(home));
        wait(&mut second);
        set(&mut first, true);
        second.ai_follow_config_file();
        wait(&mut second);
        assert!(second.ai.config.sync_chats_icloud);
        assert!(second.ai.storage.as_ref().unwrap().uses_icloud());
        set(&mut second, false);
        first.ai_follow_config_file();
        wait(&mut first);
        assert!(!first.ai.config.sync_chats_icloud);
        assert!(!first.ai.storage.as_ref().unwrap().uses_icloud());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn unavailable_icloud_cannot_be_enabled_and_existing_opt_in_can_be_disabled() {
        let (base, home, mut s) = setup("unavailable", false);
        assert!(
            s.run("ai.setICloudSync", &json!({"enabled": true}))
                .is_err()
        );
        assert!(!s.ai.config.sync_chats_icloud);
        assert!(
            s.run("ai.setICloudSync", &json!({"enabled": "true"}))
                .is_err()
        );
        s.ai.config.sync_chats_icloud = true;
        s.save_ai_config().unwrap();
        wait(&mut s);
        assert!(!s.ai.storage.as_ref().unwrap().uses_icloud());
        set(&mut s, false);
        assert!(!s.ai.config.sync_chats_icloud);
        assert!(
            !home
                .join("Library/Mobile Documents/com~apple~CloudDocs")
                .exists()
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_failed_copy_keeps_the_original_and_can_be_retried() {
        let (base, home, mut s) = setup("retry", true);
        s.ai.messages = vec![message("Keep me")];
        s.ai_save_chat();
        let id = s.ai.chat_id.clone().unwrap();
        let local = s.ai.chats.clone().unwrap();
        let blocked = icloud_drive(&home).unwrap().join("TexCraft");
        std::fs::write(&blocked, "not a folder").unwrap();
        s.run("ai.setICloudSync", &json!({"enabled": true}))
            .unwrap();
        wait(&mut s);
        assert!(s.ai.storage.as_ref().unwrap().error.is_some());
        assert_eq!(local.load(None, &id).unwrap().0[0].text, "Keep me");
        assert_eq!(s.ai.messages[0].text, "Keep me");
        std::fs::remove_file(blocked).unwrap();
        set(&mut s, true);
        assert_eq!(
            s.ai.chats.as_ref().unwrap().load(None, &id).unwrap().0[0].images,
            s.ai.messages[0].images
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn disabling_sync_resumes_an_unfinished_import_after_restart() {
        let (base, home, mut s) = setup("resume", true);
        set(&mut s, true);
        let cloud = s.ai.chats.clone().unwrap();
        s.run("ai.setICloudSync", &json!({"enabled": false}))
            .unwrap();
        assert!(!s.ai.storage.as_ref().unwrap().uses_icloud());
        let saved = Config::load(&base.join("data/ai.json"));
        assert!(!saved.sync_chats_icloud);
        assert_eq!(saved.chat_storage_version, 0);
        drop(s);
        cloud
            .save(
                None,
                "remaining",
                &[message("Not copied before exit")],
                &ChatUsage::default(),
            )
            .unwrap();
        let mut s = Session::empty(Arc::new(|| {}));
        s.load_ai_config(base.join("data/ai.json"));
        s.configure_chat_storage(base.join("data/chats"), Some(home));
        wait(&mut s);
        assert!(!s.ai.config.sync_chats_icloud);
        assert_eq!(s.ai.config.chat_storage_version, 1);
        assert_eq!(
            s.ai.chats
                .as_ref()
                .unwrap()
                .load(None, "remaining")
                .unwrap()
                .0[0]
                .text,
            "Not copied before exit"
        );
        let _ = std::fs::remove_dir_all(base);
    }
}
