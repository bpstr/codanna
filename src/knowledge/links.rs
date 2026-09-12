//! Conservative explicit references. Unsupported Markdown is not guessed.
use super::*;
use regex::Regex;
use std::collections::BTreeMap;

fn span(input: &Input, path: &str, start: u32, end: u32) -> Span {
    Span { repo: input.repo.clone(), path: path.into(), start_line: start, end_line: end,
        hash: digest(input.files[path].as_bytes()) }
}

fn add_node(graph: &mut Graph, input: &Input, kind: Kind, path: &str, key: &str, label: &str, start: u32, end: u32, text: &str) -> String {
    let id = identity(&input.repo, &format!("{kind:?}"), path, key);
    graph.nodes.insert(id.clone(), Node { id: id.clone(), kind, label: label.into(),
        source: span(input, path, start, end), excerpt: excerpt(text) });
    id
}

fn connect(graph: &mut Graph, from: &str, to: &str, relation: &str, evidence: Span, method: &str) {
    graph.edges.push(Edge { from: from.into(), to: to.into(), relation: relation.into(),
        basis: Basis::Explicit, evidence, method: method.into() });
}

pub fn slug(text: &str) -> String {
    let mut result = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' { result.push(c); }
        else if c.is_whitespace() { result.push('-'); }
    }
    result
}

pub fn is_document(path: &str) -> bool {
    matches!(path.rsplit('.').next(), Some("md" | "markdown" | "txt"))
}

fn test_path(path: &str) -> bool {
    path.split('/').any(|p| matches!(p, "test" | "tests" | "__tests__"))
        || path.contains(".test.") || path.contains(".spec.") || path.ends_with("_test.go")
        || path.rsplit('/').next().is_some_and(|p| p.starts_with("test_"))
}

/// Resolve local URL paths lexically; the file must additionally occur in Input.
fn local_target(path: &str, target: &str) -> Option<(String, Option<String>)> {
    if target.contains("://") || target.starts_with("mailto:") { return None; }
    let (file, fragment) = target.split_once('#').map_or((target, None), |(p, f)| (p, Some(f.to_owned())));
    if file.starts_with('/') || file.contains(['\\', ':', '?', '\0']) { return None; }
    let mut parts: Vec<&str> = path.split('/').collect();
    if file.is_empty() { return Some((path.into(), fragment)); }
    parts.pop();
    for part in file.split('/') {
        match part { "" | "." => (), ".." => { parts.pop()?; }, p => parts.push(p) }
    }
    Some((parts.join("/"), fragment))
}

fn candidate(graph: &mut Graph, from: &str, reference: &str, candidates: Vec<String>, evidence: Span, missing_reason: &str) {
    graph.unresolved.push(Unresolved { from: from.into(), reference: reference.into(),
        reason: if candidates.is_empty() { missing_reason.into() } else { "ambiguous_reference".into() },
        candidates, evidence });
}

/// Build a complete replacement snapshot. Never mutates a published generation.
pub fn build(input: &Input) -> Result<Graph> {
    validate_repo(&input.repo)?;
    let mut graph = Graph::default();
    let mut repo = Repository { revision: input.revision.clone(), ..Repository::default() };
    let mut bytes = 0usize;
    for (path, text) in &input.files {
        validate_path(path)?;
        bytes = bytes.checked_add(text.len()).ok_or("input size overflow")?;
        if text.len() > MAX_FILE_BYTES || bytes > MAX_GRAPH_BYTES as usize || input.files.len() > 20_000 {
            return Err("knowledge input exceeds configured safety limits".into());
        }
        repo.files.insert(path.clone(), digest(text.as_bytes()));
    }
    graph.repositories.insert(input.repo.clone(), repo);
    let mut files = BTreeMap::new();
    let mut anchors = BTreeMap::new();
    let mut owners: BTreeMap<String, Vec<(u32, u32, String)>> = BTreeMap::new();
    let mut names: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut codes = BTreeMap::new();
    let mut duplicate_keys: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut symbols: Vec<_> = input.symbols.iter().collect();
    symbols.sort_by_key(|s| (&s.path, s.start_line, s.end_line, s.key));
    for (path, text) in &input.files {
        let lines = text.lines().count().max(1) as u32;
        let kind = if is_document(path) { Kind::Document } else { Kind::File };
        let id = add_node(&mut graph, input, kind, path, "", path, 1, lines, "");
        files.insert(path.clone(), id);
    }
    for symbol in symbols {
        let text = input.files.get(&symbol.path).ok_or("symbol source missing from input")?;
        if symbol.start_line == 0 || symbol.end_line < symbol.start_line || symbol.end_line as usize > text.lines().count().max(1) {
            return Err(format!("invalid or stale symbol range: {}", symbol.qualified_name).into());
        }
        let key = format!("{}\0{}", symbol.qualified_name, symbol.signature);
        let ordinal = duplicate_keys.entry((symbol.path.clone(), key.clone())).or_default();
        let key = format!("{key}\0{ordinal}");
        *ordinal += 1;
        let body = text.lines().skip(symbol.start_line as usize - 1).take((symbol.end_line - symbol.start_line + 1) as usize).collect::<Vec<_>>().join("\n");
        let kind = if test_path(&symbol.path) { Kind::Test } else { Kind::Symbol };
        let id = add_node(&mut graph, input, kind, &symbol.path, &key, &symbol.qualified_name, symbol.start_line, symbol.end_line, &body);
        if codes.insert(symbol.key, id.clone()).is_some() { return Err("duplicate code symbol id".into()); }
        names.entry(symbol.name.clone()).or_default().push(id.clone());
        if symbol.qualified_name != symbol.name { names.entry(symbol.qualified_name.clone()).or_default().push(id.clone()); }
        owners.entry(symbol.path.clone()).or_default().push((symbol.start_line, symbol.end_line, id.clone()));
        connect(&mut graph, &files[&symbol.path], &id, "defines", span(input, &symbol.path, symbol.start_line, symbol.end_line), "codanna_symbol");
    }
    // Preserve relationships already resolved by Codanna; do not upgrade their certainty.
    for edge in &input.edges {
        let from = codes.get(&edge.from).ok_or("code edge has missing source")?;
        let to = codes.get(&edge.to).ok_or("code edge has missing target")?;
        graph.edges.push(Edge { from: from.clone(), to: to.clone(), relation: edge.relation.to_lowercase(),
            basis: Basis::Resolved, evidence: graph.nodes[from].source.clone(), method: "codanna_index".into() });
    }
    // Collect section anchors first so forward document links resolve.
    for (path, text) in input.files.iter().filter(|(p, _)| is_document(p)) {
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        let mut headings = Vec::new();
        let mut fence = None;
        for (i, line) in text.lines().enumerate() {
            if fenced(line, &mut fence) { continue; }
            let trimmed = line.trim_start();
            let count = trimmed.bytes().take_while(|&b| b == b'#').count();
            if (1..=6).contains(&count) && trimmed.as_bytes().get(count) == Some(&b' ') {
                headings.push((i as u32 + 1, trimmed[count..].trim().trim_end_matches('#').trim().to_owned()));
            }
        }
        for (i, (start, label)) in headings.iter().enumerate() {
            let end = headings.get(i + 1).map_or(text.lines().count().max(1) as u32, |(n, _)| n - 1);
            let raw = slug(label);
            let n = seen.entry(raw.clone()).or_default();
            let anchor = if *n == 0 { raw } else { format!("{raw}-{n}") };
            *n += 1;
            let body = text.lines().skip(*start as usize - 1).take((end - start + 1) as usize).collect::<Vec<_>>().join("\n");
            let id = add_node(&mut graph, input, Kind::Section, path, &anchor, label, *start, end, &body);
            anchors.insert((path.clone(), anchor), id.clone());
            owners.entry(path.clone()).or_default().push((*start, end, id.clone()));
            connect(&mut graph, &files[path], &id, "contains", span(input, path, *start, *start), "markdown_heading");
        }
    }
    let links = Regex::new(r"\[[^\]\n]*\]\(([^\s)]+)\)")?;
    let ticks = Regex::new(r"`([^`\n]+)`")?;
    let adr = Regex::new(r"(?i)\b(?:ADR|RFC)[-_ ]?(\d+)\b")?;
    let mut decisions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (path, id) in &files {
        if !is_document(path) { continue; }
        for cap in adr.captures_iter(path.rsplit('/').next().unwrap_or(path)) {
            let full = cap[0].to_uppercase().replace(['-', '_', ' '], "");
            decisions.entry(full).or_default().push(id.clone());
        }
    }
    for (path, text) in &input.files {
        let doc = is_document(path);
        let mut fence = None;
        for (i, raw) in text.lines().enumerate() {
            if doc && fenced(raw, &mut fence) { continue; }
            let line = i as u32 + 1;
            let comment = raw.trim().strip_prefix("//").or_else(|| raw.trim().strip_prefix('#')).or_else(|| raw.trim().strip_prefix("--")).or_else(|| raw.trim().strip_prefix("* "));
            if !doc && comment.is_none() { continue; }
            let source = if doc { raw } else { comment.unwrap_or("") };
            let evidence = span(input, path, line, line);
            let owner = owners.get(path).and_then(|entries| entries.iter().filter(|(a, b, _)| *a <= line && line <= *b).min_by_key(|(a, b, _)| b - a)).map(|(_, _, id)| id.clone()).unwrap_or_else(|| files[path].clone());
            let owner = if !doc && (source.trim().starts_with("WHY:") || source.trim().starts_with("NOTE:")) {
                let id = add_node(&mut graph, input, Kind::Rationale, path, &format!("{line}"), source.trim(), line, line, source);
                connect(&mut graph, &id, &owner, "explains", evidence.clone(), "rationale_comment");
                id
            } else { owner };
            for cap in links.captures_iter(source) {
                let target = &cap[1];
                if target.contains("://") || target.starts_with("mailto:") { continue; }
                let resolved = local_target(path, target).and_then(|(p, f)| {
                    if let Some(fragment) = f {
                        if fragment.starts_with('L') && fragment[1..].chars().all(|c| c.is_ascii_digit() || c == '-' || c == 'L') {
                            let nums: Vec<_> = fragment.split('-').filter_map(|s| s.trim_start_matches('L').parse::<u32>().ok()).collect();
                            let lines = input.files.get(&p)?.lines().count().max(1) as u32;
                            if nums.is_empty() || nums.iter().any(|&n| n == 0 || n > lines) || nums.first() > nums.last() { return None; }
                            files.get(&p).cloned()
                        } else { anchors.get(&(p, fragment)).cloned() }
                    } else { files.get(&p).cloned() }
                });
                if let Some(to) = resolved { connect(&mut graph, &owner, &to, "references", evidence.clone(), "markdown_link"); }
                else { candidate(&mut graph, &owner, target, vec![], evidence.clone(), "unresolved_local_link"); }
            }
            for cap in ticks.captures_iter(source) {
                let reference = &cap[1];
                if let Some(found) = names.get(reference) {
                    if let [to] = found.as_slice() { connect(&mut graph, &owner, to, "references", evidence.clone(), "exact_symbol_mention"); }
                    else { candidate(&mut graph, &owner, reference, found.clone(), evidence.clone(), "unknown_symbol"); }
                }
                // Unknown inline code may be a literal, not a broken symbol reference.
            }
            for cap in adr.captures_iter(source) {
                let reference = cap[0].to_uppercase().replace(['-', '_', ' '], "");
                let found = decisions.get(&reference).cloned().unwrap_or_default();
                if let [to] = found.as_slice() { if to != &owner { connect(&mut graph, &owner, to, "references", evidence.clone(), "decision_reference"); } }
                else { candidate(&mut graph, &owner, &cap[0], found, evidence.clone(), "unresolved_decision"); }
            }
        }
    }
    graph.limitations.push("Code relationships inherit Codanna's static resolution; dump-to-source freshness is not independently proven.".into());
    graph.limitations.push("Markdown support: ATX headings, inline local links and exact inline-code symbol mentions; fenced blocks are excluded. Unknown inline literals are not broken references.".into());
    graph.limitations.push("Test nodes identify test source, not executed coverage or proof of correctness.".into());
    graph.normalize();
    graph.validate()?;
    Ok(graph)
}

fn fenced(line: &str, fence: &mut Option<(char, usize)>) -> bool {
    let trimmed = line.trim_start();
    let first = trimmed.chars().next().unwrap_or(' ');
    let count = trimmed.chars().take_while(|c| *c == first).count();
    if let Some((marker, size)) = *fence {
        if first == marker && count >= size && trimmed[count..].trim().is_empty() { *fence = None; }
        return true;
    }
    if matches!(first, '`' | '~') && count >= 3 { *fence = Some((first, count)); return true; }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn fixture() -> Input {
        Input { repo: "core".into(), files: BTreeMap::from([
            ("src/upload.rs".into(), "fn upload() {\n// WHY: enforce ADR-001\n}\n".into()),
            ("docs/ADR-001.md".into(), "# Upload limits\nUse `upload`. [source](../src/upload.rs#L1-L3)\n".into()),
        ]), symbols: vec![CodeSymbol { key: 1, name: "upload".into(), qualified_name: "upload".into(), signature: "fn upload()".into(), path: "src/upload.rs".into(), start_line: 1, end_line: 3 }], ..Input::default() }
    }
    #[test] fn links_rationale_and_reverse_edges() {
        let graph = build(&fixture()).unwrap();
        let symbol = graph.resolve("upload").unwrap();
        assert!(graph.edges.iter().any(|e| e.to == symbol.id && e.method == "exact_symbol_mention"));
        assert!(graph.edges.iter().any(|e| e.relation == "explains"));
        assert!(graph.edges.iter().any(|e| e.method == "decision_reference"));
        graph.validate().unwrap();
    }
    #[test] fn ambiguity_is_not_an_edge() {
        let mut input = fixture();
        let mut second = input.symbols[0].clone(); second.key = 2; second.qualified_name = "other::upload".into(); input.symbols.push(second);
        let graph = build(&input).unwrap();
        assert!(graph.unresolved.iter().any(|r| r.reference == "upload" && r.candidates.len() == 2));
        assert!(!graph.edges.iter().any(|e| e.method == "exact_symbol_mention"));
    }
    #[test] fn fences_do_not_create_fake_headings_or_links() {
        let mut input = fixture();
        input.files.insert("docs/fences.md".into(), "```rust\n# Fake\n`upload` [bad](missing.md)\n```\n# Real\n`upload`\n".into());
        let graph = build(&input).unwrap();
        assert!(graph.resolve("Fake").is_err()); assert!(graph.resolve("Real").is_ok());
        assert!(!graph.unresolved.iter().any(|r| r.reference == "missing.md"));
    }
    #[test] fn replacement_drops_deleted_links_and_keeps_identity_after_line_shift() {
        let input = fixture(); let old = build(&input).unwrap();
        let mut changed = input; changed.files.remove("docs/ADR-001.md");
        changed.files.insert("src/upload.rs".into(), "\nfn upload() {\n// WHY: enforce ADR-001\n}\n".into());
        changed.symbols[0].start_line += 1; changed.symbols[0].end_line += 1;
        let new = build(&changed).unwrap();
        assert_eq!(old.resolve("upload").unwrap().id, new.resolve("upload").unwrap().id);
        assert!(new.unresolved.iter().any(|r| r.reason == "unresolved_decision"));
        assert!(!new.edges.iter().any(|e| e.method == "exact_symbol_mention"));
    }
    #[test] fn path_escape_is_rejected() {
        assert!(local_target("docs/a.md", "../../secret").is_none());
        assert!(validate_path("../secret").is_err()); assert!(validate_repo("a:b").is_err());
    }
    #[test] fn stable_output_and_roundtrip() {
        let a = build(&fixture()).unwrap(); let b = build(&fixture()).unwrap();
        assert_eq!(serde_json::to_vec(&a).unwrap(), serde_json::to_vec(&b).unwrap());
        let decoded: Graph = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap(); decoded.validate().unwrap();
    }
    #[test] fn corrupt_schema_and_dangling_edges_fail() {
        let mut graph = build(&fixture()).unwrap(); graph.schema_version = 999; assert!(graph.validate().is_err());
        graph.schema_version = 1; graph.edges[0].to = "missing".into(); assert!(graph.validate().is_err());
    }
    #[test] fn duplicate_headings_are_addressable() {
        let mut input = fixture(); input.files.insert("docs/a.md".into(), "# Same\n# Same\n[second](#same-1)\n".into());
        let graph = build(&input).unwrap(); assert!(!graph.unresolved.iter().any(|r| r.reference == "#same-1"));
    }
}
