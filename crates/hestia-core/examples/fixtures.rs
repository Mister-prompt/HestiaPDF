//! Génère des fichiers d'essai pour tester Hestia PDF à la main :
//!
//!   cargo run -p hestia-core --example fixtures -- <dossier>
//!
//! Produit : rapport.pdf (12 pages illustrées), photos.pdf (lourd, à
//! compresser), protege.pdf (mot de passe « hestia »), et quelques images.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use hestia_core::Job;
use hestia_core::images_to_pdf::{ImageInput, ImagesToPdfOptions, images_to_pdf};
use image::{ImageEncoder, RgbImage, RgbaImage};
use lopdf::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
use lopdf::{Document, EncryptionState, EncryptionVersion, Object, Permissions, Stream, dictionary};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args().nth(1).ok_or("usage : fixtures <dossier>")?;
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir)?;

    illustrated_pdf(&dir.join("rapport.pdf"), 12, "Rapport annuel")?;
    illustrated_pdf(&dir.join("annexe.pdf"), 3, "Annexe")?;

    let mut doc = Document::load(dir.join("annexe.pdf"))?;
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
    let state = EncryptionState::try_from(EncryptionVersion::V4 {
        document: &doc,
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: "proprietaire",
        user_password: "hestia",
        permissions: Permissions::all(),
    })?;
    doc.encrypt(&state)?;
    doc.save(dir.join("protege.pdf"))?;

    let mut inputs = Vec::new();
    for (i, (w, h)) in [(3000u32, 2000u32), (2000, 3000), (2800, 2100)].into_iter().enumerate() {
        let path = dir.join(format!("photo-{}.jpg", i + 1));
        let file = std::fs::File::create(&path)?;
        image::codecs::jpeg::JpegEncoder::new_with_quality(file, 96).write_image(
            photo(w, h, i as u32).as_raw(),
            w,
            h,
            image::ExtendedColorType::Rgb8,
        )?;
        inputs.push(ImageInput { path, rotation: 0 });
    }
    RgbaImage::from_fn(600, 600, |x, y| {
        let d = ((x as f32 - 300.0).powi(2) + (y as f32 - 300.0).powi(2)).sqrt();
        image::Rgba([185, 28, 28, if d < 280.0 { 255 } else { 0 }])
    })
    .save(dir.join("logo.png"))?;
    images_to_pdf(inputs, ImagesToPdfOptions::default(), dir.join("photos.pdf"), Job::silent())?;

    println!("Fichiers d'essai créés dans {}", dir.display());
    Ok(())
}

fn photo(w: u32, h: u32, seed: u32) -> RgbImage {
    let mut s = 0x9e37_79b9_u32 ^ seed.wrapping_mul(7919);
    RgbImage::from_fn(w, h, |x, y| {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let n = (s % 20) as u8;
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        let sky = fy < 0.55;
        if sky {
            image::Rgb([(90.0 + 80.0 * fy) as u8 + n, (150.0 + 60.0 * fy) as u8 + n / 2, 230u8.saturating_sub(n)])
        } else {
            let hill = ((fx * 9.0 + seed as f32).sin() * 0.05 + 0.6 - fy).abs() < 0.01;
            if hill {
                image::Rgb([40, 60, 40])
            } else {
                image::Rgb([(60.0 + 90.0 * fx) as u8 + n, (120.0 - 40.0 * fy) as u8 + n, 50 + n])
            }
        }
    })
}

fn illustrated_pdf(path: &Path, pages: u32, title: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold" });
    let body_font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let colors = [(0.73, 0.11, 0.11), (0.15, 0.39, 0.92), (0.02, 0.59, 0.41), (0.85, 0.47, 0.02)];
    let mut kids = Vec::new();
    for i in 1..=pages {
        let (r, g, b) = colors[(i as usize - 1) % colors.len()];
        let mut c = format!(
            "{r} {g} {b} rg 0 762 595 80 re f 1 1 1 rg BT /F1 28 Tf 50 790 Td ({title}) Tj ET \
             0.2 0.2 0.2 rg BT /F1 64 Tf 50 640 Td (Page {i}) Tj ET \
             {r} {g} {b} rg 50 300 495 260 re f 1 1 1 rg BT /F1 22 Tf 70 420 Td (Section {i}) Tj ET\n"
        );
        for line in 0..8 {
            c.push_str(&format!(
                "0.35 0.35 0.35 rg BT /F2 12 Tf 50 {} Td (Ligne de texte d'exemple numero {} pour la page {i}.) Tj ET\n",
                260 - line * 20,
                line + 1
            ));
        }
        let content = doc.add_object(Stream::new(dictionary! {}, c.into_bytes()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Contents" => content,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font, "F2" => body_font } },
        });
        kids.push(Object::from(page));
    }
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages as i64 }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc.trailer.set(
        "ID",
        vec![
            Object::string_literal(b"hestia-fixture-01".to_vec()),
            Object::string_literal(b"hestia-fixture-01".to_vec()),
        ],
    );
    doc.compress();
    doc.save(path)?;
    Ok(())
}
