//! Garde-fous de sécurité côté Rust.
//!
//! L'interface (WebView) ne peut agir que sur des fichiers que
//! l'utilisateur a **explicitement** choisis (boîte de dialogue ou
//! glisser-déposer) et sur les fichiers produits par l'appli. Même si du
//! code malveillant s'exécutait dans l'interface, il ne pourrait ni lire
//! ni écrire ailleurs sur le disque.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::Url;

use crate::error::CommandError;

/// Chemins autorisés pour la session en cours.
#[derive(Default)]
pub struct Allowlist {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Fichiers choisis par l'utilisateur (lecture).
    inputs: HashSet<String>,
    /// Fichiers produits par l'appli (ouverture, affichage dans l'Explorateur).
    outputs: HashSet<String>,
}

/// Clé de comparaison : chemin absolu exact, insensible à la casse sous Windows.
fn key(path: &Path) -> String {
    let s = path.to_string_lossy();
    if cfg!(windows) { s.to_lowercase() } else { s.into_owned() }
}

impl Allowlist {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // Un verrou empoisonné ne contient que des ensembles de chaînes : on continue.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn allow_input(&self, path: &Path) {
        self.lock().inputs.insert(key(path));
    }

    pub fn allow_output(&self, path: &Path) {
        let mut inner = self.lock();
        inner.outputs.insert(key(path));
        // Un PDF produit peut ensuite être ouvert dans la visionneuse ou retraité.
        inner.inputs.insert(key(path));
    }

    pub fn check_input(&self, path: &Path) -> Result<(), CommandError> {
        if path.is_absolute() && self.lock().inputs.contains(&key(path)) {
            Ok(())
        } else {
            Err(CommandError::forbidden(path))
        }
    }

    pub fn check_output(&self, path: &Path) -> Result<(), CommandError> {
        if path.is_absolute() && self.lock().outputs.contains(&key(path)) {
            Ok(())
        } else {
            Err(CommandError::forbidden(path))
        }
    }
}

/// Seules les pages de l'appli elle-même peuvent être affichées : aucune
/// navigation vers Internet ou vers un fichier local n'est permise.
pub fn is_allowed_navigation(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    match url.scheme() {
        "tauri" => host == "localhost",
        "http" | "https" if host == "tauri.localhost" => true,
        // Serveur de développement Vite (uniquement en mode debug).
        "http" if cfg!(debug_assertions) => host == "localhost" && url.port() == Some(1420),
        _ => false,
    }
}

/// Ajoute l'extension `ext` si le nom choisi ne l'a pas déjà.
pub fn with_extension(path: PathBuf, ext: &str) -> PathBuf {
    let has_ext = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(ext));
    if has_ext {
        path
    } else {
        let mut s = path.into_os_string();
        s.push(".");
        s.push(ext);
        PathBuf::from(s)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn navigation_is_restricted_to_the_app() {
        let ok = |s: &str| is_allowed_navigation(&Url::parse(s).expect("url"));
        assert!(ok("tauri://localhost/index.html"));
        assert!(ok("http://tauri.localhost/"));
        assert!(!ok("https://example.com/"));
        assert!(!ok("file:///C:/Windows/win.ini"));
        assert!(!ok("http://tauri.localhost.evil.com/"));
        assert!(!ok("javascript:alert(1)"));
    }

    #[test]
    fn allowlist_requires_exact_paths() {
        let list = Allowlist::default();
        let chosen = if cfg!(windows) { Path::new(r"C:\Docs\Contrat.pdf") } else { Path::new("/docs/Contrat.pdf") };
        list.allow_input(chosen);
        assert!(list.check_input(chosen).is_ok());
        let sneaky =
            if cfg!(windows) { Path::new(r"C:\Docs\..\Windows\win.ini") } else { Path::new("/docs/../etc/passwd") };
        assert!(list.check_input(sneaky).is_err());
        assert!(list.check_input(Path::new("Contrat.pdf")).is_err());
        assert!(list.check_output(chosen).is_err(), "une entrée n'est pas une sortie");
    }

    #[test]
    fn extension_is_added_once() {
        assert_eq!(with_extension(PathBuf::from("a"), "pdf"), PathBuf::from("a.pdf"));
        assert_eq!(with_extension(PathBuf::from("a.PDF"), "pdf"), PathBuf::from("a.PDF"));
        assert_eq!(with_extension(PathBuf::from("a.v2"), "pdf"), PathBuf::from("a.v2.pdf"));
    }
}
