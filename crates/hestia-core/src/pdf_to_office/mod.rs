//! PDF → Word, PowerPoint, Excel, sans Office : Hestia lit la mise en page
//! du PDF (texte, positions, polices, tailles, gras/italique, couleurs,
//! images) et écrit lui-même un fichier .docx, .pptx ou .xlsx.
//!
//! - Word : paragraphes dans l'ordre de lecture, styles conservés, images.
//! - PowerPoint : une diapositive par page ; la page sans son texte en fond
//!   (rendu fidèle), et le texte par-dessus dans des zones modifiables.
//! - Excel : une feuille par page ; les colonnes sont retrouvées d'après
//!   l'alignement du texte, les nombres deviennent de vrais nombres.
//!
//! Comme toute conversion depuis un PDF, le résultat est une reconstruction :
//! les documents simples (courriers, rapports, factures, tableaux) sortent
//! bien ; les mises en page complexes ou les scans (images de texte) moins.

mod docx;
mod pptx;
mod xlsx;
pub(crate) mod zip;

use std::io::Write as _;
use std::path::Path;

use image::DynamicImage;
use pdfium_render::prelude::*;
use serde::Deserialize;

use crate::engine::{EngineCtx, PdfEngine, PdfInput};
use crate::error::{HestiaError, Result};
use crate::imaging::encode_jpeg;
use crate::job::Job;
use crate::output::{OutputFile, ensure_not_input, write_atomic};

/// Format de sortie.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OfficeFormat {
    Docx,
    Pptx,
    Xlsx,
}

impl OfficeFormat {
    pub fn extension(self) -> &'static str {
        match self {
            OfficeFormat::Docx => "docx",
            OfficeFormat::Pptx => "pptx",
            OfficeFormat::Xlsx => "xlsx",
        }
    }
}

/// Résultat d'une conversion depuis un PDF.
#[derive(Clone, Debug)]
pub struct OfficeReport {
    pub output: OutputFile,
    /// Aucun texte n'a été trouvé (PDF scanné) : le résultat ne contient que des images.
    pub no_text: bool,
}

/// Convertit un PDF en document Word, PowerPoint ou Excel.
pub fn pdf_to_office(
    engine: &PdfEngine,
    input: PdfInput,
    format: OfficeFormat,
    output: &Path,
    job: &Job,
) -> Result<OfficeReport> {
    ensure_not_input(output, [input.path.as_path()])?;
    let page_count = engine.run({
        let input = input.clone();
        move |ctx| Ok(ctx.load(&input)?.pages().len() as u32)
    })?;
    if page_count == 0 {
        return Err(HestiaError::InvalidPdf("le document ne contient aucune page".into()));
    }
    let with_background = format == OfficeFormat::Pptx;
    let with_images = format == OfficeFormat::Docx;
    let mut pages = Vec::with_capacity(page_count as usize);
    for index in 0..page_count {
        job.check()?;
        let input = input.clone();
        pages.push(engine.run(move |ctx| read_page(ctx, &input, index, with_images, with_background))?);
        job.report(index + 1, page_count + 1);
    }
    job.check()?;
    let no_text = pages.iter().all(|p| p.lines.is_empty());
    let bytes = match format {
        OfficeFormat::Docx => docx::write(&pages)?,
        OfficeFormat::Pptx => pptx::write(&pages)?,
        OfficeFormat::Xlsx => xlsx::write(&pages)?,
    };
    job.check()?;
    let out = write_atomic(output, |f| f.write_all(&bytes).map_err(Into::into))?;
    job.report(page_count + 1, page_count + 1);
    Ok(OfficeReport { output: out, no_text })
}

// ---------------------------------------------------------------------------
// Mise en page lue dans le PDF (points, origine en haut à gauche)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Style {
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: [u8; 3],
}

#[derive(Clone, Debug)]
pub(crate) struct Run {
    pub text: String,
    pub style: Style,
}

/// Morceau de ligne séparé des autres par un grand espace (colonnes, cellules).
#[derive(Clone, Debug)]
pub(crate) struct Span {
    pub x0: f32,
    pub x1: f32,
    pub runs: Vec<Run>,
}

impl Span {
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }

    pub fn bold(&self) -> bool {
        self.runs.iter().all(|r| r.style.bold || r.text.trim().is_empty())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Line {
    /// Ligne de base, depuis le haut de la page.
    pub baseline: f32,
    /// Taille de police dominante.
    pub size: f32,
    pub spans: Vec<Span>,
}

impl Line {
    pub fn x0(&self) -> f32 {
        self.spans.first().map_or(0.0, |s| s.x0)
    }

    pub fn x1(&self) -> f32 {
        self.spans.last().map_or(0.0, |s| s.x1)
    }

    pub fn top(&self) -> f32 {
        self.baseline - self.size * 0.9
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PageImage {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub data: Vec<u8>,
    pub png: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PageLayout {
    pub width: f32,
    pub height: f32,
    pub lines: Vec<Line>,
    pub images: Vec<PageImage>,
    /// Rendu JPEG de la page sans son texte (fond des diapositives).
    pub background: Option<Vec<u8>>,
}

/// Caractère lu dans le PDF.
struct Glyph {
    ch: char,
    x0: f32,
    x1: f32,
    baseline: f32,
    style: Style,
}

fn read_page(ctx: &EngineCtx, input: &PdfInput, index: u32, images: bool, background: bool) -> Result<PageLayout> {
    let doc = ctx.load(input)?;
    let page = doc.pages().get(index as PdfPageIndex)?;
    let (width, height) = (page.width().value, page.height().value);

    let mut glyphs = Vec::new();
    let text = page.text()?;
    for c in text.chars().iter() {
        let Some(ch) = c.unicode_char() else { continue };
        if ch.is_control() && ch != '\t' {
            continue;
        }
        // Texte penché ou vertical : hors du flux normal, ignoré.
        if c.angle_degrees().map(|a| a.abs() > 1.0 && (a - 360.0).abs() > 1.0).unwrap_or(false) {
            continue;
        }
        let Ok(bounds) = c.loose_bounds() else { continue };
        let Ok((_, oy)) = c.origin() else { continue };
        let mut size = c.scaled_font_size().value;
        if !(1.0..=500.0).contains(&size) {
            size = bounds.height().value.clamp(4.0, 200.0);
        }
        let name = c.font_name();
        let lower = name.to_ascii_lowercase();
        let bold = c.font_weight().is_some_and(|w| {
            matches!(
                w,
                PdfFontWeight::Weight600
                    | PdfFontWeight::Weight700Bold
                    | PdfFontWeight::Weight800
                    | PdfFontWeight::Weight900
            ) || matches!(w, PdfFontWeight::Custom(v) if v >= 600)
        }) || lower.contains("bold")
            || lower.contains("black")
            || lower.contains("heavy")
            || c.font_is_bold_reenforced();
        let italic = c.font_is_italic() || lower.contains("italic") || lower.contains("oblique");
        let color = c.fill_color().map(|k| [k.red(), k.green(), k.blue()]).unwrap_or([0, 0, 0]);
        glyphs.push(Glyph {
            ch: if ch == '\t' { ' ' } else { ch },
            x0: bounds.left().value,
            x1: bounds.right().value,
            baseline: height - oy.value,
            style: Style { font: clean_font_name(&name), size: (size * 2.0).round() / 2.0, bold, italic, color },
        });
    }
    drop(text);
    let lines = build_lines(glyphs);

    let mut found = Vec::new();
    if images {
        for object in page.objects().iter() {
            if found.len() >= 200 {
                break;
            }
            let Some(img) = object.as_image_object() else { continue };
            let Ok(quad) = object.bounds() else { continue };
            let r = quad.to_rect();
            let (w, h) = (r.width().value, r.height().value);
            // Les très petites images (puces, filets) sont ignorées.
            if w < 12.0 || h < 12.0 {
                continue;
            }
            let Ok(raw) = img.get_processed_image(&doc).or_else(|_| img.get_raw_image()) else { continue };
            if let Some((data, png)) = encode_image(raw) {
                found.push(PageImage { x: r.left().value, y: height - r.top().value, w, h, data, png });
            }
        }
    }
    drop(page);

    let background = if background { Some(render_without_text(ctx, input, index)?) } else { None };
    Ok(PageLayout { width, height, lines, images: found, background })
}

/// Page rendue sans son texte : le texte sera ajouté, modifiable, par-dessus.
fn render_without_text(ctx: &EngineCtx, input: &PdfInput, index: u32) -> Result<Vec<u8>> {
    let doc = ctx.load(input)?;
    let mut page = doc.pages().get(index as PdfPageIndex)?;
    page.set_content_regeneration_strategy(PdfPageContentRegenerationStrategy::Manual);
    let count = page.objects().len();
    for i in (0..count).rev() {
        let is_text = page.objects().get(i).map(|o| o.object_type() == PdfPageObjectType::Text).unwrap_or(false);
        if is_text {
            let _ = page.objects_mut().remove_object_at_index(i);
        }
    }
    let target = ((page.width().value / 72.0 * 150.0) as i32).clamp(400, 2400);
    let config = PdfRenderConfig::new()
        .set_target_width(target)
        .set_maximum_height(target * 4)
        .render_form_data(true)
        .render_annotations(true);
    let image = page.render_with_config(&config)?.as_image()?;
    encode_jpeg(&image, 85)
}

/// Image d'une page : JPEG pour les photos, PNG pour le reste (transparence, aplats).
fn encode_image(img: DynamicImage) -> Option<(Vec<u8>, bool)> {
    let img = if img.width().max(img.height()) > 2000 { img.thumbnail(2000, 2000) } else { img };
    if img.width() == 0 || img.height() == 0 {
        return None;
    }
    if img.color().has_alpha() || img.width() * img.height() < 160_000 {
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).ok()?;
        Some((out.into_inner(), true))
    } else {
        encode_jpeg(&img, 88).ok().map(|d| (d, false))
    }
}

/// « ABCDEF+TimesNewRomanPS-BoldMT » → « Times New Roman ».
fn clean_font_name(raw: &str) -> String {
    let name = raw.split_once('+').map_or(raw, |(tag, rest)| if tag.len() == 6 { rest } else { raw });
    let base = name.split([',', '-']).next().unwrap_or(name);
    let base = base.trim_end_matches("MT").trim_end_matches("PS").trim_end_matches("PSMT");
    let lower = base.to_ascii_lowercase();
    let known = [
        ("timesnewroman", "Times New Roman"),
        ("times", "Times New Roman"),
        ("arial", "Arial"),
        ("helvetica", "Arial"),
        ("couriernew", "Courier New"),
        ("courier", "Courier New"),
        ("calibri", "Calibri"),
        ("cambria", "Cambria"),
        ("georgia", "Georgia"),
        ("verdana", "Verdana"),
        ("tahoma", "Tahoma"),
        ("segoeui", "Segoe UI"),
        ("garamond", "Garamond"),
        ("trebuchet", "Trebuchet MS"),
        ("symbol", "Symbol"),
    ];
    for (key, value) in known {
        if lower.starts_with(key) {
            return value.to_string();
        }
    }
    // « OpenSans » → « Open Sans ».
    let mut out = String::new();
    for (i, ch) in base.chars().enumerate() {
        if i > 0 && ch.is_uppercase() && !out.ends_with(' ') {
            let prev = out.chars().last().unwrap_or(' ');
            if prev.is_lowercase() {
                out.push(' ');
            }
        }
        out.push(ch);
    }
    if out.trim().is_empty() { "Calibri".into() } else { out.trim().to_string() }
}

/// Regroupe les caractères en lignes (même ligne de base), puis en morceaux
/// séparés par de grands espaces, puis en suites de même style.
fn build_lines(mut glyphs: Vec<Glyph>) -> Vec<Line> {
    glyphs.sort_by(|a, b| a.baseline.total_cmp(&b.baseline));
    let mut rows: Vec<Vec<Glyph>> = Vec::new();
    for g in glyphs {
        match rows.last_mut() {
            Some(row) if (row[0].baseline - g.baseline).abs() <= row[0].style.size.max(g.style.size) * 0.35 => {
                row.push(g)
            }
            _ => rows.push(vec![g]),
        }
    }
    let mut lines = Vec::new();
    for mut row in rows {
        row.sort_by(|a, b| a.x0.total_cmp(&b.x0));
        // Doublons (texte en faux gras dessiné deux fois).
        row.dedup_by(|b, a| a.ch == b.ch && (a.x0 - b.x0).abs() < 0.5);
        let size = dominant_size(&row);
        let baseline = row.iter().map(|g| g.baseline).sum::<f32>() / row.len() as f32;
        let mut spans: Vec<Span> = Vec::new();
        let mut prev_x1 = f32::MIN;
        for g in row {
            let gap = g.x0 - prev_x1;
            let new_span = spans.is_empty() || gap > size * 1.6;
            if new_span {
                spans.push(Span { x0: g.x0, x1: g.x1, runs: Vec::new() });
            } else if gap > size * 0.18 && !last_char_is_space(&spans) && g.ch != ' ' {
                // Espace non écrit dans le PDF (mots placés un par un).
                push_char(spans.last_mut(), ' ', &g.style);
            }
            if let Some(span) = spans.last_mut() {
                span.x1 = span.x1.max(g.x1);
            }
            push_char(spans.last_mut(), g.ch, &g.style);
            prev_x1 = g.x1;
        }
        for span in &mut spans {
            trim_span(span);
        }
        spans.retain(|s| !s.runs.is_empty());
        if !spans.is_empty() {
            lines.push(Line { baseline, size, spans });
        }
    }
    lines
}

fn dominant_size(row: &[Glyph]) -> f32 {
    let mut sizes: Vec<(f32, usize)> = Vec::new();
    for g in row {
        match sizes.iter_mut().find(|(s, _)| (*s - g.style.size).abs() < 0.3) {
            Some(entry) => entry.1 += 1,
            None => sizes.push((g.style.size, 1)),
        }
    }
    sizes.into_iter().max_by_key(|(_, n)| *n).map_or(11.0, |(s, _)| s)
}

fn last_char_is_space(spans: &[Span]) -> bool {
    spans.last().and_then(|s| s.runs.last()).is_some_and(|r| r.text.ends_with(' '))
}

fn push_char(span: Option<&mut Span>, ch: char, style: &Style) {
    let Some(span) = span else { return };
    if ch == ' ' && (span.runs.is_empty() || last_char_is_space(std::slice::from_ref(span))) {
        return;
    }
    match span.runs.last_mut() {
        Some(run) if run.style == *style || ch == ' ' => run.text.push(ch),
        _ => span.runs.push(Run { text: ch.to_string(), style: style.clone() }),
    }
}

fn trim_span(span: &mut Span) {
    while let Some(last) = span.runs.last_mut() {
        let trimmed = last.text.trim_end().len();
        last.text.truncate(trimmed);
        if last.text.is_empty() {
            span.runs.pop();
        } else {
            break;
        }
    }
}

/// Points → EMU (unité des fichiers Office : 1 pt = 12 700 EMU).
pub(crate) fn emu(pt: f32) -> i64 {
    (pt * 12_700.0).round() as i64
}

/// Couleur hexadécimale « RRGGBB ».
pub(crate) fn hex(c: [u8; 3]) -> String {
    format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_names_are_cleaned() {
        assert_eq!(clean_font_name("ABCDEF+TimesNewRomanPS-BoldMT"), "Times New Roman");
        assert_eq!(clean_font_name("ArialMT"), "Arial");
        assert_eq!(clean_font_name("QWERTY+OpenSans-Regular"), "Open Sans");
        assert_eq!(clean_font_name("Helvetica-Bold"), "Arial");
    }

    fn glyph(ch: char, x0: f32, baseline: f32) -> Glyph {
        Glyph {
            ch,
            x0,
            x1: x0 + 6.0,
            baseline,
            style: Style { font: "Arial".into(), size: 12.0, bold: false, italic: false, color: [0, 0, 0] },
        }
    }

    #[test]
    fn glyphs_become_lines_and_spans() {
        let mut g = Vec::new();
        for (i, ch) in "Nom".chars().enumerate() {
            g.push(glyph(ch, 50.0 + i as f32 * 6.0, 100.0));
        }
        for (i, ch) in "12,5".chars().enumerate() {
            g.push(glyph(ch, 300.0 + i as f32 * 6.0, 100.4));
        }
        for (i, ch) in "Bas".chars().enumerate() {
            g.push(glyph(ch, 50.0 + i as f32 * 6.0, 120.0));
        }
        let lines = build_lines(g);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans.len(), 2, "deux colonnes");
        assert_eq!(lines[0].spans[0].text(), "Nom");
        assert_eq!(lines[0].spans[1].text(), "12,5");
        assert_eq!(lines[1].spans[0].text(), "Bas");
    }
}
