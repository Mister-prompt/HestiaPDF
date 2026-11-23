//! Modifier un PDF : ajout de texte, de formes, de dessins à main levée et
//! d'images, comme sur iLovePDF.
//!
//! Les éléments sont « tamponnés » dans la page : un nouveau flux de contenu
//! est ajouté, le contenu d'origine n'est pas réécrit (il est seulement
//! isolé entre `q` et `Q`, ce qui neutralise les fichiers dont le contenu
//! laisse le repère décalé). Le résultat s'affiche et s'imprime à
//! l'identique dans tous les lecteurs PDF.
//!
//! ## Repère
//! L'interface envoie des coordonnées en points (1/72 de pouce), origine en
//! haut à gauche de la page **telle qu'elle est affichée** (rotation de la
//! page comprise, zone visible = CropBox). La conversion vers le repère PDF
//! est faite ici, page par page.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

use lopdf::{Dictionary, Document, Object, ObjectId, SaveOptions, Stream, dictionary};
use serde::{Deserialize, Serialize};
use subsetter::GlyphRemapper;
use ttf_parser::GlyphId;

use crate::engine::{PdfEngine, PdfInput};
use crate::error::{HestiaError, Result};
use crate::fonts::{self, FontSource};
use crate::images_to_pdf::{Affine, Op, load_image, num, orientation_ops, place_in_box};
use crate::job::Job;
use crate::lopdf_io;
use crate::output::{OutputFile, ensure_not_input, write_atomic};

/// Hauteur d'une ligne de texte, en multiple de la taille de police.
/// Doit rester identique dans l'interface (src/views/editor/model.ts).
pub const TEXT_LINE_HEIGHT: f32 = 1.25;
/// Position de la ligne de base sous le haut de la ligne, en multiple de la taille.
pub const TEXT_BASELINE: f32 = 0.95;

// Limites de sécurité (valeurs absurdes ou malveillantes refusées).
const MAX_OBJECTS_PER_PAGE: usize = 5_000;
const MAX_TEXT_CHARS: usize = 100_000;
const MAX_INK_POINTS: usize = 50_000;
const MAX_COORD: f32 = 100_000.0;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditDocument {
    pub pages: Vec<PageEdits>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageEdits {
    /// Indice de page, à partir de 0.
    pub page: u32,
    pub objects: Vec<EditObject>,
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EditObject {
    #[serde(rename_all = "camelCase")]
    Text {
        x: f32,
        y: f32,
        size: f32,
        /// `helvetica`, `times`, `courier` (polices standard) ou une police
        /// livrée avec l'application (voir `fonts::FAMILIES`).
        font: String,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        italic: bool,
        color: String,
        #[serde(default = "one")]
        opacity: f32,
        lines: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        stroke: Option<String>,
        fill: Option<String>,
        stroke_width: f32,
        #[serde(default = "one")]
        opacity: f32,
    },
    #[serde(rename_all = "camelCase")]
    Ellipse {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        stroke: Option<String>,
        fill: Option<String>,
        stroke_width: f32,
        #[serde(default = "one")]
        opacity: f32,
    },
    #[serde(rename_all = "camelCase")]
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: String,
        width: f32,
        #[serde(default = "one")]
        opacity: f32,
        #[serde(default)]
        arrow: bool,
    },
    #[serde(rename_all = "camelCase")]
    Ink {
        points: Vec<[f32; 2]>,
        color: String,
        width: f32,
        #[serde(default = "one")]
        opacity: f32,
    },
    #[serde(rename_all = "camelCase")]
    Image {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        path: PathBuf,
        #[serde(default = "one")]
        opacity: f32,
        /// Rotation dans le sens horaire (0, 90, 180 ou 270), appliquée après le miroir.
        #[serde(default)]
        rotation: u16,
        /// Miroir horizontal de l'image.
        #[serde(default)]
        flip_h: bool,
        /// Noir et blanc (niveaux de gris).
        #[serde(default)]
        grayscale: bool,
    },
}

impl EditObject {
    /// Fichiers images utilisés (à vérifier par l'appelant).
    pub fn image_path(&self) -> Option<&std::path::Path> {
        match self {
            EditObject::Image { path, .. } => Some(path),
            _ => None,
        }
    }
}

impl EditDocument {
    pub fn image_paths(&self) -> impl Iterator<Item = &std::path::Path> {
        self.pages.iter().flat_map(|p| p.objects.iter().filter_map(EditObject::image_path))
    }

    pub fn object_count(&self) -> usize {
        self.pages.iter().map(|p| p.objects.len()).sum()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditReport {
    pub output: OutputFile,
    /// Caractères absents des polices standard, remplacés par « ? ».
    pub replaced_chars: u32,
    /// Le document a dû être reconstruit (protégé ou abîmé) : signets perdus.
    pub rebuilt: bool,
}

/// Applique les modifications et enregistre le résultat dans `output`.
/// `fonts` fournit les fichiers des polices livrées avec l'application.
pub fn apply_edits(
    engine: &PdfEngine,
    input: PdfInput,
    edits: EditDocument,
    output: PathBuf,
    job: Job,
    fonts: &dyn FontSource,
) -> Result<EditReport> {
    ensure_not_input(&output, [input.path.as_path()])?;
    validate(&edits)?;
    if edits.object_count() == 0 {
        return Err(HestiaError::NoInput);
    }

    let steps = edits.pages.len() as u32 + 2;
    job.report(0, steps);
    let original = std::fs::read(&input.path)?;
    let lopdf_io::Loaded { mut doc, page_count, rebuilt } = lopdf_io::load(engine, &input, &original)?;
    drop(original);
    let pages = doc.get_pages();

    let mut stamp = Stamp::new(fonts);
    for (i, page_edits) in edits.pages.iter().enumerate() {
        job.check()?;
        if page_edits.objects.is_empty() {
            continue;
        }
        let page_id = *pages.get(&(page_edits.page + 1)).ok_or_else(|| {
            HestiaError::InvalidRange(format!("page {} inexistante ({page_count} pages)", page_edits.page + 1))
        })?;
        stamp.edit_page(&mut doc, page_id, &page_edits.objects)?;
        job.report(i as u32 + 1, steps);
    }
    // Polices intégrées : sous-ensembles des glyphes réellement utilisés.
    stamp.finish(&mut doc)?;

    doc.compress();
    let mut bytes = Vec::new();
    doc.save_with_options(
        &mut bytes,
        SaveOptions::builder().use_object_streams(true).use_xref_streams(true).compression_level(6).build(),
    )?;
    drop(doc);
    job.check()?;
    lopdf_io::verify(engine, bytes.clone(), page_count)?;
    job.report(steps - 1, steps);
    job.check()?;

    let out = write_atomic(&output, |f| f.write_all(&bytes).map_err(Into::into))?;
    job.report(steps, steps);
    Ok(EditReport { output: out, replaced_chars: stamp.replaced, rebuilt })
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn invalid(what: &str) -> HestiaError {
    HestiaError::Internal(format!("modification invalide : {what}"))
}

fn coord(v: f32) -> Result<()> {
    if v.is_finite() && v.abs() <= MAX_COORD { Ok(()) } else { Err(invalid("coordonnée hors limites")) }
}

fn positive(v: f32, max: f32, what: &str) -> Result<()> {
    if v.is_finite() && v >= 0.0 && v <= max { Ok(()) } else { Err(invalid(what)) }
}

fn color(c: &str) -> Result<(f32, f32, f32)> {
    let hex = c.strip_prefix('#').ok_or_else(|| invalid("couleur"))?;
    if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(invalid("couleur"));
    }
    let channel =
        |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map(|v| v as f32 / 255.0).map_err(|_| invalid("couleur"));
    Ok((channel(0)?, channel(2)?, channel(4)?))
}

fn validate(edits: &EditDocument) -> Result<()> {
    for page in &edits.pages {
        if page.objects.len() > MAX_OBJECTS_PER_PAGE {
            return Err(invalid("trop d'éléments sur une page"));
        }
        for obj in &page.objects {
            match obj {
                EditObject::Text { x, y, size, font, color: c, opacity, lines, .. } => {
                    if !fonts::is_known(font) {
                        return Err(invalid("police inconnue"));
                    }
                    coord(*x)?;
                    coord(*y)?;
                    positive(*size, 500.0, "taille de texte")?;
                    if *size < 1.0 {
                        return Err(invalid("taille de texte"));
                    }
                    positive(*opacity, 1.0, "opacité")?;
                    color(c)?;
                    if lines.iter().map(|l| l.chars().count()).sum::<usize>() > MAX_TEXT_CHARS {
                        return Err(invalid("texte trop long"));
                    }
                }
                EditObject::Rect { x, y, w, h, stroke, fill, stroke_width, opacity }
                | EditObject::Ellipse { x, y, w, h, stroke, fill, stroke_width, opacity } => {
                    for v in [x, y] {
                        coord(*v)?;
                    }
                    positive(*w, MAX_COORD, "largeur")?;
                    positive(*h, MAX_COORD, "hauteur")?;
                    positive(*stroke_width, 200.0, "épaisseur")?;
                    positive(*opacity, 1.0, "opacité")?;
                    for c in [stroke, fill].into_iter().flatten() {
                        color(c)?;
                    }
                }
                EditObject::Line { x1, y1, x2, y2, color: c, width, opacity, .. } => {
                    for v in [x1, y1, x2, y2] {
                        coord(*v)?;
                    }
                    positive(*width, 200.0, "épaisseur")?;
                    positive(*opacity, 1.0, "opacité")?;
                    color(c)?;
                }
                EditObject::Ink { points, color: c, width, opacity } => {
                    if points.is_empty() || points.len() > MAX_INK_POINTS {
                        return Err(invalid("tracé"));
                    }
                    for [px, py] in points {
                        coord(*px)?;
                        coord(*py)?;
                    }
                    positive(*width, 200.0, "épaisseur")?;
                    positive(*opacity, 1.0, "opacité")?;
                    color(c)?;
                }
                EditObject::Image { x, y, w, h, opacity, rotation, .. } => {
                    coord(*x)?;
                    coord(*y)?;
                    positive(*w, MAX_COORD, "largeur")?;
                    positive(*h, MAX_COORD, "hauteur")?;
                    positive(*opacity, 1.0, "opacité")?;
                    if !matches!(rotation, 0 | 90 | 180 | 270) {
                        return Err(invalid("rotation d'image"));
                    }
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Géométrie de la page
// ---------------------------------------------------------------------------

/// Zone visible et rotation d'une page.
struct PageGeometry {
    x0: f32,
    y0: f32,
    width: f32,
    height: f32,
    rotation: i64,
}

impl PageGeometry {
    /// Hauteur de la page affichée (rotation comprise).
    fn display_height(&self) -> f32 {
        if self.rotation % 180 == 0 { self.height } else { self.width }
    }

    /// Matrice du repère « page affichée, y vers le haut » vers le repère PDF.
    fn to_user_space(&self) -> Affine {
        let (x0, y0, w, h) = (self.x0, self.y0, self.width, self.height);
        match self.rotation {
            90 => Affine { a: 0.0, b: 1.0, c: -1.0, d: 0.0, e: x0 + w, f: y0 },
            180 => Affine { a: -1.0, b: 0.0, c: 0.0, d: -1.0, e: x0 + w, f: y0 + h },
            270 => Affine { a: 0.0, b: -1.0, c: 1.0, d: 0.0, e: x0, f: y0 + h },
            _ => Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: x0, f: y0 },
        }
    }
}

/// Attribut éventuellement hérité des nœuds parents de l'arbre des pages.
fn inherited<'a>(doc: &'a Document, page_id: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut current = doc.get_dictionary(page_id).ok()?;
    for _ in 0..64 {
        if let Ok(value) = current.get(key) {
            return doc.dereference(value).ok().map(|(_, o)| o);
        }
        let parent = current.get(b"Parent").and_then(Object::as_reference).ok()?;
        current = doc.get_dictionary(parent).ok()?;
    }
    None
}

fn rect_of(doc: &Document, obj: &Object) -> Option<[f32; 4]> {
    let arr = obj.as_array().ok()?;
    if arr.len() != 4 {
        return None;
    }
    let mut v = [0f32; 4];
    for (i, item) in arr.iter().enumerate() {
        let (_, item) = doc.dereference(item).ok()?;
        v[i] = item.as_float().ok().or_else(|| item.as_i64().ok().map(|n| n as f32))?;
    }
    Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

fn page_geometry(doc: &Document, page_id: ObjectId) -> PageGeometry {
    let media = inherited(doc, page_id, b"MediaBox").and_then(|o| rect_of(doc, o)).unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let crop = inherited(doc, page_id, b"CropBox").and_then(|o| rect_of(doc, o)).unwrap_or(media);
    // Comme PDFium : zone visible = CropBox ∩ MediaBox.
    let mut b = [crop[0].max(media[0]), crop[1].max(media[1]), crop[2].min(media[2]), crop[3].min(media[3])];
    if b[2] <= b[0] || b[3] <= b[1] {
        b = media;
    }
    let rotate = inherited(doc, page_id, b"Rotate").and_then(|o| o.as_i64().ok()).unwrap_or(0);
    let rotation = ((rotate % 360) + 360) % 360 / 90 * 90;
    PageGeometry { x0: b[0], y0: b[1], width: b[2] - b[0], height: b[3] - b[1], rotation }
}

// ---------------------------------------------------------------------------
// Écriture du contenu
// ---------------------------------------------------------------------------

/// Ressources partagées par tout le document (polices, transparences, images).
struct Stamp<'a> {
    source: &'a dyn FontSource,
    fonts: HashMap<&'static str, ObjectId>,
    embedded: HashMap<String, EmbeddedFont>,
    gstates: HashMap<u32, ObjectId>,
    images: HashMap<(PathBuf, bool), (ObjectId, Vec<Op>)>,
    replaced: u32,
}

/// Police livrée avec l'application, intégrée au PDF (sous-ensemble).
struct EmbeddedFont {
    /// Objet `Type0` réservé dès la première utilisation, écrit à la fin.
    id: ObjectId,
    file: String,
    data: Vec<u8>,
    bold: bool,
    italic: bool,
    remapper: GlyphRemapper,
    /// Glyphe d'origine de chaque glyphe du sous-ensemble (indice = nouveau numéro).
    order: Vec<u16>,
    /// Caractère représenté par chaque glyphe du sous-ensemble (copier-coller, recherche).
    unicode: HashMap<u16, char>,
}

#[derive(Clone, Copy)]
enum Kind {
    Font,
    Gs,
    XObject,
}

/// Noms de ressources utilisés par une page.
struct PageNames {
    taken: HashSet<Vec<u8>>,
    font: HashMap<ObjectId, String>,
    gs: HashMap<ObjectId, String>,
    xobject: HashMap<ObjectId, String>,
    counter: u32,
}

impl PageNames {
    fn name(&mut self, prefix: &str) -> String {
        loop {
            self.counter += 1;
            let candidate = format!("Hst{prefix}{}", self.counter);
            if self.taken.insert(candidate.as_bytes().to_vec()) {
                return candidate;
            }
        }
    }

    /// Nom de la ressource `id` dans la page (créé au premier usage).
    fn of(&mut self, kind: Kind, id: ObjectId) -> String {
        let (map, prefix) = match kind {
            Kind::Font => (&self.font, "F"),
            Kind::Gs => (&self.gs, "GS"),
            Kind::XObject => (&self.xobject, "Im"),
        };
        if let Some(n) = map.get(&id) {
            return n.clone();
        }
        let n = self.name(prefix);
        match kind {
            Kind::Font => self.font.insert(id, n.clone()),
            Kind::Gs => self.gs.insert(id, n.clone()),
            Kind::XObject => self.xobject.insert(id, n.clone()),
        };
        n
    }
}

const KAPPA: f32 = 0.552_284_8;

impl<'a> Stamp<'a> {
    fn new(source: &'a dyn FontSource) -> Self {
        Self {
            source,
            fonts: HashMap::new(),
            embedded: HashMap::new(),
            gstates: HashMap::new(),
            images: HashMap::new(),
            replaced: 0,
        }
    }

    fn edit_page(&mut self, doc: &mut Document, page_id: ObjectId, objects: &[EditObject]) -> Result<()> {
        let geom = page_geometry(doc, page_id);
        let height = geom.display_height();
        let mut resources = resolved_resources(doc, page_id);
        let mut names = PageNames {
            taken: existing_names(doc, &resources),
            font: HashMap::new(),
            gs: HashMap::new(),
            xobject: HashMap::new(),
            counter: 0,
        };

        let m = geom.to_user_space();
        let mut c = String::from("Q\nq\n");
        let _ = writeln!(c, "{} {} {} {} {} {} cm", num(m.a), num(m.b), num(m.c), num(m.d), num(m.e), num(m.f));
        for obj in objects {
            self.draw(doc, obj, height, &mut names, &mut c)?;
        }
        c.push_str("Q\n");

        // Ressources : copie propre à la page, complétée des nôtres.
        add_entries(doc, &mut resources, b"Font", &names.font);
        add_entries(doc, &mut resources, b"ExtGState", &names.gs);
        add_entries(doc, &mut resources, b"XObject", &names.xobject);

        // Contenu : [q] + contenu d'origine + [Q + nos éléments].
        let q_id = doc.add_object(Stream::new(dictionary! {}, b"q\n".to_vec()));
        let ours = doc.add_object(Stream::new(dictionary! {}, c.into_bytes()));
        let page = doc.get_dictionary(page_id)?;
        let mut contents = vec![Object::Reference(q_id)];
        match page.get(b"Contents") {
            Ok(Object::Reference(id)) => {
                // Une référence peut désigner un tableau de flux.
                match doc.get_object(*id) {
                    Ok(Object::Array(items)) => contents.extend(items.iter().cloned()),
                    _ => contents.push(Object::Reference(*id)),
                }
            }
            Ok(Object::Array(items)) => contents.extend(items.iter().cloned()),
            _ => {}
        }
        contents.push(Object::Reference(ours));
        let page = doc.get_dictionary_mut(page_id)?;
        page.set("Contents", Object::Array(contents));
        page.set("Resources", Object::Dictionary(resources));
        Ok(())
    }

    fn draw(
        &mut self,
        doc: &mut Document,
        obj: &EditObject,
        height: f32,
        names: &mut PageNames,
        c: &mut String,
    ) -> Result<()> {
        let flip = |y: f32| height - y;
        c.push_str("q\n");
        match obj {
            EditObject::Text { x, y, size, font, bold, italic, color: col, opacity, lines } => {
                self.opacity(doc, *opacity, names, c);
                let (r, g, b) = color(col)?;
                // Police livrée avec l'application, ou police standard du PDF.
                let face = fonts::family(font).map(|f| f.face(*bold, *italic));
                let font_id = match &face {
                    Some(face) => self.embedded_font(doc, face)?,
                    None => self.font(doc, base_font(font, *bold, *italic)),
                };
                let fname = names.of(Kind::Font, font_id);
                let _ = writeln!(c, "{} {} {} rg\nBT\n/{fname} {} Tf", num(r), num(g), num(b), num(*size));
                for (i, line) in lines.iter().enumerate() {
                    let baseline = y + (i as f32 * TEXT_LINE_HEIGHT + TEXT_BASELINE) * size;
                    let shown = match &face {
                        Some(face) => self.encode_embedded(&face.file, line),
                        None => {
                            let (encoded, replaced) = encode_win_ansi(line);
                            self.replaced += replaced;
                            pdf_string(&encoded)
                        }
                    };
                    let _ = writeln!(c, "1 0 0 1 {} {} Tm\n{shown} Tj", num(*x), num(flip(baseline)));
                }
                c.push_str("ET\n");
            }
            EditObject::Rect { x, y, w, h, stroke, fill, stroke_width, opacity } => {
                self.opacity(doc, *opacity, names, c);
                let paint = paint_setup(stroke.as_deref(), fill.as_deref(), *stroke_width, c)?;
                let _ = writeln!(c, "{} {} {} {} re {paint}", num(*x), num(flip(y + h)), num(*w), num(*h));
            }
            EditObject::Ellipse { x, y, w, h, stroke, fill, stroke_width, opacity } => {
                self.opacity(doc, *opacity, names, c);
                let paint = paint_setup(stroke.as_deref(), fill.as_deref(), *stroke_width, c)?;
                let (rx, ry) = (w / 2.0, h / 2.0);
                let (cx, cy) = (x + rx, flip(y + ry));
                let (kx, ky) = (rx * KAPPA, ry * KAPPA);
                let p = |v: f32| num(v);
                let _ = writeln!(
                    c,
                    "{} {} m\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\nh {paint}",
                    p(cx + rx),
                    p(cy),
                    p(cx + rx),
                    p(cy + ky),
                    p(cx + kx),
                    p(cy + ry),
                    p(cx),
                    p(cy + ry),
                    p(cx - kx),
                    p(cy + ry),
                    p(cx - rx),
                    p(cy + ky),
                    p(cx - rx),
                    p(cy),
                    p(cx - rx),
                    p(cy - ky),
                    p(cx - kx),
                    p(cy - ry),
                    p(cx),
                    p(cy - ry),
                    p(cx + kx),
                    p(cy - ry),
                    p(cx + rx),
                    p(cy - ky),
                    p(cx + rx),
                    p(cy),
                );
            }
            EditObject::Line { x1, y1, x2, y2, color: col, width, opacity, arrow } => {
                self.opacity(doc, *opacity, names, c);
                let (r, g, b) = color(col)?;
                let _ = writeln!(
                    c,
                    "{} {} {} RG {} {} {} rg {} w 1 J 1 j",
                    num(r),
                    num(g),
                    num(b),
                    num(r),
                    num(g),
                    num(b),
                    num(*width)
                );
                let (ax, ay, bx, by) = (*x1, flip(*y1), *x2, flip(*y2));
                if *arrow {
                    let head = arrow_head(ax, ay, bx, by, *width);
                    let _ = writeln!(c, "{} {} m {} {} l S", num(ax), num(ay), num(head.base.0), num(head.base.1));
                    let _ = writeln!(
                        c,
                        "{} {} m {} {} l {} {} l h f",
                        num(bx),
                        num(by),
                        num(head.left.0),
                        num(head.left.1),
                        num(head.right.0),
                        num(head.right.1)
                    );
                } else {
                    let _ = writeln!(c, "{} {} m {} {} l S", num(ax), num(ay), num(bx), num(by));
                }
            }
            EditObject::Ink { points, color: col, width, opacity } => {
                self.opacity(doc, *opacity, names, c);
                let (r, g, b) = color(col)?;
                let _ = writeln!(c, "{} {} {} RG {} w 1 J 1 j", num(r), num(g), num(b), num(*width));
                let mut first = true;
                for [px, py] in points {
                    let _ = write!(c, "{} {} {} ", num(*px), num(flip(*py)), if first { "m" } else { "l" });
                    first = false;
                }
                if points.len() == 1 {
                    let [px, py] = points[0];
                    let _ = write!(c, "{} {} l ", num(px), num(flip(py)));
                }
                c.push_str("S\n");
            }
            EditObject::Image { x, y, w, h, path, opacity, rotation, flip_h, grayscale } => {
                self.opacity(doc, *opacity, names, c);
                let (image_id, mut ops) = self.image(doc, path, *grayscale)?;
                // Orientation EXIF, puis miroir, puis rotation choisis dans l'éditeur.
                if *flip_h {
                    ops.push(Op::FlipH);
                }
                ops.extend(std::iter::repeat_n(Op::Rotate90, usize::from(rotation / 90 % 4)));
                let xname = names.of(Kind::XObject, image_id);
                // `(x, y, w, h)` est le cadre affiché, rotation comprise.
                let m = place_in_box(&ops, *x, flip(y + h), *w, *h);
                let _ = writeln!(
                    c,
                    "{} {} {} {} {} {} cm /{xname} Do",
                    num(m.a),
                    num(m.b),
                    num(m.c),
                    num(m.d),
                    num(m.e),
                    num(m.f)
                );
            }
        }
        c.push_str("Q\n");
        Ok(())
    }

    fn opacity(&mut self, doc: &mut Document, opacity: f32, names: &mut PageNames, c: &mut String) {
        if opacity >= 0.999 {
            return;
        }
        let key = (opacity.clamp(0.0, 1.0) * 1000.0).round() as u32;
        let id = *self.gstates.entry(key).or_insert_with(|| {
            let a = key as f32 / 1000.0;
            doc.add_object(dictionary! { "Type" => "ExtGState", "CA" => a, "ca" => a })
        });
        let name = names.of(Kind::Gs, id);
        let _ = writeln!(c, "/{name} gs");
    }

    fn font(&mut self, doc: &mut Document, base: &'static str) -> ObjectId {
        *self.fonts.entry(base).or_insert_with(|| {
            doc.add_object(dictionary! {
                "Type" => "Font",
                "Subtype" => "Type1",
                "BaseFont" => base,
                "Encoding" => "WinAnsiEncoding",
            })
        })
    }

    /// Image intégrée une seule fois par fichier (et par variante couleur / gris).
    fn image(&mut self, doc: &mut Document, path: &std::path::Path, grayscale: bool) -> Result<(ObjectId, Vec<Op>)> {
        let key = (path.to_path_buf(), grayscale);
        if let Some(found) = self.images.get(&key) {
            return Ok(found.clone());
        }
        let img = load_image(path, grayscale).map_err(|e| e.in_file(path))?;
        let ops = orientation_ops(img.orientation, 0);
        let mut stream = img.stream;
        if let Some(mask) = img.smask {
            let mask_id = doc.add_object(mask);
            stream.dict.set("SMask", mask_id);
        }
        let id = doc.add_object(stream);
        self.images.insert(key, (id, ops.clone()));
        Ok((id, ops))
    }

    /// Police livrée avec l'application : l'objet est réservé maintenant,
    /// écrit par `finish` une fois connus tous les glyphes utilisés.
    fn embedded_font(&mut self, doc: &mut Document, face: &fonts::Face) -> Result<ObjectId> {
        if let Some(found) = self.embedded.get(&face.file) {
            return Ok(found.id);
        }
        let data = self
            .source
            .load(&face.file)
            .ok_or_else(|| HestiaError::Internal(format!("police {} introuvable", face.file)))?;
        ttf_parser::Face::parse(&data, 0).map_err(|e| font_error(&face.file, e))?;
        let id = doc.new_object_id();
        self.embedded.insert(
            face.file.clone(),
            EmbeddedFont {
                id,
                file: face.file.clone(),
                data,
                bold: face.bold,
                italic: face.italic,
                remapper: GlyphRemapper::new(),
                order: vec![0],
                unicode: HashMap::new(),
            },
        );
        Ok(id)
    }

    /// Texte en numéros de glyphes du sous-ensemble (`<0001 0002…>`). Les
    /// caractères absents de la police deviennent « ? » et sont comptés.
    fn encode_embedded(&mut self, file: &str, text: &str) -> String {
        let mut hex = String::from("<");
        let Some(EmbeddedFont { data, remapper, order, unicode, .. }) = self.embedded.get_mut(file) else {
            return "<>".into();
        };
        let Ok(face) = ttf_parser::Face::parse(data, 0) else {
            return "<>".into();
        };
        let mut replaced = 0;
        for ch in text.chars() {
            let ch = if ch == '\t' { ' ' } else { ch };
            if ch.is_control() {
                continue;
            }
            let (gid, shown) = match face.glyph_index(ch) {
                Some(GlyphId(g)) if g != 0 => (g, ch),
                _ => {
                    replaced += 1;
                    (face.glyph_index('?').map_or(0, |g| g.0), '?')
                }
            };
            let new = remapper.remap(gid);
            if usize::from(new) == order.len() {
                order.push(gid);
            }
            if gid != 0 {
                unicode.entry(new).or_insert(shown);
            }
            let _ = write!(hex, "{new:04X}");
        }
        hex.push('>');
        self.replaced += replaced;
        hex
    }

    /// Écrit les polices intégrées (sous-ensembles, largeurs, table Unicode).
    fn finish(&mut self, doc: &mut Document) -> Result<()> {
        for font in self.embedded.values() {
            let dict = embedded_font_dict(doc, font)?;
            doc.objects.insert(font.id, Object::Dictionary(dict));
        }
        Ok(())
    }
}

fn font_error(file: &str, e: impl std::fmt::Display) -> HestiaError {
    HestiaError::Internal(format!("police {file} : {e}"))
}

/// Police `Type0` / `CIDFontType2` (Identity-H) avec le sous-ensemble des glyphes utilisés.
fn embedded_font_dict(doc: &mut Document, font: &EmbeddedFont) -> Result<Dictionary> {
    let face = ttf_parser::Face::parse(&font.data, 0).map_err(|e| font_error(&font.file, e))?;
    let subset = subsetter::subset(&font.data, 0, &font.remapper).map_err(|e| font_error(&font.file, e))?;
    let upem = f32::from(face.units_per_em().max(1));
    let k = |v: f32| Object::Integer((v * 1000.0 / upem).round() as i64);

    let name = format!("{}+{}", subset_tag(font), postscript_name(&face, &font.file));
    let widths: Vec<Object> =
        font.order.iter().map(|&g| k(f32::from(face.glyph_hor_advance(GlyphId(g)).unwrap_or(0)))).collect();
    let bbox = face.global_bounding_box();
    let mut flags = 32; // Non symbolique.
    if face.is_monospaced() {
        flags |= 1;
    }
    if font.italic {
        flags |= 64;
    }

    let file_id = doc.add_object(Stream::new(dictionary! { "Length1" => subset.len() as i64 }, subset));
    let descriptor = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => Object::Name(name.clone().into_bytes()),
        "Flags" => flags,
        "FontBBox" => vec![
            k(f32::from(bbox.x_min)),
            k(f32::from(bbox.y_min)),
            k(f32::from(bbox.x_max)),
            k(f32::from(bbox.y_max)),
        ],
        "ItalicAngle" => face.italic_angle(),
        "Ascent" => k(f32::from(face.ascender())),
        "Descent" => k(f32::from(face.descender())),
        "CapHeight" => k(f32::from(face.capital_height().unwrap_or(face.ascender()))),
        "StemV" => if font.bold { 120 } else { 80 },
        "FontFile2" => file_id,
    });
    let cid_font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => Object::Name(name.clone().into_bytes()),
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0,
        },
        "FontDescriptor" => descriptor,
        // Les glyphes du sous-ensemble sont numérotés à partir de 0, sans trou.
        "W" => vec![Object::Integer(0), Object::Array(widths)],
        "CIDToGIDMap" => "Identity",
    });
    let to_unicode = doc.add_object(Stream::new(dictionary! {}, to_unicode_cmap(&font.unicode).into_bytes()));
    Ok(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => Object::Name(name.into_bytes()),
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![Object::Reference(cid_font)],
        "ToUnicode" => to_unicode,
    })
}

/// Nom PostScript de la police (table `name`), sinon le nom du fichier.
fn postscript_name(face: &ttf_parser::Face, file: &str) -> String {
    let from_table = face
        .names()
        .into_iter()
        .filter(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
        .find_map(|n| n.to_string());
    let raw = from_table.unwrap_or_else(|| file.trim_end_matches(".ttf").to_string());
    let clean: String = raw.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(60).collect();
    if clean.is_empty() { "HestiaFont".into() } else { clean }
}

/// Préfixe de sous-ensemble (six majuscules), stable pour un même contenu.
fn subset_tag(font: &EmbeddedFont) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in font.file.bytes().chain(font.order.iter().flat_map(|g| g.to_be_bytes())) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    (0..6)
        .map(|i| {
            let letter = (hash >> (i * 8)) % 26;
            char::from(b'A' + letter as u8)
        })
        .collect()
}

/// Table de correspondance glyphe → Unicode (CMap `ToUnicode`).
fn to_unicode_cmap(unicode: &HashMap<u16, char>) -> String {
    let mut entries: Vec<(u16, char)> = unicode.iter().map(|(&g, &c)| (g, c)).collect();
    entries.sort_unstable();
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    for chunk in entries.chunks(100) {
        let _ = writeln!(s, "{} beginbfchar", chunk.len());
        for (gid, ch) in chunk {
            let mut utf16 = [0u16; 2];
            let units: String = ch.encode_utf16(&mut utf16).iter().map(|u| format!("{u:04X}")).collect();
            let _ = writeln!(s, "<{gid:04X}> <{units}>");
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s
}

/// Couleurs et épaisseur du contour/remplissage ; renvoie l'opérateur de peinture.
fn paint_setup(stroke: Option<&str>, fill: Option<&str>, width: f32, c: &mut String) -> Result<&'static str> {
    let stroke = stroke.filter(|_| width > 0.0);
    if let Some(s) = stroke {
        let (r, g, b) = color(s)?;
        let _ = writeln!(c, "{} {} {} RG {} w 0 J 0 j", num(r), num(g), num(b), num(width));
    }
    if let Some(f) = fill {
        let (r, g, b) = color(f)?;
        let _ = writeln!(c, "{} {} {} rg", num(r), num(g), num(b));
    }
    Ok(match (stroke.is_some(), fill.is_some()) {
        (true, true) => "B",
        (true, false) => "S",
        (false, true) => "f",
        (false, false) => "n",
    })
}

/// Pointe de flèche : triangle plein au bout de la ligne.
/// Même géométrie que dans l'interface (src/views/editor/model.ts).
pub struct ArrowHead {
    pub base: (f32, f32),
    pub left: (f32, f32),
    pub right: (f32, f32),
}

pub fn arrow_head(ax: f32, ay: f32, bx: f32, by: f32, width: f32) -> ArrowHead {
    let (dx, dy) = (bx - ax, by - ay);
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    let (ux, uy) = (dx / len, dy / len);
    let head_len = (width * 4.0).max(8.0).min(len);
    let half = head_len * 0.55;
    let base = (bx - ux * head_len, by - uy * head_len);
    ArrowHead { base, left: (base.0 - uy * half, base.1 + ux * half), right: (base.0 + uy * half, base.1 - ux * half) }
}

/// Police standard du PDF (« helvetica », « times », « courier »).
fn base_font(family: &str, bold: bool, italic: bool) -> &'static str {
    match (family, bold, italic) {
        ("times", false, false) => "Times-Roman",
        ("times", true, false) => "Times-Bold",
        ("times", false, true) => "Times-Italic",
        ("times", true, true) => "Times-BoldItalic",
        ("courier", false, false) => "Courier",
        ("courier", true, false) => "Courier-Bold",
        ("courier", false, true) => "Courier-Oblique",
        ("courier", true, true) => "Courier-BoldOblique",
        (_, false, false) => "Helvetica",
        (_, true, false) => "Helvetica-Bold",
        (_, false, true) => "Helvetica-Oblique",
        (_, true, true) => "Helvetica-BoldOblique",
    }
}

/// Encode du texte en WinAnsi (polices standard). Les caractères absents
/// (emoji, alphabets non latins…) deviennent « ? » et sont comptés.
pub fn encode_win_ansi(text: &str) -> (Vec<u8>, u32) {
    let mut out = Vec::with_capacity(text.len());
    let mut replaced = 0;
    for ch in text.chars() {
        let byte = match ch {
            '\t' => Some(b' '),
            ' '..='~' => Some(ch as u8),
            '\u{A0}'..='\u{FF}' => Some(ch as u32 as u8),
            '€' => Some(0x80),
            '‚' => Some(0x82),
            'ƒ' => Some(0x83),
            '„' => Some(0x84),
            '…' => Some(0x85),
            '†' => Some(0x86),
            '‡' => Some(0x87),
            'ˆ' => Some(0x88),
            '‰' => Some(0x89),
            'Š' => Some(0x8A),
            '‹' => Some(0x8B),
            'Œ' => Some(0x8C),
            'Ž' => Some(0x8E),
            '‘' => Some(0x91),
            '’' => Some(0x92),
            '“' => Some(0x93),
            '”' => Some(0x94),
            '•' => Some(0x95),
            '–' => Some(0x96),
            '—' => Some(0x97),
            '˜' => Some(0x98),
            '™' => Some(0x99),
            'š' => Some(0x9A),
            '›' => Some(0x9B),
            'œ' => Some(0x9C),
            'ž' => Some(0x9E),
            'Ÿ' => Some(0x9F),
            // Espaces typographiques courantes (fines, insécables étroites).
            '\u{2009}' | '\u{202F}' | '\u{2007}' => Some(0xA0),
            _ => None,
        };
        match byte {
            Some(b) => out.push(b),
            None => {
                out.push(b'?');
                replaced += 1;
            }
        }
    }
    (out, replaced)
}

/// Chaîne littérale PDF `(...)` correctement échappée.
fn pdf_string(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() + 2);
    s.push('(');
    for &b in bytes {
        match b {
            b'(' | b')' | b'\\' => {
                s.push('\\');
                s.push(b as char);
            }
            0x20..=0x7E => s.push(b as char),
            _ => {
                let _ = write!(s, "\\{b:03o}");
            }
        }
    }
    s.push(')');
    s
}

/// Copie du dictionnaire de ressources de la page (hérité si besoin).
fn resolved_resources(doc: &Document, page_id: ObjectId) -> Dictionary {
    inherited(doc, page_id, b"Resources").and_then(|o| o.as_dict().ok().cloned()).unwrap_or_default()
}

/// Noms déjà utilisés dans les sous-dictionnaires de ressources.
fn existing_names(doc: &Document, resources: &Dictionary) -> HashSet<Vec<u8>> {
    let mut taken = HashSet::new();
    for key in [b"Font".as_slice(), b"ExtGState", b"XObject", b"ColorSpace", b"Pattern", b"Shading", b"Properties"] {
        if let Ok(sub) = resources.get(key)
            && let Ok((_, sub)) = doc.dereference(sub)
            && let Ok(dict) = sub.as_dict()
        {
            taken.extend(dict.iter().map(|(k, _)| k.clone()));
        }
    }
    taken
}

/// Ajoute nos entrées dans le sous-dictionnaire `key` (copié s'il était partagé).
fn add_entries(doc: &Document, resources: &mut Dictionary, key: &[u8], entries: &HashMap<ObjectId, String>) {
    if entries.is_empty() {
        return;
    }
    let mut sub = resources
        .get(key)
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok().cloned())
        .unwrap_or_default();
    for (id, name) in entries {
        sub.set(name.as_bytes().to_vec(), Object::Reference(*id));
    }
    resources.set(key.to_vec(), Object::Dictionary(sub));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_ansi_covers_french() {
        let (bytes, replaced) = encode_win_ansi("Été à Noël : 10 € — « œuvre »");
        assert_eq!(replaced, 0);
        assert_eq!(bytes[0], 0xC9); // É
        assert!(bytes.contains(&0x80)); // €
        assert!(bytes.contains(&0x97)); // —
        assert!(bytes.contains(&0x9C)); // œ
        let (_, replaced) = encode_win_ansi("Bonjour 👋 世界");
        assert_eq!(replaced, 3);
    }

    #[test]
    fn pdf_strings_are_escaped() {
        assert_eq!(pdf_string(b"a(b)c\\"), "(a\\(b\\)c\\\\)");
        assert_eq!(pdf_string(&[0xC9, b'\n']), "(\\311\\012)");
    }

    #[test]
    fn colors_are_parsed_strictly() {
        assert_eq!(color("#ff0000").ok(), Some((1.0, 0.0, 0.0)));
        for bad in ["ff0000", "#ff00", "#gg0000", "", "#ff0000ff"] {
            assert!(color(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rotation_matrices_map_display_corners() {
        let g = |rotation| PageGeometry { x0: 10.0, y0: 20.0, width: 600.0, height: 800.0, rotation };
        // Coin bas-gauche affiché → coin attendu dans le repère PDF.
        assert_eq!(g(0).to_user_space().apply(0.0, 0.0), (10.0, 20.0));
        // 90° : la page affichée fait 800 × 600 ; son coin bas-gauche est le coin bas-droit de la page PDF.
        assert_eq!(g(90).to_user_space().apply(0.0, 0.0), (610.0, 20.0));
        assert_eq!(g(90).display_height(), 600.0);
        assert_eq!(g(180).to_user_space().apply(0.0, 0.0), (610.0, 820.0));
        assert_eq!(g(270).to_user_space().apply(0.0, 0.0), (10.0, 820.0));
    }

    #[test]
    fn absurd_values_are_rejected() {
        let doc = |obj: EditObject| EditDocument { pages: vec![PageEdits { page: 0, objects: vec![obj] }] };
        let rect = |w: f32| EditObject::Rect {
            x: 0.0,
            y: 0.0,
            w,
            h: 10.0,
            stroke: None,
            fill: Some("#000000".into()),
            stroke_width: 1.0,
            opacity: 1.0,
        };
        assert!(validate(&doc(rect(10.0))).is_ok());
        assert!(validate(&doc(rect(f32::NAN))).is_err());
        assert!(validate(&doc(rect(-1.0))).is_err());
        assert!(
            validate(&doc(EditObject::Ink { points: vec![], color: "#000000".into(), width: 1.0, opacity: 1.0 }))
                .is_err()
        );
    }
}
