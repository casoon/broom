use crate::discover::{DiscoverOptions, discover_targets};
use crate::report::{OutputFormat, format_bytes};
use anyhow::Result;
use runemark::{ColorMode, Console, Tone};
use std::io::Write;
use std::path::Path;

pub fn run_inspect(
    root_path: &Path,
    output_format: OutputFormat,
    color_mode: ColorMode,
    out: &mut dyn Write,
) -> Result<()> {
    let options = DiscoverOptions::default();
    let targets = discover_targets(root_path, &options)?;

    if output_format == OutputFormat::Json {
        let json = serde_json::to_string_pretty(&targets)?;
        writeln!(out, "{}", json)?;
        return Ok(());
    }

    let console = Console::new(color_mode, true);
    let title = console.paint(
        Tone::Title,
        format!("cargo-broom Inspect — {}", root_path.display()),
    );
    writeln!(out, "{}", title)?;

    let total_size: u64 = targets.iter().map(|t| t.size_bytes).sum();
    let summary_msg = console.paint(
        Tone::Muted,
        format!(
            "Found {} Rust target directories occupying {}",
            targets.len(),
            format_bytes(total_size)
        ),
    );
    writeln!(out, "{}", summary_msg)?;
    writeln!(out)?;

    for t in &targets {
        let override_note = if t.has_target_override {
            " [Target Override]"
        } else {
            ""
        };
        writeln!(
            out,
            " • {:<25} {:>10}  {}{}",
            t.project_name,
            format_bytes(t.size_bytes),
            t.target_path.display(),
            override_note
        )?;
    }

    Ok(())
}
