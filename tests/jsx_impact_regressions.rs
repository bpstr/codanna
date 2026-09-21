//! JSX owner, import identity, and lifecycle witnesses for PR #43 / T03.
//! Only synthetic source files enter the index; assertions remain outside it.
//! Semantic search is disabled. No model, provider, or private source is used.

use codanna::indexing::facade::IndexFacade;
use codanna::mcp::requests::AnalyzeImpactRequest;
use codanna::mcp::server::CodeIntelligenceServer;
use codanna::parsing::LanguageParser;
use codanna::parsing::typescript::TypeScriptParser;
use codanna::{IndexPersistence, Range, RelationKind, Settings, Symbol};
use rmcp::handler::server::wrapper::Parameters;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

fn parser_witness(code: &str, expected: &[(&str, &str, &str)]) {
    let mut parser = TypeScriptParser::new().unwrap();
    let mut actual: Vec<_> = parser
        .find_uses(code)
        .into_iter()
        .map(|(from, to, range)| format!("{from}->{to}@{range:?}"))
        .collect();
    let mut expected: Vec<_> = expected
        .iter()
        .map(|(from, to, snippet)| {
            let start = code.find(snippet).expect("unique expected JSX snippet");
            assert_eq!(code.matches(snippet).count(), 1);
            let before = &code[..start];
            let row = before.bytes().filter(|byte| *byte == b'\n').count() as u32;
            let column = before.rsplit('\n').next().unwrap().len() as u32;
            let range = Range::new(row, column, row, column + snippet.len() as u32);
            format!("{from}->{to}@{range:?}")
        })
        .collect();
    actual.sort();
    expected.sort();
    assert_eq!(
        actual, expected,
        "owner, target, count, and range must agree"
    );
}

#[test]
fn assigned_and_nested_arrows_keep_their_own_jsx_evidence() {
    parser_witness(
        "export const Page = () => <Calendar />;\nexport function Shell() {\n  const Panel = () => <Calendar compact />;\n  return <Panel />;\n}\n",
        &[
            ("Page", "Calendar", "<Calendar />"),
            ("Panel", "Calendar", "<Calendar compact />"),
            ("Shell", "Panel", "<Panel />"),
        ],
    );
}

#[test]
fn namespace_members_are_values_even_with_lowercase_names() {
    parser_witness(
        "function Lower() { return <ui.Calendar />; }\nfunction Upper() { return <UI.Calendar />; }\nfunction Member() { return <motion.div />; }\nfunction Intrinsic() { return <div><span /></div>; }\n",
        &[
            ("Lower", "ui.Calendar", "<ui.Calendar />"),
            ("Upper", "UI.Calendar", "<UI.Calendar />"),
            ("Member", "motion.div", "<motion.div />"),
        ],
    );
}

#[test]
fn class_render_methods_do_not_lose_their_jsx_owner() {
    parser_witness(
        "class Screen { render() { return <Calendar />; } }\n",
        &[("render", "Calendar", "<Calendar />")],
    );
}

#[test]
fn closing_tags_and_outside_expressions_do_not_duplicate_or_leak_owners() {
    parser_witness(
        "const element = <Outside />;\nfunction First() { return <Calendar></Calendar>; }\nconst other = <OutsideAgain />;\nfunction Last() { return <Calendar last />; }\n",
        &[
            ("First", "Calendar", "<Calendar></Calendar>"),
            ("Last", "Calendar", "<Calendar last />"),
        ],
    );
}

fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, IndexFacade) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("src");
    for (name, code) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, code).unwrap();
    }
    let mut settings = Settings {
        workspace_root: Some(root.clone()),
        index_path: temp.path().join("index"),
        ..Default::default()
    };
    settings.semantic_search.enabled = false;
    settings.add_indexed_path(root.clone()).unwrap();
    let mut index = IndexFacade::new(Arc::new(settings)).unwrap();
    index.index_directory(&root, true).unwrap();
    (temp, index)
}

fn target(index: &IndexFacade, path: &str, name: &str) -> Symbol {
    let found: Vec<_> = index
        .find_symbols_by_name(name, Some("typescript"))
        .into_iter()
        .filter(|symbol| Path::new(symbol.file_path.as_ref()).ends_with(Path::new(path)))
        .collect();
    assert_eq!(found.len(), 1, "ambiguous {path}:{name}: {found:?}");
    found[0].clone()
}

fn uses(index: &IndexFacade, path: &str, owner: &str) -> BTreeSet<String> {
    let owner = target(index, path, owner);
    let edges = index
        .graph_neighbors(owner.id, RelationKind::Uses, false, None)
        .unwrap();
    edges
        .into_iter()
        .map(|(symbol, _)| format!("{}:{}", symbol.file_path.as_ref(), symbol.name))
        .collect()
}

fn assert_uses(index: &IndexFacade, path: &str, owner: &str, expected: &[(&str, &str)]) {
    let expected: BTreeSet<_> = expected
        .iter()
        .map(|(path, name)| {
            let symbol = target(index, path, name);
            format!("{}:{}", symbol.file_path.as_ref(), symbol.name)
        })
        .collect();
    assert_eq!(uses(index, path, owner), expected, "{path}:{owner}");
}

const PROVIDER: &str = "export function Calendar() { return null; }\n";
const NAMED_VIEW: &str = "import { PublicCalendar as Day } from './barrel';\nexport const Page = () => <Day />;\nexport function Shell() {\n  const Panel = () => <Day compact />;\n  return <Panel />;\n}\n";

#[test]
fn aliased_barrel_components_reach_the_correct_persisted_uses_graph() {
    let (_temp, index) = fixture(&[
        ("provider.tsx", PROVIDER),
        ("reference.tsx", PROVIDER),
        (
            "barrel.ts",
            "export { Calendar as PublicCalendar } from './provider';\n",
        ),
        ("view.tsx", NAMED_VIEW),
    ]);
    assert_uses(&index, "view.tsx", "Page", &[("provider.tsx", "Calendar")]);
    assert_uses(&index, "view.tsx", "Panel", &[("provider.tsx", "Calendar")]);
    assert_uses(&index, "view.tsx", "Shell", &[("view.tsx", "Panel")]);
    let page = target(&index, "view.tsx", "Page");
    let edges = index
        .document_index()
        .get_relationships_from(page.id, RelationKind::Uses)
        .unwrap();
    assert_eq!(edges.len(), 1);
    let metadata = edges[0]
        .2
        .metadata
        .as_ref()
        .expect("persist JSX source location");
    assert_eq!(metadata.line, Some(1));
    assert_eq!(metadata.column, Some(26));
    assert!(
        index.get_called_functions(page.id).is_empty(),
        "rendering is not a Calls edge"
    );
}

#[test]
fn namespace_exports_do_not_bind_to_a_local_or_reference_namesake() {
    let (_temp, index) = fixture(&[
        ("provider.tsx", PROVIDER),
        ("reference.tsx", PROVIDER),
        (
            "barrel.ts",
            "export { Calendar as PublicCalendar } from './provider';\n",
        ),
        (
            "view.tsx",
            "import * as ui from './barrel';\nimport * as UI from './barrel';\nfunction PublicCalendar() { return null; }\nexport function Lower() { return <ui.PublicCalendar />; }\nexport function Upper() { return <UI.PublicCalendar />; }\nexport function Missing() { return <UI.Calendar />; }\n",
        ),
    ]);
    for owner in ["Lower", "Upper"] {
        assert_uses(&index, "view.tsx", owner, &[("provider.tsx", "Calendar")]);
    }
    assert_uses(&index, "view.tsx", "Missing", &[]);
}

#[test]
fn shadowed_and_external_namespace_roots_never_guess_a_local_component() {
    let (_temp, index) = fixture(&[
        ("provider.tsx", PROVIDER),
        (
            "view.tsx",
            "import * as ui from './provider';\nimport * as external from 'unindexed-library';\nfunction Calendar() { return null; }\nexport function Parameter(ui) { return <ui.Calendar />; }\nexport function Local(other) { const ui = other; return <ui.Calendar local />; }\nexport function External() { return <external.Calendar />; }\n",
        ),
    ]);
    for owner in ["Parameter", "Local", "External"] {
        assert_uses(&index, "view.tsx", owner, &[]);
    }
}

#[tokio::test]
async fn public_impact_reaches_arrow_consumers_without_reaching_reference_decoys() {
    let (_temp, index) = fixture(&[
        ("provider.tsx", PROVIDER),
        ("reference.tsx", PROVIDER),
        (
            "barrel.ts",
            "export { Calendar as PublicCalendar } from './provider';\n",
        ),
        ("view.tsx", NAMED_VIEW),
    ]);
    let actual = target(&index, "provider.tsx", "Calendar");
    let decoy = target(&index, "reference.tsx", "Calendar");
    let page = target(&index, "view.tsx", "Page");
    let panel = target(&index, "view.tsx", "Panel");
    let shell = target(&index, "view.tsx", "Shell");
    assert_eq!(
        index
            .get_impact_radius(actual.id, Some(1))
            .into_iter()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([page.id, panel.id])
    );
    assert_eq!(
        index
            .get_impact_radius(actual.id, Some(2))
            .into_iter()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([page.id, panel.id, shell.id])
    );
    assert!(index.get_impact_radius(decoy.id, Some(3)).is_empty());
    let server = CodeIntelligenceServer::new(index);
    let response = server
        .analyze_impact(Parameters(AnalyzeImpactRequest {
            symbol_name: None,
            symbol_id: Some(actual.id.value()),
            max_depth: 2,
        }))
        .await
        .unwrap();
    assert_ne!(response.is_error, Some(true));
    let rendered = serde_json::to_string(&response).unwrap();
    for owner in ["Page", "Panel", "Shell"] {
        assert!(rendered.contains(owner), "{rendered}");
    }
    assert!(!rendered.contains("reference.tsx"), "{rendered}");
}

#[test]
fn namespace_uses_survive_reopen_import_edit_deletion_and_recreation() {
    let (temp, mut index) = fixture(&[
        ("provider.tsx", PROVIDER),
        ("replacement.tsx", PROVIDER),
        (
            "view.tsx",
            "import * as ui from './provider';\nexport const Page = () => <ui.Calendar />;\n",
        ),
    ]);
    assert_uses(&index, "view.tsx", "Page", &[("provider.tsx", "Calendar")]);
    let settings = Arc::clone(index.settings());
    let persistence = IndexPersistence::new(settings.index_path.clone());
    persistence.save_facade(&index).unwrap();
    drop(index);
    index = persistence.load_facade_lite(settings).unwrap();
    assert_uses(&index, "view.tsx", "Page", &[("provider.tsx", "Calendar")]);
    let root = temp.path().join("src");
    let view = root.join("view.tsx");
    std::fs::write(
        &view,
        "import * as ui from './replacement';\nexport const Page = () => <ui.Calendar />;\n",
    )
    .unwrap();
    index.index_file(&view).unwrap();
    assert_uses(
        &index,
        "view.tsx",
        "Page",
        &[("replacement.tsx", "Calendar")],
    );
    let replacement = root.join("replacement.tsx");
    std::fs::remove_file(&replacement).unwrap();
    index.remove_file(&replacement).unwrap();
    assert_uses(&index, "view.tsx", "Page", &[]);
    std::fs::write(&replacement, PROVIDER).unwrap();
    index.index_file(&replacement).unwrap();
    assert_uses(
        &index,
        "view.tsx",
        "Page",
        &[("replacement.tsx", "Calendar")],
    );
}
