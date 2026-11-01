//! The editor's right-click AI actions (crates/ai/src/actions.rs, crates/engine/src/ai_actions.rs,
//! the AI submenu in crates/ui/src/editor.rs, Settings → AI → Actions in crates/ui/src/prefs.rs,
//! the result card in crates/ui/src/ai_card.rs).

pub(super) const ENTRIES: super::Entries = &[
    // Built-in actions (crates/ai actions.rs)
    ("Polish", "润色"),
    ("Grammar Check", "语法检查"),
    ("Shorten", "精简"),
    ("Translate to English", "翻译为英文"),
    ("Explain", "解释"),
    ("Untitled Action", "未命名操作"),
    // Commands (engine commands.rs)
    ("Run AI Action", "运行 AI 操作"),
    ("Replace with the Rewrite", "用改写替换原文"),
    ("Edit AI Actions…", "编辑 AI 操作…"),
    ("Copy Result", "拷贝结果"),
    ("Try Again", "重试"),
    ("Discard Result", "放弃结果"),
    ("Continue in Assistant", "在 AI 助手中继续"),
    ("No AI action has run.", "还没有运行 AI 操作。"),
    // Running and applying (engine ai_actions.rs)
    ("No AI action “{id}”.", "没有 AI 操作“{id}”。"),
    (
        "“{name}” has no prompt: write one in Settings → AI → Actions.",
        "“{name}”没有提示词：请在“设置 → AI → 操作”中填写。",
    ),
    (
        "Select some text first, or put the caret in a paragraph.",
        "请先选择文本，或将光标放在一个段落中。",
    ),
    (
        "That answer is not a rewrite of the document's text.",
        "这个回答不是对文稿文本的改写。",
    ),
    ("Wait for the answer first.", "请先等待回答完成。"),
    (
        "The answer has no text to apply.",
        "回答中没有可应用的文本。",
    ),
    (
        "The document of that text is no longer open.",
        "那段文本所在的文稿已不再打开。",
    ),
    (
        "That text is no longer in the document: it changed after the action ran.",
        "那段文本已不在文稿中：它在操作运行后被改动了。",
    ),
    // The right-click menu (editor.rs) and the Assistant (ai_panel.rs)
    ("No Actions", "没有操作"),
    // Settings → AI → Actions (prefs.rs)
    ("Providers", "服务商"),
    ("Actions", "操作"),
    ("Replaces the text", "替换原文"),
    ("Shows the answer", "显示回答"),
    ("Add an Action", "添加操作"),
    ("Remove the Action", "移除操作"),
    ("Move Up", "上移"),
    ("Move Down", "下移"),
    ("New Action", "新建操作"),
    ("Built-in Actions", "内置操作"),
    ("Prompt", "提示词"),
    (
        "Sent to the model as written, followed by the selected text (or the paragraph at the caret) in a code block.",
        "按原样发送给模型，后面附上所选文本（或光标所在段落），放在代码块中。",
    ),
    (
        "Offer to replace the text with the answer",
        "提供用回答替换原文的选项",
    ),
    (
        "The answer is the rewritten text, shown with its changes marked; Replace puts it in place of the text.",
        "回答是改写后的文本，并标出改动；“替换”用它替换原文。",
    ),
    (
        "The answer shows as it is, and Continue in Assistant carries it into the conversation.",
        "回答按原样显示，“在 AI 助手中继续”可把它带入对话。",
    ),
    ("Restore Default Prompt", "恢复默认提示词"),
    ("The default prompt.", "默认提示词。"),
    (
        "Add an action with +: a new one, or a built-in one you removed.",
        "用 + 添加操作：新建一个，或加回已移除的内置操作。",
    ),
    (
        "Right-click in the editor and choose AI to run an action on the selected text, or on the paragraph at the caret when nothing is selected. The answer appears in a card under the text: Replace puts a rewrite in place of it (⌘Z undoes that), and Continue in Assistant carries an answer into the conversation.",
        "在编辑器中右键点按并选取 AI，即可对所选文本运行操作；未选择文本时作用于光标所在段落。回答显示在文本下方的卡片中：“替换”用改写替换原文（⌘Z 可撤销），“在 AI 助手中继续”把回答带入对话。",
    ),
];
