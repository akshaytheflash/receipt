use receipt::model::Session;
use std::fs;
use std::process::Command;

#[test]
fn fixture_roundtrip() {
    let root = std::env::temp_dir().join(format!("receipt-it-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create fixture root");

    let fixture = env!("CARGO_BIN_EXE_fixture_installer");
    let bin = env!("CARGO_BIN_EXE_receipt");

    let out = Command::new(bin)
        .args(["run", "--json", "--timeout", "90", "--"])
        .arg(fixture)
        .arg(&root)
        .output()
        .expect("run receipt");

    assert!(
        out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let s: Session = serde_json::from_slice(&out.stdout).expect("manifest json");

    assert!(
        s.manifest
            .files
            .created
            .iter()
            .any(|p| p.ends_with("alpha.txt")),
        "alpha.txt not recorded: {:?}",
        s.manifest.files
    );
    assert!(
        s.manifest
            .registry
            .created_keys
            .iter()
            .any(|k| k.contains("ReceiptFixture")),
        "registry key not recorded: {:?}",
        s.manifest.registry
    );
    assert!(!s.manifest.network.is_empty(), "no network events recorded");
    assert!(
        s.manifest
            .processes
            .iter()
            .any(|p| p.name.to_lowercase().contains("fixture")),
        "fixture process not recorded"
    );

    let out = Command::new(bin)
        .args(["revert", &s.id, "--yes"])
        .output()
        .expect("revert");
    assert!(
        out.status.success(),
        "revert failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        !root.join("alpha.txt").exists(),
        "alpha.txt survived revert"
    );

    let _ = fs::remove_dir_all(&root);
}
