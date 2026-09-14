---
title: Installation
description: Install cargo-broom from crates.io, straight from the Git repository, or from a local checkout. Cargo then runs it as `cargo broom`.
order: 1
---

## From crates.io

```sh
cargo install cargo-broom
```

The crate is published as `cargo-broom`. The similarly named crate `broom` on crates.io is an
unrelated project.

## From the repository

```sh
cargo install --git https://github.com/casoon/broom
```

## From a checkout

```sh
git clone https://github.com/casoon/broom.git
cd broom
cargo install --path .
```

## Requirements

cargo-broom needs Rust 1.89 or newer. It uses `std::fs::File::try_lock` to detect a build that
is still running before it touches that build's target directory.

It calls `cargo metadata` to resolve each project's real target directory, so `cargo` must be
on the `PATH` when it runs. That matters for scheduled jobs, where `PATH` is often minimal.

CI runs formatting, Clippy and the test suite on Linux, macOS and Windows.

## Verify the install

```sh
cargo broom --version
# cargo-broom 0.1.0
```

Cargo finds any binary named `cargo-<name>` on the `PATH` and runs it as `cargo <name>`, so
both `cargo broom` and `cargo-broom` work.

## Next steps

Continue with the [Quickstart](../quickstart/) for a first dry run.
