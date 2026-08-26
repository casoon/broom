# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - 2026-08-26

Initial release.

### Added

- Recursive project discovery across a directory tree, with target size, last
  modified time, and shared/override detection.
- Level A cleanup: removes complete `target/` directories selected by
  `--keep-days` and, optionally, `--keep-size`.
- Level B cleanup (`--experimental-fine`, opt-in): fingerprint- and duplicate-based
  pruning within an active `target/`, plus `--clean-incremental`, `--clean-doc`,
  and `--tests-only`.
- `--trash` moves Level A targets to the OS trash/recycle bin instead of deleting
  them permanently.
- Target directories locked by a running `cargo` process are skipped instead of
  racing the build.
- `inspect`, `doctor`, and `toolchains` subcommands for read-only diagnostics.
- `project` subcommand to clean a single project with the same policy and safety
  gates as a recursive run.
- `registry` subcommand to remove cached `.crate` archives not referenced by any
  `Cargo.lock` below the scanned root.
- `--interactive` selection before deletion, and `--dry-run` for a no-op preview.
- `--format json` machine-readable output for scripting and scheduled dry runs.
- `--history` opt-in trend tracking: rolling 14-day JSON Lines log of target sizes,
  surfaced as a size-over-time comparison in reports.
- `broom.toml` / `~/.config/cargo-broom/config.toml` configuration file support.

[0.1.0]: https://github.com/casoon/broom/releases/tag/v0.1.0
