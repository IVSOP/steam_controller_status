mod config;
mod monitor;
mod protocol;
mod tray;

use anyhow::{Context, Result, anyhow};
use better_tokio_select::tokio_select;
use clap::Parser;
use ksni::TrayMethods;
use tokio::sync::{mpsc, watch};

use config::Config;
use monitor::Snapshot;
use tray::ControllerTray;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Config::parse();
    let initial = if args.demo {
        Snapshot::demo()
    } else {
        Snapshot::default()
    };
    let (quit_tx, mut quit_rx) = mpsc::unbounded_channel();
    let handle = ControllerTray::new(
        initial.clone(),
        quit_tx,
        args.demo,
        args.low_battery_threshold,
    )
    .assume_sni_available(true)
    .spawn()
    .await
    .context("Cannot start tray; run in a desktop session with a session D-Bus")?;

    let (snapshot_tx, mut snapshot_rx) = watch::channel(initial);
    // An unpolled monitor also keeps the watch channel open in demo mode.
    let mut monitor = Box::pin(monitor::run(snapshot_tx, &args.monitor));
    let signal = shutdown_signal();
    tokio::pin!(signal);
    let result = loop {
        tokio_select!(
            biased,
            match .. {
                .. if let _ = quit_rx.recv() => break Ok(()),
                .. if let signal = &mut signal => break signal,
                .. if let result = &mut monitor
                    && !args.demo =>
                {
                    break match result {
                        Err(error) => Err(error),
                        Ok(()) => Err(anyhow!("Controller monitor stopped unexpectedly")),
                    };
                }
                .. if let changed = snapshot_rx.changed() => {
                    if changed.is_err() {
                        break Err(anyhow!("Controller monitor disconnected"));
                    }
                    let snapshot = snapshot_rx.borrow_and_update().clone();
                    if handle
                        .update(move |tray| tray.snapshot = snapshot)
                        .await
                        .is_none()
                    {
                        break Err(anyhow!("System tray service stopped"));
                    }
                }
            }
        );
    };
    // Cancel pending asynchronous reads and close their device handles on Quit.
    drop(monitor);
    handle.shutdown().await;
    result
}

async fn shutdown_signal() -> Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio_select!(match .. {
        .. if let result = tokio::signal::ctrl_c() => result.context("Cannot listen for Ctrl-C"),
        .. if let _ = terminate.recv() => Ok(()),
    })
}
