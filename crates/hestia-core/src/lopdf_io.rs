//! Chargement et vérification des documents modifiés avec `lopdf`
//! (compression, édition).

use lopdf::{Document, LoadOptions};

use crate::engine::{PdfEngine, PdfInput};
use crate::error::{HestiaError, Result};

/// Limite de décompression d'un flux (protection contre les « bombes »).
pub(crate) const MAX_STREAM_SIZE: usize = 512 * 1024 * 1024;

/// Document chargé, avec son nombre de pages selon PDFium.
pub(crate) struct Loaded {
    pub doc: Document,
    pub page_count: u32,
    /// Le document a dû être reconstruit (protégé ou mal formé) : les
    /// signets et données globales ont pu être perdus.
    pub rebuilt: bool,
}

/// Charge le document avec lopdf. Les fichiers protégés ou que lopdf lit
/// mal sont d'abord reconstruits par PDFium (qui répare et déchiffre).
pub(crate) fn load(engine: &PdfEngine, input: &PdfInput, original: &[u8]) -> Result<Loaded> {
    let probe = input.clone();
    let page_count = engine.run(move |ctx| {
        let doc = ctx.load(&probe)?;
        Ok(doc.pages().len() as u32)
    })?;
    if page_count == 0 {
        return Err(HestiaError::InvalidPdf("le document ne contient aucune page".into()));
    }

    let options = || LoadOptions::with_max_decompressed_size(MAX_STREAM_SIZE);
    if input.password().is_none()
        && let Ok(doc) = Document::load_mem_with_options(original, options())
        && !doc.is_encrypted()
        && !doc.was_encrypted()
        && doc.get_pages().len() as u32 == page_count
    {
        return Ok(Loaded { doc, page_count, rebuilt: false });
    }

    let source = input.clone();
    let rebuilt = engine.run(move |ctx| {
        let src = ctx.load(&source)?;
        let mut doc = ctx.pdfium().create_new_pdf()?;
        doc.pages_mut().append(&src)?;
        Ok(doc.save_to_bytes()?)
    })?;
    let doc = Document::load_mem_with_options(&rebuilt, options())?;
    if doc.get_pages().len() as u32 != page_count {
        return Err(HestiaError::InvalidPdf("structure des pages incohérente".into()));
    }
    Ok(Loaded { doc, page_count, rebuilt: true })
}

/// Ouvre le résultat avec PDFium et charge chaque page : le fichier doit
/// être lisible et avoir le même nombre de pages que l'original.
pub(crate) fn verify(engine: &PdfEngine, bytes: Vec<u8>, expected_pages: u32) -> Result<()> {
    engine.run(move |ctx| {
        let doc = ctx
            .pdfium()
            .load_pdf_from_byte_vec(bytes, None)
            .map_err(|e| HestiaError::Internal(format!("le fichier produit est illisible ({e:?})")))?;
        let count = doc.pages().len() as u32;
        if count != expected_pages {
            return Err(HestiaError::Internal(format!(
                "le fichier produit a {count} pages au lieu de {expected_pages}"
            )));
        }
        for page in doc.pages().iter() {
            drop(page);
        }
        Ok(())
    })
}
