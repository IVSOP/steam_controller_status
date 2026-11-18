use std::time::Duration;

use clap::{Args, Parser};

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Config {
    /// Show two simulated controllers without accessing hardware.
    #[arg(long, env, default_value_t = false)]
    pub demo: bool,

    /// Battery percentage at or below which the tray icon turns red.
    #[arg(long, env, default_value_t = 20,
        value_parser = clap::value_parser!(u8).range(0..=100))]
    pub low_battery_threshold: u8,

    #[command(flatten)]
    pub monitor: MonitorOptions,
}

#[derive(Debug, Args)]
pub struct MonitorOptions {
    /// How often to discover connected HID devices.
    #[arg(long, env, default_value = "2s",
        value_parser = positive_duration)]
    pub scan_interval: Duration,

    /// Remove a controller after this long without activity.
    #[arg(long, env, default_value = "10s",
        value_parser = positive_duration)]
    pub activity_timeout: Duration,

    /// Stop displaying a battery reading after this long without a battery report.
    #[arg(long, env, default_value = "12s",
        value_parser = positive_duration)]
    pub battery_timeout: Duration,
}

fn positive_duration(value: &str) -> Result<Duration, String> {
    let duration = humantime::parse_duration(value).map_err(|error| error.to_string())?;
    if duration.is_zero() {
        return Err("duration must be greater than zero".into());
    }
    Ok(duration)
}
