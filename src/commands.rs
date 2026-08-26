use anyhow::Result;
use clap::{Args, Subcommand};
use runemark::ColorMode;
use std::io::Write;
use std::path::PathBuf;

use crate::budget_command::run_budget;
use crate::doctor_command::run_doctor;
use crate::inspect_command::run_inspect;
use crate::registry_command::run_registry;
use crate::report::{OutputFormat, parse_size_string};
use crate::toolchains_command::run_toolchains;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Inspect target directories and disk usage without deleting anything
    Inspect(InspectArgs),
    /// Run broom targeting a single specific project directory
    Project(ProjectArgs),
    /// Run diagnostic checks for target directory anomalies and configuration issues
    Doctor(DoctorArgs),
    /// Scan and clean unreferenced crate versions in ~/.cargo/registry cache
    Registry(RegistryArgs),
    /// Inspect installed rustup toolchains and detect unused toolchains
    Toolchains(ToolchainsArgs),
    /// Report total target disk usage across all discovered projects against a budget
    Budget(BudgetArgs),
}

#[derive(Debug, Args)]
pub struct InspectArgs {
    /// Target root directory to inspect (default: current directory)
    pub path: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ProjectArgs {
    /// Path to the specific project directory
    pub path: PathBuf,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Target root directory to diagnose (default: current directory)
    pub path: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct RegistryArgs {
    /// Target root directory containing projects to analyze (default: current directory)
    pub path: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ToolchainsArgs {
    /// Target root directory containing projects to analyze (default: current directory)
    pub path: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct BudgetArgs {
    /// Target root directory containing projects to analyze (default: current directory)
    pub path: Option<PathBuf>,

    /// Total size budget across all discovered target directories (e.g. 50GB). Without
    /// it, budget only reports current total usage.
    #[arg(long)]
    pub limit: Option<String>,
}

impl Command {
    pub fn run(
        self,
        output_format: OutputFormat,
        color_mode: ColorMode,
        dry_run: bool,
        auto_confirm: bool,
        out: &mut dyn Write,
    ) -> Result<()> {
        match self {
            Command::Inspect(args) => {
                let root = args.path.unwrap_or_else(|| PathBuf::from("."));
                run_inspect(&root, output_format, color_mode, out)
            }
            Command::Project(args) => {
                anyhow::bail!(
                    "project cleanup must be dispatched through the main CLI: {}",
                    args.path.display()
                )
            }
            Command::Doctor(args) => {
                let root = args.path.unwrap_or_else(|| PathBuf::from("."));
                run_doctor(&root, output_format, color_mode, out)
            }
            Command::Registry(args) => {
                let root = args.path.unwrap_or_else(|| PathBuf::from("."));
                run_registry(&root, dry_run, auto_confirm, output_format, color_mode, out)
            }
            Command::Toolchains(args) => {
                let root = args.path.unwrap_or_else(|| PathBuf::from("."));
                run_toolchains(&root, output_format, color_mode, out)
            }
            Command::Budget(args) => {
                let root = args.path.unwrap_or_else(|| PathBuf::from("."));
                let limit_bytes = parse_size_string(args.limit.as_deref())?;
                run_budget(&root, limit_bytes, output_format, color_mode, out)
            }
        }
    }
}
