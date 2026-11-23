//! The desktop app shell (apps/texcraft: native menu bar extras, file panels, alerts).

pub(super) const ENTRIES: super::Entries = &[
    // Menu bar items AppKit provides (muda's predefined items) and submenus
    ("Services", "服务"),
    ("Hide TexCraft", "隐藏 TexCraft"),
    ("Hide Others", "隐藏其他"),
    ("Show All", "全部显示"),
    ("Open Recent", "打开最近使用"),
    ("No Recent Documents", "没有最近使用的文稿"),
    ("New from Template", "从模板新建"),
    ("Template Gallery…", "模板库…"),
    ("Minimize", "最小化"),
    ("Zoom", "缩放"),
    ("Enter Full Screen", "进入全屏幕"),
    ("Bring All to Front", "前置全部窗口"),
    // File panels and alerts
    ("Cannot open", "无法打开"),
    ("Cannot open a new window", "无法打开新窗口"),
    (
        "{path} is saved, but TexCraft could not open a window for it ({e}). Open it from File → Open Recent.",
        "{path} 已保存，但 TexCraft 无法为它打开窗口（{e}）。可从“文件 → 打开最近使用”打开它。",
    ),
    ("TeX documents", "TeX 文稿"),
    ("All files", "所有文件"),
    ("TeX document", "TeX 文稿"),
];
