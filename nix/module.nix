{ config, lib, pkgs, ... }:

let
  cfg = config.services.zfshealth;
  tomlFormat = pkgs.formats.toml { };
  generatedSettings =
    cfg.settings
    // lib.optionalAttrs (cfg.emailPasswordFile != null) {
      email = (cfg.settings.email or { }) // {
        password_file = toString cfg.emailPasswordFile;
      };
    };
  configFile = tomlFormat.generate "zfshealth-config.toml" generatedSettings;
in
{
  options.services.zfshealth = {
    enable = lib.mkEnableOption "the zfshealth daemon";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ./package.nix { };
      description = "The zfshealth package to run.";
    };

    settings = lib.mkOption {
      type = tomlFormat.type;
      default = { };
      example = {
        scrub.schedule.cron = "15 3 * * 3";
        status.schedule = {
          cron = "*/15 * * * *";
          repeat_after = "24h";
        };
      };
      description = "Non-secret zfshealth configuration rendered to TOML.";
    };

    emailPasswordFile = lib.mkOption {
      type = lib.types.nullOr (lib.types.oneOf [ lib.types.path lib.types.str ]);
      default = null;
      example = "/run/secrets/zfshealth-smtp-password";
      description = "Runtime path to the SMTP password file.";
    };

    environment = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      example = {
        ZFSHEALTH_STATUS__SCHEDULE__CRON = "*/30 * * * *";
      };
      description = "Extra environment variables passed to the zfshealth service.";
    };

    environmentFile = lib.mkOption {
      type = lib.types.nullOr (lib.types.oneOf [ lib.types.path lib.types.str ]);
      default = null;
      example = "/run/secrets/zfshealth.env";
      description = "Optional environment file passed to the zfshealth service.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !(cfg.emailPasswordFile != null && lib.hasAttrByPath [ "email" "password" ] cfg.settings);
        message = "services.zfshealth.emailPasswordFile cannot be used together with services.zfshealth.settings.email.password.";
      }
      {
        assertion = !(cfg.emailPasswordFile != null && lib.hasAttrByPath [ "email" "password_file" ] cfg.settings);
        message = "services.zfshealth.emailPasswordFile replaces services.zfshealth.settings.email.password_file.";
      }
    ];

    environment.etc."zfshealth/config.toml".source = configFile;

    systemd.services.zfshealth = {
      description = "ZFS Health daemon";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      path = [
        config.boot.zfs.package
      ];
      environment = cfg.environment // {
        SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
      };
      serviceConfig =
        {
          Type = "simple";
          ExecStart = "${cfg.package}/bin/zfshealth daemon --config /etc/zfshealth/config.toml";
          ExecReload = "${pkgs.coreutils}/bin/kill -HUP $MAINPID";
          Restart = "on-failure";
        }
        // lib.optionalAttrs (cfg.environmentFile != null) {
          EnvironmentFile = toString cfg.environmentFile;
        };
    };
  };
}
