---
title: Quickstart
description: Look at what is there, preview a cleanup, then run it. The first two steps change nothing on disk.
order: 2
---

## 1. See what is there

Point `inspect` at the directory that holds your projects:

```sh
cargo broom inspect ~/code
```

It lists every Rust target directory it finds, with its size. Nothing is evaluated or removed.

## 2. Preview the cleanup

```sh
cargo broom --dry-run ~/code
```

The dry run applies the default policy, target directories whose newest file is at least 14
days old, and reports what it would remove and how much space that frees:

```text
cargo-broom Summary (Dry Run)
ℹ cargo-broom

  Reclaimable: 2.89 MiB

● Level A — Full Target Clean (Coarse) (2)
  - blog-engine (1.86 MiB) at ~/code/blog-engine/target
  - weather-api (1.03 MiB) at ~/code/weather-api/target
```

Adjust the policy until the list matches what you expect, for example with a longer age and a
minimum size:

```sh
cargo broom --dry-run --keep-days 30 --keep-size 50MB ~/code
```

## 3. Clean up

Pick the targets yourself:

```sh
cargo broom --interactive --keep-days 30 --keep-size 50MB ~/code
```

Or confirm the whole selection up front, and send it to the trash instead of deleting it:

```sh
cargo broom --yes --trash --keep-days 30 --keep-size 50MB ~/code
```

Without `--dry-run`, `--interactive` or `--yes`, cargo-broom refuses to start. The
[safety rules](../../guides/safety/) explain what it never touches.

## Next steps

- [Multi-project mode](../../guides/multi-project/): how projects, workspaces and shared targets
  are found.
- [Cleanup modes](../../guides/cleanup-modes/): selective cleanup for projects you still work on.
- [Configuration](../../reference/configuration/): defaults in `broom.toml` and scheduled runs.
