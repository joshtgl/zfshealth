{
  description = "zfshealth flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = lib.genAttrs systems;
    in
    {
      nixosModules.default = import ./nix/module.nix;

      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        rec {
          zfshealth = pkgs.callPackage ./nix/package.nix { };
          default = zfshealth;
        });

      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clippy
              nixfmt
              rust-analyzer
              rustc
              rustfmt
            ];
          };
        });

      checks = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          zfshealth = pkgs.callPackage ./nix/package.nix { };
          eval = lib.nixosSystem {
            inherit system;
            modules = [
              self.nixosModules.default
              ({ config, ... }: {
                system.stateVersion = "26.11";
                services.zfshealth = {
                  enable = true;
                  package = zfshealth;
                  emailPasswordFile = "/run/secrets/zfshealth-smtp-password";
                  environment = {
                    ZFSHEALTH_STATUS__SCHEDULE__CRON = "*/30 * * * *";
                  };
                  settings = {
                    scrub.schedule.cron = "15 3 * * 3";
                    status.schedule = {
                      cron = "*/15 * * * *";
                      repeat_after = "24h";
                    };
                    email = {
                      from = "zfshealth@example.com";
                      to = "admin@example.com";
                      host = "smtp.example.com";
                      port = 587;
                      username = "smtp-user";
                    };
                  };
                };
              })
            ];
          };
          renderedConfig = builtins.readFile eval.config.environment.etc."zfshealth/config.toml".source;
          execStart = eval.config.systemd.services.zfshealth.serviceConfig.ExecStart;
          hasZfsPath =
            builtins.any
              (pkg: lib.hasInfix "/zfs" (toString pkg) || lib.hasInfix "-zfs" (toString pkg))
              eval.config.systemd.services.zfshealth.path;
          envValue = eval.config.systemd.services.zfshealth.environment.ZFSHEALTH_STATUS__SCHEDULE__CRON;
          envFile = eval.config.systemd.services.zfshealth.serviceConfig.EnvironmentFile or "";
        in
        {
          zfshealth = zfshealth;
          nixos-module = pkgs.runCommand "zfshealth-module-check"
            {
              inherit renderedConfig execStart envValue envFile;
              hasZfsPath = if hasZfsPath then "1" else "0";
            }
            ''
              case "$renderedConfig" in
                *'password_file = "/run/secrets/zfshealth-smtp-password"'*) ;;
                *)
                  echo "generated config is missing password_file"
                  exit 1
                  ;;
              esac

              case "$renderedConfig" in
                *'password ='*)
                  echo "generated config contains inline password"
                  exit 1
                  ;;
              esac

              case "$execStart" in
                *'/bin/zfshealth daemon --config /etc/zfshealth/config.toml'*) ;;
                *)
                  echo "unexpected ExecStart: $execStart"
                  exit 1
                  ;;
              esac

              [ "$hasZfsPath" = "1" ]

              [ "$envValue" = "*/30 * * * *" ]
              [ "$envFile" = "" ]

              touch "$out"
            '';
        });
    };
}
