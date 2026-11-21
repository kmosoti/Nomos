{ config, lib, pkgs, ... }:

let
  cfg = config.services.nomos.ipv6;
in
{
  options.services.nomos.ipv6 = {
    enable = lib.mkEnableOption "Nomos pure-IPv6 host networking and RA forwarding sysctls";

    primaryInterface = lib.mkOption {
      type = lib.types.str;
      default = "eth0";
      description = "Primary physical or virtual uplink interface receiving router advertisements.";
    };

    acceptRaMode = lib.mkOption {
      type = lib.types.ints.between 0 2;
      default = 2;
      description = "IPv6 accept_ra sysctl value (2 allows router advertisements while forwarding is enabled).";
    };

    enableForwarding = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Enable kernel IPv6 packet forwarding.";
    };

    nameservers = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [
        "2001:4860:4860::8888"
        "2606:4700:4700::1111"
      ];
      description = "Pure-IPv6 recursive DNS resolvers.";
    };

    allowedTcpPorts = lib.mkOption {
      type = lib.types.listOf lib.types.port;
      default = [ ];
      description = "Optional IPv6 TCP ports allowed in addition to SSH/control listeners.";
    };
  };

  config = lib.mkIf cfg.enable {
    networking.enableIPv6 = true;
    networking.nameservers = cfg.nameservers;
    networking.firewall.enable = true;
    networking.firewall.allowPing = true;
    networking.firewall.allowedTCPPorts = cfg.allowedTcpPorts;

    boot.kernel.sysctl = {
      "net.ipv6.conf.all.forwarding" = lib.mkDefault (if cfg.enableForwarding then 1 else 0);
      "net.ipv6.conf.default.accept_ra" = lib.mkDefault cfg.acceptRaMode;
      "net.ipv6.conf.${cfg.primaryInterface}.accept_ra" = lib.mkForce cfg.acceptRaMode;
    };

    environment.systemPackages = with pkgs; [
      iproute2
      ndisc6
    ];
  };
}
