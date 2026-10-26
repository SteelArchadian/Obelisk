// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Started by an AI assistant as its MCP server: no window, just stdio.
    if std::env::args().any(|a| a == "--mcp") {
        return obelisk_lib::mcp_serve();
    }
    // Asked by the installer (clean install) and the uninstaller, while no Obelisk window runs.
    if std::env::args().any(|a| a == "--reset-data") {
        return obelisk_lib::reset_data();
    }
    if std::env::args().any(|a| a == "--uninstall-cleanup") {
        let keep_data = std::env::args().any(|a| a == "--keep-data");
        if let Err(e) = obelisk_lib::uninstall_cleanup(keep_data) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().any(|a| a == "--build-id") {
        return println!("{}", obelisk_lib::build_id());
    }
    obelisk_lib::run()
}
