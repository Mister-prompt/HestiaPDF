//! Erreurs renvoyées à l'interface : un code stable (traduit côté interface),
//! le fichier concerné et un détail technique.

use std::path::Path;

use hestia_core::HestiaError;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: String,
    pub file: Option<String>,
    pub detail: String,
}

impl CommandError {
    pub fn new(code: &str, detail: impl Into<String>) -> Self {
        Self { code: code.to_string(), file: None, detail: detail.into() }
    }

    pub fn forbidden(path: &Path) -> Self {
        Self {
            code: "forbidden".into(),
            file: path.file_name().map(|n| n.to_string_lossy().into_owned()),
            detail: "fichier non autorisé : choisissez-le à nouveau".into(),
        }
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::new("internal", detail)
    }
}

impl From<HestiaError> for CommandError {
    fn from(err: HestiaError) -> Self {
        Self { code: err.code().to_string(), file: err.file_name().map(str::to_string), detail: err.detail() }
    }
}

impl From<tauri::Error> for CommandError {
    fn from(err: tauri::Error) -> Self {
        Self::internal(err.to_string())
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.detail)
    }
}
