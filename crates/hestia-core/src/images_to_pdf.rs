//! Images (JPG, PNG, WebP, BMP, TIFF, GIF) en PDF.
//!
//! - Les JPEG sont intégrés **tels quels** (aucune perte, aucune
//!   recompression) ; les autres formats sont compressés sans perte.
//! - L'orientation EXIF des photos (téléphones) est respectée, ainsi que la
//!   rotation choisie par l'utilisateur, sans retoucher les pixels.
//! - La transparence (PNG…) est conservée via un masque.

use std::io::Write;
use std::path::{Path, PathBuf};

use flate2::{Compression, write::ZlibEncoder};
use image::{ColorType, DynamicImage, metadata::Orientation};
use lopdf::{Document, Object, SaveOptions, Stream, dictionary};
use serde::Deserialize;

use crate::error::{HestiaError, Result};
use crate::imaging::{decode_bytes, encode_jpeg, parse_jpeg_header, read_orientation};
use crate::job::Job;
use crate::output::{OutputFile, ensure_dir, ensure_not_input, file_stem, write_atomic, write_new_in_dir};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInput {
    pub path: PathBuf,
    /// Rotation supplémentaire dans le sens horaire : 0, 90, 180 ou 270.
    #[serde(default)]
    pub rotation: u16,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PageSizeOption {
    #[default]
    A4,
    Letter,
    /// La page prend la taille de l'image.
    Fit,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PageOrientation {
    /// Paysage pour les images plus larges que hautes.
    #[default]
    Auto,
    Portrait,
    Landscape,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MarginOption {
    #[default]
    None,
    Small,
    Large,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ImagesToPdfOptions {
    pub page_size: PageSizeOption,
    pub orientation: PageOrientation,
    pub margin: MarginOption,
}

/// Toutes les images dans un seul PDF, dans l'ordre donné.
pub fn images_to_pdf(
    inputs: Vec<ImageInput>,
    options: ImagesToPdfOptions,
    output: PathBuf,
    job: Job,
) -> Result<OutputFile> {
    if inputs.is_empty() {
        return Err(HestiaError::NoInput);
    }
    ensure_not_input(&output, inputs.iter().map(|i| i.path.as_path()))?;
    let total = inputs.len() as u32;
    let mut builder = PdfBuilder::new();
    for (i, input) in inputs.iter().enumerate() {
        job.check()?;
        builder.add_image(input, &options).map_err(|e| e.in_file(&input.path))?;
        job.report(i as u32 + 1, total);
    }
    job.check()?;
    let bytes = builder.finish()?;
    write_atomic(&output, |f| f.write_all(&bytes).map_err(Into::into))
}

/// Un PDF par image, dans le dossier `dir` (nommé d'après l'image).
pub fn images_to_pdf_each(
    inputs: Vec<ImageInput>,
    options: ImagesToPdfOptions,
    dir: PathBuf,
    job: Job,
) -> Result<Vec<OutputFile>> {
    if inputs.is_empty() {
        return Err(HestiaError::NoInput);
    }
    let dir = ensure_dir(&dir)?;
    let total = inputs.len() as u32;
    let mut outputs = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        job.check()?;
        let mut builder = PdfBuilder::new();
        builder.add_image(input, &options).map_err(|e| e.in_file(&input.path))?;
        let bytes = builder.finish()?;
        outputs
            .push(write_new_in_dir(&dir, &file_stem(&input.path), "pdf", |f| f.write_all(&bytes).map_err(Into::into))?);
        job.report(i as u32 + 1, total);
    }
    Ok(outputs)
}

/// Assemble un PDF page par page.
struct PdfBuilder {
    doc: Document,
    pages_id: lopdf::ObjectId,
    kids: Vec<Object>,
}

impl PdfBuilder {
    fn new() -> Self {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        Self { doc, pages_id, kids: Vec::new() }
    }

    fn add_image(&mut self, input: &ImageInput, options: &ImagesToPdfOptions) -> Result<()> {
        if !input.rotation.is_multiple_of(90) || input.rotation >= 360 {
            return Err(HestiaError::Internal(format!("rotation invalide : {}", input.rotation)));
        }
        let image = load_image(&input.path, false)?;
        let ops = orientation_ops(image.orientation, input.rotation);
        let layout = layout(image.width, image.height, &ops, options);

        let mut stream = image.stream;
        if let Some(mask) = image.smask {
            let mask_id = self.doc.add_object(mask);
            stream.dict.set("SMask", mask_id);
        }
        let image_id = self.doc.add_object(stream);

        let m = layout.matrix;
        let content =
            format!("q {} {} {} {} {} {} cm /Im0 Do Q", num(m.a), num(m.b), num(m.c), num(m.d), num(m.e), num(m.f));
        let content_id = self.doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page_id = self.doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => self.pages_id,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(layout.page_w), Object::Real(layout.page_h)],
            "Contents" => content_id,
            "Resources" => dictionary! {
                "XObject" => dictionary! { "Im0" => image_id },
            },
        });
        self.kids.push(page_id.into());
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<u8>> {
        let count = self.kids.len() as i64;
        self.doc.objects.insert(
            self.pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => self.kids,
                "Count" => count,
            }),
        );
        let catalog_id = self.doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => self.pages_id,
        });
        let info_id = self.doc.add_object(dictionary! {
            "Producer" => Object::string_literal("Hestia PDF"),
        });
        self.doc.trailer.set("Root", catalog_id);
        self.doc.trailer.set("Info", info_id);
        self.doc.compress();
        let mut out = Vec::new();
        self.doc.save_with_options(
            &mut out,
            SaveOptions::builder().use_object_streams(true).use_xref_streams(true).compression_level(6).build(),
        )?;
        Ok(out)
    }
}

/// Image prête à intégrer.
pub(crate) struct EmbeddedImage {
    pub stream: Stream,
    pub smask: Option<Stream>,
    pub width: u32,
    pub height: u32,
    pub orientation: Orientation,
}

/// `grayscale` : l'image est convertie en niveaux de gris (« noir et blanc »).
pub(crate) fn load_image(path: &Path, grayscale: bool) -> Result<EmbeddedImage> {
    let bytes = std::fs::read(path)?;
    let jpeg = parse_jpeg_header(&bytes);

    // JPEG : copie directe des données (sans perte).
    if !grayscale
        && let Some(h) = jpeg
        && h.precision == 8
        && matches!(h.components, 1 | 3 | 4)
    {
        let color_space = match h.components {
            1 => "DeviceGray",
            3 => "DeviceRGB",
            _ => "DeviceCMYK",
        };
        let mut dict = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => h.width as i64,
            "Height" => h.height as i64,
            "ColorSpace" => color_space,
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        };
        if h.components == 4 && h.adobe {
            // Les JPEG CMJN d'Adobe stockent des valeurs inversées.
            dict.set("Decode", [1, 0, 1, 0, 1, 0, 1, 0].iter().map(|&v| Object::Integer(v)).collect::<Vec<_>>());
        }
        let orientation = read_orientation(&bytes);
        let mut stream = Stream::new(dict, bytes);
        stream.allows_compression = false;
        return Ok(EmbeddedImage { stream, smask: None, width: h.width, height: h.height, orientation });
    }

    // Autres formats : décodage puis compression sans perte.
    let decoded = decode_bytes(&bytes)?;
    let img = decoded.image;
    let (width, height) = (img.width(), img.height());

    // Photo passée en noir et blanc : reste un JPEG (taille raisonnable), en gris.
    if grayscale && jpeg.is_some() {
        let data = encode_jpeg(&DynamicImage::ImageLuma8(img.to_luma8()), 92)?;
        let mut stream = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => width as i64,
                "Height" => height as i64,
                "ColorSpace" => "DeviceGray",
                "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            },
            data,
        );
        stream.allows_compression = false;
        return Ok(EmbeddedImage { stream, smask: None, width, height, orientation: decoded.orientation });
    }

    let gray = grayscale || matches!(img.color(), ColorType::L8 | ColorType::La8 | ColorType::L16 | ColorType::La16);
    let (pixels, color_space) =
        if gray { (img.to_luma8().into_raw(), "DeviceGray") } else { (img.to_rgb8().into_raw(), "DeviceRGB") };
    let smask = alpha_mask(&img)?;
    let stream = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => width as i64,
            "Height" => height as i64,
            "ColorSpace" => color_space,
            "BitsPerComponent" => 8,
            "Filter" => "FlateDecode",
        },
        zlib(&pixels)?,
    );
    let mut stream = stream;
    stream.allows_compression = false;
    Ok(EmbeddedImage { stream, smask, width, height, orientation: decoded.orientation })
}

/// Masque de transparence, si l'image en a une réellement.
fn alpha_mask(img: &DynamicImage) -> Result<Option<Stream>> {
    if !img.color().has_alpha() {
        return Ok(None);
    }
    let alpha: Vec<u8> = img.to_rgba8().pixels().map(|p| p[3]).collect();
    if alpha.iter().all(|&a| a == 255) {
        return Ok(None);
    }
    let mut stream = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => img.width() as i64,
            "Height" => img.height() as i64,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
            "Filter" => "FlateDecode",
        },
        zlib(&alpha)?,
    );
    stream.allows_compression = false;
    Ok(Some(stream))
}

fn zlib(data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::with_capacity(data.len() / 2), Compression::new(6));
    enc.write_all(data)?;
    Ok(enc.finish()?)
}

/// Transformation élémentaire appliquée à l'image affichée.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    /// Rotation de 90° dans le sens horaire.
    Rotate90,
    /// Miroir horizontal.
    FlipH,
}

/// Suite d'opérations : orientation EXIF puis rotation de l'utilisateur.
pub(crate) fn orientation_ops(exif: Orientation, user_rotation: u16) -> Vec<Op> {
    use Op::{FlipH as F, Rotate90 as R};
    let mut ops = match exif {
        Orientation::NoTransforms => vec![],
        Orientation::Rotate90 => vec![R],
        Orientation::Rotate180 => vec![R, R],
        Orientation::Rotate270 => vec![R, R, R],
        Orientation::FlipHorizontal => vec![F],
        Orientation::FlipVertical => vec![R, R, F],
        Orientation::Rotate90FlipH => vec![R, F],
        Orientation::Rotate270FlipH => vec![R, R, R, F],
    };
    ops.extend(std::iter::repeat_n(R, (user_rotation / 90 % 4) as usize));
    ops
}

/// Matrice affine PDF `[a b c d e f]` : x' = a·x + c·y + e ; y' = b·x + d·y + f.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub(crate) fn scale(sx: f32, sy: f32) -> Self {
        Self { a: sx, b: 0.0, c: 0.0, d: sy, e: 0.0, f: 0.0 }
    }

    pub(crate) fn translate(tx: f32, ty: f32) -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: tx, f: ty }
    }

    /// `self` puis `next`.
    pub(crate) fn then(self, n: Affine) -> Affine {
        Affine {
            a: self.a * n.a + self.b * n.c,
            b: self.a * n.b + self.b * n.d,
            c: self.c * n.a + self.d * n.c,
            d: self.c * n.b + self.d * n.d,
            e: self.e * n.a + self.f * n.c + n.e,
            f: self.e * n.b + self.f * n.d + n.f,
        }
    }

    #[cfg(test)]
    pub(crate) fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }
}

struct Layout {
    page_w: f32,
    page_h: f32,
    matrix: Affine,
}

const A4: (f32, f32) = (595.28, 841.89);
const LETTER: (f32, f32) = (612.0, 792.0);
/// Taille « Ajuster » : 1 pixel = 0,75 pt (96 dpi).
const FIT_SCALE: f32 = 0.75;

fn layout(img_w: u32, img_h: u32, ops: &[Op], options: &ImagesToPdfOptions) -> Layout {
    let swapped = ops.iter().filter(|o| **o == Op::Rotate90).count() % 2 == 1;
    let (dw_px, dh_px) = if swapped { (img_h as f32, img_w as f32) } else { (img_w as f32, img_h as f32) };
    let margin = match options.margin {
        MarginOption::None => 0.0,
        MarginOption::Small => 20.0,
        MarginOption::Large => 50.0,
    };

    let (page_w, page_h, scale) = match options.page_size {
        PageSizeOption::Fit => (dw_px * FIT_SCALE + 2.0 * margin, dh_px * FIT_SCALE + 2.0 * margin, FIT_SCALE),
        size => {
            let (short, long) = if size == PageSizeOption::Letter { LETTER } else { A4 };
            let landscape = match options.orientation {
                PageOrientation::Auto => dw_px > dh_px,
                PageOrientation::Portrait => false,
                PageOrientation::Landscape => true,
            };
            let (pw, ph) = if landscape { (long, short) } else { (short, long) };
            let (aw, ah) = ((pw - 2.0 * margin).max(1.0), (ph - 2.0 * margin).max(1.0));
            (pw, ph, (aw / dw_px).min(ah / dh_px))
        }
    };

    // Dimensions dessinées avant rotation, puis opérations, puis centrage.
    let (m, bw, bh) = oriented(img_w as f32 * scale, img_h as f32 * scale, ops);
    let matrix = m.then(Affine::translate((page_w - bw) / 2.0, (page_h - bh) / 2.0));
    Layout { page_w, page_h, matrix }
}

/// Image dessinée en `sw × sh` puis transformée par `ops` : renvoie la
/// matrice (depuis le carré unité de l'image) et l'encombrement final.
fn oriented(sw: f32, sh: f32, ops: &[Op]) -> (Affine, f32, f32) {
    let mut m = Affine::scale(sw, sh);
    let (mut bw, mut bh) = (sw, sh);
    for op in ops {
        let step = match op {
            Op::Rotate90 => Affine { a: 0.0, b: -1.0, c: 1.0, d: 0.0, e: 0.0, f: bw },
            Op::FlipH => Affine { a: -1.0, b: 0.0, c: 0.0, d: 1.0, e: bw, f: 0.0 },
        };
        m = m.then(step);
        if *op == Op::Rotate90 {
            std::mem::swap(&mut bw, &mut bh);
        }
    }
    (m, bw, bh)
}

/// Matrice qui place l'image (orientée par `ops`) dans le rectangle
/// `(x, y, w, h)` d'un repère y vers le haut, en le remplissant.
pub(crate) fn place_in_box(ops: &[Op], x: f32, y: f32, w: f32, h: f32) -> Affine {
    let swapped = ops.iter().filter(|o| **o == Op::Rotate90).count() % 2 == 1;
    let (sw, sh) = if swapped { (h, w) } else { (w, h) };
    let (m, _, _) = oriented(sw, sh, ops);
    m.then(Affine::translate(x, y))
}

/// Nombre compact pour un flux de contenu PDF.
pub(crate) fn num(v: f32) -> String {
    let v = if v.abs() < 0.0005 { 0.0 } else { v };
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_string() } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }

    #[test]
    fn number_formatting() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(595.28), "595.28");
        assert_eq!(num(0.1234), "0.123");
    }

    #[test]
    fn fit_page_without_rotation() {
        let opts = ImagesToPdfOptions { page_size: PageSizeOption::Fit, ..Default::default() };
        let l = layout(400, 200, &[], &opts);
        assert!(close((l.page_w, l.page_h), (300.0, 150.0)));
        // Coin haut-gauche de l'image (0,1) → haut-gauche de la page.
        assert!(close(l.matrix.apply(0.0, 1.0), (0.0, 150.0)));
        assert!(close(l.matrix.apply(1.0, 0.0), (300.0, 0.0)));
    }

    #[test]
    fn rotation_90_turns_top_left_to_top_right() {
        let opts = ImagesToPdfOptions { page_size: PageSizeOption::Fit, ..Default::default() };
        let ops = orientation_ops(Orientation::NoTransforms, 90);
        let l = layout(400, 200, &ops, &opts);
        // L'image 400×200 tournée devient 150×300 pt.
        assert!(close((l.page_w, l.page_h), (150.0, 300.0)));
        assert!(close(l.matrix.apply(0.0, 1.0), (150.0, 300.0)), "{:?}", l.matrix.apply(0.0, 1.0));
        assert!(close(l.matrix.apply(0.0, 0.0), (0.0, 300.0)));
    }

    #[test]
    fn exif_rotation_is_applied() {
        assert_eq!(orientation_ops(Orientation::Rotate90, 0), vec![Op::Rotate90]);
        assert_eq!(orientation_ops(Orientation::Rotate270, 90).len(), 4);
        let ops = orientation_ops(Orientation::FlipHorizontal, 0);
        let opts = ImagesToPdfOptions { page_size: PageSizeOption::Fit, ..Default::default() };
        let l = layout(100, 100, &ops, &opts);
        // Miroir : le coin haut-gauche de l'image passe en haut à droite.
        assert!(close(l.matrix.apply(0.0, 1.0), (75.0, 75.0)));
    }

    #[test]
    fn a4_auto_orientation_and_margins() {
        let opts = ImagesToPdfOptions { margin: MarginOption::Large, ..Default::default() };
        let l = layout(3000, 2000, &[], &opts);
        assert!(l.page_w > l.page_h, "image paysage → page paysage");
        let (x0, y0) = l.matrix.apply(0.0, 0.0);
        let (x1, y1) = l.matrix.apply(1.0, 1.0);
        assert!(x0 >= 49.9 && y0 >= 49.9 && x1 <= l.page_w - 49.9 && y1 <= l.page_h - 49.9);
    }
}
