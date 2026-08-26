use anyhow::{Context, Result};
use runemark::{
    ColorMode, Console, ErrorBlock, Finding, FindingGroup, Location, Metric, NextStep, Report,
    Tone, Trend, Verdict,
};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

use crate::history::HistoryTrend;
use crate::level_b::FingerprintSummary;

pub fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    if bytes >= TIB {
        format!("{:.2} TiB", bytes as f64 / TIB as f64)
    } else if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.2} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.2} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleaningLevel {
    Coarse,
    Fine,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectActionResult {
    pub project_name: String,
    pub project_path: PathBuf,
    pub target_path: PathBuf,
    pub level: CleaningLevel,
    pub original_size_bytes: u64,
    pub reclaimed_bytes: u64,
    pub details: String,
    pub fingerprint_summary: Option<FingerprintSummary>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroomReportSummary {
    pub root_path: PathBuf,
    pub dry_run: bool,
    pub total_projects_scanned: usize,
    pub coarse_cleaned_count: usize,
    pub fine_cleaned_count: usize,
    pub skipped_count: usize,
    pub error_count: usize,
    pub total_reclaimed_bytes: u64,
    pub history_trend: Option<HistoryTrend>,
    pub results: Vec<ProjectActionResult>,
}

pub fn render_report(
    summary: &BroomReportSummary,
    output_format: OutputFormat,
    color_mode: ColorMode,
    out: &mut dyn Write,
) -> Result<()> {
    match output_format {
        OutputFormat::Tty => render_tty_report(summary, color_mode, out),
        OutputFormat::Json => render_json_report(summary, out),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Tty,
    Json,
}

fn render_tty_report(
    summary: &BroomReportSummary,
    color_mode: ColorMode,
    out: &mut dyn Write,
) -> Result<()> {
    let console = Console::new(color_mode, true);

    let mode_str = if summary.dry_run { " (Dry Run)" } else { "" };
    let title = console.paint(Tone::Title, format!("cargo-broom Summary{}", mode_str));
    writeln!(out, "{}", title)?;

    let verdict = if summary.error_count > 0 {
        Verdict::Warning
    } else if summary.total_reclaimed_bytes > 0 {
        Verdict::Info
    } else {
        Verdict::Passed
    };

    let mut report = Report::new("cargo-broom", verdict);

    if summary.total_reclaimed_bytes > 0 {
        let metric_label = if summary.dry_run {
            "Reclaimable"
        } else {
            "Reclaimed"
        };
        report = report.add_metric(Metric::new(
            metric_label,
            format_bytes(summary.total_reclaimed_bytes),
        ));
    }

    if let Some(trend) = &summary.history_trend {
        report = report.add_metric(history_metric(trend));
    }

    // Group A: Coarse Cleaned (Level A)
    let coarse_results: Vec<_> = summary
        .results
        .iter()
        .filter(|r| r.level == CleaningLevel::Coarse)
        .collect();
    if !coarse_results.is_empty() {
        let mut group = FindingGroup::new("Level A — Full Target Clean (Coarse)");
        for res in coarse_results {
            let finding = Finding::new(
                Tone::Info,
                format!(
                    "{} ({})",
                    res.project_name,
                    format_bytes(res.reclaimed_bytes)
                ),
            )
            .with_location(Location::Artifact(res.target_path.clone()));
            group = group.add_finding(finding);
        }
        report = report.add_group(group);
    }

    // Group B: Fine Cleaned (Level B)
    let fine_results: Vec<_> = summary
        .results
        .iter()
        .filter(|r| r.level == CleaningLevel::Fine)
        .collect();
    if !fine_results.is_empty() {
        let mut group = FindingGroup::new("Level B — Selective / Experimental Fine Clean");
        for res in fine_results {
            let details = if let Some(ref fp) = res.fingerprint_summary {
                format!(
                    "{} (pruned {} stale fingerprints, {} files)",
                    res.project_name, fp.stale_fingerprints, fp.removed_files_count
                )
            } else {
                res.project_name.clone()
            };
            let finding = Finding::new(
                Tone::Info,
                format!("{} — {}", details, format_bytes(res.reclaimed_bytes)),
            )
            .with_location(Location::Artifact(res.target_path.clone()));
            group = group.add_finding(finding);
        }
        report = report.add_group(group);
    }

    // Next step recommendation
    if summary.dry_run && summary.total_reclaimed_bytes > 0 {
        let step = NextStep::new(format!(
            "Run `cargo broom -y` to reclaim {}",
            format_bytes(summary.total_reclaimed_bytes)
        ))
        .with_command("cargo broom -y");
        report = report.add_next_step(step);
    }

    let rendered = report.render(console);
    writeln!(out, "{}", rendered)?;

    // Output individual project errors using ErrorBlock
    for res in &summary.results {
        if let Some(ref err_msg) = res.error {
            let block = ErrorBlock::new(format!("Error processing {}", res.project_name))
                .with_explanation(err_msg);
            writeln!(out, "{}", block.render(console))?;
        }
    }

    Ok(())
}

/// Turns a `HistoryTrend` (net drift since each tracked target's oldest still-retained
/// `--history` entry) into a `runemark::Metric`: `Trend::Positive` when targets are net
/// smaller than they were, `Trend::Negative` when they have grown back despite cleanup.
fn history_metric(trend: &HistoryTrend) -> Metric {
    let delta = trend.current_total_bytes as i64 - trend.oldest_total_bytes as i64;
    let (direction, sign) = if delta <= 0 {
        (Trend::Positive, "-")
    } else {
        (Trend::Negative, "+")
    };
    let delta_str = format!(
        "{sign}{} over {}d ({} tracked)",
        format_bytes(delta.unsigned_abs()),
        crate::history::RETENTION_DAYS,
        trend.tracked_targets
    );

    Metric::new("History", format_bytes(trend.current_total_bytes)).with_trend(direction, delta_str)
}

fn render_json_report(summary: &BroomReportSummary, out: &mut dyn Write) -> Result<()> {
    let json = serde_json::to_string_pretty(summary)?;
    writeln!(out, "{}", json)?;
    Ok(())
}

/// Parses a human size string like `50MB`, `1GB`, or a plain byte count. Shared by
/// `--keep-size` and `budget --limit`.
pub fn parse_size_string(s: Option<&str>) -> Result<Option<u64>> {
    let Some(s) = s else {
        return Ok(None);
    };
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("size value must not be empty");
    }
    let lower = s.to_lowercase();
    let parsed = if let Some(num_str) = lower.strip_suffix("mb") {
        parse_scaled_size(num_str, 1024 * 1024)
    } else if let Some(num_str) = lower.strip_suffix("gb") {
        parse_scaled_size(num_str, 1024 * 1024 * 1024)
    } else if let Some(num_str) = lower.strip_suffix("kb") {
        parse_scaled_size(num_str, 1024)
    } else if let Some(num_str) = lower.strip_suffix('b') {
        parse_scaled_size(num_str, 1)
    } else {
        parse_scaled_size(s, 1)
    }
    .with_context(|| format!("invalid size value `{s}`"))?;
    Ok(Some(parsed))
}

fn parse_scaled_size(value: &str, multiplier: u64) -> Result<u64> {
    value
        .trim()
        .parse::<u64>()?
        .checked_mul(multiplier)
        .context("size value is too large")
}
