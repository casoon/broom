use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use runemark::ColorMode;
use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use broom::commands::Command;
use broom::config::BroomConfig;
use broom::report::{OutputFormat, parse_size_string};
use broom::{BroomRunnerOptions, run_broom};

#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum CliOutputFormat {
    Tty,
    Json,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum CliColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Parser)]
#[command(
    name = "cargo-broom",
    version,
    about = "Conservative Cargo build-artifact cleaner for one or many projects",
    long_about = "cargo-broom safely reclaims regenerable Rust build artifacts with explicit confirmation, protected local-target cleanup, and optional experimental fingerprint pruning."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Target root directory to scan (default: current directory)
    pub path: Option<PathBuf>,

    /// Days of inactivity before a target directory qualifies for coarse Level A clean
    #[arg(long, global = true)]
    pub keep_days: Option<u64>,

    /// Minimum target size threshold (e.g. 50MB, 1GB)
    #[arg(long, global = true)]
    pub keep_size: Option<String>,

    /// Enable experimental Cargo fingerprint pruning
    #[arg(long, global = true, conflicts_with = "coarse_only")]
    pub experimental_fine: bool,

    /// Only perform selected fine cleanup operations (never delete full target/)
    #[arg(long, global = true, conflicts_with = "coarse_only")]
    pub fine_only: bool,

    /// Only perform coarse target/ directory removal (disable fingerprint sweep)
    #[arg(
        long,
        global = true,
        conflicts_with_all = [
            "fine_only",
            "experimental_fine",
            "tests_only",
            "clean_incremental",
            "clean_doc"
        ]
    )]
    pub coarse_only: bool,

    /// Keep fingerprints built with these rustup toolchains (e.g. `stable`, `1.88.0`);
    /// prune fingerprints built with any other toolchain. Repeatable.
    #[arg(long, global = true, requires = "experimental_fine")]
    pub toolchains: Vec<String>,

    /// Keep fingerprints built with any currently installed rustup toolchain instead
    /// of a specific --toolchains list
    #[arg(long, global = true, requires = "experimental_fine")]
    pub installed: bool,

    /// Target compiled test binaries in deps/ specifically
    #[arg(long, global = true, requires = "experimental_fine")]
    pub tests_only: bool,

    /// Clean target/*/incremental build caches aggressively
    #[arg(long, global = true)]
    pub clean_incremental: bool,

    /// Clean target/doc generated documentation directories
    #[arg(long, global = true)]
    pub clean_doc: bool,

    /// Move Level A (coarse) target directories to the OS trash/recycle bin instead of
    /// deleting them permanently
    #[arg(long, global = true)]
    pub trash: bool,

    /// Track resulting target sizes in ~/.local/state/cargo-broom/history.jsonl (14-day
    /// rolling log) and report growth/shrink trend since each target's oldest entry
    #[arg(long, global = true)]
    pub history: bool,

    /// Calculate reclaimable disk space without deleting any files
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Interactively select target directories to clean
    #[arg(short = 'i', long, global = true)]
    pub interactive: bool,

    /// Automatically confirm deletion without prompting
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,

    /// Output format (tty or json)
    #[arg(long, value_enum, default_value_t = CliOutputFormat::Tty, global = true)]
    pub format: CliOutputFormat,

    /// Color mode (auto, always, never)
    #[arg(long, value_enum, default_value_t = CliColorChoice::Auto, global = true)]
    pub color: CliColorChoice,

    /// Show every finding in a report instead of the first few per group
    #[arg(long, global = true)]
    pub details: bool,

    /// Explicit path to configuration file (broom.toml)
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Include hidden directories (starting with .)
    #[arg(long, global = true)]
    pub hidden: bool,

    /// Ignore directory patterns
    #[arg(long, global = true)]
    pub ignore: Vec<String>,

    /// Skip directory patterns
    #[arg(long, global = true)]
    pub skip: Vec<String>,
}

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "broom" {
        args.remove(1);
    }

    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
            err.exit();
        }
    };

    let mut stdout = std::io::stdout();
    match run(cli, &mut stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

pub fn run(mut cli: Cli, out: &mut dyn Write) -> Result<()> {
    let config = BroomConfig::load(cli.config.as_deref())?;

    let output_format = match cli.format {
        CliOutputFormat::Tty => OutputFormat::Tty,
        CliOutputFormat::Json => OutputFormat::Json,
    };

    let color_mode = match cli.color {
        CliColorChoice::Auto => ColorMode::Auto,
        CliColorChoice::Always => ColorMode::Always,
        CliColorChoice::Never => ColorMode::Never,
    };

    let project_path = match cli.command.take() {
        Some(Command::Project(args)) => Some(args.path),
        Some(subcommand) => {
            return subcommand.run(
                output_format,
                color_mode,
                cli.dry_run,
                cli.yes,
                cli.details,
                out,
            );
        }
        None => None,
    };

    let root_path = project_path
        .or(cli.path)
        .or(config.root_path)
        .unwrap_or_else(|| PathBuf::from("."));
    let root_path = expand_tilde(root_path)?;

    let keep_days = cli.keep_days.or(config.keep_days).unwrap_or(14);
    let config_keep_size = config
        .keep_size_mb
        .map(|mb| {
            mb.checked_mul(1024 * 1024)
                .context("keep_size_mb is too large")
        })
        .transpose()?;
    let keep_size_bytes = parse_size_string(cli.keep_size.as_deref())?.or(config_keep_size);

    let experimental_fine = cli.experimental_fine || config.experimental_fine.unwrap_or(false);
    let fine_only = cli.fine_only || config.fine_only.unwrap_or(false);
    let coarse_only = cli.coarse_only || config.coarse_only.unwrap_or(false);
    let trash = cli.trash || config.trash.unwrap_or(false);
    let history = cli.history || config.history.unwrap_or(false);
    let hidden = cli.hidden || config.hidden.unwrap_or(false);

    let mut ignore = cli.ignore;
    if let Some(cfg_ignore) = config.ignore {
        ignore.extend(cfg_ignore);
    }

    let mut skip = cli.skip;
    if let Some(cfg_skip) = config.skip {
        skip.extend(cfg_skip);
    }

    let opts = BroomRunnerOptions {
        root_path,
        dry_run: cli.dry_run,
        interactive: cli.interactive,
        auto_confirm: cli.yes,
        keep_days,
        keep_size_bytes,
        experimental_fine,
        fine_only,
        coarse_only,
        toolchains: cli.toolchains,
        installed: cli.installed,
        tests_only: cli.tests_only,
        clean_incremental: cli.clean_incremental,
        clean_doc: cli.clean_doc,
        trash,
        history,
        hidden,
        skip,
        ignore,
        output_format,
        color_mode,
        details: cli.details,
    };

    run_broom(opts, out)
}

fn expand_tilde(path: PathBuf) -> Result<PathBuf> {
    let Some(path_str) = path.to_str() else {
        return Ok(path);
    };
    if path_str == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("cannot expand `~`: HOME is not set");
    }
    if let Some(rest) = path_str.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("cannot expand `~`: HOME is not set")?;
        return Ok(home.join(rest));
    }
    Ok(path)
}
