pub mod budget_command;
pub mod commands;
pub mod config;
pub mod discover;
pub mod doctor_command;
pub mod history;
pub mod inspect_command;
pub mod interactive;
pub mod level_a;
pub mod level_b;
pub mod registry_command;
pub mod report;
pub mod toolchains_command;

use anyhow::Result;
use runemark::{ColorMode, Console, ProgressMode, ProgressSink, TerminalProgress};
use std::io::Write;
use std::path::PathBuf;

use discover::{DiscoverOptions, discover_targets, is_build_running};
use history::{HistoryEntry, HistoryTrend};
use interactive::{CandidateTarget, prompt_interactive_selection};
use level_a::{LevelAOptions, clean_coarse, should_clean_coarse};
use level_b::{LevelBOptions, clean_fine};
use report::{
    BroomReportSummary, CleaningLevel, OutputFormat, ProjectActionResult, format_bytes,
    render_report,
};

#[derive(Debug, Clone)]
pub struct BroomRunnerOptions {
    pub root_path: PathBuf,
    pub dry_run: bool,
    pub interactive: bool,
    pub auto_confirm: bool,
    pub keep_days: u64,
    pub keep_size_bytes: Option<u64>,
    pub experimental_fine: bool,
    pub fine_only: bool,
    pub coarse_only: bool,
    pub toolchains: Vec<String>,
    pub installed: bool,
    pub tests_only: bool,
    pub clean_incremental: bool,
    pub clean_doc: bool,
    pub trash: bool,
    pub history: bool,
    pub hidden: bool,
    pub skip: Vec<String>,
    pub ignore: Vec<String>,
    pub output_format: OutputFormat,
    pub color_mode: ColorMode,
}

/// Records this run's resulting target sizes to the `--history` log and, if any of
/// them were already tracked, returns the net drift since each target's oldest
/// still-retained entry (see `history::RETENTION_DAYS`).
fn record_history(results: &[ProjectActionResult]) -> Option<HistoryTrend> {
    let path = history::history_file_path()?;
    record_history_at(&path, results, history::now_unix())
}

/// Core logic behind `--history`, with the log path and current time as explicit
/// parameters so tests can exercise it without touching the real
/// `~/.local/state/cargo-broom/history.jsonl` or depending on wall-clock time.
fn record_history_at(
    path: &std::path::Path,
    results: &[ProjectActionResult],
    now: u64,
) -> Option<HistoryTrend> {
    let existing = history::load_entries(path);

    let new_entries: Vec<HistoryEntry> = results
        .iter()
        .map(|r| HistoryEntry {
            timestamp_unix: now,
            target_path: r.target_path.clone(),
            project_name: r.project_name.clone(),
            size_bytes: r.original_size_bytes.saturating_sub(r.reclaimed_bytes),
        })
        .collect();

    let oldest = history::oldest_size_per_target(&existing);
    let mut oldest_total_bytes = 0u64;
    let mut current_total_bytes = 0u64;
    let mut tracked_targets = 0usize;
    for entry in &new_entries {
        if let Some(&old_size) = oldest.get(&entry.target_path) {
            oldest_total_bytes += old_size;
            current_total_bytes += entry.size_bytes;
            tracked_targets += 1;
        }
    }

    if let Err(err) = history::append_and_prune(path, &existing, &new_entries, now) {
        eprintln!(
            "Warning: failed to update history log at {}: {err}",
            path.display()
        );
    }

    if tracked_targets == 0 {
        return None; // nothing seen before, so there is no trend to report yet
    }

    Some(HistoryTrend {
        tracked_targets,
        oldest_total_bytes,
        current_total_bytes,
    })
}

pub fn run_broom(opts: BroomRunnerOptions, out: &mut dyn Write) -> Result<()> {
    if !opts.dry_run && !opts.interactive && !opts.auto_confirm {
        anyhow::bail!("refusing to delete without --dry-run, --interactive, or --yes");
    }
    if opts.fine_only && opts.coarse_only {
        anyhow::bail!("--fine-only conflicts with --coarse-only");
    }
    if opts.coarse_only
        && (opts.experimental_fine || opts.tests_only || opts.clean_incremental || opts.clean_doc)
    {
        anyhow::bail!("--coarse-only conflicts with fine-cleaning options");
    }
    if opts.fine_only && !opts.experimental_fine && !opts.clean_incremental && !opts.clean_doc {
        anyhow::bail!(
            "--fine-only requires --experimental-fine, --clean-incremental, or --clean-doc"
        );
    }
    if opts.tests_only && !opts.experimental_fine {
        anyhow::bail!("--tests-only requires --experimental-fine");
    }

    let discover_opts = DiscoverOptions {
        hidden: opts.hidden,
        skip_patterns: opts.skip.clone(),
        ignore_patterns: opts.ignore.clone(),
    };

    let targets = discover_targets(&opts.root_path, &discover_opts)?;
    let level_a_opts = LevelAOptions {
        keep_days: opts.keep_days,
        keep_size_bytes: opts.keep_size_bytes,
    };

    let level_b_opts = LevelBOptions {
        keep_days: opts.keep_days,
        toolchains: opts.toolchains.clone(),
        installed: opts.installed,
        experimental_fingerprints: opts.experimental_fine,
        tests_only: opts.tests_only,
        clean_incremental: opts.clean_incremental,
        clean_doc: opts.clean_doc,
    };

    let total_scanned = targets.len();
    let fine_requested =
        opts.experimental_fine || opts.tests_only || opts.clean_incremental || opts.clean_doc;
    let mut candidates = Vec::new();
    let mut skipped_count = 0;
    let mut results = Vec::new();
    for target in targets {
        if is_build_running(&target.target_path) {
            // A running `cargo build` holds this target directory's lock; deleting or
            // pruning files under it now could corrupt the in-progress build. Skip it
            // outright rather than racing it, and say why instead of a silent no-op.
            skipped_count += 1;
            results.push(ProjectActionResult {
                project_name: target.project_name.clone(),
                project_path: target.project_path.clone(),
                target_path: target.target_path.clone(),
                level: CleaningLevel::Skipped,
                original_size_bytes: target.size_bytes,
                reclaimed_bytes: 0,
                details: "Build in progress, skipped".to_string(),
                fingerprint_summary: None,
                error: None,
            });
            continue;
        }

        let proposed_level = if !opts.fine_only && should_clean_coarse(&target, &level_a_opts) {
            CleaningLevel::Coarse
        } else if !opts.coarse_only && fine_requested {
            CleaningLevel::Fine
        } else {
            CleaningLevel::Skipped
        };

        let reclaimable_bytes = match proposed_level {
            CleaningLevel::Coarse => target.size_bytes,
            CleaningLevel::Fine => clean_fine(&target, &level_b_opts, true)
                .map(|summary| summary.reclaimed_bytes)
                .unwrap_or(0),
            CleaningLevel::Skipped => 0,
        };

        if proposed_level == CleaningLevel::Skipped {
            skipped_count += 1;
        } else {
            candidates.push(CandidateTarget {
                target,
                proposed_level,
                reclaimable_bytes,
            });
        }
    }

    let candidate_count = candidates.len();
    let selected_candidates = if opts.interactive {
        prompt_interactive_selection(candidates)?
    } else {
        candidates
    };

    skipped_count += candidate_count.saturating_sub(selected_candidates.len());
    let mut coarse_count = 0;
    let mut fine_count = 0;
    let mut error_count = 0;
    let mut total_reclaimed = 0u64;

    let is_tty = opts.output_format == OutputFormat::Tty;
    let progress_mode = if is_tty {
        ProgressMode::Auto
    } else {
        ProgressMode::Never
    };
    let console = Console::new(opts.color_mode, is_tty);
    let progress = TerminalProgress::stderr(progress_mode, console, is_tty);
    progress.start(
        total_scanned as u64,
        "Scanning & cleaning target directories",
    );

    for (idx, cand) in selected_candidates.into_iter().enumerate() {
        let target = cand.target;
        progress.advance(idx as u64 + 1, &format!("Cleaning {}", target.project_name));

        match cand.proposed_level {
            CleaningLevel::Coarse => match clean_coarse(&target, opts.dry_run, opts.trash) {
                Ok(freed) => {
                    coarse_count += 1;
                    total_reclaimed += freed;
                    let details = if opts.trash {
                        format!("Coarse clean, moved to trash ({})", format_bytes(freed))
                    } else {
                        format!("Coarse clean ({})", format_bytes(freed))
                    };
                    results.push(ProjectActionResult {
                        project_name: target.project_name.clone(),
                        project_path: target.project_path.clone(),
                        target_path: target.target_path.clone(),
                        level: CleaningLevel::Coarse,
                        original_size_bytes: target.size_bytes,
                        reclaimed_bytes: freed,
                        details,
                        fingerprint_summary: None,
                        error: None,
                    });
                }
                Err(err) => {
                    error_count += 1;
                    results.push(ProjectActionResult {
                        project_name: target.project_name.clone(),
                        project_path: target.project_path.clone(),
                        target_path: target.target_path.clone(),
                        level: CleaningLevel::Coarse,
                        original_size_bytes: target.size_bytes,
                        reclaimed_bytes: 0,
                        details: "Error during coarse clean".to_string(),
                        fingerprint_summary: None,
                        error: Some(err.to_string()),
                    });
                }
            },
            CleaningLevel::Fine => match clean_fine(&target, &level_b_opts, opts.dry_run) {
                Ok(summary) => {
                    if summary.unsupported_fingerprint_data {
                        error_count += 1;
                        results.push(ProjectActionResult {
                            project_name: target.project_name.clone(),
                            project_path: target.project_path.clone(),
                            target_path: target.target_path.clone(),
                            level: CleaningLevel::Fine,
                            original_size_bytes: target.size_bytes,
                            reclaimed_bytes: 0,
                            details: "Unsupported or unreadable fingerprint data".to_string(),
                            fingerprint_summary: None,
                            error: Some(
                                "fine cleanup skipped; coarse fallback is intentionally disabled"
                                    .to_string(),
                            ),
                        });
                    } else {
                        fine_count += 1;
                        total_reclaimed += summary.reclaimed_bytes;
                        results.push(ProjectActionResult {
                            project_name: target.project_name.clone(),
                            project_path: target.project_path.clone(),
                            target_path: target.target_path.clone(),
                            level: CleaningLevel::Fine,
                            original_size_bytes: target.size_bytes,
                            reclaimed_bytes: summary.reclaimed_bytes,
                            details: format!(
                                "Fine clean (reclaimed {})",
                                format_bytes(summary.reclaimed_bytes)
                            ),
                            fingerprint_summary: Some(summary),
                            error: None,
                        });
                    }
                }
                Err(err) => {
                    error_count += 1;
                    results.push(ProjectActionResult {
                        project_name: target.project_name.clone(),
                        project_path: target.project_path.clone(),
                        target_path: target.target_path.clone(),
                        level: CleaningLevel::Fine,
                        original_size_bytes: target.size_bytes,
                        reclaimed_bytes: 0,
                        details: "Error during fine clean".to_string(),
                        fingerprint_summary: None,
                        error: Some(err.to_string()),
                    });
                }
            },
            CleaningLevel::Skipped => {
                skipped_count += 1;
            }
        }
    }

    // A dry run does not actually reach the sizes it reports, so recording it would
    // pollute the trend log with states that never happened.
    let history_trend = if opts.history && !opts.dry_run {
        record_history(&results)
    } else {
        None
    };

    let report_summary = BroomReportSummary {
        root_path: opts.root_path,
        dry_run: opts.dry_run,
        total_projects_scanned: total_scanned,
        coarse_cleaned_count: coarse_count,
        fine_cleaned_count: fine_count,
        skipped_count,
        error_count,
        total_reclaimed_bytes: total_reclaimed,
        history_trend,
        results,
    };

    render_report(&report_summary, opts.output_format, opts.color_mode, out)?;
    if error_count > 0 {
        anyhow::bail!("{} project(s) could not be cleaned", error_count);
    }
    Ok(())
}

#[cfg(test)]
mod history_wiring_tests {
    use super::*;
    use report::CleaningLevel;
    use std::path::PathBuf;

    fn unique_history_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "broom_lib_history_test_{}_{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("history.jsonl")
    }

    fn result_with_size(target_path: &str, size_bytes: u64) -> ProjectActionResult {
        ProjectActionResult {
            project_name: "dummy".to_string(),
            project_path: PathBuf::from("/dummy"),
            target_path: PathBuf::from(target_path),
            level: CleaningLevel::Skipped,
            original_size_bytes: size_bytes,
            reclaimed_bytes: 0,
            details: String::new(),
            fingerprint_summary: None,
            error: None,
        }
    }

    #[test]
    fn record_history_at_returns_none_on_first_ever_run() {
        let path = unique_history_path("first_run");
        let results = vec![result_with_size("/repo/target", 100)];

        let trend = record_history_at(&path, &results, 1_000_000_000);
        assert!(trend.is_none(), "nothing to compare against yet");
        assert_eq!(history::load_entries(&path).len(), 1);
    }

    #[test]
    fn record_history_at_reports_growth_across_runs() {
        let path = unique_history_path("growth");

        let first = vec![result_with_size("/repo/target", 100)];
        record_history_at(&path, &first, 1_000_000_000);

        let second = vec![result_with_size("/repo/target", 400)];
        let trend = record_history_at(&path, &second, 1_000_000_100).unwrap();

        assert_eq!(trend.tracked_targets, 1);
        assert_eq!(trend.oldest_total_bytes, 100);
        assert_eq!(trend.current_total_bytes, 400);
    }
}
