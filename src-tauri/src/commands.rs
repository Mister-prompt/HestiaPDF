//! Commandes appelées par l'interface.
//!
//! Règles communes :
//! - chaque chemin reçu est vérifié par la liste blanche ([`Allowlist`]) ;
//! - la destination est toujours choisie par l'utilisateur dans une boîte
//!   de dialogue native ouverte **par Rust** (l'interface ne fournit jamais
//!   de chemin de sortie) ;
//! - les traitements lourds tournent hors du thread principal et sont
//!   annulables.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hestia_core::compress::{CompressionLevel, compress};
use hestia_core::edit::{EditDocument, apply_edits};
use hestia_core::images_to_pdf::{ImageInput, ImagesToPdfOptions, images_to_pdf, images_to_pdf_each};
use hestia_core::inspect::{PdfInfo, inspect, render_page_jpeg};
use hestia_core::merge::merge;
use hestia_core::office::{DocKind, OfficeStatus, office_to_pdf, offline_default_printer};
use hestia_core::organize::{PageSpec, organize};
use hestia_core::output::{available_path, file_stem, sanitize_stem};
use hestia_core::pdf_to_images::{ImageExport, pdf_to_images};
use hestia_core::pdf_to_office::{OfficeFormat, pdf_to_office};
use hestia_core::split::{SplitMode, SplitTarget, plan, split};
use hestia_core::{
    CancelToken, HestiaError, IMAGE_EXTENSIONS, Job, OutputFile, PdfEngine, PdfInput, Progress, ProgressFn,
};
use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, Response};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use crate::error::CommandError;
use crate::security::{Allowlist, with_extension};

type CmdResult<T> = Result<T, CommandError>;

/// État partagé de l'application.
pub struct AppState {
    pub pdfium_path: PathBuf,
    pub allowlist: Allowlist,
    pub jobs: Mutex<HashMap<String, CancelToken>>,
    /// Fichiers passés en argument au lancement (« Ouvrir avec »).
    pub startup_files: Mutex<Vec<PathBuf>>,
}

impl AppState {
    pub fn new(pdfium_path: PathBuf, startup_files: Vec<PathBuf>) -> Self {
        Self {
            pdfium_path,
            allowlist: Allowlist::default(),
            jobs: Mutex::new(HashMap::new()),
            startup_files: Mutex::new(startup_files),
        }
    }

    /// Le moteur PDF est démarré au premier besoin : l'appli s'ouvre
    /// instantanément, PDFium n'est chargé que lorsqu'on l'utilise.
    fn engine(&self) -> CmdResult<&'static PdfEngine> {
        PdfEngine::init(&self.pdfium_path).map_err(Into::into)
    }

    fn start_job(&self, id: &str) -> JobGuard<'_> {
        let token = CancelToken::new();
        self.jobs.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), token.clone());
        JobGuard { state: self, id: id.to_string(), token }
    }
}

/// Retire le traitement de la liste des traitements annulables à la fin.
struct JobGuard<'a> {
    state: &'a AppState,
    id: String,
    token: CancelToken,
}

impl JobGuard<'_> {
    fn job(&self, progress: ProgressFn) -> Job {
        Job::new(progress, self.token.clone())
    }
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        self.state.jobs.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
}

// ---------------------------------------------------------------------------
// Types échangés avec l'interface
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    Pdf,
    Image,
    Word,
    Powerpoint,
    Excel,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub kind: FileKind,
}

pub fn classify(path: &Path) -> Option<FileKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext == "pdf" {
        Some(FileKind::Pdf)
    } else if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        Some(FileKind::Image)
    } else {
        DocKind::of(path).map(|k| match k {
            DocKind::Word => FileKind::Word,
            DocKind::Powerpoint => FileKind::Powerpoint,
            DocKind::Excel => FileKind::Excel,
        })
    }
}

/// Décrit un fichier pris en charge (sinon `None`).
pub fn describe(path: &Path) -> Option<FileInfo> {
    let kind = classify(path)?;
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(FileInfo {
        path: path.to_path_buf(),
        name: path.file_name()?.to_string_lossy().into_owned(),
        size: meta.len(),
        kind,
    })
}

/// Enregistre des fichiers choisis par l'utilisateur et les décrit.
pub fn register_files(allowlist: &Allowlist, paths: impl IntoIterator<Item = PathBuf>) -> Vec<FileInfo> {
    paths
        .into_iter()
        .filter(|p| p.is_absolute())
        .filter_map(|p| describe(&p))
        .inspect(|info| allowlist.allow_input(&info.path))
        .collect()
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemOutcome {
    pub name: String,
    pub outputs: Vec<OutputFile>,
    pub error: Option<CommandError>,
    pub original_size: Option<u64>,
    pub compressed_size: Option<u64>,
    pub already_optimal: bool,
}

/// Résultat d'un traitement.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobOutcome {
    /// L'utilisateur a fermé la boîte de dialogue ou annulé.
    pub cancelled: bool,
    pub outputs: Vec<OutputFile>,
    /// Détail par fichier (traitements par lot).
    pub items: Vec<ItemOutcome>,
    /// Dossier de destination (sorties multiples).
    pub folder: Option<PathBuf>,
    /// Un fichier d'entrée était protégé : le résultat ne l'est plus.
    pub had_password: bool,
}

impl JobOutcome {
    fn cancelled() -> Self {
        Self { cancelled: true, ..Default::default() }
    }
}

// ---------------------------------------------------------------------------
// Utilitaires
// ---------------------------------------------------------------------------

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| CommandError::internal(format!("tâche interrompue : {e}")))
}

fn progress_to(channel: Channel<Progress>) -> ProgressFn {
    Arc::new(move |p| {
        let _ = channel.send(p);
    })
}

fn silent_progress() -> ProgressFn {
    Arc::new(|_| {})
}

/// Boîte « Enregistrer sous » native (appel bloquant).
fn ask_save_pdf(window: &WebviewWindow, suggested: &str, near: Option<&Path>) -> Option<PathBuf> {
    ask_save(window, suggested, near, "PDF", "pdf")
}

fn ask_save(window: &WebviewWindow, suggested: &str, near: Option<&Path>, label: &str, ext: &str) -> Option<PathBuf> {
    let mut dialog = window
        .dialog()
        .file()
        .set_parent(window)
        .add_filter(label, &[ext])
        .set_file_name(format!("{}.{ext}", sanitize_stem(suggested)));
    if let Some(dir) = near.filter(|d| d.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    let path = dialog.blocking_save_file()?.into_path().ok()?;
    Some(with_extension(path, ext))
}

/// Boîte de choix d'un dossier (appel bloquant).
fn ask_folder(window: &WebviewWindow, near: Option<&Path>) -> Option<PathBuf> {
    let mut dialog = window.dialog().file().set_parent(window);
    if let Some(dir) = near.filter(|d| d.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    dialog.blocking_pick_folder()?.into_path().ok()
}

fn parent_of(path: &Path) -> Option<PathBuf> {
    path.parent().map(Path::to_path_buf)
}

fn check_inputs<'a>(state: &AppState, paths: impl IntoIterator<Item = &'a Path>) -> CmdResult<()> {
    let mut any = false;
    for p in paths {
        state.allowlist.check_input(p)?;
        any = true;
    }
    if any { Ok(()) } else { Err(HestiaError::NoInput.into()) }
}

/// Convertit le résultat d'un traitement unique en `JobOutcome`.
fn single_outcome(
    state: &AppState,
    result: hestia_core::Result<Vec<OutputFile>>,
    folder: Option<PathBuf>,
    had_password: bool,
) -> CmdResult<JobOutcome> {
    match result {
        Ok(outputs) => {
            for o in &outputs {
                state.allowlist.allow_output(&o.path);
            }
            Ok(JobOutcome { outputs, folder, had_password, ..Default::default() })
        }
        Err(HestiaError::Cancelled) => Ok(JobOutcome::cancelled()),
        Err(e) => Err(e.into()),
    }
}

// ---------------------------------------------------------------------------
// Fichiers
// ---------------------------------------------------------------------------

/// Ouvre la boîte de dialogue « Ouvrir » et autorise les fichiers choisis.
#[tauri::command]
pub async fn pick_files(
    window: WebviewWindow,
    state: State<'_, AppState>,
    kind: FileKind,
    multiple: bool,
) -> CmdResult<Vec<FileInfo>> {
    let w = window.clone();
    let picked = blocking(move || {
        let dialog = w.dialog().file().set_parent(&w);
        let dialog = match kind {
            FileKind::Pdf => dialog.add_filter("PDF", &["pdf"]),
            FileKind::Image => dialog.add_filter("Images", IMAGE_EXTENSIONS),
            FileKind::Word => dialog.add_filter("Word", DocKind::Word.extensions()),
            FileKind::Powerpoint => dialog.add_filter("PowerPoint", DocKind::Powerpoint.extensions()),
            FileKind::Excel => dialog.add_filter("Excel", DocKind::Excel.extensions()),
        };
        if multiple { dialog.blocking_pick_files() } else { dialog.blocking_pick_file().map(|f| vec![f]) }
    })
    .await?;
    let paths = picked.unwrap_or_default().into_iter().filter_map(|f| f.into_path().ok());
    Ok(register_files(&state.allowlist, paths))
}

/// Fichiers passés au lancement (« Ouvrir avec Hestia PDF »).
#[tauri::command]
pub fn startup_files(state: State<'_, AppState>) -> Vec<FileInfo> {
    let paths = std::mem::take(&mut *state.startup_files.lock().unwrap_or_else(|e| e.into_inner()));
    register_files(&state.allowlist, paths)
}

#[tauri::command]
pub async fn inspect_pdf(state: State<'_, AppState>, input: PdfInput) -> CmdResult<PdfInfo> {
    state.allowlist.check_input(&input.path)?;
    let engine = state.engine()?;
    let path = input.path.clone();
    blocking(move || inspect(engine, input)).await?.map_err(|e| e.in_file(&path).into())
}

/// Rendu JPEG d'une page (miniatures, visionneuse), renvoyé en binaire.
#[tauri::command]
pub async fn render_page(state: State<'_, AppState>, input: PdfInput, page: u32, width: u32) -> CmdResult<Response> {
    state.allowlist.check_input(&input.path)?;
    let engine = state.engine()?;
    let bytes = blocking(move || render_page_jpeg(engine, input, page, width)).await??;
    Ok(Response::new(bytes))
}

/// Miniature d'une image (outil « JPG en PDF »), renvoyée en binaire.
#[tauri::command]
pub async fn image_thumbnail(state: State<'_, AppState>, path: PathBuf, size: u32) -> CmdResult<Response> {
    state.allowlist.check_input(&path)?;
    let p = path.clone();
    let bytes = blocking(move || hestia_core::imaging::thumbnail_jpeg(&p, size))
        .await?
        .map_err(|e| CommandError::from(e.in_file(&path)))?;
    Ok(Response::new(bytes))
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, job_id: String) {
    if let Some(token) = state.jobs.lock().unwrap_or_else(|e| e.into_inner()).get(&job_id) {
        token.cancel();
    }
}

/// Affiche un fichier produit dans l'Explorateur Windows.
#[tauri::command]
pub fn reveal_in_folder(state: State<'_, AppState>, path: PathBuf) -> CmdResult<()> {
    state.allowlist.check_output(&path)?;
    reveal(&path)
}

#[cfg(windows)]
fn reveal(path: &Path) -> CmdResult<()> {
    use std::os::windows::process::CommandExt;
    // Un chemin Windows ne peut pas contenir de guillemet : la citation est sûre.
    let arg = format!("/select,\"{}\"", path.display());
    std::process::Command::new("explorer.exe")
        .raw_arg(arg)
        .spawn()
        .map(|_| ())
        .map_err(|e| CommandError::new("io", e.to_string()))
}

#[cfg(not(windows))]
fn reveal(_path: &Path) -> CmdResult<()> {
    Err(CommandError::new("unsupported", "non disponible sur ce système"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo { version: env!("CARGO_PKG_VERSION") }
}

// ---------------------------------------------------------------------------
// Outils
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn merge_pdfs(
    window: WebviewWindow,
    state: State<'_, AppState>,
    job_id: String,
    inputs: Vec<PdfInput>,
    suggested_name: String,
    on_progress: Channel<Progress>,
) -> CmdResult<JobOutcome> {
    check_inputs(&state, inputs.iter().map(|i| i.path.as_path()))?;
    let engine = state.engine()?;
    let near = parent_of(&inputs[0].path);
    let w = window.clone();
    let Some(output) = blocking(move || ask_save_pdf(&w, &suggested_name, near.as_deref())).await? else {
        return Ok(JobOutcome::cancelled());
    };
    let had_password = inputs.iter().any(|i| i.password().is_some());
    let guard = state.start_job(&job_id);
    let job = guard.job(progress_to(on_progress));
    let result = blocking(move || merge(engine, inputs, output, job).map(|o| vec![o])).await?;
    single_outcome(&state, result, None, had_password)
}

#[tauri::command]
pub async fn organize_pdf(
    window: WebviewWindow,
    state: State<'_, AppState>,
    job_id: String,
    inputs: Vec<PdfInput>,
    pages: Vec<PageSpec>,
    suggested_name: String,
    on_progress: Channel<Progress>,
) -> CmdResult<JobOutcome> {
    check_inputs(&state, inputs.iter().map(|i| i.path.as_path()))?;
    let engine = state.engine()?;
    let near = parent_of(&inputs[0].path);
    let w = window.clone();
    let Some(output) = blocking(move || ask_save_pdf(&w, &suggested_name, near.as_deref())).await? else {
        return Ok(JobOutcome::cancelled());
    };
    let had_password = inputs.iter().any(|i| i.password().is_some());
    let guard = state.start_job(&job_id);
    let job = guard.job(progress_to(on_progress));
    let result = blocking(move || organize(engine, inputs, pages, output, job).map(|o| vec![o])).await?;
    single_outcome(&state, result, None, had_password)
}

#[tauri::command]
pub async fn split_pdf(
    window: WebviewWindow,
    state: State<'_, AppState>,
    job_id: String,
    input: PdfInput,
    mode: SplitMode,
    on_progress: Channel<Progress>,
) -> CmdResult<JobOutcome> {
    state.allowlist.check_input(&input.path)?;
    let engine = state.engine()?;
    let probe = input.clone();
    let info = blocking(move || inspect(engine, probe)).await??;
    let parts = plan(info.page_count, &mode)?;

    let stem = file_stem(&input.path);
    let near = parent_of(&input.path);
    let w = window.clone();
    let (target, folder) = if parts.len() == 1 {
        let suggested = format!("{stem}_{}", parts[0].label);
        match blocking(move || ask_save_pdf(&w, &suggested, near.as_deref())).await? {
            Some(path) => (SplitTarget::File(path), None),
            None => return Ok(JobOutcome::cancelled()),
        }
    } else {
        match blocking(move || ask_folder(&w, near.as_deref())).await? {
            Some(dir) => (SplitTarget::Folder { dir: dir.clone(), stem }, Some(dir)),
            None => return Ok(JobOutcome::cancelled()),
        }
    };
    let had_password = input.password().is_some();
    let guard = state.start_job(&job_id);
    let job = guard.job(progress_to(on_progress));
    let result = blocking(move || split(engine, input, parts, target, job)).await?;
    single_outcome(&state, result, folder, had_password)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn compress_pdfs(
    window: WebviewWindow,
    state: State<'_, AppState>,
    job_id: String,
    inputs: Vec<PdfInput>,
    level: CompressionLevel,
    suffix: String,
    on_progress: Channel<Progress>,
) -> CmdResult<JobOutcome> {
    check_inputs(&state, inputs.iter().map(|i| i.path.as_path()))?;
    let engine = state.engine()?;
    let suffix = sanitize_stem(&suffix);
    let near = parent_of(&inputs[0].path);
    let had_password = inputs.iter().any(|i| i.password().is_some());
    let w = window.clone();

    // Un seul fichier : « Enregistrer sous ». Plusieurs : un dossier.
    if inputs.len() == 1 {
        let input = inputs.into_iter().next().ok_or_else(|| CommandError::from(HestiaError::NoInput))?;
        let suggested = format!("{}{suffix}", file_stem(&input.path));
        let Some(output) = blocking(move || ask_save_pdf(&w, &suggested, near.as_deref())).await? else {
            return Ok(JobOutcome::cancelled());
        };
        let guard = state.start_job(&job_id);
        let job = guard.job(progress_to(on_progress));
        let name = file_name(&input.path);
        let result = blocking(move || compress(engine, input, level, output, job)).await?;
        return match result {
            Ok(report) => {
                state.allowlist.allow_output(&report.output.path);
                Ok(JobOutcome {
                    outputs: vec![report.output.clone()],
                    items: vec![ItemOutcome {
                        name,
                        outputs: vec![report.output],
                        original_size: Some(report.original_size),
                        compressed_size: Some(report.compressed_size),
                        already_optimal: report.already_optimal,
                        error: None,
                    }],
                    had_password,
                    ..Default::default()
                })
            }
            Err(HestiaError::Cancelled) => Ok(JobOutcome::cancelled()),
            Err(e) => Err(e.into()),
        };
    }

    let Some(dir) = blocking(move || ask_folder(&w, near.as_deref())).await? else {
        return Ok(JobOutcome::cancelled());
    };
    let guard = state.start_job(&job_id);
    let token = guard.token.clone();
    let progress = progress_to(on_progress);
    let folder = dir.clone();
    let items = blocking(move || {
        let total = inputs.len() as u32;
        let mut items = Vec::with_capacity(inputs.len());
        for (i, input) in inputs.into_iter().enumerate() {
            if token.is_cancelled() {
                break;
            }
            let name = file_name(&input.path);
            let output = available_path(&dir, &format!("{}{suffix}", file_stem(&input.path)), "pdf");
            let path = input.path.clone();
            let job = Job::new(silent_progress(), token.clone());
            let item = match compress(engine, input, level, output, job) {
                Ok(r) => ItemOutcome {
                    name,
                    outputs: vec![r.output],
                    original_size: Some(r.original_size),
                    compressed_size: Some(r.compressed_size),
                    already_optimal: r.already_optimal,
                    error: None,
                },
                Err(HestiaError::Cancelled) => break,
                Err(e) => ItemOutcome { name, error: Some(e.in_file(&path).into()), ..Default::default() },
            };
            items.push(item);
            progress(Progress { done: i as u32 + 1, total });
        }
        items
    })
    .await?;
    Ok(batch_outcome(&state, items, folder, had_password, guard.token.is_cancelled()))
}

#[tauri::command]
pub async fn pdf_to_jpg(
    window: WebviewWindow,
    state: State<'_, AppState>,
    job_id: String,
    inputs: Vec<PdfInput>,
    mode: ImageExport,
    on_progress: Channel<Progress>,
) -> CmdResult<JobOutcome> {
    check_inputs(&state, inputs.iter().map(|i| i.path.as_path()))?;
    let engine = state.engine()?;
    let near = parent_of(&inputs[0].path);
    let had_password = inputs.iter().any(|i| i.password().is_some());
    let w = window.clone();
    let Some(dir) = blocking(move || ask_folder(&w, near.as_deref())).await? else {
        return Ok(JobOutcome::cancelled());
    };
    let guard = state.start_job(&job_id);
    let token = guard.token.clone();
    let progress = progress_to(on_progress);
    let folder = dir.clone();
    let items = blocking(move || {
        let single = inputs.len() == 1;
        let total = inputs.len() as u32;
        let mut items = Vec::with_capacity(inputs.len());
        for (i, input) in inputs.into_iter().enumerate() {
            if token.is_cancelled() {
                break;
            }
            let name = file_name(&input.path);
            let path = input.path.clone();
            let stem = file_stem(&input.path);
            // Un seul PDF : progression page par page ; sinon fichier par fichier.
            let job = Job::new(if single { progress.clone() } else { silent_progress() }, token.clone());
            let item = match pdf_to_images(engine, input, mode, dir.clone(), stem, job) {
                Ok(outputs) => ItemOutcome { name, outputs, ..Default::default() },
                Err(HestiaError::Cancelled) => break,
                Err(e) => ItemOutcome { name, error: Some(e.in_file(&path).into()), ..Default::default() },
            };
            items.push(item);
            if !single {
                progress(Progress { done: i as u32 + 1, total });
            }
        }
        items
    })
    .await?;
    Ok(batch_outcome(&state, items, folder, had_password, guard.token.is_cancelled()))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn jpg_to_pdf(
    window: WebviewWindow,
    state: State<'_, AppState>,
    job_id: String,
    inputs: Vec<ImageInput>,
    options: ImagesToPdfOptions,
    merge_all: bool,
    suggested_name: String,
    on_progress: Channel<Progress>,
) -> CmdResult<JobOutcome> {
    check_inputs(&state, inputs.iter().map(|i| i.path.as_path()))?;
    let near = parent_of(&inputs[0].path);
    let w = window.clone();

    if merge_all || inputs.len() == 1 {
        let suggested = if inputs.len() == 1 { file_stem(&inputs[0].path) } else { suggested_name };
        let Some(output) = blocking(move || ask_save_pdf(&w, &suggested, near.as_deref())).await? else {
            return Ok(JobOutcome::cancelled());
        };
        let guard = state.start_job(&job_id);
        let job = guard.job(progress_to(on_progress));
        let result = blocking(move || images_to_pdf(inputs, options, output, job).map(|o| vec![o])).await?;
        return single_outcome(&state, result, None, false);
    }

    let Some(dir) = blocking(move || ask_folder(&w, near.as_deref())).await? else {
        return Ok(JobOutcome::cancelled());
    };
    let guard = state.start_job(&job_id);
    let job = guard.job(progress_to(on_progress));
    let folder = dir.clone();
    let result = blocking(move || images_to_pdf_each(inputs, options, dir, job)).await?;
    single_outcome(&state, result, Some(folder), false)
}

/// « Modifier PDF » : applique les éléments ajoutés dans l'éditeur.
#[tauri::command]
pub async fn edit_pdf(
    window: WebviewWindow,
    state: State<'_, AppState>,
    job_id: String,
    input: PdfInput,
    edits: EditDocument,
    suffix: String,
    on_progress: Channel<Progress>,
) -> CmdResult<EditOutcome> {
    state.allowlist.check_input(&input.path)?;
    // Les images insérées doivent avoir été choisies par l'utilisateur.
    for path in edits.image_paths() {
        state.allowlist.check_input(path)?;
    }
    if edits.object_count() == 0 {
        return Err(HestiaError::NoInput.into());
    }
    let engine = state.engine()?;
    let near = parent_of(&input.path);
    let suggested = format!("{}{}", file_stem(&input.path), sanitize_stem(&suffix));
    let w = window.clone();
    let Some(output) = blocking(move || ask_save_pdf(&w, &suggested, near.as_deref())).await? else {
        return Ok(EditOutcome::default());
    };
    let had_password = input.password().is_some();
    let guard = state.start_job(&job_id);
    let job = guard.job(progress_to(on_progress));
    let app = window.app_handle().clone();
    let fonts = move |name: &str| bundled_font(&app, name);
    let result = blocking(move || apply_edits(engine, input, edits, output, job, &fonts)).await?;
    match result {
        Ok(report) => {
            state.allowlist.allow_output(&report.output.path);
            Ok(EditOutcome {
                job: JobOutcome { outputs: vec![report.output], had_password, ..Default::default() },
                replaced_chars: report.replaced_chars,
                rebuilt: report.rebuilt,
            })
        }
        Err(HestiaError::Cancelled) => Ok(EditOutcome::default()),
        Err(e) => Err(e.into()),
    }
}

// ---------------------------------------------------------------------------
// Conversions Office
// ---------------------------------------------------------------------------

/// Logiciels disponibles pour les conversions, et imprimante hors ligne éventuelle.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficeInfo {
    #[serde(flatten)]
    status: OfficeStatus,
    offline_printer: Option<String>,
}

#[tauri::command]
pub async fn office_status() -> CmdResult<OfficeInfo> {
    blocking(|| {
        let printer = std::thread::spawn(offline_default_printer);
        let status = OfficeStatus::detect();
        OfficeInfo { status, offline_printer: printer.join().ok().flatten() }
    })
    .await
}

/// Word, PowerPoint ou Excel → PDF.
#[tauri::command]
pub async fn office_to_pdf_files(
    window: WebviewWindow,
    state: State<'_, AppState>,
    job_id: String,
    inputs: Vec<PathBuf>,
    on_progress: Channel<Progress>,
) -> CmdResult<JobOutcome> {
    check_inputs(&state, inputs.iter().map(PathBuf::as_path))?;
    let engine = state.engine()?;
    let near = parent_of(&inputs[0]);
    let w = window.clone();
    let status = blocking(OfficeStatus::detect).await?;

    if inputs.len() == 1 {
        let input = inputs[0].clone();
        let suggested = file_stem(&input);
        let Some(output) = blocking(move || ask_save_pdf(&w, &suggested, near.as_deref())).await? else {
            return Ok(JobOutcome::cancelled());
        };
        let guard = state.start_job(&job_id);
        let job = guard.job(progress_to(on_progress));
        let result =
            blocking(move || office_to_pdf(engine, &status, &input, &output, &job).map_err(|e| e.in_file(&input)))
                .await?;
        return single_outcome(&state, result.map(|o| vec![o]), None, false);
    }

    let Some(dir) = blocking(move || ask_folder(&w, near.as_deref())).await? else {
        return Ok(JobOutcome::cancelled());
    };
    let guard = state.start_job(&job_id);
    let token = guard.token.clone();
    let progress = progress_to(on_progress);
    let folder = dir.clone();
    let items = blocking(move || {
        let total = inputs.len() as u32;
        progress(Progress { done: 0, total });
        let mut items = Vec::with_capacity(inputs.len());
        for (i, input) in inputs.into_iter().enumerate() {
            if token.is_cancelled() {
                break;
            }
            let name = file_name(&input);
            let output = available_path(&dir, &file_stem(&input), "pdf");
            let job = Job::new(silent_progress(), token.clone());
            let item = match office_to_pdf(engine, &status, &input, &output, &job) {
                Ok(out) => ItemOutcome { name, outputs: vec![out], ..Default::default() },
                Err(HestiaError::Cancelled) => break,
                Err(e) => ItemOutcome { name, error: Some(e.in_file(&input).into()), ..Default::default() },
            };
            items.push(item);
            progress(Progress { done: i as u32 + 1, total });
        }
        items
    })
    .await?;
    Ok(batch_outcome(&state, items, folder, false, guard.token.is_cancelled()))
}

/// Résultat de PDF → Office : résultat commun + avertissement « PDF scanné ».
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfToOfficeOutcome {
    #[serde(flatten)]
    job: JobOutcome,
    /// Au moins un PDF ne contenait aucun texte (scan) : seules des images ont été reprises.
    no_text: bool,
}

/// PDF → Word, PowerPoint ou Excel.
#[tauri::command]
pub async fn pdf_to_office_files(
    window: WebviewWindow,
    state: State<'_, AppState>,
    job_id: String,
    inputs: Vec<PdfInput>,
    format: OfficeFormat,
    on_progress: Channel<Progress>,
) -> CmdResult<PdfToOfficeOutcome> {
    check_inputs(&state, inputs.iter().map(|i| i.path.as_path()))?;
    let engine = state.engine()?;
    let near = parent_of(&inputs[0].path);
    let had_password = inputs.iter().any(|i| i.password().is_some());
    let ext = format.extension();
    let label = match format {
        OfficeFormat::Docx => "Word",
        OfficeFormat::Pptx => "PowerPoint",
        OfficeFormat::Xlsx => "Excel",
    };
    let w = window.clone();

    if inputs.len() == 1 {
        let input = inputs.into_iter().next().ok_or_else(|| CommandError::from(HestiaError::NoInput))?;
        let suggested = file_stem(&input.path);
        let Some(output) = blocking(move || ask_save(&w, &suggested, near.as_deref(), label, ext)).await? else {
            return Ok(PdfToOfficeOutcome { job: JobOutcome::cancelled(), no_text: false });
        };
        let guard = state.start_job(&job_id);
        let job = guard.job(progress_to(on_progress));
        let path = input.path.clone();
        let result = blocking(move || pdf_to_office(engine, input, format, &output, &job)).await?;
        return match result {
            Ok(report) => {
                state.allowlist.allow_output(&report.output.path);
                Ok(PdfToOfficeOutcome {
                    job: JobOutcome { outputs: vec![report.output], had_password, ..Default::default() },
                    no_text: report.no_text,
                })
            }
            Err(HestiaError::Cancelled) => Ok(PdfToOfficeOutcome { job: JobOutcome::cancelled(), no_text: false }),
            Err(e) => Err(e.in_file(&path).into()),
        };
    }

    let Some(dir) = blocking(move || ask_folder(&w, near.as_deref())).await? else {
        return Ok(PdfToOfficeOutcome { job: JobOutcome::cancelled(), no_text: false });
    };
    let guard = state.start_job(&job_id);
    let token = guard.token.clone();
    let progress = progress_to(on_progress);
    let folder = dir.clone();
    let (items, no_text) = blocking(move || {
        let total = inputs.len() as u32;
        let mut items = Vec::with_capacity(inputs.len());
        let mut no_text = false;
        for (i, input) in inputs.into_iter().enumerate() {
            if token.is_cancelled() {
                break;
            }
            let name = file_name(&input.path);
            let path = input.path.clone();
            let output = available_path(&dir, &file_stem(&input.path), ext);
            let job = Job::new(silent_progress(), token.clone());
            let item = match pdf_to_office(engine, input, format, &output, &job) {
                Ok(r) => {
                    no_text |= r.no_text;
                    ItemOutcome { name, outputs: vec![r.output], ..Default::default() }
                }
                Err(HestiaError::Cancelled) => break,
                Err(e) => ItemOutcome { name, error: Some(e.in_file(&path).into()), ..Default::default() },
            };
            items.push(item);
            progress(Progress { done: i as u32 + 1, total });
        }
        (items, no_text)
    })
    .await?;
    Ok(PdfToOfficeOutcome {
        job: batch_outcome(&state, items, folder, had_password, guard.token.is_cancelled()),
        no_text,
    })
}

/// Police livrée avec l'interface (`public/fonts`, intégrée à l'exécutable).
/// `name` vient du catalogue de hestia-core, jamais de l'interface.
fn bundled_font(app: &AppHandle, name: &str) -> Option<Vec<u8>> {
    // En développement : lue dans le projet, toujours à jour.
    #[cfg(debug_assertions)]
    if let Ok(bytes) = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../public/fonts").join(name)) {
        return Some(bytes);
    }
    app.asset_resolver().get(format!("fonts/{name}")).map(|asset| asset.bytes)
}

/// Résultat de « Modifier PDF » : résultat commun + avertissements propres.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditOutcome {
    #[serde(flatten)]
    pub job: JobOutcome,
    pub replaced_chars: u32,
    pub rebuilt: bool,
}

impl Default for EditOutcome {
    fn default() -> Self {
        Self { job: JobOutcome::cancelled(), replaced_chars: 0, rebuilt: false }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn batch_outcome(
    state: &AppState,
    items: Vec<ItemOutcome>,
    folder: PathBuf,
    had_password: bool,
    cancelled: bool,
) -> JobOutcome {
    let outputs: Vec<OutputFile> = items.iter().flat_map(|i| i.outputs.clone()).collect();
    for o in &outputs {
        state.allowlist.allow_output(&o.path);
    }
    JobOutcome { cancelled: cancelled && outputs.is_empty(), outputs, items, folder: Some(folder), had_password }
}
