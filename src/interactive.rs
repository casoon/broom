use crate::discover::ProjectTarget;
use crate::report::{CleaningLevel, format_bytes};
use anyhow::Result;
use dialoguer::{MultiSelect, theme::ColorfulTheme};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct CandidateTarget {
    pub target: ProjectTarget,
    pub proposed_level: CleaningLevel,
    pub reclaimable_bytes: u64,
}

/// Presents an interactive TUI prompt for selecting targets to clean.
pub fn prompt_interactive_selection(
    candidates: Vec<CandidateTarget>,
) -> Result<Vec<CandidateTarget>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let items: Vec<String> = candidates
        .iter()
        .map(|c| {
            let level_str = match c.proposed_level {
                CleaningLevel::Coarse => "Level A (Full)",
                CleaningLevel::Fine => "Level B (Fingerprints)",
                CleaningLevel::Skipped => "Skip",
            };
            format!(
                "[{}] {} — {} ({})",
                level_str,
                c.target.project_name,
                format_bytes(c.reclaimable_bytes),
                c.target.target_path.display()
            )
        })
        .collect();

    // Default all candidates to selected
    let defaults = vec![true; items.len()];

    let selections = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select target directories to clean (Space to toggle, Enter to confirm):")
        .items(&items)
        .defaults(&defaults)
        .interact()?;

    let mut candidates_map: HashMap<usize, CandidateTarget> =
        candidates.into_iter().enumerate().collect();
    let mut selected_candidates = Vec::new();
    for idx in selections {
        if let Some(cand) = candidates_map.remove(&idx) {
            selected_candidates.push(cand);
        }
    }

    Ok(selected_candidates)
}
