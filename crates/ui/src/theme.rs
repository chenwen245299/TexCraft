//! Colours, metrics and fonts. Values follow the target's default theme as sampled in
//! `docs/texifier-reference.md` (§1, §2, §4); widgets read them from [`Tokens`], never hard-code.

use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};
use texcraft_syntax::TokenKind;

/// Design tokens for one appearance.
#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
    /// Dark appearance.
    pub dark: bool,
    /// Unified title bar + toolbar background.
    pub toolbar_bg: Color32,
    /// Toolbar icon colour (secondary glyphs).
    pub toolbar_icon: Color32,
    /// Toolbar glyphs and text (primary).
    pub toolbar_fg: Color32,
    /// Toolbar capsule fill.
    pub capsule_bg: Color32,
    /// Toolbar capsule outline and inner separators.
    pub capsule_border: Color32,
    /// Selected segment inside a capsule.
    pub segment_selected: Color32,
    /// Toolbar back/forward when there is nowhere to go.
    pub nav_disabled: Color32,
    /// Misspelling underline.
    pub spell_error: Color32,
    /// The AUTO label.
    pub auto_label: Color32,
    /// Typeset chain: the engine.
    pub chain_fg: Color32,
    /// Typeset chain: later steps.
    pub chain_weak: Color32,
    /// Gear glyph in the toolbar.
    pub gear: Color32,
    /// Sidebar tab glyphs and other grey icons.
    pub sidebar_icon: Color32,
    /// Sidebar filter field fill.
    pub filter_bg: Color32,
    /// Sidebar filter field border.
    pub filter_border: Color32,
    /// Count badge outline.
    pub badge_border: Color32,
    /// Separators inside the toolbar capsule.
    pub toolbar_separator: Color32,
    /// Role badge fill (Files list: ROOT, SUB, OUTPUT, TEMP).
    pub badge_fill: Color32,
    /// Folder icons in the Files list.
    pub folder: Color32,
    /// Count badge digits.
    pub badge_text: Color32,
    /// PDF header glyphs.
    pub pdf_icon: Color32,
    /// Disabled PDF header glyphs.
    pub pdf_icon_disabled: Color32,
    /// Line between PDF pages.
    pub page_gap: Color32,
    /// Editor jump-bar text.
    pub jump_text: Color32,
    /// The app icon's blue (welcome window).
    pub brand: Color32,
    /// Settings, sheets and the welcome window: background.
    pub form_bg: Color32,
    /// Welcome window background when the system material is unavailable.
    pub welcome_bg: Color32,
    /// Welcome window: the grey line-art icon.
    pub welcome_icon: Color32,
    /// Welcome window: column and tab dividers (translucent).
    pub welcome_divider: Color32,
    /// Welcome window: the template pop-up (translucent).
    pub welcome_popup: Color32,
    /// Welcome window: hovered rows and tabs (translucent).
    pub welcome_hover: Color32,
    /// Forms: hairlines between sections.
    pub form_hairline: Color32,
    /// Forms: stronger hairlines (footer, sheet border).
    pub form_hairline_strong: Color32,
    /// Forms: bold labels and headings.
    pub form_label: Color32,
    /// Settings: descriptions.
    pub form_desc: Color32,
    /// Sheets: grey explanations.
    pub form_note: Color32,
    /// Pop-up buttons.
    pub form_popup: Color32,
    /// Pop-up buttons, hovered.
    pub control_hover: Color32,
    /// Pop-up and push-button text.
    pub control_text: Color32,
    /// Unchecked checkbox and radio fill.
    pub check_off: Color32,
    /// Unchecked checkbox and radio border.
    pub check_off_border: Color32,
    /// Settings toolbar icons and labels.
    pub pane_item: Color32,
    /// Settings toolbar: selected item background.
    pub pane_item_selected: Color32,
    /// Sidebar background.
    pub sidebar_bg: Color32,
    /// Sidebar text.
    pub sidebar_text: Color32,
    /// Secondary (grey) text.
    pub text_weak: Color32,
    /// Primary UI text.
    pub text: Color32,
    /// Pane dividers.
    pub divider: Color32,
    /// The 26 pt header bars above the editor and PDF.
    pub header_bg: Color32,
    /// Accent (selection, selected sidebar tab).
    pub accent: Color32,
    /// Selected row background in lists.
    pub row_selected: Color32,
    /// Hovered row background.
    pub row_hover: Color32,
    /// "LIVE" label.
    pub live: Color32,
    /// Error red.
    pub error: Color32,
    /// Warning yellow/orange.
    pub warning: Color32,
    /// Success green.
    pub success: Color32,
    /// Editor background.
    pub editor_bg: Color32,
    /// Editor plain text.
    pub editor_text: Color32,
    /// Control sequences.
    pub syn_command: Color32,
    /// Maths.
    pub syn_math: Color32,
    /// Comments.
    pub syn_comment: Color32,
    /// Verbatim text.
    pub syn_verbatim: Color32,
    /// Line numbers.
    pub line_number: Color32,
    /// An inline AI suggestion (grey text at the caret).
    pub ghost_text: Color32,
    /// Current line number.
    pub line_number_current: Color32,
    /// Current line band.
    pub current_line: Color32,
    /// Text selection (text fields).
    pub selection: Color32,
    /// Selection in the editor: a neutral tint over the page colour (measured on the target).
    pub editor_selection: Color32,
    /// Find matches.
    pub find_match: Color32,
    /// Matching-bracket highlight.
    pub bracket_match: Color32,
    /// PDF pane background around pages.
    pub pdf_bg: Color32,
    /// Page shadow/border.
    pub page_border: Color32,
    /// SyncTeX flash.
    pub sync_flash: Color32,
    /// Popup/popover background.
    pub popup_bg: Color32,
    /// The "M" of a modified file in Source Control.
    pub git_modified: Color32,
    /// Menu / pop-over outline.
    pub menu_border: Color32,
    /// Separators in menus.
    pub menu_separator: Color32,
    /// Disabled menu items.
    pub menu_disabled: Color32,
    /// Control (text field, segmented control) background.
    pub control_bg: Color32,
    /// Control border.
    pub control_border: Color32,
}

// Metrics measured on the target app under macOS 26 (2 px per pt screenshots).

/// Toolbar height (pt).
pub const TOOLBAR_HEIGHT: f32 = 52.0;
/// Height of the editor/PDF header bars and the sidebar tab row (pt).
pub const HEADER_HEIGHT: f32 = 26.5;
/// Default sidebar width (pt).
pub const SIDEBAR_WIDTH: f32 = 200.0;
/// Sidebar outline row height (pt).
pub const ROW_HEIGHT: f32 = 22.0;
/// Sidebar group header row height (pt).
pub const GROUP_ROW_HEIGHT: f32 = 24.0;
/// Left edge of the close button (pt).
pub const TRAFFIC_LIGHTS_LEFT: f32 = 19.0;
/// Width reserved for the macOS window buttons at the left of the toolbar (pt).
pub const TRAFFIC_LIGHTS_WIDTH: f32 = 95.0;
/// Sidebar text size (pt).
pub const SIDEBAR_FONT: f32 = 11.75;

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Tokens {
    /// The light appearance.
    pub fn light() -> Self {
        Self {
            dark: false,
            toolbar_bg: hex(0xFFFFFF),
            toolbar_icon: hex(0x6E6E73),
            toolbar_fg: hex(0x0A0A0A),
            capsule_bg: hex(0xFBFBFC),
            capsule_border: hex(0xE1E2E5),
            segment_selected: hex(0xE2E2E2),
            nav_disabled: hex(0x9C9C9C),
            spell_error: hex(0xFF7268),
            auto_label: hex(0x1B72DC),
            chain_fg: hex(0x4E4E4E),
            chain_weak: hex(0xBABABA),
            gear: hex(0x525154),
            sidebar_icon: hex(0x7D7D7D),
            filter_bg: hex(0xCBCBCB),
            filter_border: hex(0xB4B4B4),
            badge_border: hex(0xB7B7B7),
            toolbar_separator: hex(0x8C8C8C),
            badge_fill: Color32::from_black_alpha(8),
            folder: hex(0x4AA3EF),
            badge_text: hex(0x7F7F7F),
            pdf_icon: hex(0x6E6E6E),
            pdf_icon_disabled: hex(0xCDCDCD),
            page_gap: hex(0xCCCCCC),
            jump_text: hex(0x848484),
            brand: hex(0xD2364E),
            form_bg: Color32::WHITE,
            welcome_bg: hex(0xE4E5E4),
            welcome_icon: hex(0x9BA3A7),
            welcome_divider: Color32::from_black_alpha(46),
            welcome_popup: Color32::from_black_alpha(22),
            welcome_hover: Color32::from_black_alpha(12),
            form_hairline: hex(0xE6E6E6),
            form_hairline_strong: hex(0xD3D3D3),
            form_label: hex(0x1F1F1F),
            form_desc: hex(0x3F3F3F),
            form_note: hex(0x8C8C8C),
            form_popup: hex(0xEFEFEF),
            control_hover: hex(0xE6E6E6),
            control_text: hex(0x262626),
            check_off: hex(0xEDEDED),
            check_off_border: hex(0xD5D5D5),
            pane_item: hex(0x8A8A8A),
            pane_item_selected: Color32::from_black_alpha(8),
            sidebar_bg: hex(0xE4E4E4),
            sidebar_text: hex(0x474747),
            text_weak: hex(0x7A7A7A),
            text: hex(0x1F1F1F),
            divider: hex(0xB7B7B7),
            header_bg: hex(0xFEFEFE),
            accent: hex(0x0270F6),
            row_selected: Color32::from_rgba_unmultiplied(0, 112, 244, 46),
            row_hover: Color32::from_rgba_unmultiplied(0, 0, 0, 14),
            live: hex(0x5DB83B),
            error: hex(0xE0393E),
            warning: hex(0xE8A317),
            success: hex(0x41B862),
            editor_bg: hex(0xFFFFFF),
            editor_text: hex(0x000000),
            syn_command: hex(0x0E6FD0),
            syn_math: hex(0xD06F0E),
            syn_comment: hex(0x339666),
            syn_verbatim: hex(0x6A6A6A),
            line_number: hex(0xC3BEAE),
            ghost_text: hex(0x9C978B),
            line_number_current: hex(0x3C3C3C),
            current_line: Color32::TRANSPARENT,
            selection: Color32::from_rgba_unmultiplied(0, 112, 244, 60),
            editor_selection: Color32::from_black_alpha(15),
            find_match: Color32::from_rgba_unmultiplied(255, 214, 0, 120),
            bracket_match: Color32::from_rgba_unmultiplied(0, 112, 244, 40),
            pdf_bg: hex(0xECECEC),
            page_border: hex(0xD0D0D2),
            sync_flash: Color32::from_rgba_unmultiplied(80, 160, 255, 70),
            popup_bg: hex(0xF2F2F2),
            git_modified: hex(0xB7791F),
            menu_border: Color32::from_black_alpha(38),
            menu_separator: Color32::from_black_alpha(26),
            menu_disabled: hex(0xB2B2B2),
            control_bg: hex(0xFFFFFF),
            control_border: hex(0xCDCDCD),
        }
    }

    /// The dark appearance.
    pub fn dark() -> Self {
        Self {
            dark: true,
            toolbar_bg: hex(0x232425),
            toolbar_icon: hex(0xA9A9AA),
            toolbar_fg: hex(0xE8E8EA),
            capsule_bg: hex(0x3A3A3C),
            capsule_border: hex(0x4C4C4F),
            segment_selected: hex(0x58585C),
            nav_disabled: hex(0x6E6E72),
            spell_error: hex(0xE07D7D),
            auto_label: hex(0x4C9BFF),
            chain_fg: hex(0xDDDDDD),
            chain_weak: hex(0x7A7A7A),
            gear: hex(0xD0D0D2),
            sidebar_icon: hex(0xA0A0A2),
            filter_bg: hex(0x3A3A3C),
            filter_border: hex(0x505052),
            badge_border: hex(0x5A5A5C),
            toolbar_separator: hex(0x6A6A6E),
            badge_fill: Color32::from_white_alpha(10),
            folder: hex(0x5AAFF5),
            badge_text: hex(0xA0A0A2),
            pdf_icon: hex(0xB0B0B2),
            pdf_icon_disabled: hex(0x5A5A5C),
            page_gap: hex(0x111111),
            jump_text: hex(0xA0A0A2),
            brand: hex(0xD2364E),
            form_bg: hex(0x2B2B2D),
            welcome_bg: hex(0x2A2A2C),
            welcome_icon: hex(0x8A8F93),
            welcome_divider: Color32::from_white_alpha(40),
            welcome_popup: Color32::from_white_alpha(22),
            welcome_hover: Color32::from_white_alpha(14),
            form_hairline: hex(0x3B3B3D),
            form_hairline_strong: hex(0x4A4A4D),
            form_label: hex(0xECECEC),
            form_desc: hex(0xCBCBCB),
            form_note: hex(0x98989D),
            form_popup: hex(0x3C3C3F),
            control_hover: hex(0x464649),
            control_text: hex(0xE6E6E6),
            check_off: hex(0x404043),
            check_off_border: hex(0x58585C),
            pane_item: hex(0x98989D),
            pane_item_selected: Color32::from_white_alpha(14),
            sidebar_bg: hex(0x28282A),
            sidebar_text: hex(0xD2D2D2),
            text_weak: hex(0x8D8D8D),
            text: hex(0xE4E4E4),
            divider: hex(0x5F5E5F),
            header_bg: hex(0x2A2A2B),
            accent: hex(0x0A84FF),
            row_selected: Color32::from_rgba_unmultiplied(10, 132, 255, 70),
            row_hover: Color32::from_rgba_unmultiplied(255, 255, 255, 14),
            live: hex(0x6CC24A),
            error: hex(0xFF5F57),
            warning: hex(0xF5B83D),
            success: hex(0x6CC24A),
            editor_bg: hex(0x2E2E2E),
            editor_text: hex(0xB4B4B4),
            syn_command: hex(0x00A0BE),
            syn_math: hex(0xD06F0E),
            syn_comment: hex(0x42B645),
            syn_verbatim: hex(0x9A9A9A),
            line_number: hex(0x676767),
            ghost_text: hex(0x7D7D7D),
            line_number_current: hex(0xB4B4B4),
            current_line: Color32::from_rgba_unmultiplied(255, 255, 255, 9),
            selection: Color32::from_rgba_unmultiplied(10, 132, 255, 80),
            editor_selection: Color32::from_white_alpha(22),
            find_match: Color32::from_rgba_unmultiplied(255, 214, 0, 90),
            bracket_match: Color32::from_rgba_unmultiplied(10, 132, 255, 60),
            pdf_bg: hex(0x1E1E1E),
            page_border: hex(0x111111),
            sync_flash: Color32::from_rgba_unmultiplied(80, 160, 255, 80),
            popup_bg: hex(0x2C2C2E),
            git_modified: hex(0xE2C08D),
            menu_border: Color32::from_white_alpha(34),
            menu_separator: Color32::from_white_alpha(26),
            menu_disabled: hex(0x6C6C70),
            control_bg: hex(0x1F1F20),
            control_border: hex(0x4A4A4C),
        }
    }

    /// Applies an editor colour scheme on top of the appearance (light themes only; dark mode
    /// keeps the dark editor).
    pub fn with_editor_theme(mut self, theme: texcraft_engine::EditorTheme) -> Self {
        if self.dark {
            return self;
        }
        match theme {
            texcraft_engine::EditorTheme::Classic => {
                // Cream paper with olive comments, as in the target app's light editor.
                self.editor_bg = hex(0xFCF8E4);
                self.editor_text = hex(0x1F1F1F);
                self.syn_comment = hex(0x809002);
                self.line_number = hex(0xC3BEAE);
                self.line_number_current = hex(0xC3BEAE);
                self.current_line = Color32::TRANSPARENT;
            }
            texcraft_engine::EditorTheme::Plain => {}
        }
        self
    }

    /// Editor colour for a syntax token.
    pub fn syntax(&self, kind: TokenKind) -> Color32 {
        match kind {
            TokenKind::Command | TokenKind::Sectioning | TokenKind::EnvironmentCommand => {
                self.syn_command
            }
            TokenKind::Math | TokenKind::MathCommand | TokenKind::MathDelimiter => self.syn_math,
            TokenKind::Comment | TokenKind::MagicComment => self.syn_comment,
            TokenKind::Verbatim => self.syn_verbatim,
            TokenKind::Text
            | TokenKind::EnvironmentName
            | TokenKind::Reference
            | TokenKind::Brace
            | TokenKind::Bracket
            | TokenKind::Special => self.editor_text,
        }
    }

    /// egui visuals for this appearance.
    pub fn visuals(&self) -> Visuals {
        let mut v = if self.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        v.panel_fill = self.editor_bg;
        v.window_fill = self.popup_bg;
        v.extreme_bg_color = self.control_bg;
        v.faint_bg_color = self.header_bg;
        v.override_text_color = Some(self.text);
        v.hyperlink_color = self.accent;
        v.selection.bg_fill = self.selection;
        // Selected text keeps its colour (egui would paint it in this one).
        v.selection.stroke = Stroke::new(1.0, self.text);
        v.text_cursor.stroke = Stroke::new(
            1.5,
            if self.dark {
                Color32::WHITE
            } else {
                Color32::BLACK
            },
        );
        v.window_corner_radius = 12.0.into();
        // egui's own pop-ups (tool tips, menus) look like `menu::frame`.
        v.menu_corner_radius = 10.0.into();
        v.window_stroke = Stroke::new(0.5, self.menu_border);
        v.popup_shadow = crate::menu::menu_shadow();
        v.window_shadow = egui::epaint::Shadow {
            offset: [0, 10],
            blur: 36,
            spread: 0,
            color: Color32::from_black_alpha(60),
        };
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.divider);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        v.widgets.inactive.bg_fill = self.control_bg;
        v.widgets.inactive.weak_bg_fill = self.control_bg;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, self.control_border);
        v.widgets.inactive.corner_radius = 5.0.into();
        v.widgets.hovered.corner_radius = 5.0.into();
        v.widgets.active.corner_radius = 5.0.into();
        v.widgets.open.corner_radius = 5.0.into();
        v
    }
}

/// Name of the editor font family.
pub const EDITOR_FAMILY: &str = "editor";

/// The editor font at `size`.
pub fn editor_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(EDITOR_FAMILY.into()))
}

/// Font family for semibold interface text: the system font at weight 600 (its `wght` axis).
pub const SEMIBOLD_FAMILY: &str = "semibold";

/// A semibold interface font.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD_FAMILY.into()))
}

/// Installs fonts: the system UI and monospace fonts when they exist (loaded at run time, not
/// shipped), with CJK and symbol fallbacks; egui's built-in fonts otherwise.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut load = |name: &str, paths: &[&str], index: u32| -> bool {
        for p in paths {
            if let Ok(bytes) = std::fs::read(p) {
                let mut data = FontData::from_owned(bytes);
                data.index = index;
                fonts.font_data.insert(name.to_string(), Arc::new(data));
                return true;
            }
        }
        false
    };
    let ui = load(
        "ui",
        &[
            "/System/Library/Fonts/SFNS.ttf",
            "C:\\Windows\\Fonts\\segoeui.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ],
        0,
    );
    let mono = load(
        "mono",
        &[
            "/System/Library/Fonts/SFNSMono.ttf",
            "/System/Library/Fonts/Menlo.ttc",
            "C:\\Windows\\Fonts\\consola.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        ],
        0,
    );
    let symbols = load(
        "symbols",
        &[
            "/System/Library/Fonts/Apple Symbols.ttf",
            "C:\\Windows\\Fonts\\seguisym.ttf",
        ],
        0,
    );
    let math = load(
        "math",
        &[
            "/System/Library/Fonts/Supplemental/STIXTwoMath.otf",
            "/System/Library/Fonts/STIXTwoMath.otf",
        ],
        0,
    );
    // Chinese, Japanese and Korean text: PingFang SC (else Hiragino Sans GB, …), moved down so its
    // baseline meets the system font's (see `cjk_font`).
    let primary = fonts
        .font_data
        .get("ui")
        .and_then(|d| crate::cjk_font::vmetrics(&d.font, d.index));
    let cjk = match crate::cjk_font::find() {
        Some((bytes, index)) => {
            let offset = primary
                .zip(crate::cjk_font::vmetrics(&bytes, index))
                .map_or(0.0, |(p, c)| crate::cjk_font::baseline_offset(p, c));
            let mut data = FontData::from_owned(bytes);
            data.index = index;
            data.tweak.y_offset_factor = offset;
            fonts.font_data.insert("cjk".into(), Arc::new(data));
            true
        }
        None => false,
    };

    // The system font is variable: its default optical size is the Display design (28 pt), but
    // AppKit sets small interface text in the wider Text design (the axis minimum, 17), so use
    // that. Semibold is the same font at weight 600. (Fonts without these axes ignore them.)
    let mut semibold = false;
    if let Some(d) = fonts.font_data.get("ui").cloned() {
        use egui::epaint::text::VariationCoords;
        let mut regular = (*d).clone();
        regular.tweak.coords = VariationCoords::new([(b"opsz", 17.0)]);
        let mut bold = (*d).clone();
        bold.tweak.coords = VariationCoords::new([(b"opsz", 17.0), (b"wght", 600.0)]);
        fonts.font_data.insert("ui".into(), Arc::new(regular));
        fonts.font_data.insert("ui-semibold".into(), Arc::new(bold));
        semibold = true;
    }
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    if ui {
        prop.insert(0, "ui".into());
    }
    let mut editor: Vec<String> = Vec::new();
    if mono {
        editor.push("mono".into());
    }
    editor.extend(
        fonts
            .families
            .get(&FontFamily::Monospace)
            .cloned()
            .unwrap_or_default(),
    );
    let monofam = fonts.families.entry(FontFamily::Monospace).or_default();
    if mono {
        monofam.insert(0, "mono".into());
    }
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = fonts.families.entry(fam).or_default();
        if cjk {
            list.push("cjk".into());
        }
        if math {
            list.push("math".into());
        }
        if symbols {
            list.push("symbols".into());
        }
    }
    let mut bold: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if semibold {
        bold.insert(0, "ui-semibold".into());
    }
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD_FAMILY.into()), bold);
    if cjk {
        editor.push("cjk".into());
    }
    if math {
        editor.push("math".into());
    }
    if symbols {
        editor.push("symbols".into());
    }
    fonts
        .families
        .insert(FontFamily::Name(EDITOR_FAMILY.into()), editor);
    ctx.set_fonts(fonts);
}

/// Applies spacing and text styles shared by both appearances.
pub fn install_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        use egui::TextStyle;
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(13.0));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(13.0));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(11.0));
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(17.0));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(12.0));
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.interact_size.y = 22.0;
        style.spacing.menu_margin = egui::Margin::same(crate::menu::MARGIN);
        // Scroll bars as the target shows them with a mouse attached: always-visible 11 pt
        // rounded thumbs, half-transparent text colour, no track, floating over the content.
        let scroll = &mut style.spacing.scroll;
        scroll.floating = true;
        scroll.bar_width = 11.0;
        scroll.floating_width = 11.0;
        scroll.floating_allocated_width = 0.0;
        scroll.bar_inner_margin = 3.5;
        scroll.bar_outer_margin = 3.5;
        scroll.handle_min_length = 24.0;
        scroll.foreground_color = true;
        scroll.dormant_handle_opacity = 0.58;
        scroll.active_handle_opacity = 0.58;
        scroll.interact_handle_opacity = 0.7;
        scroll.dormant_background_opacity = 0.0;
        scroll.active_background_opacity = 0.0;
        scroll.interact_background_opacity = 0.0;
        style.interaction.selectable_labels = false;
    });
}
