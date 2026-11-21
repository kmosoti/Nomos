{ config, lib, pkgs, ... }:

let
  cfg = config.services.nomos.deadmanRollback;
in
{
  options.services.nomos.deadmanRollback = {
    enable = lib.mkEnableOption "Nomos automatic deadman health-check rollback guard";

    healthUrl = lib.mkOption {
      type = lib.types.str;
      default = "http://[::1]:7440/health";
      description = "Local pure-IPv6 control health endpoint verified after activation.";
    };

    graceSeconds = lib.mkOption {
      type = lib.types.ints.between 30 3600;
      default = 180;
      description = "Delay after boot/switch before the deadman probe evaluates system health.";
    };

    probePeerIpv6 = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "fd00:db8::1";
      description = "Optional peer IPv6 address verified via ICMPv6 ping alongside healthUrl.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.nomos-deadman-rollback = {
      description = "Nomos Deadman Connectivity Guard";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      serviceConfig = {
        Type = "oneshot";
        ExecStart = pkgs.writeShellScript "nomos-deadman-check" ''
          set -euo pipefail
          if ! ${pkgs.curl}/bin/curl -fsS --max-time 5 ${lib.escapeShellArg cfg.healthUrl} >/dev/null; then
            echo "nomos deadman health probe failed for ${cfg.healthUrl}; rolling back generation" >&2
            exec /run/current-system/sw/bin/nixos-rebuild switch --rollback
          fi
          ${lib.optionalString (cfg.probePeerIpv6 != null) ''
            if ! ${pkgs.iputils}/bin/ping -6 -c 1 -W 3 ${lib.escapeShellArg cfg.probePeerIpv6} >/dev/null; then
              echo "nomos deadman peer ping failed for ${cfg.probePeerIpv6}; rolling back generation" >&2
              exec /run/current-system/sw/bin/nixos-rebuild switch --rollback
            fi
          ''}
        '';
      };
    };

    systemd.timers.nomos-deadman-rollback = {
      description = "Scheduled Nomos Deadman Connectivity Check";
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnBootSec = "${toString cfg.graceSeconds}s";
        OnUnitActiveSec = "${toString cfg.graceSeconds}s";
        Unit = "nomos-deadman-rollback.service";
      };
    };
  };
}
