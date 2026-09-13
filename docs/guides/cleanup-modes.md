---
title: Cleanup modes
description: A full target removal for projects that have gone cold, and selective cleanup inside the targets you still use.
order: 3
---

## Two levels

**Level A** removes a complete standard local `target/` directory. A target qualifies when its
newest file is at least `--keep-days` old (default 14) and, if you set `--keep-size`, it is at
least that large.

**Level B** works inside targets that Level A did not select. It only runs when you ask for it:

| Option | What it removes |
| --- | --- |
| `--clean-incremental` | `target/<profile>/incremental` caches |
| `--clean-doc` | Generated `target/doc` output |
| `--experimental-fine` | Stale fingerprints and their hashed artifacts, chosen by age and duplicate heuristics |
| `--tests-only` | With `--experimental-fine`: only compiled test binaries in `deps/` older than `--keep-days` |
| `--toolchains <LIST>` | With `--experimental-fine`: also prune fingerprints built by any other rustc than the named toolchains |
| `--installed` | With `--experimental-fine`: keep fingerprints from any installed rustup toolchain instead of a list |

## Choosing what runs

| Option | Effect |
| --- | --- |
| *(default)* | Level A, plus any Level B options you add |
| `--coarse-only` | Level A only; Level B options are rejected |
| `--fine-only` | Never remove a complete target; needs `--experimental-fine`, `--clean-incremental` or `--clean-doc` |
| `--trash` | Level A moves the target to the OS trash instead of deleting it |

Conflicting combinations are rejected before anything is scanned.

## Thresholds

`--keep-size` takes `KB`, `MB`, `GB` or a plain byte count. Values use 1024 as the base. With
`--keep-size 1500KB`, the smaller of the two cold projects in the sample stays:

```text
$ cargo broom --dry-run --keep-size 1500KB ~/code
cargo-broom Summary (Dry Run)
ℹ cargo-broom

  Reclaimable: 1.86 MiB

● Level A — Full Target Clean (Coarse) (1)
  - blog-engine (1.86 MiB) at ~/code/blog-engine/target
```

## Selective cleanup for active projects

Cold projects lose their whole target; the rest lose only incremental caches and docs:

```text
$ cargo broom --dry-run --clean-incremental --clean-doc ~/code
cargo-broom Summary (Dry Run)
ℹ cargo-broom

  Reclaimable: 5.13 MiB

● Level A — Full Target Clean (Coarse) (2)
  - blog-engine (1.86 MiB) at ~/code/blog-engine/target
  - weather-api (1.03 MiB) at ~/code/weather-api/target

● Level B — Selective / Experimental Fine Clean (3)
  - invoice-cli (pruned 0 stale fingerprints, 2 files) — 1.90 MiB at ~/code/invoice-cli/target
  - plugin-a (+1 more, shared target) (pruned 0 stale fingerprints, 1 files) — 211.43 KiB at ~/code/plugins/shared-target
  - toolkit-core (pruned 0 stale fingerprints, 1 files) — 131.03 KiB at ~/code/toolkit/target
```

In Level B lines, "files" counts removed paths: the `incremental` and `doc` directories count
as one each.

## Experimental fingerprint pruning

`--experimental-fine` looks at `target/<profile>/.fingerprint`. Per crate it keeps the newest
fingerprint and marks older duplicates as stale; a single fingerprint is stale when it is older
than `--keep-days`, or, with `--toolchains`/`--installed`, when it was built by a toolchain you
no longer keep. Stale fingerprints are removed together with their matching artifacts.

Every entry is checked against its fingerprint JSON. If that cannot be read, the project is
reported as unsupported and nothing in it is pruned. Always review a dry run first:

```sh
cargo broom --dry-run --experimental-fine ~/code
cargo broom --dry-run --experimental-fine --installed ~/code
```

## Registry cache

`cargo broom registry DIR` removes cached `.crate` archives from the Cargo registry cache whose
name and version no `Cargo.lock` below `DIR` refers to. It needs `--dry-run` or `--yes`.

```sh
cargo broom registry ~/code --dry-run
```

## CI caches

cargo-broom is built for a developer machine with many long-lived checkouts. For a `target/`
directory cached between CI runs, key the cache on `Cargo.lock` and the toolchain instead, for
example with `Swatinem/rust-cache`, or use `sccache`. The README lists the options.
