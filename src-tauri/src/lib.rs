mod commands;
mod managers;
mod models;
mod runner;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::list_packages,
            commands::list_residues,
            commands::get_uninstall_plan,
            commands::get_residue_plan,
            commands::get_package_icon,
            commands::get_system_info,
            commands::preview_orphans,
            commands::clean_orphans,
            commands::execute_uninstall,
            commands::execute_residue_cleanup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
