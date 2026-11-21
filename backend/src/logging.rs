//! Where the logs go. Always to stderr, so what a command prints on stdout
//! (`feed-server readers > readers.md`, `status`) stays clean. The commands
//! that keep running (`serve`, `watch`) also write one file a day under
//! `[log] dir` (`feed-server.2026-11-21.log`), keeping the last `keep_days`.
//!
//! `RUST_LOG` picks what is logged, as before (e.g. `RUST_LOG=debug`).

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

use crate::config::LogConfig;

/// html5ever warns about every odd page it parses, and the TLS verifier logs
/// each bad certificate as an error; both are per-site problems that the
/// crawler records on the blog (last_error) anyway.
const DEFAULT_FILTER: &str = "info,sqlx=warn,tower_http=info,html5ever=error,rustls_platform_verifier=off";

/// Starts logging. Keep the guard until the program ends: it writes out
/// what is still buffered for the file.
pub fn init(cfg: &LogConfig, to_file: bool) -> Option<WorkerGuard> {
    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| DEFAULT_FILTER.into());
    let stderr = fmt::layer().with_writer(std::io::stderr).with_filter(filter());
    let opened = (to_file && cfg.keep_days > 0).then(|| {
        Builder::new()
            .rotation(Rotation::DAILY)
            .filename_prefix("feed-server")
            .filename_suffix("log")
            .max_log_files(cfg.keep_days)
            .build(&cfg.dir)
    });
    let (file, guard, failed) = match opened {
        Some(Ok(appender)) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = fmt::layer().with_ansi(false).with_writer(writer).with_filter(filter());
            (Some(layer), Some(guard), None)
        }
        Some(Err(e)) => (None, None, Some(e)),
        None => (None, None, None),
    };
    tracing_subscriber::registry().with(stderr).with(file).init();
    match (&guard, failed) {
        (Some(_), _) => {
            tracing::info!(dir = %cfg.dir.display(), days = cfg.keep_days, "logs also go to a file a day there")
        }
        (None, Some(e)) => {
            tracing::warn!(dir = %cfg.dir.display(), error = %e, "cannot write log files; logging to stderr only")
        }
        _ => {}
    }
    guard
}
