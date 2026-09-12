from pathlib import Path
import os


def replace(path, before, after):
    p = Path(path)
    text = p.read_text()
    assert text.count(before) == 1, (path, before[:100], text.count(before))
    p.write_text(text.replace(before, after))


replace('src/mcp/server.rs',
        'let mut settings = Settings { workspace_root: Some(workspace), ..Settings::default() };',
        'let settings = Settings { workspace_root: Some(workspace), ..Settings::default() };')
replace('src/storage/persistence.rs',
        'let mut settings = Settings { index_path: dir.path().to_path_buf(), ..Settings::default() };',
        'let settings = Settings { index_path: dir.path().to_path_buf(), ..Settings::default() };')

p = Path('tests/exploration/abi15_grammar_audit/helpers.rs')
s = p.read_text()
start = s.index('pub fn run_tree_structure_analysis(')
end = s.index('\n/// Generate tree structure', start)
old = s[start:end]
before, body = old.split('    if let Some(tree) = parser.parse(&code, None) {\n')
assert body.endswith('    }\n}\n')
body = body[:-8]
assert body.endswith('\n')
body = '\n'.join(line[4:] if line.startswith('    ') else line for line in body.split('\n'))
new = before + '''    let code = fs::read_to_string(config.example_file_path).expect("tree analysis requires the complete example");
    assert!(!code.trim().is_empty(), "tree example cannot be empty");
    let tree = parser.parse(&code, None).expect("tree analysis must produce a syntax tree");
''' + body + '}\n'
p.write_text(s[:start] + new + s[end:])

# Superseded by the active isolated pipeline fixture in the same test file.
p = Path('tests/parsers/typescript/test_pipeline_resolution.rs')
s = p.read_text()
start = s.index('/// Integration test: Pipeline resolution with TypeScript settings.')
end = s.index('/// Test that the pipeline cache handles import resolution.', start)
s = s[:start] + s[end:]
for line in [
    'use codanna::project_resolver::persist::ResolutionPersistence;\n',
    'use codanna::project_resolver::provider::ProjectResolutionProvider;\n',
    'use codanna::project_resolver::providers::typescript::TypeScriptProvider;\n',
]:
    assert s.count(line) == 1
    s = s.replace(line, '')
start = s.index('    // Should find the Button symbol (via import path matching)')
end = s.index('\n}\n', start)
s = s[:start] + '''    assert_eq!(result, codanna::parsing::ResolveResult::Found(SymbolId::new(1).unwrap()),
        "a single matching import must not be ambiguous");''' + s[end:]
p.write_text(s)

# Unclosed classes are legal literal text; a reversed range is genuinely invalid.
replace('src/indexing/walker.rs',
        '        fs::write(dir.path().join(".codannaignore"), "[\\n").unwrap();',
        '''        let invalid_rule = "[z-a]";
        let mut rules = ignore::gitignore::GitignoreBuilder::new(dir.path());
        assert!(rules.add_line(None, invalid_rule).is_err(), "fixture must be invalid gitignore syntax");
        fs::write(dir.path().join(".codannaignore"), format!("{invalid_rule}\\n")).unwrap();''')

# Finish independent suites to report all failures, but retain a nonzero final outcome.
p = Path('contributing/scripts/review-regressions.sh')
s = p.read_text()
s = s.replace('cargo test --locked --all-features --no-run\n', 'cargo test --locked --all-features --no-run\nstatus=0\n')
s = '\n'.join(line + ' || status=1' if line.lstrip().startswith('run_tests --') or line.startswith('node --test ') else line for line in s.split('\n'))
p.write_text(s + '\nexit "$status"\n')

# Fix the production mismatch revealed by the actual CLI regression.
p = Path('src/retrieve.rs')
s = p.read_text()
assert s.count('Envelope::not_found(') == 3
s = s.replace('Envelope::not_found(', 'not_found_envelope(')
assert s.count('Envelope::error(') == 4
s = s.replace('Envelope::error(', 'query_error_envelope(')
s = s.replace('let err: Envelope<()> = query_error_envelope(', 'let err: Envelope<()> = Envelope::error(')
marker = '// =============================================================================\n// QueryContext - Shared abstraction for retrieve commands'
assert s.count(marker) == 1
s = s.replace(marker, '''// `retrieve` preserves its typed CLI exit vocabulary (not found = 3,
// invalid query = 1). MCP keeps its distinct 1/2 envelope vocabulary.
// Declared JSON outcomes must match the process code in both interfaces.
fn not_found_envelope<T>(message: impl Into<String>) -> Envelope<T> {
    let mut envelope = Envelope::not_found(message);
    envelope.exit_code = ExitCode::NotFound as u8;
    envelope
}

fn query_error_envelope<T>(code: ResultCode, message: impl Into<String>) -> Envelope<T> {
    let mut envelope = Envelope::error(code, message);
    envelope.exit_code = ExitCode::GeneralError as u8;
    envelope
}

''' + marker)
p.write_text(s)
replace('tests/cli/test_review_cli_contracts.rs',
        '''    let missing = fixture.json(&["retrieve", "symbol", "no_such_review_symbol", "--json"], 3);
    assert_eq!(missing["status"], "not_found");
    assert!(missing["data"].is_null());''',
        '''    for command in ["symbol", "calls", "callers", "describe", "implementations", "search"] {
        let missing = fixture.json(&["retrieve", command, "no_such_review_symbol", "--json"], 3);
        assert_eq!(missing["status"], "not_found");
        assert!(missing["data"].is_null());
    }
    for command in ["symbol", "calls", "callers", "describe"] {
        let invalid = fixture.json(&["retrieve", command, "symbol_id:invalid", "--json"], 1);
        assert_eq!(invalid["code"], "INVALID_QUERY");
    }
    let projected = fixture.json(&["retrieve", "symbol", "review_entry", "--fields", "invalid_field", "--json"], 2);
    assert_eq!(projected["code"], "INVALID_QUERY");
    // MCP deliberately uses a different not-found process code; preserve it.
    let missing = fixture.json(&["mcp", "find_symbol", "name:no_such_review_symbol", "--json"], 1);
    assert_eq!(missing["status"], "not_found");''')
with Path(os.environ['RUNNER_TEMP'], 'changed-rust.txt').open('a') as f:
    f.write('\nsrc/retrieve.rs\n')
