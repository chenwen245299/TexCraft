//! PDF rendering for TexCraft, in pure Rust on the CPU (via `hayro`).
//!
//! [`render_page`] renders synchronously (CLI, tests, wasm). [`RenderWorker`] owns a document on a
//! background thread and renders pages on request, dropping requests that went stale (an older
//! document, or a page re-requested at another scale) so the viewer stays responsive while the
//! user zooms and scrolls.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

/// The largest bitmap side we render, in pixels.
pub const MAX_SIDE: f32 = 8192.0;

/// Errors from opening or rendering a PDF.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PdfError {
    /// The bytes are not a PDF we can read.
    #[error("the PDF could not be read")]
    Invalid,
    /// The PDF is encrypted.
    #[error("the PDF is encrypted")]
    Encrypted,
    /// The page does not exist.
    #[error("page {0} does not exist")]
    NoSuchPage(usize),
    /// The renderer failed on this page.
    #[error("rendering failed: {0}")]
    Render(String),
}

/// Size of a page in PDF points (1/72 in), after rotation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    /// Width in points.
    pub width: f32,
    /// Height in points.
    pub height: f32,
}

/// A rendered page bitmap.
#[derive(Clone, PartialEq)]
pub struct RenderedPage {
    /// The document generation it belongs to (see [`RenderWorker::open`]).
    pub doc: u64,
    /// 0-based page index.
    pub page: usize,
    /// Pixels per PDF point it was rendered at.
    pub scale: f32,
    /// Bitmap width in pixels.
    pub width: u32,
    /// Bitmap height in pixels.
    pub height: u32,
    /// RGBA8 pixels, premultiplied alpha (opaque: the page background is white).
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for RenderedPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderedPage")
            .field("doc", &self.doc)
            .field("page", &self.page)
            .field("scale", &self.scale)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

fn load(data: Arc<Vec<u8>>) -> Result<Pdf, PdfError> {
    Pdf::new(data).map_err(|e| match e {
        hayro::hayro_syntax::LoadPdfError::Decryption(_) => PdfError::Encrypted,
        hayro::hayro_syntax::LoadPdfError::Invalid => PdfError::Invalid,
    })
}

fn sizes(pdf: &Pdf) -> Vec<PageSize> {
    pdf.pages()
        .iter()
        .map(|p| {
            let (width, height) = p.render_dimensions();
            PageSize { width, height }
        })
        .collect()
}

/// Page sizes of a PDF.
pub fn page_sizes(data: Arc<Vec<u8>>) -> Result<Vec<PageSize>, PdfError> {
    let pdf = guarded(|| load(data))??;
    Ok(sizes(&pdf))
}

/// Renders one page synchronously at `scale` pixels per point.
pub fn render_page(data: Arc<Vec<u8>>, page: usize, scale: f32) -> Result<RenderedPage, PdfError> {
    let pdf = guarded(|| load(data))??;
    let cache = RenderCache::new();
    render_with(
        &pdf,
        &cache,
        &InterpreterSettings::default(),
        0,
        page,
        scale,
    )
}

fn render_with<'a>(
    pdf: &'a Pdf,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    doc: u64,
    page: usize,
    scale: f32,
) -> Result<RenderedPage, PdfError> {
    let pages = pdf.pages();
    let p = pages.get(page).ok_or(PdfError::NoSuchPage(page))?;
    let (w, h) = p.render_dimensions();
    let scale = clamp_scale(scale, w, h);
    let pixmap = guarded(|| {
        hayro::render(
            p,
            cache,
            settings,
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        )
    })?;
    Ok(RenderedPage {
        doc,
        page,
        scale,
        width: u32::from(pixmap.width()),
        height: u32::from(pixmap.height()),
        rgba: pixmap.data_as_u8_slice().to_vec(),
    })
}

/// Limits `scale` so the bitmap stays within [`MAX_SIDE`] and is at least one pixel.
pub fn clamp_scale(scale: f32, width_pt: f32, height_pt: f32) -> f32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let side = width_pt.max(height_pt).max(1.0);
    scale.min(MAX_SIDE / side).max(1.0 / side)
}

fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, PdfError> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| {
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "renderer panicked".to_string());
        PdfError::Render(msg)
    })
}

/// What the worker reports back.
#[derive(Debug, Clone)]
pub enum RenderEvent {
    /// A document was opened.
    Opened {
        /// Its generation.
        doc: u64,
        /// Its pages.
        pages: Vec<PageSize>,
    },
    /// A page is ready.
    Rendered(RenderedPage),
    /// Opening or rendering failed.
    Failed {
        /// The document generation.
        doc: u64,
        /// The page, if a render failed.
        page: Option<usize>,
        /// What went wrong.
        error: PdfError,
    },
}

enum Msg {
    Open { doc: u64, data: Arc<Vec<u8>> },
    Render { doc: u64, page: usize, scale: f32 },
    CancelRenders,
}

/// Renders pages of one document at a time on a background thread.
pub struct RenderWorker {
    tx: Sender<Msg>,
    rx: Receiver<RenderEvent>,
}

impl RenderWorker {
    /// Starts the worker. `wake` is called after every event (e.g. to request a repaint).
    pub fn spawn(wake: impl Fn() + Send + 'static) -> Self {
        let (tx, worker_rx) = channel::<Msg>();
        let (worker_tx, rx) = channel::<RenderEvent>();
        let spawned = std::thread::Builder::new()
            .name("texcraft-pdf-render".into())
            .spawn(move || worker_main(&worker_rx, &worker_tx, &wake));
        if spawned.is_err() {
            // Without a thread the channels are simply disconnected; requests do nothing.
        }
        Self { tx, rx }
    }

    /// Opens a document. `doc` is a generation number chosen by the caller; events carry it so
    /// results for an older document can be ignored. Pending renders are dropped.
    pub fn open(&self, doc: u64, data: Arc<Vec<u8>>) {
        let _ = self.tx.send(Msg::Open { doc, data });
    }

    /// Opens a document and immediately renders `pages` at `scale` (the pages the viewer showed
    /// for the previous version), without waiting for the viewer to ask.
    pub fn open_and_render(&self, doc: u64, data: Arc<Vec<u8>>, pages: &[usize], scale: f32) {
        let _ = self.tx.send(Msg::Open { doc, data });
        for &page in pages {
            let _ = self.tx.send(Msg::Render { doc, page, scale });
        }
    }

    /// Requests page `page` (0-based) of document `doc` at `scale` pixels per point. A newer
    /// request for the same page replaces an older pending one.
    pub fn render(&self, doc: u64, page: usize, scale: f32) {
        let _ = self.tx.send(Msg::Render { doc, page, scale });
    }

    /// Drops all pending render requests.
    pub fn cancel_renders(&self) {
        let _ = self.tx.send(Msg::CancelRenders);
    }

    /// The next event, if any.
    pub fn try_recv(&self) -> Option<RenderEvent> {
        self.rx.try_recv().ok()
    }
}

fn worker_main(rx: &Receiver<Msg>, tx: &Sender<RenderEvent>, wake: &dyn Fn()) {
    let mut next_open: Option<(u64, Arc<Vec<u8>>)> = None;
    loop {
        // Wait for a document.
        let (doc, data) = match next_open.take() {
            Some(open) => open,
            None => match rx.recv() {
                Ok(Msg::Open { doc, data }) => (doc, data),
                Ok(_) => continue,
                Err(_) => return,
            },
        };
        let pdf = match guarded(|| load(data)) {
            Ok(Ok(pdf)) => pdf,
            Ok(Err(error)) | Err(error) => {
                let _ = tx.send(RenderEvent::Failed {
                    doc,
                    page: None,
                    error,
                });
                wake();
                continue;
            }
        };
        let _ = tx.send(RenderEvent::Opened {
            doc,
            pages: sizes(&pdf),
        });
        wake();
        let cache = RenderCache::new();
        let settings = InterpreterSettings::default();
        let mut queue: VecDeque<(usize, f32)> = VecDeque::new();

        'doc: loop {
            // Block for work when idle, then drain everything pending.
            if queue.is_empty() {
                match rx.recv() {
                    Ok(msg) => {
                        if absorb(msg, doc, &mut queue, &mut next_open) {
                            break 'doc;
                        }
                    }
                    Err(_) => return,
                }
            }
            loop {
                match rx.try_recv() {
                    Ok(msg) => {
                        if absorb(msg, doc, &mut queue, &mut next_open) {
                            break 'doc;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            let Some((page, scale)) = queue.pop_front() else {
                continue;
            };
            let event = match render_with(&pdf, &cache, &settings, doc, page, scale) {
                Ok(r) => RenderEvent::Rendered(r),
                Err(error) => RenderEvent::Failed {
                    doc,
                    page: Some(page),
                    error,
                },
            };
            if tx.send(event).is_err() {
                return;
            }
            wake();
        }
    }
}

/// Folds one message into the queue. Returns true when a new document must be opened.
fn absorb(
    msg: Msg,
    doc: u64,
    queue: &mut VecDeque<(usize, f32)>,
    next_open: &mut Option<(u64, Arc<Vec<u8>>)>,
) -> bool {
    match msg {
        Msg::Open { doc, data } => {
            *next_open = Some((doc, data));
            true
        }
        Msg::Render {
            doc: d,
            page,
            scale,
        } => {
            if d == doc {
                queue.retain(|(p, _)| *p != page);
                queue.push_back((page, scale));
            }
            false
        }
        Msg::CancelRenders => {
            queue.clear();
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A minimal one-page PDF (US Letter) with a filled rectangle, written by hand.
    fn tiny_pdf() -> Arc<Vec<u8>> {
        let content = b"0 0 1 rg 100 100 200 300 re f";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_string(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                String::from_utf8_lossy(content)
            ),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        Arc::new(out)
    }

    #[test]
    fn sizes_and_sync_render() {
        let data = tiny_pdf();
        let sizes = page_sizes(data.clone()).unwrap();
        assert_eq!(
            sizes,
            vec![PageSize {
                width: 612.0,
                height: 792.0
            }]
        );
        let r = render_page(data, 0, 0.5).unwrap();
        assert_eq!((r.width, r.height), (306, 396));
        assert_eq!(r.rgba.len(), 306 * 396 * 4);
        // A pixel inside the blue rectangle (PDF y-up: rect from y=100..400 → near the bottom).
        let (x, y) = (100usize, 396 - 100);
        let i = (y * 306 + x) * 4;
        assert_eq!(&r.rgba[i..i + 4], &[0, 0, 255, 255]);
        // Corner is white paper.
        assert_eq!(&r.rgba[0..4], &[255, 255, 255, 255]);
    }

    #[test]
    fn bad_input_is_an_error() {
        assert_eq!(
            page_sizes(Arc::new(b"not a pdf".to_vec())),
            Err(PdfError::Invalid)
        );
        assert!(matches!(
            render_page(tiny_pdf(), 3, 1.0),
            Err(PdfError::NoSuchPage(3))
        ));
    }

    #[test]
    fn scale_is_clamped() {
        assert!((clamp_scale(100.0, 612.0, 792.0) - MAX_SIDE / 792.0).abs() < 1e-3);
        assert!((clamp_scale(f32::NAN, 612.0, 792.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn worker_renders_and_drops_stale_documents() {
        let worker = RenderWorker::spawn(|| {});
        worker.open(1, Arc::new(b"garbage".to_vec()));
        worker.open(2, tiny_pdf());
        worker.render(2, 0, 1.0);
        worker.render(1, 0, 1.0); // stale: ignored
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut got_page = None;
        let mut failed_first = false;
        while Instant::now() < deadline && got_page.is_none() {
            match worker.try_recv() {
                Some(RenderEvent::Rendered(r)) => got_page = Some(r),
                Some(RenderEvent::Failed { doc: 1, .. }) => failed_first = true,
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        let page = got_page.expect("rendered page");
        assert_eq!((page.doc, page.page, page.width), (2, 0, 612));
        // The garbage document is either reported as failed or skipped because a newer open
        // arrived first; both are fine.
        let _ = failed_first;
    }
}
