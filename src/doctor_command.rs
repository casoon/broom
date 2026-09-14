use crate::discover::{DiscoverOptions, discover_targets, is_build_running};
use crate::report::{OutputFormat, detail_level, format_bytes};
use anyhow::Result;
use runemark::{ColorMode, Console, Finding, FindingGroup, Location, Report, Tone, Verdict};
use std::io::Write;
use std::path::Path;

pub fn run_doctor(
    root_path: &Path,
    output_format: OutputFormat,
    color_mode: ColorMode,
    details: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let options = DiscoverOptions::default();
    let targets = discover_targets(root_path, &options)?;

    let mut findings_count = 0;
    let mut group = FindingGroup::new("Target Configuration Diagnostics");

    let global_target_var = std::env::var_os("CARGO_TARGET_DIR");

    for t in &targets {
        // Issue 1: CARGO_TARGET_DIR is set globally, but project has a local target dir or config override
        if global_target_var.is_some() && t.has_target_override {
            findings_count += 1;
            let finding = Finding::new(
                Tone::Warning,
                format!(
                    "{} has local `.cargo/config.toml` override despite global $CARGO_TARGET_DIR",
                    t.project_name
                ),
            )
            .with_location(Location::Artifact(t.project_path.clone()));
            group = group.add_finding(finding);
        }

        // Issue 2: Excessively large target folder (> 10 GB)
        if t.size_bytes >= 10 * 1024 * 1024 * 1024 {
            findings_count += 1;
            let finding = Finding::new(
                Tone::Warning,
                format!(
                    "{} target folder is unusually large ({})",
                    t.project_name,
                    format_bytes(t.size_bytes)
                ),
            )
            .with_location(Location::Artifact(t.target_path.clone()));
            group = group.add_finding(finding);
        }

        // Issue 3: a build currently holds this target's lock
        if is_build_running(&t.target_path) {
            findings_count += 1;
            let finding = Finding::new(
                Tone::Info,
                format!(
                    "{} has a build in progress (target directory is locked)",
                    t.project_name
                ),
            )
            .with_location(Location::Artifact(t.target_path.clone()));
            group = group.add_finding(finding);
        }
    }

    if output_format == OutputFormat::Json {
        let report_json = serde_json::json!({
            "root_path": root_path,
            "total_projects": targets.len(),
            "diagnostics_findings": findings_count,
        });
        writeln!(out, "{}", serde_json::to_string_pretty(&report_json)?)?;
        return Ok(());
    }

    let verdict = if findings_count > 0 {
        Verdict::Warning
    } else {
        Verdict::Passed
    };
    let mut report =
        Report::new("cargo-broom doctor", verdict).with_detail_level(detail_level(details));

    if findings_count > 0 {
        report = report.add_group(group);
    }

    let console = Console::new(color_mode, true);
    writeln!(out, "{}", report.render(console))?;
    Ok(())
}
