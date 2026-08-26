use anyhow::{Context, Result};
use runemark::{
    ColorMode, Console, Finding, FindingGroup, Location, Metric, NextStep, Report, Tone, Verdict,
};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::report::{OutputFormat, format_bytes};

pub fn run_registry(
    root_path: &Path,
    dry_run: bool,
    auto_confirm: bool,
    output_format: OutputFormat,
    color_mode: ColorMode,
    out: &mut dyn Write,
) -> Result<()> {
    if !dry_run && !auto_confirm {
        anyhow::bail!("refusing to clean the Cargo registry without --dry-run or --yes");
    }

    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs_home().map(|h| h.join(".cargo")));

    let registry_dir = match cargo_home {
        Some(home) => home.join("registry"),
        None => {
            anyhow::bail!("Could not determine $CARGO_HOME or $HOME directory");
        }
    };

    if !registry_dir.exists() {
        if output_format == OutputFormat::Tty {
            let console = Console::new(color_mode, true);
            let report = Report::new("cargo-broom registry", Verdict::Skipped);
            writeln!(out, "{}", report.render(console))?;
        }
        return Ok(());
    }

    // Step 1: Collect all (crate_name, version) pairs referenced in Cargo.lock files under root_path
    let active_crates = collect_active_lock_dependencies(root_path);

    // Step 2: Scan registry/cache subdirectories for .crate files
    let cache_dir = registry_dir.join("cache");
    let mut unreferenced_files = Vec::new();
    let mut total_reclaimable = 0u64;

    if cache_dir.exists() {
        for entry in WalkDir::new(&cache_dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("crate") {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    let crate_stem = &file_name[..file_name.len() - 6]; // strip .crate
                    if let Some((name, version)) = split_crate_name_version(crate_stem) {
                        let is_active =
                            active_crates.contains(&(name.to_string(), version.to_string()));
                        if !is_active {
                            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                            total_reclaimable += size;
                            unreferenced_files.push((
                                path.to_path_buf(),
                                name.to_string(),
                                version.to_string(),
                                size,
                            ));
                        }
                    }
                }
            }
        }
    }

    let mut total_reclaimed = 0;
    if !dry_run {
        for (path, _, _, size) in &unreferenced_files {
            fs::remove_file(path)
                .with_context(|| format!("failed to remove registry crate {}", path.display()))?;
            total_reclaimed += size;
        }
    }
    let reported_bytes = if dry_run {
        total_reclaimable
    } else {
        total_reclaimed
    };

    if output_format == OutputFormat::Json {
        let json_report = serde_json::json!({
            "root_path": root_path,
            "dry_run": dry_run,
            "unreferenced_crates_count": unreferenced_files.len(),
            "total_reclaimable_bytes": if dry_run { reported_bytes } else { 0 },
            "total_reclaimed_bytes": if dry_run { 0 } else { reported_bytes },
        });
        writeln!(out, "{}", serde_json::to_string_pretty(&json_report)?)?;
        return Ok(());
    }

    let mode_str = if dry_run { " (Dry Run)" } else { "" };
    let console = Console::new(color_mode, true);
    let title = console.paint(
        Tone::Title,
        format!("cargo-broom Registry Sweep{}", mode_str),
    );
    writeln!(out, "{}", title)?;

    let verdict = if unreferenced_files.is_empty() {
        Verdict::Passed
    } else {
        Verdict::Info
    };

    let mut report = Report::new("cargo-broom registry", verdict);
    if reported_bytes > 0 {
        let metric_label = if dry_run { "Reclaimable" } else { "Reclaimed" };
        report = report.add_metric(Metric::new(metric_label, format_bytes(reported_bytes)));
    }

    if !unreferenced_files.is_empty() {
        let mut group = FindingGroup::new("Unreferenced Registry Crates");
        for (path, name, version, size) in &unreferenced_files {
            let finding = Finding::new(
                Tone::Info,
                format!("{}-{} ({})", name, version, format_bytes(*size)),
            )
            .with_location(Location::Artifact(path.clone()));
            group = group.add_finding(finding);
        }
        report = report.add_group(group);
    }

    if dry_run && total_reclaimable > 0 {
        let step = NextStep::new(format!(
            "Run `cargo broom registry -y` to reclaim {}",
            format_bytes(total_reclaimable)
        ))
        .with_command("cargo broom registry -y");
        report = report.add_next_step(step);
    }

    writeln!(out, "{}", report.render(console))?;
    Ok(())
}

fn collect_active_lock_dependencies(root: &Path) -> HashSet<(String, String)> {
    let mut active = HashSet::new();
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() && entry.file_name() == "Cargo.lock" {
            if let Ok(content) = fs::read_to_string(entry.path()) {
                parse_cargo_lock_packages(&content, &mut active);
            }
        }
    }
    active
}

fn parse_cargo_lock_packages(content: &str, out: &mut HashSet<(String, String)>) {
    if let Ok(toml_val) = toml::from_str::<toml::Value>(content) {
        if let Some(packages) = toml_val.get("package").and_then(|p| p.as_array()) {
            for pkg in packages {
                let name = pkg.get("name").and_then(|n| n.as_str());
                let version = pkg.get("version").and_then(|v| v.as_str());
                if let (Some(n), Some(v)) = (name, version) {
                    out.insert((n.to_string(), v.to_string()));
                }
            }
        }
    }
}

fn split_crate_name_version(stem: &str) -> Option<(&str, &str)> {
    let idx = stem.rfind('-')?;
    let (name, ver) = stem.split_at(idx);
    let ver = &ver[1..];
    if !ver.is_empty() && ver.chars().next()?.is_ascii_digit() {
        Some((name, ver))
    } else {
        None
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
