# TexCraft 开发与规范指南

这份文档写给后续继续开发 TexCraft 的人（包括你自己和 AI agent），目的是让新的改动和现有代码保持同一套做法。
它讲“怎么做”；“是什么”见下面这些文档：

| 文档 | 内容 |
|---|---|
| [AGENTS.md](../AGENTS.md) | 给 agent 的简明规则（本指南是它的展开版） |
| [architecture.md](architecture.md) | crate 分层、引擎/UI 接缝、线程、实时预览流水线 |
| [texifier-reference.md](texifier-reference.md) | 对标对象 Texifier 的公开资料整理 |
| [target-app-parity.md](target-app-parity.md) | 与 Texifier 的逐项对照表和差距清单 |
| [control-protocol.md](control-protocol.md) | 控制通道（JSON over TCP）的所有方法 |
| [devlog.md](devlog.md) | 开发日志：每次做了什么、数据、还缺什么 |

---

## 1. 目标与三条红线

**目标**：用纯 Rust 做一个和 Texifier 外观、布局、交互一致的原生 LaTeX 编辑器。
功能可以比 Texifier 简单，但**做出来的界面必须和 Texifier 对得上**：位置、尺寸、字号、颜色、行为都一致。

**红线**（违反任何一条都不合并）：

1. **Clean-room**：只参考 Texifier 的公开网站、文档、截图和可观察的行为。不拷贝它的代码、图标、图片、模板、帮助文字。
   第三方素材必须是宽松许可，并记录在 `ATTRIBUTION.md`。Texifier 独有的云服务（账号、Connect）用本地等价功能代替，并在对照表里注明。
2. **不崩溃**：非测试代码里不允许 `unwrap()`、`expect()`、`panic!`、`unreachable!`、`todo!`、`unimplemented!`，也不允许用来自输入的数字直接做切片下标。
   坏日志、截断的 SyncTeX、损坏的 PDF 一律返回错误。这些由 workspace 的 clippy lint 强制检查（测试代码豁免，见 `clippy.toml`）。
3. **只有 Rust，没有 webview**：桌面端是 egui/eframe + wgpu。不引入 Tauri、Electron 或任何 webview。

---

## 2. 环境准备

- **Rust**：1.90 以上（`Cargo.toml` 的 `rust-version`）。目前用 1.95 开发，edition 2024。
- **TeX**：MacTeX / TeX Live，需要 2022 或更新版本。中文支持依赖 LaTeX 2020-10 之后的 `\AddToHook` 和 xeCJK。
  没装 TeX 时，需要真实 TeX 的测试会自动跳过。
- **Python 3**：只用标准库，给 `scripts/parity/` 下的测量脚本用。
- **gitleaks**（防止泄漏）：`brew install gitleaks`（Windows：`winget install Gitleaks.Gitleaks`），
  然后每个 clone 跑一次 `scripts/install-hooks.sh`。之后每次 `git commit`，钩子都会把暂存的文件
  取出来用 `.gitleaks.toml` 扫一遍（`.gz`/`.zip` 里面也扫），下面这些都会让提交失败：
  - 密钥：各家服务的 API key、token、私钥、密码；
  - 隐私文件：`.env`、AI 的密钥库（`.keymaster`、`api_keys.json`）、数据库、SSH 私钥、`.netrc` 之类的凭据、签名证书；
  - 个人路径：`/Users/<名字>/`、`/home/<名字>/`、`C:\Users\<名字>\`（会暴露用户名；`me`、`user` 这类占位符可以）；
  - 超过 5 MB 的文件（`TEXCRAFT_MAX_FILE_KB` 可改上限）。

  输出里写明文件、行号和规则（密钥本身打码）。误报就在那一行加注释 `gitleaks:allow`，或者在
  `.gitleaks.toml` 里加 allowlist 并写明原因。没装 gitleaks 时提交也会被拦下。
  CI 在每次 push 和 PR 时再扫一遍整个历史和当前文件（`secrets.yml`），并用 cargo-deny 检查依赖
  （`audit.yml`、`deny.toml`：没有已知漏洞、只用宽松许可证、只从 crates.io 来），每周一也会跑一次。
  **注意**：密钥一旦 push 上去，删提交也不算安全，要去对应服务把它作废、换新的。
- **macOS**：原生能力（红绿灯位置、毛玻璃、拼写检查）只在 macOS 上生效；其他平台能编译，但未经测试。

---

## 3. 仓库地图

```text
crates/
  syntax  log  synctex   L0  纯函数：LaTeX 词法/大纲/补全数据、TeX 日志解析、SyncTeX 解析
  pdf  project  spell    L1  hayro 渲染线程；引擎/设置/根文件/模板/排版计划(plan)；拼写
  typeset                L2  运行 TeX：latexmk/tectonic 完整排版、实时预览、CJK 字体(cjk.rs)
  engine                 L3  Session、ProjectIndex、命令注册表(commands.rs)、Effect
  ui                     L4  egui 工作台
  macos                  --  macOS 原生能力（全仓库唯一允许 unsafe 的地方）
apps/
  texcraft               桌面应用：main.rs(窗口/请求/持久化)、menu.rs(原生菜单)、control.rs(控制通道)
  texcraft-cli           无界面 CLI：typeset / outline / commands …
scripts/
  bundle-macos.sh        打包 TexCraft.app
  parity/                视觉对齐工具（见第 7 节）
```

**分层规则**：一个 crate 只能依赖比它低的层；`ui` 以下不能出现 egui、eframe、winit。

`crates/ui/src` 各文件分工：

| 文件 | 负责 |
|---|---|
| `lib.rs` | `Workbench`：布局、Effect→UI 状态、各窗口的调度 |
| `theme.rs` | **所有颜色（`Tokens`，浅色/深色两套）、布局常量、字体安装** |
| `toolbar.rs` / `sidebar.rs` / `editor.rs` / `pdf_view.rs` | 主窗口四大块 |
| `prefs.rs` | Settings 独立窗口、Typeset Configuration 弹层、两栏表单控件 |
| `welcome.rs` | 欢迎窗口（独立窗口 + 毛玻璃） |
| `dialogs.rs` | 小弹窗：跳转行、关于、快捷键、提示、未保存确认 |
| `icons.rs` / `widgets.rs` | 矢量图标（自己画的，不用图片）、通用控件 |
| `cjk_font.rs` / `clock.rs` | 中文界面字体和基线对齐；本地时间格式化 |

---

## 4. 日常开发循环

建议给自己用一个独立的 target 目录（和 agent 并行时可以避开 Cargo 构建锁）：

```sh
export CARGO_TARGET_DIR=target-main
cargo build -p texcraft
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

**启动一个测试用的实例**。它的 HOME 与你的日常设置隔离，并且打开了控制通道：

```sh
scripts/parity/run-dev-app.sh examples/demo-paper
scripts/parity/tc.py typeset.wait '{"timeoutMs":60000}' state.inspect '{}'
scripts/parity/tc.py ui.screenshot '{"path":"/tmp/texcraft-dev/shot.png"}'
pkill -f /tmp/texcraft-dev/token          # 停止
```

已经有实例在跑、而且里面有没保存的文档时（可能有人正在用它），脚本会拒绝重启，免得把没保存的内容丢掉；
确实要丢掉就加 `TEXCRAFT_DEV_FORCE=1`。

> 测试实例**一定要用隔离的 HOME**（脚本已经处理好）。否则它会和你日常使用的 TexCraft 共用
> `~/Library/Application Support/texcraft/app.ron`，测试文件会混进你的最近文档，测试改的设置也会被保存下来。

改动大致分三类，各自的验证方式：

| 改动 | 怎么验证 |
|---|---|
| 解析器、数据、引擎逻辑 | 单元测试 + 真实 TeX 的 oracle 测试 |
| 命令和行为 | `crates/engine/src/tests.rs`，加上控制通道 `command.run` / `state.inspect` |
| 界面 | 控制通道截图，并和 Texifier 截图并排对比（第 7 节），**必须亲眼看过 PNG** |

---

## 5. 编码规范

### 5.1 不崩溃

- 取下标用 `get()`：`text.get(..i).unwrap_or(text)`，不要写 `&text[..i]`。
- 长度和偏移用 `checked_*` / `saturating_*`；`as` 转换要注意截断，优先用 `try_from`。
- 字符串切片必须落在字符边界上：`find` 返回的位置是安全的，自己算出来的偏移不安全。
- 二进制解析（字体表、PDF、SyncTeX）一律按“输入可能是垃圾”来写，并配一个喂垃圾数据的测试。
  例子见 `cjk_font.rs` 里的 `u16_at` 和 `face_offset`。

### 5.2 unsafe 只放在 `crates/macos`

- 这个 crate 是 `unsafe_code = "deny"`，每个 `unsafe` 块上方都要有 `// SAFETY:` 注释，说明为什么安全（通常是：在主线程上，`MainThreadMarker` 已检查；对象属于我们自己的窗口）。
- 编译器提示 `unnecessary unsafe` 时就删掉这个块，objc2 里有不少方法本身就是安全的。
- 新用到的 AppKit 类型要在 `crates/macos/Cargo.toml` 的 `objc2-app-kit` features 里加上（例如 `NSVisualEffectView`）。
- egui 的子窗口拿不到窗口句柄，所以用**窗口标题**查找 NSWindow（`place_traffic_lights_titled`、`set_vibrancy_titled`）。
  因此子窗口的标题要用常量定义（`SETTINGS_TITLE`、`WELCOME_TITLE`），并隐藏不显示。

### 5.3 一切皆命令

所有用户可见的动作都是 `crates/engine/src/commands.rs` 里的一个命令。原生菜单、快捷键、CLI、控制通道都按 id 调用它：

```rust
cmd!(
    "pdf.openExternally",          // id：小写、点分、动词在后
    "Open PDF in Default Viewer",  // 菜单文字（英文，Title Case）
    Workspace,                     // 菜单
    3,                             // 菜单内分组（组之间自动加分隔线）
    None,                          // 快捷键，如 Some(Shortcut::cmd("T"))
    has_pdf,                       // enabled 判断
    |s, _| {                       // 执行：只改 Session，不碰 GUI
        if let Some(p) = s.pdf.path.clone() {
            s.effect(Effect::OpenExternally(p));
        }
        Ok(())
    }
),
```

- 命令**不能直接操作界面**。需要界面配合时，命令产生一个 `Effect`；`Workbench::apply_effects` 把它转成 UI 状态，或者转成给 app 的 `Request`（打开文件面板、Finder、用外部程序打开等）。
- 可勾选的命令加 `checked = |s| …`；带参数的命令加 `params = "…"`，这段说明会出现在 `commands.list` 里。
- 新命令自动被 `every_command_survives_adversarial_params` 用奇怪的参数轮番测试，不需要额外注册，但它不能 panic。

### 5.4 UI 保持“薄”，颜色只来自 Tokens

- 颜色只能写在 `theme.rs` 的 `Tokens::light()` / `Tokens::dark()` 里，控件代码中不能出现 `hex(…)` 或 RGB 字面量。
  新颜色就新加一个 token，**浅色和深色要同时给值**。白色、透明这类中性常量除外。
- 布局常量（工具栏高度、行高、侧栏宽度等）放在 `theme.rs`，并注明来自哪次测量。
- 字体：
  - 界面文字用 `FontId::proportional(size)`，粗体用 `theme::semibold(size)`，编辑器用 `theme::editor_font(size)`。
  - 字号都是量出来的，不要凭感觉写（第 7.3 节）。
- 业务逻辑放在 engine。UI 只负责画图，并把操作转成命令或 Session 字段的修改。
- **菜单、下拉框、右键菜单、弹出框、对话框只用 `crates/ui/src/menu.rs`**，不要直接用 `ui.button` /
  `Button::selectable` / 默认的 `egui::Modal` 外观：
  - 菜单内容先 `menu::begin(ui, 最小宽度)`，条目用 `menu::item`（勾选、快捷键、说明行、logo）、
    `menu::command`（直接跑命令，标签/快捷键/可用状态从命令表读）、`menu::separator`、`menu::heading`；
  - 菜单宽度跟着最宽的一项走：自定义的行用 `menu::row_width(ui, 自然宽度)` 分配宽度，不要直接用
    `ui.available_width()`（egui 的 sizing pass 里那是默认的 400–600 pt，菜单会宽出一大截）；
    自己用 `egui::Area` 画的菜单，每次打开时要 `.sizing_pass(true)` 一次；
  - 对话框用 `egui::Modal::new(..).frame(menu::sheet_frame(t, 20)).backdrop_color(menu::backdrop())`，
    按钮用 `menu::dialog_buttons`（默认按钮在最右，蓝底白字）；
  - egui 自带的弹层（tooltip、`Popup`、`context_menu`）的底色、圆角、阴影由 `Tokens::visuals()` 统一。
- **秘密（API Key）**：只存在 `texcraft_ai::KeyStore`（AES-256-GCM，`.keymaster` + `api_keys.json`，
  仅本人可读），只在发请求时解密；不能写进 `ai.json`、日志、`state.inspect`、命令参数或命令行
  （curl 的地址和请求头经标准输入传入），保存后界面上不再显示。

### 5.5 代码风格

- `rustfmt.toml`：`max_width = 100`。提交前跑 `cargo fmt --all`。
- 所有 `pub` 项都要有 `///` 文档注释，写清“是什么、单位是什么”（pt、px、字符、毫秒）。
- 注释写“为什么”。和 Texifier 对齐得来的数字，要注明“as measured”或说明来源。
- clippy 开 `-D warnings`。只有私有的绘图辅助函数可以加 `#[allow(clippy::too_many_arguments)]`，其他情况都要改代码。
- 命名：
  - crate 叫 `texcraft-*`，界面上的文字一律写 **TexCraft**。
  - 命令 id 用 `menu.action` 风格。
  - control 方法用 `ui.*`、`state.*` 风格。

### 5.6 性能规则

- 排版和 PDF 渲染**永远不能阻塞 UI 线程**（它们在 `texcraft-typeset` 和 `texcraft-pdf-render` 线程上）。
- **每一帧都会跑的代码要便宜**：
  - 工具栏每帧都会询问引擎（`Session::typeset_plan`），所以计划按文档版本号缓存，包检测也只扫导言区。
  - 新增类似的每帧计算时，照这个方式加缓存。
- **不要每帧都发 `ViewportCommand`**。每发一次，egui 就会再请求一帧，结果是空闲时 CPU 100%。
  必须发时，先和上次的值比较，只在变化时发送（参考 `main.rs` 里的窗口标题）。
- 空闲时 CPU 应该接近 0。怀疑有持续重绘时，看 `state.inspect` → `ui.repaintCauses`，它会给出请求重绘的文件和行号。
- 编辑器的目标：5000 行的文档，每次按键在一帧内处理完。

---

## 6. 新功能的标准步骤

1. **先在对照表里找到它**（`target-app-parity.md`），确认 Texifier 是怎么做的（看 `texifier-reference.md` 或截图）。
2. **数据和逻辑**：放进最低的合适层（解析放 syntax，设置放 project，排版放 typeset），并配单元测试。
3. **命令**：在 `commands.rs` 加命令。需要界面配合的，加 `Effect` / `Request`。在 `engine/src/tests.rs` 加行为测试。
4. **界面**：按第 7 节量好尺寸再画，颜色加到 Tokens。
5. **菜单和快捷键**：命令自带。确认它出现在正确的菜单和分组里。
6. **可驱动**：
   - 需要从外部检查的状态，加进 `state.inspect`。
   - 独立窗口要提供 `inline` 模式，并加对应的 `ui.*` 控制方法（见 7.4）。
   - 在 `control-protocol.md` 里写明。
7. **持久化**：新设置放进 `View`（界面）或 `Settings`（排版），二者都是 `#[serde(default)]`。
   如果改了**已有设置的默认值**，就把 `apps/texcraft/src/main.rs` 的 `PERSISTED_VERSION` 加一，并在 `App::new` 里写迁移逻辑。
8. **文档**：更新对照表的状态（✅ / 🟡 / ❌ 和备注），在 `devlog.md` 记一条（做了什么、关键数据、还缺什么）。
9. 跑完第 15 节的检查清单。

---

## 7. 与 Texifier 对齐的工作流（视觉一致性）

这是这个项目最主要的工作方式：**先量，再改，再截图对比，直到误差在 1pt 以内。**

### 7.1 坐标系

- egui 的单位是 pt。Retina 截图是 **2 px = 1 pt**。
- Texifier 截图如果被缩放过，用已知尺寸反推比例：
  - 红绿灯直径 14pt；
  - 侧栏行高 22pt；
  - 工具栏高 52pt。
- 我们的截图用 `ui.screenshot` 得到，同样是 2 px/pt。

### 7.2 测量工具（`scripts/parity/`）

| 工具 | 用途 |
|---|---|
| `png.py` | 读 PNG。`px` / `hexpx` 取像素颜色；`runs_row` / `runs_col` 沿一行或一列找颜色变化的分段（用来量边界、分隔线、控件尺寸）；`crop(…, scale)` 裁剪放大后再看 |
| `widths.py` | 量框内文字的墨迹宽度：`widths.py shot.png [--scale 2] name:x0,x1,y0,y1 …` |
| `compose.py` | 两张图的区域左右并排：`compose.py out.png a.png:x0,y0,x1,y1 b.png:x0,y0,x1,y1` |
| `tc.py` | 控制通道客户端：`tc.py METHOD '{params}' …` |
| `run-dev-app.sh` | 启动隔离的测试实例 |

用户发来的截图通常是 webp 或 jpg，先转成 PNG：`sips -s format png in.webp --out in.png`。
需要缩小时用 `sips -Z 1400`；需要按比例缩放到和 Texifier 截图一致时用 `sips -z H W`。

例子：量 Texifier 工具栏底线所在的 y，以及某个按钮的左右边界：

```sh
python3 -c "
import sys; sys.path.insert(0,'scripts/parity'); import png
im = png.read('texifier.png')
print(im.runs_col(600, 80, 120, 20))   # 第 600 列，y 从 80 到 120，颜色差 > 20 处分段
print(im.runs_row(132, 1180, 1400, 60))
"
```

### 7.3 字号：量文字宽度，反推字号

不要靠目测判断字号。步骤：

1. 在 Texifier 截图里，用 `widths.py` 量出某段文字的宽度（pt）。
2. 用同一段文字跑宽度工具，看我们的字体在各个字号下有多宽：

   ```sh
   TEXCRAFT_WIDTHS='Recent Documents|Open a local file…|b' \
     cargo test -p texcraft-ui --test text_widths -- --ignored --nocapture
   ```

   字符串之间用 `|` 分隔，`b` 表示“前一项用半粗体”。
3. 选最接近的字号。打印出来的宽度带有约 0.6pt 的字形侧边距，墨迹宽度要加上这点再比较。

**背景知识**：

- 系统字体 SF 是可变字体，默认的光学尺寸是 28（Display，字距紧）。macOS 在小字号下用的是 Text 设计（`opsz` 17）。
  我们在 `theme.rs` 里固定用 `opsz 17`，粗体用 `wght 600`，字宽才和 Texifier 一致。
- 实测的对应关系（我们的 pt）：
  - 主窗口里 Texifier 的 12pt 文本对应我们的 **11.75**，11pt 对应 **10.75**（那张截图本身约有 2% 缩放）；
  - Settings 窗口约 11pt；
  - Typeset Configuration 弹层的标题 13pt、说明 12pt。

### 7.4 截图与对比

- 主窗口：`tc.py ui.screenshot '{"path":"…"}'`。窗口被遮挡时可能超时，重试即可。
- **独立窗口（Settings、欢迎窗口）eframe 截不到**。它们都有 `inline` 模式，会把内容画在主窗口里，用于截图：
  - `tc.py ui.prefs '{"open":true,"pane":"editor","inline":true}'`
  - `tc.py ui.welcome '{"open":true,"inline":true}'`
  - 新增独立窗口时也照这个方式提供 inline 模式。
- 裁出相同区域，按 Texifier 截图的比例缩放后用 `compose.py` 并排，然后**亲眼看**。
  逐项量出偏差（位置、尺寸、字号、颜色），改完再截图，直到每处在 1pt 以内。
- 最后在深色模式下再看一遍：`tc.py command.run '{"id":"view.theme.dark"}'`。

### 7.5 对齐的原则

- 布局、尺寸、字号、颜色、交互（悬停、选中态、快捷键）跟 Texifier 一致；功能可以更少。
- Texifier 独有、我们没有的功能（账号、云同步），用有意义的本地功能替换。
  例如欢迎窗口的 Account → About，Connect Projects → Templates。不要保留做不到的假按钮。
- 图标自己画（`icons.rs`，矢量 16 格网格），风格接近即可，不照抄。

---

## 8. 排版与实时预览要点

- **实时预览提速**（`crates/typeset/src/live.rs`），三个引擎各自的办法：
  - pdfLaTeX：导言区预编译成格式（mylatexformat，存成不压缩的）、精简的 `pdftex.map`、只含需要文件的最小 `texmf.cnf`。约 0.07 s 一遍。
  - XeLaTeX：XeTeX 不能把系统字体存进格式，而类文件设字号时就会选字体。所以 live 文件夹里根文件的副本第一行先用 TFM 编码（OT1）读导言区，
    TexCraft 加的 xeCJK 在类之后、暂停选字体时加载；`\begin{document}` 那一行用 mylatexformat 的 `\endofdump` 结束 dump，每遍再恢复 TU
    编码（导言区选了 T1 就保持 T1）并设中文字体。行数不变，报错行号和 SyncTeX 不受影响。PDF 那一步（xdvipdfmx）用本地 `dvipdfmx.cfg`
    指向精简的字体表；精简表漏了字体时（虚拟字体的原始字体名不同）它会报错（我们把 PK 位图字体藏起来了），就把那个字体族补进去重排一遍。约 0.35 s 一遍（原来 1.2 s）。
  - LuaLaTeX：同样的格式做法，dump 时还检查导言区有没有留下 Lua 状态（包注册的回调、luaotfload 载入的字体），有就不用格式；
    再加精简的 `pdftex.map`。约 0.45 s 一遍（原来 0.8 s）。
  - 导言区用 fontspec 设字体之类、做不成格式时，自动退回普通的单遍排版，用户文件原样排。

- **排版计划**：`texcraft_project::plan()` 决定用哪个引擎，以及要不要自动加中文支持；`Session::typeset_plan()` 是它带缓存的入口。优先级如下：
  1. `% !TEX program` 魔法注释；
  2. Auto-Sense：fontspec / ctex / unicode-math 或正文里有中日韩字符 → XeLaTeX；luacode / luatexja → LuaLaTeX；
  3. 设置里选的引擎。
- **中文**：文档里有中日韩字符、但没有自己配置时（ctex、xeCJK、CJKutf8、luatexja…），TexCraft 会在 `\documentclass` 之后加载 xeCJK 和系统字体，**不修改用户文件**：
  - 完整排版：通过 `latexmk -usepretex`；
  - 实时预览：把代码放在命令行的 `\input{root}` 前面。
  - 代码见 `typeset/src/cjk.rs`。
- **实时预览**：
  - 未保存的内容镜像到 `<project>/.texcraft/live/`；
  - pdfLaTeX 会预编译导言区、裁剪字体映射，并使用快速环境 `.texcraft-fast`（详见 architecture.md）；
  - XeLaTeX 没有这些加速，单次约 0.95s（pdfLaTeX 约 0.23s）。
- **调试排版**：
  - `tc.py typeset.wait '{}'` 返回耗时、错误数、`fastEnv`、`latencyMs`；
  - 中间文件在 `.texcraft/build`（完整排版）和 `.texcraft/live`（实时预览），可以直接看 `.log`。
- 用 mylatexformat 预编译导言区时，**不能**在命令行里加前缀代码；所以前置代码只用于没有预编译格式的引擎。

---

## 9. 测试规范

- 单元测试写在代码旁边（`#[cfg(test)] mod tests`），夹具放在各 crate 的 `tests/data/`。
- **Oracle 测试**：解析器用真实 TeX 产生的产物来验证，例如故意写错的文档生成的 `.log`、`synctex` 命令行的输出。
- **需要真实 TeX 的测试**，开头先检测工具链，没装就 `eprintln!("skipping …")` 后返回，不能失败。
  例如 `typesets_chinese_with_the_cjk_setup_when_installed`。
- 引擎行为测试在 `crates/engine/src/tests.rs`：用 `session()` 新建会话，用 `s.run("cmd.id", &json!(…))` 驱动，用 `s.tick()` 推进排版。
- 测量工具类的测试标 `#[ignore]`（例如 `text_widths`），不进日常测试。
- 不写依赖时序的脆弱断言：等待排版完成要用循环加截止时间，不要固定 `sleep`。

---

## 10. 调试手册

| 想知道 | 怎么看 |
|---|---|
| 当前状态 | `tc.py state.inspect '{}'`：文档、引擎、`cjkSetup`、设置、排版统计、PDF、`ui.dialog`、`ui.settings`、`ui.welcome`、`ui.windows`（所有原生窗口的标题、是否可见、位置）、`ui.repaintCauses` |
| 为什么一直重绘 | `ui.repaintCauses`（文件和行号） |
| 为什么空闲 CPU 高 | `sample <pid> 3` 看热点函数，再结合 repaintCauses |
| 窗口是否真的显示了 | `ui.windows` |
| 排版为什么失败 | `typeset.wait` 的结果，加上 `.texcraft/{build,live}/*.log` |
| 设置存在哪 | `~/Library/Application Support/texcraft/app.ron`（RON 格式，测试实例在 `/tmp/texcraft-dev/home` 下） |

---

## 11. macOS 原生能力（`crates/macos`）

目前已有：

- `place_traffic_lights` / `place_traffic_lights_titled`：红绿灯按钮的位置，以及只保留关闭按钮；
- `set_vibrancy_titled`：系统毛玻璃；
- `windows`：列出应用的所有窗口；
- 拼写检查：`spelling_ok`、`spelling_guesses`、`learn_word`。

添加新能力的套路：

1. 在 `imp` 模块里实现，用 `MainThreadMarker::new()` 确认在主线程；非 macOS 平台给一个空实现。
2. 函数要能每帧调用而且开销很小：先检查是否已经设置过，再决定是否动手（参考 `vibrancy_titled`）。
3. **毛玻璃有一个前提**：egui-wgpu 是否支持透明背景，由**主窗口**的设置决定，所以主窗口也开了 `with_transparent(true)`。主窗口仍然自己画满每一个像素。
   `App::clear_color` 返回全透明。

---

## 12. 打包发布

```sh
scripts/bundle-macos.sh                     # 生成 $CARGO_TARGET_DIR/macos/TexCraft.app（release 构建）
rm -f dist/TexCraft-0.1.0-macos.zip
ditto -c -k --keepParent target/macos/TexCraft.app dist/TexCraft-0.1.0-macos.zip
```

如果设置了 `CARGO_TARGET_DIR`，把上面的 `target/` 换成对应目录（例如 `target-main/`）。
打包后用隔离的 HOME 启动一次 .app 做冒烟测试：能打开文档、排版成功、空闲 CPU 接近 0。`dist/` 不进版本库。

### 12.1 正式发布（GitHub Actions）

推送 `v` 开头的版本 tag，就会触发 `.github/workflows/release.yml`：

```sh
git tag -a v0.2.0 -m "## What's new
- 这里写更新内容，支持 Markdown（标题、列表、**粗体**、`代码`）"
git push origin v0.2.0
```

- 版本号以 tag 为准：CI 会把 tag 写进 `Cargo.toml` 的 workspace 版本再编译，所以本地忘了改版本号也不要紧。
- 产物有三个：
  - `TexCraft-<版本>-macos.zip`：通用二进制（Apple 芯片 + Intel），ad-hoc 签名，**未公证**，用户第一次打开要右键 › 打开；
  - `TexCraft-<版本>-windows-x64.zip`：exe 内嵌图标；
  - `latest.json`：更新清单，由 `scripts/ci/latest_json.py` 生成，含每个平台的下载地址、SHA-256 和大小，以及 tag 里的说明。
- 带后缀的 tag（`v0.3.0-beta.1`）会发成 pre-release，**不会推送给用户**（`releases/latest` 不指向 pre-release）。
- 在 Actions 页面可以用 `workflow_dispatch` 输入一个已有的 tag 重新构建。
- 安全措施：泄漏扫描和依赖审查（`secrets.yml`、`audit.yml`）通过之后才开始编译；只有最后发布的那一步
  有写仓库的权限（编译会跑依赖的 build script）；action 都钉在 commit hash 上（Dependabot 每周提更新）；
  发布构建不用缓存（防缓存投毒）；checkout 不保留 token；手动输入的 tag 走环境变量，不直接拼进脚本。

### 12.2 自动更新

- 代码分布：
  - `crates/update`：读清单、比较版本、用系统 `curl` 下载、校验 SHA-256、解压（macOS 用 `ditto`，Windows 用 `tar`），以及在应用退出后替换安装（macOS 用 sh 脚本，Windows 用 .cmd 脚本）；
  - `crates/engine/src/update.rs`：状态机和自动检查（**每次启动** 3 秒后都检查一次，不管上次什么时候查过；之后每 6 小时一次）；
  - `crates/ui/src/update.rs`：更新窗口，样式参照 Argus。
- 默认行为：自动检查、自动下载。**启动时**查到新版本就立刻弹出更新窗口（边弹边在后台下载，窗口里有进度）；
  启动时主窗口藏在欢迎窗口后面，这时更新窗口画在欢迎窗口上。之后的定时检查查到新版本，等下载完才弹窗。
  用户可以“Restart Now”，否则退出时自动安装；“Skip This Version”之后这个版本不再提醒。两个开关在 Settings › General。
- **本地测试更新流程**（不需要真的发版）：

  ```sh
  # 1. 准备一个“新版本”：在 /tmp/release 里放 TexCraft-0.2.0-macos.zip 和一个 Windows 占位 zip
  python3 scripts/ci/latest_json.py 0.2.0 v0.2.0 chenwen245299/TexCraft /tmp/release /tmp/release/notes.md
  #    再把 latest.json 里的下载地址改成 http://127.0.0.1:8765/...
  (cd /tmp/release && python3 -m http.server 8765 --bind 127.0.0.1 &)
  # 2. 让测试实例读这个清单
  TEXCRAFT_UPDATE_URL=http://127.0.0.1:8765/latest.json scripts/parity/run-dev-app.sh
  scripts/parity/tc.py command.run '{"id":"app.checkForUpdates"}' state.inspect '{}'
  ```

  - 安装这一步只在**打包好的 .app** 里有效（开发版不在 .app 里，会提示无法安装）。可以把 .app 复制到一个临时目录里运行，再用 `app.quit` 退出，检查它是否被换成了新版本。
  - `TEXCRAFT_NO_UPDATES=1` 关闭自动检查；`run-dev-app.sh` 默认就是关闭的。
  - 测试会话默认指向一个不存在的本地清单，单元测试永远不会联网。

### 12.3 图标

- 源文件是 `assets/icon/texcraft.svg`，原创矢量图，风格与 Craft 系列一致：纯色圆角底、粗黑描边、平涂。
- 改完 SVG 后，依次运行：
  - `scripts/render-icon.sh`：用无界面的 Chrome / Edge 渲染出透明圆角的 `texcraft-1024.png`；
  - `scripts/make-ico.py`：生成 Windows 用的 `texcraft.ico`。
- 应用通过 `include_bytes!` 内嵌这个 PNG，作为窗口图标；`bundle-macos.sh` 用它生成 `.icns`；Windows 的 exe 图标由 `apps/texcraft/build.rs` 嵌入。
- 欢迎窗口里的灰色线稿版图标在 `welcome.rs` 的 `icon()` 里用代码画，改了造型要一起改。

---

## 13. 文档维护

- **`devlog.md`**：每完成一项工作写一段。格式是“标题（日期）+ 要点列表”：做了什么、关键数据（耗时、误差、CPU）、还缺什么或没验证到什么。
- **`target-app-parity.md`**：状态用 ✅（一致）/ 🟡（部分）/ ❌（没有），备注写清差异。
- **`control-protocol.md`**：新增或修改控制方法时同步更新。
- **`README.md`**：只在用户可见的大功能变化时更新。

---

## 14. 踩过的坑（别再踩一次）

| 现象 | 原因 | 做法 |
|---|---|---|
| 测试数据里带着本机的绝对路径（用户名） | SyncTeX 文件、TeX 日志在临时目录生成时会记下完整路径；`.gz` 里的普通 grep 看不到 | 生成脚本把路径换成占位符（`crates/synctex/tests/data/generate.py` 的 `scrub`，`--scrub` 可单独重跑）；提交钩子也会扫压缩包里的个人路径 |
| 空闲时 CPU 100% | 每帧发送 `ViewportCommand::Title`，每个窗口命令都会再请求一帧 | 只在值变化时发送 |
| 弹窗越长越高，内容被推出屏幕（“白条”） | 弹窗里的竖向 `ui.separator()` 会撑满上一帧的高度 | 弹窗用固定尺寸，分隔线自己画 |
| 中文比英文高出约 4pt | egui 把回退字体的行高差居中；冬青黑体 lineGap 为 0.5em | 读取字体度量表，设置 `y_offset_factor`（`cjk_font.rs`） |
| 界面文字比 Texifier 窄 5–8% | SF 默认使用 Display 光学尺寸 | 固定 `opsz 17` |
| 子窗口截图超时 | eframe 不支持对 immediate viewport 截图 | 提供 inline 模式 |
| 毛玻璃下面是黑底 | 透明后备缓冲由主窗口设置决定 | 主窗口也设置 `with_transparent(true)` |
| xeCJK 报 `Undefined control sequence` | 在 `begindocument/before` 里加载 xeCJK 太晚 | 改用 `class/after` 钩子 |
| mylatexformat 报 “no legal \end found” | 在命令行里加了前缀代码 | 预编译格式时不加前缀 |
| 最近文档里出现测试文件 | 测试实例和日常实例共用存储 | 用隔离的 HOME（`run-dev-app.sh`） |
| 新编译的版本没生效 | 旧实例占着控制端口，新实例启动失败 | 启动前先结束旧实例；查看日志里的 “Address already in use” |
| 侧栏拖不宽（总回到最小宽度） | egui 的 `Panel` 记住的是内容占用的宽度，内容画在 `new_child` 里不算 | 面板回调里先 `ui.expand_to_include_rect(ui.max_rect())` |
| 选中的代码全变成一种颜色 | egui 用 `visuals.selection.stroke` 给选中字重新上色 | 编辑器把绘制的 galley 换回原样，自己在底下画选区（`editor::paint_selection`） |
| 单行输入框按回车没反应 | 回车在同一帧就让输入框失去焦点 | 用 `response.lost_focus() && key_pressed(Enter)` |
| 菜单截图是半透明的 | 弹层有淡入动画 | `ui.wait` 20 帧以上再截图 |

---

## 15. 提交前检查清单

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- [ ] 没有新增 `unwrap` / `expect` / 下标切片（非测试代码）。
- [ ] 没有在 `crates/macos` 之外写 `unsafe`。
- [ ] 新的用户动作是命令，并且有测试。
- [ ] 新的颜色进了 `Tokens`，浅色和深色都有值。
- [ ] 界面改动截过图，和 Texifier 并排对比过，深色模式也看过。
- [ ] 空闲 CPU 仍然接近 0（`ui.repaintCauses` 为空，或只有控制通道的轮询）。
- [ ] 需要时更新了 `state.inspect`、`control-protocol.md`、对照表。
- [ ] `devlog.md` 记了一条。
- [ ] 提交时 gitleaks 钩子没报（装过 `scripts/install-hooks.sh`）；测试里的假密钥别写成真实格式。
