//! Source Control (crates/ui/src/git_panel.rs).

pub(super) const ENTRIES: super::Entries = &[
    // Empty states
    ("No Project Folder", "没有项目文件夹"),
    (
        "Open a project folder to manage it with Git.",
        "打开项目文件夹以使用 Git 管理它。",
    ),
    ("Git Is Not Installed", "未安装 Git"),
    (
        "Install Git (git-scm.com, or the Xcode Command Line Tools on macOS) to use Source Control.",
        "安装 Git（git-scm.com，或 macOS 上的 Xcode 命令行工具）以使用源代码管理。",
    ),
    ("No Repository", "没有仓库"),
    (
        "The project folder is not a Git repository yet. Initialize one to track the history of your document.",
        "项目文件夹还不是 Git 仓库。初始化一个仓库以跟踪文稿的历史记录。",
    ),
    // Header: branch and actions
    ("{head} (detached)", "{head}（已分离）"),
    ("No commits yet", "尚无提交"),
    ("Pull {n} commit", "拉取 {n} 个提交"),
    ("Pull {n} commits", "拉取 {n} 个提交"),
    ("Push {n} commit", "推送 {n} 个提交"),
    ("Push {n} commits", "推送 {n} 个提交"),
    ("Publish Branch", "发布分支"),
    ("More Actions", "更多操作"),
    ("Show Repository in Explorer", "在资源管理器中显示仓库"),
    ("Reveal Repository in Finder", "在访达中显示仓库"),
    // Message box and groups
    (
        "Message (⌘↩ to commit on “{branch}”)",
        "提交信息（按 ⌘↩ 提交到“{branch}”）",
    ),
    ("Commit All", "全部提交"),
    ("Staged Changes", "已暂存的更改"),
    ("Changes", "更改"),
    ("No changes", "没有更改"),
    ("Commits", "提交记录"),
    ("Open File", "打开文件"),
    ("Show in Explorer", "在资源管理器中显示"),
    // Change kinds (texcraft-git's `ChangeKind::describe`, shown in tool tips)
    ("Modified", "已修改"),
    ("Index Added", "已添加到暂存区"),
    ("Deleted", "已删除"),
    ("Renamed", "已重新命名"),
    ("Copied", "已复制"),
    ("Type Changed", "类型已更改"),
    ("Untracked", "未跟踪"),
    ("Conflict", "冲突"),
    // Commit times
    ("just now", "刚刚"),
    ("{n} min ago", "{n} 分钟前"),
    ("{n} h ago", "{n} 小时前"),
    ("{n} day ago", "{n} 天前"),
    ("{n} days ago", "{n} 天前"),
    // Discard confirmation and the diff sheet
    (
        "Discard the changes in “{path}”?",
        "要放弃“{path}”中的更改吗？",
    ),
    (
        "The file is not in the repository yet: it will be moved to the Trash.",
        "此文件尚未加入仓库：它将被移到废纸篓。",
    ),
    (
        "The file goes back to its last committed (or staged) version. This cannot be undone.",
        "文件将恢复为上次提交（或暂存）时的版本。此操作无法撤销。",
    ),
    ("Cancel", "取消"),
    ("Discard", "放弃"),
    ("Close", "关闭"),
];
