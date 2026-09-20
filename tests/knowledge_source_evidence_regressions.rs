//! Regression fixtures for source-backed knowledge links and comment ownership.
//! These tests use independent local sources and require no models or providers.
#[allow(dead_code)]
#[path = "../src/knowledge/mod.rs"]
mod knowledge;

use knowledge::{CodeSymbol, Graph, Input, Kind, digest, io, links};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn input(files: &[(&str, &str)], symbols: Vec<CodeSymbol>) -> Input {
    Input {
        repo: "source-fixture".into(),
        files: files
            .iter()
            .map(|(path, text)| ((*path).into(), (*text).into()))
            .collect(),
        symbols,
        ..Input::default()
    }
}

fn symbol(key: u64, name: &str, path: &str, start_line: u32, end_line: u32) -> CodeSymbol {
    CodeSymbol {
        key,
        name: name.into(),
        qualified_name: name.into(),
        signature: format!("def {name}()"),
        path: path.into(),
        start_line,
        end_line,
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn empty_dump(root: &Path) -> std::path::PathBuf {
    write(
        root,
        ".codanna/empty.jsonl",
        "{\"type\":\"begin\"}\n\
         {\"type\":\"summary\",\"status\":\"success\",\"data\":{\"symbols\":0,\"relationships\":0}}\n",
    );
    root.join(".codanna/empty.jsonl")
}

fn link_targets(graph: &Graph, source: &str) -> Vec<String> {
    graph
        .edges
        .iter()
        .filter(|edge| edge.method == "markdown_link" && edge.evidence.path == source)
        .map(|edge| graph.nodes[&edge.to].source.path.clone())
        .collect()
}

#[test]
fn reference_links_use_active_definition_and_retain_both_source_spans() {
    let document = "# Routing\n\
        Consult [the limit][Policy Label].\n\
        Use [POLICY LABEL][] and [policy label].\n\
        Unused definitions are inert.\n\n\
        [policy label]: target.md#limits \"Details\"\n\
        [POLICY LABEL]: missing.md\n\
        [unused]: absent.md\n";
    let sources = input(
        &[
            ("docs/routing.md", document),
            ("docs/target.md", "# Limits\nBounded.\n"),
        ],
        vec![],
    );
    let graph = links::build(&sources).unwrap();
    let target = graph.resolve("Limits").unwrap();
    let reference_edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.to == target.id && edge.method == "markdown_link")
        .collect();
    assert_eq!(
        reference_edges.len(),
        2,
        "uses on the same source line deduplicate"
    );
    assert_eq!(
        reference_edges
            .iter()
            .map(|edge| (edge.evidence.start_line, edge.evidence.end_line))
            .collect::<Vec<_>>(),
        [(2, 2), (3, 3)]
    );
    for edge in &reference_edges {
        assert_eq!(graph.nodes[&edge.from].label, "Routing");
        assert_eq!(edge.evidence.hash, digest(document));
    }
    let definition = graph
        .edges
        .iter()
        .find(|edge| edge.method == "markdown_reference_definition")
        .unwrap();
    assert_eq!(definition.to, target.id);
    assert_eq!(
        (definition.evidence.start_line, definition.evidence.end_line),
        (6, 6)
    );
    assert_eq!(definition.evidence.hash, digest(document));
    assert!(graph.unresolved.is_empty());
}

#[test]
fn multiline_reference_use_and_definition_preserve_their_containing_lines() {
    let document = "# Guide\n\
        Read [the\n\
        procedure][RUN BOOK].\n\n\
        [run book]:\n\
          <Run Book.md>\n";
    let graph = links::build(&input(
        &[
            ("docs/guide.md", document),
            ("docs/Run Book.md", "# Procedure\n"),
        ],
        vec![],
    ))
    .unwrap();
    let usage = graph
        .edges
        .iter()
        .find(|edge| edge.method == "markdown_link")
        .unwrap();
    assert_eq!((usage.evidence.start_line, usage.evidence.end_line), (2, 3));
    let definition = graph
        .edges
        .iter()
        .find(|edge| edge.method == "markdown_reference_definition")
        .unwrap();
    assert_eq!(
        (definition.evidence.start_line, definition.evidence.end_line),
        (5, 6)
    );
    assert_eq!(graph.nodes[&usage.to].source.path, "docs/Run Book.md");
}

#[test]
fn parser_excludes_images_code_html_remote_and_unused_reference_definitions() {
    let document = concat!(
        "# Guide\n",
        "[real](target.md)\n\n",
        "![image](target.md)\n",
        "![image with [link](target.md)](image.png)\n",
        "`[inline example](target.md)`\n",
        "\\[escaped](target.md)\n\n",
        "    [indented example](target.md)\n\n",
        "```markdown\n[example][hidden]\n[hidden]: target.md\n```\n\n",
        "<div>\n[HTML example](target.md)\n</div>\n\n",
        "[remote](https://example.invalid/target.md) [network](//example.invalid/target.md)\n",
        "[mail](mailto:someone@example.invalid) [data](data:text/plain,example)\n",
        "[unknown][missing definition]\n\n",
        "[unused]: target.md\n",
    );
    let graph = links::build(&input(
        &[
            ("docs/guide.md", document),
            ("docs/target.md", "# Target\n"),
        ],
        vec![],
    ))
    .unwrap();
    assert_eq!(link_targets(&graph, "docs/guide.md"), ["docs/target.md"]);
    assert!(graph.unresolved.is_empty());
}

#[test]
fn local_paths_decode_once_and_keep_filename_delimiters_distinct_from_url_parts() {
    let document = concat!(
        "# Links\n",
        "[space](Read%20Me.md)\n",
        "[unicode](%E6%8C%87%E9%87%9D.md#%C3%A9lan)\n",
        "[literal escape](%252e%252e.toml)\n",
        "[hash](hash%23file.md)\n",
        "[question](query%3Ffile.md)\n",
        "[parentheses](draft\\(old\\).md \"Draft\")\n",
        "[code line](../src/job.py#L1-L2)\n",
    );
    let graph = links::build(&input(
        &[
            ("docs/links.md", document),
            ("docs/Read Me.md", "# Read me\n"),
            ("docs/指針.md", "# Élan\n"),
            ("docs/%2e%2e.toml", "enabled = true\n"),
            ("docs/hash#file.md", "# Hash\n"),
            ("docs/query?file.md", "# Question\n"),
            ("docs/draft(old).md", "# Draft\n"),
            ("src/job.py", "def job():\n    pass\n"),
        ],
        vec![],
    ))
    .unwrap();
    assert_eq!(link_targets(&graph, "docs/links.md").len(), 7);
    assert!(graph.unresolved.is_empty());
    assert!(
        graph.edges.iter().any(|edge| {
            edge.method == "markdown_link" && graph.nodes[&edge.to].label == "Élan"
        })
    );
}

#[test]
fn decoded_traversal_and_ambiguous_host_paths_never_resolve_as_local_files() {
    for target in [
        "../../outside.toml",
        "%2e%2e/%2e%2e/outside.toml",
        "..%2f..%2foutside.toml",
        "%2Fetc%2Fsettings.toml",
        "%2F%2Fhost%2Fsettings.toml",
        "C%3A/settings.toml",
        "C:/settings.toml",
        "%5C%5Chost%5Csettings.toml",
        "..%5Csettings.toml",
        "settings%00.toml",
        "settings%0a.toml",
        "settings%FF.toml",
        "settings%2.toml",
        "settings%XZ.toml",
        "settings.toml?download=1",
    ] {
        assert!(
            links::local_target("docs/guide.md", target).is_none(),
            "{target}"
        );
    }
    assert_eq!(
        links::local_target("docs/guide.md", "%2e%2e/config/settings.toml"),
        Some(("config/settings.toml".into(), None))
    );
}

#[test]
fn decoded_missing_anchors_and_out_of_range_lines_remain_unresolved() {
    let graph = links::build(&input(
        &[
            (
                "docs/links.md",
                "[missing](target.md#%61bsent)\n[invalid line](../src/job.py#L9)\n",
            ),
            ("docs/target.md", "# Present\n"),
            ("src/job.py", "def job():\n    pass\n"),
        ],
        vec![],
    ))
    .unwrap();
    assert!(link_targets(&graph, "docs/links.md").is_empty());
    assert_eq!(graph.unresolved.len(), 2);
    assert!(
        graph
            .unresolved
            .iter()
            .any(|reference| reference.reference == "target.md#%61bsent")
    );
}

#[test]
fn comment_reference_definitions_respect_block_boundaries_and_source_lines() {
    let source = concat!(
        "def first():\n",
        "    # [local][settings]\n",
        "    #\n",
        "    # [settings]: ../config/worker.toml\n",
        "    pass\n",
        "\n",
        "# [unrelated][settings]\n",
        "def second():\n",
        "    pass\n",
    );
    let graph = links::build(&input(
        &[
            ("src/workers.py", source),
            ("config/worker.toml", "max_workers = 6\n"),
        ],
        vec![
            symbol(1, "first", "src/workers.py", 1, 5),
            symbol(2, "second", "src/workers.py", 8, 9),
        ],
    ))
    .unwrap();
    assert_eq!(
        link_targets(&graph, "src/workers.py"),
        ["config/worker.toml"]
    );
    let usage = graph
        .edges
        .iter()
        .find(|edge| edge.method == "markdown_link")
        .unwrap();
    let definition = graph
        .edges
        .iter()
        .find(|edge| edge.method == "markdown_reference_definition")
        .unwrap();
    assert_eq!(usage.evidence.start_line, 2);
    assert_eq!(definition.evidence.start_line, 4);
    assert_eq!(graph.nodes[&usage.from].label, "first");
    assert_eq!(usage.evidence.hash, digest(source));
}

#[test]
fn declarations_own_only_adjacent_comments_and_keep_real_rationale_spans() {
    let source = concat!(
        "# WHY: validate before allocation.\n",
        "# The comment block continues.\n",
        "def allocate():\n",
        "    # NOTE: keep the local cache bounded.\n",
        "    pass\n",
        "\n",
        "# WHY: separated explanation.\n",
        "\n",
        "def separated():\n",
        "    pass\n",
        "\n",
        "# NOTE: configuration applies to the module.\n",
        "SETTING = True\n",
        "def later():\n",
        "    pass\n",
        "\n",
        "# WHY: FILE: this module stages resources.\n",
        "def final_step():\n",
        "    pass\n",
    );
    let graph = links::build(&input(
        &[("src/resources.py", source)],
        vec![
            symbol(1, "<module>", "src/resources.py", 1, 19),
            symbol(2, "allocate", "src/resources.py", 3, 5),
            symbol(3, "separated", "src/resources.py", 9, 10),
            symbol(4, "later", "src/resources.py", 14, 15),
            symbol(5, "final_step", "src/resources.py", 18, 19),
        ],
    ))
    .unwrap();
    let owners: BTreeMap<_, _> = graph
        .edges
        .iter()
        .filter(|edge| edge.relation == "explains")
        .map(|edge| {
            let rationale = &graph.nodes[&edge.from];
            assert_eq!(rationale.kind, Kind::Rationale);
            assert_eq!(rationale.source, edge.evidence);
            assert_eq!(rationale.source.hash, digest(source));
            (
                rationale.source.start_line,
                graph.nodes[&edge.to].label.as_str(),
            )
        })
        .collect();
    assert_eq!(
        owners,
        BTreeMap::from([
            (1, "allocate"),
            (4, "allocate"),
            (7, "src/resources.py"),
            (12, "src/resources.py"),
            (17, "src/resources.py"),
        ])
    );
}

#[test]
fn leading_comments_do_not_cross_scope_or_choose_between_colliding_declarations() {
    let source = "# WHY: top-level guidance.\n    def nested():\n        pass\n\
                  # WHY: unresolved ownership.\ndef shared():\n    pass\n";
    let graph = links::build(&input(
        &[("src/scope.py", source)],
        vec![
            symbol(1, "nested", "src/scope.py", 2, 3),
            symbol(2, "first::shared", "src/scope.py", 5, 6),
            symbol(3, "second::shared", "src/scope.py", 5, 6),
        ],
    ))
    .unwrap();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.relation == "explains")
    {
        assert_eq!(graph.nodes[&edge.to].kind, Kind::File);
    }
}

#[test]
fn inner_documentation_and_file_metadata_keep_file_ownership() {
    let source = "//! NOTE: crate-wide invariant.\nfn inner() {}\n\n\
                  // @fileoverview All declarations share this resource.\n\
                  // WHY: preserve the resource lifecycle.\nfn next() {}\n";
    let graph = links::build(&input(
        &[("src/resources.rs", source)],
        vec![
            symbol(1, "inner", "src/resources.rs", 2, 2),
            symbol(2, "next", "src/resources.rs", 6, 6),
        ],
    ))
    .unwrap();
    let rationales: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.relation == "explains")
        .collect();
    assert_eq!(rationales.len(), 2);
    assert!(
        rationales
            .iter()
            .all(|edge| graph.nodes[&edge.to].kind == Kind::File)
    );
}

#[test]
fn only_explicitly_linked_visible_configuration_is_ingested_without_symbols() {
    let root = tempfile::tempdir().unwrap();
    let document = "# Settings\n\
        Read [runtime][settings] and [ignored](../config/private.toml).\n\n\
        [settings]: ../config/runtime%20settings.toml#L2\n\
        [unused]: ../config/unused.toml\n\
        ![image](../config/image.toml)\n\
        `[example](../config/example.toml)`\n\
        [binary](../assets/raw.bin)\n";
    let configuration = "# [next](other.yaml)\nmax_workers = 6\n";
    write(root.path(), "docs/settings.md", document);
    write(root.path(), "config/runtime settings.toml", configuration);
    write(root.path(), "config/other.yaml", "workers: 6\n");
    for path in ["private", "unused", "image", "example"] {
        write(
            root.path(),
            &format!("config/{path}.toml"),
            "include = false\n",
        );
    }
    write(root.path(), "assets/raw.bin", "not configuration\n");
    write(root.path(), ".codannaignore", "config/private.toml\n");
    let dump = empty_dump(root.path());
    let sources = io::input(root.path(), "source-fixture", Some(&dump)).unwrap();
    assert!(sources.symbols.is_empty());
    assert_eq!(
        sources.files.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "config/other.yaml",
            "config/runtime settings.toml",
            "docs/settings.md"
        ]
    );
    let graph = links::build(&sources).unwrap();
    let configuration_node = graph.resolve("config/runtime settings.toml").unwrap();
    assert_eq!(configuration_node.kind, Kind::File);
    assert_eq!(configuration_node.excerpt, configuration);
    assert_eq!(configuration_node.source.hash, digest(configuration));
    assert_eq!(configuration_node.source.end_line, 2);
    assert_eq!(
        link_targets(&graph, "docs/settings.md"),
        ["config/runtime settings.toml"]
    );
    assert!(graph.nodes.values().all(|node| node.kind != Kind::Symbol));
    assert!(
        graph
            .unresolved
            .iter()
            .any(|reference| reference.reference == "../config/private.toml")
    );

    write(
        root.path(),
        "docs/settings.md",
        "# Settings\nReferences removed.\n",
    );
    let replacement = io::input(root.path(), "source-fixture", Some(&dump)).unwrap();
    assert_eq!(
        replacement
            .files
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["docs/settings.md"]
    );
}

#[cfg(unix)]
#[test]
fn linked_configuration_cannot_bypass_symlink_or_directory_ignore_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "external.toml", "outside = true\n");
    write(root.path(), "config/hidden.toml", "ignored = true\n");
    write(root.path(), ".codannaignore", "config/\n");
    std::os::unix::fs::symlink(
        outside.path().join("external.toml"),
        root.path().join("linked.toml"),
    )
    .unwrap();
    write(
        root.path(),
        "guide.md",
        "# Guide\n[escape](linked.toml)\n[ignored](config/hidden.toml)\n[encoded](%2e%2e/external.toml)\n",
    );
    let dump = empty_dump(root.path());
    let sources = io::input(root.path(), "source-fixture", Some(&dump)).unwrap();
    assert_eq!(
        sources.files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["guide.md"]
    );
    let graph = links::build(&sources).unwrap();
    assert!(link_targets(&graph, "guide.md").is_empty());
    assert_eq!(graph.unresolved.len(), 3);
}

#[test]
fn linked_configuration_obeys_existing_file_byte_limits() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "guide.md", "[configuration](large.toml)\n");
    write(
        root.path(),
        "large.toml",
        &"x".repeat(knowledge::MAX_FILE_BYTES + 1),
    );
    let dump = empty_dump(root.path());
    let error = io::input(root.path(), "source-fixture", Some(&dump)).unwrap_err();
    assert!(error.to_string().contains("byte limit"));
}
