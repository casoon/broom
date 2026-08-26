use anyhow::Result;
use runemark::{
    ColorMode, Console, Finding, FindingGroup, Location, NextStep, Report, Tone, Verdict,
};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::report::OutputFormat;

pub fn run_toolchains(
    root_path: &Path,
    output_format: OutputFormat,
    color_mode: ColorMode,
    out: &mut dyn Write,
) -> Result<()> {
    let rustup_home = std::env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs_home().map(|h| h.join(".rustup")));

    let toolchains_dir = match rustup_home {
        Some(home) => home.join("toolchains"),
        None => {
            anyhow::bail!("Could not determine $RUSTUP_HOME or $HOME directory");
        }
    };

    if !toolchains_dir.exists() {
        if output_format == OutputFormat::Tty {
            let console = Console::new(color_mode, true);
            let report = Report::new("cargo-broom toolchains", Verdict::Skipped);
            writeln!(out, "{}", report.render(console))?;
        }
        return Ok(());
    }

    // Step 1: List installed toolchains in ~/.rustup/toolchains
    let mut installed_toolchains = Vec::new();
    if let Ok(entries) = fs::read_dir(&toolchains_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                installed_toolchains.push((name.to_string(), path));
            }
        }
    }

    // Step 2: Collect referenced toolchain names from rust-toolchain[.toml] files under root_path
    let referenced_toolchains = collect_referenced_toolchains(root_path);

    // Step 3: Find unreferenced toolchains
    let mut unreferenced = Vec::new();
    for (name, path) in &installed_toolchains {
        let is_referenced = referenced_toolchains
            .iter()
            .any(|ref_name| name.contains(ref_name) || ref_name.contains(name));
        if !is_referenced {
            unreferenced.push((name.clone(), path.clone()));
        }
    }

    if output_format == OutputFormat::Json {
        let json_report = serde_json::json!({
            "root_path": root_path,
            "installed_toolchains_count": installed_toolchains.len(),
            "unreferenced_toolchains": unreferenced.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        });
        writeln!(out, "{}", serde_json::to_string_pretty(&json_report)?)?;
        return Ok(());
    }

    let console = Console::new(color_mode, true);
    let title = console.paint(Tone::Title, "cargo-broom Toolchain Inspection");
    writeln!(out, "{}", title)?;

    let verdict = if unreferenced.is_empty() {
        Verdict::Passed
    } else {
        Verdict::Info
    };

    let mut report = Report::new("cargo-broom toolchains", verdict);

    if !unreferenced.is_empty() {
        let mut group = FindingGroup::new("Unreferenced rustup Toolchains");
        for (name, path) in &unreferenced {
            let finding = Finding::new(
                Tone::Info,
                format!(
                    "Toolchain `{}` is not referenced by any local project",
                    name
                ),
            )
            .with_location(Location::Artifact(path.clone()));
            group = group.add_finding(finding);
        }
        report = report.add_group(group);

        let step = NextStep::new("Uninstall unneeded toolchains using rustup")
            .with_command(format!("rustup toolchain uninstall {}", unreferenced[0].0));
        report = report.add_next_step(step);
    }

    writeln!(out, "{}", report.render(console))?;
    Ok(())
}

fn collect_referenced_toolchains(root: &Path) -> HashSet<String> {
    let mut ref_set = HashSet::new();
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy();
        if (name == "rust-toolchain" || name == "rust-toolchain.toml")
            && let Ok(content) = fs::read_to_string(entry.path())
        {
            if let Ok(toml_val) = toml::from_str::<toml::Value>(&content) {
                if let Some(channel) = toml_val
                    .get("toolchain")
                    .and_then(|t| t.get("channel"))
                    .and_then(|c| c.as_str())
                {
                    ref_set.insert(channel.to_string());
                }
            } else {
                ref_set.insert(content.trim().to_string());
            }
        }
    }
    ref_set
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
