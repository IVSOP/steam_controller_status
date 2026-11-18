#!/usr/bin/env python3
"""Exercise the real D-Bus menu and Quit action on a private session bus.

Run: dbus-run-session -- python3 tests/tray_smoke.py ./target/debug/steam-controller-status
No controller, compositor, or existing desktop tray is needed.
"""

import json
import os
import subprocess
import sys
import time


def bus_call(service, command, *args):
    return subprocess.run(
        ["busctl", "--user", "--json=short", command, service, *args],
        capture_output=True,
        text=True,
        timeout=3,
    )


def environment(overrides):
    settings = {
        "DEMO", "LOW_BATTERY_THRESHOLD", "SCAN_INTERVAL",
        "ACTIVITY_TIMEOUT", "BATTERY_TIMEOUT",
    }
    env = {key: value for key, value in os.environ.items() if key not in settings}
    env.update(overrides)
    return env


def run(binary, args, overrides, expected_color):
    process = subprocess.Popen([binary, *args], env=environment(overrides), stdout=subprocess.DEVNULL)
    service = f"org.kde.StatusNotifierItem-{process.pid}-1"
    try:
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            assert process.poll() is None, "Applet exited before exporting its menu"
            reply = bus_call(
                service, "call", "/MenuBar", "com.canonical.dbusmenu",
                "GetLayout", "iias", "--", "0", "-1", "0",
            )
            if reply.returncode == 0:
                break
            time.sleep(0.05)
        else:
            raise AssertionError(f"Applet did not export its menu: {reply.stderr}")

        children = json.loads(reply.stdout)["data"][1][2]
        rows = [child["data"] for child in children]
        labels = [props["label"]["data"] for _, props, _ in rows if "label" in props]
        if expected_color is not None:
            assert labels == [
                "[Demo] Steam Controller 1: 76% · Discharging",
                "[Demo] Steam Controller 2: 42% · Charging",
                "Quit",
            ], labels
            assert all(not rows[i][1]["enabled"]["data"] for i in (0, 1))
            assert rows[2][1]["type"]["data"] == "separator"

            tooltip = bus_call(
                service, "get-property", "/StatusNotifierItem",
                "org.kde.StatusNotifierItem", "ToolTip",
            )
            tooltip.check_returncode()
            tip_data = json.loads(tooltip.stdout)["data"]
            assert "[Demo]" in tip_data[2]
            assert "76%" in tip_data[3] and "42%" in tip_data[3]

            # Verify configuration changes the exported icon, not only the parser.
            icon = bus_call(
                service, "get-property", "/StatusNotifierItem",
                "org.kde.StatusNotifierItem", "IconPixmap",
            )
            icon.check_returncode()
            pixels = json.loads(icon.stdout)["data"][0][2]
            assert any(pixels[i:i + 4] == expected_color for i in range(0, len(pixels), 4))

        else:
            assert "Quit" in labels
            # Allow the monitor to start awaiting input before quitting.
            time.sleep(0.2)
            reply = bus_call(
                service, "call", "/MenuBar", "com.canonical.dbusmenu",
                "GetLayout", "iias", "--", "0", "-1", "0",
            )
            reply.check_returncode()
            rows = [child["data"] for child in json.loads(reply.stdout)["data"][1][2]]

        # Invoke the same dbusmenu Event that a tray host emits on a Quit click.
        quit_id = next(row_id for row_id, props, _ in rows if props.get("label", {}).get("data") == "Quit")
        quit_reply = bus_call(
            service, "call", "/MenuBar", "com.canonical.dbusmenu",
            "Event", "isvu", str(quit_id), "clicked", "i", "0", "0",
        )
        quit_reply.check_returncode()
        assert process.wait(timeout=5) == 0, "Quit did not exit successfully"
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    binary = sys.argv[1]
    green = [255, 102, 210, 150]
    red = [255, 239, 83, 80]
    run(binary, ["--demo"], {}, green)
    run(binary, [], {
        "DEMO": "true",
        "LOW_BATTERY_THRESHOLD": "90",
        "SCAN_INTERVAL": "5s",
        "ACTIVITY_TIMEOUT": "20s",
        "BATTERY_TIMEOUT": "30s",
    }, red)
    run(binary, ["--low-battery-threshold", "20", "--scan-interval", "100ms"], {
        "DEMO": "true",
        "LOW_BATTERY_THRESHOLD": "90",
        "SCAN_INTERVAL": "invalid",  # The valid CLI value must take precedence.
    }, green)

    run(binary, [], {"SCAN_INTERVAL": "1h"}, None)

    for key, value, flag in [
        ("SCAN_INTERVAL", "0s", "--scan-interval"),
        ("ACTIVITY_TIMEOUT", "invalid", "--activity-timeout"),
        ("BATTERY_TIMEOUT", "0s", "--battery-timeout"),
        ("LOW_BATTERY_THRESHOLD", "101", "--low-battery-threshold"),
    ]:
        rejected = subprocess.run([binary], env=environment({key: value}), capture_output=True, text=True, timeout=3)
        assert rejected.returncode == 2 and flag in rejected.stderr, rejected.stderr
    print("Tray smoke test passed: controller rows, tooltip, Quit, defaults, env, CLI precedence, validation, and prompt cancellation of the async monitor")
