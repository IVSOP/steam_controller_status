use ksni::{Icon, MenuItem, ToolTip, menu::StandardItem};
use tokio::sync::mpsc;

use crate::monitor::Snapshot;

pub struct ControllerTray {
    pub snapshot: Snapshot,
    quit: mpsc::UnboundedSender<()>,
    demo: bool,
    low_battery_threshold: u8,
}

impl ControllerTray {
    pub fn new(
        snapshot: Snapshot,
        quit: mpsc::UnboundedSender<()>,
        demo: bool,
        low_battery_threshold: u8,
    ) -> Self {
        Self {
            snapshot,
            quit,
            demo,
            low_battery_threshold,
        }
    }

    fn lines(&self) -> Vec<String> {
        let mut lines: Vec<_> = self
            .snapshot
            .controllers
            .iter()
            .map(|controller| controller.label())
            .collect();
        if lines.is_empty() {
            lines.push("No Steam Controllers connected".into());
        }
        if self.snapshot.problem.is_some() {
            lines.push("Cannot read controller(s) — check HID permissions or logs".into());
        }
        lines
    }
}

impl ksni::Tray for ControllerTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "steam-controller-status".into()
    }

    fn title(&self) -> String {
        if self.demo {
            "Steam Controller Status [Demo]"
        } else {
            "Steam Controller Status"
        }
        .into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        // Embedded ARGB battery glyph: no icon theme required. Fill shows the
        // lowest reported battery; the menu always shows each controller.
        let battery = self
            .snapshot
            .controllers
            .iter()
            .filter_map(|controller| controller.battery)
            .min_by_key(|battery| battery.percent);
        let color = match battery {
            Some(battery) if battery.percent <= self.low_battery_threshold => [255, 239, 83, 80],
            Some(_) => [255, 102, 210, 150],
            None => [255, 170, 177, 190],
        };
        let filled = battery.map_or(0, |battery| {
            (20 * usize::from(battery.percent)).div_ceil(100)
        });
        let mut argb = vec![0; 32 * 32 * 4];
        for y in 8..24 {
            for x in 3..30 {
                let border = x < 27 && (!(5..25).contains(&x) || !(10..22).contains(&y));
                let terminal = x >= 27 && (12..20).contains(&y);
                let fill = (6..6 + filled).contains(&x) && (11..21).contains(&y);
                if border || terminal || fill {
                    argb[(y * 32 + x) * 4..(y * 32 + x + 1) * 4].copy_from_slice(&color);
                }
            }
        }
        vec![Icon {
            width: 32,
            height: 32,
            data: argb,
        }]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.title(),
            description: self.lines().join("\n"),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<_> = self
            .lines()
            .into_iter()
            .map(|label| {
                StandardItem {
                    label: if self.demo {
                        format!("[Demo] {label}")
                    } else {
                        label
                    },
                    enabled: false,
                    ..Default::default()
                }
                .into()
            })
            .collect();
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.quit.send(());
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        eprintln!(
            "System tray unavailable ({reason:?}); waiting for a tray host (e.g. Waybar, KDE, or GNOME AppIndicator)"
        );
        true // Survive panel restarts and starting before the panel.
    }
}
