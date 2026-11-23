// Pas de console supplémentaire sous Windows en production. NE PAS RETIRER.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    hestia_pdf_lib::run()
}
