{
  lib,
  rustPlatform,
  dbus,
  python3,
  systemd,
}:

rustPlatform.buildRustPackage {
  pname = "steam-controller-status";
  version = "0.1.0";

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../assets
      ../tests
      ../LICENSE
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;
  strictDeps = true;
  nativeInstallCheckInputs = [
    dbus
    python3
    systemd
  ];
  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    dbus-run-session --config-file=${dbus}/share/dbus-1/session.conf \
      -- python3 tests/tray_smoke.py "$out/bin/steam-controller-status"
    runHook postInstallCheck
  '';

  # async-hid's Tokio backend and ksni need no native GUI/HID libraries.
  postInstall = ''
    install -Dm644 assets/steam-controller-status.desktop \
      $out/share/applications/steam-controller-status.desktop
    install -Dm644 assets/steam-controller-status.svg \
      $out/share/icons/hicolor/scalable/apps/steam-controller-status.svg
    install -Dm644 assets/70-steam-controller-status.rules \
      $out/lib/udev/rules.d/70-steam-controller-status.rules
  '';

  meta = {
    description = "System tray battery and charge status for Steam Controller (2026)";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "steam-controller-status";
  };
}
