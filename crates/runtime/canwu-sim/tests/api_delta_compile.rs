use std::path::PathBuf;
use std::process::{Command, Output};

fn compile_fixture(extra_args: &[&str]) -> Output {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("api-delta")
        .join("Cargo.toml");
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("target")
        .join("api-delta-fixture");
    let mut command = Command::new(env!("CARGO"));
    command
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--target-dir")
        .arg(target)
        .env("CARGO_TERM_COLOR", "never");
    command.args(extra_args);
    command.output().expect("external cargo check should run")
}

#[test]
fn format_7_construction_compiles() {
    let current = compile_fixture(&[]);
    assert!(
        current.status.success(),
        "format-7 external fixture failed:\n{}",
        String::from_utf8_lossy(&current.stderr)
    );
}

#[test]
fn restricted_viewer_has_no_raw_admin_snapshot_surface() {
    for (feature, method) in [
        ("viewer-admin-leak", "snapshot"),
        ("viewer-journal-leak", "replay_journal"),
        ("viewer-domain-leak", "domain_records"),
        ("viewer-boundary-leak", "boundaries"),
    ] {
        let output = compile_fixture(&["--features", feature]);
        assert!(
            !output.status.success(),
            "restricted viewer unexpectedly exposed raw method {method}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(method),
            "unexpected compiler output for {method}: {stderr}"
        );
        assert!(
            stderr.contains("CanwuViewer"),
            "the compiler should identify the restricted viewer type: {stderr}"
        );
    }
}
