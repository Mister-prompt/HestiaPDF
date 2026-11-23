//! Fusionner plusieurs PDF en un seul, dans l'ordre donné.

use std::path::PathBuf;

use crate::engine::{PdfEngine, PdfInput};
use crate::error::{HestiaError, Result};
use crate::job::Job;
use crate::output::{OutputFile, ensure_not_input, write_atomic};

pub fn merge(engine: &PdfEngine, inputs: Vec<PdfInput>, output: PathBuf, job: Job) -> Result<OutputFile> {
    if inputs.is_empty() {
        return Err(HestiaError::NoInput);
    }
    ensure_not_input(&output, inputs.iter().map(|i| i.path.as_path()))?;

    engine.run(move |ctx| {
        let total = inputs.len() as u32;
        let mut merged = ctx.pdfium().create_new_pdf()?;
        for (i, input) in inputs.iter().enumerate() {
            job.check()?;
            let source = ctx.load(input).map_err(|e| e.in_file(&input.path))?;
            merged.pages_mut().append(&source).map_err(|e| HestiaError::from(e).in_file(&input.path))?;
            job.report(i as u32 + 1, total);
        }
        job.check()?;
        ctx.evict(&output);
        write_atomic(&output, |file| merged.save_to_writer(file).map_err(Into::into))
    })
}
