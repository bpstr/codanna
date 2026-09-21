use std::{fs, path::Path, process::Command};

fn run(root: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_codanna"))
        .current_dir(root)
        .args(args)
        .output()
        .expect("codanna command must start");
    assert!(
        output.status.success(),
        "codanna {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("codanna stdout must be UTF-8")
}

fn assert_alias_edge(root: &Path, extension: &str) {
    let edges = run(root, &["dump", "--edges", "--relation", "calls"]);
    let symbols = run(root, &["dump", "--symbols"]);
    let targets: Vec<_> = edges
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|row| {
            row["type"] == "result" && row["data"]["from"]["name"].as_str() == Some("entry")
        })
        .map(|row| {
            (
                row["data"]["to"]["name"].as_str().unwrap().to_owned(),
                row["data"]["to"]["file_path"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        targets.len(),
        1,
        "aliased call must emit one edge:\n{edges}\nsymbols:\n{symbols}"
    );
    assert_eq!(targets[0].0, "sharedTarget");
    assert!(
        targets[0].1.ends_with(&format!("target.{extension}")),
        "aliased {extension} call must resolve to the target module: {edges}"
    );
}

#[test]
fn named_import_alias_retains_exported_target_name() {
    for extension in ["ts", "js"] {
        let dir = tempfile::tempdir().expect("temporary workspace");
        let root = dir.path().canonicalize().expect("canonical workspace");
        fs::create_dir(root.join(".codanna")).expect("settings directory");
        fs::write(
            root.join(".codanna/settings.toml"),
            format!(
                "workspace_root = {}\nindex_path = {}\n[indexing]\nindexed_paths = [{}]\n[semantic_search]\nenabled = false\n",
                serde_json::to_string(&root).expect("serialize workspace path"),
                serde_json::to_string(&root.join(".codanna/index"))
                    .expect("serialize index path"),
                serde_json::to_string(&root.join("code")).expect("serialize source path")
            ),
        )
        .expect("write settings");
        fs::create_dir(root.join("code")).expect("source directory");
        fs::write(
            root.join("code").join(format!("target.{extension}")),
            "export function sharedTarget() { return 42; }\n",
        )
        .expect("write target");
        fs::write(
            root.join("code").join(format!("caller.{extension}")),
            "import { sharedTarget as renamed } from './target';\n\
             export function entry() { return renamed(); }\n",
        )
        .expect("write caller");

        run(&root, &["index", "--force", "--no-progress"]);
        assert_alias_edge(&root, extension);
    }
}
