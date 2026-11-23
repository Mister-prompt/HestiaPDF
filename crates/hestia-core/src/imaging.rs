//! Outils d'image partagés : encodage JPEG, décodage borné, lecture
//! d'en-têtes JPEG.

use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ImageDecoder, ImageReader, Limits, metadata::Orientation};

use crate::error::{HestiaError, Result};

/// Taille maximale acceptée pour une image décodée (côté, en pixels).
pub const MAX_IMAGE_SIDE: u32 = 30_000;
/// Mémoire maximale allouée pour décoder une image.
pub const MAX_IMAGE_ALLOC: u64 = 1024 * 1024 * 1024;

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_IMAGE_ALLOC);
    limits
}

/// Encode une image en JPEG.
pub fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    use jpeg_encoder::{ColorType, Encoder};

    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 {
        return Err(HestiaError::UnsupportedImage(format!("dimensions {w}×{h} impossibles en JPEG")));
    }
    let mut out = Vec::new();
    let mut enc = Encoder::new(&mut out, quality.clamp(1, 100));
    // Ne PAS activer `set_optimized_huffman_tables` : avec jpeg-encoder 0.7.1
    // il produit des fichiers corrompus (voir le test `jpeg_roundtrip_is_faithful`).
    enc.set_optimized_huffman_tables(false);
    let res = match img {
        DynamicImage::ImageLuma8(g) => enc.encode(g.as_raw(), w as u16, h as u16, ColorType::Luma),
        DynamicImage::ImageRgb8(rgb) => enc.encode(rgb.as_raw(), w as u16, h as u16, ColorType::Rgb),
        other => {
            // JPEG n'a pas de transparence : on l'aplatit sur du blanc.
            let rgb = if other.color().has_alpha() { flatten_on_white(other) } else { other.to_rgb8() };
            enc.encode(rgb.as_raw(), w as u16, h as u16, ColorType::Rgb)
        }
    };
    res.map_err(|e| HestiaError::Internal(format!("encodage JPEG : {e}")))?;
    Ok(out)
}

/// Aplatit la transparence sur un fond blanc.
pub fn flatten_on_white(img: &DynamicImage) -> image::RgbImage {
    let rgba = img.to_rgba8();
    let mut out = image::RgbImage::new(rgba.width(), rgba.height());
    for (src, dst) in rgba.pixels().zip(out.pixels_mut()) {
        let a = src[3] as u32;
        for c in 0..3 {
            dst[c] = ((src[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        }
    }
    out
}

/// Image décodée avec son orientation EXIF.
pub struct DecodedImage {
    pub image: DynamicImage,
    pub orientation: Orientation,
}

/// Décode un fichier image en respectant des limites de taille et de mémoire.
pub fn decode_file(path: &Path) -> Result<DecodedImage> {
    let bytes = std::fs::read(path)?;
    decode_bytes(&bytes)
}

pub fn decode_bytes(bytes: &[u8]) -> Result<DecodedImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(limits());
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let image = DynamicImage::from_decoder(decoder)?;
    Ok(DecodedImage { image, orientation })
}

/// Miniature JPEG d'un fichier image (orientation EXIF appliquée).
pub fn thumbnail_jpeg(path: &Path, max_side: u32) -> Result<Vec<u8>> {
    let decoded = decode_file(path)?;
    let side = max_side.clamp(16, 1024);
    let mut thumb = decoded.image.thumbnail(side, side);
    thumb.apply_orientation(decoded.orientation);
    encode_jpeg(&thumb, 82)
}

/// Orientation EXIF sans décoder les pixels.
pub fn read_orientation(bytes: &[u8]) -> Orientation {
    ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_decoder().ok())
        .and_then(|mut d| d.orientation().ok())
        .unwrap_or(Orientation::NoTransforms)
}

/// En-tête d'un fichier JPEG (lu sans décoder l'image).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JpegHeader {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub precision: u8,
    /// Présence d'un segment APP14 « Adobe » (CMYK inversé).
    pub adobe: bool,
}

/// Lit l'en-tête SOF d'un JPEG. Renvoie `None` si les données ne sont pas
/// un JPEG valide.
pub fn parse_jpeg_header(data: &[u8]) -> Option<JpegHeader> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut adobe = false;
    let mut i = 2usize;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            return None;
        }
        let marker = data[i + 1];
        // Octets de remplissage.
        if marker == 0xFF {
            i += 1;
            continue;
        }
        // Marqueurs sans longueur.
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        if marker == 0xD9 || marker == 0xDA {
            return None; // fin d'image ou début des données sans SOF
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if len < 2 || i + 2 + len > data.len() {
            return None;
        }
        let seg = &data[i + 4..i + 2 + len];
        if marker == 0xEE && seg.starts_with(b"Adobe") {
            adobe = true;
        }
        let is_sof = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof {
            if seg.len() < 6 {
                return None;
            }
            let precision = seg[0];
            let height = u16::from_be_bytes([seg[1], seg[2]]) as u32;
            let width = u16::from_be_bytes([seg[3], seg[4]]) as u32;
            let components = seg[5];
            if width == 0 || height == 0 || components == 0 {
                return None;
            }
            // Un APP14 peut suivre le SOF : on le cherche encore un peu.
            if !adobe {
                adobe = find_adobe_marker(&data[i + 2 + len..]);
            }
            return Some(JpegHeader { width, height, components, precision, adobe });
        }
        i += 2 + len;
    }
    None
}

fn find_adobe_marker(rest: &[u8]) -> bool {
    let mut i = 0usize;
    while i + 4 <= rest.len() {
        if rest[i] != 0xFF {
            return false;
        }
        let marker = rest[i + 1];
        if marker == 0xDA || marker == 0xD9 {
            return false;
        }
        let len = u16::from_be_bytes([rest[i + 2], rest[i + 3]]) as usize;
        if len < 2 || i + 2 + len > rest.len() {
            return false;
        }
        if marker == 0xEE && rest[i + 4..i + 2 + len].starts_with(b"Adobe") {
            return true;
        }
        i += 2 + len;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_header_roundtrip() -> Result<()> {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(37, 21, image::Rgb([200, 10, 10])));
        let jpg = encode_jpeg(&img, 80)?;
        let h = parse_jpeg_header(&jpg).ok_or_else(|| HestiaError::Internal("en-tête".into()))?;
        assert_eq!((h.width, h.height, h.components, h.precision), (37, 21, 3, 8));
        assert!(!h.adobe);
        Ok(())
    }

    /// Garde-fou contre un encodeur défaillant : des aplats de couleur
    /// doivent ressortir quasi identiques, quelle que soit la taille.
    #[test]
    fn jpeg_roundtrip_is_faithful() -> Result<()> {
        for (w, h) in [(75u32, 150u32), (300, 600), (64, 64), (33, 17), (1, 1)] {
            let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([255, 255, 255]));
            for y in h / 2..h {
                for x in 0..w {
                    img.put_pixel(x, y, image::Rgb([200, 20, 20]));
                }
            }
            for q in [50u8, 82, 92] {
                let jpg = encode_jpeg(&DynamicImage::ImageRgb8(img.clone()), q)?;
                let dec = image::load_from_memory(&jpg)?.to_rgb8();
                assert_eq!((dec.width(), dec.height()), (w, h));
                // Loin de la frontière rouge/blanc, l'écart doit rester faible.
                for (x, y, p) in dec.enumerate_pixels() {
                    let edge = (y as i64 - (h / 2) as i64).abs() <= 8;
                    if edge {
                        continue;
                    }
                    let o = img.get_pixel(x, y);
                    let d = (0..3).map(|c| (p[c] as i32 - o[c] as i32).abs()).max().unwrap_or(0);
                    assert!(d <= 24, "{w}×{h} q{q} : écart {d} en ({x},{y})");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn garbage_is_not_a_jpeg() {
        assert!(parse_jpeg_header(b"").is_none());
        assert!(parse_jpeg_header(b"\xFF\xD8\xFF\xE0\x00").is_none());
        assert!(parse_jpeg_header(b"%PDF-1.7").is_none());
        let mut truncated = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11];
        truncated.extend_from_slice(&[8, 0, 10]);
        assert!(parse_jpeg_header(&truncated).is_none());
    }

    #[test]
    fn transparency_is_flattened_on_white() {
        let img = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 0])));
        assert_eq!(flatten_on_white(&img).get_pixel(0, 0).0, [255, 255, 255]);
    }
}
