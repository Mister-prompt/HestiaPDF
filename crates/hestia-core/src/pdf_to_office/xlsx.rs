//! Écriture d'un classeur Excel (.xlsx) : une feuille par page, colonnes
//! retrouvées d'après l'alignement du texte, nombres reconnus.

use std::fmt::Write as _;

use super::zip::{ZipWriter, xml};
use super::{Line, PageLayout};
use crate::error::Result;

const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

pub(crate) fn write(pages: &[PageLayout]) -> Result<Vec<u8>> {
    let mut zip = ZipWriter::new();
    let mut types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
    );
    let mut sheets = String::new();
    let mut rels =
        format!(r#"{HEAD}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (i, page) in pages.iter().enumerate() {
        let n = i + 1;
        let _ = write!(
            types,
            r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        );
        let _ = write!(sheets, r#"<sheet name="Page {n}" sheetId="{n}" r:id="rId{n}"/>"#);
        let _ =
            write!(rels, r#"<Relationship Id="rId{n}" Type="{REL_NS}/worksheet" Target="worksheets/sheet{n}.xml"/>"#);
        zip.add(&format!("xl/worksheets/sheet{n}.xml"), sheet(page).as_bytes())?;
    }
    types.push_str("</Types>");
    let _ =
        write!(rels, r#"<Relationship Id="rIdStyles" Type="{REL_NS}/styles" Target="styles.xml"/></Relationships>"#);

    zip.add("[Content_Types].xml", types.as_bytes())?;
    zip.add(
        "_rels/.rels",
        format!(r#"{HEAD}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL_NS}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#).as_bytes(),
    )?;
    zip.add(
        "xl/workbook.xml",
        format!(r#"{HEAD}<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="{REL_NS}"><sheets>{sheets}</sheets></workbook>"#).as_bytes(),
    )?;
    zip.add("xl/_rels/workbook.xml.rels", rels.as_bytes())?;
    zip.add("xl/styles.xml", STYLES.as_bytes())?;
    Ok(zip.finish())
}

/// Styles : 0 normal, 1 gras, 2 pourcentage, 3 gras + pourcentage.
const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/><family val="2"/></font><font><b/><sz val="11"/><name val="Calibri"/><family val="2"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="4"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/><xf numFmtId="10" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="10" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1" applyNumberFormat="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#;

/// Bandes verticales (colonnes) déduites des lignes à plusieurs morceaux.
fn columns(lines: &[Line]) -> Vec<(f32, f32)> {
    let mut bands: Vec<(f32, f32)> =
        lines.iter().filter(|l| l.spans.len() >= 2).flat_map(|l| l.spans.iter().map(|s| (s.x0, s.x1))).collect();
    bands.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32)> = Vec::new();
    for (x0, x1) in bands {
        match merged.last_mut() {
            Some(last) if x0 <= last.1 + 2.0 => last.1 = last.1.max(x1),
            _ => merged.push((x0, x1)),
        }
    }
    if merged.is_empty() {
        merged.push((0.0, f32::MAX));
    }
    merged
}

fn column_of(bands: &[(f32, f32)], x0: f32, x1: f32) -> usize {
    let center = (x0 + x1) / 2.0;
    bands
        .iter()
        .position(|&(a, b)| center >= a && center <= b)
        .or_else(|| bands.iter().position(|&(a, b)| x0 >= a - 2.0 && x0 <= b))
        .unwrap_or_else(|| {
            // Le plus proche.
            bands
                .iter()
                .enumerate()
                .min_by(|(_, p), (_, q)| (p.0 - x0).abs().total_cmp(&(q.0 - x0).abs()))
                .map_or(0, |(i, _)| i)
        })
}

fn sheet(page: &PageLayout) -> String {
    let bands = columns(&page.lines);
    let mut data = String::new();
    let mut row = 0usize;
    let mut prev: Option<&Line> = None;
    for line in &page.lines {
        // Grand espace vertical : une ligne vide, comme sur la page.
        if let Some(p) = prev
            && line.baseline - p.baseline > p.size.max(line.size) * 2.6
        {
            row += 1;
        }
        row += 1;
        prev = Some(line);
        let mut cells: Vec<(usize, String, bool)> = Vec::new();
        for span in &line.spans {
            let col = column_of(&bands, span.x0, span.x1);
            let text = span.text();
            match cells.iter_mut().find(|c| c.0 == col) {
                Some(cell) => {
                    cell.1.push(' ');
                    cell.1.push_str(&text);
                }
                None => cells.push((col, text, span.bold())),
            }
        }
        cells.sort_by_key(|c| c.0);
        let _ = write!(data, r#"<row r="{row}">"#);
        for (col, text, bold) in cells {
            let r = format!("{}{row}", column_name(col));
            match number(&text) {
                Some((v, percent)) => {
                    let style = match (bold, percent) {
                        (false, false) => 0,
                        (true, false) => 1,
                        (false, true) => 2,
                        (true, true) => 3,
                    };
                    let _ = write!(data, r#"<c r="{r}" s="{style}"><v>{v}</v></c>"#);
                }
                None => {
                    let _ = write!(
                        data,
                        r#"<c r="{r}" s="{}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
                        u8::from(bold),
                        xml(&text)
                    );
                }
            }
        }
        data.push_str("</row>");
    }
    let mut cols = String::new();
    if bands.len() > 1 || bands[0].1 < f32::MAX {
        cols.push_str("<cols>");
        for (i, (a, b)) in bands.iter().enumerate() {
            let next = bands.get(i + 1).map_or(*b, |n| n.0);
            let width = ((next.max(*b) - a) / 5.25 + 1.0).clamp(6.0, 120.0);
            let _ = write!(cols, r#"<col min="{0}" max="{0}" width="{width:.1}" customWidth="1"/>"#, i + 1);
        }
        cols.push_str("</cols>");
    } else {
        cols.push_str(r#"<cols><col min="1" max="1" width="100" customWidth="1"/></cols>"#);
    }
    format!(
        r#"{HEAD}<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{cols}<sheetData>{data}</sheetData></worksheet>"#
    )
}

/// 0 → « A », 25 → « Z », 26 → « AA ».
fn column_name(mut index: usize) -> String {
    let mut name = Vec::new();
    loop {
        name.push(b'A' + (index % 26) as u8);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    name.reverse();
    String::from_utf8(name).unwrap_or_default()
}

/// Reconnaît un nombre écrit à la française ou à l'anglaise :
/// « 1 234,56 », « 1,234.56 », « -12 % », « (300) », « 45 € ».
fn number(raw: &str) -> Option<(f64, bool)> {
    let mut t: String =
        raw.chars().filter(|c| !c.is_whitespace() && !matches!(c, '\u{a0}' | '\u{202f}' | '\u{2009}')).collect();
    if t.is_empty() || t.len() > 30 {
        return None;
    }
    let mut negative = false;
    if t.starts_with('(') && t.ends_with(')') {
        negative = true;
        t = t[1..t.len() - 1].to_string();
    }
    let percent = t.ends_with('%');
    let t = t.trim_end_matches('%').trim_start_matches(['€', '$', '£']).trim_end_matches(['€', '$', '£']).to_string();
    let (sign, body) = match t.strip_prefix(['-', '−']) {
        Some(rest) => (true, rest.to_string()),
        None => (false, t.trim_start_matches('+').to_string()),
    };
    if body.is_empty()
        || !body.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.')
        || !body.starts_with(|c: char| c.is_ascii_digit())
    {
        return None;
    }
    let commas = body.matches(',').count();
    let dots = body.matches('.').count();
    let normalized = match (commas, dots) {
        // « 0612… » (téléphone, code) : un zéro en tête n'est pas un nombre.
        (0, 0) if body.len() > 1 && body.starts_with('0') => return None,
        (0, 0) => body.clone(),
        (_, 0) if commas == 1 => body.replace(',', "."),
        (0, _) if dots == 1 => body.clone(),
        // « 1.234.567 » ou « 1,234,567 » : séparateurs de milliers.
        (0, _) => thousands(&body, '.')?,
        (_, 0) => thousands(&body, ',')?,
        _ => {
            // Les deux : le dernier est la virgule décimale.
            let last_comma = body.rfind(',').unwrap_or(0);
            let last_dot = body.rfind('.').unwrap_or(0);
            if last_comma > last_dot { body.replace('.', "").replace(',', ".") } else { body.replace(',', "") }
        }
    };
    let mut value: f64 = normalized.parse().ok()?;
    if sign || negative {
        value = -value;
    }
    if percent {
        value /= 100.0;
    }
    Some((value, percent))
}

fn thousands(body: &str, sep: char) -> Option<String> {
    let groups: Vec<&str> = body.split(sep).collect();
    let ok = groups.first().is_some_and(|g| (1..=3).contains(&g.len())) && groups[1..].iter().all(|g| g.len() == 3);
    ok.then(|| groups.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_in_french_and_english() {
        assert_eq!(number("1 234,56"), Some((1234.56, false)));
        assert_eq!(number("1,234.56"), Some((1234.56, false)));
        assert_eq!(number("12,5 %"), Some((0.125, true)));
        assert_eq!(number("(300)"), Some((-300.0, false)));
        assert_eq!(number("45 €"), Some((45.0, false)));
        assert_eq!(number("1.234.567"), Some((1_234_567.0, false)));
        assert_eq!(number("-7"), Some((-7.0, false)));
        assert_eq!(number("Loyer"), None);
        assert_eq!(number("12/03/2026"), None);
        assert_eq!(number("06 12 34 56 78"), None, "numéro de téléphone");
    }

    #[test]
    fn column_names() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(701), "ZZ");
    }
}
