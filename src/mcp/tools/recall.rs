//! Directory-scoped, model-free recall. Import remains explicit; selecting a
//! workspace never authorizes discovery of the user's private transcript tree.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_OUTPUT: usize = 64 * 1024;

/// Resolve the same local project boundary as the workspace MCP, without writes.
/// Used by the recall CLI when no legacy namespace was explicitly selected.
pub fn scope_for_directory(directory: &Path) -> Result<String, crate::IndexError> {
    let root = crate::cli::automatic::resolve_session_root(directory, dirs::home_dir().as_deref())?;
    Ok(scope_for_root(&root))
}

/// The caller supplies a validated canonical workspace root. Basenames, aliases,
/// Git remotes, and lossy path strings are never used as privacy identities.
/// Moving a checkout deliberately creates a new namespace; old history is not
/// silently relabelled. Canonical symlink aliases share one namespace.
pub fn scope_for_root(root: &Path) -> String {
    let mut hash = Sha256::new();
    hash.update(b"codanna-recall-project-v1\0");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hash.update(b"unix\0");
        hash.update(root.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        hash.update(b"windows\0");
        for unit in root.as_os_str().encode_wide() {
            hash.update(unit.to_le_bytes());
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        hash.update(b"native\0");
        hash.update(root.as_os_str().as_encoded_bytes());
    }
    format!("project-v1:{}", hex::encode(hash.finalize()))
}

pub(super) async fn conversation_context(query: &str, limit: usize, root: Option<&Path>) -> String {
    // Automatic/selected/router launches strip inherited workspace overrides.
    // Explicit legacy --config launches may still opt into a named namespace.
    let workspace = match std::env::var("CODANNA_RECALL_WORKSPACE") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => match root {
            Some(root) => scope_for_root(root),
            None => return "Conversation recall unavailable: no workspace context.\n".into(),
        },
    };
    let binary = recall_binary();
    let mut command = Command::new(binary);
    if let Some(index) = std::env::var_os("CODANNA_RECALL_INDEX").filter(|value| !value.is_empty())
    {
        command.arg("--index").arg(index);
    }
    command
        .arg("--workspace")
        .arg(&workspace)
        .arg("search")
        .arg(query)
        .arg("--limit")
        .arg(limit.to_string());
    match capture(command).await {
        Ok((status, bytes, detail)) if !status.success() => {
            let detail = std::str::from_utf8(&detail).unwrap_or("recall command failed");
            let detail = detail.lines().next().unwrap_or("recall command failed");
            let _ = bytes;
            format!("Conversation recall unavailable: {detail}\n")
        }
        Ok((_, bytes, _)) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => render(&value, &workspace, limit),
            Err(_) => "Conversation recall returned invalid JSON.\n".into(),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            "Conversation recall is not installed. No transcript files were read.\n".into()
        }
        Err(error) if error.kind() == io::ErrorKind::TimedOut => {
            "Conversation recall timed out after 3 seconds.\n".into()
        }
        Err(_) => "Conversation recall unavailable; the local reader failed or exceeded its output budget.\n".into(),
    }
}

async fn bounded_read(reader: impl AsyncRead + Unpin, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > limit {
        return Err(io::Error::other("recall output budget exceeded"));
    }
    Ok(bytes)
}

async fn capture(mut command: Command) -> io::Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing recall stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing recall stderr"))?;
    let result = tokio::time::timeout(TIMEOUT, async {
        let (stdout, stderr, status) = tokio::try_join!(
            bounded_read(stdout, MAX_OUTPUT),
            bounded_read(stderr, 4096),
            child.wait(),
        )?;
        Ok((status, stdout, stderr))
    })
    .await
    .unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "recall deadline")));
    if result.is_err() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    result
}

fn render(value: &Value, workspace: &str, limit: usize) -> String {
    if value.get("workspace").and_then(Value::as_str) != Some(workspace) {
        return "Conversation recall refused a mismatched workspace response.\n".into();
    }
    let Some(results) = value.get("results").and_then(Value::as_array) else {
        return "Conversation recall returned no result list.\n".into();
    };
    if results.len() > limit {
        return "Conversation recall exceeded the requested result limit.\n".into();
    }
    if results.is_empty() {
        return "No matching conversation messages.\n".into();
    }
    let mut output = String::new();
    for (i, hit) in results.iter().enumerate() {
        let Some(message) = hit.get("message") else {
            continue;
        };
        let text = |key| message.get(key).and_then(Value::as_str).unwrap_or("");
        let timestamp = text("timestamp");
        let line = message.get("line").and_then(Value::as_u64).unwrap_or(0);
        output.push_str(&format!(
            "{}. {}/{} {} — {}:{line} [recall_id:{}]\n   {}\n",
            i + 1,
            text("provider"),
            text("role"),
            timestamp,
            text("source_path"),
            text("id"),
            text("text")
                .chars()
                .take(800)
                .collect::<String>()
                .replace('\n', " ")
        ));
    }
    output
}

fn recall_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("CODANNA_RECALL_BIN").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    let name = if cfg!(windows) {
        "codanna-recall.exe"
    } else {
        "codanna-recall"
    };
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hardening_workspace_recall_scopes_distinguish_equal_basenames() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("one/project");
        let b = temp.path().join("two/project");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert_ne!(
            scope_for_directory(&a).unwrap(),
            scope_for_directory(&b).unwrap()
        );
        assert_eq!(
            scope_for_directory(&a).unwrap(),
            scope_for_directory(&a.join(".")).unwrap()
        );
        assert!(!a.join(".codanna").exists());
    }
    #[test]
    fn hardening_workspace_recall_rejects_foreign_reply_before_rendering_text() {
        let response = serde_json::json!({"workspace":"other","results":[{"message":{"text":"PRIVATE_SENTINEL"}}]});
        let output = render(&response, "current", 8);
        assert!(output.contains("mismatched workspace"));
        assert!(!output.contains("PRIVATE_SENTINEL"));
    }
    #[tokio::test]
    async fn hardening_workspace_recall_output_is_bounded() {
        assert!(bounded_read(&b"12345"[..], 4).await.is_err());
        assert_eq!(bounded_read(&b"1234"[..], 4).await.unwrap(), b"1234");
    }
    #[cfg(unix)]
    #[test]
    fn hardening_workspace_recall_does_not_collapse_non_utf8_paths() {
        use std::os::unix::ffi::OsStringExt;
        let a = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/project-\xff".to_vec()));
        let b = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/project-\xfe".to_vec()));
        assert_eq!(a.to_string_lossy(), b.to_string_lossy());
        assert_ne!(scope_for_root(&a), scope_for_root(&b));
    }
}
