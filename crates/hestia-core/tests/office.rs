//! Conversions avec Microsoft Office installé. Ces tests pilotent Word,
//! Excel et PowerPoint (sans fenêtre) : ils sont ignorés par défaut.
//!
//!   cargo test -p hestia-core --test office -- --ignored --test-threads=1

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use hestia_core::inspect::inspect;
use hestia_core::office::{OfficeStatus, office_to_pdf};
use hestia_core::{Job, PdfEngine, PdfInput};

fn engine() -> &'static PdfEngine {
    let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/pdfium/pdfium.dll");
    PdfEngine::init(&dll).expect("moteur PDFium")
}

/// Crée des documents d'essai avec Office (Word, Excel, PowerPoint).
fn make_documents(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let (docx, xlsx, pptx) = (dir.join("lettre.docx"), dir.join("budget.xlsx"), dir.join("présentation.pptx"));
    let script = r#"
$ErrorActionPreference = 'Stop'
$w = New-Object -ComObject Word.Application; $w.Visible = $false; $w.DisplayAlerts = 0
$d = $w.Documents.Add()
$d.Content.Text = "Hestia PDF`rLettre d'essai : été, Noël, 10 €."
$d.Paragraphs(1).Style = -2
$d.SaveAs2($env:T_DOCX, 16); $d.Close(0); $w.Quit()
$x = New-Object -ComObject Excel.Application; $x.Visible = $false; $x.DisplayAlerts = $false
$b = $x.Workbooks.Add(); $s = $b.Worksheets.Item(1)
$s.Cells.Item(1, 1).Value2 = 'Poste'; $s.Cells.Item(1, 2).Value2 = 'Montant'
$s.Cells.Item(2, 1).Value2 = 'Loyer'; $s.Cells.Item(2, 2).Value2 = 850
$s.Cells.Item(3, 1).Value2 = 'Courses'; $s.Cells.Item(3, 2).Value2 = 320.5
$b.SaveAs($env:T_XLSX, 51); $b.Close($false); $x.Quit()
$p = New-Object -ComObject PowerPoint.Application
$pr = $p.Presentations.Add(0)
$sl = $pr.Slides.Add(1, 1); $sl.Shapes.Item(1).TextFrame.TextRange.Text = 'Hestia PDF'
$sl2 = $pr.Slides.Add(2, 2); $sl2.Shapes.Item(1).TextFrame.TextRange.Text = 'Deuxième diapositive'
$pr.SaveAs($env:T_PPTX, 24); $pr.Close()
if ($p.Presentations.Count -eq 0) { $p.Quit() }
foreach ($o in $sl2, $sl, $pr, $p, $s, $b, $x, $d, $w) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($o) }
[GC]::Collect(); [GC]::WaitForPendingFinalizers()
exit 0
"#;
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("T_DOCX", &docx)
        .env("T_XLSX", &xlsx)
        .env("T_PPTX", &pptx)
        .status()
        .unwrap();
    assert!(status.success(), "création des documents d'essai");
    // Laisse les instances d'Office se fermer avant de convertir.
    for _ in 0..60 {
        let out = Command::new("tasklist").args(["/FI", "IMAGENAME eq POWERPNT.EXE", "/NH"]).output().unwrap();
        if !String::from_utf8_lossy(&out.stdout).contains("POWERPNT") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    (docx, xlsx, pptx)
}

fn text_of(path: &Path) -> String {
    let path = path.to_path_buf();
    engine()
        .run(move |ctx| {
            let doc = ctx.pdfium().load_pdf_from_file(&path, None)?;
            let mut all = String::new();
            for page in doc.pages().iter() {
                all.push_str(&page.text()?.all());
                all.push('\n');
            }
            Ok(all)
        })
        .unwrap()
}

#[test]
#[ignore = "pilote Microsoft Office"]
fn office_documents_become_pdf_and_back() {
    let status = OfficeStatus::detect();
    assert!(status.word && status.excel && status.powerpoint, "Office requis : {status:?}");
    let dir = tempfile::tempdir().unwrap();
    let (docx, xlsx, pptx) = make_documents(dir.path());

    let out = dir.path().join("lettre.pdf");
    office_to_pdf(engine(), &status, &docx, &out, &Job::silent()).unwrap();
    let text = text_of(&out);
    assert!(text.contains("Hestia PDF") && text.contains("10 €"), "texte Word : {text:?}");

    let out = dir.path().join("budget.pdf");
    office_to_pdf(engine(), &status, &xlsx, &out, &Job::silent()).unwrap();
    let text = text_of(&out);
    assert!(text.contains("Loyer") && text.contains("850"), "texte Excel : {text:?}");

    let out = dir.path().join("présentation.pdf");
    office_to_pdf(engine(), &status, &pptx, &out, &Job::silent()).unwrap();
    assert_eq!(inspect(engine(), PdfInput::new(&out)).unwrap().page_count, 2, "une page par diapositive");
    assert!(text_of(&out).contains("Deuxième diapositive"));

    // Le fichier d'origine n'est jamais remplacé.
    assert!(office_to_pdf(engine(), &status, &docx, &docx, &Job::silent()).is_err());
}

#[test]
#[ignore = "pilote Microsoft Office"]
fn broken_document_reports_an_error() {
    let status = OfficeStatus::detect();
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("abîmé.docx");
    std::fs::write(&fake, b"ceci n'est pas un document Word").unwrap();
    let err = office_to_pdf(engine(), &status, &fake, &dir.path().join("x.pdf"), &Job::silent()).unwrap_err();
    assert!(matches!(err.code(), "office_failed" | "office_protected"), "{err}");
    assert!(!dir.path().join("x.pdf").exists());
}
