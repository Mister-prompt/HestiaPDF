//! Écriture sûre des fichiers produits.
//!
//! - Écriture **atomique** : on écrit dans un fichier temporaire du même
//!   dossier, puis on le renomme. Un plantage ou une annulation ne laisse
//!   jamais de fichier à moitié écrit.
//! - Le fichier d'origine n'est **jamais** remplacé.
//! - Dans un dossier, les fichiers existants ne sont jamais écrasés
//!   (`nom (2).pdf`, `nom (3).pdf`…).

use std::fs::File;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{HestiaError, Result};

/// Fichier produit par un traitement.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutputFile {
    pub path: PathBuf,
    pub size: u64,
}

impl OutputFile {
    fn from_path(path: PathBuf) -> Result<Self> {
        let size = std::fs::metadata(&path)?.len();
        Ok(Self { path, size })
    }
}

/// Refuse une destination qui désignerait l'un des fichiers d'entrée.
pub fn ensure_not_input<'a>(output: &Path, inputs: impl IntoIterator<Item = &'a Path>) -> Result<()> {
    let out = normalize(output);
    for input in inputs {
        if normalize(input) == out {
            return Err(HestiaError::WouldOverwriteInput);
        }
    }
    Ok(())
}

/// Chemin comparable : dossier parent canonique + nom (insensible à la casse
/// sous Windows). Fonctionne même si le fichier n'existe pas encore.
fn normalize(path: &Path) -> String {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).and_then(|p| p.canonicalize().ok());
    let full = match (parent, path.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    };
    let s = full.to_string_lossy().into_owned();
    if cfg!(windows) { s.to_lowercase() } else { s }
}

fn temp_in(dir: &Path) -> Result<tempfile::NamedTempFile> {
    tempfile::Builder::new().prefix(".hestia-").suffix(".tmp").tempfile_in(dir).map_err(HestiaError::Io)
}

fn parent_dir(path: &Path) -> Result<&Path> {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => Ok(p),
        _ => Err(HestiaError::Io(io::Error::new(ErrorKind::InvalidInput, "dossier de destination introuvable"))),
    }
}

/// Écrit `output` de façon atomique. Remplace `output` s'il existe déjà :
/// l'utilisateur l'a confirmé dans la fenêtre « Enregistrer sous ».
pub fn write_atomic(output: &Path, write: impl FnOnce(&mut File) -> Result<()>) -> Result<OutputFile> {
    let mut tmp = temp_in(parent_dir(output)?)?;
    write(tmp.as_file_mut())?;
    tmp.as_file().sync_all()?;
    tmp.persist(output)?;
    OutputFile::from_path(output.to_path_buf())
}

/// Crée un nouveau fichier `stem.ext` dans `dir` sans jamais écraser
/// un fichier existant.
pub fn write_new_in_dir(
    dir: &Path,
    stem: &str,
    ext: &str,
    write: impl FnOnce(&mut File) -> Result<()>,
) -> Result<OutputFile> {
    let mut tmp = temp_in(dir)?;
    write(tmp.as_file_mut())?;
    tmp.as_file().sync_all()?;

    let stem = sanitize_stem(stem);
    for n in 1..=9999u32 {
        let name = if n == 1 { format!("{stem}.{ext}") } else { format!("{stem} ({n}).{ext}") };
        let candidate = dir.join(name);
        match tmp.persist_noclobber(&candidate) {
            Ok(_) => return OutputFile::from_path(candidate),
            Err(e) if e.error.kind() == ErrorKind::AlreadyExists || candidate.exists() => {
                tmp = e.file;
            }
            Err(e) => return Err(HestiaError::Io(e.error)),
        }
    }
    Err(HestiaError::Io(io::Error::new(ErrorKind::AlreadyExists, "trop de fichiers portant le même nom")))
}

/// Premier chemin libre `dir/stem.ext`, `dir/stem (2).ext`… (pour les
/// traitements qui écrivent ensuite de façon atomique à cet endroit).
pub fn available_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let stem = sanitize_stem(stem);
    (1..=9999u32)
        .map(|n| if n == 1 { dir.join(format!("{stem}.{ext}")) } else { dir.join(format!("{stem} ({n}).{ext}")) })
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(format!("{stem} (hestia).{ext}")))
}

/// Nom de fichier sans extension, sûr pour Windows.
pub fn sanitize_stem(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let mut s = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if s.chars().count() > 120 {
        s = s.chars().take(120).collect::<String>().trim_end().to_string();
    }
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let base = s.split('.').next().unwrap_or_default().to_ascii_uppercase();
    if RESERVED.contains(&base.as_str()) {
        s.insert(0, '_');
    }
    if s.is_empty() { "document".to_string() } else { s }
}

/// Nom du fichier sans extension (`/a/rapport.pdf` → `rapport`).
pub fn file_stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "document".to_string())
}

/// Vérifie qu'un dossier de destination existe.
pub fn ensure_dir(dir: &Path) -> Result<PathBuf> {
    if dir.is_dir() {
        Ok(dir.to_path_buf())
    } else {
        Err(HestiaError::Io(io::Error::new(ErrorKind::NotFound, "dossier de destination introuvable")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_forbidden_characters() {
        assert_eq!(sanitize_stem("a<b>c:d\"e/f\\g|h?i*j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize_stem("  rapport. "), "rapport");
        assert_eq!(sanitize_stem("CON"), "_CON");
        assert_eq!(sanitize_stem("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_stem(""), "document");
        assert_eq!(sanitize_stem("contrat été"), "contrat été");
    }

    #[test]
    fn write_new_never_overwrites() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let a = write_new_in_dir(dir.path(), "doc", "pdf", |f| {
            use std::io::Write;
            f.write_all(b"one").map_err(Into::into)
        })?;
        let b = write_new_in_dir(dir.path(), "doc", "pdf", |f| {
            use std::io::Write;
            f.write_all(b"two").map_err(Into::into)
        })?;
        assert_eq!(a.path.file_name().and_then(|n| n.to_str()), Some("doc.pdf"));
        assert_eq!(b.path.file_name().and_then(|n| n.to_str()), Some("doc (2).pdf"));
        assert_eq!(std::fs::read(&a.path)?, b"one");
        assert_eq!(std::fs::read(&b.path)?, b"two");
        Ok(())
    }

    #[test]
    fn failed_write_leaves_nothing_behind() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("out.pdf");
        let res = write_atomic(&target, |_| Err(HestiaError::Cancelled));
        assert!(res.is_err());
        assert!(!target.exists());
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 0, "pas de fichier temporaire oublié");
        Ok(())
    }

    #[test]
    fn output_equal_to_input_is_refused() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let input = dir.path().join("Contrat.pdf");
        std::fs::write(&input, b"x")?;
        let same = dir.path().join("contrat.PDF");
        let other = dir.path().join("autre.pdf");
        assert!(
            matches!(ensure_not_input(&same, [input.as_path()]), Err(HestiaError::WouldOverwriteInput))
                || !cfg!(windows)
        );
        assert!(ensure_not_input(&other, [input.as_path()]).is_ok());
        assert!(matches!(ensure_not_input(&input, [input.as_path()]), Err(HestiaError::WouldOverwriteInput)));
        Ok(())
    }
}
