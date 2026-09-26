//! Conservative explicit references. Unsupported Markdown is not guessed.
use super::*;
use pulldown_cmark::{Event, LinkType, Parser, Tag, TagEnd};
use regex::Regex;
use std::collections::BTreeMap;
use std::ops::Range;

fn span(input: &Input, path: &str, start: u32, end: u32) -> Span {
    Span {
        repo: input.repo.clone(),
        path: path.into(),
        start_line: start,
        end_line: end,
        hash: digest(input.files[path].as_bytes()),
    }
}

fn add_node(
    graph: &mut Graph,
    input: &Input,
    kind: Kind,
    path: &str,
    key: &str,
    label: &str,
    start: u32,
    end: u32,
    text: &str,
) -> String {
    let id = identity(&input.repo, &format!("{kind:?}"), path, key);
    graph.nodes.insert(
        id.clone(),
        Node {
            id: id.clone(),
            kind,
            label: label.into(),
            source: span(input, path, start, end),
            excerpt: excerpt(text),
        },
    );
    id
}

fn connect(graph: &mut Graph, from: &str, to: &str, relation: &str, evidence: Span, method: &str) {
    graph.edges.push(Edge {
        from: from.into(),
        to: to.into(),
        relation: relation.into(),
        basis: Basis::Explicit,
        evidence,
        method: method.into(),
    });
}

pub fn slug(text: &str) -> String {
    let mut result = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            result.push(c);
        } else if c.is_whitespace() {
            result.push('-');
        }
    }
    result
}

pub fn is_document(path: &str) -> bool {
    matches!(path.rsplit('.').next(), Some("md" | "markdown" | "txt"))
}

/// Configuration is source evidence, independent of code symbol extraction.
pub fn is_configuration(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some("toml" | "json" | "json5" | "yaml" | "yml" | "ini" | "cfg" | "conf" | "xml")
    )
}

#[derive(Debug)]
pub(crate) struct MarkdownLink {
    pub target: String,
    pub start_line: u32,
    pub end_line: u32,
    pub definition_lines: Option<(u32, u32)>,
}

fn comment_text(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix("//!")
        .or_else(|| trimmed.strip_prefix("///"))
        .or_else(|| trimmed.strip_prefix("//"))
        .or_else(|| trimmed.strip_prefix('#'))
        .or_else(|| trimmed.strip_prefix("--"))
        .or_else(|| trimmed.strip_prefix("* "))
}

/// Parse whole documents so reference definitions can precede or follow their uses.
/// Code comments are parsed in contiguous blocks, preserving original line numbers
/// without allowing a definition in an unrelated declaration to supply a target.
pub(crate) fn source_links(path: &str, text: &str) -> Vec<MarkdownLink> {
    if is_document(path) {
        markdown_links(text)
    } else {
        let lines: Vec<_> = text.lines().collect();
        let mut links = Vec::new();
        let mut cursor = 0;
        while cursor < lines.len() {
            if comment_text(lines[cursor]).is_none() {
                cursor += 1;
                continue;
            }
            let start = cursor;
            let mut comments = String::new();
            while let Some(comment) = lines.get(cursor).and_then(|raw| comment_text(raw)) {
                comments.push_str(comment.trim_start());
                comments.push('\n');
                cursor += 1;
            }
            for mut link in markdown_links(&comments) {
                link.start_line += start as u32;
                link.end_line += start as u32;
                if let Some((begin, end)) = &mut link.definition_lines {
                    *begin += start as u32;
                    *end += start as u32;
                }
                links.push(link);
            }
            cursor += 1;
        }
        links
    }
}

fn markdown_links(text: &str) -> Vec<MarkdownLink> {
    let mut line_starts = vec![0];
    line_starts.extend(text.match_indices('\n').map(|(offset, _)| offset + 1));
    let lines = |range: Range<usize>| {
        (
            line_starts.partition_point(|&offset| offset <= range.start) as u32,
            line_starts.partition_point(|&offset| offset <= range.end.saturating_sub(1)) as u32,
        )
    };
    let mut parser = Parser::new(text).into_offset_iter();
    let mut links = Vec::new();
    let mut image_depth = 0usize;
    while let Some((event, range)) = parser.next() {
        match &event {
            Event::Start(Tag::Image { .. }) => image_depth += 1,
            Event::End(TagEnd::Image) => image_depth = image_depth.saturating_sub(1),
            _ => (),
        }
        if image_depth > 0 {
            continue;
        }
        let Event::Start(Tag::Link {
            link_type,
            dest_url,
            id,
            ..
        }) = event
        else {
            continue;
        };
        // Images and text inside code/HTML blocks never become Link events.
        if !matches!(
            link_type,
            LinkType::Inline | LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut
        ) || external_target(&dest_url)
        {
            continue;
        }
        let definition_lines = if matches!(
            link_type,
            LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut
        ) {
            parser
                .reference_definitions()
                .get(&id)
                .map(|definition| lines(definition.span.clone()))
        } else {
            None
        };
        let (start_line, end_line) = lines(range);
        links.push(MarkdownLink {
            target: dest_url.into_string(),
            start_line,
            end_line,
            definition_lines,
        });
    }
    links
}

fn external_target(target: &str) -> bool {
    target.starts_with("//")
        || target.split_once(':').is_some_and(|(scheme, _)| {
            !scheme.is_empty()
                && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
        })
}

/// Strict UTF-8 percent decoding, applied once after separating the URL fragment.
fn decode_url_component(value: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = (input.next()? as char).to_digit(16)?;
            let low = (input.next()? as char).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

fn test_path(path: &str) -> bool {
    path.split('/')
        .any(|p| matches!(p, "test" | "tests" | "__tests__"))
        || path.contains(".test.")
        || path.contains(".spec.")
        || path.ends_with("_test.go")
        || path
            .rsplit('/')
            .next()
            .is_some_and(|p| p.starts_with("test_"))
}

/// Resolve decoded local URL paths lexically; the file must additionally occur in Input.
pub(crate) fn local_target(path: &str, target: &str) -> Option<(String, Option<String>)> {
    if external_target(target) {
        return None;
    }
    let (file, fragment) = target
        .split_once('#')
        .map_or((target, None), |(p, f)| (p, Some(f)));
    // A query is not a filesystem path. Encoded question/hash characters remain
    // legal filename characters because delimiters are separated before decoding.
    if file.contains('?') {
        return None;
    }
    let file = decode_url_component(file)?;
    let fragment = match fragment {
        Some(fragment) => Some(decode_url_component(fragment)?),
        None => None,
    };
    if file.starts_with('/') || file.contains(['\\', ':']) || file.chars().any(char::is_control) {
        return None;
    }
    let mut parts: Vec<&str> = path.split('/').collect();
    if file.is_empty() {
        return Some((path.into(), fragment));
    }
    parts.pop();
    for part in file.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    let resolved = parts.join("/");
    validate_path(&resolved).ok()?;
    Some((resolved, fragment))
}

fn candidate(
    graph: &mut Graph,
    from: &str,
    reference: &str,
    candidates: Vec<String>,
    evidence: Span,
    missing_reason: &str,
) {
    graph.unresolved.push(Unresolved {
        from: from.into(),
        reference: reference.into(),
        reason: if candidates.is_empty() {
            missing_reason.into()
        } else {
            "ambiguous_reference".into()
        },
        candidates,
        evidence,
    });
}

fn file_level_comment(raw: &str) -> bool {
    if raw.trim_start().starts_with("//!") {
        return true;
    }
    let Some(comment) = comment_text(raw) else {
        return false;
    };
    let comment = comment.trim_start();
    if comment.starts_with("@file ") || comment == "@file" || comment.starts_with("@fileoverview") {
        return true;
    }
    comment
        .strip_prefix("WHY:")
        .or_else(|| comment.strip_prefix("NOTE:"))
        .is_some_and(|text| {
            let text = text.trim_start();
            text.starts_with("FILE:") || text.starts_with("MODULE:")
        })
}

/// A declaration can own only the immediately preceding, equally indented
/// comment block. Blank lines, statements and explicit file metadata stop it.
fn leading_comment_owners(
    input: &Input,
    codes: &BTreeMap<u64, String>,
) -> BTreeMap<(String, u32), String> {
    let mut candidates: BTreeMap<(String, u32), Vec<String>> = BTreeMap::new();
    let mut by_file: BTreeMap<&str, Vec<&CodeSymbol>> = BTreeMap::new();
    for symbol in &input.symbols {
        if !is_document(&symbol.path) {
            by_file.entry(&symbol.path).or_default().push(symbol);
        }
    }
    for (path, symbols) in by_file {
        let lines: Vec<_> = input.files[path].lines().collect();
        for symbol in symbols {
            // Synthetic module/initializer symbols are not following declarations.
            if symbol.name.starts_with('<') {
                continue;
            }
            let start = symbol.start_line as usize - 1;
            let Some(declaration) = lines.get(start).copied() else {
                continue;
            };
            if declaration.trim().is_empty() || comment_text(declaration).is_some() {
                continue;
            }
            let indentation = &declaration[..declaration.len() - declaration.trim_start().len()];
            let mut begin = start;
            while begin > 0 {
                let raw = lines[begin - 1];
                let prefix = &raw[..raw.len() - raw.trim_start().len()];
                if prefix != indentation || comment_text(raw).is_none() {
                    break;
                }
                begin -= 1;
            }
            if lines[begin..start]
                .iter()
                .any(|raw| file_level_comment(raw))
            {
                continue;
            }
            for line in begin..start {
                candidates
                    .entry((path.into(), line as u32 + 1))
                    .or_default()
                    .push(codes[&symbol.key].clone());
            }
        }
    }
    candidates
        .into_iter()
        .filter_map(|(line, owners)| match owners.as_slice() {
            [owner] => Some((line, owner.clone())),
            _ => None,
        })
        .collect()
}

fn enclosing_owner(entries: Option<&[(u32, u32, String)]>, line: u32) -> Option<&String> {
    entries
        .and_then(|entries| {
            entries
                .iter()
                .filter(|(start, end, _)| *start <= line && line <= *end)
                .min_by_key(|(start, end, _)| end - start)
        })
        .map(|(_, _, id)| id)
}

/// Build a complete replacement snapshot. Never mutates a published generation.
pub fn build(input: &Input) -> Result<Graph> {
    validate_repo(&input.repo)?;
    let mut graph = Graph::default();
    let mut repo = Repository {
        revision: input.revision.clone(),
        ..Repository::default()
    };
    let mut bytes = 0usize;
    for (path, text) in &input.files {
        validate_path(path)?;
        bytes = bytes.checked_add(text.len()).ok_or("input size overflow")?;
        if text.len() > MAX_FILE_BYTES
            || bytes > MAX_GRAPH_BYTES as usize
            || input.files.len() > 20_000
        {
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
        let kind = if is_document(path) {
            Kind::Document
        } else {
            Kind::File
        };
        let body = if is_configuration(path) {
            text.as_str()
        } else {
            ""
        };
        let id = add_node(&mut graph, input, kind, path, "", path, 1, lines, body);
        files.insert(path.clone(), id);
    }
    for symbol in symbols {
        let text = input
            .files
            .get(&symbol.path)
            .ok_or("symbol source missing from input")?;
        if symbol.start_line == 0
            || symbol.end_line < symbol.start_line
            || symbol.end_line as usize > text.lines().count().max(1)
        {
            return Err(format!("invalid or stale symbol range: {}", symbol.qualified_name).into());
        }
        let key = format!("{}\0{}", symbol.qualified_name, symbol.signature);
        let ordinal = duplicate_keys
            .entry((symbol.path.clone(), key.clone()))
            .or_default();
        let key = format!("{key}\0{ordinal}");
        *ordinal += 1;
        let body = text
            .lines()
            .skip(symbol.start_line as usize - 1)
            .take((symbol.end_line - symbol.start_line + 1) as usize)
            .collect::<Vec<_>>()
            .join("\n");
        let kind = if test_path(&symbol.path) {
            Kind::Test
        } else {
            Kind::Symbol
        };
        let id = add_node(
            &mut graph,
            input,
            kind,
            &symbol.path,
            &key,
            &symbol.qualified_name,
            symbol.start_line,
            symbol.end_line,
            &body,
        );
        if codes.insert(symbol.key, id.clone()).is_some() {
            return Err("duplicate code symbol id".into());
        }
        names
            .entry(symbol.name.clone())
            .or_default()
            .push(id.clone());
        if symbol.qualified_name != symbol.name {
            names
                .entry(symbol.qualified_name.clone())
                .or_default()
                .push(id.clone());
        }
        if !symbol.name.starts_with('<') {
            owners.entry(symbol.path.clone()).or_default().push((
                symbol.start_line,
                symbol.end_line,
                id.clone(),
            ));
        }
        connect(
            &mut graph,
            &files[&symbol.path],
            &id,
            "defines",
            span(input, &symbol.path, symbol.start_line, symbol.end_line),
            "codanna_symbol",
        );
    }
    // Preserve relationships already resolved by Codanna; do not upgrade their certainty.
    for edge in &input.edges {
        let from = codes
            .get(&edge.from)
            .ok_or("code edge has missing source")?;
        let to = codes.get(&edge.to).ok_or("code edge has missing target")?;
        graph.edges.push(Edge {
            from: from.clone(),
            to: to.clone(),
            relation: edge.relation.to_lowercase(),
            basis: Basis::Resolved,
            evidence: graph.nodes[from].source.clone(),
            method: "codanna_index".into(),
        });
    }
    // Collect section anchors first so forward document links resolve.
    for (path, text) in input.files.iter().filter(|(p, _)| is_document(p)) {
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        let mut headings = Vec::new();
        let mut heading = None;
        let mut line_offset = 0;
        let mut line_number = 1;
        for (event, range) in Parser::new(text).into_offset_iter() {
            match event {
                Event::Start(Tag::Heading { .. }) => {
                    line_number += text[line_offset..range.start]
                        .bytes()
                        .filter(|&b| b == b'\n')
                        .count() as u32;
                    line_offset = range.start;
                    heading = Some((line_number, String::new()));
                }
                Event::Text(value) | Event::Code(value) => {
                    if let Some((_, label)) = &mut heading {
                        label.push_str(&value);
                    }
                }
                Event::SoftBreak | Event::HardBreak => {
                    if let Some((_, label)) = &mut heading {
                        label.push(' ');
                    }
                }
                Event::End(TagEnd::Heading(_)) => {
                    if let Some(entry) = heading.take() {
                        headings.push(entry);
                    }
                }
                _ => (),
            }
        }
        for (i, (start, label)) in headings.iter().enumerate() {
            let end = headings
                .get(i + 1)
                .map_or(text.lines().count().max(1) as u32, |(n, _)| n - 1);
            let raw = slug(label);
            let n = seen.entry(raw.clone()).or_default();
            let anchor = if *n == 0 { raw } else { format!("{raw}-{n}") };
            *n += 1;
            let body = text
                .lines()
                .skip(*start as usize - 1)
                .take((end - start + 1) as usize)
                .collect::<Vec<_>>()
                .join("\n");
            let id = add_node(
                &mut graph,
                input,
                Kind::Section,
                path,
                &anchor,
                label,
                *start,
                end,
                &body,
            );
            anchors.insert((path.clone(), anchor), id.clone());
            owners
                .entry(path.clone())
                .or_default()
                .push((*start, end, id.clone()));
            connect(
                &mut graph,
                &files[path],
                &id,
                "contains",
                span(input, path, *start, *start),
                "markdown_heading",
            );
        }
    }
    let leading_owners = leading_comment_owners(input, &codes);
    let ticks = Regex::new(r"`([^`\n]+)`")?;
    let adr = Regex::new(r"(?i)\b(?:ADR|RFC)[-_ ]?(\d+)\b")?;
    let mut decisions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (path, id) in &files {
        if !is_document(path) {
            continue;
        }
        for cap in adr.captures_iter(path.rsplit('/').next().unwrap_or(path)) {
            let full = cap[0].to_uppercase().replace(['-', '_', ' '], "");
            decisions.entry(full).or_default().push(id.clone());
        }
    }
    for (path, text) in &input.files {
        let doc = is_document(path);
        let mut links_by_line: BTreeMap<u32, Vec<MarkdownLink>> = BTreeMap::new();
        for link in source_links(path, text) {
            links_by_line.entry(link.start_line).or_default().push(link);
        }
        let mut fence = None;
        for (i, raw) in text.lines().enumerate() {
            if doc && fenced(raw, &mut fence) {
                continue;
            }
            let line = i as u32 + 1;
            let comment = comment_text(raw);
            if !doc && comment.is_none() {
                continue;
            }
            let source = if doc { raw } else { comment.unwrap_or("") };
            let evidence = span(input, path, line, line);
            let owner = if !doc && file_level_comment(raw) {
                &files[path]
            } else {
                leading_owners
                    .get(&(path.clone(), line))
                    .or_else(|| enclosing_owner(owners.get(path).map(Vec::as_slice), line))
                    .unwrap_or(&files[path])
            }
            .clone();
            let owner = if !doc
                && (source.trim().starts_with("WHY:") || source.trim().starts_with("NOTE:"))
            {
                let id = add_node(
                    &mut graph,
                    input,
                    Kind::Rationale,
                    path,
                    &format!("{line}"),
                    source.trim(),
                    line,
                    line,
                    source,
                );
                connect(
                    &mut graph,
                    &id,
                    &owner,
                    "explains",
                    evidence.clone(),
                    "rationale_comment",
                );
                id
            } else {
                owner
            };
            for link in links_by_line.remove(&line).unwrap_or_default() {
                let target = &link.target;
                let evidence = span(input, path, link.start_line, link.end_line);
                let resolved = local_target(path, target).and_then(|(p, f)| {
                    if let Some(fragment) = f {
                        if fragment.starts_with('L')
                            && fragment[1..]
                                .chars()
                                .all(|c| c.is_ascii_digit() || c == '-' || c == 'L')
                        {
                            let nums: Vec<_> = fragment
                                .split('-')
                                .filter_map(|s| s.trim_start_matches('L').parse::<u32>().ok())
                                .collect();
                            let lines = input.files.get(&p)?.lines().count().max(1) as u32;
                            if nums.is_empty()
                                || nums.iter().any(|&n| n == 0 || n > lines)
                                || nums.first() > nums.last()
                            {
                                return None;
                            }
                            files.get(&p).cloned()
                        } else {
                            anchors.get(&(p, fragment)).cloned()
                        }
                    } else {
                        files.get(&p).cloned()
                    }
                });
                if let Some(to) = resolved {
                    connect(
                        &mut graph,
                        &owner,
                        &to,
                        "references",
                        evidence.clone(),
                        "markdown_link",
                    );
                    if let Some((start, end)) = link.definition_lines {
                        connect(
                            &mut graph,
                            &owner,
                            &to,
                            "references",
                            span(input, path, start, end),
                            "markdown_reference_definition",
                        );
                    }
                } else {
                    candidate(
                        &mut graph,
                        &owner,
                        target,
                        vec![],
                        evidence.clone(),
                        "unresolved_local_link",
                    );
                }
            }
            for cap in ticks.captures_iter(source) {
                let reference = &cap[1];
                if let Some(found) = names.get(reference) {
                    if let [to] = found.as_slice() {
                        connect(
                            &mut graph,
                            &owner,
                            to,
                            "references",
                            evidence.clone(),
                            "exact_symbol_mention",
                        );
                    } else {
                        candidate(
                            &mut graph,
                            &owner,
                            reference,
                            found.clone(),
                            evidence.clone(),
                            "unknown_symbol",
                        );
                    }
                }
                // Unknown inline code may be a literal, not a broken symbol reference.
            }
            for cap in adr.captures_iter(source) {
                let reference = cap[0].to_uppercase().replace(['-', '_', ' '], "");
                let found = decisions.get(&reference).cloned().unwrap_or_default();
                if let [to] = found.as_slice() {
                    if to != &owner {
                        connect(
                            &mut graph,
                            &owner,
                            to,
                            "references",
                            evidence.clone(),
                            "decision_reference",
                        );
                    }
                } else {
                    candidate(
                        &mut graph,
                        &owner,
                        &cap[0],
                        found,
                        evidence.clone(),
                        "unresolved_decision",
                    );
                }
            }
        }
    }
    graph.limitations.push("Code relationships inherit Codanna's static resolution; dump-to-source freshness is not independently proven.".into());
    graph.limitations.push("Markdown links use CommonMark inline/full/collapsed/shortcut reference syntax with source spans; images, remote links and links in code/HTML blocks are excluded. Local URL paths and fragments are percent-decoded once. Headings support ATX and Setext anchors derived from parsed text; unknown inline literals are not broken references.".into());
    graph.limitations.push("Rationale comments attach to the smallest enclosing symbol or a uniquely identified adjacent declaration at the same indentation. Gaps, statements and explicit file-level metadata prevent leading attachment; this is source association, not compiler comment semantics.".into());
    graph.limitations.push(
        "Test nodes identify test source, not executed coverage or proof of correctness.".into(),
    );
    graph.normalize();
    graph.validate()?;
    Ok(graph)
}

fn fenced(line: &str, fence: &mut Option<(char, usize)>) -> bool {
    let trimmed = line.trim_start();
    let first = trimmed.chars().next().unwrap_or(' ');
    let count = trimmed.chars().take_while(|c| *c == first).count();
    if let Some((marker, size)) = *fence {
        if first == marker && count >= size && trimmed[count..].trim().is_empty() {
            *fence = None;
        }
        return true;
    }
    if matches!(first, '`' | '~') && count >= 3 {
        *fence = Some((first, count));
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn fixture() -> Input {
        Input {
            repo: "core".into(),
            files: BTreeMap::from([
                (
                    "src/upload.rs".into(),
                    "fn upload() {\n// WHY: enforce ADR-001\n}\n".into(),
                ),
                (
                    "docs/ADR-001.md".into(),
                    "# Upload limits\nUse `upload`. [source](../src/upload.rs#L1-L3)\n".into(),
                ),
            ]),
            symbols: vec![CodeSymbol {
                key: 1,
                name: "upload".into(),
                qualified_name: "upload".into(),
                signature: "fn upload()".into(),
                path: "src/upload.rs".into(),
                start_line: 1,
                end_line: 3,
            }],
            ..Input::default()
        }
    }
    #[test]
    fn links_rationale_and_reverse_edges() {
        let graph = build(&fixture()).unwrap();
        let symbol = graph.resolve("upload").unwrap();
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.to == symbol.id && e.method == "exact_symbol_mention")
        );
        assert!(graph.edges.iter().any(|e| e.relation == "explains"));
        assert!(graph.edges.iter().any(|e| e.method == "decision_reference"));
        graph.validate().unwrap();
    }
    #[test]
    fn ambiguity_is_not_an_edge() {
        let mut input = fixture();
        let mut second = input.symbols[0].clone();
        second.key = 2;
        second.qualified_name = "other::upload".into();
        input.symbols.push(second);
        let graph = build(&input).unwrap();
        assert!(
            graph
                .unresolved
                .iter()
                .any(|r| r.reference == "upload" && r.candidates.len() == 2)
        );
        assert!(
            !graph
                .edges
                .iter()
                .any(|e| e.method == "exact_symbol_mention")
        );
    }
    #[test]
    fn fences_do_not_create_fake_headings_or_links() {
        let mut input = fixture();
        input.files.insert(
            "docs/fences.md".into(),
            "```rust\n# Fake\n`upload` [bad](missing.md)\n```\n# Real\n`upload`\n".into(),
        );
        let graph = build(&input).unwrap();
        assert!(graph.resolve("Fake").is_err());
        assert!(graph.resolve("Real").is_ok());
        assert!(!graph.unresolved.iter().any(|r| r.reference == "missing.md"));
    }
    #[test]
    fn replacement_drops_deleted_links_and_keeps_identity_after_line_shift() {
        let input = fixture();
        let old = build(&input).unwrap();
        let mut changed = input;
        changed.files.remove("docs/ADR-001.md");
        changed.files.insert(
            "src/upload.rs".into(),
            "\nfn upload() {\n// WHY: enforce ADR-001\n}\n".into(),
        );
        changed.symbols[0].start_line += 1;
        changed.symbols[0].end_line += 1;
        let new = build(&changed).unwrap();
        assert_eq!(
            old.resolve("upload").unwrap().id,
            new.resolve("upload").unwrap().id
        );
        assert!(
            new.unresolved
                .iter()
                .any(|r| r.reason == "unresolved_decision")
        );
        assert!(!new.edges.iter().any(|e| e.method == "exact_symbol_mention"));
    }
    #[test]
    fn path_escape_is_rejected() {
        assert!(local_target("docs/a.md", "../../secret").is_none());
        assert!(validate_path("../secret").is_err());
        assert!(validate_repo("a:b").is_err());
    }
    #[test]
    fn stable_output_and_roundtrip() {
        let a = build(&fixture()).unwrap();
        let b = build(&fixture()).unwrap();
        assert_eq!(
            serde_json::to_vec(&a).unwrap(),
            serde_json::to_vec(&b).unwrap()
        );
        let decoded: Graph = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
        decoded.validate().unwrap();
    }
    #[test]
    fn corrupt_schema_and_dangling_edges_fail() {
        let mut graph = build(&fixture()).unwrap();
        graph.schema_version = 999;
        assert!(graph.validate().is_err());
        graph.schema_version = 1;
        graph.edges[0].to = "missing".into();
        assert!(graph.validate().is_err());
    }
    #[test]
    fn formatted_heading_links_resolve_using_visible_text() {
        let mut input = fixture();
        input.files.insert(
            "docs/a.md".into(),
            "# Contract readiness ([core-work endpoints](target.md), [OpenAPI][api])\n\
             # **Bold** and `code` &amp; *emphasis*\n\
             # [Repeated](target.md)\n\
             # Repeated\n\
             Setext title\n============\n\n\
             [api]: target.md\n\n\
             [contract](#contract-readiness-core-work-endpoints-openapi)\n\
             [formatted](#bold-and-code--emphasis)\n\
             [duplicate](#repeated-1)\n\
             [setext](#setext-title)\n"
                .into(),
        );
        input
            .files
            .insert("docs/target.md".into(), "# Target\n".into());
        input.files.insert(
            "docs/reference.md".into(),
            "[contract](a.md#contract-readiness-core-work-endpoints-openapi)\n".into(),
        );
        let graph = build(&input).unwrap();
        assert!(graph.unresolved.is_empty(), "{:?}", graph.unresolved);
        assert!(graph.edges.iter().any(|edge| {
            edge.method == "markdown_link"
                && edge.evidence.path == "docs/reference.md"
                && graph.nodes[&edge.to].source.path == "docs/a.md"
                && graph.nodes[&edge.to].source.start_line == 1
        }));
        for line in [1, 10, 11, 12, 13] {
            assert!(
                graph.edges.iter().any(|edge| {
                    edge.method == "markdown_link"
                        && edge.evidence.path == "docs/a.md"
                        && edge.evidence.start_line == line
                }),
                "missing link evidence at line {line}"
            );
        }
        graph.validate().unwrap();
    }

    #[test]
    fn duplicate_headings_are_addressable() {
        let mut input = fixture();
        input.files.insert(
            "docs/a.md".into(),
            "# Same\n# Same\n[second](#same-1)\n".into(),
        );
        let graph = build(&input).unwrap();
        assert!(!graph.unresolved.iter().any(|r| r.reference == "#same-1"));
    }
}
