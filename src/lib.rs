//! pdfcraft-explorer-thumbs: first-page thumbnails of PDF files for Windows File Explorer.
//!
//! A local add-on, built and installed separately from PdfCraft (see the README). The DLL links
//! the renderer statically, so it keeps working whichever PdfCraft version is installed, or none.
//!
//! [`first_page`] is the platform-independent part. It uses three things from `pdfcraft-render`
//! and nothing else: `PageRenderer::new`, `page_count` and `render`. Keeping that surface small
//! is what lets this crate follow `main` without edits. The [`windows`] module is the COM glue.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RenderedPage, RequestKind};

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows;

/// The largest square a thumbnail is fitted into. File Explorer's largest cached size is 2560.
pub const MAX_SIDE: u32 = 2560;

/// The scale of the first, small render, which exists only to learn the page's proportions: the
/// renderer on `main` has no call that returns a page's size. US Letter comes out 123 × 159.
const PROBE_SCALE: f32 = 0.2;

/// A picture of a first page: `width` × `height` pixels, rows from the top, four bytes per pixel
/// in B, G, R, A order (a Windows 32-bit bitmap), every pixel opaque.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Why there is no thumbnail. File Explorer shows the file's icon in every case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThumbnailError {
    /// Not a readable PDF, password-protected, or without pages.
    Unreadable,
    /// The first page could not be drawn; the renderer's message.
    Render(String),
    /// The renderer panicked. Caught here, so the host process survives.
    Crashed,
}

impl fmt::Display for ThumbnailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable => f.write_str("the file could not be opened as a PDF, or has no pages"),
            Self::Render(why) => write!(f, "the first page could not be drawn: {why}"),
            Self::Crashed => f.write_str("the renderer crashed"),
        }
    }
}

impl std::error::Error for ThumbnailError {}

/// Draws the first page of the PDF in `bytes` on white paper, fitted inside a `side` × `side`
/// square (`side` is kept between 16 and [`MAX_SIDE`]). Never panics.
pub fn first_page(bytes: Vec<u8>, side: u32) -> Result<Thumbnail, ThumbnailError> {
    let side = side.clamp(16, MAX_SIDE);
    catch_unwind(AssertUnwindSafe(|| draw(bytes, side))).unwrap_or(Err(ThumbnailError::Crashed))
}

fn draw(bytes: Vec<u8>, side: u32) -> Result<Thumbnail, ThumbnailError> {
    let mut renderer = PageRenderer::new(Arc::new(bytes), RenderConfig::default());
    if renderer.page_count() == 0 {
        return Err(ThumbnailError::Unreadable);
    }
    let probe = render(&mut renderer, PROBE_SCALE)?;
    let mut scale = PROBE_SCALE * side as f32 / probe.width.max(probe.height) as f32;
    let mut page = render(&mut renderer, scale)?;
    // The probe rounds the page up to whole pixels, so the page is usually a little smaller than
    // the square. A page the renderer had to shrink to its own limits in the probe comes out
    // larger; one more render at the measured ratio brings it inside.
    let longest = page.width.max(page.height);
    if longest > side {
        scale *= (side as f32 - 0.5) / longest as f32;
        page = render(&mut renderer, scale)?;
    }
    if page.width.max(page.height) > side {
        return Err(ThumbnailError::Render("the page does not fit the requested size".into()));
    }
    Ok(Thumbnail { width: page.width, height: page.height, bgra: on_white_bgra(&page.rgba) })
}

fn render(renderer: &mut PageRenderer, scale: f32) -> Result<RenderedPage, ThumbnailError> {
    let page = renderer.render(RenderRequest { page: 0, kind: RequestKind::Pixels, tile: None, scale, tag: 0, preview: true });
    if let Some(why) = page.error {
        return Err(ThumbnailError::Render(why));
    }
    let pixels = (page.width as usize).checked_mul(page.height as usize).and_then(|n| n.checked_mul(4));
    if page.width == 0 || page.height == 0 || pixels != Some(page.rgba.len()) {
        return Err(ThumbnailError::Render("the renderer returned no pixels".into()));
    }
    Ok(page)
}

/// Premultiplied RGBA over white paper, as opaque BGRA: each channel gains the share of white
/// that shows through, 255 − alpha.
fn on_white_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut bgra = Vec::with_capacity(rgba.len());
    for px in rgba.as_chunks::<4>().0 {
        let paper = 255 - px[3];
        bgra.extend_from_slice(&[px[2].saturating_add(paper), px[1].saturating_add(paper), px[0].saturating_add(paper), 255]);
    }
    bgra
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A one-page PDF, `width` × `height` points, drawing `content`.
    pub(crate) fn one_page_pdf(width: u32, height: u32, content: &str) -> Vec<u8> {
        let bodies = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Contents 4 0 R >>"),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut file = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (number, body) in bodies.iter().enumerate() {
            offsets.push(file.len());
            file.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", number + 1).as_bytes());
        }
        let xref = file.len();
        file.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
        for offset in offsets {
            file.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        file.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
        file
    }

    fn pixel(thumb: &Thumbnail, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * thumb.width + x) * 4) as usize;
        [thumb.bgra[at], thumb.bgra[at + 1], thumb.bgra[at + 2], thumb.bgra[at + 3]]
    }

    #[test]
    fn a_page_fills_the_square_along_its_longer_side_and_keeps_its_shape() {
        for (width, height) in [(600, 300), (300, 600), (612, 792)] {
            for side in [96, 256, 1024] {
                let thumb = first_page(one_page_pdf(width, height, ""), side).expect("thumbnail");
                let (longest, shortest) = (thumb.width.max(thumb.height), thumb.width.min(thumb.height));
                assert!(longest <= side, "{width}×{height} at {side}: {longest} px does not fit");
                assert!(longest * 100 >= side * 97, "{width}×{height} at {side}: only {longest} px used");
                assert_eq!(thumb.width > thumb.height, width > height, "{width}×{height}: orientation");
                let drawn = shortest as f32 / longest as f32;
                let real = width.min(height) as f32 / width.max(height) as f32;
                assert!((drawn - real).abs() < 0.03, "{width}×{height} at {side}: proportions {drawn} vs {real}");
                assert_eq!(thumb.bgra.len(), (thumb.width * thumb.height * 4) as usize);
            }
        }
    }

    #[test]
    fn paper_is_white_and_colours_are_in_windows_byte_order() {
        // The lower half of the page (PDF's origin is bottom-left) filled with pure red.
        let thumb = first_page(one_page_pdf(200, 200, "1 0 0 rg 0 0 200 100 re f"), 200).expect("thumbnail");
        let paper = pixel(&thumb, thumb.width / 2, thumb.height / 4);
        let red = pixel(&thumb, thumb.width / 2, thumb.height * 3 / 4);
        assert_eq!(paper, [255, 255, 255, 255], "unpainted paper is opaque white, not transparent");
        assert!(red[2] > 200 && red[0] < 50 && red[1] < 50, "red is the third byte (B, G, R, A): {red:?}");
        assert_eq!(red[3], 255);
    }

    #[test]
    fn files_that_are_not_pdfs_are_errors() {
        let whole = one_page_pdf(200, 200, "");
        for bytes in [Vec::new(), b"not a PDF at all".to_vec(), whole[..40].to_vec(), vec![0xFF; 4096]] {
            assert!(first_page(bytes, 256).is_err());
        }
    }
}
