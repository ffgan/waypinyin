use std::process::Command;

#[test]
fn help_version_and_invalid_arguments_do_not_start_ibus() {
    let binary = env!("CARGO_BIN_EXE_waypinyin");
    for argument in ["--help", "-h", "--version", "-V"] {
        let result = Command::new(binary)
            .arg(argument)
            .env_remove("IBUS_ADDRESS")
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("waypinyin")
                || String::from_utf8_lossy(&result.stdout).contains("Waypinyin")
        );
    }
    for args in [vec!["--unknown"], vec!["--settings", "--version"]] {
        let result = Command::new(binary).args(args).output().unwrap();
        assert_eq!(result.status.code(), Some(2));
    }
}

#[test]
fn explicit_missing_dictionary_is_an_error() {
    let result = Command::new(env!("CARGO_BIN_EXE_waypinyin"))
        .arg("--check-data")
        .env(
            "WAYPINYIN_LEXICON",
            "/nonexistent-waypinyin-test/lexicon.bin",
        )
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("/nonexistent-waypinyin-test/lexicon.bin")
    );
}

#[test]
fn invalid_optional_model_falls_back_without_losing_dictionary() {
    let path =
        std::env::temp_dir().join(format!("waypinyin-invalid-model-{}.lm", std::process::id()));
    std::fs::write(&path, b"WAYPINYIN-LM\t999\t0\t0\ncorrupt").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_waypinyin"))
        .arg("--check-data")
        .env("WAYPINYIN_LEXICON", env!("WAYPINYIN_LEXICON"))
        .env("WAYPINYIN_MODEL", &path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("dictionary fallback"));
    assert!(String::from_utf8_lossy(&result.stderr).contains("mismatch"));
}

#[test]
fn valid_lexicon_can_be_checked_without_ibus_or_a_display() {
    let result = Command::new(env!("CARGO_BIN_EXE_waypinyin"))
        .arg("--check-data")
        .env("WAYPINYIN_LEXICON", env!("WAYPINYIN_LEXICON"))
        .env_remove("IBUS_ADDRESS")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains(env!("WAYPINYIN_LEXICON_KIND")));
}
