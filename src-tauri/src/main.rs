// Release builds on Windows must not open a console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let mut args = std::env::args().skip(1);
    // `nzap-engine mcp`: an AI agent's MCP server on stdin/stdout, no window.
    if args.next().as_deref() == Some("mcp") {
        std::process::exit(nzap_engine_lib::run_mcp(args.collect()));
    }
    nzap_engine_lib::run();
}
