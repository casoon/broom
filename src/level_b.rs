use anyhow::{Context, Result};
use rustc_stable_hash::StableSipHasher128;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
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
    /// Rustup toolchain names (e.g. `stable`, `1.88.0`) whose rustc hash fingerprints
    /// should be kept. Any fingerprint built with a different rustc is stale,
    /// regardless of age. Takes precedence over `installed` when non-empty.
    pub toolchains: Vec<String>,
    /// Keep fingerprints built with any currently `rustup`-installed toolchain
    /// instead of a specific list. Ignored when `toolchains` is non-empty.
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
    /// The `rustc` field from the entry's own fingerprint JSON file: a hash of the
    /// `rustc -vV` output of the toolchain it was built with.
    pub rustc_hash: u64,
}

/// The subset of a Cargo fingerprint JSON file (`.fingerprint/<pkg>-<hash>/*.json`)
/// this module cares about. Cargo's own struct has many more fields (features,
/// target, profile, deps, ...); unknown fields are ignored by serde by default, so
/// this stays forward-compatible with newer Cargo versions that add fields.
#[derive(Debug, Deserialize)]
struct Fingerprint {
    rustc: u64,
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

    // `Some(hashes)` when --toolchains or --installed selects specific toolchains to
    // keep: any fingerprint whose rustc hash isn't in this set is stale regardless of
    // age. `None` disables toolchain-based filtering entirely (pure age/duplicate
    // heuristics, the pre-existing behavior).
    let toolchain_hashes = if options.experimental_fingerprints {
        resolve_toolchain_rustc_hashes(options)?
    } else {
        None
    };

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
                        if let Ok(elapsed) = now.duration_since(mtime)
                            && elapsed >= age_threshold
                        {
                            files_to_remove.insert(p.to_path_buf());
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
                // Single fingerprint for crate, check if it's expired or built with a
                // toolchain that is no longer kept.
                let fp = &crate_fps[0];
                let is_old = now
                    .duration_since(fp.mtime)
                    .map(|e| e >= age_threshold)
                    .unwrap_or(false);
                let is_toolchain_mismatch = is_toolchain_stale(fp, &toolchain_hashes);
                if is_old || is_toolchain_mismatch {
                    stale_fingerprints += 1;
                    collect_stale_artifacts(&profile, fp, &mut files_to_remove);
                }
            } else {
                // Multiple fingerprints for crate: sort by mtime descending
                crate_fps.sort_by_key(|entry| std::cmp::Reverse(entry.mtime));
                // Keep newest fingerprint (index 0) unless it no longer matches a kept
                // toolchain; mark all older ones as stale regardless.
                for (i, fp) in crate_fps.iter().enumerate() {
                    let is_toolchain_mismatch = is_toolchain_stale(fp, &toolchain_hashes);
                    if i > 0 || is_toolchain_mismatch {
                        stale_fingerprints += 1;
                        collect_stale_artifacts(&profile, fp, &mut files_to_remove);
                    }
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

/// True when toolchain-based filtering is active and `fp`'s rustc hash isn't in the
/// kept set. `None` (no `--toolchains`/`--installed`) always returns `false`.
fn is_toolchain_stale(fp: &FingerprintEntry, toolchain_hashes: &Option<HashSet<u64>>) -> bool {
    toolchain_hashes
        .as_ref()
        .is_some_and(|kept| !kept.contains(&fp.rustc_hash))
}

/// This has to match the way Cargo hashes a rustc version into a fingerprint's
/// `rustc` field. Ported from `cargo-sweep` (MIT), which in turn mirrors Cargo's own
/// (unstable, internal) hashing.
fn hash_u64<H: Hash>(hashable: &H) -> u64 {
    let mut hasher = StableSipHasher128::new();
    hashable.hash(&mut hasher);
    Hasher::finish(&hasher)
}

/// The hash algorithm Cargo used prior to Rust 1.85.0. Fingerprints from toolchains
/// older than that still carry hashes computed this way, so both are checked.
#[allow(deprecated)]
fn hash_u64_old<H: Hash>(hashable: &H) -> u64 {
    let mut hasher = std::hash::SipHasher::new_with_keys(0, 0);
    hashable.hash(&mut hasher);
    hasher.finish()
}

/// Runs `rustc [+toolchain] -vV` and returns both the current and the pre-1.85 hash
/// of its output, matching whichever algorithm actually produced the `rustc` field
/// in an on-disk fingerprint built by that toolchain.
fn hash_rustc_version(toolchain: Option<&str>) -> Result<[u64; 2]> {
    let mut cmd = Command::new("rustc");
    if let Some(toolchain) = toolchain {
        cmd.arg(format!("+{toolchain}"));
    }
    cmd.arg("-vV");
    let output = cmd.output().context("failed to run `rustc`")?;
    if !output.status.success() {
        let toolchain_label = toolchain.unwrap_or("default");
        anyhow::bail!(
            "failed to determine rustc version for toolchain `{toolchain_label}`: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let version_str = String::from_utf8_lossy(&output.stdout).to_string();
    Ok([hash_u64(&version_str), hash_u64_old(&version_str)])
}

/// Lists installed rustup toolchain names via `rustup toolchain list`, or `None` if
/// `rustup` itself isn't available.
fn installed_toolchain_names() -> Option<Vec<String>> {
    let output = Command::new("rustup")
        .args(["toolchain", "list"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .map(|name| name.trim().to_string())
            .collect(),
    )
}

/// Resolves `--toolchains`/`--installed` into the set of rustc hashes to keep, or
/// `None` if neither is set (toolchain-based filtering disabled). A fingerprint's
/// build script output can carry a `rustc` hash of `0`; that is always kept, mirroring
/// `cargo-sweep`.
fn resolve_toolchain_rustc_hashes(options: &LevelBOptions) -> Result<Option<HashSet<u64>>> {
    if options.toolchains.is_empty() && !options.installed {
        return Ok(None);
    }

    let mut hashes = HashSet::new();
    hashes.insert(0);

    if !options.toolchains.is_empty() {
        for toolchain in &options.toolchains {
            for hash in hash_rustc_version(Some(toolchain))? {
                hashes.insert(hash);
            }
        }
        return Ok(Some(hashes));
    }

    match installed_toolchain_names() {
        Some(names) if !names.is_empty() => {
            for name in names {
                for hash in hash_rustc_version(Some(&name))? {
                    hashes.insert(hash);
                }
            }
        }
        // No rustup, or rustup reports no toolchains: fall back to the bare `rustc`
        // on PATH, same as cargo-sweep.
        _ => {
            for hash in hash_rustc_version(None)? {
                hashes.insert(hash);
            }
        }
    }

    Ok(Some(hashes))
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
            let rustc_hash = load_fingerprint_json(&entry_path).with_context(|| {
                format!(
                    "unreadable or unsupported fingerprint JSON in: {}",
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
                rustc_hash,
            });
        }
    }

    Ok(entries)
}

/// Loads the `rustc` field from the fingerprint dir's `.json` file, validating that
/// this is really a Cargo fingerprint entry rather than a directory that merely
/// matches the `<crate>-<hash>` naming pattern by coincidence. Mirrors
/// `cargo-sweep`'s `Fingerprint::load`: cargo does not fix the JSON file's name
/// (`lib-<crate>.json`, `bin-<crate>.json`, `run-build-script-<crate>.json`, ...), so
/// this reads the first `.json` file found that parses successfully.
fn load_fingerprint_json(fingerprint_dir: &Path) -> Result<u64> {
    for entry in fs::read_dir(fingerprint_dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let contents = fs::read_to_string(&path)?;
            if let Ok(fingerprint) = serde_json::from_str::<Fingerprint>(&contents) {
                return Ok(fingerprint.rustc);
            }
        }
    }
    anyhow::bail!(
        "no parseable fingerprint JSON file found in {}",
        fingerprint_dir.display()
    )
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
            if let Ok(mtime) = entry.metadata().and_then(|m| m.modified())
                && mtime > latest
            {
                latest = mtime;
            }
        }
    }
    latest
}

fn collect_stale_artifacts(profile: &Path, fp: &FingerprintEntry, out: &mut HashSet<PathBuf>) {
    out.insert(fp.path.clone());

    // Check profile/deps/
    let deps_dir = profile.join("deps");
    if deps_dir.exists()
        && let Ok(entries) = fs::read_dir(&deps_dir)
    {
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path();
            let file_name = p.file_name().unwrap_or_default().to_string_lossy();
            if file_name.contains(&fp.hash) {
                out.insert(p);
            }
        }
    }

    // Check profile/build/
    let build_dir = profile.join("build");
    if build_dir.exists()
        && let Ok(entries) = fs::read_dir(&build_dir)
    {
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path();
            let file_name = p.file_name().unwrap_or_default().to_string_lossy();
            if file_name.contains(&fp.hash) {
                out.insert(p);
            }
        }
    }

    // Check profile/native/
    let native_dir = profile.join("native");
    if native_dir.exists()
        && let Ok(entries) = fs::read_dir(&native_dir)
    {
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path();
            let file_name = p.file_name().unwrap_or_default().to_string_lossy();
            if file_name.contains(&fp.hash) {
                out.insert(p);
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

    /// Sets a path's mtime, file or directory alike. Uses the `filetime` crate instead of
    /// `File::open(path).set_modified()`: on Windows, a read-only handle lacks the
    /// FILE_WRITE_ATTRIBUTES access needed to change mtime, and directories can't be
    /// opened with plain `OpenOptions::write(true)` at all — `filetime` handles both
    /// cross-platform. Test-only; the shipped binary never needs to set mtimes.
    fn set_mtime(path: &Path, time: SystemTime) {
        let ft = filetime::FileTime::from_system_time(time);
        filetime::set_file_mtime(path, ft).unwrap();
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

    /// Arbitrary rustc hash used by tests that don't exercise `--toolchains`/`--installed`
    /// filtering, where the exact value doesn't matter (toolchain-based filtering stays
    /// off unless `LevelBOptions.toolchains`/`installed` is set).
    const DEFAULT_RUSTC_HASH: u64 = 42;

    /// Creates `target/debug/.fingerprint/<crate_name>-<hash>/` plus matching
    /// `deps/<crate_name>-<hash>` and `build/<crate_name>-<hash>` artifacts, and sets
    /// the fingerprint directory's mtime to `age`.
    fn make_fingerprint(
        target_dir: &Path,
        crate_name: &str,
        hash: &str,
        age: SystemTime,
        rustc_hash: u64,
    ) -> PathBuf {
        let fp_dir = target_dir
            .join("debug")
            .join(".fingerprint")
            .join(format!("{}-{}", crate_name, hash));
        fs::create_dir_all(&fp_dir).unwrap();
        let dep_lib = fp_dir.join("dep-lib");
        fs::write(&dep_lib, b"fingerprint").unwrap();
        // Real Cargo fingerprint dirs carry a `<kind>-<crate>.json` file with a `rustc`
        // field; load_fingerprint_json() now requires one to accept the entry as valid.
        let fp_json = fp_dir.join(format!("lib-{}.json", crate_name));
        fs::write(&fp_json, format!(r#"{{"rustc":{}}}"#, rustc_hash)).unwrap();
        // compute_dir_mtime() takes the newest of the dir itself and its children, so
        // every child file must be aged too or the dir's own aged mtime gets overridden.
        set_mtime(&dep_lib, age);
        set_mtime(&fp_json, age);

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
        let old_fp = make_fingerprint(
            &target_dir,
            "mycrate",
            "1111111111111111",
            days_ago(60),
            DEFAULT_RUSTC_HASH,
        );
        let new_fp = make_fingerprint(
            &target_dir,
            "mycrate",
            "2222222222222222",
            days_ago(1),
            DEFAULT_RUSTC_HASH,
        );

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
        let fp = make_fingerprint(
            &target_dir,
            "mycrate",
            "1111111111111111",
            days_ago(30),
            DEFAULT_RUSTC_HASH,
        );

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
        let fp = make_fingerprint(
            &target_dir,
            "mycrate",
            "1111111111111111",
            days_ago(1),
            DEFAULT_RUSTC_HASH,
        );

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
        let fp = make_fingerprint(
            &target_dir,
            "mycrate",
            "1111111111111111",
            days_ago(30),
            DEFAULT_RUSTC_HASH,
        );

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
        make_fingerprint(
            &target_dir,
            "mycrate",
            "1111111111111111",
            days_ago(30),
            DEFAULT_RUSTC_HASH,
        );

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

    // --- Cargo-aware fingerprint JSON validation ---
    //
    // Expected hash values below were computed once with `rustc-stable-hash` /
    // `std::hash::SipHasher` directly against the fixed fixture strings (not against
    // this machine's installed rustc), so these tests are portable across CI runners
    // and don't depend on which rustc version happens to be installed.

    #[test]
    fn hash_u64_matches_known_fixture() {
        let fixture =
            "rustc 1.75.0 (82e1608df 2023-12-21)\nbinary: rustc\nhost: x86_64-unknown-linux-gnu\n"
                .to_string();
        assert_eq!(hash_u64(&fixture), 7566640334266345872);
    }

    #[test]
    fn hash_u64_old_matches_known_fixture() {
        let fixture =
            "rustc 1.75.0 (82e1608df 2023-12-21)\nbinary: rustc\nhost: x86_64-unknown-linux-gnu\n"
                .to_string();
        assert_eq!(hash_u64_old(&fixture), 8083042295820737217);
    }

    #[test]
    fn hash_u64_differs_for_different_rustc_versions() {
        // Two distinct toolchains must not collide, or --toolchains/--installed
        // filtering would keep fingerprints built by the wrong compiler.
        let v1 = "rustc 1.75.0 (82e1608df 2023-12-21)\n".to_string();
        let v2 = "rustc 1.95.0 (59807616e 2026-04-14)\n".to_string();
        assert_ne!(hash_u64(&v1), hash_u64(&v2));
    }

    #[test]
    fn load_fingerprint_json_reads_rustc_field() {
        let dir = unique_dir("json_valid");
        fs::write(dir.join("lib-mycrate.json"), r#"{"rustc":12345}"#).unwrap();
        assert_eq!(load_fingerprint_json(&dir).unwrap(), 12345);
    }

    #[test]
    fn load_fingerprint_json_ignores_unknown_fields() {
        // A real fingerprint JSON carries many more fields (features, target, profile,
        // deps, local, rustflags, config, compile_kind, ...) that vary across Cargo
        // versions. Parsing must not require or choke on any of them.
        let dir = unique_dir("json_extra_fields");
        let real_shaped_json = r#"{"rustc":2179919275645516985,"features":"[\"default\"]","declared_features":"[\"default\"]","target":6810695588070812737,"profile":5347358027863023418,"path":4770492767391667477,"deps":[[1,"dep",false,2]],"local":[{"CheckDepInfo":{"dep_info":"debug/.fingerprint/x/dep-lib-x","checksum":false}}],"rustflags":[],"config":8247474407144887393,"compile_kind":0}"#;
        fs::write(dir.join("lib-mycrate.json"), real_shaped_json).unwrap();
        assert_eq!(load_fingerprint_json(&dir).unwrap(), 2179919275645516985);
    }

    #[test]
    fn load_fingerprint_json_rejects_missing_rustc_field() {
        let dir = unique_dir("json_missing_field");
        fs::write(dir.join("lib-mycrate.json"), r#"{"features":"[]"}"#).unwrap();
        assert!(load_fingerprint_json(&dir).is_err());
    }

    #[test]
    fn load_fingerprint_json_rejects_malformed_json() {
        let dir = unique_dir("json_malformed");
        fs::write(dir.join("lib-mycrate.json"), "not json at all").unwrap();
        assert!(load_fingerprint_json(&dir).is_err());
    }

    #[test]
    fn load_fingerprint_json_rejects_missing_json_file() {
        let dir = unique_dir("json_missing_file");
        fs::write(dir.join("dep-lib-mycrate"), "binary dep-info, not JSON").unwrap();
        assert!(load_fingerprint_json(&dir).is_err());
    }

    #[test]
    fn clean_fine_treats_fingerprint_dir_without_json_as_unsupported() {
        // Folder name matches the `<crate>-<hash>` pattern, but there is no fingerprint
        // JSON at all (e.g. an unexpected future Cargo layout). Must degrade the whole
        // project to Level A instead of guessing from the directory name alone.
        let root = unique_dir("no_json");
        let target_dir = root.join("target");
        let fp_dir = target_dir
            .join("debug")
            .join(".fingerprint")
            .join("mycrate-1111111111111111");
        fs::create_dir_all(&fp_dir).unwrap();
        fs::write(fp_dir.join("dep-lib"), b"fingerprint").unwrap();

        let target = make_project_target(&root);
        let options = LevelBOptions {
            keep_days: 14,
            experimental_fingerprints: true,
            ..Default::default()
        };
        let result = clean_fine(&target, &options, false).unwrap();

        assert!(result.unsupported_fingerprint_data);
    }

    #[test]
    fn clean_fine_prunes_fingerprints_built_with_a_different_toolchain() {
        // Isolates toolchain-hash filtering from the age/duplicate heuristics: two
        // different crates, each with a single fresh fingerprint (age well within
        // keep_days), so only a toolchain mismatch can make either one stale.
        let root = unique_dir("toolchain_filter");
        let target_dir = root.join("target");

        let current_hash = hash_rustc_version(None).expect("rustc must be on PATH to run tests")[0];
        let kept = make_fingerprint(
            &target_dir,
            "keptcrate",
            "1111111111111111",
            days_ago(1),
            current_hash,
        );
        let pruned = make_fingerprint(
            &target_dir,
            "prunedcrate",
            "2222222222222222",
            days_ago(1),
            0xDEAD_BEEF,
        );

        let target = make_project_target(&root);
        let options = LevelBOptions {
            keep_days: 14,
            experimental_fingerprints: true,
            installed: true,
            ..Default::default()
        };
        let summary = clean_fine(&target, &options, false).unwrap();

        assert_eq!(summary.stale_fingerprints, 1);
        assert!(
            !pruned.exists(),
            "fingerprint built with a different rustc must be pruned"
        );
        assert!(
            kept.exists(),
            "fingerprint matching the current toolchain must survive"
        );
    }
}
