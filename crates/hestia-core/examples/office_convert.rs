//! Conversion manuelle :
//!   cargo run -p hestia-core --example office_convert -- <entrée> <sortie>
//! Document Office → .pdf (avec Office installé), ou PDF → .docx / .pptx / .xlsx.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::time::Instant;

use hestia_core::office::{OfficeStatus, office_to_pdf};
use hestia_core::pdf_to_office::{OfficeFormat, pdf_to_office};
use hestia_core::{Job, PdfEngine, PdfInput};

fn main() {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/pdfium/pdfium.dll");
    let engine = PdfEngine::init(&dll).expect("moteur PDFium");
    let start = Instant::now();
    let ext = args[1].extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let format = match ext.as_str() {
        "docx" => Some(OfficeFormat::Docx),
        "pptx" => Some(OfficeFormat::Pptx),
        "xlsx" => Some(OfficeFormat::Xlsx),
        _ => None,
    };
    match format {
        Some(f) => {
            let r = pdf_to_office(engine, PdfInput::new(&args[0]), f, &args[1], &Job::silent());
            println!("{r:?}");
        }
        None => {
            let status = OfficeStatus::detect();
            println!("{status:?}");
            println!("{:?}", office_to_pdf(engine, &status, &args[0], &args[1], &Job::silent()));
        }
    }
    println!("en {:.1} s", start.elapsed().as_secs_f32());
}
