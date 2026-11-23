//! Erreurs du moteur. Chaque erreur porte un code stable (`code()`) que
//! l'interface traduit ; le message `Display` sert aux journaux et aux tests.

use std::path::Path;

pub type Result<T, E = HestiaError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum HestiaError {
    #[error("erreur de lecture/écriture : {0}")]
    Io(#[from] std::io::Error),

    #[error("ce PDF est protégé par un mot de passe")]
    PasswordRequired,

    #[error("mot de passe incorrect")]
    WrongPassword,

    #[error("PDF invalide ou endommagé : {0}")]
    InvalidPdf(String),

    #[error("image illisible ou format non pris en charge : {0}")]
    UnsupportedImage(String),

    #[error("sélection de pages invalide : {0}")]
    InvalidRange(String),

    #[error("aucun fichier à traiter")]
    NoInput,

    #[error("opération annulée")]
    Cancelled,

    #[error("le fichier d'origine ne peut pas être remplacé ; choisissez un autre nom")]
    WouldOverwriteInput,

    #[error("moteur PDF indisponible : {0}")]
    EngineUnavailable(String),

    #[error("aucun logiciel de bureautique compatible (Microsoft Office ou LibreOffice) n'est installé")]
    OfficeMissing,

    #[error("le document est protégé par un mot de passe")]
    OfficeProtected,

    #[error("la conversion a pris trop de temps et a été arrêtée")]
    OfficeTimeout,

    #[error("la conversion a échoué : {0}")]
    OfficeFailed(String),

    #[error("erreur interne : {0}")]
    Internal(String),

    #[error("{name} : {source}")]
    InFile {
        name: String,
        #[source]
        source: Box<HestiaError>,
    },
}

impl HestiaError {
    /// Code stable, indépendant de la langue, utilisé par l'interface.
    pub fn code(&self) -> &'static str {
        match self {
            HestiaError::Io(_) => "io",
            HestiaError::PasswordRequired => "password_required",
            HestiaError::WrongPassword => "wrong_password",
            HestiaError::InvalidPdf(_) => "invalid_pdf",
            HestiaError::UnsupportedImage(_) => "unsupported_image",
            HestiaError::InvalidRange(_) => "invalid_range",
            HestiaError::NoInput => "no_input",
            HestiaError::Cancelled => "cancelled",
            HestiaError::WouldOverwriteInput => "would_overwrite_input",
            HestiaError::EngineUnavailable(_) => "engine_unavailable",
            HestiaError::OfficeMissing => "office_missing",
            HestiaError::OfficeProtected => "office_protected",
            HestiaError::OfficeTimeout => "office_timeout",
            HestiaError::OfficeFailed(_) => "office_failed",
            HestiaError::Internal(_) => "internal",
            HestiaError::InFile { source, .. } => source.code(),
        }
    }

    /// Nom du fichier concerné, si l'erreur a été rattachée à un fichier.
    pub fn file_name(&self) -> Option<&str> {
        match self {
            HestiaError::InFile { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Détail technique (sans le nom de fichier), pour l'aide au diagnostic.
    pub fn detail(&self) -> String {
        match self {
            HestiaError::InFile { source, .. } => source.detail(),
            other => other.to_string(),
        }
    }

    /// Rattache l'erreur au fichier `path` (n'empile pas deux contextes).
    pub fn in_file(self, path: &Path) -> HestiaError {
        match self {
            e @ HestiaError::InFile { .. } | e @ HestiaError::Cancelled => e,
            other => HestiaError::InFile { name: display_name(path), source: Box::new(other) },
        }
    }
}

pub(crate) fn display_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned())
}

impl From<pdfium_render::prelude::PdfiumError> for HestiaError {
    fn from(err: pdfium_render::prelude::PdfiumError) -> Self {
        use pdfium_render::prelude::{PdfiumError as E, PdfiumInternalError as I};
        match err {
            E::PdfiumLibraryInternalError(I::PasswordError) => HestiaError::PasswordRequired,
            E::PdfiumLibraryInternalError(I::FormatError) => {
                HestiaError::InvalidPdf("structure du fichier non reconnue".into())
            }
            E::PdfiumLibraryInternalError(I::FileError) => {
                HestiaError::InvalidPdf("fichier introuvable ou illisible".into())
            }
            E::PdfiumLibraryInternalError(I::SecurityError) => {
                HestiaError::InvalidPdf("mécanisme de protection non pris en charge".into())
            }
            E::IoError(io) => HestiaError::Io(io),
            other => HestiaError::Internal(format!("PDFium : {other:?}")),
        }
    }
}

impl From<lopdf::Error> for HestiaError {
    fn from(err: lopdf::Error) -> Self {
        match err {
            lopdf::Error::IO(io) => HestiaError::Io(io),
            other => HestiaError::InvalidPdf(other.to_string()),
        }
    }
}

impl From<image::ImageError> for HestiaError {
    fn from(err: image::ImageError) -> Self {
        match err {
            image::ImageError::IoError(io) => HestiaError::Io(io),
            other => HestiaError::UnsupportedImage(other.to_string()),
        }
    }
}

impl From<tempfile::PersistError> for HestiaError {
    fn from(err: tempfile::PersistError) -> Self {
        HestiaError::Io(err.error)
    }
}
