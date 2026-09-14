---
title: Command line
description: Every command and option of cargo-broom 0.1.0, exit status and JSON output.
order: 1
---

```text
cargo broom [OPTIONS] [PATH] [COMMAND]
```

Without a command, cargo-broom scans `PATH` (default: the current directory, or `root_path`
from the configuration file) and applies the cleanup policy to every project it finds.

## Commands

| Command | Purpose |
| --- | --- |
| `inspect [DIR]` | List discovered target directories and their disk usage. Read-only. |
| `project <PATH>` | Run the cleanup policy for one project, with the same safety gates. |
| `doctor [DIR]` | Report target overrides that conflict with `CARGO_TARGET_DIR`, targets over 10 GiB and builds in progress. Read-only. |
| `registry [DIR]` | Remove cached `.crate` archives that no `Cargo.lock` below `DIR` refers to. Needs `--dry-run` or `--yes`. |
| `toolchains [DIR]` | List installed rustup toolchains that no `rust-toolchain` file below `DIR` refers to. Read-only. |
| `budget [DIR] [--limit SIZE]` | Total target usage across all projects; with `--limit`, the largest contributors when over budget. Read-only. |

## Confirmation

| Option | Effect |
| --- | --- |
| `--dry-run` | Calculate reclaimable space without deleting anything |
| `-i`, `--interactive` | Select target directories before they are cleaned |
| `-y`, `--yes` | Confirm deletion without prompting |

A cleanup run needs one of these. See [Safety rules](../../guides/safety/).

## Policy

| Option | Effect |
| --- | --- |
| `--keep-days <N>` | Days of inactivity before a target qualifies for a full removal. Default 14. |
| `--keep-size <SIZE>` | Minimum target size for a full removal, e.g. `50MB`, `1GB` (base 1024). |
| `--coarse-only` | Full target removal only |
| `--fine-only` | Never remove a complete target; needs a Level B option |
| `--clean-incremental` | Remove `target/*/incremental` caches |
| `--clean-doc` | Remove `target/doc` |
| `--experimental-fine` | Enable experimental fingerprint pruning |
| `--tests-only` | With `--experimental-fine`: only compiled test binaries in `deps/` |
| `--toolchains <NAME>` | With `--experimental-fine`: keep fingerprints of these toolchains, prune the rest. Repeatable. |
| `--installed` | With `--experimental-fine`: keep fingerprints of any installed rustup toolchain |
| `--trash` | Move fully removed targets to the OS trash instead of deleting them |

Details and examples: [Cleanup modes](../../guides/cleanup-modes/).

## Discovery

| Option | Effect |
| --- | --- |
| `--hidden` | Include directories starting with `.` |
| `--skip <PATTERN>` | Skip directories by name or path suffix. Repeatable. |
| `--ignore <PATTERN>` | Ignore directories by name or path suffix. Repeatable. |

## Output and configuration

| Option | Effect |
| --- | --- |
| `--format <tty\|json>` | Output format. Default `tty`. |
| `--color <auto\|always\|never>` | Colour mode. Default `auto`. |
| `--details` | List every entry of a report group. Without it, a group shows the first three and says how many more there are. |
| `--history` | Record resulting target sizes in a 14-day log and report the trend |
| `--config <FILE>` | Use this configuration file instead of looking for one |

## Exit status

`0` on success. Errors, including a project that could not be cleaned, produce a non-zero exit
status. Reports go to stdout, errors to stderr.

## JSON output

`--format json` prints the report as JSON on stdout, for scripts and logged dry runs. From the
sample projects used throughout these docs:

```sh
cargo broom --dry-run --format json ~/code
```

```json
{
  "root_path": "~/code",
  "dry_run": true,
  "total_projects_scanned": 5,
  "coarse_cleaned_count": 2,
  "fine_cleaned_count": 0,
  "skipped_count": 3,
  "error_count": 0,
  "total_reclaimed_bytes": 3032952,
  "history_trend": null,
  "results": [
    {
      "project_name": "blog-engine",
      "project_path": "~/code/blog-engine",
      "target_path": "~/code/blog-engine/target",
      "level": "Coarse",
      "original_size_bytes": 1950676,
      "reclaimed_bytes": 1950676,
      "details": "Coarse clean (1.86 MiB)",
      "fingerprint_summary": null,
      "error": null
    }
  ]
}
```

The second result is omitted here; the full output is in the
[showcase](../../../showcase/dry-run-json/). In a dry run, `total_reclaimed_bytes` is the space
that would be freed. `inspect`, `doctor`, `budget` and `registry` accept `--format json` too.

## Library API

The crate also exposes its internals as the library `broom`. It is documented on
[docs.rs/cargo-broom](https://docs.rs/cargo-broom); it is not a stable interface yet.
