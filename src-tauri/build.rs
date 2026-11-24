// Les commandes de l'appli sont déclarées ici : chacune devient une
// permission (`allow-<commande>`) qui doit être accordée explicitement à la
// fenêtre dans `capabilities/main.json`. Une commande non listée est refusée.
const COMMANDS: &[&str] = &[
    "pick_files",
    "startup_files",
    "inspect_pdf",
    "render_page",
    "image_thumbnail",
    "cancel_job",
    "reveal_in_folder",
    "app_info",
    "merge_pdfs",
    "organize_pdf",
    "split_pdf",
    "compress_pdfs",
    "pdf_to_jpg",
    "jpg_to_pdf",
    "edit_pdf",
    "office_status",
    "office_to_pdf_files",
    "pdf_to_office_files",
];

fn main() {
    let attributes = tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if let Err(err) = tauri_build::try_build(attributes) {
        panic!("échec de la préparation Tauri : {err:#}");
    }
}
