self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.steam-controller-status;
in
{
  options.programs.steam-controller-status = {
    enable = lib.mkEnableOption "the Steam Controller battery tray applet and HID permissions";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      description = "The Steam Controller Status package to use.";
    };
    autostart = lib.mkEnableOption "starting the applet with the graphical user session";
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    services.udev.packages = [ cfg.package ];

    systemd.user.services.steam-controller-status = lib.mkIf cfg.autostart {
      description = "Steam Controller battery tray applet";
      wantedBy = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      serviceConfig = {
        ExecStart = lib.getExe cfg.package;
        Restart = "on-failure";
        RestartSec = 5;
      };
    };
  };
}
