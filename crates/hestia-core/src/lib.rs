//! # hestia-core
//!
//! Moteur PDF 100 % local d'Hestia PDF. Aucune donnée ne quitte la machine :
//! ce crate ne contient aucun code réseau.
//!
//! - [`engine`] : accès à PDFium (rendu, import de pages) sur un thread dédié.
//! - [`merge`], [`split`], [`compress`] : opérations sur les PDF.
//! - [`pdf_to_images`], [`images_to_pdf`] : conversions images.
//! - [`output`] : écritures atomiques, jamais d'écrasement des originaux.

pub mod compress;
pub mod edit;
pub mod engine;
pub mod error;
pub mod fonts;
pub mod images_to_pdf;
pub mod imaging;
pub mod inspect;
pub mod job;
mod lopdf_io;
pub mod merge;
pub mod office;
pub mod output;
pub mod pdf_to_images;
pub mod pdf_to_office;
pub mod ranges;
pub mod split;

pub use engine::{PdfEngine, PdfInput};
pub use error::{HestiaError, Result};
pub use job::{CancelToken, Job, Progress, ProgressFn};
pub use output::OutputFile;

/// Extensions d'images acceptées par « JPG en PDF ».
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff", "gif"];
