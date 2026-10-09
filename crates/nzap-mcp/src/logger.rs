//! Logs go to stderr: stdout carries the protocol. MCP clients keep a
//! server's stderr in their own logs (`claude --debug`, Claude Desktop's
//! `mcp-server-*.log`).

use std::io::Write as _;

struct StderrLogger {
    level: log::LevelFilter,
}

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        let chatty = ["hyper", "rustls", "tungstenite", "tokio_tungstenite", "reqwest"]
            .iter()
            .any(|prefix| metadata.target().starts_with(prefix));
        metadata.level() <= if chatty { log::LevelFilter::Warn } else { self.level }
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            let _ = writeln!(std::io::stderr(), "[nzap-mcp {}] {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

/// Install the stderr logger (`NZAP_LOG=debug` for more detail). The
/// engine's `tracing` events reach it through `log`.
pub fn init_stderr_logger() {
    let level = match std::env::var("NZAP_LOG").as_deref() {
        Ok("debug") => log::LevelFilter::Debug,
        Ok("trace") => log::LevelFilter::Trace,
        Ok("warn") => log::LevelFilter::Warn,
        Ok("error") => log::LevelFilter::Error,
        _ => log::LevelFilter::Info,
    };
    static LOGGER: std::sync::OnceLock<StderrLogger> = std::sync::OnceLock::new();
    if log::set_logger(LOGGER.get_or_init(|| StderrLogger { level })).is_ok() {
        log::set_max_level(level);
    }
}
