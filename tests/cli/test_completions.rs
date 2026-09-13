use std::process::Command;

#[test]
fn completion_scripts_are_generated_without_project_setup() {
    let workspace = tempfile::tempdir().expect("temporary workspace");

    for shell in ["bash", "elvish", "fish", "powershell", "zsh"] {
        let output = Command::new(env!("CARGO_BIN_EXE_codanna"))
            .args(["completions", shell])
            .current_dir(workspace.path())
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("GOOGLE_API_KEY")
            // Completion output remains a thin path even when the caller's
            // normal CLI environment requests a provider.
            .env("CODANNA_EMBED_PROVIDER", "invalid-fixture-provider")
            .output()
            .expect("run completion generator");

        assert!(output.status.success(), "{shell} generation failed");
        let stdout = String::from_utf8(output.stdout).expect("UTF-8 completion script");
        let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
        assert!(
            stdout.contains("codanna"),
            "{shell} script names the command"
        );
        assert!(stdout.len() > 100, "{shell} script is not empty");
        assert!(
            stderr.is_empty(),
            "completion generation touched project setup: {stderr}"
        );
    }
}

#[test]
fn bash_completions_include_current_top_level_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_codanna"))
        .args(["completions", "bash"])
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("GOOGLE_API_KEY")
        .env_remove("CODANNA_EMBED_PROVIDER")
        .output()
        .expect("run bash completion generator");

    assert!(output.status.success());
    let script = String::from_utf8(output.stdout).expect("UTF-8 completion script");
    for command in ["retrieve", "serve", "documents", "profile", "completions"] {
        assert!(script.contains(command), "bash script is missing {command}");
    }
}
