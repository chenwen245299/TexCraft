//! Measurement tool, not a check (ignored by default): prints how wide strings are in TexCraft's
//! interface fonts at several sizes, to find the size that matches a target screenshot (measure
//! the target with `scripts/parity/widths.py`, then pick the size whose width is closest; the
//! printed widths include ~0.6 pt of side bearings that ink measurements do not).
//!
//! ```sh
//! TEXCRAFT_WIDTHS='Recent Documents|Open a local file…|b' \
//!   cargo test -p texcraft-ui --test text_widths -- --ignored --nocapture
//! ```
//!
//! Strings are separated by `|`; a lone `b` after a string means "the semibold face".

use egui::{Color32, FontFamily, FontId};

#[test]
#[ignore = "measurement tool; run with --ignored"]
fn print_text_widths() {
    let spec = std::env::var("TEXCRAFT_WIDTHS").unwrap_or_else(|_| "Files|Words".to_string());
    let mut items: Vec<(String, bool)> = Vec::new();
    for part in spec.split('|') {
        if part == "b" {
            if let Some(last) = items.last_mut() {
                last.1 = true;
            }
        } else {
            items.push((part.to_string(), false));
        }
    }
    let ctx = egui::Context::default();
    texcraft_ui::theme::install_fonts(&ctx);
    ctx.begin_pass(egui::RawInput::default());
    for (text, bold) in &items {
        let family = if *bold {
            FontFamily::Name(texcraft_ui::theme::SEMIBOLD_FAMILY.into())
        } else {
            FontFamily::Proportional
        };
        let mut line = format!("{text:40} {}|", if *bold { "semibold " } else { "" });
        for size in [10.5_f32, 11.0, 11.5, 11.75, 12.0, 12.5, 13.0, 14.0] {
            let w = ctx.fonts_mut(|f| {
                f.layout_no_wrap(
                    text.clone(),
                    FontId::new(size, family.clone()),
                    Color32::BLACK,
                )
                .size()
                .x
            });
            line += &format!(" {size}:{w:.1}");
        }
        println!("{line}");
    }
    let _ = ctx.end_pass();
    ctx.tex_manager().write().take_delta().clear();
}
