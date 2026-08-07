use std::io::IsTerminal;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

const SELF_TARGET: &str = env!("CARGO_CRATE_NAME");

#[derive(clap::Args, Debug, PartialEq)]
#[group(id = "logger")]
pub(crate) struct Logger {
    /// Increase log verbosity: -v for debug, -vv for trace
    #[arg(
        short,
        long,
        global = true,
        action = clap::ArgAction::Count,
        conflicts_with = "log_level",
    )]
    verbose: u8,

    /// Log level
    #[arg(long, global = true, value_name = "LEVEL", conflicts_with = "verbose")]
    log_level: Option<LevelFilter>,

    /// When to colorize log output
    #[arg(long, global = true, value_name = "WHEN", default_value_t = Color::Auto)]
    color: Color,
}

impl Logger {
    fn level(&self) -> LevelFilter {
        self.log_level.unwrap_or(match self.verbose {
            0 => LevelFilter::INFO,
            1 => LevelFilter::DEBUG,
            _ => LevelFilter::TRACE,
        })
    }

    fn default_targets(&self) -> Targets {
        Targets::new()
            .with_target(SELF_TARGET, self.level())
            // dependency logs are pinned to WARN
            .with_default(LevelFilter::WARN)
    }

    fn targets(&self) -> Targets {
        let rust_log =
            std::env::var("RUST_LOG")
                .ok()
                .and_then(|value| match value.parse::<Targets>() {
                    Ok(targets) => Some(targets),
                    Err(e) => {
                        eprintln!("ignoring RUST_LOG={value:?}: {e}");
                        None
                    }
                });

        rust_log.unwrap_or_else(|| self.default_targets())
    }

    pub(crate) fn init(&self) {
        let level = self.level();
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stdout)
            .with_ansi(self.color.enabled())
            .with_target(level >= LevelFilter::DEBUG);

        if level >= LevelFilter::DEBUG {
            tracing_subscriber::registry()
                .with(layer.with_filter(self.targets()))
                .init();
        } else {
            let layer = layer.map_event_format(|f| f.without_time());
            tracing_subscriber::registry()
                .with(layer.with_filter(self.targets()))
                .init();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub(crate) enum Color {
    Auto,
    Always,
    Never,
}

impl Color {
    fn enabled(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal(),
        }
    }
}
