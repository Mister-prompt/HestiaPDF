//! Suivi de progression et annulation des traitements longs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::error::{HestiaError, Result};

/// Jeton d'annulation partagé entre l'interface et le traitement.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Renvoie `Err(Cancelled)` si l'utilisateur a demandé l'arrêt.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() { Err(HestiaError::Cancelled) } else { Ok(()) }
    }
}

/// Avancement d'un traitement : `done` étapes sur `total`.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct Progress {
    pub done: u32,
    pub total: u32,
}

/// Fonction appelée à chaque étape franchie (doit être peu coûteuse).
pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync + 'static>;

/// Rapporteur qui ne fait rien (tests, appels internes).
pub fn no_progress() -> ProgressFn {
    Arc::new(|_| {})
}

/// Contexte commun à un traitement : progression + annulation.
#[derive(Clone)]
pub struct Job {
    pub progress: ProgressFn,
    pub cancel: CancelToken,
}

impl Job {
    pub fn new(progress: ProgressFn, cancel: CancelToken) -> Self {
        Self { progress, cancel }
    }

    /// Job sans suivi (tests, appels internes).
    pub fn silent() -> Self {
        Self::new(no_progress(), CancelToken::new())
    }

    pub fn report(&self, done: u32, total: u32) {
        (self.progress)(Progress { done, total });
    }

    pub fn check(&self) -> Result<()> {
        self.cancel.check()
    }
}
