use std::io::IsTerminal;
use tracing_subscriber::{
    Layer,
    filter::{LevelFilter, Targets},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

const SELF_TARGET: &str = env!("CARGO_CRATE_NAME");

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

#[derive(clap::Args, Debug, PartialEq)]
#[group(id = "logging")]
pub(crate) struct Logging {
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

impl Logging {
    pub(crate) fn init(&self) {
        let level = self.log_level.unwrap_or(match self.verbose {
            0 => LevelFilter::INFO,
            1 => LevelFilter::DEBUG,
            _ => LevelFilter::TRACE,
        });

        let targets = {
            let from_env =
                std::env::var("RUST_LOG")
                    .ok()
                    .and_then(|value| match value.parse::<Targets>() {
                        Ok(targets) => Some(targets),
                        Err(e) => {
                            eprintln!("ignoring RUST_LOG={value:?}: {e}");
                            None
                        }
                    });

            from_env.unwrap_or_else(|| {
                Targets::new()
                    .with_target(SELF_TARGET, level)
                    // dependency logs are pinned to WARN
                    .with_default(LevelFilter::WARN)
            })
        };

        let layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stdout)
            .with_ansi(self.color.enabled())
            .with_target(level >= LevelFilter::DEBUG);

        let reg = tracing_subscriber::registry();

        if level >= LevelFilter::DEBUG {
            reg.with(layer.with_filter(targets)).init();
        } else {
            let layer = layer.map_event_format(|f| f.without_time());
            reg.with(layer.with_filter(targets)).init();
        }
    }
}
