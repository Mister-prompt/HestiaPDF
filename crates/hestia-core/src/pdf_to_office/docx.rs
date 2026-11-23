//! Écriture d'un document Word (.docx) à partir de la mise en page lue.

use std::fmt::Write as _;

use super::zip::{ZipWriter, xml};
use super::{Line, PageImage, PageLayout, Run, emu, hex};
use crate::error::Result;

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture""#;

/// Marges et largeur utile du document.
struct Frame {
    page_w: f32,
    page_h: f32,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
}

impl Frame {
    fn content_width(&self) -> f32 {
        (self.page_w - self.left - self.right).max(72.0)
    }
}

pub(crate) fn write(pages: &[PageLayout]) -> Result<Vec<u8>> {
    let frame = frame(pages);
    let mut body = String::new();
    let mut media: Vec<(String, &PageImage)> = Vec::new();

    for (index, page) in pages.iter().enumerate() {
        if index > 0 {
            body.push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#);
        }
        // Paragraphes et images, dans l'ordre vertical de la page.
        let mut items: Vec<(f32, Item)> =
            paragraphs(&page.lines).into_iter().map(|p| (p.top(), Item::Text(p))).collect();
        let page_area = page.width * page.height;
        for img in &page.images {
            // Image de fond couvrant la page alors qu'il y a du texte : ignorée.
            if !page.lines.is_empty() && img.w * img.h > page_area * 0.7 {
                continue;
            }
            items.push((img.y, Item::Image(img)));
        }
        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut prev_bottom: Option<f32> = None;
        for (_, item) in items {
            match item {
                Item::Text(p) => {
                    let gap = prev_bottom.map_or(0.0, |b| (p.top() - b).max(0.0));
                    write_paragraph(&mut body, &p, &frame, page, gap);
                    prev_bottom = Some(p.bottom());
                }
                Item::Image(img) => {
                    media.push((format!("image{}.{}", media.len() + 1, if img.png { "png" } else { "jpeg" }), img));
                    let id = media.len();
                    write_image(&mut body, img, id, &frame);
                    prev_bottom = Some(img.y + img.h);
                }
            }
        }
    }

    let mut doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>"#);
    doc.push_str(&body);
    let _ = write!(
        doc,
        r#"<w:sectPr><w:pgSz w:w="{}" w:h="{}"{}/><w:pgMar w:top="{}" w:right="{}" w:bottom="{}" w:left="{}" w:header="360" w:footer="360" w:gutter="0"/></w:sectPr></w:body></w:document>"#,
        twips(frame.page_w),
        twips(frame.page_h),
        if frame.page_w > frame.page_h { r#" w:orient="landscape""# } else { "" },
        twips(frame.top),
        twips(frame.right),
        twips(frame.bottom),
        twips(frame.left),
    );

    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    );
    for (i, (name, _)) in media.iter().enumerate() {
        let _ = write!(
            rels,
            r#"<Relationship Id="rIdImg{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{name}"/>"#,
            i + 1
        );
    }
    rels.push_str("</Relationships>");

    let mut zip = ZipWriter::new();
    zip.add(
        "[Content_Types].xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#,
    )?;
    zip.add(
        "_rels/.rels",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
    )?;
    zip.add("word/document.xml", doc.as_bytes())?;
    zip.add("word/_rels/document.xml.rels", rels.as_bytes())?;
    zip.add("word/styles.xml", STYLES.as_bytes())?;
    for (name, img) in &media {
        zip.add(&format!("word/media/{name}"), &img.data)?;
    }
    Ok(zip.finish())
}

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/><w:lang w:val="fr-FR"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style></w:styles>"#;

enum Item<'a> {
    Text(Paragraph<'a>),
    Image(&'a PageImage),
}

/// Lignes consécutives formant un paragraphe.
struct Paragraph<'a> {
    lines: Vec<&'a Line>,
}

impl Paragraph<'_> {
    fn top(&self) -> f32 {
        self.lines.first().map_or(0.0, |l| l.top())
    }

    fn bottom(&self) -> f32 {
        self.lines.last().map_or(0.0, |l| l.baseline + l.size * 0.25)
    }

    fn x0(&self) -> f32 {
        self.lines.iter().map(|l| l.x0()).fold(f32::MAX, f32::min)
    }

    fn x1(&self) -> f32 {
        self.lines.iter().map(|l| l.x1()).fold(f32::MIN, f32::max)
    }
}

fn frame(pages: &[PageLayout]) -> Frame {
    let first = pages.first();
    let page_w = first.map_or(595.0, |p| p.width);
    let page_h = first.map_or(842.0, |p| p.height);
    let lines = pages.iter().flat_map(|p| p.lines.iter());
    let (mut x0, mut x1, mut y0) = (f32::MAX, f32::MIN, f32::MAX);
    for l in lines {
        x0 = x0.min(l.x0());
        x1 = x1.max(l.x1());
        y0 = y0.min(l.top());
    }
    let clamp = |v: f32, default: f32| if v.is_finite() { v.clamp(18.0, 108.0) } else { default };
    Frame { page_w, page_h, left: clamp(x0, 72.0), right: clamp(page_w - x1, 72.0), top: clamp(y0, 72.0), bottom: 36.0 }
}

fn paragraphs(lines: &[Line]) -> Vec<Paragraph<'_>> {
    let mut out: Vec<Paragraph<'_>> = Vec::new();
    for line in lines {
        let joins = out.last().and_then(|p| p.lines.last().map(|prev| (p, *prev))).is_some_and(|(p, prev)| {
            let gap = line.baseline - prev.baseline;
            let same_size = (line.size - prev.size).abs() <= 1.0;
            let aligned =
                (line.x0() - prev.x0()).abs() <= prev.size * 1.5 || (line.x0() - p.x0()).abs() <= prev.size * 1.5;
            // La ligne précédente allait jusqu'au bout : le texte continue.
            let full = prev.x1() >= p.x1() - prev.size * 4.0;
            gap > 0.0
                && gap <= prev.size * 1.75
                && same_size
                && aligned
                && full
                && line.spans.len() == 1
                && prev.spans.len() == 1
        });
        if joins {
            if let Some(p) = out.last_mut() {
                p.lines.push(line);
            }
        } else {
            out.push(Paragraph { lines: vec![line] });
        }
    }
    out
}

fn write_paragraph(out: &mut String, p: &Paragraph<'_>, frame: &Frame, page: &PageLayout, gap: f32) {
    let first = p.lines[0];
    let width = p.x1() - p.x0();
    let center = (p.x0() + p.x1()) / 2.0;
    let centered = (center - page.width / 2.0).abs() < page.width * 0.04 && width < frame.content_width() * 0.8;
    let indent = (p.x0() - frame.left).max(0.0);
    let normal_gap = first.size * 0.3;
    out.push_str("<w:p><w:pPr>");
    if gap > normal_gap {
        let _ = write!(out, r#"<w:spacing w:before="{}"/>"#, twips((gap - normal_gap).min(72.0)));
    }
    if centered {
        out.push_str(r#"<w:jc w:val="center"/>"#);
    } else if indent > 4.0 {
        let _ = write!(out, r#"<w:ind w:left="{}"/>"#, twips(indent));
    }
    out.push_str("</w:pPr>");
    for (i, line) in p.lines.iter().enumerate() {
        if i > 0 {
            // Fin de ligne : une espace, sauf mot coupé par un trait d'union.
            let prev_text: String = p.lines[i - 1].spans.iter().map(|s| s.text()).collect();
            let next_starts_lower = line.spans[0].text().chars().next().is_some_and(char::is_lowercase);
            if !(prev_text.ends_with('-') && next_starts_lower) {
                write_run(out, &Run { text: " ".into(), style: line.spans[0].runs[0].style.clone() }, false);
            }
        }
        for (j, span) in line.spans.iter().enumerate() {
            if j > 0 {
                out.push_str("<w:r><w:tab/></w:r>");
            }
            let last_line = i + 1 < p.lines.len();
            for (k, run) in span.runs.iter().enumerate() {
                let is_last_run = k + 1 == span.runs.len();
                let strip_hyphen =
                    last_line && is_last_run && j + 1 == line.spans.len() && run.text.ends_with('-') && {
                        p.lines[i + 1].spans[0].text().chars().next().is_some_and(char::is_lowercase)
                    };
                write_run(out, run, strip_hyphen);
            }
        }
    }
    out.push_str("</w:p>");
}

fn write_run(out: &mut String, run: &Run, strip_hyphen: bool) {
    let s = &run.style;
    let text = if strip_hyphen { run.text.trim_end_matches('-') } else { run.text.as_str() };
    let font = xml(&s.font);
    let _ = write!(
        out,
        r#"<w:r><w:rPr><w:rFonts w:ascii="{font}" w:hAnsi="{font}" w:cs="{font}"/>{}{}"#,
        if s.bold { "<w:b/>" } else { "" },
        if s.italic { "<w:i/>" } else { "" },
    );
    if s.color != [0, 0, 0] {
        let _ = write!(out, r#"<w:color w:val="{}"/>"#, hex(s.color));
    }
    let half_points = (s.size * 2.0).round().clamp(2.0, 3276.0) as i32;
    let _ = write!(
        out,
        r#"<w:sz w:val="{half_points}"/><w:szCs w:val="{half_points}"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r>"#,
        xml(text)
    );
}

fn write_image(out: &mut String, img: &PageImage, id: usize, frame: &Frame) {
    let max_w = frame.content_width();
    let scale = if img.w > max_w { max_w / img.w } else { 1.0 };
    let (cx, cy) = (emu(img.w * scale), emu(img.h * scale));
    let indent = (img.x - frame.left).clamp(0.0, (max_w - img.w * scale).max(0.0));
    let _ = write!(
        out,
        r#"<w:p><w:pPr><w:ind w:left="{}"/></w:pPr><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{id}" name="Image {id}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{id}" name="Image {id}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdImg{id}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
        twips(indent)
    );
}

/// Points → vingtièmes de point (unité de mise en page de Word).
fn twips(pt: f32) -> i64 {
    (pt * 20.0).round() as i64
}
