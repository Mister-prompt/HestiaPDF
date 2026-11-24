//! « Organiser » avec la vraie bibliothèque PDFium : ordre, rotation,
//! pages blanches, plusieurs fichiers.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use hestia_core::inspect::inspect;
use hestia_core::organize::{PageSpec, organize};
use hestia_core::{HestiaError, Job, PdfEngine, PdfInput};
use lopdf::{Document, Object, Stream, dictionary};

fn engine() -> &'static PdfEngine {
    let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/pdfium/pdfium.dll");
    PdfEngine::init(&dll).expect("moteur PDFium")
}

/// PDF dont chaque page a sa propre largeur (pour la reconnaître) et
/// éventuellement une rotation propre.
fn pdf(path: &Path, pages: &[(i64, i64)]) -> PathBuf {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let mut kids = Vec::new();
    for (i, &(width, rotate)) in pages.iter().enumerate() {
        let content =
            doc.add_object(Stream::new(dictionary! {}, format!("BT /F1 12 Tf 10 10 Td (P{i}) Tj ET").into_bytes()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), width.into(), 400.into()],
            "Rotate" => rotate,
            "Contents" => content,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        });
        kids.push(Object::Reference(page));
    }
    let count = kids.len() as i64;
    doc.objects
        .insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
    path.to_path_buf()
}

fn sizes(path: &Path) -> Vec<(f32, f32)> {
    let info = inspect(engine(), PdfInput::new(path)).unwrap();
    info.pages.iter().map(|p| (p.width.round(), p.height.round())).collect()
}

fn rotations(path: &Path) -> Vec<i64> {
    let doc = Document::load(path).unwrap();
    doc.get_pages()
        .values()
        .map(|id| doc.get_dictionary(*id).unwrap().get(b"Rotate").and_then(Object::as_i64).unwrap_or(0))
        .collect()
}

#[test]
fn reorders_rotates_and_mixes_files() {
    let dir = tempfile::tempdir().unwrap();
    let a = pdf(&dir.path().join("a.pdf"), &[(100, 0), (200, 0), (300, 0)]);
    let b = pdf(&dir.path().join("b.pdf"), &[(500, 0), (600, 90)]);
    let out = dir.path().join("organise.pdf");

    let plan = vec![
        PageSpec::Source { file: 1, page: 0, rotation: 0 },
        PageSpec::Source { file: 0, page: 2, rotation: 0 },
        PageSpec::Source { file: 0, page: 0, rotation: 0 },
        PageSpec::Blank { width: 50.0, height: 60.0, rotation: 0 },
        // Rotation propre de 90° + 90° demandés.
        PageSpec::Source { file: 1, page: 1, rotation: 90 },
        PageSpec::Source { file: 0, page: 1, rotation: 270 },
    ];
    let written =
        organize(engine(), vec![PdfInput::new(&a), PdfInput::new(&b)], plan, out.clone(), Job::silent()).unwrap();
    assert_eq!(written.path, out);

    // Tailles affichées, rotation comprise : l'ordre est respecté.
    assert_eq!(
        sizes(&out),
        vec![(500.0, 400.0), (300.0, 400.0), (100.0, 400.0), (50.0, 60.0), (600.0, 400.0), (400.0, 200.0)]
    );
    assert_eq!(rotations(&out), vec![0, 0, 0, 0, 180, 270]);
    // Le texte suit sa page.
    let text = Document::load(&out).unwrap().extract_text(&[2]).unwrap();
    assert!(text.contains("P2"), "{text}");
    // L'original n'a pas bougé.
    assert_eq!(sizes(&a), vec![(100.0, 400.0), (200.0, 400.0), (300.0, 400.0)]);
}

#[test]
fn refuses_bad_requests() {
    let dir = tempfile::tempdir().unwrap();
    let a = pdf(&dir.path().join("a.pdf"), &[(100, 0)]);
    let out = dir.path().join("out.pdf");
    let one = || vec![PdfInput::new(&a)];

    // Page inexistante : erreur, et aucun fichier laissé derrière.
    let err =
        organize(engine(), one(), vec![PageSpec::Source { file: 0, page: 5, rotation: 0 }], out.clone(), Job::silent());
    assert!(matches!(err, Err(HestiaError::InvalidRange(_)) | Err(HestiaError::InFile { .. })), "{err:?}");
    assert!(!out.exists());

    // Jamais par-dessus l'original.
    let err =
        organize(engine(), one(), vec![PageSpec::Source { file: 0, page: 0, rotation: 0 }], a.clone(), Job::silent());
    assert!(err.is_err());
    assert_eq!(sizes(&a), vec![(100.0, 400.0)]);

    // Aucune page.
    assert!(organize(engine(), one(), vec![], out.clone(), Job::silent()).is_err());
}
