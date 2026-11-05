// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `Mirona mcp`: serve meetings to MCP clients over stdio, no window. The app binary is
    // what the Settings snippet points Claude at, so every install has a working server.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        if let Err(e) = mirona_mcp::serve() {
            eprintln!("mirona: {e}");
            std::process::exit(1);
        }
        return;
    }
    mirona_app_lib::run();
}
