//! Bounded, local-only snapshot and Codanna dump I/O.
use super::*;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() { return Err("expected a regular file".into()); }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit { return Err(format!("file exceeds byte limit: {}", path.display()).into()); }
    Ok(bytes)
}

pub fn load(path: &Path) -> Result<Graph> {
    let graph: Graph = serde_json::from_slice(&read_bounded(path, MAX_GRAPH_BYTES)?)?;
    graph.validate()?;
    Ok(graph)
}

/// Unique temporary inode, sync before rename, no modifications after publication.
pub fn save(graph: &Graph, path: &Path) -> Result<()> {
    graph.validate()?;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut temporary, graph)?;
    temporary.flush()?;
    if temporary.as_file().metadata()?.len() > MAX_GRAPH_BYTES { return Err("graph exceeds byte limit".into()); }
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

/// Canonical containment check also rejects symlinks escaping a registered root.
pub fn source_path(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_path(relative)?;
    let root = root.canonicalize()?;
    let path = root.join(relative).canonicalize()?;
    if !path.starts_with(&root) { return Err("source resolves outside repository root".into()); }
    Ok(path)
}

pub fn revision(root: &Path) -> Option<String> {
    let output = Command::new("git").args(["rev-parse", "HEAD"]).current_dir(root).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Use a fresh `codanna dump` or a supplied complete JSONL stream. Never execute repository scripts.
pub fn input(root: &Path, repo: &str, dump: Option<&Path>) -> Result<Input> {
    validate_repo(repo)?;
    let root = root.canonicalize()?;
    let generated;
    let dump = match dump {
        Some(path) => path.to_path_buf(),
        None => {
            generated = tempfile::NamedTempFile::new()?;
            let status = Command::new("codanna").arg("dump").current_dir(&root)
                .stdin(Stdio::null()).stdout(Stdio::from(generated.reopen()?)).status()?;
            if !status.success() { return Err("codanna dump failed; index code first or provide --dump".into()); }
            generated.path().to_path_buf()
        }
    };
    let bytes = read_bounded(&dump, MAX_GRAPH_BYTES)?;
    let mut input = parse_dump(BufReader::new(bytes.as_slice()), repo)?;
    input.revision = revision(&root);
    let mut paths: std::collections::BTreeSet<String> = input.symbols.iter().map(|s| s.path.clone()).collect();
    // Honor .gitignore and .codannaignore without following symlink directories.
    for entry in ignore::WalkBuilder::new(&root).hidden(false).add_custom_ignore_filename(".codannaignore").follow_links(false).build() {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_file()) { continue; }
        let relative = entry.path().strip_prefix(&root)?.to_str().ok_or("non UTF-8 source path")?.replace('\\', "/");
        if relative.split('/').any(|p| matches!(p, ".git" | ".codanna" | "node_modules" | "target")) { continue; }
        if links::is_document(&relative) { paths.insert(relative); }
        if paths.len() > 20_000 { return Err("too many source files".into()); }
    }
    let mut total = 0usize;
    for path in paths {
        let text = String::from_utf8(read_bounded(&source_path(&root, &path)?, MAX_FILE_BYTES as u64)?)?;
        total = total.checked_add(text.len()).ok_or("source size overflow")?;
        if total > MAX_GRAPH_BYTES as usize { return Err("source byte budget exceeded".into()); }
        input.files.insert(path, text);
    }
    Ok(input)
}

pub fn parse_dump(reader: impl BufRead, repo: &str) -> Result<Input> {
    let mut input = Input { repo: repo.into(), ..Input::default() };
    let mut began = false;
    let mut ended = false;
    let mut expected = None;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() { continue; }
        if ended { return Err("data after dump summary".into()); }
        let row: serde_json::Value = serde_json::from_str(&line)?;
        match row["type"].as_str() {
            Some("begin") if !began => { began = true; }
            Some("result") if began => {
                if row["status"] != "success" { return Err("unsuccessful dump row".into()); }
                let data = &row["data"];
                match row["meta"]["entity_type"].as_str() {
                    Some("symbol") => {
                        let name = string(data, "name")?;
                        let module = data["module_path"].as_str().unwrap_or("");
                        let path = string(data, "file_path")?; validate_path(&path)?;
                        let start = data["range"]["start_line"].as_u64().ok_or("missing start line")?;
                        let end = data["range"]["end_line"].as_u64().ok_or("missing end line")?;
                        input.symbols.push(CodeSymbol { key: data["id"].as_u64().ok_or("missing symbol id")?,
                            qualified_name: if module.is_empty() { name.clone() } else { format!("{module}::{name}") },
                            name, signature: data["signature"].as_str().unwrap_or("").into(), path,
                            start_line: u32::try_from(start)?.checked_add(1).ok_or("line overflow")?,
                            end_line: u32::try_from(end)?.checked_add(1).ok_or("line overflow")? });
                    }
                    Some("relationship") => input.edges.push(CodeEdge {
                        from: data["from"]["id"].as_u64().ok_or("missing edge source")?,
                        to: data["to"]["id"].as_u64().ok_or("missing edge target")?, relation: string(data, "relation")? }),
                    _ => return Err("unknown dump entity type".into()),
                }
            }
            Some("summary") if began => {
                if row["status"] != "success" { return Err("unsuccessful dump summary".into()); }
                expected = Some((row["data"]["symbols"].as_u64().ok_or("missing symbol count")?, row["data"]["relationships"].as_u64().ok_or("missing relationship count")?));
                ended = true;
            }
            _ => return Err("invalid or incomplete Codanna dump stream".into()),
        }
    }
    if expected != Some((input.symbols.len() as u64, input.edges.len() as u64)) { return Err("missing summary or dump count mismatch".into()); }
    Ok(input)
}

fn string(value: &serde_json::Value, key: &str) -> Result<String> {
    Ok(value[key].as_str().ok_or_else(|| format!("missing string {key}"))?.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn incomplete_dumps_fail() {
        assert!(parse_dump(BufReader::new(b"{\"type\":\"begin\"}\n".as_slice()), "r").is_err());
        let empty = b"{\"type\":\"begin\"}\n{\"type\":\"summary\",\"status\":\"success\",\"data\":{\"symbols\":0,\"relationships\":0}}\n";
        assert!(parse_dump(BufReader::new(empty.as_slice()), "r").is_ok());
    }
    #[test] fn replacement_roundtrip_and_size_limit() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("graph.json");
        save(&Graph::default(), &path).unwrap(); load(&path).unwrap();
        assert!(read_bounded(&path, 1).is_err());
        save(&Graph::default(), &path).unwrap(); assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[cfg(unix)]
    #[test] fn symlink_escape_is_rejected() {
        let root = tempfile::tempdir().unwrap(); let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), root.path().join("link")).unwrap();
        assert!(source_path(root.path(), "link").is_err());
    }
}
