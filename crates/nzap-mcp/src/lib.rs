//! NZAP Engine as an MCP server (`nzap-engine mcp`).
//!
//! An AI agent (Claude Code, Claude Desktop, Cursor, …) launches the
//! installed app with the `mcp` argument and speaks the Model Context
//! Protocol over stdin/stdout: newline-delimited JSON-RPC 2.0. No port is
//! opened, so nothing else on the machine can reach the server.
//!
//! The server drives the same engine as the app, with the same Google
//! connection (connect once in the app). It keeps its own runtime list,
//! releases the runtimes it started when the agent disconnects, and reads
//! and writes local files only inside the folders it was given.

mod files;
mod logger;
mod options;
mod protocol;
mod server;
mod tools;
mod transcript;

use std::sync::Arc;

pub use files::LocalFiles;
pub use logger::init_stderr_logger;
pub use options::{Options, USAGE};
pub use protocol::{serve, PROTOCOL_VERSIONS};
pub use server::Server;

/// Serve one agent on stdin/stdout until it disconnects or the process is
/// asked to stop, then release what this server started.
pub async fn run_stdio(server: Arc<Server>) -> std::io::Result<()> {
    let served = serve(server.clone(), tokio::io::stdin(), tokio::io::stdout());
    let result = tokio::select! {
        result = served => result,
        () = shutdown_signal() => {
            tracing::info!("Stopping on a signal");
            Ok(())
        }
    };
    server.shutdown().await;
    result
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut term), Ok(mut interrupt), Ok(mut hangup)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
        signal(SignalKind::hangup()),
    ) else {
        tracing::warn!("Cannot listen for stop signals");
        return std::future::pending().await;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = interrupt.recv() => {}
        _ = hangup.recv() => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}
