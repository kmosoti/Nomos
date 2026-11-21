{ config, lib, pkgs, ... }:

let
  cfg = config.services.nomos.deadmanRollback;
  sentinelPath = "/run/nomos-switch-pending";
  attemptGuardPath = "/run/nomos-deadman-attempted";
in
{
  options.services.nomos.deadmanRollback = {
    enable = lib.mkEnableOption "Nomos one-shot post-switch deadman health-check rollback guard";

    healthUrl = lib.mkOption {
      type = lib.types.str;
      default = "http://[::1]:7440/health";
      description = "Local pure-IPv6 control health endpoint verified once after activation.";
    };

    graceSeconds = lib.mkOption {
      type = lib.types.ints.between 30 3600;
      default = 180;
      description = "One-shot delay after boot/switch before the deadman probe evaluates system health.";
    };

    probePeerIpv6 = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "fd00:db8::1";
      description = "Optional peer IPv6 address verified via ICMPv6 ping alongside healthUrl.";
    };
  };

  config = lib.mkIf cfg.enable {
    # Arm the one-shot post-switch sentinel only during system activation,
    # unless a rollback attempt already ran for this boot/switch cycle.
    system.activationScripts.nomosDeadmanArm = lib.stringAfter [ "etc" ] ''
      if [ ! -e ${attemptGuardPath} ]; then
        umask 077
        : > ${sentinelPath}
      fi
    '';

    systemd.services.nomos-deadman-rollback = {
      description = "Nomos One-Shot Deadman Connectivity Guard";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      unitConfig = {
        ConditionPathExists = sentinelPath;
        ConditionPathExistsGlob = "!${attemptGuardPath}";
      };
      serviceConfig = {
        Type = "oneshot";
        ExecStart = pkgs.writeShellScript "nomos-deadman-check" ''
          set -euo pipefail
          if [ ! -e ${sentinelPath} ] || [ -e ${attemptGuardPath} ]; then
            exit 0
          fi

          rollback_once() {
            local reason="$1"
            echo "nomos deadman probe failed ($reason); attempting single rollback" >&2
            umask 077
            : > ${attemptGuardPath}
            rm -f ${sentinelPath}
            exec /run/current-system/sw/bin/nixos-rebuild switch --rollback
          }

          if ! ${pkgs.curl}/bin/curl -fsS --max-time 5 ${lib.escapeShellArg cfg.healthUrl} >/dev/null; then
            rollback_once "healthUrl ${cfg.healthUrl}"
          fi
          ${lib.optionalString (cfg.probePeerIpv6 != null) ''
            if ! ${pkgs.iputils}/bin/ping -6 -c 1 -W 3 ${lib.escapeShellArg cfg.probePeerIpv6} >/dev/null; then
              rollback_once "probePeerIpv6 ${cfg.probePeerIpv6}"
            fi
          ''}

          # Health checks passed: disarm sentinel so no further checks or rollbacks run.
          rm -f ${sentinelPath}
        '';
      };
    };

    # Strictly one-shot timer (no OnUnitActiveSec) so transient blips never loop rollbacks.
    systemd.timers.nomos-deadman-rollback = {
      description = "One-Shot Post-Switch Nomos Deadman Connectivity Check";
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnBootSec = "${toString cfg.graceSeconds}s";
        Unit = "nomos-deadman-rollback.service";
      };
    };
  };
}
