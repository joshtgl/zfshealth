# Changelog

## [0.1.4](https://github.com/joshtgl/zfshealth/compare/v0.1.3...v0.1.4) - 2026-09-04

### Added

- support 5 entry cron

### Fixed

- *(deps)* update rust crate uuid to 1.26.0 ([#23](https://github.com/joshtgl/zfshealth/pull/23))
- *(deps)* update rust dependencies ([#21](https://github.com/joshtgl/zfshealth/pull/21))
- *(deps)* update rust dependencies ([#20](https://github.com/joshtgl/zfshealth/pull/20))
- *(deps)* update rust dependencies ([#19](https://github.com/joshtgl/zfshealth/pull/19))
- use log printing
- verbose config error logs
- configurable ca certificate path for nix module
- *(deps)* update rust crate jiff to 0.2.31 ([#17](https://github.com/joshtgl/zfshealth/pull/17))

### Other

- *(deps)* update github actions ([#22](https://github.com/joshtgl/zfshealth/pull/22))

## [0.1.3](https://github.com/joshtgl/zfshealth/compare/v0.1.2...v0.1.3) - 2026-06-27

### Other

- *(ci)* Use staging job step

## [0.1.2](https://github.com/joshtgl/zfshealth/compare/v0.1.1...v0.1.2) - 2026-06-27

### Fixed

- sbom filename handling

### Other

- add dependency automerge and minimum age
- add PR dry run support
- split steps, add dry run ability for release builds

## [0.1.1](https://github.com/joshtgl/zfshealth/compare/v0.1.0...v0.1.1) - 2026-06-25

### Added

- nix definitions, rename systemd to just zfshealth
- support configuration via env vars, password file
- Add zfs status daemon checks
- static binary, sbom, attest

### Other

- Update rust dependencies
- Merge pull request #10 from joshtgl/renovate/cargo-zigbuild-0.x
- Merge pull request #2 from joshtgl/renovate/github-actions
- Update rust dependencies
- Update github actions to v7

## [0.1.0] - 2026-06-14

- Initial release.
