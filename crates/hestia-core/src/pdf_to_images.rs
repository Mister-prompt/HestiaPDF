//! PDF en JPG : chaque page en image, ou extraction des images intégrées.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::path::PathBuf;

use pdfium_render::prelude::*;
use serde::Deserialize;

use crate::engine::{PdfEngine, PdfInput};
use crate::error::{HestiaError, Result};
use crate::imaging::{encode_jpeg, parse_jpeg_header};
use crate::job::Job;
use crate::output::{OutputFile, ensure_dir, write_new_in_dir};

/// Nombre maximal de pixels d'une page rendue (évite d'épuiser la mémoire
/// sur les très grands formats, ex. affiches).
const MAX_RENDER_PIXELS: f32 = 80_000_000.0;
/// Les images plus petites sont des décorations sans intérêt.
const MIN_EXTRACT_SIDE: u32 = 16;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExportQuality {
    /// 150 dpi : écran, partage.
    Normal,
    /// 300 dpi : impression.
    High,
}

impl ExportQuality {
    fn dpi(self) -> f32 {
        match self {
            ExportQuality::Normal => 150.0,
            ExportQuality::High => 300.0,
        }
    }

    fn jpeg_quality(self) -> u8 {
        match self {
            ExportQuality::Normal => 85,
            ExportQuality::High => 92,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum ImageExport {
    /// Chaque page devient une image JPG.
    Pages { quality: ExportQuality },
    /// Les images contenues dans le PDF sont extraites.
    Extract,
}

/// Convertit `input` en images JPG dans le dossier `dir`, en préfixant les
/// noms par `stem`.
pub fn pdf_to_images(
    engine: &PdfEngine,
    input: PdfInput,
    mode: ImageExport,
    dir: PathBuf,
    stem: String,
    job: Job,
) -> Result<Vec<OutputFile>> {
    let dir = ensure_dir(&dir)?;
    engine.run(move |ctx| {
        let doc = ctx.load(&input).map_err(|e| e.in_file(&input.path))?;
        match mode {
            ImageExport::Pages { quality } => export_pages(&doc, quality, &dir, &stem, &job),
            ImageExport::Extract => extract_images(&doc, &dir, &stem, &job),
        }
    })
}

fn export_pages(
    doc: &PdfDocument<'_>,
    quality: ExportQuality,
    dir: &std::path::Path,
    stem: &str,
    job: &Job,
) -> Result<Vec<OutputFile>> {
    let count = doc.pages().len().max(0) as u32;
    let digits = count.to_string().len().max(2);
    let mut outputs = Vec::with_capacity(count as usize);
    for index in 0..count {
        job.check()?;
        let page = doc.pages().get(index as PdfPageIndex)?;
        let (w_pt, h_pt) = (page.width().value.max(1.0), page.height().value.max(1.0));
        let mut scale = quality.dpi() / 72.0;
        let pixels = w_pt * h_pt * scale * scale;
        if pixels > MAX_RENDER_PIXELS {
            scale *= (MAX_RENDER_PIXELS / pixels).sqrt();
        }
        // Limite du format JPEG : 65 535 px de côté.
        scale = scale.min(65_000.0 / w_pt.max(h_pt));
        let config = PdfRenderConfig::new()
            .scale_page_by_factor(scale)
            .render_form_data(true)
            .render_annotations(true)
            .use_print_quality(true);
        let image = page.render_with_config(&config)?.as_image()?;
        drop(page);
        let jpeg = encode_jpeg(&image, quality.jpeg_quality())?;
        let name = format!("{stem}_page-{:0digits$}", index + 1);
        outputs.push(write_new_in_dir(dir, &name, "jpg", |f| f.write_all(&jpeg).map_err(Into::into))?);
        job.report(index + 1, count);
    }
    Ok(outputs)
}

fn extract_images(doc: &PdfDocument<'_>, dir: &std::path::Path, stem: &str, job: &Job) -> Result<Vec<OutputFile>> {
    let count = doc.pages().len().max(0) as u32;
    let mut seen: HashSet<u64> = HashSet::new();
    let mut outputs = Vec::new();
    for index in 0..count {
        job.check()?;
        let page = doc.pages().get(index as PdfPageIndex)?;
        let objects = page.objects();
        for i in 0..objects.len() {
            if let Ok(obj) = objects.get(i) {
                walk(&obj, &mut |img| {
                    if let Some(jpeg) = image_as_jpeg(img, &mut seen) {
                        let name = format!("{stem}_image-{:03}", outputs.len() + 1);
                        match write_new_in_dir(dir, &name, "jpg", |f| f.write_all(&jpeg).map_err(Into::into)) {
                            Ok(out) => outputs.push(out),
                            Err(e) => return Err(e),
                        }
                    }
                    Ok(())
                })?;
            }
        }
        job.report(index + 1, count);
    }
    if outputs.is_empty() {
        return Err(HestiaError::InvalidRange("ce PDF ne contient aucune image à extraire".into()));
    }
    Ok(outputs)
}

/// Parcourt un objet de page et ses sous-objets (formulaires XObject).
fn walk(obj: &PdfPageObject<'_>, f: &mut dyn FnMut(&PdfPageImageObject<'_>) -> Result<()>) -> Result<()> {
    match obj {
        PdfPageObject::Image(img) => f(img),
        PdfPageObject::XObjectForm(form) => {
            for i in 0..form.len() {
                if let Ok(child) = form.get(i) {
                    walk(&child, f)?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// JPEG d'une image intégrée, ou `None` si elle est trop petite ou déjà
/// extraite (même image réutilisée sur plusieurs pages).
fn image_as_jpeg(img: &PdfPageImageObject<'_>, seen: &mut HashSet<u64>) -> Option<Vec<u8>> {
    let raw = img.get_raw_image_data().ok()?;
    if raw.is_empty() {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    raw.hash(&mut hasher);
    if !seen.insert(hasher.finish()) {
        return None;
    }

    // Un JPEG RVB ou gris est copié tel quel, sans perte.
    let filters: Vec<String> = img.filters().iter().map(|f| f.name().to_string()).collect();
    let is_cmyk = matches!(img.color_space(), Ok(PdfColorSpace::DeviceCMYK));
    if filters.len() == 1
        && filters[0] == "DCTDecode"
        && !is_cmyk
        && let Some(h) = parse_jpeg_header(&raw)
        && (h.components == 1 || h.components == 3)
    {
        if h.width < MIN_EXTRACT_SIDE || h.height < MIN_EXTRACT_SIDE {
            return None;
        }
        return Some(raw);
    }

    let image = img.get_raw_image().ok()?;
    if image.width() < MIN_EXTRACT_SIDE || image.height() < MIN_EXTRACT_SIDE {
        return None;
    }
    encode_jpeg(&image, 92).ok()
}
