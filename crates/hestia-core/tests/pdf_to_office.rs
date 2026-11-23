//! PDF → Word / PowerPoint / Excel (moteur intégré, sans Office).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Read as _;
use std::path::Path;

use hestia_core::edit::{EditDocument, EditObject, PageEdits, apply_edits};
use hestia_core::fonts::DirFonts;
use hestia_core::pdf_to_office::{OfficeFormat, pdf_to_office};
use hestia_core::{Job, PdfEngine, PdfInput};
use lopdf::{Document, Object, Stream, dictionary};

fn engine() -> &'static PdfEngine {
    let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/pdfium/pdfium.dll");
    PdfEngine::init(&dll).expect("moteur PDFium")
}

fn text(x: f32, y: f32, size: f32, bold: bool, line: &str) -> EditObject {
    EditObject::Text {
        x,
        y,
        size,
        font: "helvetica".into(),
        bold,
        italic: false,
        color: "#000000".into(),
        opacity: 1.0,
        lines: vec![line.into()],
    }
}

/// PDF d'essai : un titre centré, un paragraphe, un petit tableau.
fn sample(dir: &Path) -> std::path::PathBuf {
    let blank = dir.join("blanc.pdf");
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        "Contents" => content, "Resources" => dictionary! {},
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc.save(&blank).unwrap();

    let out = dir.join("facture.pdf");
    let fonts = DirFonts(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fonts"));
    let objects = vec![
        text(230.0, 60.0, 24.0, true, "Facture 2026"),
        text(72.0, 120.0, 12.0, false, "Merci pour votre confiance. Voici le détail de la prestation."),
        text(72.0, 200.0, 12.0, true, "Désignation"),
        text(400.0, 200.0, 12.0, true, "Montant"),
        text(72.0, 220.0, 12.0, false, "Site internet"),
        text(400.0, 220.0, 12.0, false, "1 250,50"),
        text(72.0, 240.0, 12.0, false, "Hébergement"),
        text(400.0, 240.0, 12.0, false, "120"),
        text(72.0, 260.0, 12.0, false, "TVA"),
        text(400.0, 260.0, 12.0, false, "20 %"),
    ];
    apply_edits(
        engine(),
        PdfInput::new(&blank),
        EditDocument { pages: vec![PageEdits { page: 0, objects }] },
        out.clone(),
        Job::silent(),
        &fonts,
    )
    .unwrap();
    out
}

/// Contenu d'un fichier dans une archive ZIP (décompression avec flate2).
fn zip_entry(archive: &[u8], name: &str) -> Option<String> {
    let mut i = 0;
    while i + 30 <= archive.len() && archive[i..i + 4] == *b"PK\x03\x04" {
        let le16 = |o: usize| u16::from_le_bytes([archive[o], archive[o + 1]]) as usize;
        let le32 = |o: usize| u32::from_le_bytes([archive[o], archive[o + 1], archive[o + 2], archive[o + 3]]) as usize;
        let method = le16(i + 8);
        let size = le32(i + 18);
        let name_len = le16(i + 26);
        let extra = le16(i + 28);
        let entry = std::str::from_utf8(&archive[i + 30..i + 30 + name_len]).unwrap_or("");
        let start = i + 30 + name_len + extra;
        let body = &archive[start..start + size];
        if entry == name {
            let mut out = String::new();
            if method == 8 {
                flate2::read::DeflateDecoder::new(body).read_to_string(&mut out).ok()?;
            } else {
                out = String::from_utf8_lossy(body).into_owned();
            }
            return Some(out);
        }
        i = start + size;
    }
    None
}

#[test]
fn pdf_becomes_word_powerpoint_and_excel() {
    let dir = tempfile::tempdir().unwrap();
    let src = sample(dir.path());

    // Word : texte, styles, titre centré.
    let docx = dir.path().join("facture.docx");
    let report = pdf_to_office(engine(), PdfInput::new(&src), OfficeFormat::Docx, &docx, &Job::silent()).unwrap();
    assert!(!report.no_text);
    let bytes = std::fs::read(&docx).unwrap();
    let doc = zip_entry(&bytes, "word/document.xml").expect("document.xml");
    assert!(doc.contains("Facture 2026"), "{doc}");
    assert!(doc.contains("Merci pour votre confiance. Voici le détail de la prestation."));
    assert!(doc.contains(r#"<w:jc w:val="center"/>"#), "titre centré");
    assert!(doc.contains("<w:b/>"), "gras conservé");
    assert!(doc.contains(r#"<w:sz w:val="48"/>"#), "taille 24 pt");
    assert!(zip_entry(&bytes, "[Content_Types].xml").is_some());

    // PowerPoint : une diapositive, fond + zones de texte.
    let pptx = dir.path().join("facture.pptx");
    pdf_to_office(engine(), PdfInput::new(&src), OfficeFormat::Pptx, &pptx, &Job::silent()).unwrap();
    let bytes = std::fs::read(&pptx).unwrap();
    let slide = zip_entry(&bytes, "ppt/slides/slide1.xml").expect("slide1.xml");
    assert!(slide.contains("Facture 2026") && slide.contains("1 250,50"));
    assert!(slide.contains(r#"r:embed="rId2""#), "fond de page");
    assert!(zip_entry(&bytes, "ppt/slides/slide2.xml").is_none());
    let pres = zip_entry(&bytes, "ppt/presentation.xml").unwrap();
    assert!(pres.contains(r#"<p:sldSz cx="7556500" cy="10693400"/>"#), "taille A4 : {pres}");

    // Excel : colonnes et nombres.
    let xlsx = dir.path().join("facture.xlsx");
    pdf_to_office(engine(), PdfInput::new(&src), OfficeFormat::Xlsx, &xlsx, &Job::silent()).unwrap();
    let bytes = std::fs::read(&xlsx).unwrap();
    let sheet = zip_entry(&bytes, "xl/worksheets/sheet1.xml").expect("sheet1.xml");
    assert!(sheet.contains("Désignation") && sheet.contains("Montant"));
    assert!(sheet.contains("<v>1250.5</v>"), "nombre français reconnu : {sheet}");
    assert!(sheet.contains("<v>0.2</v>"), "pourcentage");
    // « Montant » et « 1 250,50 » sont dans la même colonne (B).
    let col_of = |needle: &str| {
        let at = sheet.find(needle).unwrap();
        let cell = sheet[..at].rfind("<c r=\"").unwrap();
        sheet[cell + 6..cell + 7].to_string()
    };
    assert_eq!(col_of("Montant"), "B");
    assert_eq!(col_of("<v>1250.5</v>"), "B");
    assert_eq!(col_of("Site internet"), "A");

    // Jamais par-dessus l'original.
    assert!(pdf_to_office(engine(), PdfInput::new(&src), OfficeFormat::Docx, &src, &Job::silent()).is_err());
}
