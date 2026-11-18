use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
};

use anyhow::Result;
use async_hid::{AsyncHidRead, DeviceId, DeviceInfo, HidBackend};
use better_tokio_select::tokio_select;
use futures_util::{Stream, StreamExt, stream};
use tokio::{
    sync::watch,
    time::{self, Instant, MissedTickBehavior},
};
use tokio_stream::StreamMap;

use crate::{
    config::MonitorOptions,
    protocol::{self, Battery, ChargeState, Report},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Controller {
    pub number: usize,
    pub battery: Option<Battery>,
}

impl Controller {
    pub fn label(&self) -> String {
        match self.battery {
            Some(battery) => format!(
                "Steam Controller {}: {}% · {}",
                self.number, battery.percent, battery.charge
            ),
            None => format!(
                "Steam Controller {}: battery status unavailable",
                self.number
            ),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub controllers: Vec<Controller>,
    pub problem: Option<String>,
}

impl Snapshot {
    pub fn demo() -> Self {
        Self {
            controllers: vec![
                Controller {
                    number: 1,
                    battery: Some(Battery {
                        percent: 76,
                        charge: ChargeState::Discharging,
                    }),
                },
                Controller {
                    number: 2,
                    battery: Some(Battery {
                        percent: 42,
                        charge: ChargeState::Charging,
                    }),
                },
            ],
            problem: None,
        }
    }
}

#[derive(Default)]
struct Reading {
    last_activity: Option<Instant>,
    battery: Option<(Battery, Instant)>,
}

impl Reading {
    fn update(&mut self, report: Report, now: Instant, options: &MonitorOptions) {
        match report {
            Report::Battery(battery) => {
                self.last_activity = Some(now);
                self.battery = Some((battery, now));
            }
            Report::Activity => {
                // A silent slot may now contain a different controller.
                if !self.is_active(now, options) {
                    self.battery = None;
                }
                self.last_activity = Some(now);
            }
            Report::Disconnected => *self = Self::default(),
        }
    }

    fn is_active(&self, now: Instant, options: &MonitorOptions) -> bool {
        self.last_activity
            .is_some_and(|last| now.duration_since(last) < options.activity_timeout)
    }

    fn battery(&self, now: Instant, options: &MonitorOptions) -> Option<Battery> {
        self.battery
            .filter(|(_, last)| now.duration_since(*last) < options.battery_timeout)
            .map(|(battery, _)| battery)
    }
}

// unfold retains each pending read when tokio_select! takes a timer branch.
type ReportStream = Pin<Box<dyn Stream<Item = Result<Option<Report>>> + Send>>;

fn reports(reader: impl AsyncHidRead + Send + 'static) -> ReportStream {
    stream::unfold(reader, |mut reader| async move {
        let mut packet = [0; 128];
        let result = match reader.read_input_report(&mut packet).await {
            Ok(0) => Err(anyhow::anyhow!("Controller disconnected")),
            Ok(length) => Ok(protocol::parse(&packet[..length])),
            Err(error) => Err(error.into()),
        };
        Some((result, reader))
    })
    .boxed()
}

fn supported(info: &DeviceInfo) -> bool {
    info.vendor_id == 0x28de
        && match info.product_id {
            // Bluetooth exposes its collections on one HID node.
            0x1303 => true,
            // Controller/receiver slots use usage 1; receiver control uses 2.
            0x1302 | 0x1304 | 0x1305 => info.usage_page == 0xff00 && info.usage_id == 1,
            _ => false,
        }
}

#[derive(Default)]
struct Monitor {
    readings: HashMap<DeviceId, Reading>,
    reports: StreamMap<DeviceId, ReportStream>,
    numbers: HashMap<DeviceId, usize>,
    next_number: usize,
    problem: Option<String>,
}

impl Monitor {
    async fn discover(&mut self, backend: &HidBackend) {
        let mut devices = match backend.enumerate().await {
            Ok(devices) => devices,
            Err(error) => {
                self.problem = Some(format!("Cannot discover controllers: {error}"));
                return;
            }
        };
        self.problem = None;
        let mut present = HashSet::new();
        let mut inaccessible = 0;
        while let Some(device) = devices.next().await {
            if !supported(&device) {
                continue;
            }
            let id = device.id.clone();
            present.insert(id.clone());
            if self.readings.contains_key(&id) {
                continue;
            }
            match device.open_readable().await {
                Ok(reader) => {
                    self.reports.insert(id.clone(), reports(reader));
                    self.readings.insert(id, Reading::default());
                }
                Err(error) => {
                    inaccessible += 1;
                    self.problem = Some(format!(
                        "Cannot read {inaccessible} controller interface(s). Check HID permissions. {error}"
                    ));
                }
            }
        }
        let removed: Vec<_> = self
            .readings
            .keys()
            .filter(|id| !present.contains(*id))
            .cloned()
            .collect();
        for id in removed {
            self.remove(&id);
        }
    }

    fn remove(&mut self, id: &DeviceId) {
        self.readings.remove(id);
        // Dropping the stream cancels the pending read and closes its handle.
        self.reports.remove(id);
    }

    fn snapshot(&mut self, now: Instant, options: &MonitorOptions) -> Snapshot {
        let mut controllers = Vec::new();
        for (id, reading) in &self.readings {
            if !reading.is_active(now, options) {
                continue;
            }
            let number = *self.numbers.entry(id.clone()).or_insert_with(|| {
                self.next_number += 1;
                self.next_number
            });
            controllers.push(Controller {
                number,
                battery: reading.battery(now, options),
            });
        }
        controllers.sort_by_key(|controller| controller.number);
        Snapshot {
            controllers,
            problem: self.problem.clone(),
        }
    }

    fn next_deadline(&self, now: Instant, options: &MonitorOptions) -> Option<Instant> {
        self.readings
            .values()
            .filter_map(|reading| reading.next_deadline(now, options))
            .min()
    }
}

impl Reading {
    fn next_deadline(&self, now: Instant, options: &MonitorOptions) -> Option<Instant> {
        if !self.is_active(now, options) {
            return None;
        }
        let activity = self
            .last_activity
            .map(|last| last + options.activity_timeout);
        let battery = self.battery.map(|(_, last)| last + options.battery_timeout);
        [activity, battery]
            .into_iter()
            .flatten()
            .filter(|deadline| *deadline > now)
            .min()
    }
}

async fn expire_at(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

pub async fn run(sender: watch::Sender<Snapshot>, options: &MonitorOptions) -> Result<()> {
    let backend = HidBackend::default();
    let mut monitor = Monitor::default();
    let mut scan = time::interval(options.scan_interval);
    scan.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        let deadline = monitor.next_deadline(Instant::now(), options);
        tokio_select!(match .. {
            .. if let _ = sender.closed() => return Ok(()),
            .. if let _ = scan.tick() => monitor.discover(&backend).await,
            .. if let Some((id, result)) = monitor.reports.next()
                && !monitor.reports.is_empty() =>
            {
                match result {
                    Ok(Some(report)) => {
                        if let Some(reading) = monitor.readings.get_mut(&id) {
                            reading.update(report, Instant::now(), options);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("Controller {id:?}: {error}");
                        monitor.remove(&id);
                    }
                }
            }
            .. if let _ = expire_at(deadline) => {}
        });
        let snapshot = monitor.snapshot(Instant::now(), options);
        sender.send_if_modified(|previous| {
            if *previous == snapshot {
                return false;
            }
            if previous.problem != snapshot.problem
                && let Some(problem) = &snapshot.problem
            {
                eprintln!("{problem}");
            }
            *previous = snapshot;
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn options() -> MonitorOptions {
        MonitorOptions {
            scan_interval: Duration::from_millis(100),
            activity_timeout: Duration::from_secs(4),
            battery_timeout: Duration::from_secs(6),
        }
    }

    struct TestReader {
        packets: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        attempts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl AsyncHidRead for TestReader {
        async fn read_input_report(&mut self, buffer: &mut [u8]) -> async_hid::HidResult<usize> {
            self.attempts
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let packet = self
                .packets
                .recv()
                .await
                .ok_or(async_hid::HidError::Disconnected)?;
            buffer[..packet.len()].copy_from_slice(&packet);
            Ok(packet.len())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn awaited_reads_survive_timeouts_and_do_not_block_other_controllers() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let mut readers = StreamMap::new();
        let (first_tx, first_rx) = tokio::sync::mpsc::unbounded_channel();
        let (second_tx, second_rx) = tokio::sync::mpsc::unbounded_channel();
        let first_attempts = Arc::new(AtomicUsize::new(0));
        let second_attempts = Arc::new(AtomicUsize::new(0));
        readers.insert(
            1,
            reports(TestReader {
                packets: first_rx,
                attempts: first_attempts.clone(),
            }),
        );
        readers.insert(
            2,
            reports(TestReader {
                packets: second_rx,
                attempts: second_attempts.clone(),
            }),
        );

        // No packets: one pending read per controller, with no timer polling.
        assert!(
            time::timeout(Duration::from_secs(30), readers.next())
                .await
                .is_err()
        );
        assert_eq!(first_attempts.load(Ordering::Relaxed), 1);
        assert_eq!(second_attempts.load(Ordering::Relaxed), 1);

        let mut packet = vec![0; 15];
        packet[..3].copy_from_slice(&[0x43, 2, 94]);
        second_tx.send(packet).unwrap();
        let sent_at = Instant::now();
        let (id, report) = time::timeout(Duration::from_secs(30), readers.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id, 2);
        assert_eq!(
            report.unwrap(),
            Some(Report::Battery(Battery {
                percent: 94,
                charge: ChargeState::Charging
            }))
        );
        assert_eq!(
            Instant::now(),
            sent_at,
            "Data must wake the reader without a timer tick"
        );
        assert_eq!(
            first_attempts.load(Ordering::Relaxed),
            1,
            "Timeout must retain the pending read"
        );

        // Removing a controller and cancelling the monitor release pending reads.
        readers.remove(&1);
        assert!(first_tx.is_closed());
        drop(readers);
        assert!(second_tx.is_closed());
    }

    #[test]
    fn expiry_deadlines_advance_without_rearming_expired_timers() {
        let mut options = options();
        options.battery_timeout = Duration::from_secs(2);
        let now = Instant::now();
        let mut reading = Reading::default();
        reading.update(
            Report::Battery(Battery {
                percent: 80,
                charge: ChargeState::Charging,
            }),
            now,
            &options,
        );
        assert_eq!(
            reading.next_deadline(now, &options),
            Some(now + options.battery_timeout)
        );
        let expired = now + options.battery_timeout;
        assert_eq!(reading.battery(expired, &options), None);
        assert_eq!(
            reading.next_deadline(expired, &options),
            Some(now + options.activity_timeout)
        );
        assert_eq!(
            reading.next_deadline(now + options.activity_timeout, &options),
            None
        );
    }

    #[test]
    fn idle_receiver_slots_are_not_connected_controllers() {
        let mut reading = Reading::default();
        let now = Instant::now();
        let options = options();
        assert!(!reading.is_active(now, &options));
        reading.update(Report::Activity, now, &options);
        assert!(reading.is_active(now, &options));
        assert_eq!(reading.battery(now, &options), None);
        assert!(!reading.is_active(now + options.activity_timeout, &options));
    }

    #[test]
    fn stale_battery_expires_even_if_input_reports_continue() {
        let mut reading = Reading::default();
        let now = Instant::now();
        let options = options();
        let battery = Battery {
            percent: 80,
            charge: ChargeState::Charging,
        };
        reading.update(Report::Battery(battery), now, &options);
        reading.update(Report::Activity, now + Duration::from_secs(3), &options);
        assert_eq!(
            reading.battery(now + Duration::from_secs(5), &options),
            Some(battery)
        );
        assert!(reading.is_active(now + options.battery_timeout, &options));
        assert_eq!(
            reading.battery(now + options.battery_timeout, &options),
            None
        );
    }

    #[test]
    fn disconnect_and_reconnect_cannot_reuse_an_old_reading() {
        let mut reading = Reading::default();
        let now = Instant::now();
        let options = options();
        reading.update(
            Report::Battery(Battery {
                percent: 80,
                charge: ChargeState::Charging,
            }),
            now,
            &options,
        );
        reading.update(Report::Disconnected, now, &options);
        assert!(!reading.is_active(now, &options));
        reading.update(Report::Activity, now, &options);
        assert_eq!(reading.battery(now, &options), None);
        reading.update(
            Report::Battery(Battery {
                percent: 50,
                charge: ChargeState::Discharging,
            }),
            now,
            &options,
        );
        reading.update(Report::Activity, now + options.activity_timeout, &options);
        assert_eq!(
            reading.battery(now + options.activity_timeout, &options),
            None
        );
    }
}
