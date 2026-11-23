//! Compresser un PDF.
//!
//! 1. Lecture avec `lopdf` (ou reconstruction via PDFium si le fichier est
//!    protégé ou mal formé).
//! 2. Recompression des images photographiques en JPEG, avec réduction de
//!    résolution selon le niveau choisi. Les images « graphiques » (peu de
//!    couleurs) et les masques de transparence restent sans perte.
//! 3. Suppression des données inutiles à l'affichage (miniatures
//!    intégrées, données privées d'Illustrator…), compression des flux,
//!    flux d'objets.
//! 4. Vérification du résultat par PDFium **avant** écriture. Si le
//!    fichier n'est pas plus petit, l'original est conservé tel quel.

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

use image::{DynamicImage, GrayImage, RgbImage, imageops::FilterType};
use lopdf::{Dictionary, Document, Object, ObjectId, SaveOptions, Stream};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::engine::{PdfEngine, PdfInput};
use crate::error::{HestiaError, Result};
use crate::imaging::{decode_bytes, encode_jpeg, parse_jpeg_header};
use crate::job::Job;
use crate::lopdf_io;
use crate::output::{OutputFile, ensure_not_input, write_atomic};

/// Au-delà, une image n'est pas recompressée (mémoire).
const MAX_IMAGE_PIXELS: u64 = 120_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CompressionLevel {
    /// Qualité maximale : seules les images très haute définition sont réduites.
    Low,
    /// Bon équilibre taille / qualité (~150 dpi).
    Recommended,
    /// Taille minimale (~96 dpi), qualité d'écran.
    Extreme,
}

#[derive(Clone, Copy, Debug)]
struct Settings {
    /// Plus grand côté autorisé pour une image, en pixels.
    max_side: u32,
    quality: u8,
    /// Réencoder les JPEG même s'ils ne sont pas réduits.
    recompress_jpeg: bool,
    /// Convertir les images sans perte (photos) en JPEG.
    convert_lossless: bool,
}

impl CompressionLevel {
    fn settings(self) -> Settings {
        match self {
            // Côté max ≈ A4 à 250 / 150 / 96 dpi.
            CompressionLevel::Low => {
                Settings { max_side: 2900, quality: 85, recompress_jpeg: false, convert_lossless: false }
            }
            CompressionLevel::Recommended => {
                Settings { max_side: 1750, quality: 72, recompress_jpeg: true, convert_lossless: true }
            }
            CompressionLevel::Extreme => {
                Settings { max_side: 1120, quality: 50, recompress_jpeg: true, convert_lossless: true }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressReport {
    pub output: OutputFile,
    pub original_size: u64,
    pub compressed_size: u64,
    pub images_optimized: u32,
    /// Le fichier était déjà optimisé : l'original a été copié tel quel.
    pub already_optimal: bool,
}

pub fn compress(
    engine: &PdfEngine,
    input: PdfInput,
    level: CompressionLevel,
    output: PathBuf,
    job: Job,
) -> Result<CompressReport> {
    ensure_not_input(&output, [input.path.as_path()])?;
    let settings = level.settings();
    const STEPS: u32 = 4;

    job.report(0, STEPS);
    let original = std::fs::read(&input.path)?;
    let original_size = original.len() as u64;
    let lopdf_io::Loaded { mut doc, page_count, .. } = lopdf_io::load(engine, &input, &original)?;
    job.check()?;
    job.report(1, STEPS);

    let images_optimized = optimize_images(&mut doc, settings, &job)?;
    job.check()?;
    job.report(2, STEPS);

    strip_extras(&mut doc);
    doc.compress();
    doc.prune_objects();
    doc.renumber_objects();
    let mut candidate = Vec::with_capacity(original.len());
    doc.save_with_options(
        &mut candidate,
        SaveOptions::builder().use_object_streams(true).use_xref_streams(true).compression_level(9).build(),
    )?;
    drop(doc);
    job.check()?;
    job.report(3, STEPS);

    let (bytes, already_optimal) = if (candidate.len() as u64) < original_size {
        lopdf_io::verify(engine, candidate.clone(), page_count)?;
        (candidate, false)
    } else {
        (original, true)
    };
    job.check()?;

    let out = write_atomic(&output, |f| f.write_all(&bytes).map_err(Into::into))?;
    job.report(STEPS, STEPS);
    Ok(CompressReport { compressed_size: out.size, output: out, original_size, images_optimized, already_optimal })
}

/// Image candidate à la recompression, avec ce qu'il faut savoir d'elle.
struct Candidate {
    id: ObjectId,
    stream: Stream,
    components: Option<u8>,
    allow_resize: bool,
}

fn optimize_images(doc: &mut Document, settings: Settings, job: &Job) -> Result<u32> {
    // Les masques de transparence restent sans perte.
    let mut masks: HashSet<ObjectId> = HashSet::new();
    for obj in doc.objects.values() {
        if let Object::Stream(st) = obj
            && is_image(&st.dict)
        {
            for key in [b"SMask".as_slice(), b"Mask".as_slice()] {
                if let Ok(id) = st.dict.get(key).and_then(Object::as_reference) {
                    masks.insert(id);
                }
            }
        }
    }

    let ids: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter_map(|(id, obj)| match obj {
            Object::Stream(st) if is_image(&st.dict) && !masks.contains(id) => Some(*id),
            _ => None,
        })
        .collect();

    let mut work = Vec::with_capacity(ids.len());
    for id in ids {
        let (components, allow_resize) = match doc.objects.get(&id) {
            Some(Object::Stream(st)) => (color_components(doc, &st.dict), !smask_has_matte(doc, &st.dict)),
            _ => continue,
        };
        if let Some(Object::Stream(stream)) = doc.objects.remove(&id) {
            work.push(Candidate { id, stream, components, allow_resize });
        }
    }

    // Peu de threads : chaque grande image peut occuper des centaines de Mo.
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 4);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|e| HestiaError::Internal(format!("pool de threads : {e}")))?;

    let results: Vec<(ObjectId, Stream, bool)> = pool.install(|| {
        work.into_par_iter()
            .map(|c| {
                if job.cancel.is_cancelled() {
                    return (c.id, c.stream, false);
                }
                match recompress(&c, settings) {
                    Some(new) => (c.id, new, true),
                    None => (c.id, c.stream, false),
                }
            })
            .collect()
    });

    let mut optimized = 0;
    for (id, stream, changed) in results {
        optimized += changed as u32;
        doc.objects.insert(id, Object::Stream(stream));
    }
    job.check()?;
    Ok(optimized)
}

fn is_image(dict: &Dictionary) -> bool {
    matches!(dict.get(b"Subtype").and_then(Object::as_name), Ok(b"Image"))
}

/// Nombre de composantes de l'espace colorimétrique (1 = gris, 3 = RVB),
/// ou `None` pour les espaces qu'on ne touche pas (CMJN, indexé, Lab…).
fn color_components(doc: &Document, dict: &Dictionary) -> Option<u8> {
    let cs = dict.get(b"ColorSpace").ok()?;
    let (_, cs) = doc.dereference(cs).ok()?;
    match cs {
        Object::Name(n) => match n.as_slice() {
            b"DeviceRGB" => Some(3),
            b"DeviceGray" => Some(1),
            _ => None,
        },
        Object::Array(arr) => {
            let family = arr.first()?.as_name().ok()?;
            match family {
                b"CalRGB" => Some(3),
                b"CalGray" => Some(1),
                b"ICCBased" => {
                    let (_, icc) = doc.dereference(arr.get(1)?).ok()?;
                    match icc.as_stream().ok()?.dict.get(b"N").and_then(Object::as_i64) {
                        Ok(3) => Some(3),
                        Ok(1) => Some(1),
                        _ => None,
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Un masque avec `/Matte` doit garder les dimensions de son image.
fn smask_has_matte(doc: &Document, dict: &Dictionary) -> bool {
    dict.get(b"SMask")
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_stream().ok())
        .is_some_and(|s| s.dict.has(b"Matte"))
}

enum Source {
    Jpeg,
    Raw,
}

/// Renvoie le nouveau flux si la recompression vaut la peine.
fn recompress(c: &Candidate, s: Settings) -> Option<Stream> {
    let st = &c.stream;
    let dict = &st.dict;
    if dict.get(b"ImageMask").and_then(Object::as_bool).unwrap_or(false) || dict.has(b"Decode") {
        return None;
    }
    let w = u32::try_from(dict.get(b"Width").and_then(Object::as_i64).ok()?).ok()?;
    let h = u32::try_from(dict.get(b"Height").and_then(Object::as_i64).ok()?).ok()?;
    if w < 32 || h < 32 || (w as u64) * (h as u64) > MAX_IMAGE_PIXELS || st.content.len() < 8 * 1024 {
        return None;
    }
    let comps = c.components?;
    let filters: Vec<&[u8]> = st.filters().unwrap_or_default();
    let source = match filters.as_slice() {
        [b"DCTDecode"] => Source::Jpeg,
        [] | [b"FlateDecode"] | [b"LZWDecode"] | [b"RunLengthDecode"] => Source::Raw,
        _ => return None,
    };
    let bpc = dict.get(b"BitsPerComponent").and_then(Object::as_i64).unwrap_or(8);
    let needs_resize = c.allow_resize && w.max(h) > s.max_side;

    let image = match source {
        Source::Jpeg => {
            if !needs_resize && !s.recompress_jpeg {
                return None;
            }
            let header = parse_jpeg_header(&st.content)?;
            if header.components != comps || header.precision != 8 || (header.width, header.height) != (w, h) {
                return None;
            }
            decode_bytes(&st.content).ok()?.image
        }
        Source::Raw => {
            if !s.convert_lossless || bpc != 8 {
                return None;
            }
            let expected = w as usize * h as usize * comps as usize;
            let data = st.decompressed_content_with_limit(expected.saturating_add(64 * 1024)).ok()?;
            if data.len() < expected {
                return None;
            }
            let data = data[..expected].to_vec();
            let img = if comps == 1 {
                DynamicImage::ImageLuma8(GrayImage::from_raw(w, h, data)?)
            } else {
                DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, data)?)
            };
            if looks_like_graphics(&img) {
                return None;
            }
            img
        }
    };
    if (image.width(), image.height()) != (w, h) {
        return None;
    }

    let image = if needs_resize {
        let scale = s.max_side as f64 / w.max(h) as f64;
        let nw = ((w as f64 * scale).round() as u32).max(1);
        let nh = ((h as f64 * scale).round() as u32).max(1);
        image.resize_exact(nw, nh, FilterType::CatmullRom)
    } else {
        image
    };
    let image =
        if comps == 1 { DynamicImage::ImageLuma8(image.to_luma8()) } else { DynamicImage::ImageRgb8(image.to_rgb8()) };
    let encoded = encode_jpeg(&image, s.quality).ok()?;

    // On ne garde le résultat que s'il est nettement plus léger.
    let threshold = if needs_resize { st.content.len() } else { st.content.len() * 9 / 10 };
    if encoded.len() >= threshold {
        return None;
    }

    let mut new_dict = dict.clone();
    new_dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
    new_dict.remove(b"DecodeParms");
    new_dict.set("Width", image.width() as i64);
    new_dict.set("Height", image.height() as i64);
    new_dict.set("BitsPerComponent", 8i64);
    let mut stream = Stream::new(new_dict, encoded);
    stream.allows_compression = false;
    Some(stream)
}

/// Une image avec peu de couleurs distinctes (schéma, capture, logo)
/// se dégraderait en JPEG : on la laisse sans perte.
fn looks_like_graphics(img: &DynamicImage) -> bool {
    const MAX_SAMPLES: u64 = 40_000;
    const MAX_COLORS: usize = 256;
    let (w, h) = (img.width() as u64, img.height() as u64);
    let step = ((w * h) as f64 / MAX_SAMPLES as f64).sqrt().max(1.0) as u64;
    let mut colors: HashSet<[u8; 3]> = HashSet::new();
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let p = match img {
                DynamicImage::ImageLuma8(g) => {
                    let v = g.get_pixel(x as u32, y as u32)[0];
                    [v, v, v]
                }
                DynamicImage::ImageRgb8(rgb) => rgb.get_pixel(x as u32, y as u32).0,
                other => {
                    let px = other.to_rgb8();
                    return looks_like_graphics(&DynamicImage::ImageRgb8(px));
                }
            };
            colors.insert(p);
            if colors.len() > MAX_COLORS {
                return false;
            }
            x += step;
        }
        y += step;
    }
    true
}

/// Retire les données sans effet sur l'affichage.
fn strip_extras(doc: &mut Document) {
    for obj in doc.objects.values_mut() {
        let dict = match obj {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &mut s.dict,
            _ => continue,
        };
        // Données privées d'applications (Illustrator, Acrobat…).
        dict.remove(b"PieceInfo");
        // Miniatures de pages intégrées (les lecteurs les recalculent).
        if dict.has_type(b"Page") {
            dict.remove(b"Thumb");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphics_detection() {
        let flat = DynamicImage::ImageRgb8(RgbImage::from_pixel(200, 200, image::Rgb([255, 255, 255])));
        assert!(looks_like_graphics(&flat));
        let mut noisy = RgbImage::new(200, 200);
        for (x, y, p) in noisy.enumerate_pixels_mut() {
            *p = image::Rgb([(x * 7 % 256) as u8, (y * 13 % 256) as u8, ((x * y) % 256) as u8]);
        }
        assert!(!looks_like_graphics(&DynamicImage::ImageRgb8(noisy)));
    }
}
