//! Organiser les pages : nouvel ordre, pages retirées, pivotées ou blanches,
//! à partir d'un ou plusieurs PDF.

use std::path::PathBuf;

use pdfium_render::prelude::*;
use serde::Deserialize;

use crate::engine::{PdfEngine, PdfInput};
use crate::error::{HestiaError, Result};
use crate::job::Job;
use crate::output::{OutputFile, ensure_not_input, write_atomic};

/// Côté maximal d'une page blanche, en points (PDF : 14 400 au plus).
const MAX_SIDE: f32 = 14_400.0;

/// Une page du document final, dans l'ordre.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PageSpec {
    /// Page `page` (à partir de 0) du fichier `file` (indice dans les
    /// entrées), tournée de `rotation` degrés en plus de sa rotation propre.
    #[serde(rename_all = "camelCase")]
    Source {
        file: usize,
        page: u32,
        #[serde(default)]
        rotation: u16,
    },
    /// Page vide de `width` × `height` points, tournée de `rotation` degrés.
    #[serde(rename_all = "camelCase")]
    Blank {
        width: f32,
        height: f32,
        #[serde(default)]
        rotation: u16,
    },
}

impl PageSpec {
    fn rotation(&self) -> u16 {
        match self {
            PageSpec::Source { rotation, .. } | PageSpec::Blank { rotation, .. } => *rotation,
        }
    }
}

/// Vérifie le plan avant d'ouvrir les fichiers.
fn validate(inputs: &[PdfInput], pages: &[PageSpec]) -> Result<()> {
    if pages.is_empty() {
        return Err(HestiaError::InvalidRange("le document final ne contient aucune page".into()));
    }
    for spec in pages {
        if spec.rotation() % 90 != 0 || spec.rotation() >= 360 {
            return Err(HestiaError::InvalidRange(format!("rotation invalide : {}°", spec.rotation())));
        }
        match spec {
            PageSpec::Source { file, .. } if *file >= inputs.len() => {
                return Err(HestiaError::InvalidRange(format!("fichier {file} inconnu")));
            }
            PageSpec::Blank { width, height, .. }
                if !(1.0..=MAX_SIDE).contains(width) || !(1.0..=MAX_SIDE).contains(height) =>
            {
                return Err(HestiaError::InvalidRange(format!("page blanche de taille invalide ({width} × {height})")));
            }
            _ => {}
        }
    }
    Ok(())
}

fn render_rotation(degrees: u16) -> PdfPageRenderRotation {
    match degrees % 360 {
        90 => PdfPageRenderRotation::Degrees90,
        180 => PdfPageRenderRotation::Degrees180,
        270 => PdfPageRenderRotation::Degrees270,
        _ => PdfPageRenderRotation::None,
    }
}

fn degrees(rotation: PdfPageRenderRotation) -> u16 {
    match rotation {
        PdfPageRenderRotation::None => 0,
        PdfPageRenderRotation::Degrees90 => 90,
        PdfPageRenderRotation::Degrees180 => 180,
        PdfPageRenderRotation::Degrees270 => 270,
    }
}

/// Écrit dans `output` un PDF composé des pages `pages`, dans cet ordre.
pub fn organize(
    engine: &PdfEngine,
    inputs: Vec<PdfInput>,
    pages: Vec<PageSpec>,
    output: PathBuf,
    job: Job,
) -> Result<OutputFile> {
    if inputs.is_empty() && pages.iter().any(|p| matches!(p, PageSpec::Source { .. })) {
        return Err(HestiaError::NoInput);
    }
    validate(&inputs, &pages)?;
    ensure_not_input(&output, inputs.iter().map(|i| i.path.as_path()))?;

    engine.run(move |ctx| {
        // Chaque fichier n'est ouvert qu'une fois, et seulement s'il sert.
        let mut sources: Vec<Option<PdfDocument<'static>>> = (0..inputs.len()).map(|_| None).collect();
        for spec in &pages {
            if let PageSpec::Source { file, page, .. } = spec {
                let input = &inputs[*file];
                if sources[*file].is_none() {
                    job.check()?;
                    sources[*file] = Some(ctx.load(input).map_err(|e| e.in_file(&input.path))?);
                }
                let count = sources[*file].as_ref().map_or(0, |d| d.pages().len() as u32);
                if *page >= count {
                    return Err(HestiaError::InvalidRange(format!("page {} inexistante ({count} pages)", page + 1))
                        .in_file(&input.path));
                }
            }
        }

        let total = pages.len() as u32;
        let mut doc = ctx.pdfium().create_new_pdf()?;
        let mut i = 0;
        while i < pages.len() {
            job.check()?;
            let at = doc.pages().len();
            match &pages[i] {
                PageSpec::Blank { width, height, .. } => {
                    doc.pages_mut().create_page_at_end(PdfPagePaperSize::new_custom(
                        PdfPoints::new(*width),
                        PdfPoints::new(*height),
                    ))?;
                    i += 1;
                }
                PageSpec::Source { file, .. } => {
                    // Pages consécutives du même fichier : un seul import, pour
                    // que les polices et images partagées ne soient copiées qu'une fois.
                    let mut numbers = Vec::new();
                    while let Some(PageSpec::Source { file: f, page, .. }) = pages.get(i) {
                        if f != file {
                            break;
                        }
                        numbers.push((page + 1).to_string());
                        i += 1;
                    }
                    let source =
                        sources[*file].as_ref().ok_or_else(|| HestiaError::Internal("source non ouverte".into()))?;
                    doc.pages_mut()
                        .copy_pages_from_document(source, &numbers.join(","), at)
                        .map_err(|e| HestiaError::from(e).in_file(&inputs[*file].path))?;
                }
            }
            job.report(i as u32, total);
        }

        if doc.pages().len() as usize != pages.len() {
            return Err(HestiaError::Internal(format!(
                "{} pages produites au lieu de {}",
                doc.pages().len(),
                pages.len()
            )));
        }
        for (index, spec) in pages.iter().enumerate() {
            if spec.rotation() == 0 {
                continue;
            }
            let mut page = doc.pages().get(index as PdfPageIndex)?;
            let current = degrees(page.rotation()?);
            page.set_rotation(render_rotation(current + spec.rotation()));
        }

        job.check()?;
        ctx.evict(&output);
        write_atomic(&output, |file| doc.save_to_writer(file).map_err(Into::into))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_plans() {
        let one = vec![PdfInput::new("a.pdf")];
        assert!(validate(&one, &[]).is_err());
        assert!(validate(&one, &[PageSpec::Source { file: 1, page: 0, rotation: 0 }]).is_err());
        assert!(validate(&one, &[PageSpec::Source { file: 0, page: 0, rotation: 45 }]).is_err());
        assert!(validate(&one, &[PageSpec::Source { file: 0, page: 0, rotation: 360 }]).is_err());
        assert!(validate(&one, &[PageSpec::Blank { width: 0.0, height: 800.0, rotation: 0 }]).is_err());
        assert!(validate(&one, &[PageSpec::Blank { width: f32::NAN, height: 800.0, rotation: 0 }]).is_err());
        assert!(validate(&one, &[PageSpec::Source { file: 0, page: 3, rotation: 270 }]).is_ok());
    }

    #[test]
    fn reads_the_interface_format() {
        let specs: Vec<PageSpec> = serde_json::from_str(
            r#"[{"kind":"source","file":1,"page":4,"rotation":90},{"kind":"blank","width":595,"height":842}]"#,
        )
        .unwrap_or_default();
        assert_eq!(
            specs,
            vec![
                PageSpec::Source { file: 1, page: 4, rotation: 90 },
                PageSpec::Blank { width: 595.0, height: 842.0, rotation: 0 },
            ]
        );
    }
}
