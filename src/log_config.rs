use std::io::Write;

use env_logger::{Builder, Env, DEFAULT_FILTER_ENV};
use log::LevelFilter;

/// Initialize logger in the systemd format, default to WARN log level if not specified
pub fn init() {
    let env = Env::default().filter_or(DEFAULT_FILTER_ENV, LevelFilter::Warn.as_str());
    Builder::from_env(env)
        .format(|buf, record| {
            writeln!(
                buf,
                "<{}>{}: {}",
                match record.level() {
                    log::Level::Error => 3,
                    log::Level::Warn => 4,
                    log::Level::Info => 6,
                    log::Level::Debug => 7,
                    log::Level::Trace => 7,
                },
                record.target(),
                record.args()
            )
        })
        .init()
}
