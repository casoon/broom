---
title: Multi-project mode
description: Point cargo-broom at a directory that holds many Rust projects. It finds every workspace, resolves each real target directory and applies the policy per project.
order: 2
---

## One root, many projects

The default command scans a whole directory tree:

```sh
cargo broom --dry-run ~/code
```

Without a path it scans the current directory. `root_path` in the
[configuration file](../../reference/configuration/) sets a default root.

## How projects are found

1. cargo-broom walks the tree below the root and collects every `Cargo.toml`. Manifests inside
   a directory named `target` are ignored, and so are hidden directories unless you pass
   `--hidden`.
2. For each manifest it runs `cargo metadata --no-deps`. That returns the workspace root and
   the target directory Cargo would really use, including `CARGO_TARGET_DIR` and
   `.cargo/config.toml` overrides.
3. Members of a workspace that has already been resolved are skipped, so a workspace appears
   once, under its root package or first member.
4. Target directories that do not exist on disk are dropped. What remains is measured: total
   size and newest modification time.
5. If several projects resolve to the same target directory, it is listed once with the number
   of additional owners, for example `plugin-a (+1 more, shared target)`.

If `cargo metadata` fails for a manifest, for example because it is invalid, cargo-broom falls
back to `<project>/target` and treats the project as overridden when `CARGO_TARGET_DIR` is set
or `.cargo/config.toml` exists.

## See what is there first

`inspect` lists every discovered target without applying any policy:

```text
$ cargo broom inspect ~/code
cargo-broom Inspect — ~/code
Found 5 Rust target directories occupying 8.86 MiB

 • blog-engine                 1.86 MiB  ~/code/blog-engine/target
 • invoice-cli                 2.83 MiB  ~/code/invoice-cli/target
 • plugin-a (+1 more, shared target)   2.06 MiB  ~/code/plugins/shared-target [Target Override]
 • toolkit-core                1.07 MiB  ~/code/toolkit/target
 • weather-api                 1.03 MiB  ~/code/weather-api/target
```

This output, like the others in these docs, comes from sample projects created by
`examples/generate.sh` in the repository. The [showcase](../../../showcase/) shows it in colour.

Three more read-only commands work on the same set of projects:

- `cargo broom budget ~/code --limit 50GB` adds up all targets and, when the total is over the
  limit, names the largest contributors. Unlike `--keep-size`, which is a per-project threshold,
  this is one budget for the whole tree.
- `cargo broom doctor ~/code` reports local target overrides that conflict with a global
  `CARGO_TARGET_DIR`, target directories over 10 GiB, and builds in progress.
- `cargo broom toolchains ~/code` lists installed rustup toolchains that no `rust-toolchain`
  file below the root refers to.

## The policy is applied per project

Each target is judged on its own. With the defaults, a standard local target whose newest file
is at least 14 days old is proposed for a full removal. Targets built more recently, shared
targets and overridden targets are left alone:

```text
$ cargo broom --dry-run ~/code
cargo-broom Summary (Dry Run)
ℹ cargo-broom

  Reclaimable: 2.89 MiB

● Level A — Full Target Clean (Coarse) (2)
  - blog-engine (1.86 MiB) at ~/code/blog-engine/target
  - weather-api (1.03 MiB) at ~/code/weather-api/target
```

The shared `plugins/shared-target` in this sample is just as old, but it is shared and
overridden, so it is not proposed. See [Cleanup modes](../cleanup-modes/) for the options that
change the selection.

## Narrow the scan

| Option | Effect |
| --- | --- |
| `--skip <NAME>` | Do not descend into directories with this name or path suffix. Repeatable. |
| `--ignore <NAME>` | Same matching; use it for projects you want to keep out of cleanup. Repeatable. |
| `--hidden` | Also descend into directories starting with `.`. |

`skip` and `ignore` lists from the configuration file are added to the ones on the command line.

## Pick targets by hand

`--interactive` shows the proposed targets and lets you choose which to clean:

```sh
cargo broom --interactive ~/code
```

## One project only

The `project` subcommand runs the same policy and safety gates for a single project:

```text
$ cargo broom project ~/code/weather-api --dry-run
cargo-broom Summary (Dry Run)
ℹ cargo-broom

  Reclaimable: 1.03 MiB

● Level A — Full Target Clean (Coarse) (1)
  - weather-api (1.03 MiB) at ~/code/weather-api/target
```
