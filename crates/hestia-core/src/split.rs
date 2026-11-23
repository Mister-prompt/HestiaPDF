//! Diviser un PDF : par plages, par blocs de N pages, page par page,
//! ou en extrayant une sélection de pages.

use std::path::PathBuf;

use pdfium_render::prelude::*;
use serde::Deserialize;

use crate::engine::{PdfEngine, PdfInput};
use crate::error::{HestiaError, Result};
use crate::job::Job;
use crate::output::{OutputFile, ensure_not_input, write_atomic, write_new_in_dir};
use crate::ranges::parse_ranges;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum SplitMode {
    /// Plages personnalisées (`1-3, 5, 8-`). `merge` : tout dans un seul PDF.
    Ranges { ranges: String, merge: bool },
    /// Blocs fixes de `size` pages.
    Every { size: u32 },
    /// Un PDF par page.
    AllPages,
    /// Pages choisies (numéros à partir de 1). `merge` : un seul PDF.
    Select { pages: Vec<u32>, merge: bool },
}

/// Un fichier à produire : ses pages (indices à partir de 0) et son libellé.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitPart {
    pub pages: Vec<u32>,
    pub label: String,
}

/// Calcule les fichiers à produire pour un document de `page_count` pages.
pub fn plan(page_count: u32, mode: &SplitMode) -> Result<Vec<SplitPart>> {
    if page_count == 0 {
        return Err(HestiaError::InvalidRange("le document ne contient aucune page".into()));
    }
    let parts = match mode {
        SplitMode::Ranges { ranges, merge } => {
            let ranges = parse_ranges(ranges, page_count)?;
            if *merge {
                vec![part(ranges.iter().flat_map(|r| r.indices()).collect())]
            } else {
                ranges.iter().map(|r| part(r.indices().collect())).collect()
            }
        }
        SplitMode::Every { size } => {
            if *size == 0 {
                return Err(HestiaError::InvalidRange("le nombre de pages par fichier doit être d'au moins 1".into()));
            }
            (0..page_count).collect::<Vec<_>>().chunks(*size as usize).map(|c| part(c.to_vec())).collect()
        }
        SplitMode::AllPages => (0..page_count).map(|p| part(vec![p])).collect(),
        SplitMode::Select { pages, merge } => {
            let mut selected: Vec<u32> = pages.clone();
            selected.sort_unstable();
            selected.dedup();
            if selected.is_empty() {
                return Err(HestiaError::InvalidRange("aucune page sélectionnée".into()));
            }
            if let Some(bad) = selected.iter().find(|&&p| p == 0 || p > page_count) {
                return Err(HestiaError::InvalidRange(format!("page {bad} inexistante ({page_count} pages)")));
            }
            let indices: Vec<u32> = selected.iter().map(|p| p - 1).collect();
            if *merge { vec![part(indices)] } else { indices.into_iter().map(|p| part(vec![p])).collect() }
        }
    };
    Ok(parts)
}

fn part(pages: Vec<u32>) -> SplitPart {
    let label = label_for(&pages);
    SplitPart { pages, label }
}

/// `page-4`, `pages-2-7` ou `pages-1-3_5_8-9` (numéros à partir de 1).
fn label_for(pages: &[u32]) -> String {
    let runs = runs(pages);
    if pages.len() == 1 {
        return format!("page-{}", pages[0] + 1);
    }
    let text = runs
        .iter()
        .map(|&(a, b)| if a == b { format!("{}", a + 1) } else { format!("{}-{}", a + 1, b + 1) })
        .collect::<Vec<_>>()
        .join("_");
    let text = if text.len() > 60 { format!("{}-{}", pages[0] + 1, pages[pages.len() - 1] + 1) } else { text };
    format!("pages-{text}")
}

/// Regroupe des indices en suites consécutives `(début, fin)`.
fn runs(pages: &[u32]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for &p in pages {
        match out.last_mut() {
            Some((_, end)) if p == *end + 1 => *end = p,
            _ => out.push((p, p)),
        }
    }
    out
}

/// Où écrire le résultat.
#[derive(Clone, Debug)]
pub enum SplitTarget {
    /// Un seul fichier, choisi par l'utilisateur.
    File(PathBuf),
    /// Plusieurs fichiers `{stem}_{label}.pdf` dans un dossier.
    Folder { dir: PathBuf, stem: String },
}

pub fn split(
    engine: &PdfEngine,
    input: PdfInput,
    parts: Vec<SplitPart>,
    target: SplitTarget,
    job: Job,
) -> Result<Vec<OutputFile>> {
    if parts.is_empty() {
        return Err(HestiaError::NoInput);
    }
    if let SplitTarget::File(out) = &target {
        ensure_not_input(out, [input.path.as_path()])?;
        if parts.len() != 1 {
            return Err(HestiaError::Internal("plusieurs fichiers pour une seule destination".into()));
        }
    }

    engine.run(move |ctx| {
        let source = ctx.load(&input).map_err(|e| e.in_file(&input.path))?;
        let total = parts.len() as u32;
        let mut outputs = Vec::with_capacity(parts.len());
        for (i, part) in parts.iter().enumerate() {
            job.check()?;
            let mut doc = ctx.pdfium().create_new_pdf()?;
            for (start, end) in runs(&part.pages) {
                let at = doc.pages().len();
                doc.pages_mut().copy_page_range_from_document(
                    &source,
                    start as PdfPageIndex..=end as PdfPageIndex,
                    at,
                )?;
            }
            let written = match &target {
                SplitTarget::File(path) => {
                    ctx.evict(path);
                    write_atomic(path, |f| doc.save_to_writer(f).map_err(Into::into))?
                }
                SplitTarget::Folder { dir, stem } => {
                    write_new_in_dir(dir, &format!("{stem}_{}", part.label), "pdf", |f| {
                        doc.save_to_writer(f).map_err(Into::into)
                    })?
                }
            };
            outputs.push(written);
            job.report(i as u32 + 1, total);
        }
        Ok(outputs)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(parts: &[SplitPart]) -> Vec<Vec<u32>> {
        parts.iter().map(|p| p.pages.clone()).collect()
    }

    #[test]
    fn plans_each_mode() -> Result<()> {
        let r = plan(10, &SplitMode::Ranges { ranges: "1-3, 5".into(), merge: false })?;
        assert_eq!(pages(&r), vec![vec![0, 1, 2], vec![4]]);
        assert_eq!(r[0].label, "pages-1-3");
        assert_eq!(r[1].label, "page-5");

        let r = plan(10, &SplitMode::Ranges { ranges: "1-2, 9-".into(), merge: true })?;
        assert_eq!(pages(&r), vec![vec![0, 1, 8, 9]]);
        assert_eq!(r[0].label, "pages-1-2_9-10");

        let r = plan(5, &SplitMode::Every { size: 2 })?;
        assert_eq!(pages(&r), vec![vec![0, 1], vec![2, 3], vec![4]]);

        assert_eq!(plan(3, &SplitMode::AllPages)?.len(), 3);

        let r = plan(6, &SplitMode::Select { pages: vec![6, 2, 2, 3], merge: true })?;
        assert_eq!(pages(&r), vec![vec![1, 2, 5]]);
        Ok(())
    }

    #[test]
    fn rejects_bad_plans() {
        assert!(plan(5, &SplitMode::Every { size: 0 }).is_err());
        assert!(plan(5, &SplitMode::Select { pages: vec![], merge: true }).is_err());
        assert!(plan(5, &SplitMode::Select { pages: vec![6], merge: false }).is_err());
        assert!(plan(0, &SplitMode::AllPages).is_err());
    }
}
