//! Tests d'intégration avec la vraie bibliothèque PDFium
//! (`src-tauri/resources/pdfium/pdfium.dll`, voir scripts/fetch-pdfium.ps1).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hestia_core::compress::{CompressionLevel, compress};
use hestia_core::images_to_pdf::{ImageInput, ImagesToPdfOptions, PageSizeOption, images_to_pdf, images_to_pdf_each};
use hestia_core::inspect::{inspect, render_page_jpeg};
use hestia_core::merge::merge;
use hestia_core::pdf_to_images::{ExportQuality, ImageExport, pdf_to_images};
use hestia_core::split::{SplitMode, SplitTarget, plan, split};
use hestia_core::{CancelToken, HestiaError, Job, PdfEngine, PdfInput};
use image::{DynamicImage, ImageEncoder, RgbImage, RgbaImage};
use lopdf::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
use lopdf::{Document, EncryptionState, EncryptionVersion, Object, Permissions, Stream, dictionary};

fn engine() -> &'static PdfEngine {
    let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/pdfium/pdfium.dll");
    assert!(dll.exists(), "pdfium.dll absent : lancez scripts/fetch-pdfium.ps1");
    PdfEngine::init(&dll).expect("démarrage du moteur PDFium")
}

/// PDF texte de `pages` pages (« Page 1 », « Page 2 »…).
fn text_pdf(path: &Path, pages: u32) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let mut kids = Vec::new();
    for i in 1..=pages {
        let content = format!("BT /F1 36 Tf 72 720 Td (Page {i}) Tj ET");
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Contents" => content_id,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
        });
        kids.push(Object::from(page_id));
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
            Object::string_literal(b"0123456789abcdef".to_vec()),
            Object::string_literal(b"0123456789abcdef".to_vec()),
        ],
    );
    doc.save(path).unwrap();
}

/// Image « photo » : dégradés + bruit (beaucoup de couleurs).
fn photo(w: u32, h: u32) -> RgbImage {
    let mut seed = 0x2545_f491_u32;
    RgbImage::from_fn(w, h, |x, y| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let n = (seed % 24) as u8;
        image::Rgb([
            ((x * 255 / w) as u8).saturating_add(n),
            ((y * 255 / h) as u8).saturating_add(n / 2),
            (((x + y) * 128 / (w + h)) as u8).saturating_add(n),
        ])
    })
}

fn write_jpeg(path: &Path, img: &RgbImage, quality: u8) {
    let file = std::fs::File::create(path).unwrap();
    image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality)
        .write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgb8)
        .unwrap();
}

fn page_count(path: &Path) -> u32 {
    inspect(engine(), PdfInput::new(path)).unwrap().page_count
}

fn job() -> Job {
    Job::silent()
}

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn inspect_and_render() {
    let dir = tmp();
    let pdf = dir.path().join("trois pages.pdf");
    text_pdf(&pdf, 3);
    let info = inspect(engine(), PdfInput::new(&pdf)).unwrap();
    assert_eq!(info.page_count, 3);
    assert_eq!((info.pages[0].width, info.pages[0].height), (595.0, 842.0));

    let jpeg = render_page_jpeg(engine(), PdfInput::new(&pdf), 1, 300).unwrap();
    let img = image::load_from_memory(&jpeg).unwrap();
    assert_eq!(img.width(), 300);
    assert!(render_page_jpeg(engine(), PdfInput::new(&pdf), 3, 300).is_err(), "page hors limites");
}

#[test]
fn merge_keeps_order_and_pages() {
    let dir = tmp();
    let (a, b) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));
    text_pdf(&a, 3);
    text_pdf(&b, 2);
    let out = dir.path().join("fusion.pdf");

    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = calls.clone();
    let job = Job::new(Arc::new(move |p| seen.lock().unwrap().push(p.done)), CancelToken::new());
    let res = merge(engine(), vec![PdfInput::new(&a), PdfInput::new(&b)], out.clone(), job).unwrap();
    assert_eq!(res.path, out);
    assert_eq!(page_count(&out), 5);
    assert_eq!(*calls.lock().unwrap(), vec![1, 2]);
}

#[test]
fn merge_never_overwrites_an_input() {
    let dir = tmp();
    let a = dir.path().join("a.pdf");
    text_pdf(&a, 1);
    let before = std::fs::read(&a).unwrap();
    let err = merge(engine(), vec![PdfInput::new(&a), PdfInput::new(&a)], a.clone(), job()).unwrap_err();
    assert!(matches!(err, HestiaError::WouldOverwriteInput));
    assert_eq!(std::fs::read(&a).unwrap(), before);
}

#[test]
fn cancelled_merge_writes_nothing() {
    let dir = tmp();
    let a = dir.path().join("a.pdf");
    text_pdf(&a, 2);
    let out = dir.path().join("out.pdf");
    let cancel = CancelToken::new();
    cancel.cancel();
    let err = merge(engine(), vec![PdfInput::new(&a)], out.clone(), Job::new(hestia_core::job::no_progress(), cancel))
        .unwrap_err();
    assert!(matches!(err, HestiaError::Cancelled));
    assert!(!out.exists());
}

#[test]
fn split_modes_produce_expected_files() {
    let dir = tmp();
    let src = dir.path().join("rapport.pdf");
    text_pdf(&src, 6);

    let parts = plan(6, &SplitMode::Ranges { ranges: "1-2, 5-".into(), merge: false }).unwrap();
    let out_dir = dir.path().join("sortie");
    std::fs::create_dir(&out_dir).unwrap();
    let files = split(
        engine(),
        PdfInput::new(&src),
        parts,
        SplitTarget::Folder { dir: out_dir.clone(), stem: "rapport".into() },
        job(),
    )
    .unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0].path.ends_with("rapport_pages-1-2.pdf"));
    assert_eq!(page_count(&files[0].path), 2);
    assert_eq!(page_count(&files[1].path), 2);

    let parts = plan(6, &SplitMode::Select { pages: vec![6, 1, 3], merge: true }).unwrap();
    let single = dir.path().join("extrait.pdf");
    let files = split(engine(), PdfInput::new(&src), parts, SplitTarget::File(single.clone()), job()).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(page_count(&single), 3);
}

fn image_pdf(dir: &Path) -> (PathBuf, PathBuf) {
    let png = dir.join("photo.png");
    photo(1600, 1600).save(&png).unwrap();
    let jpg = dir.join("photo.jpg");
    write_jpeg(&jpg, &photo(2800, 2000), 97);
    let pdf = dir.join("photos.pdf");
    images_to_pdf(
        vec![ImageInput { path: png, rotation: 0 }, ImageInput { path: jpg.clone(), rotation: 0 }],
        ImagesToPdfOptions::default(),
        pdf.clone(),
        job(),
    )
    .unwrap();
    (pdf, jpg)
}

#[test]
fn compress_shrinks_photos_and_keeps_pages() {
    let dir = tmp();
    let (pdf, _) = image_pdf(dir.path());
    let original = std::fs::metadata(&pdf).unwrap().len();

    let out = dir.path().join("photos_compresse.pdf");
    let report = compress(engine(), PdfInput::new(&pdf), CompressionLevel::Recommended, out.clone(), job()).unwrap();
    assert!(!report.already_optimal);
    assert_eq!(report.images_optimized, 2);
    assert!(report.compressed_size < original / 3, "{} -> {}", original, report.compressed_size);
    assert_eq!(page_count(&out), 2);

    // Le niveau extrême est plus petit que le recommandé.
    let out2 = dir.path().join("photos_extreme.pdf");
    let extreme = compress(engine(), PdfInput::new(&pdf), CompressionLevel::Extreme, out2, job()).unwrap();
    assert!(extreme.compressed_size < report.compressed_size);
}

#[test]
fn compress_text_pdf_stays_valid() {
    let dir = tmp();
    let pdf = dir.path().join("texte.pdf");
    text_pdf(&pdf, 20);
    let out = dir.path().join("texte_compresse.pdf");
    let report = compress(engine(), PdfInput::new(&pdf), CompressionLevel::Low, out.clone(), job()).unwrap();
    assert!(report.compressed_size <= report.original_size);
    assert_eq!(page_count(&out), 20);
}

#[test]
fn pdf_pages_to_jpg_at_150_dpi() {
    let dir = tmp();
    let pdf = dir.path().join("doc.pdf");
    text_pdf(&pdf, 2);
    let out_dir = dir.path().join("jpg");
    std::fs::create_dir(&out_dir).unwrap();
    let files = pdf_to_images(
        engine(),
        PdfInput::new(&pdf),
        ImageExport::Pages { quality: ExportQuality::Normal },
        out_dir,
        "doc".into(),
        job(),
    )
    .unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0].path.ends_with("doc_page-01.jpg"));
    let img = image::open(&files[0].path).unwrap();
    // 595 × 842 pt à 150 dpi ≈ 1240 × 1754 px.
    assert!((img.width() as i32 - 1240).abs() <= 2 && (img.height() as i32 - 1754).abs() <= 2);
}

#[test]
fn extracted_jpeg_is_bit_identical() {
    let dir = tmp();
    let (pdf, jpg) = image_pdf(dir.path());
    let out_dir = dir.path().join("images");
    std::fs::create_dir(&out_dir).unwrap();
    let files =
        pdf_to_images(engine(), PdfInput::new(&pdf), ImageExport::Extract, out_dir, "photos".into(), job()).unwrap();
    assert_eq!(files.len(), 2);
    let original = std::fs::read(&jpg).unwrap();
    assert!(
        files.iter().any(|f| std::fs::read(&f.path).unwrap() == original),
        "le JPEG doit être extrait sans aucune perte"
    );
}

#[test]
fn images_to_pdf_handles_transparency_and_rotation() {
    let dir = tmp();
    let png = dir.path().join("logo.png");
    RgbaImage::from_fn(200, 100, |x, _| image::Rgba([200, 20, 20, if x < 100 { 0 } else { 255 }])).save(&png).unwrap();
    let pdf = dir.path().join("logo.pdf");
    images_to_pdf(
        vec![ImageInput { path: png.clone(), rotation: 90 }],
        ImagesToPdfOptions { page_size: PageSizeOption::Fit, ..Default::default() },
        pdf.clone(),
        job(),
    )
    .unwrap();
    let info = inspect(engine(), PdfInput::new(&pdf)).unwrap();
    // 200×100 px tourné de 90° → page 75 × 150 pt.
    assert_eq!(info.page_count, 1);
    assert!((info.pages[0].width - 75.0).abs() < 0.1 && (info.pages[0].height - 150.0).abs() < 0.1);

    // La partie transparente reste blanche au rendu, la partie opaque est rouge.
    let jpeg = render_page_jpeg(engine(), PdfInput::new(&pdf), 0, 75).unwrap();
    let img = image::load_from_memory(&jpeg).unwrap().to_rgb8();
    let top = img.get_pixel(37, 10).0; // haut = moitié gauche de l'image d'origine (transparente)
    let bottom = img.get_pixel(37, 140).0;
    assert!(top.iter().all(|&c| c > 230), "transparent → blanc, obtenu {top:?}");
    assert!(bottom[0] > 150 && bottom[1] < 80, "opaque → rouge, obtenu {bottom:?}");

    let out_dir = dir.path().join("each");
    std::fs::create_dir(&out_dir).unwrap();
    let files =
        images_to_pdf_each(vec![ImageInput { path: png, rotation: 0 }], ImagesToPdfOptions::default(), out_dir, job())
            .unwrap();
    assert!(files[0].path.ends_with("logo.pdf"));
}

#[test]
fn corrupt_files_return_errors_not_crashes() {
    let dir = tmp();
    let bad = dir.path().join("abime.pdf");
    std::fs::write(&bad, b"%PDF-1.7\n1 0 obj << /Type /Catalog >>\ngarbage").unwrap();
    assert!(inspect(engine(), PdfInput::new(&bad)).is_err());
    let out = dir.path().join("out.pdf");
    assert!(merge(engine(), vec![PdfInput::new(&bad)], out.clone(), job()).is_err());
    assert!(!out.exists());
    assert!(compress(engine(), PdfInput::new(&bad), CompressionLevel::Recommended, out.clone(), job()).is_err());

    let not_image = dir.path().join("faux.jpg");
    std::fs::write(&not_image, b"ceci n'est pas une image").unwrap();
    let err = images_to_pdf(
        vec![ImageInput { path: not_image, rotation: 0 }],
        ImagesToPdfOptions::default(),
        out.clone(),
        job(),
    )
    .unwrap_err();
    assert_eq!(err.code(), "unsupported_image");
    assert_eq!(err.file_name(), Some("faux.jpg"));
    assert!(!out.exists());
}

#[test]
fn password_protected_pdf() {
    let dir = tmp();
    let plain = dir.path().join("clair.pdf");
    text_pdf(&plain, 2);
    let mut doc = Document::load(&plain).unwrap();
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
    let state = EncryptionState::try_from(EncryptionVersion::V4 {
        document: &doc,
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: "proprio",
        user_password: "secret",
        permissions: Permissions::all(),
    })
    .unwrap();
    doc.encrypt(&state).unwrap();
    let locked = dir.path().join("protege.pdf");
    doc.save(&locked).unwrap();

    let err = inspect(engine(), PdfInput::new(&locked)).unwrap_err();
    assert!(matches!(err, HestiaError::PasswordRequired), "{err:?}");
    let err = inspect(engine(), PdfInput::with_password(&locked, "faux")).unwrap_err();
    assert!(matches!(err, HestiaError::WrongPassword), "{err:?}");
    assert_eq!(inspect(engine(), PdfInput::with_password(&locked, "secret")).unwrap().page_count, 2);

    // Les traitements fonctionnent avec le mot de passe.
    let out = dir.path().join("protege_compresse.pdf");
    compress(engine(), PdfInput::with_password(&locked, "secret"), CompressionLevel::Recommended, out.clone(), job())
        .unwrap();
    assert_eq!(page_count(&out), 2);
    let merged = dir.path().join("fusion.pdf");
    merge(engine(), vec![PdfInput::with_password(&locked, "secret"), PdfInput::new(&plain)], merged.clone(), job())
        .unwrap();
    assert_eq!(page_count(&merged), 4);
}

#[test]
fn jpeg_photo_is_embedded_without_recompression() {
    let dir = tmp();
    let jpg = dir.path().join("p.jpg");
    write_jpeg(&jpg, &photo(640, 480), 90);
    let pdf = dir.path().join("p.pdf");
    images_to_pdf(
        vec![ImageInput { path: jpg.clone(), rotation: 0 }],
        ImagesToPdfOptions::default(),
        pdf.clone(),
        job(),
    )
    .unwrap();
    let doc = Document::load(&pdf).unwrap();
    let original = std::fs::read(&jpg).unwrap();
    let embedded = doc.objects.values().any(|o| matches!(o, Object::Stream(s) if s.content == original));
    assert!(embedded, "les octets JPEG doivent être intégrés tels quels");
    let _ = DynamicImage::ImageRgb8(photo(1, 1));
}
