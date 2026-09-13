---
title: Overview
description: What cargo-broom does, where it stops, and how this documentation is organised.
order: 0
---

cargo-broom finds Cargo build output across one project or a whole directory tree and removes
the parts that can be regenerated. It is a Cargo subcommand: install the `cargo-broom` crate and
run it as `cargo broom`.

`cargo clean` works well for one project you have in front of you. It does not tell you which
of dozens of local checkouts use the most space, which targets have gone cold, or what a
scheduled cleanup would remove. cargo-broom adds recursive discovery, age and size policies,
dry runs, interactive selection and JSON reports.

## What it covers

- Full removal of cold, standard local `target/` directories (Level A), optionally to the trash.
- Selective cleanup inside active targets: incremental caches, generated docs and, opt-in,
  experimental fingerprint pruning (Level B).
- Read-only inspection: `inspect`, `doctor`, `budget`, `toolchains`.
- Removal of unreferenced `.crate` archives from the Cargo registry cache.

## Where it stops

cargo-broom does not remove shared, overridden or symlinked target directories as a whole, and it
does not touch a target while a build holds its lock. It never deletes without `--dry-run`,
`--interactive` or `--yes`. Version 0.1.0 is a safety-first alpha; fingerprint pruning is
experimental and off by default.

It is meant for a developer machine with many long-lived checkouts, not for `target/`
directories cached between CI runs.

## How the docs are organised

- **Getting started**: install the crate and run a first dry run.
- **Guides**: the [safety rules](guides/safety/), [multi-project mode](guides/multi-project/)
  and the [cleanup modes](guides/cleanup-modes/).
- **Reference**: every command and option, and the configuration file. The library API is
  documented on [docs.rs](https://docs.rs/cargo-broom).
