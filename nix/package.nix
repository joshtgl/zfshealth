{
  lib,
  rustPlatform,
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile ../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = cargoToml.package.name;
  version = cargoToml.package.version;

  src = lib.cleanSource ../.;

  cargoLock = {
    lockFile = ../Cargo.lock;
  };

  postInstall = ''
    install -Dm644 ${../packaging/config.toml} $out/share/doc/zfshealth/examples/config.toml
  '';

  meta = with lib; {
    description = "ZFS scrub scheduler and health notifier";
    homepage = "https://github.com/joshtgl/zfshealth";
    license = with licenses; [ mit asl20 ];
    mainProgram = "zfshealth";
    platforms = platforms.linux;
  };
}
