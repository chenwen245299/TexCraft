//! Prints the extracted text and line boxes of a PDF page: `cargo run -p texcraft-pdf --example dump_text -- file.pdf [page]`.

use std::sync::Arc;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: dump_text <file.pdf> [page]");
        return;
    };
    let page = args.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let data = match std::fs::read(&path) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            eprintln!("{path}: {e}");
            return;
        }
    };
    match texcraft_pdf::page_text(data, page) {
        Ok(text) => {
            for line in &text.lines {
                let r = line.rect;
                println!(
                    "[{:7.2} {:7.2} {:7.2} {:7.2}] {:?}",
                    r.x0,
                    r.y0,
                    r.x1,
                    r.y1,
                    text.text(line.glyphs.clone())
                );
            }
        }
        Err(e) => eprintln!("{e}"),
    }
}
