use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use walkdir::WalkDir;

use crate::discover::ProjectTarget;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FingerprintSummary {
    pub total_fingerprints: usize,
    pub stale_fingerprints: usize,
    pub removed_files_count: usize,
    pub reclaimed_bytes: u64,
    pub unsupported_fingerprint_data: bool,
}

#[derive(Debug, Clone)]
pub struct LevelBOptions {
    pub keep_days: u64,
    pub toolchains: Vec<String>,
    pub installed: bool,
    pub experimental_fingerprints: bool,
    pub tests_only: bool,
    pub clean_incremental: bool,
    pub clean_doc: bool,
}

impl Default for LevelBOptions {
    fn default() -> Self {
        Self {
            keep_days: 14,
            toolchains: Vec::new(),
            installed: false,
            experimental_fingerprints: false,
            tests_only: false,
            clean_incremental: false,
            clean_doc: false,
        }
    }
}

/// Information about a single `.fingerprint/<package>-<hash>` entry.
#[derive(Debug, Clone)]
struct FingerprintEntry {
    pub path: PathBuf,
    pub crate_name: String,
    pub hash: String,
    pub mtime: SystemTime,
}

/// Performs fine-grained fingerprint pruning (Level B) within target profiles (debug, release, etc.)
pub fn clean_fine(
    target: &ProjectTarget,
    options: &LevelBOptions,
    dry_run: bool,
) -> Result<FingerprintSummary> {
    let target_dir = &target.target_path;
    if !target_dir.exists() {
        return Ok(FingerprintSummary {
            total_fingerprints: 0,
            stale_fingerprints: 0,
            removed_files_count: 0,
            reclaimed_bytes: 0,
            unsupported_fingerprint_data: false,
        });
    }

    let mut files_to_remove: HashSet<PathBuf> = HashSet::new();

    // Clean target/doc if --clean-doc is requested
    if options.clean_doc {
        let doc_dir = target_dir.join("doc");
        if doc_dir.exists() {
            files_to_remove.insert(doc_dir);
        }
    }

    // Find profile directories inside target (e.g. debug, release, or custom target triplets)
    let profile_dirs = find_profile_dirs(target_dir);
    let mut total_fingerprints = 0;
    let mut stale_fingerprints = 0;

    let now = SystemTime::now();
    let age_threshold = Duration::from_secs(options.keep_days * 86400);

    for profile in profile_dirs {
        // Clean profile/incremental if --clean-incremental is requested
        if options.clean_incremental {
            let inc_dir = profile.join("incremental");
            if inc_dir.exists() {
                files_to_remove.insert(inc_dir);
            }
        }

        if options.tests_only {
            // --tests-only: specifically clean compiled test binaries in profile/deps
            let deps_dir = profile.join("deps");
            if deps_dir.exists() {
                for entry in WalkDir::new(&deps_dir)
                    .min_depth(1)
                    .max_depth(1)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    let p = entry.path();
                    if p.is_file() && is_test_binary(p) {
                        let mtime = p
                            .metadata()
                            .and_then(|m| m.modified())
                            .unwrap_or(SystemTime::UNIX_EPOCH);
                        if let Ok(elapsed) = now.duration_since(mtime) {
                            if elapsed >= age_threshold {
                                files_to_remove.insert(p.to_path_buf());
                            }
                        }
                    }
                }
            }
            continue;
        }

        if !options.experimental_fingerprints {
            continue;
        }

        let fp_dir = profile.join(".fingerprint");
        if !fp_dir.exists() {
            continue;
        }

        let entries = match parse_fingerprint_dir(&fp_dir) {
            Ok(entries) => entries,
            Err(_) => {
                return Ok(FingerprintSummary {
                    total_fingerprints: 0,
                    stale_fingerprints: 0,
                    removed_files_count: 0,
                    reclaimed_bytes: 0,
                    unsupported_fingerprint_data: true,
                });
            }
        };

        total_fingerprints += entries.len();

        // Group fingerprints by crate name
        let mut grouped: BTreeMap<String, Vec<FingerprintEntry>> = BTreeMap::new();
        for entry in entries {
            grouped
                .entry(entry.crate_name.clone())
                .or_default()
                .push(entry);
        }

        for (_crate_name, mut crate_fps) in grouped {
            if crate_fps.len() <= 1 {
                // Single fingerprint for crate, check if it's expired
                let fp = &crate_fps[0];
                let is_old = now
                    .duration_since(fp.mtime)
                    .map(|e| e >= age_threshold)
                    .unwrap_or(false);
                if is_old {
                    stale_fingerprints += 1;
                    collect_stale_artifacts(&profile, fp, &mut files_to_remove);
                }
            } else {
                // Multiple fingerprints for crate: sort by mtime descending
                crate_fps.sort_by_key(|entry| std::cmp::Reverse(entry.mtime));
                // Keep newest fingerprint (index 0), mark all older ones as stale
                for fp in crate_fps.iter().skip(1) {
                    stale_fingerprints += 1;
                    collect_stale_artifacts(&profile, fp, &mut files_to_remove);
                }
            }
        }
    }

    let mut reclaimed_bytes = 0u64;
    let mut removed_files_count = 0;

    for file_path in &files_to_remove {
        let path_bytes = compute_path_bytes(file_path);
        if dry_run {
            reclaimed_bytes += path_bytes;
            removed_files_count += 1;
        } else if file_path.is_dir() {
            fs::remove_dir_all(file_path)
                .with_context(|| format!("failed to remove {}", file_path.display()))?;
            reclaimed_bytes += path_bytes;
            removed_files_count += 1;
        } else {
            fs::remove_file(file_path)
                .with_context(|| format!("failed to remove {}", file_path.display()))?;
            reclaimed_bytes += path_bytes;
            removed_files_count += 1;
        }
    }

    Ok(FingerprintSummary {
        total_fingerprints,
        stale_fingerprints,
        removed_files_count,
        reclaimed_bytes,
        unsupported_fingerprint_data: false,
    })
}

fn compute_path_bytes(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    if path.is_file() {
        return path.metadata().map(|m| m.len()).unwrap_or(0);
    }
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

fn find_profile_dirs(target_dir: &Path) -> Vec<PathBuf> {
    let mut profiles = Vec::new();
    if let Ok(entries) = fs::read_dir(target_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                // Detect regular profile directories and target-triple/profile nesting.
                if is_profile_dir(&path) {
                    profiles.push(path);
                } else {
                    if let Ok(sub_entries) = fs::read_dir(&path) {
                        for sub in sub_entries.filter_map(|e| e.ok()) {
                            let sub_path = sub.path();
                            if sub_path.is_dir() && is_profile_dir(&sub_path) {
                                profiles.push(sub_path);
                            }
                        }
                    }
                }
            }
        }
    }
    profiles
}

fn is_profile_dir(path: &Path) -> bool {
    path.join(".fingerprint").exists()
        || path.join("incremental").exists()
        || path.join("deps").exists()
}

fn parse_fingerprint_dir(fp_dir: &Path) -> Result<Vec<FingerprintEntry>> {
    let mut entries = Vec::new();
    let read_dir = fs::read_dir(fp_dir)?;

    for entry in read_dir {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            let name_os = entry_path.file_name().unwrap_or_default();
            let name = name_os.to_string_lossy();
            let (crate_name, hash) = split_crate_hash(&name).with_context(|| {
                format!(
                    "unsupported fingerprint directory: {}",
                    entry_path.display()
                )
            })?;
            let mtime = compute_dir_mtime(&entry_path);
            let crate_name_str = crate_name.to_string();
            let hash_str = hash.to_string();
            entries.push(FingerprintEntry {
                path: entry_path,
                crate_name: crate_name_str,
                hash: hash_str,
                mtime,
            });
        }
    }

    Ok(entries)
}

fn split_crate_hash(folder_name: &str) -> Option<(&str, &str)> {
    let idx = folder_name.rfind('-')?;
    let (crate_name, hash) = folder_name.split_at(idx);
    let hash = &hash[1..];
    if hash.len() >= 8 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        Some((crate_name, hash))
    } else {
        None
    }
}

fn compute_dir_mtime(dir: &Path) -> SystemTime {
    let mut latest = dir
        .metadata()
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) {
                if mtime > latest {
                    latest = mtime;
                }
            }
        }
    }
    latest
}

fn collect_stale_artifacts(profile: &Path, fp: &FingerprintEntry, out: &mut HashSet<PathBuf>) {
    out.insert(fp.path.clone());

    // Check profile/deps/
    let deps_dir = profile.join("deps");
    if deps_dir.exists() {
        if let Ok(entries) = fs::read_dir(&deps_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let p = entry.path();
                let file_name = p.file_name().unwrap_or_default().to_string_lossy();
                if file_name.contains(&fp.hash) {
                    out.insert(p);
                }
            }
        }
    }

    // Check profile/build/
    let build_dir = profile.join("build");
    if build_dir.exists() {
        if let Ok(entries) = fs::read_dir(&build_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let p = entry.path();
                let file_name = p.file_name().unwrap_or_default().to_string_lossy();
                if file_name.contains(&fp.hash) {
                    out.insert(p);
                }
            }
        }
    }

    // Check profile/native/
    let native_dir = profile.join("native");
    if native_dir.exists() {
        if let Ok(entries) = fs::read_dir(&native_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let p = entry.path();
                let file_name = p.file_name().unwrap_or_default().to_string_lossy();
                if file_name.contains(&fp.hash) {
                    out.insert(p);
                }
            }
        }
    }
}

fn is_test_binary(path: &Path) -> bool {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    // Test binaries in deps/ usually look like `my_test-1234567890abcdef` without extension
    if path.extension().is_none() && file_name.contains('-') {
        let is_exec = is_executable(path);
        return is_exec;
    }
    false
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = path.metadata() {
        return metadata.permissions().mode() & 0o111 != 0;
    }
    false
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    fn unique_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "broom_level_b_test_{}_{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn set_mtime(path: &Path, time: SystemTime) {
        let file = File::open(path).unwrap();
        file.set_modified(time).unwrap();
    }

    fn days_ago(days: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(days * 86400)
    }

    fn make_project_target(root: &Path) -> ProjectTarget {
        ProjectTarget {
            project_path: root.to_path_buf(),
            project_name: "dummy".to_string(),
            target_path: root.join("target"),
            size_bytes: 0,
            last_modified: SystemTime::now(),
            has_target_override: false,
            owner_count: 1,
        }
    }

    /// Creates `target/debug/.fingerprint/<crate_name>-<hash>/` plus matching
    /// `deps/<crate_name>-<hash>` and `build/<crate_name>-<hash>` artifacts, and sets
    /// the fingerprint directory's mtime to `age`.
    fn make_fingerprint(
        target_dir: &Path,
        crate_name: &str,
        hash: &str,
        age: SystemTime,
    ) -> PathBuf {
        let fp_dir = target_dir
            .join("debug")
            .join(".fingerprint")
            .join(format!("{}-{}", crate_name, hash));
        fs::create_dir_all(&fp_dir).unwrap();
        let dep_lib = fp_dir.join("dep-lib");
        fs::write(&dep_lib, b"fingerprint").unwrap();
        // compute_dir_mtime() takes the newest of the dir itself and its children, so the
        // child file must be aged too or the dir's own aged mtime gets overridden.
        set_mtime(&dep_lib, age);

        let deps_dir = target_dir.join("debug").join("deps");
        fs::create_dir_all(&deps_dir).unwrap();
        fs::write(
            deps_dir.join(format!("lib{}-{}.rlib", crate_name, hash)),
            b"artifact",
        )
        .unwrap();

        let build_dir = target_dir.join("debug").join("build");
        fs::create_dir_all(&build_dir).unwrap();
        fs::write(
            build_dir.join(format!("{}-{}", crate_name, hash)),
            b"build-script",
        )
        .unwrap();

        set_mtime(&fp_dir, age);
        fp_dir
    }

    #[test]
    fn split_crate_hash_accepts_valid_name() {
        assert_eq!(
            split_crate_hash("mycrate-1234567890abcdef"),
            Some(("mycrate", "1234567890abcdef"))
        );
    }

    #[test]
    fn split_crate_hash_rejects_short_hash() {
        assert_eq!(split_crate_hash("mycrate-1234"), None);
    }

    #[test]
    fn split_crate_hash_rejects_non_hex_hash() {
        assert_eq!(split_crate_hash("mycrate-zzzzzzzz"), None);
    }

    #[test]
    fn split_crate_hash_rejects_missing_hyphen() {
        assert_eq!(split_crate_hash("mycrate"), None);
    }

    #[test]
    fn compute_dir_mtime_uses_newest_child() {
        let dir = unique_dir("mtime");
        set_mtime(&dir, days_ago(30));
        let child = dir.join("child");
        fs::write(&child, b"x").unwrap();
        set_mtime(&child, days_ago(1));

        let mtime = compute_dir_mtime(&dir);
        // Newest child mtime should win over the (older) directory mtime itself.
        assert!(mtime > days_ago(2));
    }

    #[test]
    fn clean_fine_keeps_newest_removes_older_duplicate_for_same_crate() {
        let root = unique_dir("dup");
        let target_dir = root.join("target");
        let old_fp = make_fingerprint(&target_dir, "mycrate", "1111111111111111", days_ago(60));
        let new_fp = make_fingerprint(&target_dir, "mycrate", "2222222222222222", days_ago(1));

        let target = make_project_target(&root);
        let options = LevelBOptions {
            keep_days: 14,
            experimental_fingerprints: true,
            ..Default::default()
        };
        let summary = clean_fine(&target, &options, false).unwrap();

        assert_eq!(summary.stale_fingerprints, 1);
        assert!(!summary.unsupported_fingerprint_data);
        assert!(
            !old_fp.exists(),
            "older duplicate fingerprint should be removed"
        );
        assert!(new_fp.exists(), "newest fingerprint must be kept");
    }

    #[test]
    fn clean_fine_removes_single_fingerprint_older_than_threshold() {
        let root = unique_dir("single_old");
        let target_dir = root.join("target");
        let fp = make_fingerprint(&target_dir, "mycrate", "1111111111111111", days_ago(30));

        let target = make_project_target(&root);
        let options = LevelBOptions {
            keep_days: 14,
            experimental_fingerprints: true,
            ..Default::default()
        };
        let summary = clean_fine(&target, &options, false).unwrap();

        assert_eq!(summary.stale_fingerprints, 1);
        assert!(!fp.exists());
        // Matching deps/build artifacts for the stale hash must be removed too.
        assert!(
            !target_dir
                .join("debug")
                .join("deps")
                .join("libmycrate-1111111111111111.rlib")
                .exists()
        );
        assert!(
            !target_dir
                .join("debug")
                .join("build")
                .join("mycrate-1111111111111111")
                .exists()
        );
    }

    #[test]
    fn clean_fine_keeps_single_fingerprint_within_threshold() {
        let root = unique_dir("single_fresh");
        let target_dir = root.join("target");
        let fp = make_fingerprint(&target_dir, "mycrate", "1111111111111111", days_ago(1));

        let target = make_project_target(&root);
        let options = LevelBOptions {
            keep_days: 14,
            experimental_fingerprints: true,
            ..Default::default()
        };
        let summary = clean_fine(&target, &options, false).unwrap();

        assert_eq!(summary.stale_fingerprints, 0);
        assert!(fp.exists());
    }

    #[test]
    fn clean_fine_dry_run_does_not_delete_anything() {
        let root = unique_dir("dry_run");
        let target_dir = root.join("target");
        let fp = make_fingerprint(&target_dir, "mycrate", "1111111111111111", days_ago(30));

        let target = make_project_target(&root);
        let options = LevelBOptions {
            keep_days: 14,
            experimental_fingerprints: true,
            ..Default::default()
        };
        let summary = clean_fine(&target, &options, true).unwrap();

        assert_eq!(summary.stale_fingerprints, 1);
        assert!(summary.removed_files_count > 0);
        assert!(fp.exists(), "dry_run must not delete files on disk");
    }

    #[cfg(unix)]
    #[test]
    fn clean_fine_falls_back_to_coarse_on_unreadable_fingerprint_dir() {
        use std::os::unix::fs::PermissionsExt;

        let root = unique_dir("unreadable");
        let target_dir = root.join("target");
        make_fingerprint(&target_dir, "mycrate", "1111111111111111", days_ago(30));

        let fp_parent = target_dir.join("debug").join(".fingerprint");
        fs::set_permissions(&fp_parent, fs::Permissions::from_mode(0o000)).unwrap();

        let target = make_project_target(&root);
        let options = LevelBOptions {
            keep_days: 14,
            experimental_fingerprints: true,
            ..Default::default()
        };
        let result = clean_fine(&target, &options, false).unwrap();

        // Restore permissions so the temp dir can be cleaned up.
        fs::set_permissions(&fp_parent, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            result.unsupported_fingerprint_data,
            "unreadable fingerprint dir must degrade to Level A, not crash or silently no-op"
        );
    }
}
