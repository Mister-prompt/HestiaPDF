//! Moteur PDFium.
//!
//! PDFium n'est pas conçu pour être appelé depuis plusieurs threads : toutes
//! les opérations passent donc par **un thread dédié** qui possède la
//! bibliothèque et un petit cache de documents ouverts (pour les aperçus).
//! Les autres threads envoient des tâches et attendent leur résultat.
//!
//! Une tâche qui panique n'arrête pas le moteur : la panique est rattrapée,
//! le cache est vidé et l'appelant reçoit une erreur.

use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};
use std::time::SystemTime;

use pdfium_render::prelude::*;
use serde::Deserialize;

use crate::error::{HestiaError, Result};

/// Nombre de documents gardés ouverts pour les aperçus.
const CACHE_CAPACITY: usize = 4;

/// Nom du fichier de la bibliothèque PDFium sur ce système.
pub fn library_file_name() -> &'static str {
    if cfg!(windows) {
        "pdfium.dll"
    } else if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else {
        "libpdfium.so"
    }
}

/// Un PDF à traiter, avec son mot de passe éventuel.
#[derive(Clone, Deserialize)]
pub struct PdfInput {
    pub path: PathBuf,
    #[serde(default)]
    pub password: Option<String>,
}

impl PdfInput {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), password: None }
    }

    pub fn with_password(path: impl Into<PathBuf>, password: impl Into<String>) -> Self {
        Self { path: path.into(), password: Some(password.into()) }
    }

    /// Mot de passe non vide, ou `None`.
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref().filter(|p| !p.is_empty())
    }
}

// Ne jamais afficher un mot de passe dans les journaux.
impl fmt::Debug for PdfInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PdfInput").field("path", &self.path).field("password", &self.password().map(|_| "***")).finish()
    }
}

type Task = Box<dyn FnOnce(&mut EngineCtx) + Send + 'static>;

/// Accès au thread PDFium.
pub struct PdfEngine {
    tx: mpsc::Sender<Task>,
}

static ENGINE: OnceLock<std::result::Result<PdfEngine, String>> = OnceLock::new();

impl PdfEngine {
    /// Démarre le moteur au premier appel (les appels suivants renvoient
    /// l'instance existante). `library_path` est le chemin de `pdfium.dll`.
    pub fn init(library_path: &Path) -> Result<&'static PdfEngine> {
        ENGINE
            .get_or_init(|| PdfEngine::start(library_path.to_path_buf()))
            .as_ref()
            .map_err(|e| HestiaError::EngineUnavailable(e.clone()))
    }

    /// Instance déjà démarrée, le cas échéant.
    pub fn get() -> Option<&'static PdfEngine> {
        ENGINE.get().and_then(|r| r.as_ref().ok())
    }

    fn start(library_path: PathBuf) -> std::result::Result<PdfEngine, String> {
        let (tx, rx) = mpsc::channel::<Task>();
        let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<(), String>>();

        std::thread::Builder::new()
            .name("hestia-pdfium".into())
            .spawn(move || {
                let bindings = match Pdfium::bind_to_library(&library_path) {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("impossible de charger {} ({e})", library_path.display())));
                        return;
                    }
                };
                // Une seule instance pour toute la vie du processus.
                let pdfium: &'static Pdfium = Box::leak(Box::new(Pdfium::new(bindings)));
                let _ = ready_tx.send(Ok(()));

                let mut ctx = EngineCtx { pdfium, cache: Vec::new() };
                for task in rx {
                    let outcome = panic::catch_unwind(AssertUnwindSafe(|| task(&mut ctx)));
                    if outcome.is_err() {
                        // L'état des documents ouverts n'est plus fiable.
                        ctx.cache.clear();
                    }
                }
            })
            .map_err(|e| format!("impossible de démarrer le thread PDF : {e}"))?;

        ready_rx.recv().map_err(|_| "le thread PDF s'est arrêté au démarrage".to_string())??;
        Ok(PdfEngine { tx })
    }

    /// Exécute `f` sur le thread PDFium et attend son résultat.
    pub fn run<R, F>(&self, f: F) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut EngineCtx) -> Result<R> + Send + 'static,
    {
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        self.tx
            .send(Box::new(move |ctx| {
                let _ = result_tx.send(f(ctx));
            }))
            .map_err(|_| HestiaError::EngineUnavailable("le thread PDF est arrêté".into()))?;
        result_rx
            .recv()
            .map_err(|_| HestiaError::Internal("le traitement PDF s'est interrompu de façon inattendue".into()))?
    }
}

#[derive(Clone, PartialEq, Eq)]
struct DocKey {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

impl DocKey {
    fn of(path: &Path) -> Result<Self> {
        let meta = std::fs::metadata(path)?;
        Ok(Self { path: path.to_path_buf(), len: meta.len(), modified: meta.modified().ok() })
    }
}

struct CachedDoc {
    key: DocKey,
    doc: PdfDocument<'static>,
}

/// État du thread PDFium, accessible aux tâches.
pub struct EngineCtx {
    pdfium: &'static Pdfium,
    /// Documents ouverts, le plus récemment utilisé en dernier.
    cache: Vec<CachedDoc>,
}

impl EngineCtx {
    pub fn pdfium(&self) -> &'static Pdfium {
        self.pdfium
    }

    /// Ouvre un document pour un traitement (hors cache).
    pub fn load(&self, input: &PdfInput) -> Result<PdfDocument<'static>> {
        load_document(self.pdfium, input)
    }

    /// Document ouvert depuis le cache (aperçus, visionneuse). Le cache est
    /// invalidé automatiquement si le fichier a changé sur le disque.
    pub fn cached(&mut self, input: &PdfInput) -> Result<&PdfDocument<'static>> {
        let key = DocKey::of(&input.path)?;
        if let Some(pos) = self.cache.iter().position(|c| c.key == key) {
            let entry = self.cache.remove(pos);
            self.cache.push(entry);
        } else {
            self.cache.retain(|c| c.key.path != key.path);
            let doc = load_document(self.pdfium, input)?;
            if self.cache.len() >= CACHE_CAPACITY {
                self.cache.remove(0);
            }
            self.cache.push(CachedDoc { key, doc });
        }
        self.cache.last().map(|c| &c.doc).ok_or_else(|| HestiaError::Internal("cache vide".into()))
    }

    /// Ferme le document `path` s'il est en cache (avant de l'écraser, etc.).
    pub fn evict(&mut self, path: &Path) {
        self.cache.retain(|c| c.key.path != path);
    }

    /// Ferme tous les documents du cache.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

fn load_document(pdfium: &'static Pdfium, input: &PdfInput) -> Result<PdfDocument<'static>> {
    match pdfium.load_pdf_from_file(&input.path, input.password()) {
        Ok(doc) => Ok(doc),
        Err(PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError)) => {
            if input.password().is_some() {
                Err(HestiaError::WrongPassword)
            } else {
                Err(HestiaError::PasswordRequired)
            }
        }
        Err(e) => Err(e.into()),
    }
}
