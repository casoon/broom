# cargo-broom

`cargo-broom` finds Cargo build artifacts across one project or an entire directory
tree and removes regenerable data without turning a routine cleanup into a risky
filesystem operation.

The current `0.1.0` codebase is a safety-first alpha. Conservative target cleanup,
inspection, JSON reporting, and selective cache cleanup are available. Cargo
fingerprint pruning remains experimental and is disabled by default.

## Why cargo-broom?

`cargo clean` works well for one known project. It does not answer which of dozens
of local projects occupy the most space, which targets have gone cold, or what a
scheduled cleanup would remove. `cargo-broom` adds recursive discovery, age and
size policies, dry runs, interactive selection, and machine-readable reports.

## Quick start

Install from a checkout:

```bash
git clone https://github.com/casoon/broom.git
cd broom
cargo install --path .
```

Inspect first. Neither command modifies files:

```bash
cargo broom inspect ~/GitHub
cargo broom --dry-run ~/GitHub
```

Run a confirmed cleanup after reviewing the dry run:

```bash
cargo broom --yes --keep-days 14 --keep-size 50MB ~/GitHub
```

Calling `cargo broom` without `--dry-run`, `--interactive`, or `--yes` fails before
the project scan begins.

## Safety guarantees

- Project cleanup requires `--interactive` or `--yes`; registry cleanup requires
  `--yes`.
- Whole-target cleanup is limited to a standard local `<workspace>/target`.
- Shared targets, configured target overrides, symlinked targets, and targets
  outside the workspace are never removed wholesale.
- A fingerprint parser error skips fine cleanup for that project. It never falls
  back to deleting the complete target.
- Staleness is judged by `mtime`, not `atime`. `atime` is unreliable on
  `noatime`/`relatime` mounts (common on CI runners and some macOS setups), where it
  is not updated on every read or only coarsely, so recently-scanned-but-unused
  artifacts look falsely "recent". `mtime` is written on every actual Cargo build
  and does not have this problem.
- A target directory currently locked by a running `cargo` process is skipped
  entirely, at both cleanup levels, instead of racing the build.
- Filesystem deletion errors are reported and produce a failing exit status.
- Fingerprint pruning requires the explicit `--experimental-fine` opt-in.
- `--trash` moves Level A targets to the OS trash/recycle bin instead of deleting
  them permanently.

All removed data is generated Cargo output. Deletion can still cause rebuilds, so
use `--dry-run` before enabling an automated job.

## Cleanup modes

| Mode | Selection | What it removes |
| --- | --- | --- |
| Default / Level A | Target is at least `--keep-days` old and, when supplied, at least `--keep-size` large | Complete standard local `target/` |
| `--coarse-only` | Same policy as Level A; fine operations are rejected | Complete standard local `target/` |
| `--clean-incremental` | Targets not selected for Level A | `target/*/incremental` caches |
| `--clean-doc` | Targets not selected for Level A | Generated `target/doc` output |
| `--experimental-fine` | Targets not selected for Level A | Fingerprints and matching hashed artifacts selected by experimental age and duplicate heuristics |
| `--toolchains <LIST>` | Modifier for `--experimental-fine` | Also prunes fingerprints built with a rustc other than the named toolchain(s) |
| `--installed` | Modifier for `--experimental-fine` | Same, but keeps any currently `rustup`-installed toolchain instead of a specific list |
| `--fine-only` | Disables Level A | Only explicitly requested fine operations |
| `--trash` | Modifier for Level A, any policy above | Moves the target to the OS trash/recycle bin instead of deleting it permanently |

Examples:

```bash
# Conservative whole-target cleanup only
cargo broom --yes --coarse-only --keep-days 14 --keep-size 50MB ~/GitHub

# Same, but recoverable: goes to the trash instead of a permanent delete
cargo broom --yes --coarse-only --trash --keep-days 14 --keep-size 50MB ~/GitHub

# Level A for cold projects; incremental caches and docs for the rest
cargo broom --yes --clean-incremental --clean-doc ~/GitHub

# Review experimental fingerprint pruning
cargo broom --dry-run --experimental-fine ~/GitHub

# Also prune fingerprints from toolchains rustup no longer has installed
cargo broom --dry-run --experimental-fine --installed ~/GitHub

# Never remove a complete target
cargo broom --yes --fine-only --clean-incremental --clean-doc ~/GitHub
```

`--fine-only` requires `--experimental-fine`, `--clean-incremental`, or
`--clean-doc`. `--tests-only`, `--toolchains`, and `--installed` require
`--experimental-fine`. Conflicting mode combinations are rejected by the CLI.

`--experimental-fine` validates each fingerprint entry against its own JSON file
(the `rustc` hash Cargo records there) rather than trusting the `.fingerprint/`
directory naming pattern alone; an entry with no parseable fingerprint JSON marks
the whole project unsupported for fine cleanup instead of guessing (see Safety
guarantees above).

## Commands

### Inspect and diagnose

```bash
cargo broom inspect [DIR]
cargo broom doctor [DIR]
cargo broom toolchains [DIR]
cargo broom budget [DIR] --limit 50GB
```

- `inspect` lists discovered targets and their disk usage without applying cleanup
  policies.
- `doctor` reports target overrides, unusually large targets, and target lock
  findings.
- `toolchains` reports installed rustup toolchains that are not referenced by a
  `rust-toolchain` file below `DIR`. It does not uninstall anything.
- `budget` reports total target disk usage across all discovered projects and, with
  `--limit`, flags when that total exceeds the budget together with the largest
  contributors. This is a whole-tree budget, unlike `--keep-size`, which is a
  per-project Level A threshold. Analysis only; it never deletes anything.

### Clean one project

`project` uses the same policy and safety gates as a recursive run:

```bash
cargo broom project ./my-crate --dry-run
cargo broom project ./my-crate --yes --coarse-only --keep-days 14
```

### Clean the Cargo registry cache

```bash
cargo broom registry ~/GitHub --dry-run
cargo broom registry ~/GitHub --yes
```

The registry command scans `Cargo.lock` files below `DIR` and removes cached
`.crate` archives whose name and version are not referenced there. Projects outside
`DIR` are not considered. Cached archives can be downloaded again by Cargo, but a
dry run over the broadest relevant project root is recommended.

## Interactive and JSON output

Use `--interactive` to select proposed targets before deletion:

```bash
cargo broom --interactive ~/GitHub
```

Use JSON for scripts, reports, or scheduled dry runs:

```bash
cargo broom --dry-run --format json ~/GitHub
cargo broom inspect --format json ~/GitHub
cargo broom registry --dry-run --format json ~/GitHub
```

Errors produce a non-zero exit status. JSON reports are written to stdout; process
errors are written to stderr.

## History tracking

`--history` (or `history = true` in `broom.toml`) is an opt-in, off-by-default flag
that appends each target's resulting size to a rolling 14-day JSON Lines log at
`~/.local/state/cargo-broom/history.jsonl`, one line per target per run. Entries
older than 14 days are pruned automatically; dry runs are never recorded, since they
do not reach the sizes they report.

```bash
cargo broom --yes --history ~/GitHub
```

Once a target has at least one prior entry, the report gains a `History` metric
comparing the current size against that target's oldest still-retained entry —
answering "is this growing back despite regular cleanup?" instead of only showing
the current run's numbers. The same data is available under `history_trend` in
`--format json` output for scripting.

## Configuration

The first applicable configuration source is used in this order:

1. the file passed through `--config`;
2. `./broom.toml`;
3. `~/.config/cargo-broom/config.toml`.

An explicit file is not merged with the local or global file. Scalar CLI values
take precedence; `ignore` and `skip` lists are combined. Invalid files and unknown
keys are errors instead of being silently ignored. `~` is expanded in `root_path`.

```toml
root_path = "~/GitHub"
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

Confirmation is intentionally not configurable. An unattended destructive run
must include `--yes` in the command itself.

## Scheduled cleanup

Start by logging dry-run output for several runs:

```bash
cargo broom --dry-run --format json ~/GitHub
```

After reviewing the policy, an explicit conservative command is suitable for cron
or launchd:

```bash
cargo broom --yes --coarse-only --keep-days 14 --keep-size 50MB ~/GitHub
```

The included `de.casoon.cargo-broom.plist` is a machine-specific example. Adjust
its executable, root, and log paths before loading it.

## CI build caches

`cargo-broom` targets a developer machine with many long-lived local checkouts,
cleaned up periodically by age and size. It is not a fit for a `target/` directory
persisted across CI runs (e.g. via `actions/cache`): that cache grows for a
different reason — every dependency bump leaves behind fingerprints from the
previous `Cargo.lock` state — and calls for cache-key-based or content-addressed
invalidation instead of an age heuristic. For that case, prefer a tool built for it:

- **[`Swatinem/rust-cache`](https://github.com/Swatinem/rust-cache)** — GitHub
  Action that keys the cache on `Cargo.lock` plus toolchain version and prunes
  known-safe-to-drop paths (`incremental/`, final binaries) before saving. The
  default choice for GitHub Actions.
- **[`sccache`](https://github.com/mozilla/sccache) with a remote backend** (S3,
  GCS, or the GitHub Actions cache backend) — content-addressed compiler cache, so
  entries are looked up by input hash rather than file age; sidesteps staleness
  entirely.
- **[`cargo-chef`](https://github.com/LukeMathWalker/cargo-chef)** — for
  Docker-based pipelines, separates dependency compilation from application
  compilation into distinct, correctly invalidated Docker layers.
- **Registry-only caching** — cache just `~/.cargo/registry` and `~/.cargo/git`
  and skip `target/` altogether. Simpler, and often enough when CI runners are
  fast and dependency compilation dominates build time.

## Development

The project requires Rust 1.89 or newer (for `std::fs::File::try_lock`, used to detect
an in-progress build before cleaning its target directory).

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo package
```

CI runs formatting, Clippy, and tests on Linux, macOS, and Windows. The package uses
the published `runemark` dependency and does not require a sibling repository.

## License

MIT

The rustc-version hashing in `--toolchains`/`--installed` (`src/level_b.rs`) is
ported from [`cargo-sweep`](https://github.com/holmgr/cargo-sweep) (MIT), which
mirrors Cargo's own internal fingerprint hash.
