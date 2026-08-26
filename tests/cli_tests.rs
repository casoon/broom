use broom::commands::{Command, DoctorArgs, InspectArgs, RegistryArgs, ToolchainsArgs};
use broom::discover::{DiscoverOptions, discover_targets};
use broom::level_a::{LevelAOptions, clean_coarse, should_clean_coarse};
use broom::level_b::{LevelBOptions, clean_fine};
use broom::report::OutputFormat;
use broom::{BroomRunnerOptions, run_broom};
use runemark::ColorMode;
use std::fs;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

fn create_temp_workspace(name: &str) -> PathBuf {
    let temp_dir = std::env::temp_dir().join(format!("broom_test_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let manifest = r#"[package]
name = "dummy-pkg"
version = "0.1.0"
edition = "2021"
"#;
    fs::write(temp_dir.join("Cargo.toml"), manifest).unwrap();

    let lock = r#"
[[package]]
name = "dummy-pkg"
version = "0.1.0"

[[package]]
name = "serde"
version = "1.0.200"
"#;
    fs::write(temp_dir.join("Cargo.lock"), lock).unwrap();

    let target_dir = temp_dir.join("target");
    let debug_dir = target_dir.join("debug");
    let fp_dir = debug_dir
        .join(".fingerprint")
        .join("dummy-pkg-1234567890abcdef");
    let deps_dir = debug_dir.join("deps");

    fs::create_dir_all(&fp_dir).unwrap();
    fs::create_dir_all(&deps_dir).unwrap();

    fs::write(fp_dir.join("invokation-12345"), "fp contents").unwrap();
    fs::write(
        deps_dir.join("libdummy_pkg-1234567890abcdef.rlib"),
        "rlib binary bytes",
    )
    .unwrap();

    temp_dir
}

fn cargo_broom_command() -> ProcessCommand {
    let mut command = ProcessCommand::new(env!("CARGO_BIN_EXE_cargo-broom"));
    command.env_remove("CARGO_TARGET_DIR");
    command
}

#[test]
fn test_discover_targets() {
    let ws = create_temp_workspace("discover");
    let opts = DiscoverOptions::default();
    let targets = discover_targets(&ws, &opts).unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].project_name, "dummy-pkg");
    assert_eq!(targets[0].owner_count, 1);
    let _ = fs::remove_dir_all(ws);
}

/// Regression test for a global CARGO_TARGET_DIR scenario: multiple distinct projects whose
/// target_directory resolves to the exact same physical path (via .cargo/config.toml
/// target-dir here, equivalent to a shared CARGO_TARGET_DIR env var) must collapse into a
/// single ProjectTarget with owner_count reflecting all owners, not silently drop all but one.
#[test]
fn test_discover_targets_shared_target_dir_tracks_all_owners() {
    let parent =
        std::env::temp_dir().join(format!("broom_test_shared_parent_{}", std::process::id()));
    let _ = fs::remove_dir_all(&parent);
    fs::create_dir_all(&parent).unwrap();

    let shared_target = parent.join("shared_target");
    fs::create_dir_all(&shared_target).unwrap();

    for name in ["proj-a", "proj-b"] {
        let proj_dir = parent.join(name);
        fs::create_dir_all(proj_dir.join(".cargo")).unwrap();
        fs::create_dir_all(proj_dir.join("src")).unwrap();
        fs::write(
            proj_dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                name
            ),
        )
        .unwrap();
        // A real crate target is required, otherwise `cargo metadata` errors out and
        // discover_targets falls back to the (config-oblivious) <project_dir>/target guess,
        // which would never exercise the shared target-dir path this test is verifying.
        fs::write(proj_dir.join("src").join("lib.rs"), "").unwrap();
        fs::write(
            proj_dir.join(".cargo").join("config.toml"),
            format!("[build]\ntarget-dir = '{}'\n", shared_target.display()),
        )
        .unwrap();
    }

    let opts = DiscoverOptions::default();
    let targets = discover_targets(&parent, &opts).unwrap();

    assert_eq!(
        targets.len(),
        1,
        "both projects resolve to the same physical target dir, so exactly one ProjectTarget must represent it"
    );
    assert_eq!(targets[0].owner_count, 2);
    assert!(targets[0].has_target_override);
    assert!(targets[0].project_name.contains("+1 more"));

    let _ = fs::remove_dir_all(&parent);
}

#[test]
fn test_level_a_coarse_cleaning() {
    let ws = create_temp_workspace("level_a");
    let opts = DiscoverOptions::default();
    let targets = discover_targets(&ws, &opts).unwrap();
    let target = &targets[0];

    let level_a_opts = LevelAOptions {
        keep_days: 0, // 0 days means immediately eligible for coarse clean
        keep_size_bytes: None,
    };

    assert!(should_clean_coarse(target, &level_a_opts));

    let freed = clean_coarse(target, false, false).unwrap();
    assert!(freed > 0);
    assert!(!target.target_path.exists());

    let _ = fs::remove_dir_all(ws);
}

#[test]
fn test_level_b_fine_cleaning() {
    let ws = create_temp_workspace("level_b");

    // Add a second older fingerprint for the same crate to simulate stale artifacts
    let debug_dir = ws.join("target").join("debug");
    let old_fp_dir = debug_dir
        .join(".fingerprint")
        .join("dummy-pkg-0000000000000000");
    let deps_dir = debug_dir.join("deps");
    fs::create_dir_all(&old_fp_dir).unwrap();
    fs::write(old_fp_dir.join("invokation-00000"), "old fp contents").unwrap();
    fs::write(
        deps_dir.join("libdummy_pkg-0000000000000000.rlib"),
        "old rlib binary bytes",
    )
    .unwrap();

    let opts = DiscoverOptions::default();
    let targets = discover_targets(&ws, &opts).unwrap();
    let target = &targets[0];

    let level_b_opts = LevelBOptions {
        keep_days: 14,
        toolchains: Vec::new(),
        installed: false,
        experimental_fingerprints: true,
        tests_only: false,
        clean_incremental: false,
        clean_doc: false,
    };

    let summary = clean_fine(target, &level_b_opts, false).unwrap();
    assert_eq!(summary.total_fingerprints, 2);
    assert_eq!(summary.stale_fingerprints, 1);
    assert!(summary.reclaimed_bytes > 0);

    let _ = fs::remove_dir_all(ws);
}

#[test]
fn test_clean_incremental_and_doc() {
    let ws = create_temp_workspace("inc_doc");
    let target_dir = ws.join("target");
    let debug_dir = target_dir.join("debug");
    let inc_dir = debug_dir.join("incremental");
    let doc_dir = target_dir.join("doc");

    fs::create_dir_all(&inc_dir).unwrap();
    fs::create_dir_all(&doc_dir).unwrap();

    fs::write(inc_dir.join("cache.bin"), "incremental build cache").unwrap();
    fs::write(doc_dir.join("index.html"), "<h1>Rustdoc</h1>").unwrap();

    let opts = DiscoverOptions::default();
    let targets = discover_targets(&ws, &opts).unwrap();
    let target = &targets[0];

    let level_b_opts = LevelBOptions {
        keep_days: 14,
        toolchains: Vec::new(),
        installed: false,
        experimental_fingerprints: false,
        tests_only: false,
        clean_incremental: true,
        clean_doc: true,
    };

    let summary = clean_fine(target, &level_b_opts, false).unwrap();
    assert!(summary.reclaimed_bytes > 0);
    assert!(!inc_dir.exists());
    assert!(!doc_dir.exists());

    let _ = fs::remove_dir_all(ws);
}

#[test]
fn test_command_inspect_and_doctor() {
    let ws = create_temp_workspace("cmd");

    let mut out_inspect = Vec::new();
    let inspect_cmd = Command::Inspect(InspectArgs {
        path: Some(ws.clone()),
    });
    inspect_cmd
        .run(
            OutputFormat::Tty,
            ColorMode::Never,
            true,
            false,
            &mut out_inspect,
        )
        .unwrap();
    let inspect_output = String::from_utf8(out_inspect).unwrap();
    assert!(inspect_output.contains("cargo-broom Inspect"));
    assert!(inspect_output.contains("dummy-pkg"));

    let mut out_doctor = Vec::new();
    let doctor_cmd = Command::Doctor(DoctorArgs {
        path: Some(ws.clone()),
    });
    doctor_cmd
        .run(
            OutputFormat::Tty,
            ColorMode::Never,
            true,
            false,
            &mut out_doctor,
        )
        .unwrap();
    let doctor_output = String::from_utf8(out_doctor).unwrap();
    assert!(doctor_output.contains("cargo-broom doctor"));

    let _ = fs::remove_dir_all(ws);
}

#[test]
fn test_command_registry_and_toolchains() {
    let ws = create_temp_workspace("reg_tc");

    let mut out_reg = Vec::new();
    let reg_cmd = Command::Registry(RegistryArgs {
        path: Some(ws.clone()),
    });
    reg_cmd
        .run(
            OutputFormat::Tty,
            ColorMode::Never,
            true,
            false,
            &mut out_reg,
        )
        .unwrap();
    let reg_output = String::from_utf8(out_reg).unwrap();
    assert!(reg_output.contains("cargo-broom Registry Sweep"));

    let mut out_tc = Vec::new();
    let tc_cmd = Command::Toolchains(ToolchainsArgs {
        path: Some(ws.clone()),
    });
    tc_cmd
        .run(
            OutputFormat::Tty,
            ColorMode::Never,
            true,
            false,
            &mut out_tc,
        )
        .unwrap();
    let tc_output = String::from_utf8(out_tc).unwrap();
    assert!(tc_output.contains("cargo-broom Toolchain Inspection"));

    let _ = fs::remove_dir_all(ws);
}

#[test]
fn test_run_broom_dry_run_json() {
    let ws = create_temp_workspace("dry_run_json");
    let mut out = Vec::new();

    let opts = BroomRunnerOptions {
        root_path: ws.clone(),
        dry_run: true,
        interactive: false,
        auto_confirm: true,
        keep_days: 14,
        keep_size_bytes: None,
        experimental_fine: false,
        fine_only: false,
        coarse_only: false,
        tests_only: false,
        clean_incremental: false,
        clean_doc: false,
        trash: false,
        hidden: false,
        skip: Vec::new(),
        ignore: Vec::new(),
        output_format: OutputFormat::Json,
        color_mode: ColorMode::Never,
    };

    run_broom(opts, &mut out).unwrap();
    let json_output = String::from_utf8(out).unwrap();
    let json_val: serde_json::Value = serde_json::from_str(&json_output).unwrap();

    assert_eq!(json_val["dry_run"], true);
    assert_eq!(json_val["total_projects_scanned"], 1);

    let _ = fs::remove_dir_all(ws);
}

/// Regression test: a target directory whose `.cargo-lock` is held by a running `cargo`
/// process must be skipped entirely, even when it is otherwise eligible for an immediate
/// coarse clean (`keep_days: 0`) and the run is non-interactive/auto-confirmed.
#[test]
fn test_run_broom_skips_target_with_active_build_lock() {
    let ws = create_temp_workspace("locked_target");
    let lock_path = ws.join("target").join(".cargo-lock");
    let holder = fs::File::create(&lock_path).unwrap();
    holder.lock().unwrap();

    let mut out = Vec::new();
    let opts = BroomRunnerOptions {
        root_path: ws.clone(),
        dry_run: false,
        interactive: false,
        auto_confirm: true,
        keep_days: 0,
        keep_size_bytes: None,
        experimental_fine: false,
        fine_only: false,
        coarse_only: false,
        tests_only: false,
        clean_incremental: false,
        clean_doc: false,
        trash: false,
        hidden: false,
        skip: Vec::new(),
        ignore: Vec::new(),
        output_format: OutputFormat::Json,
        color_mode: ColorMode::Never,
    };

    run_broom(opts, &mut out).unwrap();
    let json_val: serde_json::Value = serde_json::from_slice(&out).unwrap();

    assert_eq!(json_val["coarse_cleaned_count"], 0);
    assert_eq!(json_val["skipped_count"], 1);
    assert!(
        ws.join("target").exists(),
        "locked target dir must not be deleted"
    );

    holder.unlock().unwrap();
    let _ = fs::remove_dir_all(ws);
}

#[test]
fn cli_refuses_unconfirmed_cleanup() {
    let ws = create_temp_workspace("cli_unconfirmed");
    let output = cargo_broom_command().arg(&ws).output().unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("refusing to delete"));
    assert!(ws.join("target").exists());

    let _ = fs::remove_dir_all(ws);
}

#[test]
fn cli_yes_allows_standard_coarse_cleanup() {
    let ws = create_temp_workspace("cli_confirmed");
    let output = cargo_broom_command()
        .args(["--yes", "--coarse-only", "--keep-days", "0"])
        .arg(&ws)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!ws.join("target").exists());

    let _ = fs::remove_dir_all(ws);
}

#[test]
fn cli_project_runs_cleanup_for_exact_project() {
    let ws = create_temp_workspace("cli_project");
    let output = cargo_broom_command()
        .args(["project"])
        .arg(&ws)
        .args(["--yes", "--coarse-only", "--keep-days", "0"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!ws.join("target").exists());

    let _ = fs::remove_dir_all(ws);
}

#[test]
fn cli_never_coarse_cleans_overridden_target() {
    let ws = create_temp_workspace("cli_override");
    let shared_target = ws.join("shared-target");
    fs::rename(ws.join("target"), &shared_target).unwrap();
    fs::create_dir_all(ws.join("src")).unwrap();
    fs::write(ws.join("src/lib.rs"), "").unwrap();
    fs::create_dir_all(ws.join(".cargo")).unwrap();
    fs::write(
        ws.join(".cargo/config.toml"),
        format!("[build]\ntarget-dir = '{}'\n", shared_target.display()),
    )
    .unwrap();

    let output = cargo_broom_command()
        .args(["--yes", "--coarse-only", "--keep-days", "0"])
        .arg(&ws)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(shared_target.exists());

    let _ = fs::remove_dir_all(ws);
}

#[test]
fn cli_fingerprint_parse_error_never_falls_back_to_coarse() {
    let ws = create_temp_workspace("cli_bad_fingerprint");
    fs::create_dir_all(ws.join("target/debug/.fingerprint/unsupported-layout")).unwrap();

    let output = cargo_broom_command()
        .args(["--yes", "--experimental-fine", "--keep-days", "999"])
        .arg(&ws)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(ws.join("target").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not be cleaned"));

    let _ = fs::remove_dir_all(ws);
}

#[test]
fn cli_registry_refuses_unconfirmed_cleanup() {
    let ws = create_temp_workspace("cli_registry_guard");
    let cargo_home = ws.join("cargo-home");
    let crate_file = cargo_home
        .join("registry/cache/index.example")
        .join("unused-1.0.0.crate");
    fs::create_dir_all(crate_file.parent().unwrap()).unwrap();
    fs::write(&crate_file, "crate archive").unwrap();

    let output = cargo_broom_command()
        .env("CARGO_HOME", &cargo_home)
        .args(["registry"])
        .arg(&ws)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(crate_file.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("without --dry-run or --yes"));

    let dry_run = cargo_broom_command()
        .env("CARGO_HOME", &cargo_home)
        .args(["registry"])
        .arg(&ws)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(dry_run.status.success());
    assert!(crate_file.exists());

    let confirmed = cargo_broom_command()
        .env("CARGO_HOME", &cargo_home)
        .args(["registry"])
        .arg(&ws)
        .arg("--yes")
        .output()
        .unwrap();
    assert!(
        confirmed.status.success(),
        "{}",
        String::from_utf8_lossy(&confirmed.stderr)
    );
    assert!(!crate_file.exists());

    let _ = fs::remove_dir_all(ws);
}

#[test]
fn cli_does_not_prune_fingerprints_without_experimental_opt_in() {
    let ws = create_temp_workspace("cli_no_experimental_fine");
    let fingerprint = ws.join("target/debug/.fingerprint/dummy-pkg-1234567890abcdef");

    let output = cargo_broom_command()
        .args(["--yes", "--keep-days", "999"])
        .arg(&ws)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fingerprint.exists());

    let _ = fs::remove_dir_all(ws);
}

#[test]
fn cli_rejects_invalid_config_and_conflicting_modes() {
    let ws = create_temp_workspace("cli_invalid_config");
    let config = ws.join("invalid.toml");
    fs::write(&config, "keep_dayz = 7\n").unwrap();

    let invalid_config = cargo_broom_command()
        .args(["--dry-run", "--config"])
        .arg(&config)
        .arg(&ws)
        .output()
        .unwrap();
    assert!(!invalid_config.status.success());
    assert!(String::from_utf8_lossy(&invalid_config.stderr).contains("Failed to parse"));

    let conflicting = cargo_broom_command()
        .args(["--dry-run", "--coarse-only", "--clean-doc"])
        .arg(&ws)
        .output()
        .unwrap();
    assert!(!conflicting.status.success());

    let _ = fs::remove_dir_all(ws);
}
