---
title: Configuration
description: Defaults in broom.toml, where cargo-broom looks for them, and how to run it on a schedule.
order: 2
---

## Where it looks

The first file found is used:

1. the file passed with `--config`;
2. `./broom.toml` in the current directory;
3. `~/.config/cargo-broom/config.toml`.

Files are not merged. Values on the command line win over the file, except `ignore` and `skip`,
whose lists are combined. Invalid files and unknown keys are errors. `~` is expanded in
`root_path`.

## Keys

```toml
root_path = "~/code"
keep_days = 14
keep_size_mb = 50

# Keep experimental pruning disabled for unattended runs.
experimental_fine = false
fine_only = false
coarse_only = false
trash = false
history = false

hidden = false
ignore = ["archived-repo"]
skip = ["node_modules"]
```

| Key | Command-line equivalent |
| --- | --- |
| `root_path` | `PATH` |
| `keep_days` | `--keep-days` |
| `keep_size_mb` | `--keep-size`, in MiB |
| `experimental_fine`, `fine_only`, `coarse_only` | the flags of the same name |
| `trash`, `history`, `hidden` | the flags of the same name |
| `ignore`, `skip` | `--ignore`, `--skip` |

There is no key for confirmation. A run that deletes needs `--yes` or `--interactive` on its
command line.

## History

`--history`, or `history = true`, appends each target's resulting size to
`~/.local/state/cargo-broom/history.jsonl`, one line per target per run. Entries older than 14
days are pruned. Once a target has an earlier entry, the report gains a `History` metric that
compares today's size with the oldest retained one, and JSON output gains `history_trend`. Dry
runs are not recorded.

## Scheduled cleanup

Log dry runs for a while first:

```sh
cargo broom --dry-run --format json ~/code
```

When the selection looks right, an explicit, conservative command is suitable for cron or
launchd:

```sh
cargo broom --yes --coarse-only --keep-days 14 --keep-size 50MB ~/code
```

The repository contains `de.casoon.cargo-broom.plist`, a launchd example for one machine. Adjust
its executable, root and log paths before loading it, and make sure `cargo` is on the `PATH` the
job runs with.
