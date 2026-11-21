{ config, lib, pkgs, ... }:

let
  cfg = config.services.nomos.secrets;
in
{
  options.services.nomos.secrets = {
    enable = lib.mkEnableOption "Nomos credential directory and vTPM 2.0 runtime prerequisites";

    credentialDir = lib.mkOption {
      type = lib.types.path;
      default = "/etc/nomos/secrets";
      description = "Root-owned directory holding staged credentials for Nomos services.";
    };

    vaultDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/nomos/vault";
      description = "Encrypted age vault storage path.";
    };

    enableTpm2 = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Enable kernel TPM 2.0 character device and userspace tpm2-tss libraries for systemd-creds.";
    };
  };

  config = lib.mkIf cfg.enable {
    security.tpm2 = lib.mkIf cfg.enableTpm2 {
      enable = true;
      pkcs11.enable = false;
      tctiEnvironment.enable = true;
    };

    systemd.tmpfiles.rules = [
      "d ${toString cfg.credentialDir} 0750 root root - -"
      "d ${toString cfg.vaultDir} 0700 root root - -"
    ];

    environment.systemPackages = with pkgs; [
      age
    ] ++ lib.optionals cfg.enableTpm2 [
      tpm2-tools
      tpm2-tss
    ];
  };
}
