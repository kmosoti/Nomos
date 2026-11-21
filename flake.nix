{
  description = "Nomos reusable pure-IPv6 NixOS modules for WireGuard mesh, vTPM secrets, and deadman rollback";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }: {
    nixosModules = rec {
      base-ipv6 = import ./nix/modules/base-ipv6.nix;
      wireguard-wg0 = import ./nix/modules/wireguard-wg0.nix;
      secrets = import ./nix/modules/secrets.nix;
      deadman-rollback = import ./nix/modules/deadman-rollback.nix;

      default = {
        imports = [
          base-ipv6
          wireguard-wg0
          secrets
          deadman-rollback
        ];
      };
    };

    devShells = nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-linux" ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        default = pkgs.mkShell {
          packages = with pkgs; [
            pkg-config
            sqlite
            systemd
            wireguard-tools
          ];
        };
      });
  };
}
