//! Versioned, conservative transcript adapters. Text blocks only; no inference.
use anyhow::{Context, Result, ensure};
use clap::ValueEnum;
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const MAX_FILE: usize = 32 * 1024 * 1024;
const MAX_LINE: usize = 1024 * 1024;
const MAX_MESSAGE: usize = 64 * 1024;
const MAX_MESSAGES: usize = 20_000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ValueEnum, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Codex,
    Claude,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub source_id: String,
    pub source_path: String,
    pub provider: String,
    pub thread_id: String,
    pub native_message_id: Option<String>,
    pub timestamp: Option<String>,
    pub line: usize,
    pub role: String,
    pub text: String,
    pub content_sha256: String,
}

pub struct Transcript {
    pub messages: Vec<Message>,
    pub skipped_records: usize,
    pub duplicate_records: usize,
    pub pending_tail_bytes: usize,
}

/// Length-prefixed components prevent delimiter collisions in opaque identifiers.
pub fn digest(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    hex::encode(hash.finalize())
}

pub fn content_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn text_content(content: &Value, provider: Provider) -> String {
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }
    content.as_array().into_iter().flatten().filter_map(|block| {
        let kind = block.get("type")?.as_str()?;
        let accepted = match provider {
            Provider::Codex => matches!(kind, "input_text" | "output_text"),
            Provider::Claude => kind == "text",
        };
        accepted.then(|| block.get("text").and_then(Value::as_str)).flatten()
    }).collect::<Vec<_>>().join("\n")
}

fn injected_user_context(text: &str) -> bool {
    let text = text.trim_start();
    ["# AGENTS.md instructions", "<environment_context>", "<permissions instructions>"]
        .iter().any(|prefix| text.starts_with(prefix))
}

/// Only newline-terminated records are committed; an active partial tail waits.
/// Codex response_item/message is canonical. event_msg mirrors are not imported.
pub fn parse(
    bytes: &[u8],
    provider: Provider,
    source_id: &str,
    source_path: &str,
) -> Result<Transcript> {
    ensure!(bytes.len() <= MAX_FILE, "transcript exceeds 32 MiB import limit");
    let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let text = std::str::from_utf8(&bytes[..complete]).context("transcript is not UTF-8")?;
    let mut messages = BTreeMap::new();
    let mut thread = String::new();
    let mut recognized = 0usize;
    let mut skipped = 0;
    let mut duplicates = 0;
    for (offset, line) in text.lines().enumerate() {
        let number = offset + 1;
        ensure!(number <= 100_000, "transcript exceeds 100,000 records");
        ensure!(line.len() <= MAX_LINE, "record {number} exceeds 1 MiB");
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .with_context(|| format!("invalid complete JSON record at line {number}"))?;
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        let payload = match provider {
            Provider::Codex => {
                if kind == "session_meta" {
                    if let Some(id) = value.pointer("/payload/id").and_then(Value::as_str) {
                        ensure!(thread.is_empty() || thread == id, "mixed session IDs");
                        thread = id.to_owned();
                    }
                    skipped += 1;
                    continue;
                }
                if kind != "response_item"
                    || value.pointer("/payload/type").and_then(Value::as_str) != Some("message")
                {
                    skipped += 1;
                    continue;
                }
                &value["payload"]
            }
            Provider::Claude => {
                if !matches!(kind, "user" | "assistant")
                    || ["isMeta", "isCompactSummary", "isSidechain"].iter()
                        .any(|key| value.get(*key).and_then(Value::as_bool) == Some(true))
                {
                    skipped += 1;
                    continue;
                }
                if let Some(id) = value.get("sessionId").and_then(Value::as_str) {
                    ensure!(thread.is_empty() || thread == id, "mixed session IDs");
                    thread = id.to_owned();
                }
                &value["message"]
            }
        };
        let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
        if !matches!(role, "user" | "assistant")
            || payload.get("channel").and_then(Value::as_str) == Some("analysis")
        {
            skipped += 1;
            continue;
        }
        recognized += 1;
        let body = text_content(&payload["content"], provider);
        if body.trim().is_empty() || (role == "user" && injected_user_context(&body)) {
            skipped += 1;
            continue;
        }
        ensure!(body.len() <= MAX_MESSAGE, "message at line {number} exceeds 64 KiB");
        let native_id = match provider {
            Provider::Codex => payload.get("id"),
            Provider::Claude => value.get("uuid"),
        }.and_then(Value::as_str).map(str::to_owned);
        let timestamp = value.get("timestamp").and_then(Value::as_str).map(str::to_owned);
        ensure!(native_id.as_ref().is_none_or(|id| id.len() <= 256), "message ID too long");
        ensure!(timestamp.as_ref().is_none_or(|time| time.len() <= 128), "timestamp too long");
        ensure!(thread.len() <= 256, "thread ID too long");
        let position = number.to_string();
        let identity = native_id.as_deref().unwrap_or(&position);
        let id = digest(&[source_id, role, identity]);
        let message = Message {
            id: id.clone(), source_id: source_id.to_owned(),
            source_path: source_path.to_owned(), provider: provider.name().to_owned(),
            thread_id: thread.clone(), native_message_id: native_id, timestamp,
            line: number, role: role.to_owned(), content_sha256: content_hash(body.as_bytes()),
            text: body,
        };
        if messages.insert(id, message).is_some() {
            duplicates += 1;
        }
        ensure!(messages.len() <= MAX_MESSAGES, "transcript exceeds 20,000 messages");
    }
    ensure!(recognized > 0, "no supported user/assistant records; wrong format or unfinished transcript");
    let mut messages: Vec<Message> = messages.into_values().collect();
    for message in &mut messages {
        if message.thread_id.is_empty() {
            message.thread_id = if thread.is_empty() { source_id.to_owned() } else { thread.clone() };
        }
    }
    messages.sort_by_key(|message| message.line);
    Ok(Transcript {
        messages, skipped_records: skipped, duplicate_records: duplicates,
        pending_tail_bytes: bytes.len() - complete,
    })
}
