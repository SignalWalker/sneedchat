use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, clap::ValueEnum)]
pub(super) enum LogFormat {
    Compact,
    Full,
    Pretty,
    Json,
}

impl std::fmt::Display for LogFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogFormat::Compact => f.write_str("compact"),
            LogFormat::Full => f.write_str("full"),
            LogFormat::Pretty => f.write_str("pretty"),
            LogFormat::Json => f.write_str("json"),
        }
    }
}

pub(super) fn initialize_tracing(log_filter: impl AsRef<str>, log_format: LogFormat) {
    let tsub = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_timer(tracing_subscriber::fmt::time::OffsetTime::new(
            time::UtcOffset::current_local_offset().unwrap_or_else(|e| {
                tracing::warn!("couldn't get local time offset: {:?}", e);
                time::UtcOffset::UTC
            }),
            time::macros::format_description!("[hour]:[minute]:[second]"),
        ))
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_env_filter(log_filter);

    match log_format {
        LogFormat::Compact => tsub.compact().init(),
        LogFormat::Full => tsub.init(),
        LogFormat::Pretty => tsub.pretty().init(),
        LogFormat::Json => tsub.json().init(),
    }
}

#[derive(Parser, Debug)]
#[command(version, author, about)]
pub(super) struct Cli {
    /// Logging output filters; comma-separated
    #[arg(
        short,
        long,
        default_value = "warn,bonfire=info,bonfire-gui=info",
        env = "BONFIRE_LOG_FILTER"
    )]
    pub(super) log_filter: String,
    /// Logging output format
    #[arg(long, default_value_t = LogFormat::Pretty)]
    pub(super) log_format: LogFormat,
    /// Path to the configuration directory.
    #[arg(long, env = "BONFIRE_CONFIG_DIR")]
    pub(super) config_dir: Option<PathBuf>,
}
