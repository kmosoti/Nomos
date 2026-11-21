{ config, lib, pkgs, ... }:

let
  cfg = config.services.nomos.wireguard;
  derivedMtu = cfg.outerPmtu - 80;
  peerSubmodule = lib.types.submodule {
    options = {
      publicKey = lib.mkOption {
        type = lib.types.str;
        description = "Base64 WireGuard public key for the remote peer.";
      };

      allowedIPs = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        example = [ "fd00:db8::2/128" ];
        description = "Pure-IPv6 CIDR routes allowed for this peer.";
      };

      endpoint = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "[2001:db8::2]:51820";
        description = "Optional pure-IPv6 UDP endpoint.";
      };

      persistentKeepalive = lib.mkOption {
        type = lib.types.ints.between 0 65535;
        default = 25;
        description = "WireGuard persistent keepalive interval in seconds.";
      };
    };
  };
in
{
  options.services.nomos.wireguard = {
    enable = lib.mkEnableOption "Nomos pure-IPv6 WireGuard wg0 control mesh";

    interfaceName = lib.mkOption {
      type = lib.types.str;
      default = "wg0";
      description = "WireGuard network interface name.";
    };

    outerPmtu = lib.mkOption {
      type = lib.types.ints.between 1280 9000;
      default = 1460;
      description = "Measured outer IPv6 path MTU; wg0 MTU is set to outerPmtu - 80.";
    };

    addresses = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "fd00:db8::1/64" ];
      description = "Local pure-IPv6 addresses assigned to the WireGuard interface.";
    };

    listenPort = lib.mkOption {
      type = lib.types.port;
      default = 51820;
      description = "UDP port opened in the firewall for WireGuard transport.";
    };

    privateKeyFile = lib.mkOption {
      type = lib.types.path;
      default = "/run/credentials/wireguard-wg0/private.key";
      description = "Path to root-readable WireGuard private key file provisioned via systemd credentials or secrets module.";
    };

    peers = lib.mkOption {
      type = lib.types.listOf peerSubmodule;
      default = [ ];
      description = "Caller-supplied pure-IPv6 WireGuard peers.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = derivedMtu >= 1200;
        message = "services.nomos.wireguard.outerPmtu must yield MTU >= 1200 (outerPmtu >= 1280).";
      }
      {
        assertion = lib.all (addr: lib.hasInfix ":" addr) cfg.addresses;
        message = "services.nomos.wireguard.addresses must contain IPv6 addresses only.";
      }
    ];

    networking.firewall.allowedUDPPorts = [ cfg.listenPort ];

    networking.wireguard.enable = true;
    networking.wireguard.interfaces.${cfg.interfaceName} = {
      ips = cfg.addresses;
      listenPort = cfg.listenPort;
      mtu = derivedMtu;
      privateKeyFile = toString cfg.privateKeyFile;
      peers = map (p: {
        inherit (p) publicKey allowedIPs persistentKeepalive;
      } // lib.optionalAttrs (p.endpoint != null) {
        inherit (p) endpoint;
      }) cfg.peers;
    };

    environment.systemPackages = [ pkgs.wireguard-tools ];
  };
}
