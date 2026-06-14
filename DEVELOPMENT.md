# Development

This document covers running, building, and testing `zfshealth` from source.

## Requirements

- Rust toolchain
- `cargo`
- ZFS userspace tools available on the host if you want to exercise real scrub commands

## Run

Run a one-shot scrub using the example config:

```bash
cargo run -- run-once --config examples/config.toml
```

Run the daemon in the foreground:

```bash
cargo run -- daemon --config examples/config.toml
```

If you want to test config discovery instead of passing `--config`, place a config at:

`~/.config/zfshealth/config.toml`

## Build

Build a debug binary:

```bash
cargo build
```

Build a release binary:

```bash
cargo build --release
```

Build the Debian package:

```bash
cargo install cargo-deb --locked
cargo deb
```

## Test

Run the test suite:

```bash
cargo test
```

Check formatting:

```bash
cargo fmt --check
```

The current tests cover config parsing and scheduler validation. End-to-end scrub behavior depends on local ZFS and SMTP environment availability.
