//! Polices proposées dans « Modifier PDF ».
//!
//! Deux sortes de polices :
//! - les trois polices standard du PDF (Helvetica, Times, Courier), que tous
//!   les lecteurs connaissent : rien à intégrer ;
//! - des polices libres (SIL Open Font License) livrées avec l'application
//!   (dossier `public/fonts/` de l'interface). Seuls les glyphes utilisés
//!   sont intégrés au PDF (voir `edit.rs`).
//!
//! La liste doit rester identique à `src/views/editor/fonts.ts` (vérifié par
//! un test).

use std::path::PathBuf;

/// Police livrée avec l'application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Family {
    /// Identifiant envoyé par l'interface.
    pub id: &'static str,
    /// Début des noms de fichiers (`<stem>-Regular.ttf`…).
    pub stem: &'static str,
    /// Graisse grasse disponible.
    pub bold: bool,
    /// Italique disponible (et gras italique si `bold`).
    pub italic: bool,
}

const fn fam(id: &'static str, stem: &'static str, bold: bool, italic: bool) -> Family {
    Family { id, stem, bold, italic }
}

pub const FAMILIES: &[Family] = &[
    fam("opensans", "OpenSans", true, true),
    fam("roboto", "Roboto", true, true),
    fam("lato", "Lato", true, true),
    fam("montserrat", "Montserrat", true, true),
    fam("poppins", "Poppins", true, true),
    fam("nunito", "Nunito", true, true),
    fam("oswald", "Oswald", true, false),
    fam("bebasneue", "BebasNeue", false, false),
    fam("lora", "Lora", true, true),
    fam("playfair", "PlayfairDisplay", true, true),
    fam("ebgaramond", "EBGaramond", true, true),
    fam("robotomono", "RobotoMono", true, true),
    fam("caveat", "Caveat", true, false),
    fam("dancingscript", "DancingScript", true, false),
    fam("pacifico", "Pacifico", false, false),
];

/// Polices standard du PDF.
pub const STANDARD: &[&str] = &["helvetica", "times", "courier"];

pub fn family(id: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| f.id == id)
}

pub fn is_known(id: &str) -> bool {
    STANDARD.contains(&id) || family(id).is_some()
}

/// Style réellement utilisé et fichier correspondant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Face {
    pub file: String,
    pub bold: bool,
    pub italic: bool,
}

impl Family {
    /// Le gras ou l'italique absents de la police sont ignorés (jamais simulés),
    /// comme dans l'interface.
    pub fn face(&self, bold: bool, italic: bool) -> Face {
        let bold = bold && self.bold;
        let italic = italic && self.italic;
        let suffix = match (bold, italic) {
            (false, false) => "Regular",
            (true, false) => "Bold",
            (false, true) => "Italic",
            (true, true) => "BoldItalic",
        };
        Face { file: format!("{}-{suffix}.ttf", self.stem), bold, italic }
    }

    /// Tous les fichiers de la famille.
    pub fn files(&self) -> Vec<String> {
        let mut out = vec![self.face(false, false).file];
        if self.bold {
            out.push(self.face(true, false).file);
        }
        if self.italic {
            out.push(self.face(false, true).file);
            if self.bold {
                out.push(self.face(true, true).file);
            }
        }
        out
    }
}

/// D'où viennent les fichiers de polices (ressources de l'application, dossier…).
pub trait FontSource: Sync {
    /// Contenu du fichier `name` (un nom du catalogue, jamais un chemin).
    fn load(&self, name: &str) -> Option<Vec<u8>>;
}

impl<F: Fn(&str) -> Option<Vec<u8>> + Sync> FontSource for F {
    fn load(&self, name: &str) -> Option<Vec<u8>> {
        self(name)
    }
}

/// Polices lues dans un dossier.
pub struct DirFonts(pub PathBuf);

impl FontSource for DirFonts {
    fn load(&self, name: &str) -> Option<Vec<u8>> {
        if name.contains(['/', '\\']) || name.contains("..") {
            return None;
        }
        std::fs::read(self.0.join(name)).ok()
    }
}

/// Aucune police intégrée disponible (seules les polices standard marchent).
pub struct NoFonts;

impl FontSource for NoFonts {
    fn load(&self, _name: &str) -> Option<Vec<u8>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fonts")
    }

    #[test]
    fn faces_fall_back_to_available_styles() {
        let oswald = family("oswald").map(|f| f.face(true, true));
        assert_eq!(oswald, Some(Face { file: "Oswald-Bold.ttf".into(), bold: true, italic: false }));
        let pacifico = family("pacifico").map(|f| f.face(true, true));
        assert_eq!(pacifico, Some(Face { file: "Pacifico-Regular.ttf".into(), bold: false, italic: false }));
        assert!(is_known("helvetica") && is_known("lato") && !is_known("../lato"));
    }

    #[test]
    fn every_catalog_file_is_a_static_truetype_font() {
        let dir = DirFonts(fonts_dir());
        for fam in FAMILIES {
            for file in fam.files() {
                let bytes = dir.load(&file).unwrap_or_else(|| panic!("{file} manquant"));
                let face = ttf_parser::Face::parse(&bytes, 0).unwrap_or_else(|e| panic!("{file} : {e}"));
                assert!(!face.is_variable(), "{file} est une police variable");
                assert!(face.tables().glyf.is_some(), "{file} n'a pas de contours TrueType");
                for ch in "AÉàçœ€?".chars() {
                    assert!(face.glyph_index(ch).is_some(), "{file} : « {ch} » absent");
                }
            }
        }
    }

    #[test]
    fn interface_catalog_matches() {
        let ts =
            std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/views/editor/fonts.ts"))
                .unwrap_or_default();
        for fam in FAMILIES {
            let entry = format!("[\"{}\", \"{}\", {}, {}", fam.id, fam.stem, fam.bold, fam.italic);
            assert!(ts.contains(&entry), "fonts.ts ne déclare pas {entry}");
        }
        let declared = ts.matches("\", true, ").count() + ts.matches("\", false, ").count();
        assert_eq!(declared, FAMILIES.len(), "fonts.ts déclare d'autres polices");
        let on_disk = std::fs::read_dir(fonts_dir())
            .map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "ttf")).count())
            .unwrap_or(0);
        assert_eq!(
            on_disk,
            FAMILIES.iter().map(|f| f.files().len()).sum::<usize>(),
            "fichiers en trop dans public/fonts"
        );
    }
}
