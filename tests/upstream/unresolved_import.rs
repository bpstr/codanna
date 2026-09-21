use std::{collections::BTreeSet, fs, path::Path, process::Command};

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

fn caller_targets(edges: &str) -> BTreeSet<(String, String)> {
    edges
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|row| row["type"] == "result")
        .filter_map(|row| {
            let caller = row["data"]["from"]["name"].as_str()?;
            if !matches!(caller, "entry" | "arrowEntry") {
                return None;
            }
            Some((
                caller.to_owned(),
                row["data"]["to"]["file_path"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

#[test]
fn deleted_relative_import_never_captures_another_roots_export() {
    for extension in ["ts", "js"] {
        let dir = tempfile::tempdir().expect("temporary workspace");
        let root = dir.path().canonicalize().expect("canonical workspace");
        fs::create_dir(root.join(".codanna")).expect("settings directory");
        fs::write(
            root.join(".codanna/settings.toml"),
            format!(
                "workspace_root = {}\n[indexing]\nindexed_paths = [\"repo_a\", \"repo_b\"]\n[semantic_search]\nenabled = false\n",
                serde_json::to_string(&root).expect("serialize workspace path")
            ),
        )
        .expect("write settings");

        for repo in ["repo_a", "repo_b"] {
            fs::create_dir(root.join(repo)).expect("repository directory");
            fs::write(
                root.join(repo).join(format!("target.{extension}")),
                "export function sharedTarget() { return 42; }\n",
            )
            .expect("write target");
        }
        fs::write(
            root.join("repo_a").join(format!("caller.{extension}")),
            "import { sharedTarget } from './target';\n\
             export function entry() { return sharedTarget(); }\n\
             export const arrowEntry = () => sharedTarget();\n",
        )
        .expect("write caller");

        run(&root, &["index", "--no-progress"]);
        let initial = caller_targets(&run(&root, &["dump", "--edges", "--relation", "calls"]));
        assert_eq!(initial.len(), 2, "both {extension} callers must resolve");
        assert!(
            initial
                .iter()
                .all(|(_, target)| target.contains("repo_a/target")),
            "valid relative imports must remain in repo_a: {initial:?}"
        );

        fs::remove_file(root.join("repo_a").join(format!("target.{extension}")))
            .expect("delete imported target");
        for args in [
            ["index", "--no-progress"].as_slice(),
            ["index", "--force", "--no-progress"].as_slice(),
        ] {
            run(&root, args);
            let after_delete =
                caller_targets(&run(&root, &["dump", "--edges", "--relation", "calls"]));
            assert!(
                after_delete.is_empty(),
                "unresolved {extension} import captured another root after {args:?}: {after_delete:?}"
            );
        }
    }
}
