use std::process::Command;

#[test]
fn embedding_info_is_configuration_and_model_free() {
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let output = Command::new(env!("CARGO_BIN_EXE_codanna"))
        .args([
            "--config",
            "/missing/fixture/settings.toml",
            "embedding-info",
        ])
        .current_dir(workspace.path())
        .env_clear()
        .env("CODANNA_EMBED_PROVIDER", "invalid-fixture-provider")
        .env("CODANNA_EMBED_PROVIDER_STRICT", "1")
        .output()
        .expect("run capability diagnostic");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 output");
    assert!(stdout.contains("Compiled embedding providers: cpu"));
    assert!(stdout.contains("does not verify"));
    assert!(!workspace.path().join(".codanna").exists());
}

#[test]
fn strict_invalid_provider_stops_before_configuration_loading() {
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let output = Command::new(env!("CARGO_BIN_EXE_codanna"))
        .args(["--config", "/missing/fixture/settings.toml", "config"])
        .current_dir(workspace.path())
        .env_clear()
        .env("CODANNA_EMBED_PROVIDER", "invalid-fixture-provider")
        .env("CODANNA_EMBED_PROVIDER_STRICT", "1")
        .output()
        .expect("run strict selection fixture");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("unsupported embedding execution provider"));
    assert!(!stderr.contains("using CPU"));
    assert!(output.stdout.is_empty());
}
