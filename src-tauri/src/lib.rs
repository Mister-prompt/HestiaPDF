//! Application de bureau Hestia PDF (Tauri 2).

mod commands;
mod error;
mod security;

use std::path::PathBuf;

use serde::Serialize;
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, DragDropEvent, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::commands::{AppState, register_files};

/// Événement envoyé à l'interface pendant un glisser-déposer.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DragState {
    active: bool,
}

pub fn run() {
    let startup_files =
        std::env::args_os().skip(1).map(PathBuf::from).filter(|p| p.is_absolute() && p.is_file()).collect::<Vec<_>>();

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let pdfium = locate_pdfium(app.handle());
            app.manage(AppState::new(pdfium, startup_files.clone()));

            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("Hestia PDF")
                .inner_size(1180.0, 800.0)
                .min_inner_size(880.0, 600.0)
                .center()
                // Affichée une fois la page prête : pas de flash blanc au démarrage.
                .visible(false)
                .on_navigation(security::is_allowed_navigation)
                .on_new_window(|_, _| NewWindowResponse::Deny)
                .on_page_load(|window, payload| {
                    if payload.event() == PageLoadEvent::Finished {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                })
                .build()?;

            let handle = app.handle().clone();
            window.on_window_event(move |event| on_window_event(&handle, event));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::pick_files,
            commands::startup_files,
            commands::inspect_pdf,
            commands::render_page,
            commands::image_thumbnail,
            commands::cancel_job,
            commands::reveal_in_folder,
            commands::app_info,
            commands::merge_pdfs,
            commands::split_pdf,
            commands::compress_pdfs,
            commands::pdf_to_jpg,
            commands::jpg_to_pdf,
            commands::edit_pdf,
            commands::office_status,
            commands::office_to_pdf_files,
            commands::pdf_to_office_files,
        ])
        .run(tauri::generate_context!());

    if let Err(err) = result {
        eprintln!("Hestia PDF n'a pas pu démarrer : {err}");
        std::process::exit(1);
    }
}

/// Glisser-déposer depuis l'Explorateur : les fichiers déposés sont
/// autorisés côté Rust puis transmis à l'interface.
fn on_window_event(app: &AppHandle, event: &WindowEvent) {
    let WindowEvent::DragDrop(drag) = event else {
        return;
    };
    match drag {
        DragDropEvent::Enter { .. } => {
            let _ = app.emit_to("main", "hestia://drag", DragState { active: true });
        }
        DragDropEvent::Leave => {
            let _ = app.emit_to("main", "hestia://drag", DragState { active: false });
        }
        DragDropEvent::Drop { paths, .. } => {
            let _ = app.emit_to("main", "hestia://drag", DragState { active: false });
            let state = app.state::<AppState>();
            let files = register_files(&state.allowlist, paths.clone());
            let _ = app.emit_to("main", "hestia://drop", files);
        }
        _ => {}
    }
}

/// Emplacement de PDFium : ressources de l'appli installée, ou à côté de
/// l'exécutable (version portable).
fn locate_pdfium(app: &AppHandle) -> PathBuf {
    let name = hestia_core::engine::library_file_name();
    let mut candidates = Vec::new();
    if let Ok(dir) = app.path().resource_dir() {
        candidates.push(dir.join("pdfium").join(name));
        candidates.push(dir.join("resources").join("pdfium").join(name));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)) {
        candidates.push(dir.join(name));
    }
    let fallback = candidates.first().cloned().unwrap_or_else(|| PathBuf::from(name));
    candidates.into_iter().find(|p| p.is_file()).unwrap_or(fallback)
}
