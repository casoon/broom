use crate::discover::{DiscoverOptions, discover_targets};
use crate::report::{OutputFormat, detail_level, format_bytes};
use anyhow::Result;
use runemark::{Console, Finding, FindingGroup, Location, Metric, NextStep, Report, Tone, Verdict};
use std::io::Write;
use std::path::Path;

/// Reports total target disk usage across all discovered projects against an optional
/// `--limit` budget. Pure analysis: it never deletes anything, unlike `--keep-size`,
/// which is a per-project Level A threshold.
pub fn run_budget(
    root_path: &Path,
    limit_bytes: Option<u64>,
    output_format: OutputFormat,
    color_mode: runemark::ColorMode,
    details: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let options = DiscoverOptions::default();
    let mut targets = discover_targets(root_path, &options)?;
    targets.sort_by_key(|t| std::cmp::Reverse(t.size_bytes));

    let total_bytes: u64 = targets.iter().map(|t| t.size_bytes).sum();
    let over_budget = limit_bytes.is_some_and(|limit| total_bytes > limit);

    if output_format == OutputFormat::Json {
        let json_report = serde_json::json!({
            "root_path": root_path,
            "total_projects": targets.len(),
            "total_bytes": total_bytes,
            "limit_bytes": limit_bytes,
            "over_budget": over_budget,
        });
        writeln!(out, "{}", serde_json::to_string_pretty(&json_report)?)?;
        return Ok(());
    }

    let console = Console::new(color_mode, true);
    let title = console.paint(
        Tone::Title,
        format!("cargo-broom Budget — {}", root_path.display()),
    );
    writeln!(out, "{}", title)?;

    let verdict = if over_budget {
        Verdict::Warning
    } else {
        Verdict::Passed
    };

    let mut report =
        Report::new("cargo-broom budget", verdict).with_detail_level(detail_level(details));
    report = report.add_metric(Metric::new("Total usage", format_bytes(total_bytes)));
    if let Some(limit) = limit_bytes {
        report = report.add_metric(Metric::new("Budget", format_bytes(limit)));
    }

    if over_budget {
        let limit = limit_bytes.expect("over_budget implies limit_bytes is Some");
        let mut group = FindingGroup::new("Largest Contributors");
        for t in targets.iter().take(5) {
            let finding = Finding::new(
                Tone::Info,
                format!("{} ({})", t.project_name, format_bytes(t.size_bytes)),
            )
            .with_location(Location::Artifact(t.target_path.clone()));
            group = group.add_finding(finding);
        }
        report = report.add_group(group);

        let step = NextStep::new(format!(
            "Total usage exceeds the budget by {}; review the largest projects above",
            format_bytes(total_bytes - limit)
        ));
        report = report.add_next_step(step);
    }

    writeln!(out, "{}", report.render(console))?;
    Ok(())
}
