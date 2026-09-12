use super::adapter::{self, Provider};
use super::store::Store;
use serde_json::{Value, json};
use std::fs;

fn jsonl(rows: &[Value]) -> String {
    rows.iter().map(|v| format!("{v}\n")).collect()
}

fn codex(role: &str, text: &str) -> Value {
    json!({"type":"response_item","timestamp":"2026-09-12T12:00:00Z",
        "payload":{"type":"message","role":role,"content":[{"type":"input_text","text":text}]}})
}

fn claude(role: &str, text: &str) -> Value {
    json!({"type":role,"uuid":format!("{role}-1"),"sessionId":"claude-session",
        "message":{"role":role,"content":[{"type":"text","text":text}]}})
}

#[test]
fn canonical_codex_text_without_duplicate_events_or_reasoning() {
    let input = jsonl(&[
        json!({"type":"session_meta","payload":{"id":"codex-session"}}),
        codex("user", "Status bar must remain stable"),
        json!({"type":"event_msg","payload":{"type":"user_message","message":"Status bar must remain stable"}}),
        codex("assistant", "Use a fixed status bar container"),
        json!({"type":"response_item","payload":{"type":"reasoning","summary":[{"text":"secret"}]}}),
        codex("system", "secret"),
        codex("user", "# AGENTS.md instructions for this project"),
    ]);
    let parsed = adapter::parse(input.as_bytes(), Provider::Codex, "s", "session.jsonl").unwrap();
    assert_eq!(parsed.messages.len(), 2);
    assert_eq!(parsed.messages[0].thread_id, "codex-session");
    assert_eq!(parsed.messages[1].role, "assistant");
    assert_eq!(parsed.messages[0].line, 2);
}

#[test]
fn claude_text_only_and_meta_exclusion() {
    let input = jsonl(&[
        claude("user", "Status bar rules"),
        json!({"type":"assistant","uuid":"a","message":{"role":"assistant","content":[
            {"type":"thinking","thinking":"private"},
            {"type":"tool_use","input":{"text":"private"}},
            {"type":"text","text":"Searchable answer"}]}}),
        json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"private"}]}}),
        json!({"type":"user","isMeta":true,"message":{"role":"user","content":"private"}}),
    ]);
    let parsed = adapter::parse(input.as_bytes(), Provider::Claude, "s", "c.jsonl").unwrap();
    assert_eq!(parsed.messages.len(), 2);
    assert_eq!(parsed.messages[1].text, "Searchable answer");
    assert_eq!(parsed.messages[1].thread_id, "claude-session");
}

#[test]
fn partial_tail_is_deferred_and_corrupt_complete_line_rejected() {
    let base = jsonl(&[codex("user", "status bar")]);
    let input = format!("{base}{{\"type\":");
    let parsed = adapter::parse(input.as_bytes(), Provider::Codex, "s", "p").unwrap();
    assert_eq!(parsed.messages.len(), 1);
    assert!(parsed.pending_tail_bytes > 0);
    assert!(adapter::parse(format!("{input}\n").as_bytes(), Provider::Codex, "s", "p").is_err());
}

#[test]
fn stable_ids_on_append_and_duplicate_native_ids_replace() {
    let first = jsonl(&[claude("user", "first")]);
    let second = jsonl(&[claude("user", "first"), claude("assistant", "next")]);
    let a = adapter::parse(first.as_bytes(), Provider::Claude, "s", "p").unwrap();
    let b = adapter::parse(second.as_bytes(), Provider::Claude, "s", "p").unwrap();
    assert_eq!(a.messages[0].id, b.messages[0].id);
    let repeated = jsonl(&[claude("user", "first"), claude("user", "corrected")]);
    let p = adapter::parse(repeated.as_bytes(), Provider::Claude, "s", "p").unwrap();
    assert_eq!(p.messages.len(), 1);
    assert_eq!(p.messages[0].text, "corrected");
    assert_eq!(p.duplicate_records, 1);
}

#[test]
fn rejects_unsupported_format_and_limits() {
    assert!(adapter::parse(b"{}\n", Provider::Codex, "s", "p").is_err());
    let input = jsonl(&[codex("user", &"x".repeat(65 * 1024))]);
    assert!(adapter::parse(input.as_bytes(), Provider::Codex, "s", "p").is_err());
    assert!(super::validate_workspace("").is_err());
    assert!(super::validate_workspace("a\nb").is_err());
}

#[test]
fn shared_recall_prefers_users_and_finds_assistant_only_terms() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("index"), true).unwrap();
    let c = temp.path().join("codex.jsonl");
    let a = temp.path().join("claude.jsonl");
    fs::write(&c, jsonl(&[codex("user", "status bar layout")])).unwrap();
    fs::write(&a, jsonl(&[claude("assistant", "status bar layout"), claude("user", "separate topic")])).unwrap();
    store.import("assign", Provider::Codex, &c).unwrap();
    store.import("assign", Provider::Claude, &a).unwrap();
    let hits = store.search("assign", "status bar", 8, None, None).unwrap();
    assert_eq!(hits["total_matches"], 2);
    assert_eq!(hits["results"][0]["message"]["role"], "user");
    let id = hits["results"][1]["message"]["id"].as_str().unwrap();
    assert_eq!(store.read("assign", id).unwrap()["message"]["text"], "status bar layout");
    let answers = store.search("assign", "status bar", 8, Some("assistant"), None).unwrap();
    assert_eq!(answers["total_matches"], 1);
    assert_eq!(answers["results"][0]["message"]["provider"], "claude");
}

#[test]
fn workspace_isolation_in_search_read_and_forget() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("index"), true).unwrap();
    let file = temp.path().join("log.jsonl");
    fs::write(&file, jsonl(&[codex("user", "status bar")])).unwrap();
    let imported = store.import("private", Provider::Codex, &file).unwrap();
    let hits = store.search("private", "status", 8, None, None).unwrap();
    let id = hits["results"][0]["message"]["id"].as_str().unwrap();
    assert_eq!(store.search("other", "status", 8, None, None).unwrap()["total_matches"], 0);
    assert!(store.read("other", id).is_err());
    assert!(store.forget("other", imported["source_id"].as_str().unwrap()).is_err());
}

#[test]
fn unchanged_import_replacement_failure_and_forget() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index");
    let store = Store::open(&path, true).unwrap();
    let reader = Store::open(&path, false).unwrap();
    let file = temp.path().join("log.jsonl");
    let original = jsonl(&[codex("user", "old status bar")]);
    fs::write(&file, &original).unwrap();
    let imported = store.import("assign", Provider::Codex, &file).unwrap();
    assert_eq!(store.import("assign", Provider::Codex, &file).unwrap()["unchanged"], true);
    fs::write(&file, format!("{original}broken\n")).unwrap();
    assert!(store.import("assign", Provider::Codex, &file).is_err());
    assert_eq!(reader.search("assign", "old", 8, None, None).unwrap()["total_matches"], 1);
    fs::write(&file, jsonl(&[codex("user", "new stable layout")])).unwrap();
    store.import("assign", Provider::Codex, &file).unwrap();
    assert_eq!(reader.search("assign", "old", 8, None, None).unwrap()["total_matches"], 0);
    assert_eq!(reader.search("assign", "stable", 8, None, None).unwrap()["total_matches"], 1);
    store.forget("assign", imported["source_id"].as_str().unwrap()).unwrap();
    assert_eq!(reader.search("assign", "stable", 8, None, None).unwrap()["total_matches"], 0);
    assert!(file.exists());
}

#[test]
fn query_bounds_unknown_fields_and_unicode_preview() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("index"), true).unwrap();
    let file = temp.path().join("log.jsonl");
    let text = format!("status {}", "ő界🙂".repeat(600));
    fs::write(&file, jsonl(&[claude("user", &text)])).unwrap();
    store.import("assign", Provider::Claude, &file).unwrap();
    assert!(store.search("assign", "status", 0, None, None).is_err());
    assert!(store.search("assign", "status", 21, None, None).is_err());
    assert!(store.search("assign", "", 8, None, None).is_err());
    assert!(store.search("assign", "status", 8, Some("system"), None).is_err());
    assert!(serde_json::from_value::<super::server::SearchRequest>(json!({
        "query":"status", "workspace":"escape"
    })).is_err());
    let hit = store.search("assign", "status", 8, None, None).unwrap();
    assert_eq!(hit["results"][0]["preview_truncated"], true);
    let id = hit["results"][0]["message"]["id"].as_str().unwrap();
    assert_eq!(store.read("assign", id).unwrap()["message"]["text"], text);
}

#[test]
fn rejects_other_index_schema_and_nonempty_directory() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("keep.txt"), "do not change").unwrap();
    assert!(Store::open(temp.path(), true).is_err());
    assert_eq!(fs::read_to_string(temp.path().join("keep.txt")).unwrap(), "do not change");
    let other = tempfile::tempdir().unwrap();
    tantivy::Index::create_in_dir(other.path(), tantivy::schema::Schema::builder().build()).unwrap();
    assert!(Store::open(other.path(), true).is_err());
}
