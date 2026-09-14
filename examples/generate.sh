#!/usr/bin/env bash
# Regenerates the website showcase fixtures in examples/*.txt.
#
# Creates a handful of tiny Cargo projects in a temporary directory, builds them, ages
# some of their target/ directories, and captures what `cargo broom` reports about them.
# Every cleanup command runs with --dry-run (or is refused before scanning), so nothing
# is removed; the script checks that afterwards. The temporary directory is deleted at
# the end. The only edit to the captured text: the machine-specific temp path is
# replaced with `~/code` so the fixtures stay reviewable.
#
#   examples/generate.sh            (from anywhere; needs cargo)
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
out="$repo/examples"
unset CARGO_TARGET_DIR

cargo build --release --quiet --manifest-path "$repo/Cargo.toml"
broom="$repo/target/release/cargo-broom"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
root="$tmp/code"
mkdir -p "$root"
root="$(cd "$root" && pwd -P)"

new_crate() { # <dir> <name> [bin|lib]
  mkdir -p "$1/src"
  printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n' "$2" >"$1/Cargo.toml"
  if [[ "${3:-bin}" == lib ]]; then
    printf 'pub fn run() -> u32 {\n    42\n}\n' >"$1/src/lib.rs"
  else
    printf 'fn main() {\n    println!("hello from %s");\n}\n' "$2" >"$1/src/main.rs"
  fi
}

# Last touched long ago (fixed date, so the fixtures don't depend on when they are made).
age() { find "$1" -exec touch -h -t 202601011200 {} +; }

# Two projects nobody has built for months: candidates for a full target/ removal.
new_crate "$root/weather-api" weather-api
new_crate "$root/blog-engine" blog-engine
(cd "$root/blog-engine" && cargo build --quiet && cargo build --release --quiet)
(cd "$root/weather-api" && cargo build --quiet)
age "$root/weather-api/target"
age "$root/blog-engine/target"

# A project built today, with generated docs: too recent for a full removal.
new_crate "$root/invoice-cli" invoice-cli
(cd "$root/invoice-cli" && cargo build --quiet && cargo doc --quiet)

# A workspace with two members sharing one target/.
mkdir -p "$root/toolkit"
printf '[workspace]\nmembers = ["core", "cli"]\nresolver = "2"\n' >"$root/toolkit/Cargo.toml"
new_crate "$root/toolkit/core" toolkit-core lib
new_crate "$root/toolkit/cli" toolkit-cli
(cd "$root/toolkit" && cargo build --quiet)

# Two plugins that share one target directory via .cargo/config.toml. Old, but shared
# and overridden, so it is never removed wholesale.
for p in plugin-a plugin-b; do
  new_crate "$root/plugins/$p" "$p"
  mkdir -p "$root/plugins/$p/.cargo"
  printf '[build]\ntarget-dir = "%s"\n' "$root/plugins/shared-target" >"$root/plugins/$p/.cargo/config.toml"
  (cd "$root/plugins/$p" && cargo build --quiet)
done
age "$root/plugins/shared-target"

before="$(find "$root" -type f | wc -l | tr -d ' ')"

capture() { # <file> <shown command> <args…>
  local file="$1" shown="$2"
  shift 2
  local status=0
  "$broom" broom "$@" >"$tmp/raw" 2>&1 || status=$?
  sed "s|$root|~/code|g" "$tmp/raw" >"$out/$file"
  printf '%-18s exit %s  %s\n' "$file" "$status" "$shown"
}

capture dry-run.txt 'cargo broom --dry-run ~/code' --color always --dry-run "$root"
capture keep-size.txt 'cargo broom --dry-run --keep-size 1500KB ~/code' \
  --color always --dry-run --keep-size 1500KB "$root"
capture dry-run-fine.txt 'cargo broom --dry-run --clean-incremental --clean-doc ~/code' \
  --color always --dry-run --clean-incremental --clean-doc "$root"
capture project.txt 'cargo broom project ~/code/weather-api --dry-run' \
  --color always project "$root/weather-api" --dry-run
capture inspect.txt 'cargo broom inspect ~/code' --color always inspect "$root"
capture dry-run-json.txt 'cargo broom --dry-run --format json ~/code' \
  --dry-run --format json "$root"
capture refused.txt 'cargo broom ~/code' --color always "$root"

after="$(find "$root" -type f | wc -l | tr -d ' ')"
if [[ "$before" != "$after" ]]; then
  echo "error: file count changed ($before -> $after), a command removed files" >&2
  exit 1
fi
echo "ok: $after files before and after, nothing removed"
