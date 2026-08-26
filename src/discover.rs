use anyhow::Result;
use cargo_metadata::MetadataCommand;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTarget {
    /// Path to the workspace root directory containing Cargo.toml
    pub project_path: PathBuf,
    /// Name of the project or workspace
    pub project_name: String,
    /// Absolute path to the resolved target directory
    pub target_path: PathBuf,
    /// Size of target directory in bytes
    pub size_bytes: u64,
    /// Latest modified timestamp within target directory or manifest
    pub last_modified: SystemTime,
    /// Whether target directory location is overridden via .cargo/config.toml or CARGO_TARGET_DIR
    pub has_target_override: bool,
    /// Number of distinct projects whose target_directory resolves to this same path
    /// (e.g. multiple repos sharing one global CARGO_TARGET_DIR). Level A must not
    /// coarse-clean a target directory with owner_count > 1: deleting it based on one
    /// project's inactivity would break the other owning projects.
    pub owner_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct DiscoverOptions {
    pub hidden: bool,
    pub skip_patterns: Vec<String>,
    pub ignore_patterns: Vec<String>,
}

/// Recursively search for Rust workspaces/projects and resolve their target directories.
pub fn discover_targets(root: &Path, options: &DiscoverOptions) -> Result<Vec<ProjectTarget>> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut manifest_paths = Vec::new();

    let walker = WalkDir::new(&root).into_iter().filter_entry(|e| {
        let file_name = e.file_name().to_string_lossy();
        if !options.hidden && e.depth() > 0 && file_name.starts_with('.') {
            return false;
        }
        for pattern in &options.skip_patterns {
            if file_name == *pattern || e.path().ends_with(pattern) {
                return false;
            }
        }
        for pattern in &options.ignore_patterns {
            if file_name == *pattern || e.path().ends_with(pattern) {
                return false;
            }
        }
        true
    });

    for entry in walker.filter_map(|e| e.ok()) {
        if entry.file_type().is_file() && entry.file_name() == "Cargo.toml" {
            // Avoid inspecting inside a target directory
            let is_inside_target = entry.path().components().any(|c| c.as_os_str() == "target");
            if !is_inside_target {
                manifest_paths.push(entry.path().to_path_buf());
            }
        }
    }

    // Group manifest paths by workspace root to call cargo_metadata once per workspace root
    let mut resolved_targets: BTreeMap<PathBuf, ProjectTarget> = BTreeMap::new();
    let mut owner_counts: BTreeMap<PathBuf, usize> = BTreeMap::new();
    let mut processed_workspace_roots: HashSet<PathBuf> = HashSet::new();

    for manifest_path in manifest_paths {
        let project_dir = manifest_path
            .parent()
            .unwrap_or(Path::new(""))
            .to_path_buf();

        // Skip if this project directory belongs to an already processed workspace root
        if processed_workspace_roots
            .iter()
            .any(|ws_root| project_dir.starts_with(ws_root) && project_dir != *ws_root)
        {
            continue;
        }

        // Run cargo metadata from project_dir
        let mut cmd = MetadataCommand::new();
        cmd.manifest_path(&manifest_path);
        cmd.current_dir(&project_dir);
        cmd.no_deps(); // Speed up metadata fetching

        match cmd.exec() {
            Ok(metadata) => {
                let ws_root = metadata.workspace_root.as_std_path().to_path_buf();
                processed_workspace_roots.insert(ws_root.clone());

                let target_dir = metadata.target_directory.as_std_path().to_path_buf();
                if !target_dir.exists() {
                    continue; // Skip if target directory doesn't exist on disk
                }

                let project_name = metadata
                    .root_package()
                    .map(|p| p.name.clone())
                    .or_else(|| {
                        metadata
                            .workspace_packages()
                            .first()
                            .map(|p| p.name.clone())
                    })
                    .unwrap_or_else(|| {
                        ws_root
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "workspace".to_string())
                    });

                let (size_bytes, last_modified) = compute_directory_stats(&target_dir);

                // A target_directory that differs from the plain <workspace_root>/target
                // default is overridden, whether via CARGO_TARGET_DIR or .cargo/config.toml.
                let has_target_override = target_dir != ws_root.join("target");

                *owner_counts.entry(target_dir.clone()).or_insert(0) += 1;
                resolved_targets
                    .entry(target_dir.clone())
                    .or_insert(ProjectTarget {
                        project_path: ws_root,
                        project_name,
                        target_path: target_dir,
                        size_bytes,
                        last_modified,
                        has_target_override,
                        owner_count: 0, // backfilled below once every manifest has been processed
                    });
            }
            Err(_) => {
                // If cargo_metadata fails (e.g. invalid Cargo.toml), fallback to standard target/ subfolder check
                let fallback_target = project_dir.join("target");
                if fallback_target.exists() {
                    let project_name = read_crate_name_from_manifest(&manifest_path)
                        .unwrap_or_else(|| {
                            project_dir
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| "project".to_string())
                        });
                    let (size_bytes, last_modified) = compute_directory_stats(&fallback_target);
                    // cargo_metadata failed, so the true resolved target is unknown; fall back
                    // to the env-var/config-presence heuristic as a best-effort signal.
                    let has_target_override = check_target_override(&project_dir);

                    *owner_counts.entry(fallback_target.clone()).or_insert(0) += 1;
                    resolved_targets
                        .entry(fallback_target.clone())
                        .or_insert(ProjectTarget {
                            project_path: project_dir,
                            project_name,
                            target_path: fallback_target,
                            size_bytes,
                            last_modified,
                            has_target_override,
                            owner_count: 0, // backfilled below once every manifest has been processed
                        });
                }
            }
        }
    }

    let mut targets: Vec<ProjectTarget> = resolved_targets.into_values().collect();
    for target in &mut targets {
        target.owner_count = owner_counts.get(&target.target_path).copied().unwrap_or(1);
        if target.owner_count > 1 {
            target.project_name = format!(
                "{} (+{} more, shared target)",
                target.project_name,
                target.owner_count - 1
            );
        }
    }

    Ok(targets)
}

fn read_crate_name_from_manifest(manifest_path: &Path) -> Option<String> {
    if let Ok(content) = std::fs::read_to_string(manifest_path) {
        if let Ok(toml_val) = toml::from_str::<toml::Value>(&content) {
            if let Some(pkg) = toml_val.get("package") {
                if let Some(name) = pkg.get("name").and_then(|n| n.as_str()) {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Recursively computes directory size in bytes and finds the latest modified file time (mtime).
pub fn compute_directory_stats(dir: &Path) -> (u64, SystemTime) {
    let mut total_size = 0u64;
    let mut latest_mtime = SystemTime::UNIX_EPOCH;

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if let Ok(metadata) = entry.metadata() {
            if metadata.is_file() {
                total_size += metadata.len();
            }
            if let Ok(mtime) = metadata.modified() {
                if mtime > latest_mtime {
                    latest_mtime = mtime;
                }
            }
        }
    }

    if latest_mtime == SystemTime::UNIX_EPOCH {
        latest_mtime = dir
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::now());
    }

    (total_size, latest_mtime)
}

fn check_target_override(project_dir: &Path) -> bool {
    if std::env::var_os("CARGO_TARGET_DIR").is_some() {
        return true;
    }
    let config_toml = project_dir.join(".cargo").join("config.toml");
    let config = project_dir.join(".cargo").join("config");
    config_toml.exists() || config.exists()
}
