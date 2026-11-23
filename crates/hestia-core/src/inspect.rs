//! Informations sur un PDF et rendu des pages (miniatures, visionneuse).

use pdfium_render::prelude::*;
use serde::Serialize;

use crate::engine::{PdfEngine, PdfInput};
use crate::error::{HestiaError, Result};
use crate::imaging::encode_jpeg;

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
pub struct PageSize {
    /// Largeur en points (1/72 de pouce), rotation de la page comprise.
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfInfo {
    pub page_count: u32,
    pub pages: Vec<PageSize>,
    pub file_size: u64,
    pub encrypted: bool,
}

/// Ouvre le PDF (avec son mot de passe éventuel) et décrit ses pages.
pub fn inspect(engine: &PdfEngine, input: PdfInput) -> Result<PdfInfo> {
    let file_size = std::fs::metadata(&input.path)?.len();
    let encrypted = input.password().is_some();
    engine.run(move |ctx| {
        let doc = ctx.cached(&input)?;
        let pages = doc
            .pages()
            .page_sizes()?
            .into_iter()
            .map(|r| PageSize { width: r.width().value, height: r.height().value })
            .collect::<Vec<_>>();
        Ok(PdfInfo { page_count: pages.len() as u32, pages, file_size, encrypted })
    })
}

/// Largeur maximale d'un rendu d'aperçu, en pixels.
pub const MAX_PREVIEW_WIDTH: u32 = 3000;

/// Rend la page `page_index` (à partir de 0) en JPEG, à la largeur demandée.
pub fn render_page_jpeg(engine: &PdfEngine, input: PdfInput, page_index: u32, width: u32) -> Result<Vec<u8>> {
    let width = width.clamp(32, MAX_PREVIEW_WIDTH) as Pixels;
    engine.run(move |ctx| {
        let doc = ctx.cached(&input)?;
        let count = doc.pages().len();
        if page_index as i64 >= count as i64 {
            return Err(HestiaError::InvalidRange(format!("page {} inexistante ({} pages)", page_index + 1, count)));
        }
        let page = doc.pages().get(page_index as PdfPageIndex)?;
        let config = PdfRenderConfig::new()
            .set_target_width(width)
            .set_maximum_height(width * 4)
            .render_form_data(true)
            .render_annotations(true);
        let image = page.render_with_config(&config)?.as_image()?;
        encode_jpeg(&image, 82)
    })
}
