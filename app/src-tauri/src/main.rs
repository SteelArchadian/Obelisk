// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Started by an AI assistant as its MCP server: no window, just stdio.
    if std::env::args().any(|a| a == "--mcp") {
        return obelisk_lib::mcp_serve();
    }
    if std::env::args().any(|a| a == "--build-id") {
        return println!("{}", obelisk_lib::build_id());
    }
    obelisk_lib::run()
}
