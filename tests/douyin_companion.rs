#[cfg(target_os = "macos")]
#[test]
fn replays_recorded_companion_rows_offline() {
    let scratch = tempfile::tempdir().unwrap();
    let check = scratch.path().join("check-recorded-rows");
    let compiled = std::process::Command::new("xcrun")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "swiftc",
            "native/douyin-companion/Messages.swift",
            "tests/douyin-companion/main.swift",
            "-o",
        ])
        .arg(&check)
        .status()
        .unwrap();
    assert!(compiled.success());
    assert!(
        std::process::Command::new(check)
            .status()
            .unwrap()
            .success()
    );
}
