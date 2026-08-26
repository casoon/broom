use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How long a history entry is kept before `append_and_prune` drops it.
pub const RETENTION_DAYS: u64 = 14;

/// One row in the `history.jsonl` log: the resulting size of a target directory as of
/// one `cargo broom --history` run. Used to answer "is this growing back despite
/// regular cleanup?" across runs, not just within a single run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub timestamp_unix: u64,
    pub target_path: PathBuf,
    pub project_name: String,
    pub size_bytes: u64,
}

/// Net size drift for tracked targets between the oldest still-retained entry and this
/// run, surfaced as a `runemark::Trend` metric in the report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryTrend {
    pub tracked_targets: usize,
    pub oldest_total_bytes: u64,
    pub current_total_bytes: u64,
}

/// `~/.local/state/cargo-broom/history.jsonl`, or `None` if `$HOME` is not set.
pub fn history_file_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("cargo-broom")
            .join("history.jsonl")
    })
}

/// Reads existing history entries. Missing files return an empty history; unparseable
/// lines (corrupted or hand-edited log) are skipped instead of failing the whole run.
pub fn load_entries(path: &Path) -> Vec<HistoryEntry> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

/// Writes `existing` (pruned to entries within [`RETENTION_DAYS`] of `now_unix`) followed
/// by `new_entries` back to `path`, creating its parent directory if needed.
pub fn append_and_prune(
    path: &Path,
    existing: &[HistoryEntry],
    new_entries: &[HistoryEntry],
    now_unix: u64,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create history directory: {}", parent.display()))?;
    }

    let cutoff = now_unix.saturating_sub(RETENTION_DAYS * 86400);
    let mut file = std::fs::File::create(path)
        .with_context(|| format!("Failed to write history log: {}", path.display()))?;

    for entry in existing.iter().filter(|e| e.timestamp_unix >= cutoff) {
        writeln!(file, "{}", serde_json::to_string(entry)?)?;
    }
    for entry in new_entries {
        writeln!(file, "{}", serde_json::to_string(entry)?)?;
    }
    Ok(())
}

/// For each target_path, the size recorded in its oldest still-retained entry.
pub fn oldest_size_per_target(entries: &[HistoryEntry]) -> HashMap<PathBuf, u64> {
    let mut oldest: HashMap<PathBuf, (u64, u64)> = HashMap::new();
    for entry in entries {
        oldest
            .entry(entry.target_path.clone())
            .and_modify(|(ts, size)| {
                if entry.timestamp_unix < *ts {
                    *ts = entry.timestamp_unix;
                    *size = entry.size_bytes;
                }
            })
            .or_insert((entry.timestamp_unix, entry.size_bytes));
    }
    oldest
        .into_iter()
        .map(|(path, (_, size))| (path, size))
        .collect()
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ts: u64, target: &str, size: u64) -> HistoryEntry {
        HistoryEntry {
            timestamp_unix: ts,
            target_path: PathBuf::from(target),
            project_name: "dummy".to_string(),
            size_bytes: size,
        }
    }

    #[test]
    fn load_entries_returns_empty_for_missing_file() {
        let path = PathBuf::from("/nonexistent/broom-history-test/history.jsonl");
        assert!(load_entries(&path).is_empty());
    }

    #[test]
    fn load_entries_skips_corrupt_lines() {
        let dir =
            std::env::temp_dir().join(format!("broom_history_test_corrupt_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");
        std::fs::write(
            &path,
            format!(
                "{}\nnot json at all\n{}\n",
                serde_json::to_string(&entry(1, "/a/target", 10)).unwrap(),
                serde_json::to_string(&entry(2, "/b/target", 20)).unwrap()
            ),
        )
        .unwrap();

        let entries = load_entries(&path);
        assert_eq!(entries.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_and_prune_drops_entries_older_than_retention() {
        let dir =
            std::env::temp_dir().join(format!("broom_history_test_prune_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");

        let now = 1_000_000_000u64;
        let too_old = entry(now - (RETENTION_DAYS + 1) * 86400, "/a/target", 10);
        let still_fresh = entry(now - 86400, "/a/target", 20);
        let new_entry = entry(now, "/a/target", 30);

        append_and_prune(&path, &[too_old, still_fresh], &[new_entry], now).unwrap();

        let entries = load_entries(&path);
        assert_eq!(entries.len(), 2, "the too-old entry must be pruned");
        assert!(entries.iter().all(|e| e.size_bytes != 10));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oldest_size_per_target_picks_earliest_timestamp() {
        let entries = vec![
            entry(300, "/a/target", 99),
            entry(100, "/a/target", 5),
            entry(200, "/a/target", 50),
            entry(150, "/b/target", 7),
        ];
        let oldest = oldest_size_per_target(&entries);
        assert_eq!(oldest.get(&PathBuf::from("/a/target")), Some(&5));
        assert_eq!(oldest.get(&PathBuf::from("/b/target")), Some(&7));
    }
}
