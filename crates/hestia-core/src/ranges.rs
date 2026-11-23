//! Lecture des sélections de pages saisies par l'utilisateur,
//! par exemple `1-3, 5, 8-` (pages numérotées à partir de 1).

use crate::error::{HestiaError, Result};

/// Plage de pages inclusive, numérotée à partir de 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRange {
    pub start: u32,
    pub end: u32,
}

impl PageRange {
    /// Indices de pages à partir de 0.
    pub fn indices(&self) -> impl Iterator<Item = u32> {
        (self.start - 1)..self.end
    }
}

/// Analyse `input` pour un document de `page_count` pages.
///
/// Formes acceptées (séparées par `,` ou `;`) :
/// `5` · `2-7` · `8-` (jusqu'à la fin) · `-3` (depuis le début).
pub fn parse_ranges(input: &str, page_count: u32) -> Result<Vec<PageRange>> {
    if page_count == 0 {
        return Err(HestiaError::InvalidRange("le document ne contient aucune page".into()));
    }
    let mut out = Vec::new();
    for part in input.split([',', ';']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let range = match part.split_once('-') {
            None => {
                let n = parse_page(part)?;
                PageRange { start: n, end: n }
            }
            Some((a, b)) => {
                let (a, b) = (a.trim(), b.trim());
                let start = if a.is_empty() { 1 } else { parse_page(a)? };
                let end = if b.is_empty() { page_count } else { parse_page(b)? };
                PageRange { start, end }
            }
        };
        if range.start > range.end {
            return Err(HestiaError::InvalidRange(format!("« {part} » : la première page doit précéder la dernière")));
        }
        if range.end > page_count {
            return Err(HestiaError::InvalidRange(format!("« {part} » : le document n'a que {page_count} page(s)")));
        }
        out.push(range);
    }
    if out.is_empty() {
        return Err(HestiaError::InvalidRange("aucune page indiquée".into()));
    }
    Ok(out)
}

fn parse_page(s: &str) -> Result<u32> {
    match s.parse::<u32>() {
        Ok(0) => Err(HestiaError::InvalidRange("les pages commencent à 1".into())),
        Ok(n) => Ok(n),
        Err(_) => Err(HestiaError::InvalidRange(format!("« {s} » n'est pas un numéro de page"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: u32, end: u32) -> PageRange {
        PageRange { start, end }
    }

    #[test]
    fn parses_common_forms() -> Result<()> {
        assert_eq!(parse_ranges("1-3, 5, 8-", 10)?, vec![r(1, 3), r(5, 5), r(8, 10)]);
        assert_eq!(parse_ranges("-2;4", 5)?, vec![r(1, 2), r(4, 4)]);
        assert_eq!(parse_ranges(" 7 ", 7)?, vec![r(7, 7)]);
        Ok(())
    }

    #[test]
    fn rejects_invalid_input() {
        for bad in ["", "0", "3-1", "11", "a", "1-x", " , "] {
            assert!(parse_ranges(bad, 10).is_err(), "{bad:?} aurait dû être refusé");
        }
    }

    #[test]
    fn indices_are_zero_based() {
        assert_eq!(r(2, 4).indices().collect::<Vec<_>>(), vec![1, 2, 3]);
    }
}
