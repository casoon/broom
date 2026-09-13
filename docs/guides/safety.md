---
title: Safety rules
description: cargo-broom is conservative on purpose. This page lists what it never touches, what it refuses to do without an explicit flag, and where the remaining risk is.
order: 1
---

## It never deletes without being told

A cleanup run needs one of three flags, and fails before the project scan starts without one:

| Flag | Effect |
| --- | --- |
| `--dry-run` | Reports what would be removed. Deletes nothing. |
| `--interactive`, `-i` | Shows the proposed targets and lets you pick them before anything is removed. |
| `--yes`, `-y` | Removes the selected targets without asking. |

```text
$ cargo broom ~/code
Error: refusing to delete without --dry-run, --interactive, or --yes
```

Confirmation is not configurable. `broom.toml` has no key for it, so an unattended run that
deletes must say `--yes` on its own command line. The `registry` subcommand has the same gate:
it needs `--dry-run` or `--yes`.

## What it never removes

- **Source code and manifests.** cargo-broom only removes Cargo build output inside target
  directories, and, with the `registry` subcommand, cached `.crate` archives.
- **Shared target directories, wholesale.** When several projects resolve to the same target
  directory, for example through one global `CARGO_TARGET_DIR`, removing it because one of them
  is inactive would break the others. Such a target is never selected for a full removal.
- **Overridden targets, wholesale.** A target directory moved by `CARGO_TARGET_DIR` or
  `build.target-dir` in `.cargo/config.toml` is never removed as a whole.
- **Anything but a standard local target, wholesale.** A full removal only happens when the
  target directory is exactly `<workspace root>/target`. Symlinked targets and targets outside
  the workspace are skipped.
- **A target that is being built.** Cargo holds a lock on `<target>/.cargo-lock` while it builds.
  If that lock is taken, the target is skipped entirely, at both cleanup levels, instead of
  racing the build.

## It fails instead of guessing

- If experimental fingerprint pruning (`--experimental-fine`) cannot parse a project's
  fingerprint data, fine cleanup is skipped for that project. It never falls back to deleting
  the complete target, and the run exits with a non-zero status.
- Every fingerprint is checked against its own JSON file (the `rustc` hash Cargo records
  there), not just against the directory naming pattern.
- Filesystem errors during deletion are reported per project and make the run fail.
- Invalid configuration files and unknown keys are errors, not silently ignored.

## Age is measured by modification time

A target's age is the newest modification time (`mtime`) of any file inside it. Cargo writes
`mtime` on every real build. Access time (`atime`) is not used: on `noatime` and `relatime`
mounts, common on CI runners and some macOS setups, it is updated only coarsely or not at all,
which would make unused targets look recently used.

The default threshold is 14 days (`--keep-days`). `--keep-size` adds a minimum size, so small
targets stay even when they are old.

## Read-only commands

`inspect`, `doctor`, `budget` and `toolchains` only report. `toolchains` lists rustup
toolchains that no `rust-toolchain` file below the root refers to; it does not uninstall
anything. Dry runs are never written to the `--history` log.

## Where the remaining risk is

All removed data is generated Cargo output, but removing it costs a rebuild. Keep these in mind:

- **Selective cleanup works inside shared targets.** `--clean-incremental`, `--clean-doc` and
  `--experimental-fine` remove paths *inside* a target directory, and they do so for shared and
  overridden targets too. Only the full removal is restricted to standard local targets.
- **Fingerprint pruning is experimental** and disabled by default. Review a
  `--dry-run --experimental-fine` report before using it.
- **The registry cache is global.** `cargo broom registry DIR` only knows the `Cargo.lock` files
  below `DIR`. Crates that projects elsewhere use can be removed from the cache; Cargo downloads
  them again when needed. Run it over the broadest project root you have.

If you want a way back, `--trash` moves full-target removals to the operating system's trash
instead of deleting them permanently. Selective cleanup is always a permanent delete.
