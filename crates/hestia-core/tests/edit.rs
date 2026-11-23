//! « Modifier PDF » : on vérifie au rendu (PDFium) que chaque élément ajouté
//! apparaît exactement là où l'utilisateur l'a placé.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hestia_core::edit::{EditDocument, EditObject, PageEdits, apply_edits};
use hestia_core::fonts::DirFonts;
use hestia_core::inspect::{inspect, render_page_jpeg};
use hestia_core::{Job, PdfEngine, PdfInput};
use image::RgbImage;
use lopdf::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
use lopdf::{Document, EncryptionState, EncryptionVersion, Object, Permissions, Stream, dictionary};

fn engine() -> &'static PdfEngine {
    let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/pdfium/pdfium.dll");
    PdfEngine::init(&dll).expect("moteur PDFium")
}

/// Polices livrées avec l'application (dossier de l'interface).
fn fonts() -> DirFonts {
    DirFonts(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fonts"))
}

/// PDF d'une page, avec boîtes, rotation et contenu au choix.
fn page_pdf(path: &Path, media: [i64; 4], crop: Option<[i64; 4]>, rotate: i64, content: &str, pages: usize) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let mut kids = Vec::new();
    for _ in 0..pages {
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => media.iter().map(|&v| Object::Integer(v)).collect::<Vec<_>>(),
            "Contents" => content_id,
            "Resources" => dictionary! {},
        };
        if let Some(c) = crop {
            page.set("CropBox", c.iter().map(|&v| Object::Integer(v)).collect::<Vec<_>>());
        }
        if rotate != 0 {
            page.set("Rotate", rotate);
        }
        kids.push(Object::from(doc.add_object(page)));
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
            Object::string_literal(b"hestia-edit-test".to_vec()),
            Object::string_literal(b"hestia-edit-test".to_vec()),
        ],
    );
    doc.save(path).unwrap();
}

fn a4(path: &Path) {
    page_pdf(path, [0, 0, 595, 842], None, 0, "", 1);
}

fn edit(src: &Path, out: &Path, page: u32, objects: Vec<EditObject>) -> hestia_core::edit::EditReport {
    apply_edits(
        engine(),
        PdfInput::new(src),
        EditDocument { pages: vec![PageEdits { page, objects }] },
        out.to_path_buf(),
        Job::silent(),
        &fonts(),
    )
    .unwrap()
}

/// Rendu de la page à l'échelle 1 (1 px = 1 pt) quand `width` = largeur en points.
fn render(path: &Path, page: u32, width: u32) -> RgbImage {
    let jpeg = render_page_jpeg(engine(), PdfInput::new(path), page, width).unwrap();
    image::load_from_memory(&jpeg).unwrap().to_rgb8()
}

fn near(p: [u8; 3], expected: [u8; 3], tol: i32) -> bool {
    p.iter().zip(expected).all(|(a, b)| (*a as i32 - b as i32).abs() <= tol)
}

fn px(img: &RgbImage, x: u32, y: u32) -> [u8; 3] {
    img.get_pixel(x, y).0
}

fn rect(x: f32, y: f32, w: f32, h: f32, fill: &str) -> EditObject {
    EditObject::Rect { x, y, w, h, stroke: None, fill: Some(fill.into()), stroke_width: 0.0, opacity: 1.0 }
}

const RED: [u8; 3] = [255, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

#[test]
fn rectangle_lands_where_placed() {
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));
    a4(&src);
    edit(&src, &out, 0, vec![rect(100.0, 100.0, 200.0, 100.0, "#ff0000")]);
    let img = render(&out, 0, 595);
    assert!(near(px(&img, 200, 150), RED, 30), "centre du rectangle : {:?}", px(&img, 200, 150));
    assert!(near(px(&img, 105, 105), RED, 40));
    assert!(near(px(&img, 50, 50), WHITE, 10));
    assert!(near(px(&img, 200, 215), WHITE, 10), "rien sous le rectangle");
    assert!(near(px(&img, 315, 150), WHITE, 10), "rien à droite");
}

#[test]
fn rotated_page_uses_displayed_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));
    page_pdf(&src, [0, 0, 595, 842], None, 90, "", 1);
    let info = inspect(engine(), PdfInput::new(&src)).unwrap();
    assert_eq!((info.pages[0].width, info.pages[0].height), (842.0, 595.0));
    edit(&src, &out, 0, vec![rect(50.0, 40.0, 120.0, 60.0, "#0000ff")]);
    let img = render(&out, 0, 842);
    assert!(near(px(&img, 110, 70), [0, 0, 255], 40), "{:?}", px(&img, 110, 70));
    assert!(near(px(&img, 110, 200), WHITE, 10));
    assert!(near(px(&img, 700, 70), WHITE, 10));

    for rotate in [180, 270] {
        let src2 = dir.path().join(format!("r{rotate}.pdf"));
        let out2 = dir.path().join(format!("r{rotate}-out.pdf"));
        page_pdf(&src2, [0, 0, 595, 842], None, rotate, "", 1);
        let w = if rotate == 180 { 595 } else { 842 };
        edit(&src2, &out2, 0, vec![rect(20.0, 30.0, 80.0, 50.0, "#00a000")]);
        let img = render(&out2, 0, w);
        assert!(near(px(&img, 60, 55), [0, 160, 0], 45), "rotation {rotate} : {:?}", px(&img, 60, 55));
        assert!(near(px(&img, 60, 120), WHITE, 10));
    }
}

#[test]
fn crop_box_offset_is_respected() {
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));
    page_pdf(&src, [0, 0, 700, 900], Some([50, 60, 650, 860]), 0, "", 1);
    edit(&src, &out, 0, vec![rect(0.0, 0.0, 40.0, 40.0, "#ff0000")]);
    let img = render(&out, 0, 600);
    assert!(near(px(&img, 20, 20), RED, 40), "{:?}", px(&img, 20, 20));
    assert!(near(px(&img, 60, 60), WHITE, 10));
}

#[test]
fn unbalanced_original_content_does_not_shift_additions() {
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));
    // Contenu qui décale le repère sans le restaurer (fichier mal construit).
    page_pdf(&src, [0, 0, 595, 842], None, 0, "1 0 0 1 200 300 cm 2 0 0 2 0 0 cm 0 0 1 rg 0 0 10 10 re f", 1);
    edit(&src, &out, 0, vec![rect(10.0, 10.0, 50.0, 50.0, "#00a000")]);
    let img = render(&out, 0, 595);
    assert!(near(px(&img, 35, 35), [0, 160, 0], 45), "{:?}", px(&img, 35, 35));
    // Le contenu d'origine est toujours là (carré bleu 20×20 à (200, 822-20)).
    assert!(near(px(&img, 210, 842 - 300 - 10), [0, 0, 255], 45));
}

#[test]
fn text_keeps_accents_and_position() {
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));
    a4(&src);
    let text = |lines: Vec<&str>, y: f32| EditObject::Text {
        x: 100.0,
        y,
        size: 40.0,
        font: "helvetica".into(),
        bold: true,
        italic: false,
        color: "#000000".into(),
        opacity: 1.0,
        lines: lines.into_iter().map(String::from).collect(),
    };
    let report =
        edit(&src, &out, 0, vec![text(vec!["HHHH"], 200.0), text(vec!["Été à Noël : 10 €", "Bonjour 👋"], 400.0)]);
    assert_eq!(report.replaced_chars, 1, "l'emoji est remplacé");

    let extracted = engine()
        .run({
            let out = out.clone();
            move |ctx| {
                let doc = ctx.pdfium().load_pdf_from_file(&out, None)?;
                let page = doc.pages().get(0)?;
                Ok(page.text()?.all())
            }
        })
        .unwrap();
    assert!(extracted.contains("Été à Noël : 10 €"), "texte extrait : {extracted:?}");

    // « HHHH » : ligne de base à y + 0,95 × 40 = 238 ; hauteur des capitales ≈ 29 pt.
    let img = render(&out, 0, 595);
    let dark_in_row = |y: u32| (100..250).any(|x| px(&img, x, y).iter().all(|&c| c < 90));
    assert!(dark_in_row(225), "lettres attendues vers y = 225");
    assert!(!dark_in_row(200), "rien au-dessus des capitales");
    assert!(!dark_in_row(246), "rien sous la ligne de base");
}

#[test]
fn shapes_drawings_images_and_opacity() {
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));
    a4(&src);
    let png = dir.path().join("rouge.png");
    RgbImage::from_pixel(64, 64, image::Rgb([255, 0, 0])).save(&png).unwrap();
    edit(
        &src,
        &out,
        0,
        vec![
            EditObject::Rect {
                x: 0.0,
                y: 0.0,
                w: 50.0,
                h: 50.0,
                stroke: None,
                fill: Some("#000000".into()),
                stroke_width: 0.0,
                opacity: 0.5,
            },
            EditObject::Ellipse {
                x: 400.0,
                y: 600.0,
                w: 100.0,
                h: 60.0,
                stroke: Some("#000000".into()),
                fill: Some("#00a000".into()),
                stroke_width: 2.0,
                opacity: 1.0,
            },
            EditObject::Ink {
                points: vec![[100.0, 700.0], [200.0, 700.0], [300.0, 700.0]],
                color: "#0000ff".into(),
                width: 8.0,
                opacity: 1.0,
            },
            EditObject::Line {
                x1: 100.0,
                y1: 780.0,
                x2: 300.0,
                y2: 780.0,
                color: "#ff0000".into(),
                width: 4.0,
                opacity: 1.0,
                arrow: true,
            },
            EditObject::Image {
                x: 300.0,
                y: 400.0,
                w: 100.0,
                h: 100.0,
                path: png.clone(),
                opacity: 1.0,
                rotation: 0,
                flip_h: false,
                grayscale: false,
            },
        ],
    );
    let img = render(&out, 0, 595);
    let gray = px(&img, 25, 25);
    assert!(near(gray, [128, 128, 128], 30), "noir à 50 % → gris : {gray:?}");
    assert!(near(px(&img, 450, 630), [0, 160, 0], 45), "ellipse");
    assert!(near(px(&img, 410, 605), WHITE, 15), "hors de l'ellipse (coin)");
    assert!(near(px(&img, 200, 700), [0, 0, 255], 45), "tracé");
    assert!(near(px(&img, 290, 780), RED, 60), "pointe de flèche");
    assert!(near(px(&img, 296, 772), WHITE, 40) || near(px(&img, 296, 772), RED, 60));
    assert!(near(px(&img, 350, 450), RED, 30), "image");
    assert!(near(px(&img, 350, 520), WHITE, 10), "sous l'image");
}

#[test]
fn only_the_edited_page_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));
    page_pdf(&src, [0, 0, 595, 842], None, 0, "", 2);
    edit(&src, &out, 1, vec![rect(0.0, 0.0, 595.0, 842.0, "#ff0000")]);
    assert!(near(px(&render(&out, 0, 100), 50, 50), WHITE, 10));
    assert!(near(px(&render(&out, 1, 100), 50, 50), RED, 30));
}

#[test]
fn protected_pdf_can_be_edited() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("clair.pdf");
    a4(&plain);
    let mut doc = Document::load(&plain).unwrap();
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
    let state = EncryptionState::try_from(EncryptionVersion::V4 {
        document: &doc,
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: "o",
        user_password: "secret",
        permissions: Permissions::all(),
    })
    .unwrap();
    doc.encrypt(&state).unwrap();
    let locked: PathBuf = dir.path().join("protege.pdf");
    doc.save(&locked).unwrap();

    let out = dir.path().join("modifie.pdf");
    let report = apply_edits(
        engine(),
        PdfInput::with_password(&locked, "secret"),
        EditDocument { pages: vec![PageEdits { page: 0, objects: vec![rect(10.0, 10.0, 30.0, 30.0, "#ff0000")] }] },
        out.clone(),
        Job::silent(),
        &fonts(),
    )
    .unwrap();
    assert!(report.rebuilt);
    assert!(near(px(&render(&out, 0, 595), 25, 25), RED, 40));
}

#[test]
fn invalid_requests_write_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));
    a4(&src);
    let run = |doc: EditDocument| apply_edits(engine(), PdfInput::new(&src), doc, out.clone(), Job::silent(), &fonts());
    assert!(run(EditDocument::default()).is_err(), "aucune modification");
    assert!(
        run(EditDocument { pages: vec![PageEdits { page: 5, objects: vec![rect(0.0, 0.0, 1.0, 1.0, "#000000")] }] })
            .is_err()
    );
    let missing = EditObject::Image {
        x: 0.0,
        y: 0.0,
        w: 10.0,
        h: 10.0,
        path: dir.path().join("absente.png"),
        opacity: 1.0,
        rotation: 0,
        flip_h: false,
        grayscale: false,
    };
    let err = run(EditDocument { pages: vec![PageEdits { page: 0, objects: vec![missing] }] }).unwrap_err();
    assert_eq!(err.file_name(), Some("absente.png"));
    assert!(!out.exists());
}

/// Contrat avec l'interface : le JSON produit par src/views/editor/model.ts
/// (fonction `toWire`) doit être compris tel quel par le moteur.
#[test]
fn accepts_the_exact_json_sent_by_the_interface() {
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));
    a4(&src);
    let png = dir.path().join("logo.png");
    RgbImage::from_pixel(10, 10, image::Rgb([0, 0, 255])).save(&png).unwrap();
    let json = serde_json::json!({
        "pages": [{
            "page": 0,
            "objects": [
                { "type": "text", "x": 50.5, "y": 60, "size": 16, "font": "times", "bold": false, "italic": true,
                  "color": "#111111", "opacity": 1, "lines": ["Bonjour", "Été 2026 — 10 €"] },
                { "type": "text", "x": 106.14, "y": 252.72, "size": 16, "font": "pacifico", "bold": false, "italic": false,
                  "color": "#2266cc", "opacity": 1, "lines": ["Bonjour Hestia 2026"] },
                { "type": "rect", "x": 10, "y": 10, "w": 30, "h": 20, "stroke": "#b91c1c", "fill": null, "strokeWidth": 2, "opacity": 0.8 },
                { "type": "ellipse", "x": 100, "y": 100, "w": 40, "h": 20, "stroke": null, "fill": "#facc15", "strokeWidth": 0, "opacity": 1 },
                { "type": "line", "x1": 0, "y1": 0, "x2": 100, "y2": 50, "color": "#000000", "width": 2, "opacity": 1, "arrow": true },
                { "type": "ink", "points": [[1, 2], [3, 4.5]], "color": "#facc15", "width": 14, "opacity": 0.4 },
                { "type": "image", "x": 200, "y": 300, "w": 50, "h": 50, "path": png, "opacity": 1,
                  "rotation": 270, "flipH": true, "grayscale": false }
            ]
        }]
    });
    let edits: EditDocument = serde_json::from_value(json).expect("format de l'interface");
    assert_eq!(edits.object_count(), 7);
    let report = apply_edits(engine(), PdfInput::new(&src), edits, out.clone(), Job::silent(), &fonts()).unwrap();
    assert_eq!(report.replaced_chars, 0);
    assert!(near(px(&render(&out, 0, 595), 225, 325), [0, 0, 255], 40), "l'image est à sa place");
}

/// L'aperçu de l'éditeur mesure le texte avec Arial / Times New Roman /
/// Courier New : le PDF (Helvetica / Times / Courier) doit avoir la même
/// largeur, sinon ce que l'on voit ne serait pas ce que l'on obtient.
/// Largeurs attendues = mesure de l'interface (canvas) pour la même phrase.
#[test]
fn text_width_matches_the_interface_preview() {
    let dir = tempfile::tempdir().unwrap();
    let phrase = "Hestia PDF : modifier un document";
    // Mesures faites dans l'interface à 40 px, ramenées à 20 pt.
    for (font, expected) in [("helvetica", 311.0), ("times", 286.0), ("courier", 396.0)] {
        let (src, out) = (dir.path().join("a.pdf"), dir.path().join(format!("{font}.pdf")));
        a4(&src);
        let text = EditObject::Text {
            x: 20.0,
            y: 100.0,
            size: 20.0,
            font: font.into(),
            bold: false,
            italic: false,
            color: "#000000".into(),
            opacity: 1.0,
            lines: vec![phrase.into()],
        };
        edit(&src, &out, 0, vec![text]);
        let img = render(&out, 0, 595);
        let dark: Vec<u32> =
            (0..595).filter(|&x| (100..130).any(|y| px(&img, x, y).iter().all(|&c| c < 128))).collect();
        let width = (dark.last().unwrap() - dark.first().unwrap()) as f32;
        let ratio = width / expected;
        assert!((0.96..=1.02).contains(&ratio), "{font} : largeur PDF {width} pour {expected} attendus");
    }
}

fn text_obj(font: &str, bold: bool, italic: bool, x: f32, y: f32, size: f32, line: &str) -> EditObject {
    EditObject::Text {
        x,
        y,
        size,
        font: font.into(),
        bold,
        italic,
        color: "#000000".into(),
        opacity: 1.0,
        lines: vec![line.into()],
    }
}

fn extract_text(path: &Path, page: i32) -> String {
    let path = path.to_path_buf();
    engine()
        .run(move |ctx| {
            let doc = ctx.pdfium().load_pdf_from_file(&path, None)?;
            let page = doc.pages().get(page)?;
            Ok(page.text()?.all())
        })
        .unwrap()
}

/// Polices livrées avec l'application : intégrées (sous-ensemble), le texte
/// reste copiable et la largeur est celle de l'aperçu (somme des chasses,
/// sans crénage, comme le mesure l'interface).
#[test]
fn embedded_fonts_render_match_widths_and_stay_searchable() {
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));
    page_pdf(&src, [0, 0, 595, 842], None, 0, "", 2);
    let phrase = "Hestia PDF : modifier un document";
    let accents = "Été à Noël : 10 € — « œuvre »";
    let edits = EditDocument {
        pages: vec![
            PageEdits {
                page: 0,
                objects: vec![
                    text_obj("montserrat", false, false, 20.0, 100.0, 20.0, phrase),
                    text_obj("lato", true, false, 20.0, 300.0, 24.0, accents),
                    text_obj("caveat", true, true, 20.0, 500.0, 24.0, "Bonjour 世界"),
                ],
            },
            PageEdits { page: 1, objects: vec![text_obj("lato", true, false, 20.0, 100.0, 24.0, "Encore")] },
        ],
    };
    let report = apply_edits(engine(), PdfInput::new(&src), edits, out.clone(), Job::silent(), &fonts()).unwrap();
    assert_eq!(report.replaced_chars, 2, "les idéogrammes absents de Caveat sont remplacés");

    let text = extract_text(&out, 0);
    assert!(text.contains(phrase), "texte extrait : {text:?}");
    assert!(text.contains(accents), "texte extrait : {text:?}");
    assert!(extract_text(&out, 1).contains("Encore"));

    // Largeur attendue : somme des chasses de Montserrat Regular.
    let data = std::fs::read(fonts().0.join("Montserrat-Regular.ttf")).unwrap();
    let face = ttf_parser::Face::parse(&data, 0).unwrap();
    let advance: f32 =
        phrase.chars().map(|c| face.glyph_hor_advance(face.glyph_index(c).unwrap()).unwrap() as f32).sum::<f32>()
            * 20.0
            / face.units_per_em() as f32;
    let img = render(&out, 0, 595);
    let dark: Vec<u32> = (0..595).filter(|&x| (100..130).any(|y| px(&img, x, y).iter().all(|&c| c < 128))).collect();
    let width = (dark.last().unwrap() - dark.first().unwrap()) as f32;
    assert!((0.95..=1.01).contains(&(width / advance)), "largeur {width} pour {advance} attendus");

    // Une seule police intégrée par fichier, même utilisée sur deux pages, et
    // seulement les glyphes utilisés (Lato Gras complet : 69 Kio).
    let doc = Document::load(&out).unwrap();
    let files: Vec<usize> = doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .filter_map(|d| d.get(b"FontFile2").ok())
        .map(|r| {
            let stream = doc.get_object(r.as_reference().unwrap()).unwrap().as_stream().unwrap();
            stream.dict.get(b"Length1").unwrap().as_i64().unwrap() as usize
        })
        .collect();
    assert_eq!(files.len(), 3, "Montserrat, Lato Gras, Caveat Gras");
    assert!(files.iter().all(|&len| len < 30_000), "sous-ensembles attendus : {files:?}");
}

#[test]
fn unknown_font_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("a.pdf"), dir.path().join("b.pdf"));
    a4(&src);
    for font in ["comic", "../lato", ""] {
        let edits = EditDocument {
            pages: vec![PageEdits { page: 0, objects: vec![text_obj(font, false, false, 0.0, 0.0, 12.0, "x")] }],
        };
        assert!(apply_edits(engine(), PdfInput::new(&src), edits, out.clone(), Job::silent(), &fonts()).is_err());
    }
    assert!(!out.exists());
}

/// Image de 100 × 50 px : moitié gauche rouge, moitié droite bleue.
fn two_tone(path: &Path) {
    let img = RgbImage::from_fn(100, 50, |x, _| if x < 50 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) });
    img.save(path).unwrap();
}

fn placed(path: &Path, w: f32, h: f32, rotation: u16, flip_h: bool, grayscale: bool) -> EditObject {
    EditObject::Image { x: 100.0, y: 100.0, w, h, path: path.to_path_buf(), opacity: 1.0, rotation, flip_h, grayscale }
}

/// Pivoter, miroir et noir et blanc : même résultat que l'aperçu de l'éditeur.
#[test]
fn image_rotation_mirror_and_grayscale() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("a.pdf");
    a4(&src);
    let png = dir.path().join("deux.png");
    two_tone(&png);
    const BLUE: [u8; 3] = [0, 0, 255];
    let run = |name: &str, obj: EditObject| {
        let out = dir.path().join(name);
        edit(&src, &out, 0, vec![obj]);
        render(&out, 0, 595)
    };

    // Telle quelle : cadre 200 × 100, rouge à gauche.
    let img = run("0.pdf", placed(&png, 200.0, 100.0, 0, false, false));
    assert!(near(px(&img, 150, 150), RED, 40) && near(px(&img, 250, 150), BLUE, 40));

    // Miroir horizontal : bleu à gauche.
    let img = run("h.pdf", placed(&png, 200.0, 100.0, 0, true, false));
    assert!(near(px(&img, 150, 150), BLUE, 40) && near(px(&img, 250, 150), RED, 40));

    // Quart de tour horaire : cadre 100 × 200, le côté gauche (rouge) passe en haut.
    let img = run("90.pdf", placed(&png, 100.0, 200.0, 90, false, false));
    assert!(near(px(&img, 150, 150), RED, 40), "haut : {:?}", px(&img, 150, 150));
    assert!(near(px(&img, 150, 250), BLUE, 40), "bas : {:?}", px(&img, 150, 250));

    // Quart de tour antihoraire (270) : le rouge passe en bas.
    let img = run("270.pdf", placed(&png, 100.0, 200.0, 270, false, false));
    assert!(near(px(&img, 150, 150), BLUE, 40) && near(px(&img, 150, 250), RED, 40));

    // Miroir vertical = miroir horizontal + demi-tour : rouge toujours à gauche.
    let img = run("v.pdf", placed(&png, 200.0, 100.0, 180, true, false));
    assert!(near(px(&img, 150, 150), RED, 40) && near(px(&img, 250, 150), BLUE, 40));

    // Noir et blanc (PNG puis JPEG) : rouge et bleu deviennent des gris
    // (luminance Rec. 709, comme le filtre CSS de l'aperçu).
    for file in ["deux.png", "deux.jpg"] {
        let path = dir.path().join(file);
        two_tone(&path);
        let img = run(&format!("{file}.pdf"), placed(&path, 200.0, 100.0, 0, false, true));
        let (left, right) = (px(&img, 150, 150), px(&img, 250, 150));
        for p in [left, right] {
            assert!(p[0].abs_diff(p[1]) <= 6 && p[1].abs_diff(p[2]) <= 6, "{file} : pas gris {p:?}");
        }
        assert!(near(left, [54, 54, 54], 20), "{file} : rouge → {left:?}");
        assert!(near(right, [18, 18, 18], 20), "{file} : bleu → {right:?}");
    }
    let rotated =
        EditDocument { pages: vec![PageEdits { page: 0, objects: vec![placed(&png, 10.0, 10.0, 45, false, false)] }] };
    assert!(
        apply_edits(engine(), PdfInput::new(&src), rotated, dir.path().join("x.pdf"), Job::silent(), &fonts()).is_err()
    );
}
