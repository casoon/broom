use crate::discover::ProjectTarget;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct LevelAOptions {
    pub keep_days: u64,
    pub keep_size_bytes: Option<u64>,
}

impl Default for LevelAOptions {
    fn default() -> Self {
        Self {
            keep_days: 14,
            keep_size_bytes: None,
        }
    }
}

/// Evaluates whether a target directory should undergo Level A (coarse / full target removal).
pub fn should_clean_coarse(target: &ProjectTarget, options: &LevelAOptions) -> bool {
    if !is_safe_local_target(target) {
        return false;
    }

    let now = SystemTime::now();
    let age_limit = Duration::from_secs(options.keep_days * 86400);

    let is_older_than_limit = match now.duration_since(target.last_modified) {
        Ok(elapsed) => elapsed >= age_limit,
        Err(_) => false,
    };

    let exceeds_size_limit = match options.keep_size_bytes {
        Some(limit) => target.size_bytes >= limit,
        None => true,
    };

    is_older_than_limit && exceeds_size_limit
}

/// Performs Coarse Clean (Level A) by removing the whole target directory: permanently
/// if `use_trash` is false, or by moving it to the OS trash/recycle bin if true.
pub fn clean_coarse(target: &ProjectTarget, dry_run: bool, use_trash: bool) -> Result<u64> {
    if !is_safe_local_target(target) {
        anyhow::bail!(
            "refusing coarse clean for shared, overridden, or non-standard target directory: {}",
            target.target_path.display()
        );
    }

    if !target.target_path.exists() {
        return Ok(0);
    }

    let bytes_freed = crate::discover::compute_directory_stats(&target.target_path).0;

    if !dry_run {
        if use_trash {
            trash::delete(&target.target_path).with_context(|| {
                format!(
                    "Failed to move target dir to trash: {}",
                    target.target_path.display()
                )
            })?;
        } else {
            remove_dir_all_safe(&target.target_path).with_context(|| {
                format!(
                    "Failed to remove target dir: {}",
                    target.target_path.display()
                )
            })?;
        }
    }

    Ok(bytes_freed)
}

fn is_safe_local_target(target: &ProjectTarget) -> bool {
    if target.owner_count != 1 || target.has_target_override {
        return false;
    }

    let expected = normalize_path(target.project_path.join("target"));
    let actual = normalize_path(target.target_path.clone());
    if actual != expected {
        return false;
    }

    std::fs::symlink_metadata(&target.target_path)
        .map(|metadata| !metadata.file_type().is_symlink())
        .unwrap_or(true)
}

fn normalize_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn remove_dir_all_safe(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn target_with(owner_count: usize, age_days: u64, size_bytes: u64) -> ProjectTarget {
        ProjectTarget {
            project_path: PathBuf::from("/tmp/dummy"),
            project_name: "dummy".to_string(),
            target_path: PathBuf::from("/tmp/dummy/target"),
            size_bytes,
            last_modified: SystemTime::now() - Duration::from_secs(age_days * 86400),
            has_target_override: false,
            owner_count,
        }
    }

    #[test]
    fn should_clean_coarse_true_for_single_owner_old_and_large_enough() {
        let target = target_with(1, 30, 100_000_000);
        let options = LevelAOptions {
            keep_days: 14,
            keep_size_bytes: Some(50_000_000),
        };
        assert!(should_clean_coarse(&target, &options));
    }

    #[test]
    fn should_clean_coarse_false_when_within_keep_days() {
        let target = target_with(1, 1, 100_000_000);
        let options = LevelAOptions {
            keep_days: 14,
            keep_size_bytes: Some(50_000_000),
        };
        assert!(!should_clean_coarse(&target, &options));
    }

    #[test]
    fn should_clean_coarse_false_when_below_keep_size() {
        let target = target_with(1, 30, 10_000_000);
        let options = LevelAOptions {
            keep_days: 14,
            keep_size_bytes: Some(50_000_000),
        };
        assert!(!should_clean_coarse(&target, &options));
    }

    #[test]
    fn should_clean_coarse_false_when_target_is_shared_even_if_old_and_large() {
        // Regression test: a target_directory shared by multiple projects (e.g. one global
        // CARGO_TARGET_DIR for many repos) must never be coarse-cleaned based on a single
        // owner's age/size, since the other owners may still be actively using it.
        let target = target_with(34, 999, 100_000_000_000);
        let options = LevelAOptions {
            keep_days: 14,
            keep_size_bytes: Some(50_000_000),
        };
        assert!(!should_clean_coarse(&target, &options));
    }

    #[test]
    fn should_clean_coarse_false_when_target_is_overridden() {
        let mut target = target_with(1, 999, 100_000_000_000);
        target.has_target_override = true;
        target.target_path = PathBuf::from("/tmp/shared-target");
        let options = LevelAOptions {
            keep_days: 14,
            keep_size_bytes: None,
        };

        assert!(!should_clean_coarse(&target, &options));
        assert!(clean_coarse(&target, true, false).is_err());
    }

    #[test]
    fn clean_coarse_trash_moves_target_out_of_place_without_hard_delete() {
        let dir = std::env::temp_dir().join(format!("broom_level_a_trash_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let target_path = dir.join("target");
        std::fs::create_dir_all(&target_path).unwrap();
        std::fs::write(target_path.join("marker.txt"), b"artifact").unwrap();

        let target = ProjectTarget {
            project_path: dir.clone(),
            project_name: "dummy".to_string(),
            target_path: target_path.clone(),
            size_bytes: 8,
            last_modified: SystemTime::now() - Duration::from_secs(30 * 86400),
            has_target_override: false,
            owner_count: 1,
        };

        let freed = clean_coarse(&target, false, true).unwrap();
        assert!(freed > 0);
        assert!(
            !target_path.exists(),
            "trashed target dir must be gone from its original location"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
