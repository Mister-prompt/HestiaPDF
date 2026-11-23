//! Écriture d'une présentation PowerPoint (.pptx) : une diapositive par
//! page, la page sans texte en fond et le texte modifiable par-dessus.

use std::fmt::Write as _;

use super::zip::{ZipWriter, xml};
use super::{PageLayout, emu, hex};
use crate::error::Result;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const GROUP: &str = r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>"#;

/// Bornes de taille d'une diapositive acceptées par PowerPoint (EMU).
const MIN_SIDE: i64 = 914_400;
const MAX_SIDE: i64 = 51_206_400;

pub(crate) fn write(pages: &[PageLayout]) -> Result<Vec<u8>> {
    let first = pages.first();
    let (pw, ph) = first.map_or((960.0, 540.0), |p| (p.width, p.height));
    // Taille de diapositive = première page (réduite si elle dépasse le maximum).
    let fit = (MAX_SIDE as f32 / emu(pw.max(ph)) as f32).min(1.0);
    let slide_w = (emu(pw) as f32 * fit) as i64;
    let slide_h = (emu(ph) as f32 * fit) as i64;
    let (slide_w, slide_h) = (slide_w.clamp(MIN_SIDE, MAX_SIDE), slide_h.clamp(MIN_SIDE, MAX_SIDE));

    let mut zip = ZipWriter::new();
    let mut types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="jpeg" ContentType="image/jpeg"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#,
    );
    let mut ids = String::new();
    let mut pres_rels = format!(
        r#"{HEAD}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL_NS}/slideMaster" Target="slideMasters/slideMaster1.xml"/><Relationship Id="rId2" Type="{REL_NS}/theme" Target="theme/theme1.xml"/>"#
    );

    for (i, page) in pages.iter().enumerate() {
        let n = i + 1;
        let _ = write!(
            types,
            r#"<Override PartName="/ppt/slides/slide{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#
        );
        let _ = write!(ids, r#"<p:sldId id="{}" r:id="rId{}"/>"#, 255 + n, n + 2);
        let _ = write!(
            pres_rels,
            r#"<Relationship Id="rId{}" Type="{REL_NS}/slide" Target="slides/slide{n}.xml"/>"#,
            n + 2
        );

        // Chaque page est mise à l'échelle de la diapositive, centrée.
        let scale = (slide_w as f32 / emu(page.width) as f32).min(slide_h as f32 / emu(page.height) as f32);
        let off_x = (slide_w as f32 - emu(page.width) as f32 * scale) / 2.0;
        let off_y = (slide_h as f32 - emu(page.height) as f32 * scale) / 2.0;
        let pos = |x: f32, y: f32| ((emu(x) as f32 * scale + off_x) as i64, (emu(y) as f32 * scale + off_y) as i64);
        let len = |v: f32| (emu(v) as f32 * scale).max(1.0) as i64;

        let mut tree = String::new();
        let mut rels = format!(
            r#"{HEAD}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL_NS}/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>"#
        );
        let mut shape_id = 2;
        if let Some(bg) = &page.background {
            let (x, y) = pos(0.0, 0.0);
            let _ = write!(
                tree,
                r#"<p:pic><p:nvPicPr><p:cNvPr id="{shape_id}" name="Page {n}"/><p:cNvPicPr><a:picLocks noGrp="1" noChangeAspect="1"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#,
                len(page.width),
                len(page.height)
            );
            shape_id += 1;
            let _ = write!(rels, r#"<Relationship Id="rId2" Type="{REL_NS}/image" Target="../media/page{n}.jpeg"/>"#);
            zip.add(&format!("ppt/media/page{n}.jpeg"), bg)?;
        }
        for line in &page.lines {
            for span in &line.spans {
                let (x, y) = pos(span.x0, line.baseline - line.size * 0.95);
                let _ = write!(
                    tree,
                    r#"<p:sp><p:nvSpPr><p:cNvPr id="{shape_id}" name="Texte {shape_id}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap="none" lIns="0" tIns="0" rIns="0" bIns="0" anchor="t"><a:noAutofit/></a:bodyPr><a:lstStyle/><a:p>"#,
                    len((span.x1 - span.x0) * 1.04 + line.size * 0.3),
                    len(line.size * 1.25)
                );
                shape_id += 1;
                for run in &span.runs {
                    let s = &run.style;
                    let sz = ((s.size * scale * 100.0).round() as i64).clamp(100, 400_000);
                    let font = xml(&s.font);
                    let _ = write!(
                        tree,
                        r#"<a:r><a:rPr lang="fr-FR" sz="{sz}" b="{}" i="{}" dirty="0"><a:solidFill><a:srgbClr val="{}"/></a:solidFill><a:latin typeface="{font}"/><a:cs typeface="{font}"/></a:rPr><a:t>{}</a:t></a:r>"#,
                        u8::from(s.bold),
                        u8::from(s.italic),
                        hex(s.color),
                        xml(&run.text)
                    );
                }
                tree.push_str("</a:p></p:txBody></p:sp>");
            }
        }
        rels.push_str("</Relationships>");
        let slide = format!(
            r#"{HEAD}<p:sld {NS}><p:cSld><p:spTree>{GROUP}{tree}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#
        );
        zip.add(&format!("ppt/slides/slide{n}.xml"), slide.as_bytes())?;
        zip.add(&format!("ppt/slides/_rels/slide{n}.xml.rels"), rels.as_bytes())?;
    }
    types.push_str("</Types>");
    pres_rels.push_str("</Relationships>");

    let presentation = format!(
        r#"{HEAD}<p:presentation {NS} saveSubsetFonts="1"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="{slide_w}" cy="{slide_h}"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>"#
    );
    zip.add("[Content_Types].xml", types.as_bytes())?;
    zip.add(
        "_rels/.rels",
        format!(r#"{HEAD}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL_NS}/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#).as_bytes(),
    )?;
    zip.add("ppt/presentation.xml", presentation.as_bytes())?;
    zip.add("ppt/_rels/presentation.xml.rels", pres_rels.as_bytes())?;
    zip.add(
        "ppt/slideMasters/slideMaster1.xml",
        format!(r#"{HEAD}<p:sldMaster {NS}><p:cSld><p:spTree>{GROUP}</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst></p:sldMaster>"#).as_bytes(),
    )?;
    zip.add(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        format!(r#"{HEAD}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL_NS}/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="{REL_NS}/theme" Target="../theme/theme1.xml"/></Relationships>"#).as_bytes(),
    )?;
    zip.add(
        "ppt/slideLayouts/slideLayout1.xml",
        format!(r#"{HEAD}<p:sldLayout {NS} type="blank" preserve="1"><p:cSld name="Vide"><p:spTree>{GROUP}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"#).as_bytes(),
    )?;
    zip.add(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        format!(r#"{HEAD}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL_NS}/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#).as_bytes(),
    )?;
    zip.add("ppt/theme/theme1.xml", theme().as_bytes())?;
    Ok(zip.finish())
}

fn theme() -> String {
    let colors = [
        ("dk1", "000000"),
        ("lt1", "FFFFFF"),
        ("dk2", "1E0B1B"),
        ("lt2", "EBEBEB"),
        ("accent1", "4472C4"),
        ("accent2", "ED7D31"),
        ("accent3", "A5A5A5"),
        ("accent4", "FFC000"),
        ("accent5", "5B9BD5"),
        ("accent6", "70AD47"),
        ("hlink", "0563C1"),
        ("folHlink", "954F72"),
    ];
    let scheme: String = colors.iter().map(|(k, v)| format!(r#"<a:{k}><a:srgbClr val="{v}"/></a:{k}>"#)).collect();
    let solid = r#"<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>"#;
    let line = r#"<a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>"#;
    let effect = "<a:effectStyle><a:effectLst/></a:effectStyle>";
    let font = r#"<a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/>"#;
    format!(
        r#"{HEAD}<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Hestia"><a:themeElements><a:clrScheme name="Hestia">{scheme}</a:clrScheme><a:fontScheme name="Hestia"><a:majorFont>{font}</a:majorFont><a:minorFont>{font}</a:minorFont></a:fontScheme><a:fmtScheme name="Hestia"><a:fillStyleLst>{solid}{solid}{solid}</a:fillStyleLst><a:lnStyleLst>{line}{line}{line}</a:lnStyleLst><a:effectStyleLst>{effect}{effect}{effect}</a:effectStyleLst><a:bgFillStyleLst>{solid}{solid}{solid}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#
    )
}
